use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use scraper::{Html, Selector};
use std::sync::OnceLock;
use std::time::Duration;

static URL_REGEX: OnceLock<Regex> = OnceLock::new();

pub struct UrlTitlerPlugin;

impl UrlTitlerPlugin {
    /// Beveiliging tegen Server-Side Request Forgery (SSRF):
    /// Blokkeert loopback (127.0.0.1, localhost), private LAN adressen (10.x, 192.168.x, 172.16-31.x),
    /// cloud metadata (169.254.x) en gevaarlijke interne poorten.
    pub fn is_safe_public_url(url_str: &str) -> bool {
        let Ok(parsed) = reqwest::Url::parse(url_str) else {
            return false;
        };

        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return false;
        }

        let Some(host_str) = parsed.host_str() else {
            return false;
        };

        let host_lower = host_str.to_lowercase();
        if host_lower == "localhost"
            || host_lower.ends_with(".localhost")
            || host_lower.ends_with(".local")
            || host_lower.ends_with(".internal")
            || host_lower.ends_with(".lan")
            || host_lower.ends_with(".home.arpa")
        {
            return false;
        }

        if let Ok(ip) = host_lower.parse::<std::net::IpAddr>() {
            match ip {
                std::net::IpAddr::V4(ipv4) => {
                    let octets = ipv4.octets();
                    // Loopback (127.0.0.0/8) & Unspecified (0.0.0.0/8)
                    if octets[0] == 127 || octets[0] == 0 {
                        return false;
                    }
                    // Private netwerken (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
                    if octets[0] == 10 {
                        return false;
                    }
                    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                        return false;
                    }
                    if octets[0] == 192 && octets[1] == 168 {
                        return false;
                    }
                    // Link-local / Cloud metadata (169.254.0.0/16)
                    if octets[0] == 169 && octets[1] == 254 {
                        return false;
                    }
                    if ipv4.is_broadcast() {
                        return false;
                    }
                }
                std::net::IpAddr::V6(ipv6) => {
                    if ipv6.is_loopback() || ipv6.is_unspecified() {
                        return false;
                    }
                    let segs = ipv6.segments();
                    // Unique local fc00::/7 & Link-local fe80::/10
                    if (segs[0] & 0xfe00) == 0xfc00 || (segs[0] & 0xffc0) == 0xfe80 {
                        return false;
                    }
                }
            }
        }

        // Toegestane poorten (voorkomt probing van SSH, Ollama 11434, Bot Web 9090, etc.)
        if let Some(port) = parsed.port() {
            if port != 80 && port != 443 && port != 8080 && port != 8443 {
                return false;
            }
        }

        true
    }
}

#[async_trait]
impl Plugin for UrlTitlerPlugin {
    fn name(&self) -> &'static str { "url_titler" }
    fn help(&self) -> &'static str { "Automatische preview van webpagina titels (beveiligd met SSRF-filter)" }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let re = URL_REGEX.get_or_init(|| {
            Regex::new(r"https?://[^\s/$.?#].[^\s]*").unwrap()
        });

        if let Some(mat) = re.find(&msg.content) {
            let url = mat.as_str();

            // 1. SSRF en interne IP-beveiliging
            if !Self::is_safe_public_url(url) {
                return Ok(None);
            }

            // 2. Negeer directe afbeeldings- en mediabestanden
            if url.ends_with(".png") || url.ends_with(".jpg") || url.ends_with(".jpeg") || url.ends_with(".gif") || url.ends_with(".mp4") {
                return Ok(None);
            }

            let resp = match ctx.http.get(url)
                .timeout(Duration::from_secs(4))
                .header("User-Agent", "Mozilla/5.0 (compatible; IRCordBot/1.0)")
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r,
                _ => return Ok(None),
            };

            // Check if the content-type is html
            if let Some(ct) = resp.headers().get("content-type") {
                if !ct.to_str().unwrap_or("").contains("text/html") {
                    return Ok(None);
                }
            }

            let body = resp.text().await.unwrap_or_default();
            let document = Html::parse_document(&body);
            let title_selector = Selector::parse("title").unwrap();

            if let Some(element) = document.select(&title_selector).next() {
                let title = element.text().collect::<Vec<_>>().join(" ").trim().replace('\n', " ");
                if !title.is_empty() {
                    let label = ctx.locale.t("link_title");
                    return Ok(Some(format!("🔗 [{}] {}", label, title)));
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssrf_protection() {
        // Gevaarlijke interne URL's moeten geblokkeerd worden
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://127.0.0.1:8080"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://localhost/secret"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://192.168.1.1/admin"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://10.0.0.5/api"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://172.20.0.2:9090"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://169.254.169.254/latest/meta-data"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://my-nas.local/"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://router.internal/"));
        assert!(!UrlTitlerPlugin::is_safe_public_url("http://google.com:22/")); // Gevaarlijke poort

        // Veilige publieke URL's moeten toegestaan worden
        assert!(UrlTitlerPlugin::is_safe_public_url("https://github.com/rust-lang/rust"));
        assert!(UrlTitlerPlugin::is_safe_public_url("https://tweakers.net/nieuws"));
        assert!(UrlTitlerPlugin::is_safe_public_url("http://example.com/test"));
    }
}
