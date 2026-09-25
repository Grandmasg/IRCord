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
    pub content: String,
    pub reply_to: Option<ReplyContext>,
    pub is_action: bool, // Bijv. /me of CTCP ACTION
}

#[derive(Debug, Clone)]
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
        reason: Option<String>,
    },
}
