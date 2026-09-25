use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
struct PendingLink {
    _otp: String,
    initiator_platform: String,
    initiator_id: String,
    initiator_tag: String,
    _target_nick_or_tag: String,
    expires_at: Instant,
}

pub struct IdentityPlugin {
    pending_links: Arc<RwLock<HashMap<String, PendingLink>>>,
}

impl IdentityPlugin {
    pub fn new() -> Self {
        Self {
            pending_links: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn generate_otp() -> String {
        use std::time::SystemTime;
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let code = 100000 + (nanos % 900000);
        code.to_string()
    }
}

#[async_trait]
impl Plugin for IdentityPlugin {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn triggers(&self) -> &[&'static str] {
        &["link", "whois", "bridge", "top"]
    }

    fn help(&self) -> &'static str {
        "!link <target> - Link IRC nick to Discord account | !link verify <code> | !whois <nick> | !bridge stats"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trigger = cmd.trigger.as_str();

        match trigger {
            "link" => self.handle_link(ctx, cmd).await,
            "whois" => self.handle_whois(ctx, cmd).await,
            "bridge" | "top" => self.handle_bridge_stats(ctx, cmd).await,
            _ => Ok(None),
        }
    }
}

impl IdentityPlugin {
    async fn handle_link(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = cmd.args.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(Some(
                "🔗 [Identity Link] Usage:\n\
                • '!link <discord_tag>' (from IRC) or '!link <irc_nick>' (from Discord) - Request 6-digit OTP\n\
                • '!link verify <code>' - Confirm and link your accounts\n\
                • '!link unlink' - Remove your linked identity"
                    .into(),
            ));
        }

        let action = parts[0].to_lowercase();

        if action == "unlink" {
            let rows_affected = if cmd.platform == "irc" {
                sqlx::query("DELETE FROM account_links WHERE irc_nick = ? COLLATE NOCASE")
                    .bind(&cmd.author)
                    .execute(&ctx.db)
                    .await?
                    .rows_affected()
            } else {
                sqlx::query("DELETE FROM account_links WHERE discord_tag = ? COLLATE NOCASE OR discord_id = ?")
                    .bind(&cmd.author)
                    .bind(&cmd.author)
                    .execute(&ctx.db)
                    .await?
                    .rows_affected()
            };

            if rows_affected > 0 {
                return Ok(Some("🔗 [Identity] Account link successfully removed.".into()));
            } else {
                return Ok(Some("ℹ️ [Identity] No linked account found for your profile.".into()));
            }
        }

        if action == "verify" {
            if parts.len() < 2 {
                return Ok(Some("⚠️ Usage: !link verify <6-digit-code>".into()));
            }
            let code = parts[1].trim();

            let mut pending = self.pending_links.write().await;
            pending.retain(|_, v| v.expires_at > Instant::now());

            if let Some(link) = pending.remove(code) {
                if link.initiator_platform == cmd.platform {
                    return Ok(Some(format!(
                        "⚠️ Please verify this code on the OTHER platform ({})!",
                        if cmd.platform == "irc" { "Discord" } else { "IRC" }
                    )));
                }

                let (discord_id, discord_tag, irc_nick) = if link.initiator_platform == "irc" {
                    (cmd.author.clone(), cmd.author.clone(), link.initiator_tag)
                } else {
                    (link.initiator_id, link.initiator_tag, cmd.author.clone())
                };

                sqlx::query(
                    r#"
                    INSERT INTO account_links (discord_id, discord_tag, irc_nick)
                    VALUES (?, ?, ?)
                    ON CONFLICT(discord_id) DO UPDATE SET irc_nick = excluded.irc_nick, discord_tag = excluded.discord_tag
                    "#
                )
                .bind(&discord_id)
                .bind(&discord_tag)
                .bind(&irc_nick)
                .execute(&ctx.db)
                .await?;

                return Ok(Some(format!(
                    "✅ [Identity Link] Success! IRC nick \x02{}\x02 is now linked to Discord user \x02@{}\x02.",
                    irc_nick, discord_tag
                )));
            } else {
                return Ok(Some("❌ Invalid or expired OTP code. Please request a new one with '!link <target>'.".into()));
            }
        }

        let target = parts[0].trim_start_matches('@');
        if target.is_empty() {
            return Ok(Some("⚠️ Please provide the username/nick on the other platform to link with.".into()));
        }

        let otp = Self::generate_otp();
        let target_platform = if cmd.platform == "irc" { "Discord" } else { "IRC" };

        let entry = PendingLink {
            _otp: otp.clone(),
            initiator_platform: cmd.platform.clone(),
            initiator_id: cmd.author.clone(),
            initiator_tag: cmd.author.clone(),
            _target_nick_or_tag: target.to_string(),
            expires_at: Instant::now() + Duration::from_secs(600),
        };

        {
            let mut pending = self.pending_links.write().await;
            pending.retain(|_, v| v.expires_at > Instant::now());
            pending.insert(otp.clone(), entry);
        }

        Ok(Some(format!(
            "🔑 [Identity Link] OTP Code: \x02{}\x02\n\
            To complete the link, type \x02!link verify {}\x02 on \x02{}\x02 within 10 minutes.",
            otp, otp, target_platform
        )))
    }

