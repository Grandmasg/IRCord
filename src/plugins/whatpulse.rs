use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, error};

#[derive(Debug, Deserialize, Clone)]
pub struct WpTeamStats {
    #[serde(default = "default_team_name")]
    pub name: String,
    #[serde(default)]
    pub rank: u64,
    #[serde(default)]
    pub members: u64,
    #[serde(default)]
    pub keys: u64,
    #[serde(default)]
    pub clicks: u64,
    #[serde(default)]
    pub download_mb: u64,
    #[serde(default)]
    pub upload_mb: u64,
}

fn default_team_name() -> String {
    "Deapen".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct WpMember {
    pub username: String,
    pub keys: u64,
    pub clicks: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WpResponse {
    pub team: Option<WpTeamStats>,
    #[serde(default)]
    pub top_members: Vec<WpMember>,
}

#[derive(Debug, Deserialize)]
struct WpApiTeamSearchResponse {
    teams: Option<Vec<WpApiTeamSummary>>,
}

#[derive(Debug, Deserialize)]
struct WpApiTeamSummary {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct WpApiTeamShowResponse {
    team: Option<WpApiTeamDetails>,
}

#[derive(Debug, Deserialize)]
struct WpApiTeamDetails {
    id: Option<u64>,
    name: Option<String>,
    rank: Option<u64>,
    members: Option<u64>,
    keys: Option<u64>,
    clicks: Option<u64>,
    download: Option<u64>,
    upload: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WpApiUserSearchResponse {
    users: Option<Vec<WpApiUserSummary>>,
}

#[derive(Debug, Deserialize)]
struct WpApiUserSummary {
    id: u64,
    username: String,
}

#[derive(Debug, Deserialize)]
struct WpApiUserResponse {
    user: Option<WpApiUserData>,
}

#[derive(Debug, Deserialize)]
struct WpApiUserData {
    username: Option<String>,
    totals: Option<WpApiUserTotals>,
    ranks: Option<WpApiUserRanks>,
}

#[derive(Debug, Deserialize)]
struct WpApiUserTotals {
    keys: Option<u64>,
    clicks: Option<u64>,
    download: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WpApiUserRanks {
    keys: Option<u64>,
    clicks: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WpClientStats {
    keys: Option<String>,
    keys_formatted: Option<String>,
    clicks: Option<String>,
    clicks_formatted: Option<String>,
    download_formatted: Option<String>,
    uptime_formatted: Option<String>,
}

use std::sync::atomic::{AtomicUsize, Ordering};

pub struct WhatPulsePlugin {
    cached_data: Mutex<Option<(WpResponse, Instant)>>,
    key_index: AtomicUsize,
}

impl WhatPulsePlugin {
    pub fn new() -> Self {
        Self {
            cached_data: Mutex::new(None),
            key_index: AtomicUsize::new(0),
        }
    }

    /// Verzamelt alle beschikbare WhatPulse API-sleutels (via config.toml en/of WHATPULSE_API_KEY)
    fn get_api_keys(&self, ctx: &PluginContext) -> Vec<String> {
        let mut keys = Vec::new();
        for k in &ctx.config.whatpulse.api_keys {
            let trimmed = k.trim();
            if !trimmed.is_empty() && !keys.contains(&trimmed.to_string()) {
                keys.push(trimmed.to_string());
            }
        }
        if let Some(ref single) = ctx.config.whatpulse.api_key {
            let trimmed = single.trim();
            if !trimmed.is_empty() && !keys.contains(&trimmed.to_string()) {
                keys.push(trimmed.to_string());
            }
        }
        if let Ok(env_keys) = std::env::var("WHATPULSE_API_KEY") {
            for part in env_keys.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() && !keys.contains(&trimmed.to_string()) {
                    keys.push(trimmed.to_string());
                }
            }
        }
        keys
    }

    /// Geeft de geconfigureerde Client API URL terug (lokaal op de computer of LAN)
    fn get_client_url(&self, ctx: &PluginContext) -> String {
        if let Some(ref url) = ctx.config.whatpulse.client_url {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        std::env::var("WHATPULSE_CLIENT_URL").unwrap_or_else(|_| "http://localhost:3490/v1/account-totals".to_string())
    }

    async fn fetch_stats(&self, ctx: &PluginContext) -> Result<WpResponse, Box<dyn std::error::Error + Send + Sync>> {
        let ttl = Duration::from_secs(ctx.config.whatpulse.cache_ttl_seconds);

        // Check cache
        {
            let cache = self.cached_data.lock().unwrap();
            if let Some((ref data, ref timestamp)) = *cache {
                if timestamp.elapsed() < ttl {
                    debug!("WhatPulse statistieken opgehaald uit cache");
                    return Ok(data.clone());
                }
            }
        }

        // Fetch via officiële WhatPulse Web API v1 (zie https://whatpulse.org/help/api/intro)
        let keys = self.get_api_keys(ctx);
        if keys.is_empty() {
            return Err("Geen WhatPulse API sleutel geconfigureerd (stel 'api_keys' of 'api_key' in via config/compose of WHATPULSE_API_KEY).".into());
        }

        let team_name = &ctx.config.whatpulse.team_name;
        let mut last_err = String::new();
        let mut successful_resp = None;

        // Probeer beschikbare keys (met rotatie en fallback bij 429 rate limit)
        let start_idx = self.key_index.fetch_add(1, Ordering::Relaxed) % keys.len();
        for i in 0..keys.len() {
            let current_key = &keys[(start_idx + i) % keys.len()];
            let search_url = format!("https://whatpulse.org/api/v1/teams?search={}", team_name);

            let search_resp = match ctx
                .http
                .get(&search_url)
                .bearer_auth(current_key)
                .timeout(Duration::from_secs(8))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("Verbindingsfout naar WhatPulse: {}", e);
                    continue;
                }
            };

            if search_resp.status() == 429 || search_resp.status() == 401 {
                last_err = format!("WhatPulse API status {} (rate limit of autorisatie met key #{})", search_resp.status(), (start_idx + i) % keys.len() + 1);
                continue;
            }

            if !search_resp.status().is_success() {
                last_err = format!("WhatPulse v1 API fout (status {})", search_resp.status());
                continue;
            }

            let search_data: WpApiTeamSearchResponse = match search_resp.json().await {
                Ok(d) => d,
                Err(e) => {
                    last_err = format!("Fout bij parsen team response: {}", e);
                    continue;
                }
            };

            let team_summary = match search_data.teams.and_then(|t| t.into_iter().next()) {
                Some(s) => s,
                None => return Err(format!("Team '{}' niet gevonden op WhatPulse", team_name).into()),
            };

            let show_url = format!("https://whatpulse.org/api/v1/teams/{}", team_summary.id);
            let show_resp = match ctx
                .http
                .get(&show_url)
                .bearer_auth(current_key)
                .timeout(Duration::from_secs(8))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("Fout bij ophalen team details: {}", e);
                    continue;
                }
            };

            if !show_resp.status().is_success() {
                last_err = format!("WhatPulse v1 team details fout (status {})", show_resp.status());
                continue;
            }

            let show_data: WpApiTeamShowResponse = match show_resp.json().await {
                Ok(d) => d,
                Err(e) => {
                    last_err = format!("Fout bij parsen team details: {}", e);
                    continue;
                }
            };

            if let Some(details) = show_data.team {
                successful_resp = Some(WpResponse {
                    team: Some(WpTeamStats {
                        name: details.name.unwrap_or_else(|| team_name.clone()),
                        rank: details.rank.unwrap_or(0),
                        members: details.members.unwrap_or(0),
                        keys: details.keys.unwrap_or(0),
                        clicks: details.clicks.unwrap_or(0),
                        download_mb: details.download.unwrap_or(0) / (1024 * 1024),
                        upload_mb: details.upload.unwrap_or(0) / (1024 * 1024),
                    }),
                    top_members: Vec::new(),
                });
                break;
            }
        }

        let resp: WpResponse = successful_resp.ok_or_else(|| format!("WhatPulse team data kon niet worden opgehaald: {}", last_err))?;

        // Update cache
        {
            let mut cache = self.cached_data.lock().unwrap();
            *cache = Some((resp.clone(), Instant::now()));
        }

        Ok(resp)
    }

    async fn fetch_user_stats(
        &self,
        ctx: &PluginContext,
        target: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Probeer officiële WhatPulse Web API v1 als er API keys zijn geconfigureerd
        let keys = self.get_api_keys(ctx);
        if !keys.is_empty() {
            let start_idx = self.key_index.fetch_add(1, Ordering::Relaxed) % keys.len();
            for i in 0..keys.len() {
                let current_key = &keys[(start_idx + i) % keys.len()];
                let search_url = format!("https://whatpulse.org/api/v1/users?search={}", target);
                let mut user_id_opt: Option<u64> = target.parse::<u64>().ok();

                if user_id_opt.is_none() {
                    if let Ok(s_resp) = ctx.http.get(&search_url).bearer_auth(current_key).timeout(Duration::from_secs(6)).send().await {
                        if s_resp.status().is_success() {
                            if let Ok(s_data) = s_resp.json::<WpApiUserSearchResponse>().await {
                                if let Some(first) = s_data.users.and_then(|u| u.into_iter().next()) {
                                    user_id_opt = Some(first.id);
                                }
                            }
                        } else if s_resp.status() == 429 || s_resp.status() == 401 {
                            continue; // Probeer volgende sleutel
                        }
                    }
                }

                if let Some(uid) = user_id_opt {
                    let show_url = format!("https://whatpulse.org/api/v1/users/{}", uid);
                    if let Ok(resp) = ctx.http.get(&show_url).bearer_auth(current_key).timeout(Duration::from_secs(6)).send().await {
                        if resp.status().is_success() {
                            if let Ok(data) = resp.json::<WpApiUserResponse>().await {
                                if let Some(user) = data.user {
                                    let keys = user.totals.as_ref().and_then(|t| t.keys).unwrap_or(0);
                                    let clicks = user.totals.as_ref().and_then(|t| t.clicks).unwrap_or(0);
                                    let download = user.totals.as_ref().and_then(|t| t.download).unwrap_or(0);
                                    let rank_keys = user.ranks.as_ref().and_then(|r| r.keys).unwrap_or(0);
                                    let uname = user.username.unwrap_or_else(|| target.to_string());

                                    return Ok(Some(format!(
                                        "⌨️ [WhatPulse: {}] Keys: {} (Rank #{}) | Clicks: {} | Download: {} MB",
                                        uname,
                                        Self::format_number(keys),
                                        rank_keys,
                                        Self::format_number(clicks),
                                        Self::format_number(download / (1024 * 1024))
                                    )));
                                }
                            }
                        } else if resp.status() == 429 || resp.status() == 401 {
                            continue; // Probeer volgende sleutel bij rate limit
                        }
                    }
                }
            }
        }

        // 2. Probeer WhatPulse Client API (lokaal of LAN, bijv. poort 3490)
        let client_url = self.get_client_url(ctx);
        if let Ok(resp) = ctx.http.get(&client_url).timeout(Duration::from_secs(3)).send().await {
            if resp.status().is_success() {
                if let Ok(client_stats) = resp.json::<WpClientStats>().await {
                    let keys = client_stats.keys_formatted.or(client_stats.keys).unwrap_or_default();
                    let clicks = client_stats.clicks_formatted.or(client_stats.clicks).unwrap_or_default();
                    let dl = client_stats.download_formatted.unwrap_or_default();
                    let uptime = client_stats.uptime_formatted.unwrap_or_default();

                    return Ok(Some(format!(
                        "⌨️ [WhatPulse Client: {}] Keys: {} | Clicks: {} | Download: {} | Uptime: {}",
                        target, keys, clicks, dl, uptime
                    )));
                }
            }
        }

        Ok(Some(ctx.locale.tf("whatpulse_api_key_tip", &[("target", target)])))
    }

    fn format_number(val: u64) -> String {
        let s = val.to_string();
        let mut out = String::new();
        let len = s.len();
        for (idx, ch) in s.chars().enumerate() {
            out.push(ch);
            let rem = len - 1 - idx;
            if rem > 0 && rem % 3 == 0 {
                out.push('.');
            }
        }
        out
    }
}

#[async_trait]
impl Plugin for WhatPulsePlugin {
    fn name(&self) -> &'static str { "whatpulse" }
    fn triggers(&self) -> &[&'static str] { &["wp", "whatpulse"] }
    fn help(&self) -> &'static str { "!wp - Team stats | !wp <user> / !wp me | !wp top | !wp link <user>" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.split_whitespace();
        let subcmd = parts.next().unwrap_or("").to_lowercase();

        match subcmd.as_str() {
            "top" => {
                let data = self.fetch_stats(ctx).await?;
                if data.top_members.is_empty() {
                    return Ok(Some(ctx.locale.t("whatpulse_no_top").into()));
                }
                let mut lines = Vec::new();
                for (idx, member) in data.top_members.iter().take(5).enumerate() {
                    lines.push(format!(
                        "#{}: {} ({} keys, {} clicks)",
                        idx + 1,
                        member.username,
                        Self::format_number(member.keys),
                        Self::format_number(member.clicks)
                    ));
                }
                Ok(Some(format!("🏆 [WhatPulse Top - {}] {}", ctx.config.whatpulse.team_name, lines.join(" | "))))
            }
            "link" => {
                let target = parts.next().unwrap_or("");
                if target.is_empty() {
                    return Ok(Some(ctx.locale.t("whatpulse_link_usage").into()));
                }
                sqlx::query!(
                    r#"
                    INSERT INTO whatpulse_links (nick, platform, whatpulse_username)
                    VALUES (?, ?, ?)
                    ON CONFLICT(nick, platform) DO UPDATE SET whatpulse_username = excluded.whatpulse_username
                    "#,
                    cmd.author,
                    cmd.platform,
                    target
                )
                .execute(&ctx.db)
                .await?;

                Ok(Some(ctx.locale.tf(
                    "whatpulse_linked",
                    &[("author", &cmd.author), ("username", target)],
                )))
            }
            "me" => {
                let row = sqlx::query!(
                    "SELECT whatpulse_username FROM whatpulse_links WHERE nick = ? AND platform = ?",
                    cmd.author,
                    cmd.platform
                )
                .fetch_optional(&ctx.db)
                .await?;

                if let Some(r) = row {
                    self.fetch_user_stats(ctx, &r.whatpulse_username).await
                } else {
                    Ok(Some(ctx.locale.t("whatpulse_not_linked").into()))
                }
            }
            "user" => {
                let target = parts.next().unwrap_or("");
                if target.is_empty() {
                    return Ok(Some(ctx.locale.t("whatpulse_user_usage").into()));
                }
                self.fetch_user_stats(ctx, target).await
            }
            "" => {
                // General team stats
                let data = self.fetch_stats(ctx).await?;
                let team = data.team.unwrap_or(WpTeamStats {
                    name: ctx.config.whatpulse.team_name.clone(),
                    rank: 0,
                    members: 0,
                    keys: 0,
                    clicks: 0,
                    download_mb: 0,
                    upload_mb: 0,
                });

                let rank_str = team.rank.to_string();
                let members_str = team.members.to_string();
                let keys_str = Self::format_number(team.keys);
                let clicks_str = Self::format_number(team.clicks);
                let dl_str = Self::format_number(team.download_mb);

                let stats = ctx.locale.tf(
                    "whatpulse_team_stats",
                    &[
                        ("name", &team.name),
                        ("rank", &rank_str),
                        ("members", &members_str),
                        ("keys", &keys_str),
                        ("clicks", &clicks_str),
                        ("download", &dl_str),
                    ],
                );

                Ok(Some(stats))
            }
            other => {
                self.fetch_user_stats(ctx, other).await
            }
        }
    }
}
