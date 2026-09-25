use serde::Deserialize;
use std::path::Path;
use std::fs;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub bridge: BridgeConfig,
    #[serde(default)]
    pub channels: Vec<ChannelMapping>,
    pub whatpulse: WhatPulseConfig,
    pub ai: AiConfig,
    pub moderation: ModerationConfig,
    #[serde(default)]
    pub open_meteo: OpenMeteoConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_language")]
    pub language: String,
    pub bot_owner_discord_id: u64,
    pub bot_owner_irc_nick: String,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_pastebin_threshold")]
    pub pastebin_threshold_lines: usize,
    #[serde(default = "default_admin_channel_irc")]
    pub admin_channel_irc: String,
    #[serde(default = "default_admin_channel_discord")]
    pub admin_channel_discord_id: u64,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_admin_channel_irc() -> String {
    "#bot-logs".to_string()
}

fn default_admin_channel_discord() -> u64 {
    0
}

fn default_http_port() -> u16 {
    9090
}

fn default_pastebin_threshold() -> usize {
    4
}

#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    #[serde(default = "default_loop_timeout")]
    pub loop_prevent_timeout_sec: u64,
    #[serde(default = "default_lru_capacity")]
    pub lru_cache_capacity: usize,
    #[serde(default = "default_true")]
    pub sync_presence: bool,
    #[serde(default = "default_true")]
    pub sync_edits: bool,
}

fn default_loop_timeout() -> u64 {
    10
}

fn default_lru_capacity() -> usize {
    2000
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ChannelMapping {
    pub irc_channel: String,
    pub discord_channel_id: u64,
    pub discord_webhook_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatPulseConfig {
    pub team_name: String,
    pub api_url: String,
    #[serde(default = "default_wp_poll_interval")]
    pub poll_interval_seconds: u64,
    #[serde(default = "default_wp_cache_ttl")]
    pub cache_ttl_seconds: u64,
}

fn default_wp_poll_interval() -> u64 {
    180
}

fn default_wp_cache_ttl() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiConfig {
    pub base_url: String,
    pub default_model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temp")]
    pub temperature: f32,
    #[serde(default = "default_sliding_window")]
    pub sliding_window_size: usize,
    #[serde(default = "default_token_budget")]
    pub hourly_token_budget: u64,
}

fn default_max_tokens() -> u32 {
    300
}

fn default_temp() -> f32 {
    0.7
}

fn default_sliding_window() -> usize {
    8
}

fn default_token_budget() -> u64 {
    50000
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModerationConfig {
    #[serde(default = "default_irc_flood_delay")]
    pub irc_flood_delay_ms: u64,
    #[serde(default = "default_irc_line_max")]
    pub irc_line_max_bytes: usize,
    #[serde(default = "default_raid_joins")]
    pub raid_threshold_joins_per_sec: u32,
    #[serde(default = "default_raid_mute")]
    pub raid_mute_duration_sec: u64,
}

fn default_irc_flood_delay() -> u64 {
    800
}

fn default_irc_line_max() -> usize {
    380
}

fn default_raid_joins() -> u32 {
    5
}

fn default_raid_mute() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenMeteoConfig {
    #[serde(default = "default_open_meteo_daily")]
    pub daily_limit: u64,
    #[serde(default = "default_open_meteo_hourly")]
    pub hourly_limit: u64,
    #[serde(default = "default_open_meteo_minutely")]
    pub minutely_limit: u64,
}

fn default_open_meteo_daily() -> u64 {
    9500
}

fn default_open_meteo_hourly() -> u64 {
    4500
}

fn default_open_meteo_minutely() -> u64 {
    500
}

impl Default for OpenMeteoConfig {
    fn default() -> Self {
        Self {
            daily_limit: default_open_meteo_daily(),
            hourly_limit: default_open_meteo_hourly(),
            minutely_limit: default_open_meteo_minutely(),
        }
    }
}

impl Config {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Controleer op dubbele IRC-kanalen
        let mut irc_chans = std::collections::HashSet::new();
        for mapping in &self.channels {
            if !irc_chans.insert(&mapping.irc_channel) {
                return Err(format!("Dubbel IRC kanaal gedefinieerd in configuratie: {}", mapping.irc_channel).into());
            }
        }

        // Controleer op dubbele Discord kanalen
        let mut discord_ids = std::collections::HashSet::new();
        for mapping in &self.channels {
            if !discord_ids.insert(mapping.discord_channel_id) {
                return Err(format!("Dubbel Discord kanaal-ID gedefinieerd: {}", mapping.discord_channel_id).into());
            }
            if !mapping.discord_webhook_url.starts_with("https://") {
                return Err(format!("Ongeldige Discord Webhook URL voor {}: moet beginnen met https://", mapping.irc_channel).into());
            }
        }

        Ok(())
    }
}
