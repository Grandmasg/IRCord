mod ai;
mod bridge;
mod config;
mod discord;
mod irc;
mod plugins;
mod utils;
mod web;

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serenity::prelude::GatewayIntents;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use ai::freetoken::FreeTokenClient;
use ai::manager::AiManager;
use ai::rag::RagSearcher;
use ai::vision::VisionHelper;
use bridge::router::BridgeRouter;
use bridge::{BridgeMessage, Platform};
use config::Config;
use discord::handler::DiscordHandler;
use discord::webhook::{AvatarResolver, WebhookDispatcher};
use irc::client::IrcClient;
use plugins::{
    admin::AdminPlugin, afk::AfkPlugin, ai::AiPlugin, alias::AliasPlugin, crypto::CryptoPlugin,
    google::GooglePlugin, karma::KarmaPlugin, minecraft::MinecraftPlugin, poll::PollPlugin,
    presence::PresencePlugin, quotes::QuotesPlugin, reactions::ReactionsPlugin,
    remind::RemindPlugin, safety::SafetyPlugin, sed::SedPlugin, slap::SlapPlugin,
    tell::TellPlugin, time::TimePlugin, translate::TranslatePlugin, urban::UrbanDictionaryPlugin,
    url_titler::UrlTitlerPlugin, weather::WeatherPlugin, whatpulse::WhatPulsePlugin,
    wiki::WikipediaPlugin, youtube::YouTubePlugin, birthday::BirthdayPlugin,
    identity::IdentityPlugin, sysadmin::SysadminPlugin, rss::RssPlugin, tech::TechPlugin,
    rhai::RhaiPlugin, lang::LangPlugin, rephrase::RephrasePlugin, countdown::CountdownPlugin, vakantie::VakantiePlugin,
    stats::StatsPlugin, track::TrackPlugin,
    help::HelpPlugin, profile::ProfilePlugin, channel_ops::ChannelOpsPlugin, games::GamesPlugin,
    MessageEvent, PluginContext, PluginManager,
};
use utils::error_log::ErrorLogger;
use utils::i18n::LocaleManager;
use utils::lifecycle::ShutdownManager;
use web::WebServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Initialiseer omgevingsvariabelen vanuit .env
    dotenvy::dotenv().ok();

    // 2. Initialiseer gestructureerde logging (tracing)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,ircord=debug")),
        )
        .init();

    info!("===========================================================");
    info!("   IRCord: Hybride IRC-Discord AI Bot Daemon (Rust 2021)   ");
    info!("===========================================================");

    // 3. Laad en valideer configuratie (via CONFIG_TOML omgevingsvariabele of bestand)
    let cfg = match std::env::var("CONFIG_TOML") {
        Ok(inline_toml) if !inline_toml.trim().is_empty() => {
            info!("Configuratie succesvol geladen vanuit CONFIG_TOML omgevingsvariabele (Compose/YAML)");
            match Config::load_from_str(&inline_toml) {
                Ok(c) => c,
                Err(err) => {
                    error!("Fout bij parsen van inline CONFIG_TOML omgevingsvariabele: {}", err);
                    return Err(err);
                }
            }
        }
        _ => {
            let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
            match Config::load_from_file(&config_path) {
                Ok(c) => {
                    info!("Configuratie succesvol geladen vanuit bestand: {}", config_path);
                    c
                }
                Err(err) => {
                    error!("Fout bij laden van configuratie vanuit bestand {}: {}", config_path, err);
                    return Err(err);
                }
            }
        }
    };

    crate::plugins::admin::AdminPlugin::init_start_time();
    if cfg.general.bot_owner_irc_account.trim().is_empty() {
        warn!(
            "bot_owner_irc_account is niet ingesteld: eigenaar-rechten op IRC worden alleen op nick '{}' gebaseerd en kunnen door nick-overname worden misbruikt. Stel bot_owner_irc_account in.",
            cfg.general.bot_owner_irc_nick
        );
    }
    info!("Geconfigureerde bridge-kanalen: {} mappings actief", cfg.channels.len());
    for m in &cfg.channels {
        info!("  [Bridge] IRC: {} <===> Discord Channel: {}", m.irc_channel, m.discord_channel_id);
    }

    // 4. Initialiseer SQLite Database Pool & Migraties
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://ircord.db".to_string());
    info!("Verbinden met SQLite database: {}", db_url);

    let connect_options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await?;

    info!("Uitvoeren van database migraties (schema + FTS5)...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;
    info!("Database migraties succesvol toegepast!");

    // 5. Initialiseer HTTP client & AI componenten (FlashML FreeToken, RAG, VLM)
    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let ai_base_url = std::env::var("FREETOKEN_BASE_URL").unwrap_or_else(|_| cfg.ai.base_url.clone());
    let ai_model = std::env::var("FREETOKEN_MODEL").unwrap_or_else(|_| cfg.ai.default_model.clone());
    let ai_api_key = std::env::var("FREETOKEN_API_KEY").ok().filter(|s| !s.trim().is_empty());

    let free_token_client = Arc::new(FreeTokenClient::new(
        ai_base_url,
        ai_model.clone(),
        cfg.ai.max_tokens,
        cfg.ai.temperature,
        ai_api_key,
    ));
    let ai_manager = Arc::new(AiManager::new(
        ai_model.clone(),
        cfg.ai.hourly_token_budget,
    ));
    let rag_searcher = Arc::new(RagSearcher::new(pool.clone()));
    let vision_helper = Arc::new(VisionHelper::new());

    // Asynchrone AI model pre-warm bij het opstarten zodat het direct klaarstaat in GPU VRAM
    let warm_client = free_token_client.clone();
    let warm_model = ai_model.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        info!("🔥 AI Model pre-warm gestart: inladen van model '{}' in GPU VRAM...", warm_model);
        match warm_client.ask("Warmup", "Hallo", Some(&warm_model)).await {
            Ok(_) => info!("✅ AI Model '{}' succesvol voorverwarmd en geladen in GPU VRAM!", warm_model),
            Err(e) => warn!("⚠️ AI Model pre-warm niet gelukt (laadt alsnog bij eerste vraag): {}", e),
        }
    });

    // 6. Bouw Plugin Context en registreer alle plugins
    let cfg_arc = Arc::new(cfg.clone());
    let error_logger = Arc::new(ErrorLogger::new(100));
    let locale_manager = Arc::new(LocaleManager::load("locales", &cfg.general.language));
    if let Err(e) = locale_manager.load_preferences_from_db(&pool).await {
        warn!("Failed to load user language preferences: {}", e);
    }
    let open_meteo_quota = Arc::new(crate::utils::quota::ApiQuotaGovernor::new(
        "Open-Meteo",
        cfg.open_meteo.minutely_limit,
        cfg.open_meteo.hourly_limit,
        cfg.open_meteo.daily_limit,
    ));

    // 7. Initialiseer communicatiekanalen (mpsc channels)
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<BridgeMessage>(256);
    let (outbound_irc_tx, outbound_irc_rx) = mpsc::channel::<BridgeMessage>(256);
    let (outbound_irc_raw_tx, outbound_irc_raw_rx) = mpsc::channel::<String>(128);
    let (discord_post_tx, mut discord_post_rx) = mpsc::channel::<crate::discord::webhook::DiscordPost>(16);
    let (presence_tx, mut presence_rx) = mpsc::channel::<crate::bridge::PresenceEvent>(64);
    let (discord_event_tx, mut discord_event_rx) = mpsc::channel::<crate::bridge::DiscordEvent>(64);

    let plugins_info = Arc::new(std::sync::RwLock::new(Vec::new()));
    let plugin_toggles = Arc::new(crate::plugins::toggles::PluginToggles::from_config(&cfg));
    plugin_toggles.load_db(&pool).await;
    let plugin_ctx = PluginContext {
        db: pool.clone(),
        http: http_client.clone(),
        ai_client: free_token_client.clone(),
        ai_manager: ai_manager.clone(),
        rag: rag_searcher.clone(),
        config: cfg_arc.clone(),
        error_logger: error_logger.clone(),
        locale: locale_manager.clone(),
        open_meteo_quota: open_meteo_quota.clone(),
        irc_raw_tx: Some(outbound_irc_raw_tx.clone()),
        plugins_info: plugins_info.clone(),
        toggles: plugin_toggles.clone(),
        discord_post_tx: Some(discord_post_tx),
    };

    let mut plugin_mgr = PluginManager::new(plugin_ctx);
    plugin_mgr.register(Box::new(HelpPlugin));
    plugin_mgr.register(Box::new(ProfilePlugin));
    plugin_mgr.register(Box::new(ChannelOpsPlugin));
    plugin_mgr.register(Box::new(GamesPlugin::new()));
    plugin_mgr.register(Box::new(LangPlugin));
    plugin_mgr.register(Box::new(WhatPulsePlugin::new()));
    plugin_mgr.register(Box::new(AiPlugin));
    plugin_mgr.register(Box::new(PresencePlugin));
    plugin_mgr.register(Box::new(AfkPlugin::new()));
    plugin_mgr.register(Box::new(WeatherPlugin::new()));
    plugin_mgr.register(Box::new(SlapPlugin));
    plugin_mgr.register(Box::new(AdminPlugin));
    plugin_mgr.register(Box::new(AliasPlugin));
    plugin_mgr.register(Box::new(QuotesPlugin));
    plugin_mgr.register(Box::new(KarmaPlugin));
    plugin_mgr.register(Box::new(SedPlugin::new()));
    plugin_mgr.register(Box::new(UrlTitlerPlugin));
    plugin_mgr.register(Box::new(SafetyPlugin));
    plugin_mgr.register(Box::new(PollPlugin));
    plugin_mgr.register(Box::new(RemindPlugin));
    plugin_mgr.register(Box::new(ReactionsPlugin::new()));
    plugin_mgr.register(Box::new(TellPlugin));
    plugin_mgr.register(Box::new(YouTubePlugin));
    plugin_mgr.register(Box::new(GooglePlugin));
    plugin_mgr.register(Box::new(WikipediaPlugin));
    plugin_mgr.register(Box::new(CryptoPlugin));
    plugin_mgr.register(Box::new(UrbanDictionaryPlugin));
    plugin_mgr.register(Box::new(MinecraftPlugin));
    plugin_mgr.register(Box::new(TranslatePlugin::new()));
    plugin_mgr.register(Box::new(TimePlugin));
    plugin_mgr.register(Box::new(BirthdayPlugin));
    plugin_mgr.register(Box::new(IdentityPlugin::new()));
    plugin_mgr.register(Box::new(SysadminPlugin));
    plugin_mgr.register(Box::new(RssPlugin));
    plugin_mgr.register(Box::new(TechPlugin));
    plugin_mgr.register(Box::new(RhaiPlugin::new()));
    plugin_mgr.register(Box::new(RephrasePlugin::new()));
    plugin_mgr.register(Box::new(CountdownPlugin::new()));
    plugin_mgr.register(Box::new(VakantiePlugin::new()));
    plugin_mgr.register(Box::new(StatsPlugin));
    plugin_mgr.register(Box::new(crate::plugins::media::MediaPlugin));
    plugin_mgr.register(Box::new(crate::plugins::calc::CalcPlugin));
    plugin_mgr.register(Box::new(crate::plugins::backup::BackupPlugin));
    plugin_mgr.register(Box::new(crate::plugins::chatsearch::ChatSearchPlugin));
    plugin_mgr.register(Box::new(crate::plugins::timer::TimerPlugin));
    plugin_mgr.register(Box::new(crate::plugins::lookup::LookupPlugin));
    plugin_mgr.register(Box::new(crate::plugins::plugin_admin::PluginAdminPlugin));
    plugin_mgr.register(Box::new(TrackPlugin));

    info!("Plugin Manager geïnitialiseerd met {} actieve plugins", plugin_mgr.plugin_count());
    let plugin_mgr = Arc::new(plugin_mgr);

    let bridge_router = Arc::new(BridgeRouter::with_prefixes(
        cfg.channels.clone(),
        cfg.bridge.lru_cache_capacity,
        cfg.general.command_prefixes.clone(),
    ));
    let webhook_dispatcher = Arc::new(WebhookDispatcher::new());
    let discord_token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
    let discord_http: Option<Arc<serenity::http::Http>> = if cfg.bridge.sync_topic && !discord_token.is_empty() {
        Some(Arc::new(serenity::http::Http::new(&discord_token)))
    } else {
        None
    };
    let avatar_resolver = Arc::new(AvatarResolver::new(
        pool.clone(),
        discord_token.clone(),
        cfg.general.bot_owner_irc_nick.clone(),
        cfg.general.bot_owner_discord_id,
    ));

    // 8. Initialiseer Graceful Shutdown Handler
    let shutdown = ShutdownManager::new();
    let shutdown_token = shutdown.child_token();

    // 9. Start Axum Web Server (Port 9090: /health, /metrics, /api/errors, /api/github)
    let web_cfg = cfg_arc.clone();
    let web_error_logger = error_logger.clone();
    let web_shutdown = shutdown_token.clone();
    let (announce_tx, mut announce_rx) = mpsc::channel::<String>(64);
    let web_plugin_count = plugin_mgr.plugin_count();
    tokio::spawn(async move {
        if let Err(err) = WebServer::start(web_cfg, web_error_logger, web_plugin_count, announce_tx, web_shutdown).await {
            error!("Fout bij draaien van Axum Web Server: {:?}", err);
        }
    });

    // Meldingen van de webserver (bijv. GitHub events) naar alle gekoppelde IRC- en Discord-kanalen
    {
        let channels = cfg_arc.channels.clone();
        let irc_tx = outbound_irc_tx.clone();
        let dispatcher = webhook_dispatcher.clone();
        tokio::spawn(async move {
            while let Some(line) = announce_rx.recv().await {
                for m in &channels {
                    let _ = irc_tx.send(BridgeMessage {
                        source_platform: Platform::Irc,
                        source_channel: m.irc_channel.clone(),
                        author_name: "IRCord".into(),
                        author_id: None,
                        message_id: None,
                        content: line.clone(),
                        reply_to: None,
                        is_action: false,
                    }).await;
                    let _ = dispatcher.send_message(&m.discord_webhook_url, "GitHub", &line).await;
                }
            }
        });
    }

    // 10. Start IRC Client Task
    let irc_client = IrcClient::new(
        cfg_arc.clone(),
        inbound_tx.clone(),
        outbound_irc_rx,
        outbound_irc_raw_rx,
        if cfg.bridge.sync_presence { Some(presence_tx) } else { None },
        shutdown_token.clone(),
    );
    tokio::spawn(async move {
        irc_client.run().await;
    });

    // 11. Start optionele Serenity Discord Gateway Client Task
    if !discord_token.is_empty() && discord_token != "YOUR_DISCORD_BOT_TOKEN_HERE" {
        info!("Discord client opstarten...");
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILDS;

        let handler = DiscordHandler::new(cfg_arc.clone(), inbound_tx.clone(), discord_event_tx);
        let shutdown_discord = shutdown_token.clone();

        tokio::spawn(async move {
            match serenity::Client::builder(&discord_token, intents)
                .event_handler(handler)
                .await
            {
                Ok(mut client) => {
                    tokio::select! {
                        res = client.start() => {
                            if let Err(why) = res {
                                error!("Discord client fout: {:?}", why);
                            }
                        }
                        _ = shutdown_discord.cancelled() => {
                            info!("Discord client netjes afgesloten.");
                        }
                    }
                }
                Err(err) => {
                    error!("Fout bij initialiseren van Discord client: {:?}", err);
                }
            }
        });
    } else {
        warn!("DISCORD_BOT_TOKEN is niet geconfigureerd of is default; Discord gateway taak overgeslagen.");
    }

    // 12. Centraal Bridge Router & Plugin Dispatch Loop
    let router_clone = bridge_router.clone();
    let dispatcher_clone = webhook_dispatcher.clone();
    let rag_clone = rag_searcher.clone();
    let plugins_clone = plugin_mgr.clone();
    let irc_tx_clone = outbound_irc_tx.clone();
    let avatar_res_clone = avatar_resolver.clone();
    let vision_clone = vision_helper.clone();
    let ai_client_clone = free_token_client.clone();
    let ai_manager_clone = ai_manager.clone();
    let shutdown_loop = shutdown_token.clone();
    let paste_http = http_client.clone();
    let relay_pool = pool.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_loop.cancelled() => {
                    info!("Centrale bridge dispatch loop afgesloten.");
                    break;
                }
                Some(msg) = inbound_rx.recv() => {
                    let platform_str = match msg.source_platform {
                        Platform::Irc => "irc",
                        Platform::Discord => "discord",
                    };

                    // A. Log in FTS5 SQLite database via asynchrone batching-queue
                    let _ = rag_clone.log_message(
                        &msg.source_channel,
                        &msg.author_name,
                        platform_str,
                        &msg.content,
                    ).await;

                    // B. Voer plugin handlers uit
                    let responses = plugins_clone.dispatch_message(MessageEvent {
                        platform: platform_str.to_string(),
                        channel: msg.source_channel.clone(),
                        author: msg.author_name.clone(),
                        author_id: msg.author_id.clone(),
                        content: msg.content.clone(),
                    }).await;

                    for reply in responses {
                        match msg.source_platform {
                            Platform::Irc => {
                                let _ = irc_tx_clone.send(BridgeMessage {
                                    source_platform: Platform::Irc,
                                    source_channel: msg.source_channel.clone(),
                                    author_name: "IRCord".into(),
                                    author_id: None,
                                    message_id: None,
                                    content: reply,
                                    reply_to: None,
                                    is_action: false,
                                }).await;
                            }
                            Platform::Discord => {
                                if let Ok(discord_chan_id) = msg.source_channel.parse::<u64>() {
                                    if let Some(mapping) = router_clone.get_irc_destination(discord_chan_id) {
                                        let _ = dispatcher_clone.send_message(
                                            &mapping.discord_webhook_url,
                                            "IRCord",
                                            &reply,
                                        ).await;
                                    }
                                }
                            }
                        }
                    }

                    // C. Bridge relaying naar de andere zijde (indien geen genegeerd commando)
                    if router_clone.should_ignore(&msg.content) {
                        continue;
                    }

                    match msg.source_platform {
                        Platform::Irc => {
                            if let Some(mapping) = router_clone.get_discord_destination(&msg.source_channel) {
                                let (username, formatted) = router_clone.format_for_discord_webhook(&msg);
                                let url = mapping.discord_webhook_url.clone();
                                let disp = dispatcher_clone.clone();
                                let router = router_clone.clone();
                                let chan = msg.source_channel.clone();
                                let author = msg.author_name.clone();
                                let content = msg.content.clone();
                                let avatar_res = avatar_res_clone.clone();

                                let relay_pool = relay_pool.clone();
                                tokio::spawn(async move {
                                    let avatar_url = avatar_res.resolve_avatar(&author).await;

                                    // Webhooks kunnen niet echt antwoorden: "Bob: ..." krijgt een citaat van Bobs laatste bericht
                                    let mut text = formatted;
                                    if let Some(target) = crate::bridge::mentions::leading_addressee(&content) {
                                        if let Some(prev) = router.find_recent_by_author(&chan, &target) {
                                            text = format!("{}{}", crate::bridge::mentions::format_reply_quote(&prev.author, &prev.content), text);
                                        }
                                    }
                                    // Gekoppelde nicks (@nick of "nick:") worden echte Discord-mentions; alleen die ID's mogen pingen
                                    let (text, mention_ids) = crate::bridge::mentions::resolve_irc_mentions(&relay_pool, &text).await;

                                    match disp.send_relay(&url, &username, &text, avatar_url.as_deref(), &mention_ids).await {
                                        Err(e) => warn!("Fout bij versturen naar Discord Webhook: {}", e),
                                        // Echte Discord-bericht-ID koppelen (voor reacties en reply-citaten)
                                        Ok(Some(id)) => router.record_bridge_link(id, chan, author, content),
                                        Ok(None) => {}
                                    }
                                });
                            }
                        }
                        Platform::Discord => {
                            if let Ok(discord_chan_id) = msg.source_channel.parse::<u64>() {
                                if let Some(mapping) = router_clone.get_irc_destination(discord_chan_id) {
                                    // Lange (code)berichten: uploaden of inkorten zodat IRC niet wordt overspoeld
                                    let mut relay_msg = msg.clone();
                                    relay_msg.content = crate::utils::pastebin::shorten_for_irc(
                                        &paste_http,
                                        &msg.content,
                                        cfg_arc.general.pastebin_threshold_lines,
                                        cfg_arc.general.pastebin_enabled,
                                    ).await;
                                    let formatted = router_clone.format_for_irc(&relay_msg);
                                    if let Some(ref d_id) = msg.message_id {
                                        router_clone.record_bridge_link(
                                            d_id.clone(),
                                            mapping.irc_channel.clone(),
                                            msg.author_name.clone(),
                                            msg.content.clone(),
                                        );
                                    }
                                    let _ = irc_tx_clone.send(BridgeMessage {
                                        source_platform: Platform::Irc,
                                        source_channel: mapping.irc_channel.clone(),
                                        author_name: msg.author_name.clone(),
                                        author_id: msg.author_id.clone(),
                                        message_id: msg.message_id.clone(),
                                        content: formatted,
                                        reply_to: msg.reply_to.clone(),
                                        is_action: msg.is_action,
                                    }).await;

                                    // Optionele AI Vision Alt-Text voor afbeeldingen van Discord naar IRC
                                    let words: Vec<String> = msg.content.split_whitespace().map(String::from).collect();
                                    let v_helper = vision_clone.clone();
                                    let a_client = ai_client_clone.clone();
                                    let a_mgr = ai_manager_clone.clone();
                                    let irc_tx_vis = irc_tx_clone.clone();
                                    let is_dutch = cfg_arc.general.language == "nl";
                                    let irc_dest = mapping.irc_channel.clone();

                                    tokio::spawn(async move {
                                        for word in words {
                                            if word.starts_with("http://") || word.starts_with("https://") {
                                                let clean_url = word.split('?').next().unwrap_or(&word);
                                                if clean_url.ends_with(".png")
                                                    || clean_url.ends_with(".jpg")
                                                    || clean_url.ends_with(".jpeg")
                                                    || clean_url.ends_with(".webp")
                                                    || word.contains("cdn.discordapp.com/attachments/")
                                                {
                                                    let active_model = a_mgr.get_model();
                                                    if let Some(desc) = v_helper.get_or_describe(&word, &a_client, Some(&active_model)).await {
                                                        let vision_line = VisionHelper::format_for_irc(&desc, is_dutch);
                                                        let _ = irc_tx_vis.send(BridgeMessage {
                                                            source_platform: Platform::Irc,
                                                            source_channel: irc_dest.clone(),
                                                            author_name: "IRCord".into(),
                                                            author_id: None,
                                                            message_id: None,
                                                            content: vision_line,
                                                            reply_to: None,
                                                            is_action: false,
                                                        }).await;
                                                        break; // Maximaal 1 beschrijving per bericht
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    // 13. Achtergrondtaak: Geautomatiseerde verjaardagsfelicitaties (!bday)
    let bday_pool = pool.clone();
    let bday_irc_tx = outbound_irc_tx.clone();
    let bday_router = bridge_router.clone();
    let bday_dispatcher = webhook_dispatcher.clone();
    let bday_shutdown = shutdown_token.clone();

    tokio::spawn(async move {
        // Controleer elke 15 minuten op jarigen
        let mut interval = tokio::time::interval(Duration::from_secs(900));
        // Eerste tick direct afhandelen/overslaan zodat bot eerst rustig kan opstarten
        interval.tick().await;

        loop {
            tokio::select! {
                _ = bday_shutdown.cancelled() => {
                    info!("Verjaardag achtergrondtaak afgesloten.");
                    break;
                }
                _ = interval.tick() => {
                    match BirthdayPlugin::check_and_celebrate_birthdays(&bday_pool).await {
                        Ok(announcements) => {
                            for (channel, message) in announcements {
                                info!("🎉 Verjaardagsfelicitatie versturen naar kanaal {}", channel);

                                // 1. Stuur naar IRC
                                let _ = bday_irc_tx.send(BridgeMessage {
                                    source_platform: Platform::Irc,
                                    source_channel: channel.clone(),
                                    author_name: "IRCord".into(),
                                    author_id: None,
                                    message_id: None,
                                    content: message.clone(),
                                    reply_to: None,
                                    is_action: false,
                                }).await;

                                // 2. Stuur naar Discord indien kanaal gekoppeld is
                                if let Some(mapping) = bday_router.get_discord_destination(&channel) {
                                    let _ = bday_dispatcher.send_message(
                                        &mapping.discord_webhook_url,
                                        "IRCord",
                                        &message,
                                    ).await;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Fout bij controleren van verjaardagen: {:?}", e);
                        }
                    }
                }
            }
        }
    });

    // Topic-sync: gedeeld geheugen van het laatst gesynchroniseerde onderwerp per IRC-kanaal (voorkomt lussen)
    let topic_guard: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>> = Arc::default();

    // 12b. Synchronisatie van bewerkte/verwijderde Discord-berichten naar IRC
    {
        let router = bridge_router.clone();
        let irc_tx = outbound_irc_tx.clone();
        let irc_raw_tx = outbound_irc_raw_tx.clone();
        let topic_guard_ev = topic_guard.clone();
        tokio::spawn(async move {
            let mut window_start = std::time::Instant::now();
            let mut reactions_in_window = 0u32;
            while let Some(ev) = discord_event_rx.recv().await {
                let (channel_id, message_id, new_content) = match ev {
                    crate::bridge::DiscordEvent::Edited { channel_id, message_id, new_content } => (channel_id, message_id, Some(new_content)),
                    crate::bridge::DiscordEvent::Deleted { channel_id, message_id } => (channel_id, message_id, None),
                    crate::bridge::DiscordEvent::Topic { channel_id, topic } => {
                        let Some(mapping) = router.get_irc_destination(channel_id) else { continue };
                        let clean = crate::utils::sanitizer::sanitize_for_irc(&topic).chars().take(300).collect::<String>();
                        {
                            let mut guard = topic_guard_ev.lock().unwrap_or_else(|e| e.into_inner());
                            let key = mapping.irc_channel.to_lowercase();
                            if guard.get(&key).map(|t| t == &clean).unwrap_or(false) {
                                continue;
                            }
                            guard.insert(key, clean.clone());
                        }
                        let _ = irc_raw_tx.send(format!("TOPIC {} :{}", mapping.irc_channel, clean)).await;
                        continue;
                    }
                    crate::bridge::DiscordEvent::Reaction { channel_id, message_id, user, emoji } => {
                        // Reacties op berichten die wij doorgezet hebben (beide richtingen), met een eenvoudige limiet
                        if window_start.elapsed() > Duration::from_secs(10) {
                            window_start = std::time::Instant::now();
                            reactions_in_window = 0;
                        }
                        if reactions_in_window >= 10 {
                            continue;
                        }
                        let Some(mapping) = router.get_irc_destination(channel_id) else { continue };
                        let Some(origin) = router.lookup_irc_by_discord_id(&message_id) else { continue };
                        reactions_in_window += 1;
                        let _ = irc_tx.send(BridgeMessage {
                            source_platform: Platform::Irc,
                            source_channel: mapping.irc_channel.clone(),
                            author_name: "IRCord".into(),
                            author_id: None,
                            message_id: None,
                            content: router.format_reaction_for_irc(&user, &emoji, &origin.author),
                            reply_to: None,
                            is_action: false,
                        }).await;
                        continue;
                    }
                };
                let Some(mapping) = router.get_irc_destination(channel_id) else { continue };
                // Alleen berichten die wij zelf naar IRC hebben doorgezet
                let Some(origin) = router.lookup_irc_by_discord_id(&message_id) else { continue };
                let text = match &new_content {
                    Some(c) => {
                        router.record_bridge_link(message_id.clone(), mapping.irc_channel.clone(), origin.author.clone(), c.clone());
                        router.format_edit_for_irc(&origin.author, c)
                    }
                    None => router.format_delete_for_irc(&origin.author),
                };
                let _ = irc_tx.send(BridgeMessage {
                    source_platform: Platform::Irc,
                    source_channel: mapping.irc_channel.clone(),
                    author_name: "IRCord".into(),
                    author_id: None,
                    message_id: None,
                    content: text,
                    reply_to: None,
                    is_action: false,
                }).await;
            }
        });
    }

    // 12d. Bestanden en afbeeldingen van plugins (!img, !upload) naar Discord
    {
        let dispatcher = webhook_dispatcher.clone();
        let channels = cfg.channels.clone();
        tokio::spawn(async move {
            while let Some(post) = discord_post_rx.recv().await {
                let Some(m) = channels.iter().find(|m| m.irc_channel.eq_ignore_ascii_case(&post.irc_channel)) else { continue };
                let res = match (&post.file, &post.image_url) {
                    (Some((name, bytes)), _) => dispatcher.send_file(&m.discord_webhook_url, &post.username, &post.content, name, bytes.clone()).await,
                    (None, Some(url)) => dispatcher.send_image_embed(&m.discord_webhook_url, &post.username, &post.content, url).await,
                    (None, None) => dispatcher.send_message(&m.discord_webhook_url, &post.username, &post.content).await,
                };
                if let Err(e) = res {
                    warn!("Doorsturen naar Discord mislukt: {}", e);
                }
            }
        });
    }

    // 12c. IRC join/part/quit naar Discord (met een eenvoudige limiet tegen netsplit-floods)
    {
        let router = bridge_router.clone();
        let dispatcher = webhook_dispatcher.clone();
        let channels = cfg.channels.clone();
        let topic_http = discord_http.clone();
        let topic_guard_pr = topic_guard.clone();
        tokio::spawn(async move {
            let mut window_start = std::time::Instant::now();
            let mut sent_in_window = 0u32;
            while let Some(ev) = presence_rx.recv().await {
                // IRC-topic naar het Discord-kanaalonderwerp (vereist MANAGE_CHANNELS; Discord beperkt dit zelf sterk)
                if let crate::bridge::PresenceEvent::Topic { channel, topic } = &ev {
                    if let (Some(http), Some(m)) = (topic_http.as_ref(), channels.iter().find(|m| m.irc_channel.eq_ignore_ascii_case(channel))) {
                        let clean: String = crate::utils::sanitizer::strip_mirc_codes(topic).chars().take(1000).collect();
                        let fresh = {
                            let mut guard = topic_guard_pr.lock().unwrap_or_else(|e| e.into_inner());
                            let key = m.irc_channel.to_lowercase();
                            let same = guard.get(&key).map(|t| t == &clean).unwrap_or(false);
                            if !same {
                                guard.insert(key, clean.clone());
                            }
                            !same
                        };
                        if fresh {
                            let http = http.clone();
                            let id = m.discord_channel_id;
                            tokio::spawn(async move {
                                let edit = serenity::builder::EditChannel::new().topic(clean);
                                if let Err(e) = serenity::model::id::ChannelId::new(id).edit(&*http, edit).await {
                                    warn!("Discord-kanaalonderwerp bijwerken mislukt (heeft de bot MANAGE_CHANNELS?): {}", e);
                                }
                            });
                        }
                    }
                    continue;
                }
                if window_start.elapsed() > Duration::from_secs(10) {
                    window_start = std::time::Instant::now();
                    sent_in_window = 0;
                }
                if sent_in_window >= 10 {
                    continue;
                }
                let Some((chan, text)) = router.format_presence_for_discord(&ev) else { continue };
                // Zonder bekend kanaal (QUIT) is er niets om naartoe te sturen
                let Some(chan) = chan else { continue };
                if let Some(m) = channels.iter().find(|m| m.irc_channel.eq_ignore_ascii_case(&chan)) {
                    sent_in_window += 1;
                    let _ = dispatcher.send_message(&m.discord_webhook_url, "IRC", &text).await;
                }
            }
        });
    }

    // 13. Achtergrondtaak: Periodieke RSS Feeds Monitor (elke 10 minuten)
    let rss_pool = pool.clone();
    let rss_irc_tx = outbound_irc_tx.clone();
    let rss_router = bridge_router.clone();
    let rss_dispatcher = webhook_dispatcher.clone();
    let rss_shutdown = shutdown_token.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(600));
        // Eerste tick overslaan bij opstarten
        interval.tick().await;

        loop {
            tokio::select! {
                _ = rss_shutdown.cancelled() => {
                    info!("RSS achtergrondtaak afgesloten.");
                    break;
                }
                _ = interval.tick() => {
                    let articles = RssPlugin::poll_new_articles(&rss_pool).await;
                    for (channel, message) in articles {
                        info!("📰 Nieuw RSS-artikel versturen naar kanaal {}", channel);

                        // 1. Verstuur naar IRC
                        let _ = rss_irc_tx.send(BridgeMessage {
                            source_platform: Platform::Irc,
                            source_channel: channel.clone(),
                            author_name: "IRCord".into(),
                            author_id: None,
                            message_id: None,
                            content: message.clone(),
                            reply_to: None,
                            is_action: false,
                        }).await;

                        // 2. Verstuur naar Discord indien kanaal gekoppeld is
                        if let Some(mapping) = rss_router.get_discord_destination(&channel) {
                            let _ = rss_dispatcher.send_message(
                                &mapping.discord_webhook_url,
                                "IRCord",
                                &message,
                            ).await;
                        }
                    }
                }
            }
        }
    });

    // 13b. Achtergrondtaak: dagelijkse database-back-up
    if cfg.general.backup_enabled {
        let pool = pool.clone();
        let dir = std::path::PathBuf::from(&cfg.general.backup_dir);
        let keep = cfg.general.backup_keep;
        let shutdown = shutdown_token.clone();
        tokio::spawn(async move {
            // Eerste back-up 5 minuten na het opstarten, daarna elke 24 uur
            let mut wait = Duration::from_secs(300);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(wait) => {
                        match crate::plugins::backup::backup_database(&pool, &dir, keep).await {
                            Ok(p) => info!("💾 Databaseback-up gemaakt: {}", p.display()),
                            Err(e) => warn!("Databaseback-up mislukt: {}", e),
                        }
                        wait = Duration::from_secs(24 * 3600);
                    }
                }
            }
        });
    }

    // 14. Achtergrondtaak: Geautomatiseerde herinneringen (!remind / !remindme)
    let remind_pool = pool.clone();
    let remind_irc_tx = outbound_irc_tx.clone();
    let remind_router = bridge_router.clone();
    let remind_dispatcher = webhook_dispatcher.clone();
    let remind_locale = locale_manager.clone();
    let remind_shutdown = shutdown_token.clone();

    tokio::spawn(async move {
        // Controleer elke 5 seconden op actieve herinneringen
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = remind_shutdown.cancelled() => {
                    info!("Herinneringen achtergrondtaak afgesloten.");
                    break;
                }
                _ = interval.tick() => {
                    match RemindPlugin::check_and_trigger_reminders(&remind_pool, &remind_locale).await {
                        Ok(reminders) => {
                            for (channel, platform, author, message) in reminders {
                                info!("⏰ Herinnering afleveren voor {} in {} ({})", author, channel, platform);

                                if platform.eq_ignore_ascii_case("irc") {
                                    // Alleen naar het specifieke IRC-kanaal sturen waar het gevraagd is
                                    let _ = remind_irc_tx.send(BridgeMessage {
                                        source_platform: Platform::Irc,
                                        source_channel: channel.clone(),
                                        author_name: "IRCord".into(),
                                        author_id: None,
                                        message_id: None,
                                        content: message.clone(),
                                        reply_to: None,
                                        is_action: false,
                                    }).await;
                                } else if platform.eq_ignore_ascii_case("discord") {
                                    // Alleen naar Discord sturen waar het gevraagd is
                                    let webhook_opt = if let Ok(discord_chan_id) = channel.parse::<u64>() {
                                        remind_router.get_irc_destination(discord_chan_id).map(|m| m.discord_webhook_url.clone())
                                    } else {
                                        remind_router.get_discord_destination(&channel).map(|m| m.discord_webhook_url.clone())
                                    };

                                    if let Some(webhook_url) = webhook_opt {
                                        let _ = remind_dispatcher.send_message(
                                            &webhook_url,
                                            "IRCord",
                                            &message,
                                        ).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Fout bij controleren van herinneringen: {:?}", e);
                        }
                    }
                }
            }
        }
    });

    info!("IRCord Daemon succesvol gestart en operationeel!");

    // 13. Wacht op shutdown signaal (Ctrl+C of SIGTERM)
    shutdown.wait_for_signal().await;
    info!("Afsluitsignaal ontvangen. Nette shutdown in gang gezet...");

    // Geef lopende taken kort de tijd om netjes af te sluiten
    tokio::time::sleep(Duration::from_millis(500)).await;

    info!("Sluiten van database pool...");
    pool.close().await;
    info!("IRCord Daemon netjes afgesloten. Tot ziens!");

    Ok(())
}
