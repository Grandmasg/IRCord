//! Mentions, replies, stickers en embeds tussen Discord en IRC.

use regex::Regex;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::sync::OnceLock;

const NICK_CHARS: &str = r"[A-Za-z0-9_\[\]\\`^{}|\-]";

fn discord_mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<(@!?|@&|#)(\d+)>").unwrap())
}

/// Vervangt `<@id>`, `<@!id>`, `<@&id>` en `<#id>` door leesbare namen voor IRC. Onbekende ID's worden "@onbekend" / "#onbekend".
pub fn discord_to_irc_mentions(
    content: &str,
    users: &HashMap<String, String>,
    roles: &HashMap<String, String>,
    channels: &HashMap<String, String>,
) -> String {
    discord_mention_re()
        .replace_all(content, |caps: &regex::Captures| {
            let id = &caps[2];
            match &caps[1] {
                "#" => format!("#{}", channels.get(id).map(String::as_str).unwrap_or("onbekend")),
                "@&" => format!("@{}", roles.get(id).map(String::as_str).unwrap_or("onbekend")),
                _ => format!("@{}", users.get(id).map(String::as_str).unwrap_or("onbekend")),
            }
        })
        .into_owned()
}

/// Voegt bijlagen, stickers en embed-titels toe aan de berichttekst (voor IRC, dat alleen tekst kent).
pub fn compose_discord_content(content: &str, attachment_urls: &[String], sticker_names: &[String], embeds: &[(Option<String>, Option<String>)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !content.trim().is_empty() {
        parts.push(content.trim().to_string());
    }
    parts.extend(attachment_urls.iter().cloned());
    for name in sticker_names {
        parts.push(format!("[sticker: {}]", name));
    }
    for (title, url) in embeds {
        // Link-previews van een URL die al in het bericht staat zijn dubbel
        if url.as_deref().map(|u| content.contains(u)).unwrap_or(false) {
            continue;
        }
        if let Some(t) = title.as_deref().filter(|t| !t.trim().is_empty()) {
            parts.push(format!("[embed: {}]", t.trim()));
        }
    }
    parts.join(" ")
}

/// "Bob: dat klopt" / "Bob, dat klopt" => Some("Bob")
pub fn leading_addressee(content: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(&format!(r"^({}{{1,32}})[:,]\s+\S", NICK_CHARS)).unwrap());
    re.captures(content.trim_start()).map(|c| c[1].to_string())
}

/// Citaatregel voor Discord (webhooks kunnen geen echte reply sturen).
pub fn format_reply_quote(target: &str, original: &str) -> String {
    let flat: String = original.split_whitespace().collect::<Vec<_>>().join(" ");
    let snippet: String = if flat.chars().count() > 80 {
        format!("{}…", flat.chars().take(80).collect::<String>())
    } else {
        flat
    };
    format!("> **{}**: {}\n", target, snippet)
}

/// Alle IRC-nicks in een bericht die mogelijk een mention zijn: `@nick` of een `nick:` aan het begin (max. 5).
pub fn irc_mention_candidates(content: &str) -> Vec<String> {
    static AT: OnceLock<Regex> = OnceLock::new();
    let at = AT.get_or_init(|| Regex::new(&format!(r"(?:^|\s)@({}{{1,32}})", NICK_CHARS)).unwrap());
    let mut out: Vec<String> = Vec::new();
    if let Some(n) = leading_addressee(content) {
        out.push(n);
    }
    for c in at.captures_iter(content) {
        let n = c[1].to_string();
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            out.push(n);
        }
    }
    out.truncate(5);
    out
}