    async fn handle_whois(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let query = if cmd.args.trim().is_empty() {
            cmd.author.as_str()
        } else {
            cmd.args.trim().trim_start_matches('@')
        };

        // 1. Query account links
        let link_row = sqlx::query(
            r#"
            SELECT discord_id, discord_tag, irc_nick, created_at
            FROM account_links
            WHERE irc_nick = ? COLLATE NOCASE OR discord_tag = ? COLLATE NOCASE OR discord_id = ?
            LIMIT 1
            "#
        )
        .bind(query)
        .bind(query)
        .bind(query)
        .fetch_optional(&ctx.db)
        .await?;

        // 2. Query Karma
        let karma_row = sqlx::query(
            r#"
            SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
            FROM audit_log
            WHERE details = ? COLLATE NOCASE
            "#
        )
        .bind(query)
        .fetch_one(&ctx.db)
        .await?;
        let karma_score: i64 = karma_row.try_get("total").unwrap_or(0);

        // 3. Query Birthday
        let bday_row = sqlx::query(
            r#"
            SELECT day, month, year
            FROM birthdays
            WHERE user_id = ? COLLATE NOCASE OR display_name = ? COLLATE NOCASE
            LIMIT 1
            "#
        )
        .bind(query)
        .bind(query)
        .fetch_optional(&ctx.db)
        .await?;

        let bday_str = if let Some(b) = bday_row {
            let day: i64 = b.try_get("day").unwrap_or(1);
            let month: i64 = b.try_get("month").unwrap_or(1);
            let year: Option<i64> = b.try_get("year").ok();
            if let Some(y) = year {
                format!("{:02}-{:02}-{}", day, month, y)
            } else {
                format!("{:02}-{:02}", day, month)
            }
        } else {
            "Not registered".to_string()
        };

        // 4. Query total messages sent in chat_history
        let msg_count_row = sqlx::query(
            "SELECT COUNT(*) as count FROM chat_history WHERE author = ? COLLATE NOCASE"
        )
        .bind(query)
        .fetch_one(&ctx.db)
        .await?;
        let msg_count: i64 = msg_count_row.try_get("count").unwrap_or(0);

        if let Some(link) = link_row {
            let irc_nick: String = link.try_get("irc_nick").unwrap_or_default();
            let discord_tag: String = link.try_get("discord_tag").unwrap_or_default();
            let discord_id: String = link.try_get("discord_id").unwrap_or_default();
            let created: String = link.try_get::<String, _>("created_at")
                .map(|t| t.chars().take(10).collect())
                .unwrap_or_else(|_| "recent".into());

            Ok(Some(format!(
                "👤 [Whois] IRC: \x02{}\x02 ↔ Discord: \x02@{}\x02 (ID: {}) | Karma: \x02{}\x02 | Birthday: {} | Messages: {} | Linked: {}",
                irc_nick, discord_tag, discord_id, karma_score, bday_str, msg_count, created
            )))
        } else {
            Ok(Some(format!(
                "👤 [Whois] Target: \x02{}\x02 (Unlinked) | Karma: \x02{}\x02 | Birthday: {} | Messages: {}\n\
                💡 Tip: Use '!link <other_platform_user>' to link IRC and Discord identities.",
                query, karma_score, bday_str, msg_count
            )))
        }
    }

    async fn handle_bridge_stats(
        &self,
        ctx: &PluginContext,
        _cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let total_row = sqlx::query(
            r#"
            SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN platform = 'irc' THEN 1 ELSE 0 END) as irc_count,
                SUM(CASE WHEN platform = 'discord' THEN 1 ELSE 0 END) as discord_count
            FROM chat_history
            "#
        )
        .fetch_one(&ctx.db)
        .await?;

        let total: i64 = total_row.try_get("total").unwrap_or(0);
        let irc_cnt: i64 = total_row.try_get("irc_count").unwrap_or(0);
        let disc_cnt: i64 = total_row.try_get("discord_count").unwrap_or(0);

        if total == 0 {
            return Ok(Some("📊 [Bridge Stats] No messages recorded yet.".into()));
        }

        let irc_pct = (irc_cnt as f64 / total as f64) * 100.0;
        let disc_pct = (disc_cnt as f64 / total as f64) * 100.0;

        let top_rows = sqlx::query(
            r#"
            SELECT author, platform, COUNT(*) as count
            FROM chat_history
            GROUP BY author, platform
            ORDER BY count DESC
            LIMIT 5
            "#
        )
        .fetch_all(&ctx.db)
        .await?;

        let mut top_list = Vec::new();
        for (idx, row) in top_rows.iter().enumerate() {
            let author: String = row.try_get("author").unwrap_or_else(|_| "anon".into());
            let platform: String = row.try_get("platform").unwrap_or_else(|_| "chat".into());
            let count: i64 = row.try_get("count").unwrap_or(0);
            let plat_badge = if platform == "irc" { "IRC" } else { "Discord" };
            top_list.push(format!("{}. {} [{}]: {}", idx + 1, author, plat_badge, count));
        }

        let top_str = if top_list.is_empty() {
            "None".to_string()
        } else {
            top_list.join(" | ")
        };

        Ok(Some(format!(
            "📊 [Bridge Stats] Total: {} msgs | IRC: {} ({:.1}%) ↔ Discord: {} ({:.1}%)\n\
            🏆 Top Chatters: {}",
            total, irc_cnt, irc_pct, disc_cnt, disc_pct, top_str
        )))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_otp_generation() {
        for _ in 0..20 {
            let otp = IdentityPlugin::generate_otp();
            assert_eq!(otp.len(), 6);
            assert!(otp.chars().all(|c| c.is_ascii_digit()));
            let val: u32 = otp.parse().unwrap();
            assert!(val >= 100000 && val <= 999999);
        }
    }
}

