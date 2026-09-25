use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

pub struct AfkPlugin {
    afk_users: Mutex<HashMap<String, (String, Instant)>>, // Nick -> (Reden, Tijdstip)
}

impl AfkPlugin {
    pub fn new() -> Self {
        Self {
            afk_users: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Plugin for AfkPlugin {
    fn name(&self) -> &'static str { "afk" }
    fn triggers(&self) -> &[&'static str] { &["afk"] }
    fn help(&self) -> &'static str { "!afk [reden] - Meld jezelf afwezig" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let reason = if cmd.args.trim().is_empty() {
            ctx.locale.t("afk_default_reason").to_string()
        } else {
            cmd.args.trim().to_string()
        };

        {
            let mut afk = self.afk_users.lock().unwrap();
            afk.insert(cmd.author.to_lowercase(), (reason.clone(), Instant::now()));
        }

        Ok(Some(ctx.locale.tf("afk_set", &[("author", &cmd.author), ("reason", &reason)])))
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let author_key = msg.author.to_lowercase();

        // 1. If the user chats again, clear AFK status
        {
            let mut afk = self.afk_users.lock().unwrap();
            if let Some((_, start)) = afk.remove(&author_key) {
                let mins = start.elapsed().as_secs() / 60;
                let mins_str = mins.to_string();
                return Ok(Some(ctx.locale.tf("afk_welcome_back", &[("author", &msg.author), ("mins", &mins_str)])));
            }
        }

        // 2. Check if someone mentions an AFK user
        let words: Vec<&str> = msg.content.split_whitespace().collect();
        let afk = self.afk_users.lock().unwrap();
        for word in words {
            let clean_word = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            if let Some((reason, start)) = afk.get(&clean_word) {
                let mins = start.elapsed().as_secs() / 60;
                let mins_str = mins.to_string();
                return Ok(Some(ctx.locale.tf("afk_mention", &[("user", word), ("mins", &mins_str), ("reason", reason)])));
            }
        }

        Ok(None)
    }
}
