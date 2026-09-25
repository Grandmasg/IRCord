use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Instant;

pub struct SysadminPlugin;

#[derive(Deserialize, Debug)]
struct DohAnswer {
    name: String,
    #[serde(rename = "type")]
    record_type: u16,
    #[serde(rename = "TTL")]
    ttl: u32,
    data: String,
}

#[derive(Deserialize, Debug)]
struct DohResponse {
    #[serde(rename = "Status")]
    status: i32,
    #[serde(rename = "Answer")]
    answer: Option<Vec<DohAnswer>>,
}

#[async_trait]
impl Plugin for SysadminPlugin {
    fn name(&self) -> &'static str {
        "sysadmin"
    }

    fn triggers(&self) -> &[&'static str] {
        &["nas", "hw", "sysinfo", "dns", "ssl", "http"]
    }

    fn help(&self) -> &'static str {
        "!nas - Minisforum N5 Pro & hardware telemetry | !dns <domain> [type] | !ssl <domain> | !http <url>"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trigger = cmd.trigger.as_str();

        match trigger {
            "nas" | "hw" | "sysinfo" => self.handle_nas(ctx, cmd).await,
            "dns" => self.handle_dns(ctx, cmd).await,
            "ssl" => self.handle_ssl(ctx, cmd).await,
            "http" => self.handle_http(ctx, cmd).await,
            _ => Ok(None),
        }
    }
}

impl SysadminPlugin {
    async fn handle_nas(
        &self,
        ctx: &PluginContext,
        _cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let os_name = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // Try reading memory and loadavg on Linux / container environments
        #[allow(unused_mut)]
        let mut load_str = "N/A".to_string();
        #[allow(unused_mut)]
        let mut ram_str = "N/A".to_string();

        #[cfg(target_os = "linux")]
        {
            if let Ok(load) = std::fs::read_to_string("/proc/loadavg") {
                let parts: Vec<&str> = load.split_whitespace().take(3).collect();
                if parts.len() == 3 {
                    load_str = parts.join(", ");
                }
            }

            if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
                let mut total_kb: u64 = 0;
                let mut avail_kb: u64 = 0;
                for line in mem.lines() {
                    if line.starts_with("MemTotal:") {
                        total_kb = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    } else if line.starts_with("MemAvailable:") {
                        avail_kb = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    }
                }
                if total_kb > 0 {
                    let used_kb = total_kb.saturating_sub(avail_kb);
                    let used_gb = used_kb as f64 / (1024.0 * 1024.0);
                    let total_gb = total_kb as f64 / (1024.0 * 1024.0);
                    let pct = (used_kb as f64 / total_kb as f64) * 100.0;
                    ram_str = format!("{:.1} GB / {:.1} GB ({:.0}%)", used_gb, total_gb, pct);
                }
            }
        }

        // Check GPU / ROCm device status for Minisforum N5 Pro (Radeon 890M)
        let has_rocm_devices = std::path::Path::new("/dev/kfd").exists() && std::path::Path::new("/dev/dri").exists();
        let ai_online = ctx.ai_client.ping().await;
        let gpu_status = if has_rocm_devices {
            if ai_online { "Radeon 890M GPU (ROCm Online ✅)" } else { "Radeon 890M GPU (ROCm Standby)" }
        } else if ai_online {
            "AI Accelerator (Online ✅)"
        } else {
            "AI Accelerator (Offline ❌)"
        };

        let current_model = ctx.ai_manager.get_model();

