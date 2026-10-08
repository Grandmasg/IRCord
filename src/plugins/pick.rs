use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use rand::seq::SliceRandom;
use sqlx::Row;

/// Aantal minuten waarin iemand gepraat moet hebben om in aanmerking te komen.
const ACTIVE_MINUTES: i64 = 60;

/// `!pick [vraag]` / `!wie [vraag]`: kiest willekeurig iemand die recent in dit kanaal heeft gepraat.
pub struct PickPlugin;

#[async_trait]
impl Plugin for PickPlugin {
    fn name(&self) -> &'static str { "pick" }
    fn triggers(&self) -> &[&'static str] { &["pick", "wie"] }
    fn help(&self) -> &'static str {
        "!pick [vraag] - kiest willekeurig iemand die het laatste uur heeft gepraat, bijv. !pick wie doet de afwas?"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::minutes(ACTIVE_MINUTES)).to_rfc3339();
        let rows = sqlx::query(
            "SELECT DISTINCT author FROM chat_history WHERE channel = ? AND timestamp >= ? \
             AND author NOT IN ('IRCord', 'Monkeybot') AND message NOT LIKE '!%' AND message NOT LIKE '.%'",
        )
        .bind(&cmd.channel)
        .bind(&cutoff)
        .fetch_all(&ctx.db)
        .await?;
        let mut names: Vec<String> = rows.iter().filter_map(|r| r.try_get::<String, _>("author").ok()).collect();
        // Wie het commando geeft praat nu ook; telt dus altijd mee
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&cmd.author)) {
            names.push(cmd.author.clone());
        }
        let chosen = names.choose(&mut rand::thread_rng()).cloned().unwrap_or_else(|| cmd.author.clone());
        let question = cmd.args.trim();
        Ok(Some(if question.is_empty() {
            format!("🎯 {} is gekozen!", chosen)
        } else {
            format!("🎯 {} \u{2192} \x02{}\x02 (uit {} mensen)", question.chars().take(120).collect::<String>(), chosen, names.len())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn only_recent_non_bot_chatters_are_candidates() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let now = chrono::Utc::now();
        let rows = [
            ("#a", "henk", "hoi", now.to_rfc3339()),
            ("#a", "piet", "ook hoi", (now - chrono::Duration::minutes(10)).to_rfc3339()),
            ("#a", "oud", "lang geleden", (now - chrono::Duration::hours(5)).to_rfc3339()),
            ("#a", "Monkeybot", "ik ben een bot", now.to_rfc3339()),
            ("#a", "cmdgebruiker", "!roll", now.to_rfc3339()),
            ("#b", "ander", "ander kanaal", now.to_rfc3339()),
        ];
        for (c, a, m, t) in rows {
            sqlx::query("INSERT INTO chat_history (channel, author, platform, message, timestamp) VALUES (?, ?, 'irc', ?, ?)")
                .bind(c).bind(a).bind(m).bind(t).execute(&pool).await.unwrap();
        }
        let cutoff = (now - chrono::Duration::minutes(ACTIVE_MINUTES)).to_rfc3339();
        let got: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT author FROM chat_history WHERE channel = ? AND timestamp >= ? \
             AND author NOT IN ('IRCord', 'Monkeybot') AND message NOT LIKE '!%' AND message NOT LIKE '.%' ORDER BY author",
        )
        .bind("#a").bind(&cutoff).fetch_all(&pool).await.unwrap();
        assert_eq!(got.into_iter().map(|g| g.0).collect::<Vec<_>>(), vec!["henk", "piet"]);
    }
}
