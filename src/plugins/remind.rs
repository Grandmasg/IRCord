use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::i18n::LocaleManager;
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::SqlitePool;

pub struct RemindPlugin;

impl RemindPlugin {
    /// Achtergrondcontrole: zoekt reminders die nu getriggerd moeten worden
    pub async fn check_and_trigger_reminders(
        pool: &SqlitePool,
        locale: &LocaleManager,
    ) -> Result<Vec<(String, String, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now().timestamp();
        let rows: Vec<(i64, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT id, author, channel, platform, message
            FROM reminders
            WHERE trigger_at <= ? AND delivered_at IS NULL
            ORDER BY id ASC
            LIMIT 25
            "#,
        )
        .bind(now)
        .fetch_all(pool)
        .await?;

        let mut triggers = Vec::new();

        for (id, author, channel, platform, message) in rows {
            let _ = sqlx::query("UPDATE reminders SET delivered_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await;

            let formatted = locale.tf(
                "remind_triggered",
                &[("author", &author), ("message", &message)],
            );
            triggers.push((channel, platform, author, formatted));
        }

        Ok(triggers)
    }
}

#[async_trait]
impl Plugin for RemindPlugin {
    fn name(&self) -> &'static str { "remind" }
    fn triggers(&self) -> &[&'static str] { &["remindme", "remind"] }
    fn help(&self) -> &'static str { "!remindme <getal><m/h/d> <bericht> - Stelt een actieve herinnering in (bijv. !remindme 30m pizza)" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, ' ');
        let time_str = parts.next().unwrap_or("");
        let message = parts.next().unwrap_or("").trim();

        if time_str.is_empty() || message.is_empty() {
            return Ok(Some(ctx.locale.t("remind_usage").into()));
        }

        let unit = time_str.chars().last().unwrap_or('m');
        let num_str = &time_str[..time_str.len().saturating_sub(1)];
        let count: i64 = match num_str.parse() {
            Ok(n) if n > 0 => n,
            _ => return Ok(Some(ctx.locale.t("remind_invalid_time").into())),
        };

        let duration = match unit {
            'm' | 'M' => ChronoDuration::minutes(count),
            'h' | 'H' => ChronoDuration::hours(count),
            'd' | 'D' => ChronoDuration::days(count),
            _ => ChronoDuration::minutes(count),
        };

        let trigger_at = Utc::now() + duration;
        let trigger_at_epoch = trigger_at.timestamp();

        sqlx::query(
            r#"
            INSERT INTO reminders (author, channel, platform, message, trigger_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&cmd.author)
        .bind(&cmd.channel)
        .bind(&cmd.platform)
        .bind(message)
        .bind(trigger_at_epoch)
        .execute(&ctx.db)
        .await?;

        let unit_str = if unit == 'h' {
            ctx.locale.t("remind_unit_hour")
        } else if unit == 'd' {
            ctx.locale.t("remind_unit_day")
        } else {
            ctx.locale.t("remind_unit_minute")
        };
        let count_str = count.to_string();
        let confirmation = ctx.locale.tf(
            "remind_saved",
            &[
                ("author", &cmd.author),
                ("message", message),
                ("count", &count_str),
                ("unit", unit_str),
            ],
        );

        Ok(Some(format!("⏰ {}", confirmation)))
    }
}
