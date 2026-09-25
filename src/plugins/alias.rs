use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct AliasPlugin;

#[async_trait]
impl Plugin for AliasPlugin {
    fn name(&self) -> &'static str { "alias" }
    fn triggers(&self) -> &[&'static str] { &["alias"] }
    fn help(&self) -> &'static str { "!alias add !naam <tekst> | !alias del !naam | !alias list" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut parts = cmd.args.trim().split_whitespace();
        let action = parts.next().unwrap_or("").to_lowercase();

        match action.as_str() {
            "add" => {
                let trigger = parts.next().unwrap_or("");
                let response: String = parts.collect::<Vec<&str>>().join(" ");

                if trigger.is_empty() || response.is_empty() {
                    return Ok(Some("Gebruik: !alias add !naam <tekst die de bot moet zeggen>".into()));
                }

                let clean_trigger = trigger.trim_start_matches('!').to_lowercase();

                sqlx::query!(
                    r#"
                    INSERT INTO aliases (trigger, response, creator, platform)
                    VALUES (?, ?, ?, ?)
                    ON CONFLICT(trigger) DO UPDATE SET response = excluded.response
                    "#,
                    clean_trigger,
                    response,
                    cmd.author,
                    cmd.platform
                )
                .execute(&ctx.db)
                .await?;

                Ok(Some(format!("✅ Alias '!{}' succesvol opgeslagen!", clean_trigger)))
            }
            "del" => {
                let trigger = parts.next().unwrap_or("").trim_start_matches('!').to_lowercase();
                if trigger.is_empty() {
                    return Ok(Some("Gebruik: !alias del !naam".into()));
                }

                let res = sqlx::query!("DELETE FROM aliases WHERE trigger = ?", trigger)
                    .execute(&ctx.db)
                    .await?;

                if res.rows_affected() > 0 {
                    Ok(Some(format!("🗑️ Alias '!{}' verwijderd.", trigger)))
                } else {
                    Ok(Some(format!("Alias '!{}' niet gevonden.", trigger)))
                }
            }
            "list" => {
                let rows = sqlx::query!("SELECT trigger FROM aliases LIMIT 20")
                    .fetch_all(&ctx.db)
                    .await?;

                if rows.is_empty() {
                    Ok(Some("Er zijn momenteel geen actieve aliassen gedefinieerd.".into()))
                } else {
                    let triggers: Vec<String> = rows.into_iter().map(|r| format!("!{}", r.trigger)).collect();
                    Ok(Some(format!("📋 Beschikbare aliassen: {}", triggers.join(", "))))
                }
            }
            _ => Ok(Some("Gebruik: !alias add !naam <tekst> | !alias del !naam | !alias list".into())),
        }
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trimmed = msg.content.trim();
        if trimmed.starts_with('!') {
            let trigger = trimmed[1..].split_whitespace().next().unwrap_or("").to_lowercase();
            let row = sqlx::query!("SELECT response FROM aliases WHERE trigger = ?", trigger)
                .fetch_optional(&ctx.db)
                .await?;

            if let Some(r) = row {
                return Ok(Some(r.response));
            }
        }
        Ok(None)
    }
}
