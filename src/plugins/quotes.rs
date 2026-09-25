use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct QuotesPlugin;

#[async_trait]
impl Plugin for QuotesPlugin {
    fn name(&self) -> &'static str { "quotes" }
    fn triggers(&self) -> &[&'static str] { &["quote", "q"] }
    fn help(&self) -> &'static str { "!quote add <tekst> | !quote [zoekterm] | !quote random" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, ' ');
        let first = parts.next().unwrap_or("").to_lowercase();
        let rest = parts.next().unwrap_or("").trim();

        if first == "add" {
            if rest.is_empty() {
                return Ok(Some(ctx.locale.t("quote_usage").into()));
            }

            // Save to audit_log
            sqlx::query!(
                r#"
                INSERT INTO audit_log (operator, platform, action, details)
                VALUES (?, ?, 'quote_add', ?)
                "#,
                cmd.author,
                cmd.platform,
                rest
            )
            .execute(&ctx.db)
            .await?;

            return Ok(Some(format!("📜 {}", ctx.locale.t("quote_saved"))));
        }

        // Search for a random quote or by keyword
        let row: Option<(i64, String, Option<String>)> = if first.is_empty() || first == "random" {
            sqlx::query_as(
                r#"
                SELECT id, operator, details
                FROM audit_log
                WHERE action = 'quote_add'
                ORDER BY RANDOM()
                LIMIT 1
                "#,
            )
            .fetch_optional(&ctx.db)
            .await?
        } else {
            let search = format!("%{}%", args);
            sqlx::query_as(
                r#"
                SELECT id, operator, details
                FROM audit_log
                WHERE action = 'quote_add' AND details LIKE ?
                ORDER BY RANDOM()
                LIMIT 1
                "#,
            )
            .bind(search)
            .fetch_optional(&ctx.db)
            .await?
        };

        if let Some((id, operator, details)) = row {
            let added_by = if ctx.locale.is_dutch() { "toegevoegd door" } else { "added by" };
            Ok(Some(format!(
                "💬 [Quote #{}] \"{}\" ({} {})",
                id, details.unwrap_or_default(), added_by, operator
            )))
        } else {
            Ok(Some(ctx.locale.t("quote_not_found").into()))
        }
    }
}
