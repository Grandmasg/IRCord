use regex::Regex;
use std::sync::OnceLock;

static MIRC_REGEX: OnceLock<Regex> = OnceLock::new();
static DISCORD_MENTION_REGEX: OnceLock<Regex> = OnceLock::new();
static DISCORD_EMOJI_REGEX: OnceLock<Regex> = OnceLock::new();

/// Inserts a zero-width space into the nickname to prevent unwanted highlights/pings on IRC.
pub fn anti_ping_nick(nick: &str) -> String {
    if nick.chars().count() > 1 {
        let mut chars = nick.chars();
        let first = chars.next().unwrap();
        let rest: String = chars.collect();
        format!("{}\u{200B}{}", first, rest)
    } else {
        nick.to_string()
    }
}

/// Strips mIRC control codes for colors, bold, underline, and reverse video (\x03, \x02, \x1f, etc.)
pub fn strip_mirc_codes(text: &str) -> String {
    let re = MIRC_REGEX.get_or_init(|| {
        Regex::new(r"(\x03(\d{1,2}(,\d{1,2})?)?|\x02|\x1F|\x16|\x0F)").unwrap()
    });
    re.replace_all(text, "").to_string()
}

/// Converts custom Discord emojis `<:name:12345678>` into readable `:name:` for IRC
pub fn sanitize_discord_emojis(text: &str) -> String {
    let re = DISCORD_EMOJI_REGEX.get_or_init(|| {
        Regex::new(r"<a?:([a-zA-Z0-9_]+):\d+>").unwrap()
    });
    re.replace_all(text, ":$1:").to_string()
}

/// Strips newlines and carriage returns for safe transmission to IRC PRIVMSG
pub fn sanitize_for_irc(text: &str) -> String {
    text.replace('\r', " ").replace('\n', " ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anti_ping() {
        assert_eq!(anti_ping_nick("Kuuke"), "K\u{200B}uuke");
        assert_eq!(anti_ping_nick("A"), "A");
    }

    #[test]
    fn test_strip_mirc_codes() {
        let raw = "\x0304Rode tekst\x0F en \x02vetgedrukt\x02";
        assert_eq!(strip_mirc_codes(raw), "Rode tekst en vetgedrukt");
    }

    #[test]
    fn test_sanitize_emojis() {
        let raw = "Kijk hier <:pepe:987654321> en <a:party:12345>";
        assert_eq!(sanitize_discord_emojis(raw), "Kijk hier :pepe: en :party:");
    }
}
