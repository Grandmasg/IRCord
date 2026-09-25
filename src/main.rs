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
use discord::webhook::WebhookDispatcher;
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
    rhai::RhaiPlugin,
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

    // 3. Laad en valideer config.toml
    let config_path = "config.toml";
    let cfg = match Config::load_from_file(config_path) {
        Ok(c) => {
            info!("Configuratie succesvol geladen vanuit {}", config_path);
            c
        }
        Err(err) => {
            error!("Fout bij laden van configuratie: {}", err);
            return Err(err);
        }
    };

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

    // 6. Bouw Plugin Context en registreer alle plugins
    let cfg_arc = Arc::new(cfg.clone());
    let error_logger = Arc::new(ErrorLogger::new(100));
    let locale_manager = Arc::new(LocaleManager::load("locales", &cfg.general.language));
    let open_meteo_quota = Arc::new(crate::utils::quota::ApiQuotaGovernor::new(
        "Open-Meteo",
        cfg.open_meteo.minutely_limit,
        cfg.open_meteo.hourly_limit,
        cfg.open_meteo.daily_limit,
    ));

    let plugin_ctx = PluginContext {
        db: pool.clone(),
        http: http_client.clone(),
        ai_client: free_token_client.clone(),
        ai_manager: ai_manager.clone(),
        rag: rag_searcher.clone(),
        vision: vision_helper.clone(),
        config: cfg_arc.clone(),
        error_logger: error_logger.clone(),
        locale: locale_manager.clone(),
        open_meteo_quota: open_meteo_quota.clone(),
    };

    let mut plugin_mgr = PluginManager::new(plugin_ctx);
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
    plugin_mgr.register(Box::new(TranslatePlugin));
    plugin_mgr.register(Box::new(TimePlugin));
    plugin_mgr.register(Box::new(BirthdayPlugin));
    plugin_mgr.register(Box::new(IdentityPlugin::new()));
    plugin_mgr.register(Box::new(SysadminPlugin));
    plugin_mgr.register(Box::new(RssPlugin));
    plugin_mgr.register(Box::new(TechPlugin));
    plugin_mgr.register(Box::new(RhaiPlugin::new()));

    info!("Plugin Manager geïnitialiseerd met {} actieve plugins", plugin_mgr.plugin_count());
    let plugin_mgr = Arc::new(plugin_mgr);

    // 7. Initialiseer communicatiekanalen (mpsc channels)
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<BridgeMessage>(256);
    let (outbound_irc_tx, outbound_irc_rx) = mpsc::channel::<BridgeMessage>(256);

    let bridge_router = Arc::new(BridgeRouter::new(cfg.channels.clone(), cfg.bridge.lru_cache_capacity));
    let webhook_dispatcher = Arc::new(WebhookDispatcher::new());

    // 8. Initialiseer Graceful Shutdown Handler
    let shutdown = ShutdownManager::new();
    let shutdown_token = shutdown.child_token();

    // 9. Start Axum Web Server (Port 9090: /health, /metrics, /api/errors, /api/github)
    let web_cfg = cfg_arc.clone();
    let web_error_logger = error_logger.clone();
    let web_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        if let Err(err) = WebServer::start(web_cfg, web_error_logger, web_shutdown).await {
            error!("Fout bij draaien van Axum Web Server: {:?}", err);
        }
    });

    // 10. Start IRC Client Task
    let irc_client = IrcClient::new(
        cfg_arc.clone(),
        inbound_tx.clone(),
        outbound_irc_rx,
        shutdown_token.clone(),
    );
    tokio::spawn(async move {
        irc_client.run().await;
    });

    // 11. Start optionele Serenity Discord Gateway Client Task
    let discord_token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
    if !discord_token.is_empty() && discord_token != "YOUR_DISCORD_BOT_TOKEN_HERE" {
        info!("Discord client opstarten...");
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILDS;

        let handler = DiscordHandler::new(cfg_arc.clone(), inbound_tx.clone());
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
    let vision_clone = vision_helper.clone();
    let ai_client_clone = free_token_client.clone();
    let ai_manager_clone = ai_manager.clone();
    let shutdown_loop = shutdown_token.clone();

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

                                tokio::spawn(async move {
                                    if let Err(e) = disp.send_message(&url, &username, &formatted).await {
                                        warn!("Fout bij versturen naar Discord Webhook: {}", e);
                                    } else {
                                        // Bi-directionele cache koppeling registreren
                                        let fake_id = format!("{}:{}", chan, chrono::Utc::now().timestamp_millis());
                                        router.record_bridge_link(fake_id, chan, author, content);
                                    }
                                });
                            }
                        }
                        Platform::Discord => {
                            if let Ok(discord_chan_id) = msg.source_channel.parse::<u64>() {
                                if let Some(mapping) = router_clone.get_irc_destination(discord_chan_id) {
                                    let formatted = router_clone.format_for_irc(&msg);
                                    if let Some(ref d_id) = msg.author_id {
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

    // 13. Achtergrondtaak: Periodieke RSS Feeds Monitor (elke 10 minuten)
    let rss_pool = pool.clone();
    let rss_http = http_client.clone();
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
                    let articles = RssPlugin::poll_new_articles(&rss_pool, &rss_http).await;
                    for (channel, message) in articles {
                        info!("📰 Nieuw RSS-artikel versturen naar kanaal {}", channel);

                        // 1. Verstuur naar IRC
                        let _ = rss_irc_tx.send(BridgeMessage {
                            source_platform: Platform::Irc,
                            source_channel: channel.clone(),
                            author_name: "IRCord".into(),
                            author_id: None,
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
