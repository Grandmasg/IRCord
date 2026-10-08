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
        let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT id, author, channel, platform, message, kind
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

        for (id, author, channel, platform, message, kind) in rows {
            let _ = sqlx::query("UPDATE reminders SET delivered_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await;

            let formatted = if kind == "timer" {
                if locale.is_dutch() {
                    format!("⏱️ Timer afgelopen voor \x02{}\x02: {}", author, message)
                } else {
                    format!("⏱️ Timer finished for \x02{}\x02: {}", author, message)
                }
            } else {
                locale.tf("remind_triggered", &[("author", &author), ("message", &message)])
            };
            triggers.push((channel, platform, author, formatted));
        }

        Ok(triggers)
    }
}

#[async_trait]
impl Plugin for RemindPlugin {
    fn name(&self) -> &'static str { "remind" }
    fn triggers(&self) -> &[&'static str] { &["remindme", "remind", "reminder"] }
    fn help(&self) -> &'static str { "!remind <getal><m/h/d> <bericht> - Stelt een actieve herinnering in (bijv. !remind 30m pizza)" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, ' ');
        let time_str = parts.next().unwrap_or("");
        let message = parts.next().unwrap_or("").trim();

        if time_str.is_empty() || message.is_empty() {
            return Ok(Some(ctx.locale.t("remind_usage").into()));
        }

        // Elke duur: 30m, 2h, 1h30m, 45s, 1d (een kaal getal telt als minuten)
        let Some(secs) = crate::utils::duration::parse_duration(time_str, 60).filter(|s| *s >= 10) else {
            return Ok(Some(ctx.locale.t("remind_invalid_time").into()));
        };
        let trigger_at_epoch = (Utc::now() + ChronoDuration::seconds(secs)).timestamp();

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

        // Bevestiging in de grootste eenheid waarin de duur (afgerond naar boven) netjes past
        let (count, unit_str) = if secs % 86_400 == 0 {
            (secs / 86_400, ctx.locale.t("remind_unit_day"))
        } else if secs % 3600 == 0 {
            (secs / 3600, ctx.locale.t("remind_unit_hour"))
        } else {
            ((secs + 59) / 60, ctx.locale.t("remind_unit_minute"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn timers_are_delivered_with_their_own_text() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let past = Utc::now().timestamp() - 5;
        for (msg, kind) in [("pizza", "remind"), ("koffie", "timer")] {
            sqlx::query("INSERT INTO reminders (author, channel, platform, message, trigger_at, kind) VALUES ('henk', '#a', 'irc', ?, ?, ?)")
                .bind(msg).bind(past).bind(kind).execute(&pool).await.unwrap();
        }
        let locale = LocaleManager::load("locales", "nl");
        let out = RemindPlugin::check_and_trigger_reminders(&pool, &locale).await.unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].3.contains("pizza") && !out[0].3.contains("Timer"), "{:?}", out[0]);
        assert!(out[1].3.contains("Timer afgelopen") && out[1].3.contains("koffie"), "{:?}", out[1]);
        // niet twee keer afleveren
        assert!(RemindPlugin::check_and_trigger_reminders(&pool, &locale).await.unwrap().is_empty());
    }
}
