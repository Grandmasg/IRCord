pub mod whatpulse;
pub mod ai;
pub mod presence;
pub mod afk;
pub mod weather;
pub mod slap;
pub mod admin;
pub mod alias;
pub mod quotes;
pub mod karma;
pub mod sed;
pub mod url_titler;
pub mod safety;
pub mod poll;
pub mod remind;
pub mod reactions;
pub mod tell;
pub mod youtube;
pub mod google;
pub mod wiki;
pub mod crypto;
pub mod urban;
pub mod minecraft;
pub mod translate;
pub mod time;
pub mod birthday;
pub mod identity;
pub mod sysadmin;
pub mod rss;
pub mod tech;
pub mod rhai;
pub mod lang;
pub mod rephrase;
pub mod countdown;
pub mod vakantie;
pub mod toggles;
pub mod plugin_admin;
pub mod media;
pub mod backup;
pub mod chatsearch;
pub mod timer;
pub mod lookup;
pub mod calc;
pub mod stats;
pub mod track;
pub mod help;
pub mod profile;
pub mod channel_ops;
pub mod games;

use async_trait::async_trait;
use reqwest::Client;
use sqlx::SqlitePool;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};
use tracing::{error, info, warn};

use crate::ai::freetoken::FreeTokenClient;
use crate::ai::manager::AiManager;
use crate::ai::rag::RagSearcher;
use crate::config::Config;
use crate::utils::error_log::ErrorLogger;
use crate::utils::i18n::LocaleManager;
use crate::utils::quota::ApiQuotaGovernor;

#[derive(Clone, Debug)]
pub struct PluginDescriptor {
    pub name: String,
    pub triggers: Vec<String>,
    pub help: String,
}

#[derive(Clone)]
pub struct PluginContext {
    pub db: SqlitePool,
    pub http: Client,
    pub ai_client: Arc<FreeTokenClient>,
    pub ai_manager: Arc<AiManager>,
    pub rag: Arc<RagSearcher>,
    pub config: Arc<Config>,
    pub error_logger: Arc<ErrorLogger>,
    pub locale: Arc<LocaleManager>,
    pub open_meteo_quota: Arc<ApiQuotaGovernor>,
    pub irc_raw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub plugins_info: Arc<RwLock<Vec<PluginDescriptor>>>,
    pub toggles: Arc<toggles::PluginToggles>,
    /// Kanaal naar de Discord-zijde voor plugins die bestanden of afbeeldingen willen doorsturen.
    pub discord_post_tx: Option<tokio::sync::mpsc::Sender<crate::discord::webhook::DiscordPost>>,
}

