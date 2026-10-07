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
use crate::ai::vision::VisionHelper;
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
    pub vision: Arc<VisionHelper>,
    pub config: Arc<Config>,
    pub error_logger: Arc<ErrorLogger>,
    pub locale: Arc<LocaleManager>,
    pub open_meteo_quota: Arc<ApiQuotaGovernor>,
    pub irc_raw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub plugins_info: Arc<RwLock<Vec<PluginDescriptor>>>,
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
}

impl PluginManager {
    pub fn new(ctx: PluginContext) -> Self {
        Self {
            plugins: Vec::new(),
            ctx,
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

            let cmd = CommandEvent {
                platform: msg.platform.clone(),
                channel: msg.channel.clone(),
                author: msg.author.clone(),
                trigger: canonical_trigger.clone(),
                args,
                is_operator,
                is_owner,
            };

            for p in &self.plugins {
                if p.triggers().contains(&canonical_trigger.as_str()) || p.triggers().contains(&raw_trigger.as_str()) {
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
        let futures = self.plugins.iter().map(|p| {
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
