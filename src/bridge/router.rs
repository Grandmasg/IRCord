use super::{BridgeMessage, Platform, ReplyContext};
use crate::config::ChannelMapping;
use crate::utils::sanitizer::{anti_ping_nick, sanitize_discord_emojis, sanitize_for_irc, strip_mirc_codes};
use lru::LruCache;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct IrcMessageRef {
    pub channel: String,
    pub author: String,
    pub content: String,
    pub timestamp_secs: u64,
}

pub struct BridgeRouter {
    mappings: Vec<ChannelMapping>,
    command_prefixes: Vec<String>,
    // 1. Discord Message ID -> IRC Context & Auteur
    discord_to_irc: Mutex<LruCache<String, IrcMessageRef>>,
    // 2. IRC Context Hash -> Discord Message ID
    irc_to_discord: Mutex<LruCache<u64, String>>,
}

impl BridgeRouter {
    pub fn new(mappings: Vec<ChannelMapping>, lru_capacity: usize) -> Self {
        Self::with_prefixes(mappings, lru_capacity, vec!["!".to_string(), ".".to_string()])
    }

    pub fn with_prefixes(mappings: Vec<ChannelMapping>, lru_capacity: usize, prefixes: Vec<String>) -> Self {
        let cap = NonZeroUsize::new(lru_capacity.max(2000)).unwrap();
        Self {
            mappings,
            command_prefixes: prefixes,
            discord_to_irc: Mutex::new(LruCache::new(cap)),
            irc_to_discord: Mutex::new(LruCache::new(cap)),
        }
    }

    /// Berekent een stabiele 64-bit hash van een IRC bericht voor bi-directionele caching
    pub fn compute_message_hash(channel: &str, author: &str, content: &str) -> u64 {
        let mut s = DefaultHasher::new();
        channel.hash(&mut s);
        author.to_lowercase().hash(&mut s);
        content.trim().hash(&mut s);
        s.finish()
    }

    /// Slaat een bi-directionele koppeling op tussen een Discord Message ID en IRC context
    pub fn record_bridge_link(&self, discord_id: String, channel: String, author: String, content: String) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let hash = Self::compute_message_hash(&channel, &author, &content);

        let irc_ref = IrcMessageRef {
            channel,
            author: author.clone(),
            content,
            timestamp_secs: now,
        };

        {
            let mut d2i = self.discord_to_irc.lock().unwrap();
            d2i.put(discord_id.clone(), irc_ref);
        }

        {
            let mut i2d = self.irc_to_discord.lock().unwrap();
            i2d.put(hash, discord_id);
        }
    }

    /// Zoekt de IRC context op basis van een Discord Message ID (bijv. bij een reply of emoji-reactie)
    pub fn lookup_irc_by_discord_id(&self, discord_id: &str) -> Option<IrcMessageRef> {
        let mut cache = self.discord_to_irc.lock().unwrap();
        cache.get(discord_id).cloned()
    }

    /// Zoekt het Discord Message ID op basis van een IRC bericht
    pub fn lookup_discord_by_irc(&self, channel: &str, author: &str, content: &str) -> Option<String> {
        let hash = Self::compute_message_hash(channel, author, content);
        let mut cache = self.irc_to_discord.lock().unwrap();
        cache.get(&hash).cloned()
    }

    /// Achterwaartse compatibiliteit voor reply-tracking
    pub fn record_message_author(&self, message_id: String, author: String) {
        self.record_bridge_link(message_id, "".into(), author, "".into());
    }

    pub fn lookup_reply_author(&self, message_id: &str) -> Option<String> {
        self.lookup_irc_by_discord_id(message_id).map(|r| r.author)
    }

    /// Zoekt het gekoppelde Discord kanaal en de webhook URL voor een IRC kanaal
    pub fn get_discord_destination(&self, irc_channel: &str) -> Option<&ChannelMapping> {
        self.mappings.iter().find(|m| m.irc_channel.eq_ignore_ascii_case(irc_channel))
    }

    /// Zoekt het gekoppelde IRC kanaal voor een Discord kanaal-ID
    pub fn get_irc_destination(&self, discord_channel_id: u64) -> Option<&ChannelMapping> {
        self.mappings.iter().find(|m| m.discord_channel_id == discord_channel_id)
    }

    /// Determines if a message should be ignored to avoid bridge loops or command conflicts
    pub fn should_ignore(&self, content: &str) -> bool {
        let trimmed = content.trim();
        for prefix in &self.command_prefixes {
            if trimmed.starts_with(prefix) {
                return true;
            }
        }
        if trimmed.starts_with('~') || trimmed.starts_with('/') {
            return true;
        }
        false
    }

    /// Formatteert een binnenkomend Discord-bericht voor weergave op IRC
    pub fn format_for_irc(&self, msg: &BridgeMessage) -> String {
        let safe_nick = anti_ping_nick(&msg.author_name);
        let stripped_content = sanitize_discord_emojis(&msg.content);
        let clean = sanitize_for_irc(&stripped_content);

        if let Some(ref reply) = msg.reply_to {
            format!("<{} \u{21B3} {}> {}", safe_nick, reply.target_nick, clean)
        } else if msg.is_action {
            format!("* {} {}", safe_nick, clean)
        } else {
            format!("<{}> {}", safe_nick, clean)
        }
    }

    /// Formatteert een binnenkomend IRC-bericht voor weergave via een Discord Webhook
    pub fn format_for_discord_webhook(&self, msg: &BridgeMessage) -> (String, String) {
        let username = format!("{} (IRC)", msg.author_name);
        let clean_content = strip_mirc_codes(&msg.content);
        (username, clean_content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_for_irc() {
        let mappings = vec![ChannelMapping {
            irc_channel: "#test".into(),
            discord_channel_id: 123,
            discord_webhook_url: "https://discord.com/...".into(),
            language: None,
        }];
        let router = BridgeRouter::new(mappings, 100);

        let msg = BridgeMessage {
            source_platform: Platform::Discord,
            source_channel: "123".into(),
            author_name: "Pietje".into(),
            author_id: Some("1".into()),
            content: "Hallo wereld!".into(),
            reply_to: Some(ReplyContext {
                target_nick: "Klaas".into(),
                target_message_id: None,
            }),
            is_action: false,
        };

        let formatted = router.format_for_irc(&msg);
        assert_eq!(formatted, "<P\u{200B}ietje \u{21B3} Klaas> Hallo wereld!");
    }

    #[test]
    fn test_bidirectional_lru_cache() {
        let router = BridgeRouter::new(vec![], 500);
        router.record_bridge_link(
            "999888".into(),
            "#deapen".into(),
            "Kuuke".into(),
            "Even een testpuls gedaan".into(),
        );

        let irc_ref = router.lookup_irc_by_discord_id("999888").expect("Moet IRC ref vinden");
        assert_eq!(irc_ref.author, "Kuuke");
        assert_eq!(irc_ref.channel, "#deapen");

        let discord_id = router.lookup_discord_by_irc("#deapen", "Kuuke", "Even een testpuls gedaan")
            .expect("Moet Discord ID vinden");
        assert_eq!(discord_id, "999888");
    }

    #[test]
    fn test_ignore_command_prefixes() {
        let router = BridgeRouter::new(vec![], 100);
        assert!(router.should_ignore("!ai wat is dit?"));
        assert!(router.should_ignore(".help"));
        assert!(!router.should_ignore("Gewoon een gezellig bericht!"));
    }
}
