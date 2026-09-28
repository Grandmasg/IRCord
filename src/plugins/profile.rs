use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct ProfilePlugin;

#[async_trait]
impl Plugin for ProfilePlugin {
    fn name(&self) -> &'static str { "profile" }
    fn triggers(&self) -> &[&'static str] { &["profiel", "profile", "userinfo", "top"] }
    fn help(&self) -> &'static str { "!profiel [nick] - Toont het gebruikersprofiel | !top [karma|lines] - Toont ranglijst" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if cmd.trigger == "top" {
            let arg = cmd.args.trim().to_lowercase();
            if arg == "lines" || arg == "chat" || arg == "berichten" || arg == "regels" {
                // Top chatters uit chat_history
                let rows: Vec<(String, i64)> = sqlx::query_as(
                    r#"
                    SELECT author, COUNT(*) as count
                    FROM chat_history
                    WHERE author != 'IRCord'
                    GROUP BY LOWER(author)
                    ORDER BY count DESC
                    LIMIT 5
                    "#
                )
                .fetch_all(&ctx.db)
                .await?;

                if rows.is_empty() {
                    return Ok(Some("📊 Nog geen chatgeschiedenis beschikbaar voor statistieken.".into()));
                }

                let items: Vec<String> = rows.into_iter().enumerate().map(|(i, (nick, count))| {
                    format!("{}. \x02{}\x02 ({} regels)", i + 1, nick, count)
                }).collect();

                return Ok(Some(format!("📊 \x02[Top 5 Actiefste Chatters]\x02 {}", items.join(" | "))));
            } else {
                // Standaard: Top Karma
                let rows: Vec<(String, i64)> = sqlx::query_as(
                    r#"
                    SELECT details, SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
                    FROM audit_log
                    WHERE action IN ('karma_up', 'karma_down')
                    GROUP BY LOWER(details)
                    ORDER BY total DESC
                    LIMIT 5
                    "#
                )
                .fetch_all(&ctx.db)
                .await?;

                if rows.is_empty() {
                    return Ok(Some("🏆 Nog geen karma uitgedeeld in het kanaal. Gebruik 'nick++' om karma te geven!".into()));
                }

                let items: Vec<String> = rows.into_iter().enumerate().map(|(i, (nick, score))| {
                    format!("{}. \x02{}\x02 ({:+})", i + 1, nick, score)
                }).collect();

                return Ok(Some(format!("🏆 \x02[Top 5 Karma Ranglijst]\x02 {}", items.join(" | "))));
            }
        }

        // !profiel [target]
        let target = if cmd.args.trim().is_empty() {
            cmd.author.as_str()
        } else {
            cmd.args.trim()
        };

        // 1. Is target the bot owner?
        let is_owner = target.eq_ignore_ascii_case(&ctx.config.general.bot_owner_irc_nick);

        // 2. Karma
        let karma_row: Option<(Option<i64>,)> = sqlx::query_as(
            r#"
            SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
            FROM audit_log
            WHERE LOWER(details) = LOWER(?)
            "#
        )
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let karma = karma_row.and_then(|r| r.0).unwrap_or(0);

        // 3. WhatPulse
        let wp_row: Option<(String,)> = sqlx::query_as(
            "SELECT whatpulse_username FROM whatpulse_links WHERE LOWER(nick) = LOWER(?) LIMIT 1"
        )
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let wp_name = wp_row.map(|r| r.0);

        // 4. Birthday
        let bday_row: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
            "SELECT day, month, year FROM birthdays WHERE LOWER(user_id) = LOWER(?) OR LOWER(display_name) = LOWER(?) LIMIT 1"
        )
        .bind(target)
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let bday_str = bday_row.map(|(d, m, y)| {
            match y {
                Some(yr) => format!("{:02}-{:02}-{}", d, m, yr),
                None => format!("{:02}-{:02}", d, m),
            }
        });

        // 5. Account link (Discord handle)
        let link_row: Option<(String,)> = sqlx::query_as(
            "SELECT discord_tag FROM account_links WHERE LOWER(irc_nick) = LOWER(?) OR LOWER(discord_tag) = LOWER(?) LIMIT 1"
        )
        .bind(target)
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let discord_tag = link_row.map(|r| r.0);

        // 6. User preference language
        let lang_row: Option<(String,)> = sqlx::query_as(
            "SELECT preference_value FROM user_preferences WHERE LOWER(user_id) = LOWER(?) AND preference_key = 'language' LIMIT 1"
        )
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let user_lang = lang_row.map(|r| r.0.to_uppercase()).unwrap_or_else(|| ctx.config.general.language.to_uppercase());

        // 7. Line count in chat_history
        let line_count_row: Option<(i64,)> = sqlx::query_as(
            "SELECT COUNT(*) FROM chat_history WHERE LOWER(author) = LOWER(?)"
        )
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;
        let line_count = line_count_row.map(|r| r.0).unwrap_or(0);

        let mut parts = Vec::new();
        if is_owner {
            parts.push("👑 \x02Bot Owner\x02".to_string());
        }
        if let Some(tag) = discord_tag {
            parts.push(format!("💬 Discord: \x02{}\x02", tag));
        }
        parts.push(format!("⭐ Karma: \x02{:+}\x02", karma));
        parts.push(format!("📊 Berichten: \x02{}\x02 regels", line_count));

        if let Some(wp) = wp_name {
            parts.push(format!("⌨️ WhatPulse: \x02{}\x02", wp));
        }
        if let Some(bday) = bday_str {
            parts.push(format!("🎂 Verjaardag: \x02{}\x02", bday));
        }
        parts.push(format!("🗣️ Taal: \x02{}\x02", user_lang));

        Ok(Some(format!(
            "👤 \x02[Profiel: {}]\x02 {}",
            target,
            parts.join(" | ")
        )))
    }
}
