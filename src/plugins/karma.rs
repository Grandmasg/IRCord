use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use std::sync::OnceLock;

static KARMA_REGEX: OnceLock<Regex> = OnceLock::new();

pub struct KarmaPlugin;

#[async_trait]
impl Plugin for KarmaPlugin {
    fn name(&self) -> &'static str { "karma" }
    fn triggers(&self) -> &[&'static str] { &["karma"] }
    fn help(&self) -> &'static str { "!karma [nick] - Toont de reputatie van een gebruiker | nick++ of nick--" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let target = if cmd.args.trim().is_empty() {
            cmd.author.as_str()
        } else {
            cmd.args.trim()
        };

        let row = sqlx::query!(
            r#"
            SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
            FROM audit_log
            WHERE details = ?
            "#,
            target
        )
        .fetch_one(&ctx.db)
        .await?;

        let score = row.total.unwrap_or(0);
        Ok(Some(format!("⭐ [Karma] {} heeft een score van {}", target, score)))
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let re = KARMA_REGEX.get_or_init(|| {
            Regex::new(r"([a-zA-Z0-9_\-\[\]\\`^{}|]+)(\+\+|--)").unwrap()
        });

        if let Some(caps) = re.captures(&msg.content) {
            let target = caps.get(1).map_or("", |m| m.as_str());
            let op = caps.get(2).map_or("", |m| m.as_str());

            // Zelf-stemmen niet toegestaan
            if target.eq_ignore_ascii_case(&msg.author) {
                return Ok(Some(format!("🚫 {}, je mag je eigen karma niet aanpassen!", msg.author)));
            }

            let action = if op == "++" { "karma_up" } else { "karma_down" };

            sqlx::query!(
                r#"
                INSERT INTO audit_log (operator, platform, action, details)
                VALUES (?, ?, ?, ?)
                "#,
                msg.author,
                msg.platform,
                action,
                target
            )
            .execute(&ctx.db)
            .await?;

            // Haal nieuwe score op
            let row = sqlx::query!(
                r#"
                SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
                FROM audit_log
                WHERE details = ?
                "#,
                target
            )
            .fetch_one(&ctx.db)
            .await?;

            let score = row.total.unwrap_or(0);
            return Ok(Some(format!("⭐ [Karma] {} heeft nu een score van {}", target, score)));
        }

        Ok(None)
    }
}
