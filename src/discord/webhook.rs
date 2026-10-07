use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, warn};

#[derive(Serialize)]
struct WebhookPayload<'a> {
    username: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<&'a str>,
}

/// Een verzoek van een plugin om iets naar het gekoppelde Discord-kanaal te sturen.
#[derive(Debug, Clone)]
pub struct DiscordPost {
    /// IRC-kanaal waarvan we de gekoppelde Discord-webhook opzoeken
    pub irc_channel: String,
    pub username: String,
    pub content: String,
    /// Bestand om als bijlage te uploaden (bestandsnaam, inhoud)
    pub file: Option<(String, Vec<u8>)>,
    /// Afbeeldings-URL om als embed te tonen
    pub image_url: Option<String>,
}

pub struct WebhookDispatcher {
    http: Client,
}

impl WebhookDispatcher {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    /// Verstuurt een chatbericht naar een Discord Webhook met optionele avatar
    pub async fn send_message(&self, webhook_url: &str, username: &str, content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.send_message_with_avatar(webhook_url, username, content, None).await
    }

    /// Verstuurt een bericht met een afbeelding-embed (Discord haalt de afbeelding zelf op).
    pub async fn send_image_embed(&self, webhook_url: &str, username: &str, content: &str, image_url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "username": username,
            "content": content,
            "embeds": [{ "image": { "url": image_url } }],
        });
        let resp = self.http.post(webhook_url).json(&payload).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("Webhook fout {}", resp.status()).into())
        }
    }

    /// Uploadt een bestand als bijlage via de webhook (multipart), met retry bij 429.
    pub async fn send_file(&self, webhook_url: &str, username: &str, content: &str, filename: &str, bytes: Vec<u8>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for attempt in 0..3u32 {
            let payload = serde_json::json!({ "username": username, "content": content }).to_string();
            let form = reqwest::multipart::Form::new()
                .text("payload_json", payload)
                .part("files[0]", reqwest::multipart::Part::bytes(bytes.clone()).file_name(filename.to_string()));
            let resp = self.http.post(webhook_url).multipart(form).send().await?;
            if resp.status().is_success() {
                return Ok(());
            }
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = resp.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0 * 2f64.powi(attempt as i32));
                sleep(Duration::from_millis((wait * 1000.0) as u64)).await;
                continue;
            }
            return Err(format!("Webhook bestandsupload fout {}", resp.status()).into());
        }
        Err("Discord bestandsupload mislukt na 3 pogingen".into())
    }

    /// Verstuurt een chatbericht naar een Discord Webhook met automatische 429 backoff retry en optionele profielfoto
    pub async fn send_message_with_avatar(
        &self,
        webhook_url: &str,
        username: &str,
        content: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = WebhookPayload { username, content, avatar_url };
        let max_retries = 3;

        for attempt in 0..max_retries {
            let resp = self.http.post(webhook_url)
                .json(&payload)
                .send()
                .await?;

            if resp.status().is_success() {
                return Ok(());
            }

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                // Lees optionele retry-after header uit
                let wait_ms = resp.headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|secs| (secs * 1000.0) as u64)
                    .unwrap_or_else(|| 1000 * 2u64.pow(attempt as u32));

                warn!("Discord Webhook 429 Rate Limit! Wachten voor {} ms...", wait_ms);
                sleep(Duration::from_millis(wait_ms)).await;
                continue;
            }

            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("Fout bij versturen van Discord webhook ({}): {}", status, body);
            return Err(format!("Webhook fout {}: {}", status, body).into());
        }

        Err("Discord webhook verzending mislukt na 3 pogingen".into())
    }
}

use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AvatarResolver {
    db: SqlitePool,
    discord_token: String,
    owner_nick: String,
    owner_discord_id: u64,
    cache: Arc<RwLock<HashMap<String, Option<String>>>>,
    http: Client,
}

impl AvatarResolver {
    pub fn new(
        db: SqlitePool,
        discord_token: String,
        owner_nick: String,
        owner_discord_id: u64,
    ) -> Self {
        Self {
            db,
            discord_token,
            owner_nick,
            owner_discord_id,
            cache: Arc::new(RwLock::new(HashMap::new())),
            http: Client::new(),
        }
    }

    pub async fn resolve_avatar(&self, irc_nick: &str) -> Option<String> {
        let key = irc_nick.to_lowercase();
        {
            let cache_read = self.cache.read().await;
            if let Some(cached) = cache_read.get(&key) {
                return cached.clone();
            }
        }

        // 1. Zoek discord_id (owner of gekoppeld via account_links)
        let discord_id = if irc_nick.eq_ignore_ascii_case(&self.owner_nick) && self.owner_discord_id > 0 {
            Some(self.owner_discord_id.to_string())
        } else {
            let row = sqlx::query("SELECT discord_id FROM account_links WHERE irc_nick = ? COLLATE NOCASE LIMIT 1")
                .bind(irc_nick)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten();

            row.and_then(|r| r.try_get::<String, _>("discord_id").ok())
        };

        let mut avatar_url = None;

        // 2. Haal avatar hash op via Discord API
        if let Some(id) = discord_id {
            if !self.discord_token.is_empty() {
                let url = format!("https://discord.com/api/v10/users/{}", id);
                if let Ok(resp) = self.http.get(&url)
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("User-Agent", "IRCord/1.0")
                    .send()
                    .await
                {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(hash) = json.get("avatar").and_then(|a| a.as_str()) {
                            let ext = if hash.starts_with("a_") { "gif" } else { "png" };
                            avatar_url = Some(format!("https://cdn.discordapp.com/avatars/{}/{}.{}", id, hash, ext));
                        }
                    }
                }
            }
        }

        // 3. Sla op in cache
        let mut cache_write = self.cache.write().await;
        cache_write.insert(key, avatar_url.clone());
        avatar_url
    }
}
