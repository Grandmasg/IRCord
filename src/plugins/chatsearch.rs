//! `!zoek <woorden> [van:nick]`: doorzoekt de chatgeschiedenis van dit kanaal (SQLite FTS5).

use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::sanitizer::anti_ping_nick;
use async_trait::async_trait;
use sqlx::Row;

const MAX_TERMS: usize = 6;
const SHOWN: i64 = 3;
const SNIPPET_CHARS: usize = 110;

pub struct ChatSearchPlugin;

/// Gesplitste zoekopdracht: de termen en een optionele `van:nick`-filter.
#[derive(Debug, PartialEq)]
struct Query {
    terms: Vec<String>,
    from: Option<String>,
}

/// Haalt letters, cijfers en `._-` uit een term; alles wat FTS5-syntaxis kan zijn (aanhalingstekens, `*`, `:`, AND/OR/NOT...) valt weg.
fn clean_term(raw: &str) -> String {
    raw.chars().filter(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-')).collect::<String>().trim_matches(['.', '-', '_']).to_string()
}

fn parse_query(args: &str) -> Query {
    let mut terms = Vec::new();
    let mut from = None;
    for tok in args.split_whitespace() {
        let lower = tok.to_lowercase();
        if let Some(nick) = lower.strip_prefix("van:").or_else(|| lower.strip_prefix("from:")) {
            let nick: String = nick.chars().filter(|c| c.is_alphanumeric() || "_[]\\`^{}|-".contains(*c)).collect();
            if !nick.is_empty() {
                from = Some(nick);
            }
            continue;
        }
        let t = clean_term(tok);
        // FTS5-operatoren als losse woorden zijn geen zoekterm
        if t.chars().count() >= 2 && !matches!(t.to_uppercase().as_str(), "AND" | "OR" | "NOT" | "NEAR") && terms.len() < MAX_TERMS {
            terms.push(t);
        }
    }
    Query { terms, from }
}

/// FTS5-expressie: elke term als aparte, geciteerde zoekterm in de berichtkolom (impliciete AND).
fn fts_expression(terms: &[String]) -> String {
    let parts: Vec<String> = terms.iter().map(|t| format!("\"{}\"", t)).collect();
    format!("message : ({})", parts.join(" "))
}

fn short_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|d| d.with_timezone(&chrono::Local).format("%d-%m %H:%M").to_string())
        .unwrap_or_else(|_| ts.chars().take(16).collect())
}

fn snippet(msg: &str) -> String {
    let flat: String = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SNIPPET_CHARS {
        format!("{}…", flat.chars().take(SNIPPET_CHARS).collect::<String>())
    } else {
        flat
    }
}

