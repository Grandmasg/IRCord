//! Per kanaal plugins aan/uit zetten: statisch via `disabled_plugins` in de config en dynamisch via `!plugin`.

use crate::config::Config;
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;
use std::sync::RwLock;

/// Deze plugins kunnen nooit worden uitgeschakeld (anders sluit je jezelf buiten).
pub const PROTECTED_PLUGINS: &[&str] = &["help", "plugins", "admin"];

#[derive(Default)]
pub struct PluginToggles {
    disabled: RwLock<HashSet<(String, String)>>,
}

/// Een Discord-kanaal en het gekoppelde IRC-kanaal delen dezelfde instellingen.
fn channel_keys(config: &Config, channel: &str) -> Vec<String> {
    let ch = channel.to_lowercase();
    let mut keys = vec![ch.clone()];
    for m in &config.channels {
        let irc = m.irc_channel.to_lowercase();
        let discord = m.discord_channel_id.to_string();
        if ch == irc {
            keys.push(discord);
        } else if ch == discord {
            keys.push(irc);
        }
    }
    keys
}

impl PluginToggles {
    pub fn from_config(config: &Config) -> Self {
        let t = Self::default();
        {
            let mut set = t.disabled.write().unwrap_or_else(|e| e.into_inner());
            for m in &config.channels {
                for plugin in &m.disabled_plugins {
                    let plugin = plugin.to_lowercase();
                    set.insert((m.irc_channel.to_lowercase(), plugin.clone()));
                    set.insert((m.discord_channel_id.to_string(), plugin));
                }
            }
        }
        t
    }

    /// Leest dynamisch uitgeschakelde plugins uit de database.
    pub async fn load_db(&self, db: &SqlitePool) {
        if let Ok(rows) = sqlx::query("SELECT channel, plugin FROM channel_plugins").fetch_all(db).await {
            let mut set = self.disabled.write().unwrap_or_else(|e| e.into_inner());
            for r in rows {
                let c: String = r.try_get("channel").unwrap_or_default();
                let p: String = r.try_get("plugin").unwrap_or_default();
                set.insert((c, p));
            }
        }
    }

    pub fn is_disabled(&self, channel: &str, plugin: &str) -> bool {
        if PROTECTED_PLUGINS.contains(&plugin) {
            return false;
        }
        self.disabled.read().unwrap_or_else(|e| e.into_inner()).contains(&(channel.to_lowercase(), plugin.to_lowercase()))
    }

    /// Zet een plugin aan of uit voor een kanaal (en het gekoppelde kanaal aan de andere kant).
    pub async fn set_disabled(&self, db: &SqlitePool, config: &Config, channel: &str, plugin: &str, disabled: bool) {
        let plugin = plugin.to_lowercase();
        for key in channel_keys(config, channel) {
            if disabled {
                self.disabled.write().unwrap_or_else(|e| e.into_inner()).insert((key.clone(), plugin.clone()));
                let _ = sqlx::query("INSERT OR IGNORE INTO channel_plugins (channel, plugin) VALUES (?, ?)")
                    .bind(&key)
                    .bind(&plugin)
                    .execute(db)
                    .await;
            } else {
                self.disabled.write().unwrap_or_else(|e| e.into_inner()).remove(&(key.clone(), plugin.clone()));
                let _ = sqlx::query("DELETE FROM channel_plugins WHERE channel = ? AND plugin = ?")
                    .bind(&key)
                    .bind(&plugin)
                    .execute(db)
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::load_from_str(
            r##"
            [general]
            bot_owner_discord_id = 1
            bot_owner_irc_nick = "x"
            [bridge]
            [whatpulse]
            team_name = "t"
            api_url = "https://x"
            [ai]
            base_url = "http://x"
            default_model = "m"
            [moderation]
            [open_meteo]
            [[channels]]
            irc_channel = "#Chat"
            discord_channel_id = 42
            discord_webhook_url = "https://discord.com/api/webhooks/1"
            disabled_plugins = ["Urban", "help"]
            "##,
        )
        .expect("config")
    }

    #[test]
    fn config_disables_for_both_sides_but_protects_core() {
        let c = cfg();
        let t = PluginToggles::from_config(&c);
        assert!(t.is_disabled("#chat", "urban"));
        assert!(t.is_disabled("42", "urban"));
        assert!(!t.is_disabled("#other", "urban"));
        assert!(!t.is_disabled("#chat", "help"));
    }

    #[test]
    fn keys_include_counterpart() {
        let c = cfg();
        let keys = channel_keys(&c, "#CHAT");
        assert!(keys.contains(&"#chat".to_string()) && keys.contains(&"42".to_string()));
        assert_eq!(channel_keys(&c, "42").len(), 2);
    }
}
