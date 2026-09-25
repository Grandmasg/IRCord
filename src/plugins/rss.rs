use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use reqwest::Client;

pub struct RssPlugin;

#[async_trait]
impl Plugin for RssPlugin {
    fn name(&self) -> &'static str {
        "rss"
    }

    fn triggers(&self) -> &[&'static str] {
        &["rss", "feed"]
    }

    fn help(&self) -> &'static str {
        "!rss add <url> [#channel] | !rss list | !rss del <id> | !rss latest <id>"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = cmd.args.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(Some(
                "📰 [RSS Feeds] Usage:\n\
                • '!rss add <url> [#channel]' - Subscribe to an RSS/Atom feed\n\
                • '!rss list' - List active feed subscriptions\n\
                • '!rss latest <id>' - Show the newest article from a feed\n\
                • '!rss del <id>' - Unsubscribe from a feed"
                    .into(),
            ));
        }

        let action = parts[0].to_lowercase();

        match action.as_str() {
            "add" => {
                if parts.len() < 2 {
                    return Ok(Some("⚠️ Usage: !rss add <url> [#channel]".into()));
                }
                let url = parts[1].trim();

                // Enforce SSRF safety check
                if !crate::plugins::url_titler::UrlTitlerPlugin::is_safe_public_url(url) {
                    return Ok(Some("⛔ [Security] Feed URL must be a safe public web address.".into()));
                }

                let target_channel = parts.get(2).map(|s| s.to_string()).unwrap_or_else(|| cmd.channel.clone());

                // Fetch and validate feed with feed-rs
                let resp = ctx.http.get(url).timeout(std::time::Duration::from_secs(8)).send().await;
                let bytes = match resp {
                    Ok(r) if r.status().is_success() => r.bytes().await?,
                    Ok(r) => return Ok(Some(format!("⚠️ Feed server returned status code: {}", r.status()))),
                    Err(e) => return Ok(Some(format!("❌ Failed to connect to feed URL: {}", e))),
                };

                let parsed = match feed_rs::parser::parse(&bytes[..]) {
                    Ok(f) => f,
                    Err(e) => return Ok(Some(format!("⚠️ Invalid RSS/Atom feed format: {}", e))),
                };

                let feed_title = parsed.title.map(|t| t.content).unwrap_or_else(|| "Untitled Feed".into());
                let first_guid = parsed.entries.first().map(|e| e.id.clone()).unwrap_or_default();

                // Store in database
                let row = sqlx::query(
                    r#"
                    INSERT INTO feeds (url, title, last_guid, last_checked_at)
                    VALUES (?, ?, ?, CURRENT_TIMESTAMP)
                    ON CONFLICT(url) DO UPDATE SET title = excluded.title
                    RETURNING id
                    "#
                )
                .bind(url)
                .bind(&feed_title)
                .bind(&first_guid)
                .fetch_one(&ctx.db)
                .await?;

                let feed_id: i64 = row.try_get("id").unwrap_or(0);

                sqlx::query(
                    r#"
                    INSERT INTO feed_subscriptions (feed_id, target_type, target_id, platform)
                    VALUES (?, 'channel', ?, ?)
                    "#
                )
                .bind(feed_id)
                .bind(&target_channel)
                .bind(&cmd.platform)
                .execute(&ctx.db)
                .await?;

                Ok(Some(format!(
                    "📰 [RSS] Successfully subscribed to \x02{}\x02 (ID: {}) for channel \x02{}\x02!",
                    feed_title, feed_id, target_channel
                )))
            }
            "list" => {
                let rows = sqlx::query(
                    r#"
                    SELECT f.id, f.title, f.url, s.target_id, s.platform
                    FROM feeds f
                    JOIN feed_subscriptions s ON f.id = s.feed_id
                    ORDER BY f.id ASC
                    "#
                )
                .fetch_all(&ctx.db)
                .await?;

                if rows.is_empty() {
                    return Ok(Some("📰 [RSS] No active feed subscriptions found.".into()));
                }

                let mut lines = Vec::new();
                lines.push("📰 [Active RSS Feeds]:".to_string());
                for r in rows {
                    let id: i64 = r.try_get("id").unwrap_or(0);
                    let title: String = r.try_get("title").unwrap_or_else(|_| "Untitled".into());
                    let target_id: String = r.try_get("target_id").unwrap_or_default();
                    let platform: String = r.try_get("platform").unwrap_or_default();
                    lines.push(format!("• [ID: {}] \x02{}\x02 -> {} ({})", id, title, target_id, platform));
                }

                Ok(Some(lines.join("\n")))
            }
            "del" | "delete" | "remove" => {
                if parts.len() < 2 {
                    return Ok(Some("⚠️ Usage: !rss del <feed_id>".into()));
                }
                let Ok(id) = parts[1].parse::<i64>() else {
                    return Ok(Some("⚠️ Invalid feed ID. Check !rss list for valid IDs.".into()));
                };

                let res = sqlx::query("DELETE FROM feeds WHERE id = ?")
                    .bind(id)
                    .execute(&ctx.db)
                    .await?;

                if res.rows_affected() > 0 {
                    Ok(Some(format!("🗑️ [RSS] Feed ID {} deleted successfully.", id)))
                } else {
                    Ok(Some(format!("⚠️ Feed ID {} not found.", id)))
                }
            }
            "latest" => {
                if parts.len() < 2 {
                    return Ok(Some("⚠️ Usage: !rss latest <feed_id>".into()));
                }
                let Ok(id) = parts[1].parse::<i64>() else {
                    return Ok(Some("⚠️ Invalid feed ID. Check !rss list for valid IDs.".into()));
                };

                let row = sqlx::query("SELECT url, title FROM feeds WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&ctx.db)
                    .await?;

                let Some(feed_record) = row else {
                    return Ok(Some(format!("⚠️ Feed ID {} not found.", id)));
                };

                let feed_url: String = feed_record.try_get("url").unwrap_or_default();
                let feed_title: String = feed_record.try_get("title").unwrap_or_else(|_| "RSS".into());

                let resp = ctx.http.get(&feed_url).timeout(std::time::Duration::from_secs(8)).send().await;
                let bytes = match resp {
                    Ok(r) if r.status().is_success() => r.bytes().await?,
                    Ok(r) => return Ok(Some(format!("⚠️ Feed server returned status code: {}", r.status()))),
                    Err(e) => return Ok(Some(format!("❌ Failed to connect to feed: {}", e))),
                };

                let parsed = feed_rs::parser::parse(&bytes[..])?;
                if let Some(entry) = parsed.entries.first() {
                    let entry_title = entry.title.as_ref().map(|t| t.content.clone()).unwrap_or_else(|| "Untitled".into());
                    let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();

                    Ok(Some(format!("📰 [{}] \x02{}\x02: {}", feed_title, entry_title, link)))
                } else {
                    Ok(Some("📰 Feed contains no entries.".into()))
                }
            }
            _ => Ok(Some("⚠️ Unknown RSS sub-command. Use !rss for help.".into())),
        }
    }
}

