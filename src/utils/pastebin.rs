//! Lange (code)berichten van Discord naar IRC: uploaden naar dpaste.org (opt-in) of netjes inkorten.
//! dpaste.org: `POST https://dpaste.org/api/` met formulierveld `content`; het antwoord is de paste-URL.
//! Gratis en zonder sleutel. Let op: de tekst wordt naar een derde partij gestuurd, daarom staat dit standaard uit
//! (`pastebin_enabled = false`).

use reqwest::Client;
use std::time::Duration;

const DPASTE_API: &str = "https://dpaste.org/api/";
const PASTE_EXPIRY_DAYS: &str = "7";

/// Uploadt tekst naar dpaste.org en geeft de URL terug.
pub async fn upload(http: &Client, text: &str) -> Option<String> {
    let resp = http
        .post(DPASTE_API)
        .timeout(Duration::from_secs(8))
        .form(&[("content", text), ("expiry_days", PASTE_EXPIRY_DAYS)])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    parse_paste_url(&resp.text().await.ok()?)
}

fn parse_paste_url(body: &str) -> Option<String> {
    let url = body.trim().trim_matches('"').trim();
    url.starts_with("https://dpaste.org/").then(|| url.to_string())
}

/// Kort een bericht in voor IRC. Meer dan `max_lines` regels: uploaden (indien `paste_enabled`) of inkorten.
pub async fn shorten_for_irc(http: &Client, content: &str, max_lines: usize, paste_enabled: bool) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if max_lines == 0 || lines.len() <= max_lines {
        return content.to_string();
    }
    if paste_enabled {
        if let Some(url) = upload(http, content).await {
            return format!("{} … (+{} regels) {}", lines[0].trim(), lines.len() - 1, url);
        }
    }
    let kept = lines[..max_lines].join(" ⏎ ");
    format!("{} … [+{} regels ingekort]", kept, lines.len() - max_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn short_messages_untouched_and_long_ones_truncated() {
        let http = Client::new();
        assert_eq!(shorten_for_irc(&http, "a\nb", 4, false).await, "a\nb");
        let long = "1\n2\n3\n4\n5\n6";
        assert_eq!(shorten_for_irc(&http, long, 4, false).await, "1 ⏎ 2 ⏎ 3 ⏎ 4 … [+2 regels ingekort]");
        assert_eq!(shorten_for_irc(&http, long, 0, false).await, long);
    }

    #[test]
    fn parses_dpaste_response() {
        assert_eq!(parse_paste_url("\"https://dpaste.org/sREO8\"\n").as_deref(), Some("https://dpaste.org/sREO8"));
        assert_eq!(parse_paste_url("<html>error</html>"), None);
    }
}