impl PluginContext {
    pub async fn send_irc_raw(&self, cmd: impl Into<String>) {
        if let Some(ref tx) = self.irc_raw_tx {
            let _ = tx.send(cmd.into()).await;
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandEvent {
    pub platform: String, // "irc" of "discord"
    pub channel: String,
    pub author: String,
    pub trigger: String, // bijv. "wp", "weer", "ai"
    pub args: String,
    pub is_operator: bool,
    pub is_moderator: bool,
    pub is_owner: bool,
}

#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub platform: String,
    pub channel: String,
    pub author: String,
    pub author_id: Option<String>,
    pub content: String,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn triggers(&self) -> &[&'static str] { &[] }
    fn help(&self) -> &'static str;

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let _ = (ctx, cmd);
        Ok(None)
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let _ = (ctx, msg);
        Ok(None)
    }
}

pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
    ctx: PluginContext,
    limiter: crate::utils::ratelimit::UserRateLimiter,
}

impl PluginManager {
    pub fn new(ctx: PluginContext) -> Self {
        let limiter = crate::utils::ratelimit::UserRateLimiter::new(
            ctx.config.moderation.expensive_commands_per_minute,
            std::time::Duration::from_secs(60),
        );
        Self {
            plugins: Vec::new(),
            ctx,
            limiter,
        }
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        info!("Plugin geregistreerd: [{}]", plugin.name());
        let desc = PluginDescriptor {
            name: plugin.name().to_string(),
            triggers: plugin.triggers().iter().map(|s| s.to_string()).collect(),
            help: plugin.help().to_string(),
        };
        if let Ok(mut lock) = self.ctx.plugins_info.write() {
            lock.push(desc);
        }
        self.plugins.push(plugin);
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Handelt inkomende chatberichten af met panic isolation boundaries (catch_unwind)
    pub async fn dispatch_message(&self, msg: MessageEvent) -> Vec<String> {
        let mut responses = Vec::new();
        let trimmed = msg.content.trim();

        // 1. Is it a command? (Configured command prefixes e.g. ! or .)
        if let Some(clean) = self.ctx.config.general.strip_command_prefix(trimmed) {
            let mut parts = clean.splitn(2, ' ');
            let raw_trigger = parts.next().unwrap_or("").to_lowercase();
            let canonical_trigger = self.ctx.locale.resolve_alias(&raw_trigger).to_string();
            let args = parts.next().unwrap_or("").to_string();

            let general = &self.ctx.config.general;
            let is_owner = general.is_owner(&msg.platform, &msg.author, msg.author_id.as_deref());
            let is_operator = general.is_operator(&msg.platform, &msg.author, msg.author_id.as_deref());
            let is_moderator = general.is_moderator(&msg.platform, &msg.author, msg.author_id.as_deref());

            let cmd = CommandEvent {
                platform: msg.platform.clone(),
                channel: msg.channel.clone(),
                author: msg.author.clone(),
                trigger: canonical_trigger.clone(),
                args,
                is_operator,
                is_moderator,
                is_owner,
            };

            // Dure commando's (AI, netwerk, vertaling) zijn per gebruiker begrensd; operators zijn vrijgesteld.
            let expensive = crate::utils::ratelimit::EXPENSIVE_COMMANDS
                .iter()
                .any(|t| *t == canonical_trigger.as_str() || *t == raw_trigger.as_str());
            if expensive && !is_operator {
                let key = format!("{}:{}", msg.platform, msg.author);
                if let Err(wait) = self.limiter.check(&key, std::time::Instant::now()) {
                    let text = if self.ctx.locale.is_dutch() {
                        format!("⏳ Rustig aan {}, je gebruikt dit commando te vaak. Probeer het over {}s opnieuw.", msg.author, wait)
                    } else {
                        format!("⏳ Slow down {}, too many requests. Try again in {}s.", msg.author, wait)
                    };
                    return vec![text];
                }
            }

            for p in &self.plugins {
                if p.triggers().contains(&canonical_trigger.as_str()) || p.triggers().contains(&raw_trigger.as_str()) {
                    if self.ctx.toggles.is_disabled(&msg.channel, p.name()) {
                        continue;
                    }
                    let mut ctx = self.ctx.clone();
                    // Language precedence: 1. User preference -> 2. Channel language -> 3. Global default
                    let effective_lang = if let Some(pref) = self.ctx.locale.get_user_preference(&msg.platform, &msg.author) {
                        pref
                    } else {
                        self.ctx.config.channel_language(&msg.platform, &msg.channel).to_string()
                    };

                    if effective_lang != ctx.locale.language() {
                        ctx.locale = Arc::new(self.ctx.locale.for_language(&effective_lang));
                    }
                    let cmd_clone = cmd.clone();
                    let plugin_name = p.name();

                    // Panic isolation boundary
                    let result = futures_util::FutureExt::catch_unwind(AssertUnwindSafe(
                        p.on_command(&ctx, &cmd_clone)
                    )).await;

                    match result {
                        Ok(Ok(Some(reply))) => responses.push(reply),
                        Ok(Err(err)) => {
                            let err_msg = format!("Fout in plugin [{}] bij commando !{}: {}", plugin_name, cmd.trigger, err);
                            warn!("{}", err_msg);
                            self.ctx.error_logger.record("WARN", plugin_name, &err_msg);
                        }
                        Err(_) => {
                            let panic_msg = format!("PANIC onderschept in plugin [{}] bij commando !{}!", plugin_name, cmd.trigger);
                            error!("{}", panic_msg);
                            self.ctx.error_logger.record("PANIC", plugin_name, &panic_msg);
                        }
                        _ => {}
                    }
                }
            }
            return responses;
        }

        // 2. Reguliere chat-pass-through voor passieve plugins.
        // De plugins draaien gelijktijdig (een trage AI-aanroep houdt de rest niet op), elk met panic-isolatie
        // en een harde time-out. De volgorde van de antwoorden blijft de registratievolgorde.
        const PASSIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let futures = self.plugins.iter().filter(|p| !self.ctx.toggles.is_disabled(&msg.channel, p.name())).map(|p| {
            let ctx = self.ctx.clone();
            let msg_clone = msg.clone();
            let plugin_name = p.name();
            async move {
                let guarded = futures_util::FutureExt::catch_unwind(AssertUnwindSafe(p.on_message(&ctx, &msg_clone)));
                (plugin_name, tokio::time::timeout(PASSIVE_TIMEOUT, guarded).await)
            }
        });

        for (plugin_name, outcome) in futures_util::future::join_all(futures).await {
            match outcome {
                Ok(Ok(Ok(Some(reply)))) => responses.push(reply),
                Ok(Ok(Err(err))) => {
                    let err_msg = format!("Fout in plugin [{}] bij berichtverwerking: {}", plugin_name, err);
                    warn!("{}", err_msg);
                    self.ctx.error_logger.record("WARN", plugin_name, &err_msg);
                }
                Ok(Err(_)) => {
                    let panic_msg = format!("PANIC onderschept in plugin [{}] bij berichtverwerking!", plugin_name);
                    error!("{}", panic_msg);
                    self.ctx.error_logger.record("PANIC", plugin_name, &panic_msg);
                }
                Err(_) => {
                    let msg = format!("Plugin [{}] overschreed de time-out van 30s bij berichtverwerking", plugin_name);
                    warn!("{}", msg);
                    self.ctx.error_logger.record("WARN", plugin_name, &msg);
                }
                _ => {}
            }
        }

        responses
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn manager(extra_general: &str, extra_moderation: &str) -> PluginManager {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let config = Arc::new(crate::config::test_config(extra_general, extra_moderation));
        let plugins_info = Arc::new(RwLock::new(Vec::new()));
        let ctx = PluginContext {
            db: pool.clone(),
            http: Client::new(),
            ai_client: Arc::new(FreeTokenClient::new("http://127.0.0.1:9".into(), "m".into(), 10, 0.1, None)),
            ai_manager: Arc::new(AiManager::new("m".into(), 1000)),
            rag: Arc::new(RagSearcher::new(pool.clone())),
            config: config.clone(),
            error_logger: Arc::new(ErrorLogger::new(10)),
            locale: Arc::new(LocaleManager::load("locales", "nl")),
            open_meteo_quota: Arc::new(ApiQuotaGovernor::new("t", 10, 10, 10)),
            irc_raw_tx: None,
            plugins_info: plugins_info.clone(),
            toggles: Arc::new(toggles::PluginToggles::from_config(&config)),
            discord_post_tx: None,
        };
        let mut mgr = PluginManager::new(ctx);
        mgr.register(Box::new(help::HelpPlugin));
        mgr.register(Box::new(plugin_admin::PluginAdminPlugin));
        mgr.register(Box::new(channel_ops::ChannelOpsPlugin));
        mgr.register(Box::new(slap::SlapPlugin));
        mgr
    }

    fn irc(author: &str, account: Option<&str>, content: &str) -> MessageEvent {
        MessageEvent {
            platform: "irc".into(),
            channel: "#test".into(),
            author: author.into(),
            author_id: account.map(|a| format!("irc-account:{a}")),
            content: content.into(),
        }
    }

    #[tokio::test]
    async fn owner_identity_and_roles_gate_channel_operations() {
        let mgr = manager("moderator_irc_accounts = [\"mod\"]", "").await;

        // Zelfde nick als de eigenaar zonder bevestigd account: geweigerd
        let r = mgr.dispatch_message(irc("BossNick", None, "!kick henk")).await;
        assert!(r[0].contains("Toegang geweigerd"), "{r:?}");
        // Gewone gebruiker met een eigen account: geweigerd
        let r = mgr.dispatch_message(irc("henk", Some("henk"), "!kick piet")).await;
        assert!(r[0].contains("Toegang geweigerd"), "{r:?}");
        // Eigenaar met bevestigd account mag kicken (er is geen IRC-verbinding, dus alleen de melding)
        let r = mgr.dispatch_message(irc("wie_dan_ook", Some("Boss"), "!kick henk")).await;
        assert!(r[0].contains("gekickt"), "{r:?}");
        // Moderator mag kicken maar geen op geven
        let r = mgr.dispatch_message(irc("m", Some("mod"), "!kick henk")).await;
        assert!(r[0].contains("gekickt"), "{r:?}");
        let r = mgr.dispatch_message(irc("m", Some("mod"), "!op henk")).await;
        assert!(r[0].contains("Toegang geweigerd"), "{r:?}");
    }

    #[tokio::test]
    async fn expensive_commands_are_rate_limited_but_operators_are_exempt() {
        let mgr = manager("", "expensive_commands_per_minute = 2").await;
        for _ in 0..2 {
            let r = mgr.dispatch_message(irc("spammer", None, "!dns example.com")).await;
            assert!(r.is_empty() || !r[0].contains("Rustig aan"), "{r:?}");
        }
        let r = mgr.dispatch_message(irc("spammer", None, "!dns example.com")).await;
        assert!(r[0].contains("Rustig aan") && r[0].contains("spammer"), "{r:?}");
        // andere gebruiker is niet geraakt, eigenaar is vrijgesteld
        let r = mgr.dispatch_message(irc("ander", None, "!dns example.com")).await;
        assert!(r.is_empty() || !r[0].contains("Rustig aan"), "{r:?}");
        for _ in 0..5 {
            let r = mgr.dispatch_message(irc("boss", Some("Boss"), "!dns example.com")).await;
            assert!(r.is_empty() || !r[0].contains("Rustig aan"), "{r:?}");
        }
    }

    #[tokio::test]
    async fn plugins_can_be_disabled_per_channel_by_operators_only() {
        let mgr = manager("", "").await;
        // werkt eerst
        let r = mgr.dispatch_message(irc("henk", None, "!slap piet")).await;
        assert!(!r.is_empty(), "slap hoort te antwoorden");

        // gewone gebruiker kan niet uitschakelen
        let r = mgr.dispatch_message(irc("henk", None, "!plugin disable slap")).await;
        assert!(r[0].contains("Alleen operators"), "{r:?}");
        // eigenaar wel
        let r = mgr.dispatch_message(irc("x", Some("Boss"), "!plugin disable slap")).await;
        assert!(r[0].contains("uitgeschakeld"), "{r:?}");
        let r = mgr.dispatch_message(irc("henk", None, "!slap piet")).await;
        assert!(r.is_empty(), "uitgeschakelde plugin mag niet antwoorden: {r:?}");
        // kernplugins kunnen niet uit
        let r = mgr.dispatch_message(irc("x", Some("Boss"), "!plugin disable help")).await;
        assert!(r[0].contains("kan niet worden uitgeschakeld"), "{r:?}");
        // weer aan
        mgr.dispatch_message(irc("x", Some("Boss"), "!plugin enable slap")).await;
        let r = mgr.dispatch_message(irc("henk", None, "!slap piet")).await;
        assert!(!r.is_empty());
    }
}
