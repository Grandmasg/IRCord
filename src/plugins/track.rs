use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use sqlx::Row;

const MAX_TRACKS_PER_USER: i64 = 10;

/// `!track <trefwoord>`: persoonlijke trefwoord-alerts op RSS-feeds, afgeleverd via privébericht.
pub struct TrackPlugin;

#[async_trait]
impl Plugin for TrackPlugin {
    fn name(&self) -> &'static str { "track" }
    fn triggers(&self) -> &[&'static str] { &["track", "untrack"] }
    fn help(&self) -> &'static str {
        "!track <woord> - Krijg een privébericht bij nieuwe RSS-artikelen met dat woord | !track list | !untrack <woord>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if cmd.platform != "irc" {
            return Ok(Some("🔔 [Track] Trefwoord-alerts worden via IRC-privébericht afgeleverd en zijn daarom alleen op IRC beschikbaar.".into()));
        }
        let user = cmd.author.to_lowercase();
        let args = cmd.args.trim();

        let (action, rest) = if cmd.trigger == "untrack" {
            ("del", args)
        } else {
            match args.split_once(' ') {
                Some((a, r)) if matches!(a.to_lowercase().as_str(), "add" | "del" | "remove" | "list") => (a, r.trim()),
                _ if args.eq_ignore_ascii_case("list") || args.is_empty() => ("list", ""),
                _ => ("add", args),
            }
        };

        match action.to_lowercase().as_str() {
            "list" => {
                let rows = sqlx::query("SELECT keyword FROM user_tracks WHERE platform = 'irc' AND user_id = ? ORDER BY id")
                    .bind(&user)
                    .fetch_all(&ctx.db)
                    .await?;
                if rows.is_empty() {
                    return Ok(Some("🔔 [Track] Je volgt nog geen woorden. Gebruik: !track <woord>".into()));
                }
                let words: Vec<String> = rows.iter().map(|r| r.try_get::<String, _>("keyword").unwrap_or_default()).collect();
                Ok(Some(format!("🔔 [Track] Je volgt: {}", words.join(", "))))
            }
            "del" | "remove" => {
                let kw = rest.to_lowercase();
                if kw.is_empty() {
                    return Ok(Some("Gebruik: !untrack <woord>".into()));
                }
                let res = sqlx::query("DELETE FROM user_tracks WHERE platform = 'irc' AND user_id = ? AND lower(keyword) = ?")
                    .bind(&user)
                    .bind(&kw)
                    .execute(&ctx.db)
                    .await?;
                Ok(Some(if res.rows_affected() > 0 {
                    format!("🔕 [Track] Gestopt met volgen van \x02{}\x02", kw)
                } else {
                    format!("⚠️ [Track] Je volgde \x02{}\x02 niet.", kw)
                }))
            }
            _ => {
                let kw = rest.to_lowercase();
                let len = kw.chars().count();
                if !(3..=40).contains(&len) {
                    return Ok(Some("⚠️ [Track] Een trefwoord moet 3 tot 40 tekens lang zijn.".into()));
                }
                let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM user_tracks WHERE platform = 'irc' AND user_id = ?")
                    .bind(&user)
                    .fetch_one(&ctx.db)
                    .await?
                    .try_get("n")
                    .unwrap_or(0);
                if count >= MAX_TRACKS_PER_USER {
                    return Ok(Some(format!("⚠️ [Track] Maximaal {} trefwoorden per gebruiker. Verwijder er eerst een met !untrack.", MAX_TRACKS_PER_USER)));
                }
                let exists = sqlx::query("SELECT 1 FROM user_tracks WHERE platform = 'irc' AND user_id = ? AND lower(keyword) = ?")
                    .bind(&user)
                    .bind(&kw)
                    .fetch_optional(&ctx.db)
                    .await?
                    .is_some();
                if exists {
                    return Ok(Some(format!("ℹ️ [Track] Je volgt \x02{}\x02 al.", kw)));
                }
                sqlx::query("INSERT INTO user_tracks (user_id, platform, keyword) VALUES (?, 'irc', ?)")
                    .bind(&user)
                    .bind(&kw)
                    .execute(&ctx.db)
                    .await?;
                Ok(Some(format!("🔔 [Track] Je krijgt een privébericht bij nieuwe RSS-artikelen met \x02{}\x02.", kw)))
            }
        }
    }
}
