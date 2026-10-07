use async_trait::async_trait;
use serenity::client::{Context, EventHandler};
use serenity::model::channel::{GuildChannel, Message, Reaction, ReactionType};
use serenity::model::event::MessageUpdateEvent;
use serenity::model::id::{ChannelId, GuildId, MessageId};
use serenity::model::gateway::Ready;
use tokio::sync::mpsc::Sender;
use tracing::{debug, info};

use crate::bridge::mentions::{compose_discord_content, discord_to_irc_mentions};
use crate::bridge::{BridgeMessage, DiscordEvent, Platform, ReplyContext};
use crate::config::Config;
use std::collections::HashMap;
use std::sync::Arc;

pub struct DiscordHandler {
    config: Arc<Config>,
    inbound_tx: Sender<BridgeMessage>,
    event_tx: Sender<DiscordEvent>,
}

impl DiscordHandler {
    pub fn new(config: Arc<Config>, inbound_tx: Sender<BridgeMessage>, event_tx: Sender<DiscordEvent>) -> Self {
        Self { config, inbound_tx, event_tx }
    }

    fn is_linked(&self, channel_id: u64) -> bool {
        self.config.channels.iter().any(|m| m.discord_channel_id == channel_id)
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

        // 4. Mentions naar namen, bijlagen, stickers en embeds naar tekst (IRC kent alleen tekst)
        let users: HashMap<String, String> = msg
            .mentions
            .iter()
            .map(|u| (u.id.to_string(), u.global_name.clone().unwrap_or_else(|| u.name.clone())))
            .collect();
        let (roles, channels): (HashMap<String, String>, HashMap<String, String>) = match msg.guild(&ctx.cache) {
            Some(g) => (
                g.roles.iter().map(|(id, r)| (id.to_string(), r.name.clone())).collect(),
                g.channels.iter().map(|(id, c)| (id.to_string(), c.name.clone())).collect(),
            ),
            None => (HashMap::new(), HashMap::new()),
        };
        let text = discord_to_irc_mentions(&msg.content, &users, &roles, &channels);
        let attachments: Vec<String> = msg.attachments.iter().map(|a| a.url.clone()).collect();
        let stickers: Vec<String> = msg.sticker_items.iter().map(|s| s.name.clone()).collect();
        let embeds: Vec<(Option<String>, Option<String>)> = msg.embeds.iter().map(|e| (e.title.clone(), e.url.clone())).collect();
        let full_content = compose_discord_content(&text, &attachments, &stickers, &embeds);
        if full_content.is_empty() {
            return;
        }

        let bridge_msg = BridgeMessage {
            source_platform: Platform::Discord,
            source_channel: channel_id.to_string(),
            author_name: msg.author.name.clone(),
            author_id: Some(msg.author.id.to_string()),
            message_id: Some(msg.id.to_string()),
            content: full_content,
            reply_to: reply_context,
            is_action: false,
        };

        let _ = self.inbound_tx.send(bridge_msg).await;
    }

    async fn message_update(&self, _ctx: Context, _old: Option<Message>, _new: Option<Message>, event: MessageUpdateEvent) {
        if !self.config.bridge.sync_edits {
            return;
        }
        // Bots en webhooks (inclusief onze eigen relay) negeren
        if event.author.as_ref().map(|a| a.bot).unwrap_or(false) {
            return;
        }
        let channel_id = event.channel_id.get();
        let Some(new_content) = event.content else { return };
        if !self.is_linked(channel_id) {
            return;
        }
        let _ = self
            .event_tx
            .send(DiscordEvent::Edited { channel_id, message_id: event.id.to_string(), new_content })
            .await;
    }

    async fn message_delete(&self, _ctx: Context, channel_id: ChannelId, deleted_message_id: MessageId, _guild_id: Option<GuildId>) {
        if !self.config.bridge.sync_edits || !self.is_linked(channel_id.get()) {
            return;
        }
        let _ = self
            .event_tx
            .send(DiscordEvent::Deleted { channel_id: channel_id.get(), message_id: deleted_message_id.to_string() })
            .await;
    }

    async fn reaction_add(&self, ctx: Context, add_reaction: Reaction) {
        if !self.config.bridge.sync_reactions || !self.is_linked(add_reaction.channel_id.get()) {
            return;
        }
        let Ok(user) = add_reaction.user(&ctx).await else { return };
        if user.bot {
            return;
        }
        let emoji = match &add_reaction.emoji {
            ReactionType::Unicode(s) => s.clone(),
            ReactionType::Custom { name, .. } => format!(":{}:", name.clone().unwrap_or_else(|| "emoji".into())),
            _ => return,
        };
        let _ = self
            .event_tx
            .send(DiscordEvent::Reaction {
                channel_id: add_reaction.channel_id.get(),
                message_id: add_reaction.message_id.to_string(),
                user: user.global_name.clone().unwrap_or_else(|| user.name.clone()),
                emoji,
            })
            .await;
    }

    async fn channel_update(&self, _ctx: Context, old: Option<GuildChannel>, new: GuildChannel) {
        if !self.config.bridge.sync_topic || !self.is_linked(new.id.get()) {
            return;
        }
        // Alleen als het onderwerp echt veranderd is (kanaal-updates gaan ook over namen, rechten, enz.)
        if old.as_ref().map(|o| o.topic == new.topic).unwrap_or(false) {
            return;
        }
        let _ = self
            .event_tx
            .send(DiscordEvent::Topic { channel_id: new.id.get(), topic: new.topic.clone().unwrap_or_default() })
            .await;
    }
}