#[async_trait]
impl Plugin for ChatSearchPlugin {
    fn name(&self) -> &'static str { "chatsearch" }
    fn triggers(&self) -> &[&'static str] { &["zoek", "chatzoek", "wiezei"] }
    fn help(&self) -> &'static str {
        "!zoek <woorden> [van:nick] - doorzoekt de chatgeschiedenis van dit kanaal en toont de nieuwste treffers (web zoeken: !g)"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let q = parse_query(&cmd.args);
        if q.terms.is_empty() {
            return Ok(Some("🔎 Gebruik: !zoek <woorden> [van:nick], bijvoorbeeld !zoek back-up van:henk (zoekt in de chat van dit kanaal; web zoeken doe je met !g)".into()));
        }
        let expr = fts_expression(&q.terms);
        let prefixes: Vec<String> = ctx.config.general.command_prefixes.iter().map(|p| format!("{}%", p.replace('%', ""))).collect();

        // Eigen opdrachten (!zoek zelf) en bot-antwoorden horen niet in de resultaten
        let mut where_extra = String::from(" AND author NOT IN ('IRCord', 'Monkeybot')");
        for _ in &prefixes {
            where_extra.push_str(" AND message NOT LIKE ?");
        }
        if q.from.is_some() {
            where_extra.push_str(" AND lower(author) = ?");
        }

        let base = format!("FROM chat_history WHERE chat_history MATCH ? AND channel = ?{}", where_extra);

        let count_sql = format!("SELECT COUNT(*) AS n {}", base);
        let mut count_q = sqlx::query(&count_sql).bind(&expr).bind(&cmd.channel);
        for p in &prefixes {
            count_q = count_q.bind(p);
        }
        if let Some(f) = &q.from {
            count_q = count_q.bind(f);
        }
        let total: i64 = count_q.fetch_one(&ctx.db).await?.try_get("n").unwrap_or(0);

        let label = q.terms.join(" ");
        if total == 0 {
            return Ok(Some(format!("🔎 Niets gevonden in dit kanaal voor '{}'.", label)));
        }

        let rows_sql = format!("SELECT timestamp, author, message {} ORDER BY rowid DESC LIMIT ?", base);
        let mut rows_q = sqlx::query(&rows_sql).bind(&expr).bind(&cmd.channel);
        for p in &prefixes {
            rows_q = rows_q.bind(p);
        }
        if let Some(f) = &q.from {
            rows_q = rows_q.bind(f);
        }
        let rows = rows_q.bind(SHOWN).fetch_all(&ctx.db).await?;

        let hits: Vec<String> = rows
            .iter()
            .map(|r| {
                let ts: String = r.try_get("timestamp").unwrap_or_default();
                let author: String = r.try_get("author").unwrap_or_default();
                let msg: String = r.try_get("message").unwrap_or_default();
                format!("[{}] {}: {}", short_time(&ts), anti_ping_nick(&author), snippet(&msg))
            })
            .collect();
        Ok(Some(format!("🔎 {} van {} treffers voor '{}' (nieuwste eerst): {}", hits.len(), total, label, hits.join(" | "))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn query_parsing_strips_fts_syntax() {
        let q = parse_query("back-up van:Henk \"x\" AND OR a *");
        assert_eq!(q.terms, vec!["back-up"]);
        assert_eq!(q.from.as_deref(), Some("henk"));
        let q = parse_query("kopie:hallo (test) NOT \"quote\"");
        assert_eq!(q.terms, vec!["kopiehallo", "test", "quote"]);
        assert!(parse_query("   ").terms.is_empty());
        assert_eq!(parse_query("aa bb cc dd ee ff gg hh").terms.len(), 6);
        assert_eq!(fts_expression(&["back-up".into(), "nas".into()]), "message : (\"back-up\" \"nas\")");
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(snippet("a   b\nc"), "a b c");
        assert!(snippet(&"x".repeat(300)).ends_with('…'));
        assert_eq!(short_time("kapot"), "kapot");
    }

    /// De SQL zelf, tegen een echte (in-memory) FTS5-tabel: kanaalfilter, auteurfilter, bot/commando's uitgesloten, nieuwste eerst.
    #[tokio::test]
    async fn search_sql_respects_channel_author_and_ordering() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let rows = [
            ("#a", "henk", "de back-up van de nas is mislukt"),
            ("#a", "piet", "back-up draait nu weer"),
            ("#a", "henk", "!zoek back-up"),
            ("#a", "Monkeybot", "🔎 1 van 2 treffers voor back-up"),
            ("#b", "henk", "back-up in een ander kanaal"),
            ("#a", "henk", "iets heel anders"),
            ("#a", "henk", "weer een back-up gelukt"),
        ];
        for (i, (c, a, m)) in rows.iter().enumerate() {
            sqlx::query("INSERT INTO chat_history (channel, author, platform, message, timestamp) VALUES (?, ?, 'irc', ?, ?)")
                .bind(c).bind(a).bind(m).bind(format!("2026-10-08T10:{:02}:00Z", i)).execute(&pool).await.unwrap();
        }
        let expr = fts_expression(&["back-up".to_string()]);
        let sql = "SELECT author, message FROM chat_history WHERE chat_history MATCH ? AND channel = ? \
                   AND author NOT IN ('IRCord', 'Monkeybot') AND message NOT LIKE ? ORDER BY rowid DESC LIMIT 10";
        let got: Vec<(String, String)> = sqlx::query_as(sql).bind(&expr).bind("#a").bind("!%").fetch_all(&pool).await.unwrap();
        let msgs: Vec<&str> = got.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(msgs, vec!["weer een back-up gelukt", "back-up draait nu weer", "de back-up van de nas is mislukt"]);
    }
}
