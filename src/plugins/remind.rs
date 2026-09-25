use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};

pub struct RemindPlugin;

#[async_trait]
impl Plugin for RemindPlugin {
    fn name(&self) -> &'static str { "remind" }
    fn triggers(&self) -> &[&'static str] { &["remindme", "remind"] }
    fn help(&self) -> &'static str { "!remindme <getal><m/h/d> <bericht> - Stelt een herinnering in (bijv. !remindme 30m pizza)" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, ' ');
        let time_str = parts.next().unwrap_or("");
        let message = parts.next().unwrap_or("").trim();

        if time_str.is_empty() || message.is_empty() {
            return Ok(Some("Gebruik: !remindme <getal><m/h/d> <bericht> (bijvoorbeeld: !remindme 20m pizza uit de oven)".into()));
        }

        let unit = time_str.chars().last().unwrap_or('m');
        let num_str = &time_str[..time_str.len().saturating_sub(1)];
        let count: i64 = match num_str.parse() {
            Ok(n) if n > 0 => n,
            _ => return Ok(Some("Ongeldige tijdsaanduiding. Gebruik bijv. 10m, 2h of 1d.".into())),
        };

        let duration = match unit {
            'm' | 'M' => ChronoDuration::minutes(count),
            'h' | 'H' => ChronoDuration::hours(count),
            'd' | 'D' => ChronoDuration::days(count),
            _ => ChronoDuration::minutes(count),
        };

        let trigger_at = Utc::now() + duration;
        let formatted_time = trigger_at.format("%H:%M:%S UTC").to_string();

        // Sla memo op met geplande tijd in details
        let reminder_note = format!("[HERINNERING om {}]: {}", formatted_time, message);
        sqlx::query!(
            r#"
            INSERT INTO memos (recipient, sender, platform, message)
            VALUES (?, ?, ?, ?)
            "#,
            cmd.author,
            "Herinnering",
            cmd.platform,
            reminder_note
        )
        .execute(&ctx.db)
        .await?;

        Ok(Some(format!(
            "⏰ {}, je herinnering voor '{}' staat genoteerd (over {} {})!",
            cmd.author, message, count, if unit == 'h' { "uur" } else if unit == 'd' { "dag(en)" } else { "minuten" }
        )))
    }
}