        Ok(Some(format!(
            "🖥️ [Minisforum N5 Pro / NAS] OS: {}/{} ({} CPU threads) | Load: [{}] | RAM: {}\n\
            ⚡ AI Acceleration: {} | Active Model: '{}'",
            os_name, arch, cores, load_str, ram_str, gpu_status, current_model
        )))
    }

    async fn handle_dns(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = cmd.args.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(Some("🌐 Usage: !dns <domain> [A|AAAA|MX|TXT|CNAME]".into()));
        }

        let domain = parts[0].trim_start_matches("https://").trim_start_matches("http://").trim_matches('/');
        let r_type = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_else(|| "A".into());

        // Validate domain format
        if !domain.contains('.') || domain.contains(' ') || domain.len() > 253 {
            return Ok(Some("⚠️ Invalid domain name provided.".into()));
        }

        let doh_url = format!("https://cloudflare-dns.com/dns-query?name={}&type={}", domain, r_type);

        let res = ctx.http
            .get(&doh_url)
            .header("Accept", "application/dns-json")
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await;

        match res {
            Ok(resp) if resp.status().is_success() => {
                let doh_data: DohResponse = resp.json().await?;
                if let Some(answers) = doh_data.answer {
                    if answers.is_empty() {
                        return Ok(Some(format!("🌐 [DNS] {domain} ({r_type}): No records found.")));
                    }

                    let results: Vec<String> = answers
                        .iter()
                        .take(5)
                        .map(|a| format!("{} (TTL: {}s)", a.data, a.ttl))
                        .collect();

                    Ok(Some(format!("🌐 [DNS] \x02{domain}\x02 ({r_type}): {}", results.join(", "))))
                } else {
                    Ok(Some(format!("🌐 [DNS] {domain} ({r_type}): No records found (NXDOMAIN or empty).")))
                }
            }
            Ok(resp) => Ok(Some(format!("⚠️ Cloudflare DNS query returned status code: {}", resp.status()))),
            Err(e) => Ok(Some(format!("❌ DNS lookup failed: {}", e))),
        }
    }

    async fn handle_ssl(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let domain = cmd.args.trim().trim_start_matches("https://").trim_start_matches("http://").trim_matches('/');
        if domain.is_empty() || !domain.contains('.') {
            return Ok(Some("🔒 Usage: !ssl <domain> (e.g. !ssl tweakers.net)".into()));
        }

        // Perform HTTPS check
        let url = format!("https://{}", domain);
        let start = Instant::now();
        let res = ctx.http
            .head(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;

        match res {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis();
                let has_hsts = resp.headers().contains_key("strict-transport-security");
                let hsts_badge = if has_hsts { "HSTS: Enabled ✅" } else { "HSTS: None ⚠️" };

                Ok(Some(format!(
                    "🔒 [SSL/TLS] \x02{}\x02: Valid HTTPS connection established ({}ms) | Status: {} | {}",
                    domain, latency_ms, resp.status(), hsts_badge
                )))
            }
            Err(e) => Ok(Some(format!("❌ [SSL/TLS] Connection to https://{} failed: {}", domain, e))),
        }
    }

    async fn handle_http(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let url = cmd.args.trim();
        if url.is_empty() {
            return Ok(Some("⚡ Usage: !http <url> (e.g. !http https://tweakers.net)".into()));
        }

        let full_url = if !url.starts_with("http://") && !url.starts_with("https://") {
            format!("https://{}", url)
        } else {
            url.to_string()
        };

        // Enforce SSRF safety guard
        if !crate::plugins::url_titler::UrlTitlerPlugin::is_safe_public_url(&full_url) {
            return Ok(Some("⛔ [Security] Private, localhost, or internal LAN addresses cannot be probed.".into()));
        }

        let start = Instant::now();
        let res = ctx.http
            .get(&full_url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await;

        match res {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis();
                let status = resp.status();
                let content_type = resp.headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown")
                    .split(';')
                    .next()
                    .unwrap_or("unknown");

                let server = resp.headers()
                    .get("server")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unspecified");

                let size_str = if let Some(len) = resp.content_length() {
                    let kb = len as f64 / 1024.0;
                    format!("{:.1} KB", kb)
                } else {
                    "Chunked".to_string()
                };

                Ok(Some(format!(
                    "⚡ [HTTP Probe] \x02{}\x02 -> \x02{}\x02 | Latency: {}ms | Type: {} | Size: {} | Server: {}",
                    full_url, status, latency_ms, content_type, size_str, server
                )))
            }
            Err(e) => Ok(Some(format!("❌ [HTTP Probe] Failed to connect to {}: {}", full_url, e))),
        }
    }
}
