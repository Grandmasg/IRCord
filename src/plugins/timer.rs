use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::duration::{format_duration, parse_duration};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::Row;

const MAX_ACTIVE_PER_USER: i64 = 5;

/// `!timer <duur> [label]`: een aftelling die in het kanaal afgaat. Gebruikt dezelfde achtergrondtaak als !remind.
pub struct TimerPlugin;

#[async_trait]
impl Plugin for TimerPlugin {
    fn name(&self) -> &'static str { "timer" }
    fn triggers(&self) -> &[&'static str] { &["timer"] }
    fn help(&self) -> &'static str {
        "!timer <duur> [label] - bijv. !timer 10m koffie, !timer 1u30m, !timer 90s | !timer list | !timer stop <id|alles>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let dutch = ctx.locale.is_dutch();
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();

        match first.to_lowercase().as_str() {
            "" => Ok(Some(if dutch {
                "⏱️ Gebruik: !timer <duur> [label], bijvoorbeeld !timer 10m koffie of !timer 1u30m | !timer list | !timer stop <id|alles>".into()
            } else {
                "⏱️ Usage: !timer <duration> [label], e.g. !timer 10m coffee or !timer 1h30m | !timer list | !timer stop <id|all>".into()
            })),
            "list" | "lijst" => {
                let rows = sqlx::query(
                    "SELECT id, message, trigger_at FROM reminders WHERE kind = 'timer' AND delivered_at IS NULL \
                     AND lower(author) = lower(?) AND platform = ? ORDER BY trigger_at",
                )
                .bind(&cmd.author)
                .bind(&cmd.platform)
                .fetch_all(&ctx.db)
                .await?;
                if rows.is_empty() {
                    return Ok(Some(if dutch { "⏱️ Je hebt geen actieve timers.".into() } else { "⏱️ You have no active timers.".into() }));
                }
                let now = Utc::now().timestamp();
                let items: Vec<String> = rows
                    .iter()
                    .map(|r| {
                        let id: i64 = r.try_get("id").unwrap_or(0);
                        let msg: String = r.try_get("message").unwrap_or_default();
                        let at: i64 = r.try_get("trigger_at").unwrap_or(now);
                        format!("#{} {} ({})", id, msg, format_duration((at - now).max(0), dutch))
                    })
                    .collect();
                Ok(Some(format!("⏱️ {}", items.join(" | "))))
            }
            "stop" | "cancel" | "annuleer" => {
                let res = if rest.eq_ignore_ascii_case("alles") || rest.eq_ignore_ascii_case("all") {
                    sqlx::query("DELETE FROM reminders WHERE kind = 'timer' AND delivered_at IS NULL AND lower(author) = lower(?) AND platform = ?")
                        .bind(&cmd.author)
                        .bind(&cmd.platform)
                        .execute(&ctx.db)
                        .await?
                } else {
                    let Ok(id) = rest.trim_start_matches('#').parse::<i64>() else {
                        return Ok(Some(if dutch { "⏱️ Gebruik: !timer stop <id|alles>".into() } else { "⏱️ Usage: !timer stop <id|all>".into() }));
                    };
                    sqlx::query("DELETE FROM reminders WHERE id = ? AND kind = 'timer' AND delivered_at IS NULL AND lower(author) = lower(?) AND platform = ?")
                        .bind(id)
                        .bind(&cmd.author)
                        .bind(&cmd.platform)
                        .execute(&ctx.db)
                        .await?
                };
                let n = res.rows_affected();
                Ok(Some(if dutch { format!("⏱️ {} timer(s) gestopt.", n) } else { format!("⏱️ {} timer(s) stopped.", n) }))
            }
            _ => {
                let Some(secs) = parse_duration(first, 60) else {
                    return Ok(Some(if dutch {
                        "⏱️ Ongeldige duur. Gebruik bijvoorbeeld 90s, 10m, 1u30m of 2d (maximaal een jaar).".into()
                    } else {
                        "⏱️ Invalid duration. Try 90s, 10m, 1h30m or 2d (one year at most).".into()
                    }));
                };
                let active: i64 = sqlx::query(
                    "SELECT COUNT(*) AS n FROM reminders WHERE kind = 'timer' AND delivered_at IS NULL AND lower(author) = lower(?) AND platform = ?",
                )
                .bind(&cmd.author)
                .bind(&cmd.platform)
                .fetch_one(&ctx.db)
                .await?
                .try_get("n")
                .unwrap_or(0);
                if active >= MAX_ACTIVE_PER_USER {
                    return Ok(Some(if dutch {
                        format!("⏱️ Je hebt al {} actieve timers. Stop er een met !timer stop <id>.", MAX_ACTIVE_PER_USER)
                    } else {
                        format!("⏱️ You already have {} active timers. Stop one with !timer stop <id>.", MAX_ACTIVE_PER_USER)
                    }));
                }
                let label: String = if rest.is_empty() { "timer".to_string() } else { rest.chars().take(200).collect() };
                let at = Utc::now().timestamp() + secs;
                let id = sqlx::query(
                    "INSERT INTO reminders (author, channel, platform, message, trigger_at, kind) VALUES (?, ?, ?, ?, ?, 'timer')",
                )
                .bind(&cmd.author)
                .bind(&cmd.channel)
                .bind(&cmd.platform)
                .bind(&label)
                .bind(at)
                .execute(&ctx.db)
                .await?
                .last_insert_rowid();
                Ok(Some(if dutch {
                    format!("⏱️ Timer #{} gezet: {} over {}", id, label, format_duration(secs, true))
                } else {
                    format!("⏱️ Timer #{} set: {} in {}", id, label, format_duration(secs, false))
                }))
            }
        }
    }
}
