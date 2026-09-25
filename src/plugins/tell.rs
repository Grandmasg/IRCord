use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct TellPlugin;

#[async_trait]
impl Plugin for TellPlugin {
    fn name(&self) -> &'static str {
        "tell"
    }

    fn triggers(&self) -> &[&'static str] {
        &["tell", "memo", "note"]
    }

    fn help(&self) -> &'static str {
        "!tell <gebruiker> <bericht> - Laat een offline memo achter voor iemand"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = cmd.args.trim().splitn(2, ' ').collect();
        if parts.len() < 2 || parts[0].is_empty() || parts[1].trim().is_empty() {
            return Ok(Some(format!("ℹ️ {}", ctx.locale.t("tell_usage"))));
        }

        let recipient = parts[0].trim();
        let message = parts[1].trim();

        if recipient.eq_ignore_ascii_case(&cmd.author) {
            return Ok(Some(format!("⚠️ {}", ctx.locale.t("tell_self"))));
        }

        sqlx::query(
            "INSERT INTO memos (recipient, sender, platform, message) VALUES (?, ?, ?, ?)",
        )
        .bind(recipient)
        .bind(&cmd.author)
        .bind(&cmd.platform)
        .bind(message)
        .execute(&ctx.db)
        .await?;

        Ok(Some(format!(
            "📝 {}",
            ctx.locale.tf("tell_saved", &[("recipient", recipient)])
        )))
    }

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Check for pending memos for this recipient
        let rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, sender, message, created_at FROM memos WHERE LOWER(recipient) = LOWER(?) AND delivered_at IS NULL ORDER BY id ASC LIMIT 5",
        )
        .bind(&msg.author)
        .fetch_all(&ctx.db)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let mut ids = Vec::new();
        let mut notes = Vec::new();

        for (id, sender, memo_text, created_at) in rows {
            ids.push(id);
            let time_str = created_at.unwrap_or_else(|| {
                if ctx.locale.is_dutch() { "onlangs" } else { "recently" }.to_string()
            });
            notes.push(format!(
                "📬 {}",
                ctx.locale.tf("tell_received_item", &[("sender", &sender), ("time", &time_str), ("message", &memo_text)])
            ));
        }

        // Mark as delivered
        for id in ids {
            let _ = sqlx::query("UPDATE memos SET delivered_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind(id)
                .execute(&ctx.db)
                .await;
        }

        let header = ctx.locale.tf("tell_received_header", &[("author", &msg.author)]);
        Ok(Some(format!("{}\n{}", header, notes.join("\n"))))
    }
}
