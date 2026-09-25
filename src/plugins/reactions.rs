use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ReactionsPlugin {
    cooldowns: Mutex<HashMap<String, Instant>>, // "channel:category" -> Instant
}

impl ReactionsPlugin {
    pub fn new() -> Self {
        Self {
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    fn check_and_set_cooldown(&self, channel: &str, category: &str, cooldown_secs: u64) -> bool {
        let key = format!("{}:{}", channel, category);
        let mut lock = self.cooldowns.lock().unwrap();
        let now = Instant::now();

        if let Some(last) = lock.get(&key) {
            if now.duration_since(*last) < Duration::from_secs(cooldown_secs) {
                return false; // Still on cooldown
            }
        }

        lock.insert(key, now);
        true
    }
}

impl Default for ReactionsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ReactionsPlugin {
    fn name(&self) -> &'static str {
        "reactions"
    }

    fn help(&self) -> &'static str {
        "Reageert passief op gemeenschapsbegroetingen en trefwoorden (goedemorgen, welterusten, etc.)"
    }

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Ignore bot's own messages or commands
        let trimmed = msg.content.trim();
        if ctx.config.general.is_command_trigger(trimmed) || msg.author.eq_ignore_ascii_case("IRCord") {
            return Ok(None);
        }

        let lower = trimmed.to_lowercase();
        let is_dutch = ctx.config.general.language == "nl";

        // 1. Goedemorgen / Ochtendbegroeting
        if lower.starts_with("goedemorgen")
            || lower.starts_with("goeiemorgen")
            || lower.starts_with("mogguh")
            || lower.starts_with("mogge")
            || lower == "gm"
        {
            if self.check_and_set_cooldown(&msg.channel, "morning", 300) {
                let reply = if is_dutch {
                    format!("Goedemorgen \x02{}\x02! ☕ Fijne dag gewenst!", msg.author)
                } else {
                    format!("Good morning \x02{}\x02! ☕ Have a great day!", msg.author)
                };
                return Ok(Some(reply));
            }
        }

        // 2. Goedenavond / Avondbegroeting
        if lower.starts_with("goedenavond")
            || lower.starts_with("goeonavond")
            || lower.starts_with("fijne avond")
        {
            if self.check_and_set_cooldown(&msg.channel, "evening", 300) {
                let reply = if is_dutch {
                    format!("Goedenavond \x02{}\x02! 🌆 Gezellige avond gewenst.", msg.author)
                } else {
                    format!("Good evening \x02{}\x02! 🌆 Have a pleasant evening.", msg.author)
                };
                return Ok(Some(reply));
            }
        }

        // 3. Welterusten / Nacht
        if lower.starts_with("welterusten")
            || lower.starts_with("slaap lekker")
            || lower.starts_with("trusten")
            || lower == "gn"
        {
            if self.check_and_set_cooldown(&msg.channel, "night", 300) {
                let reply = if is_dutch {
                    format!("Welterusten \x02{}\x02! 🌙 Slaap lekker.", msg.author)
                } else {
                    format!("Good night \x02{}\x02! 🌙 Sleep well.", msg.author)
                };
                return Ok(Some(reply));
            }
        }

        // 4. Directe begroeting aan de bot (bijv. "hallo bot", "hoi ircord", "hey botje")
        let bot_name = ctx.config.general.bot_owner_irc_nick.to_lowercase();
        if (lower.contains("bot") || lower.contains("ircord") || (!bot_name.is_empty() && lower.contains(&bot_name)))
            && (lower.starts_with("hallo") || lower.starts_with("hoi") || lower.starts_with("hey") || lower.starts_with("hi "))
        {
            if self.check_and_set_cooldown(&msg.channel, "hello", 120) {
                let reply = if is_dutch {
                    format!("Hoi \x02{}\x02! 👋 Alles goed?", msg.author)
                } else {
                    format!("Hello \x02{}\x02! 👋 How are you doing?", msg.author)
                };
                return Ok(Some(reply));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooldown_mechanism() {
        let plugin = ReactionsPlugin::new();
        assert!(plugin.check_and_set_cooldown("#test", "morning", 60));
        // Immediate second call should be blocked by cooldown
        assert!(!plugin.check_and_set_cooldown("#test", "morning", 60));
        // Different category or channel should succeed
        assert!(plugin.check_and_set_cooldown("#test", "evening", 60));
        assert!(plugin.check_and_set_cooldown("#other", "morning", 60));
    }
}
