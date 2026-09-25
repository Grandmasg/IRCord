use async_trait::async_trait;
use serenity::client::{Context, EventHandler};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use tokio::sync::mpsc::Sender;
use tracing::{debug, info};

use crate::bridge::{BridgeMessage, Platform, ReplyContext};
use crate::config::Config;
use std::sync::Arc;

pub struct DiscordHandler {
    config: Arc<Config>,
    inbound_tx: Sender<BridgeMessage>,
}

impl DiscordHandler {
    pub fn new(config: Arc<Config>, inbound_tx: Sender<BridgeMessage>) -> Self {
        Self { config, inbound_tx }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, _: Context, ready: Ready) {
        info!("Discord bot succesvol ingelogd als {}!", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // 1. Voorkom loops: negeer eigen berichten, bots en webhooks
        if msg.author.bot || msg.webhook_id.is_some() {
            return;
        }

        let channel_id = msg.channel_id.get();

        // 2. Controleer of dit kanaal gekoppeld is in config.toml
        let is_linked = self.config.channels.iter().any(|m| m.discord_channel_id == channel_id);
        if !is_linked {
            return;
        }

        debug!("Inkomend Discord bericht van {}: {}", msg.author.name, msg.content);

        // 3. Detecteer optionele Discord native Reply
        let mut reply_context = None;
        if let Some(ref referenced) = msg.referenced_message {
            reply_context = Some(ReplyContext {
                target_nick: referenced.author.name.clone(),
                target_message_id: Some(referenced.id.to_string()),
            });
        }

        // 4. Verwerk eventuele bijlagen (afbeeldingen, video's)
        let attachments: Vec<String> = msg.attachments.iter().map(|a| a.url.clone()).collect();
        let full_content = if attachments.is_empty() {
            msg.content.clone()
        } else {
            format!("{} {}", msg.content, attachments.join(" ")).trim().to_string()
        };

        let bridge_msg = BridgeMessage {
            source_platform: Platform::Discord,
            source_channel: channel_id.to_string(),
            author_name: msg.author.name.clone(),
            author_id: Some(msg.author.id.to_string()),
            content: full_content,
            reply_to: reply_context,
            is_action: false,
        };

        let _ = self.inbound_tx.send(bridge_msg).await;
    }
}
