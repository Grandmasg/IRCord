use super::{BridgeMessage, PresenceEvent, Platform};
use crate::config::ChannelMapping;
use crate::utils::sanitizer::{anti_ping_nick, sanitize_discord_emojis, sanitize_for_irc, strip_mirc_codes};
use lru::LruCache;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
#[allow(dead_code)] // channel/content/timestamp worden bewaard voor toekomstige echo-dedup en reply-sync
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
    #[allow(dead_code)] // productiecode gebruikt with_prefixes
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
            let mut d2i = self.discord_to_irc.lock().unwrap_or_else(|e| e.into_inner());
            d2i.put(discord_id.clone(), irc_ref);
        }

        {
            let mut i2d = self.irc_to_discord.lock().unwrap_or_else(|e| e.into_inner());
            i2d.put(hash, discord_id);
        }
    }

    /// Zoekt de IRC context op basis van een Discord Message ID (bijv. bij een reply of emoji-reactie)
    pub fn lookup_irc_by_discord_id(&self, discord_id: &str) -> Option<IrcMessageRef> {
        let mut cache = self.discord_to_irc.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(discord_id).cloned()
    }

    /// Zoekt het Discord Message ID op basis van een IRC bericht (nu alleen door tests gebruikt; voor echo-dedup)
    #[allow(dead_code)]
    pub fn lookup_discord_by_irc(&self, channel: &str, author: &str, content: &str) -> Option<String> {
        let hash = Self::compute_message_hash(channel, author, content);
        let mut cache = self.irc_to_discord.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&hash).cloned()
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
        trimmed.starts_with('~') || trimmed.starts_with('/')
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

    /// Melding voor IRC dat een Discord-bericht is bewerkt.
    pub fn format_edit_for_irc(&self, author: &str, new_content: &str) -> String {
        let clean = sanitize_for_irc(&sanitize_discord_emojis(new_content));
        format!("\u{270F}\u{FE0F} <{}> (bewerkt): {}", anti_ping_nick(author), clean)
    }

    /// Meest recente gebrugde bericht van een auteur in een IRC-kanaal (voor reply-citaten).
    pub fn find_recent_by_author(&self, irc_channel: &str, author: &str) -> Option<IrcMessageRef> {
        let cache = self.discord_to_irc.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .iter()
            .map(|(_, r)| r)
            .find(|r| r.channel.eq_ignore_ascii_case(irc_channel) && r.author.eq_ignore_ascii_case(author))
            .cloned()
    }

    /// Melding voor IRC dat iemand op Discord op een bericht reageerde.
    pub fn format_reaction_for_irc(&self, user: &str, emoji: &str, author: &str) -> String {
        format!("\u{2B50} {} reageerde met {} op het bericht van {}", anti_ping_nick(user), emoji, anti_ping_nick(author))
    }

    /// Melding voor IRC dat een Discord-bericht is verwijderd (de inhoud wordt bewust niet herhaald).
    pub fn format_delete_for_irc(&self, author: &str) -> String {
        format!("\u{1F5D1}\u{FE0F} {} heeft een bericht verwijderd", anti_ping_nick(author))
    }

    /// Zet een IRC-aanwezigheidsgebeurtenis om naar `(kanaal, tekst)` voor Discord; `None` = niets te melden.
    pub fn format_presence_for_discord(&self, ev: &PresenceEvent) -> Option<(Option<String>, String)> {
        match ev {
            PresenceEvent::Join { nick, platform: Platform::Irc, channel } => {
                Some((Some(channel.clone()), format!("\u{27A1}\u{FE0F} **{}** heeft {} betreden", nick, channel)))
            }
            PresenceEvent::Part { nick, platform: Platform::Irc, channel, reason } => {
                let why = reason.as_deref().filter(|r| !r.is_empty()).map(|r| format!(" ({})", r)).unwrap_or_default();
                Some((Some(channel.clone()), format!("\u{2B05}\u{FE0F} **{}** heeft {} verlaten{}", nick, channel, why)))
            }
            PresenceEvent::Quit { nick, platform: Platform::Irc, channel, reason } => {
                let why = reason.as_deref().filter(|r| !r.is_empty()).map(|r| format!(" ({})", r)).unwrap_or_default();
                Some((channel.clone(), format!("\u{1F50C} **{}** heeft IRC verlaten{}", nick, why)))
            }
            _ => None,
        }
    }

    /// Formatteert een binnenkomend IRC-bericht voor weergave via een Discord Webhook
    pub fn format_for_discord_webhook(&self, msg: &BridgeMessage) -> (String, String) {
        let username = msg.author_name.clone();
        let clean_content = strip_mirc_codes(&msg.content);
        (username, clean_content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::ReplyContext;

    #[test]
    fn test_format_for_irc() {
        let mappings = vec![ChannelMapping {
            irc_channel: "#test".into(),
            discord_channel_id: 123,
            discord_webhook_url: "https://discord.com/...".into(),
            language: None,
            disabled_plugins: vec![],
        }];
        let router = BridgeRouter::new(mappings, 100);

        let msg = BridgeMessage {
            source_platform: Platform::Discord,
            source_channel: "123".into(),
            author_name: "Pietje".into(),
            author_id: Some("1".into()),
            message_id: None,
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
            "TestUser".into(),
            "Even een testpuls gedaan".into(),
        );

        let irc_ref = router.lookup_irc_by_discord_id("999888").expect("Moet IRC ref vinden");
        assert_eq!(irc_ref.author, "TestUser");
        assert_eq!(irc_ref.channel, "#deapen");

        let discord_id = router.lookup_discord_by_irc("#deapen", "TestUser", "Even een testpuls gedaan")
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

    #[test]
    fn edit_delete_and_presence_formatting() {
        let router = BridgeRouter::new(vec![], 100);
        let edit = router.format_edit_for_irc("Pietje", "nieuwe tekst\nregel 2");
        assert!(edit.contains("(bewerkt): nieuwe tekst regel 2"), "{edit}");
        let del = router.format_delete_for_irc("Pietje");
        assert!(del.ends_with("heeft een bericht verwijderd") && !del.contains("tekst"));

        let join = PresenceEvent::Join { nick: "henk".into(), platform: Platform::Irc, channel: "#a".into() };
        assert_eq!(router.format_presence_for_discord(&join).unwrap().0.as_deref(), Some("#a"));
        let quit = PresenceEvent::Quit { nick: "henk".into(), platform: Platform::Irc, channel: None, reason: Some("Ping timeout".into()) };
        let (chan, text) = router.format_presence_for_discord(&quit).unwrap();
        assert!(chan.is_none() && text.contains("(Ping timeout)"), "{text}");
        let discord = PresenceEvent::Join { nick: "x".into(), platform: Platform::Discord, channel: "1".into() };
        assert!(router.format_presence_for_discord(&discord).is_none());
    }

    #[test]
    fn recent_by_author_and_reaction_format() {
        let router = BridgeRouter::new(vec![], 100);
        router.record_bridge_link("1".into(), "#a".into(), "Bob".into(), "eerste".into());
        router.record_bridge_link("2".into(), "#a".into(), "Bob".into(), "tweede".into());
        router.record_bridge_link("3".into(), "#b".into(), "Bob".into(), "ander kanaal".into());
        assert_eq!(router.find_recent_by_author("#A", "bob").unwrap().content, "tweede");
        assert!(router.find_recent_by_author("#a", "niemand").is_none());
        let r = router.format_reaction_for_irc("Kim", "\u{1F44D}", "Bob");
        assert!(r.contains("reageerde met") && r.contains("op het bericht van"), "{r}");
    }
}
