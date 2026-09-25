use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct PresencePlugin;

#[async_trait]
impl Plugin for PresencePlugin {
    fn name(&self) -> &'static str { "presence" }
    fn triggers(&self) -> &[&'static str] { &["seen", "lastonline", "online"] }
    fn help(&self) -> &'static str { "!seen <nick> / !lastonline <nick> - Toont activiteit | !online - Toont actieve chatters" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let target = cmd.args.trim();

        if cmd.trigger == "online" {
            let active_users: Vec<(String, String, Option<String>)> = sqlx::query_as(
                r#"
                SELECT nick, platform, last_event
                FROM presence
                ORDER BY last_seen_at DESC
                LIMIT 10
                "#,
            )
            .fetch_all(&ctx.db)
            .await?;

            if active_users.is_empty() {
                return Ok(Some("Geen actieve gebruikers geregistreerd in de database.".into()));
            }

            let mut irc_users = Vec::new();
            let mut discord_users = Vec::new();

            for (nick, platform, _last_event) in active_users {
                if platform == "irc" {
                    irc_users.push(nick);
                } else {
                    discord_users.push(nick);
                }
            }

            return Ok(Some(format!(
                "👥 [Online Overzicht] IRC: {} | Discord: {}",
                if irc_users.is_empty() { "geen".to_string() } else { irc_users.join(", ") },
                if discord_users.is_empty() { "geen".to_string() } else { discord_users.join(", ") }
            )));
        }

        // !seen of !lastonline
        let target = cmd.args.trim();
        if target.is_empty() {
            return Ok(Some("Gebruik: !seen <bijnaam>".into()));
        }

        let record: Option<(String, String, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT nick, platform, strftime('%Y-%m-%d %H:%M:%S', last_seen_at), strftime('%Y-%m-%d %H:%M:%S', last_spoke_at), last_event, quit_message
            FROM presence
            WHERE LOWER(nick) = LOWER(?)
            ORDER BY last_seen_at DESC
            LIMIT 1
            "#,
        )
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;

        if let Some((nick, platform, last_seen_at, _last_spoke_at, last_event, quit_message)) = record {
            let event_info = match last_event.as_deref() {
                Some("quit") => format!("(Quit: {})", quit_message.unwrap_or_else(|| "geen reden".into())),
                Some("part") => "(heeft het kanaal verlaten)".into(),
                _ => "(laatst gesproken)".into(),
            };

            Ok(Some(format!(
                "👀 [{}] {} was laatst gezien op {} om {} {}",
                nick, platform.to_uppercase(), platform, last_seen_at.unwrap_or_else(|| "onbekend".into()), event_info
            )))
        } else {
            Ok(Some(format!("Ik heb '{}' nog niet eerder gezien in het kanaal.", target)))
        }
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let now = chrono::Utc::now().to_rfc3339();

        // Werk last_spoke_at en last_seen_at bij
        let _ = sqlx::query(
            r#"
            INSERT INTO presence (nick, platform, last_seen_at, last_spoke_at, last_event)
            VALUES (?, ?, ?, ?, 'msg')
            ON CONFLICT(nick, platform) DO UPDATE SET
                last_seen_at = excluded.last_seen_at,
                last_spoke_at = excluded.last_spoke_at,
                last_event = 'msg'
            "#,
        )
        .bind(&msg.author)
        .bind(&msg.platform)
        .bind(&now)
        .bind(&now)
        .execute(&ctx.db)
        .await;

        Ok(None)
    }
}
