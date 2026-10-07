//! SSRF-bescherming voor URL's die door gebruikers worden opgegeven (!http, !ssl, !rss, !tldr, link-titels).
//!
//! Drie lagen:
//! 1. `is_safe_public_url`: schema, poort en letterlijke host/IP-controle (inclusief IPv6 en `[::1]`).
//! 2. `safe_client`: een HTTP-client waarvan de DNS-resolver alleen publieke IP's teruggeeft
//!    (stopt hostnamen zoals `127.0.0.1.nip.io` en DNS-rebinding naar intern netwerk).
//! 3. Redirects worden per stap opnieuw gecontroleerd en beperkt tot 5.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const ALLOWED_PORTS: [u16; 4] = [80, 443, 8080, 8443];
const MAX_REDIRECTS: usize = 5;

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT 100.64.0.0/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)     // IETF protocol assignments
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking 198.18.0.0/15
        || o[0] >= 240)                                 // gereserveerd
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let seg = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (seg[0] == 0x2001 && seg[1] == 0x0db8)) // documentatie
}

/// Is dit een publiek routeerbaar IP-adres?
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

/// Statische controle van een URL (zonder DNS): schema, poort, bekende interne hostnamen en IP-literals.
pub fn is_safe_public_url(url_str: &str) -> bool {
    let Ok(parsed) = Url::parse(url_str) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    if let Some(port) = parsed.port() {
        if !ALLOWED_PORTS.contains(&port) {
            return false;
        }
    }
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => is_public_v4(ip),
        Some(url::Host::Ipv6(ip)) => is_public_v6(ip),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_lowercase();
            !(d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.ends_with(".lan")
                || d.ends_with(".home.arpa")
                || !d.contains('.'))
        }
        None => false,
    }
}

/// DNS-resolver die uitsluitend publieke adressen doorgeeft.
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public_ip(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{} wijst niet naar een publiek IP-adres", host).into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// HTTP-client voor niet-vertrouwde URL's: publiek-only DNS, gecontroleerde redirects, timeout.
pub fn safe_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .dns_resolver(Arc::new(PublicOnlyResolver))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("te veel redirects")
                } else if is_safe_public_url(attempt.url().as_str()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect naar niet-publiek adres geblokkeerd")
                }
            }))
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; IRCordBot/1.0)")
            .build()
            .expect("safe HTTP client")
    })
}

/// Leest een respons tot maximaal `max_bytes` (voorkomt geheugenmisbruik door enorme bodies).
pub async fn read_limited(mut resp: reqwest::Response, max_bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        let room = max_bytes.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if out.len() >= max_bytes {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_internal_literals() {
        for u in [
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fd00::1]/",
            "http://2130706433/",      // decimaal 127.0.0.1
            "http://0x7f.1/",          // hex-notatie
            "http://100.64.0.1/",      // CGNAT
            "http://169.254.169.254/latest/meta-data",
            "http://192.168.1.1/",
            "http://localhost/",
            "http://nas/",             // kale hostnaam zonder punt
            "http://user:pw@example.com/",
            "http://example.com:22/",
            "ftp://example.com/",
        ] {
            assert!(!is_safe_public_url(u), "had geblokkeerd moeten worden: {u}");
        }
    }

    #[test]
    fn allows_public_urls() {
        for u in ["https://example.com/", "http://8.8.8.8/", "https://[2606:4700:4700::1111]/", "https://tweakers.net:443/x"] {
            assert!(is_safe_public_url(u), "had toegestaan moeten worden: {u}");
        }
    }

    #[test]
    fn ip_classification() {
        assert!(!is_public_ip("10.1.2.3".parse().unwrap()));
        assert!(!is_public_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
    }
}