/// Vervangt gekoppelde nicks door `<@discord_id>` en geeft de te pingen ID's terug.
pub fn apply_irc_mentions(content: &str, links: &[(String, String)]) -> (String, Vec<String>) {
    let mut text = content.to_string();
    let mut ids = Vec::new();
    for (nick, id) in links {
        let tag = format!("<@{}>", id);
        let escaped = regex::escape(nick);
        // "@nick" overal
        if let Ok(re) = Regex::new(&format!(r"(?i)(^|\s)@{}\b", escaped)) {
            text = re.replace_all(&text, |c: &regex::Captures| format!("{}{}", &c[1], tag)).into_owned();
        }
        // "nick:" of "nick," aan het begin
        if let Ok(re) = Regex::new(&format!(r"(?i)^(\s*){}([:,])", escaped)) {
            text = re.replace(&text, |c: &regex::Captures| format!("{}{}{}", &c[1], tag, &c[2])).into_owned();
        }
        if text.contains(&tag) && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    (text, ids)
}

/// Zoekt gekoppelde accounts (`!link`) op voor genoemde nicks en maakt er echte Discord-mentions van.
pub async fn resolve_irc_mentions(db: &SqlitePool, content: &str) -> (String, Vec<String>) {
    let mut links = Vec::new();
    for nick in irc_mention_candidates(content) {
        let row = sqlx::query("SELECT discord_id FROM account_links WHERE irc_nick = ? COLLATE NOCASE LIMIT 1")
            .bind(&nick)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
        if let Some(id) = row.and_then(|r| r.try_get::<String, _>("discord_id").ok()) {
            // Alleen cijfers: voorkomt dat een vervuilde rij ongewenste markup in het bericht zet
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                links.push((nick, id));
            }
        }
    }
    apply_irc_mentions(content, &links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn discord_mentions_become_names() {
        let out = discord_to_irc_mentions(
            "hoi <@1> en <@!2>, kijk in <#30> <@&40> <@999>",
            &map(&[("1", "Henk"), ("2", "Piet")]),
            &map(&[("40", "Mods")]),
            &map(&[("30", "algemeen")]),
        );
        assert_eq!(out, "hoi @Henk en @Piet, kijk in #algemeen @Mods @onbekend");
    }

    #[test]
    fn content_composition_with_stickers_embeds_and_attachments() {
        let c = compose_discord_content(
            "kijk https://a.nl/x",
            &["https://cdn/foto.png".to_string()],
            &["Wave".to_string()],
            &[(Some("Preview".into()), Some("https://a.nl/x".into())), (Some("Los embed".into()), None)],
        );
        assert_eq!(c, "kijk https://a.nl/x https://cdn/foto.png [sticker: Wave] [embed: Los embed]");
        assert_eq!(compose_discord_content("", &[], &["S".into()], &[]), "[sticker: S]");
    }

    #[test]
    fn addressee_and_quotes() {
        assert_eq!(leading_addressee("Bob: dat klopt").as_deref(), Some("Bob"));
        assert_eq!(leading_addressee("bob, ja").as_deref(), Some("bob"));
        assert_eq!(leading_addressee("http://x.nl"), None);
        assert_eq!(leading_addressee("gewoon tekst: hier"), None);
        let q = format_reply_quote("Bob", "regel een\nregel twee");
        assert_eq!(q, "> **Bob**: regel een regel twee\n");
        assert!(format_reply_quote("B", &"x".repeat(200)).contains('…'));
    }

    #[test]
    fn irc_mentions_resolve_only_linked_nicks() {
        assert_eq!(irc_mention_candidates("Henk: kijk @piet en @Henk"), vec!["Henk", "piet"]);
        let links = vec![("Henk".to_string(), "111".to_string())];
        let (text, ids) = apply_irc_mentions("Henk: hoi, ook @henk en @niemand", &links);
        assert_eq!(text, "<@111>: hoi, ook <@111> en @niemand");
        assert_eq!(ids, vec!["111"]);
        let (same, none) = apply_irc_mentions("geen mentions", &links);
        assert_eq!(same, "geen mentions");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn mentions_use_linked_accounts_from_database() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO account_links (discord_id, discord_tag, irc_nick) VALUES ('555', 'henk#1', 'Henk')")
            .execute(&pool)
            .await
            .unwrap();
        let (text, ids) = resolve_irc_mentions(&pool, "henk: ben je er? @onbekend ook").await;
        assert_eq!(text, "<@555>: ben je er? @onbekend ook");
        assert_eq!(ids, vec!["555"]);
    }
}
