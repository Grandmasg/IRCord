use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use sqlx::Row;

/// `!peak`: drukste dagen van het kanaal, afgeleid uit het opgeslagen chatlogboek.
pub struct StatsPlugin;

#[async_trait]
impl Plugin for StatsPlugin {
    fn name(&self) -> &'static str { "stats" }
    fn triggers(&self) -> &[&'static str] { &["peak", "drukte"] }
    fn help(&self) -> &'static str { "!peak - Drukste dag in dit kanaal (meeste unieke chatters en meeste berichten)" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let by_users = sqlx::query(
            "SELECT substr(timestamp, 1, 10) AS day, COUNT(DISTINCT lower(author)) AS users, COUNT(*) AS msgs \
             FROM chat_history WHERE channel = ? GROUP BY day ORDER BY users DESC, msgs DESC LIMIT 1",
        )
        .bind(&cmd.channel)
        .fetch_optional(&ctx.db)
        .await?;

        let Some(row) = by_users else {
            return Ok(Some("📈 [Peak] Nog geen chatgeschiedenis voor dit kanaal.".into()));
        };
        let day: String = row.try_get("day").unwrap_or_default();
        let users: i64 = row.try_get("users").unwrap_or(0);
        let msgs: i64 = row.try_get("msgs").unwrap_or(0);

        let by_msgs = sqlx::query(
            "SELECT substr(timestamp, 1, 10) AS day, COUNT(*) AS msgs \
             FROM chat_history WHERE channel = ? GROUP BY day ORDER BY msgs DESC LIMIT 1",
        )
        .bind(&cmd.channel)
        .fetch_optional(&ctx.db)
        .await?;

        let mut out = format!("📈 [Peak] Meeste unieke chatters: \x02{}\x02 op {} ({} berichten)", users, day, msgs);
        if let Some(r) = by_msgs {
            let d: String = r.try_get("day").unwrap_or_default();
            let n: i64 = r.try_get("msgs").unwrap_or(0);
            out.push_str(&format!(" | Meeste berichten: \x02{}\x02 op {}", n, d));
        }
        Ok(Some(out))
    }
}
