//! `!img <url>` en `!upload <url>`: afbeeldingen en bestanden vanaf IRC naar het gekoppelde Discord-kanaal sturen.

use super::{CommandEvent, Plugin, PluginContext};
use crate::discord::webhook::DiscordPost;
use crate::utils::ssrf;
use async_trait::async_trait;

/// Discord-webhooks accepteren standaard maximaal 8 MB per bestand (zonder boost).
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

/// Uitvoerbare of scriptbestanden sturen we nooit door.
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "dll", "bat", "cmd", "com", "msi", "scr", "ps1", "vbs", "js", "jar", "apk", "sh", "lnk", "reg", "hta",
];

pub struct MediaPlugin;

fn is_image_type(content_type: &str) -> bool {
    let ct = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    matches!(ct.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" | "image/bmp")
}

/// Bestandsnaam uit het URL-pad; alleen veilige tekens, anders een standaardnaam.
fn filename_from_url(url: &str) -> String {
    let last = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(str::to_string)))
        .unwrap_or_default();
    let clean: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(80)
        .collect();
    if clean.trim_matches('.').is_empty() { "bestand".to_string() } else { clean }
}

fn is_blocked_name(name: &str) -> bool {
    name.rsplit('.').next().map(|ext| BLOCKED_EXTENSIONS.contains(&ext.to_lowercase().as_str())).unwrap_or(false)
}

#[async_trait]
impl Plugin for MediaPlugin {
    fn name(&self) -> &'static str { "media" }
    fn triggers(&self) -> &[&'static str] { &["img", "upload"] }
    fn help(&self) -> &'static str {
        "!img <url> - toont een afbeelding in Discord | !upload <url> - stuurt een bestand (max 8 MB) naar Discord"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if cmd.platform != "irc" {
            return Ok(Some("ℹ️ Op Discord kun je bestanden en afbeeldingen gewoon zelf plaatsen; dit commando is voor IRC.".into()));
        }
        let Some(tx) = ctx.discord_post_tx.as_ref() else {
            return Ok(Some("⚠️ Discord is niet gekoppeld; er valt niets door te sturen.".into()));
        };
        if !ctx.config.channels.iter().any(|m| m.irc_channel.eq_ignore_ascii_case(&cmd.channel)) {
            return Ok(Some("ℹ️ Dit kanaal is niet aan Discord gekoppeld.".into()));
        }

        let url = cmd.args.split_whitespace().next().unwrap_or("");
        if url.is_empty() {
            return Ok(Some(format!("Gebruik: !{} <url>", cmd.trigger)));
        }
        if !ssrf::is_safe_public_url(url) {
            return Ok(Some("⛔ Deze URL is niet toegestaan (alleen publieke http(s)-adressen).".into()));
        }

        let resp = match ssrf::safe_client().get(url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => return Ok(Some(format!("⚠️ De server antwoordde met status {}.", r.status()))),
            Err(_) => return Ok(Some("⚠️ Kon de URL niet ophalen.".into())),
        };
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if resp.content_length().map(|l| l as usize > MAX_UPLOAD_BYTES).unwrap_or(false) {
            return Ok(Some("⚠️ Het bestand is groter dan 8 MB.".into()));
        }

        if cmd.trigger == "img" {
            if !is_image_type(&content_type) {
                return Ok(Some("⚠️ Dit is geen ondersteunde afbeelding (png, jpg, gif, webp, avif, bmp).".into()));
            }
            let _ = tx
                .send(DiscordPost {
                    irc_channel: cmd.channel.clone(),
                    username: format!("{} (IRC)", cmd.author),
                    content: String::new(),
                    file: None,
                    image_url: Some(url.to_string()),
                })
                .await;
            return Ok(Some(format!("🖼️ Afbeelding van {} naar Discord gestuurd.", cmd.author)));
        }

        // !upload
        let filename = filename_from_url(url);
        if is_blocked_name(&filename) || content_type.to_lowercase().starts_with("text/html") {
            return Ok(Some("⛔ Dit bestandstype wordt niet doorgestuurd.".into()));
        }
        let bytes = ssrf::read_limited(resp, MAX_UPLOAD_BYTES + 1).await;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Ok(Some("⚠️ Het bestand is groter dan 8 MB.".into()));
        }
        if bytes.is_empty() {
            return Ok(Some("⚠️ Het bestand is leeg.".into()));
        }
        let size_kb = bytes.len() / 1024;
        let _ = tx
            .send(DiscordPost {
                irc_channel: cmd.channel.clone(),
                username: format!("{} (IRC)", cmd.author),
                content: format!("📎 {} deelde een bestand: {}", cmd.author, filename),
                file: Some((filename.clone(), bytes)),
                image_url: None,
            })
            .await;
        Ok(Some(format!("📎 {} ({} KB) naar Discord gestuurd.", filename, size_kb.max(1))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_types() {
        assert!(is_image_type("image/PNG; charset=binary"));
        assert!(is_image_type("image/webp"));
        assert!(!is_image_type("image/svg+xml"));
        assert!(!is_image_type("text/html"));
    }

    #[test]
    fn filenames_and_blocking() {
        assert_eq!(filename_from_url("https://example.com/a/b/foto.png?x=1"), "foto.png");
        assert_eq!(filename_from_url("https://example.com/"), "bestand");
        assert_eq!(filename_from_url("https://example.com/a%2F..%2Fb.txt"), "a2F..2Fb.txt");
        assert!(is_blocked_name("setup.EXE"));
        assert!(is_blocked_name("x.ps1"));
        assert!(!is_blocked_name("rapport.pdf"));
    }
}
