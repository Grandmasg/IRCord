pub mod router;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Irc,
    Discord,
}

#[derive(Debug, Clone)]
pub struct BridgeMessage {
    pub source_platform: Platform,
    pub source_channel: String,
    pub author_name: String,
    pub author_id: Option<String>,
    /// Platform-specifiek bericht-ID (Discord message ID); nodig om bewerkingen/verwijderingen te koppelen.
    pub message_id: Option<String>,
    pub content: String,
    pub reply_to: Option<ReplyContext>,
    pub is_action: bool, // Bijv. /me of CTCP ACTION
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // target_message_id is bedoeld voor reply-sync
pub struct ReplyContext {
    pub target_nick: String,
    pub target_message_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PresenceEvent {
    Join {
        nick: String,
        platform: Platform,
        channel: String,
    },
    Part {
        nick: String,
        platform: Platform,
        channel: String,
        reason: Option<String>,
    },
    Quit {
        nick: String,
        platform: Platform,
        /// Laatst bekende kanaal van deze nick (uit JOIN of een chatbericht), indien bekend.
        channel: Option<String>,
        reason: Option<String>,
    },
}

/// Gebeurtenissen vanaf Discord die geen gewoon chatbericht zijn.
#[derive(Debug, Clone)]
pub enum DiscordEvent {
    Edited { channel_id: u64, message_id: String, new_content: String },
    Deleted { channel_id: u64, message_id: String },
}