impl RssPlugin {
    /// Background poller to check feeds and return new items to broadcast
    pub async fn poll_new_articles(db: &SqlitePool, http: &Client) -> Vec<(String, String)> {
        let mut broadcasts = Vec::new();

        let feeds = match sqlx::query("SELECT id, url, title, last_guid FROM feeds").fetch_all(db).await {
            Ok(f) => f,
            Err(_) => return broadcasts,
        };

        for f in feeds {
            let id: i64 = f.try_get("id").unwrap_or(0);
            let url: String = f.try_get("url").unwrap_or_default();
            let title: String = f.try_get("title").unwrap_or_else(|_| "RSS".into());
            let last_guid: Option<String> = f.try_get("last_guid").ok();

            let resp = match http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            let bytes = match resp.bytes().await {
                Ok(b) => b,
                _ => continue,
            };

            let parsed = match feed_rs::parser::parse(&bytes[..]) {
                Ok(p) => p,
                _ => continue,
            };

            let Some(newest) = parsed.entries.first() else { continue };

            // Check if this article is new
            if last_guid.as_deref() != Some(&newest.id) {
                // Update last_guid in DB
                let _ = sqlx::query("UPDATE feeds SET last_guid = ?, last_checked_at = CURRENT_TIMESTAMP WHERE id = ?")
                    .bind(&newest.id)
                    .bind(id)
                    .execute(db)
                    .await;

                let entry_title = newest.title.as_ref().map(|t| t.content.clone()).unwrap_or_else(|| "New Article".into());
                let link = newest.links.first().map(|l| l.href.clone()).unwrap_or_default();

                let message = format!("📰 [{}] \x02{}\x02: {}", title, entry_title, link);

                // Find subscribed channels
                if let Ok(subs) = sqlx::query("SELECT target_id FROM feed_subscriptions WHERE feed_id = ?").bind(id).fetch_all(db).await {
                    for sub in subs {
                        let target_id: String = sub.try_get("target_id").unwrap_or_default();
                        broadcasts.push((target_id, message.clone()));
                    }
                }
            }
        }

        broadcasts
    }
}
