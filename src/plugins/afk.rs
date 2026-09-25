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

    async fn on_command(&self, _ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let reason = if cmd.args.trim().is_empty() {
            "even afwezig".to_string()
        } else {
            cmd.args.trim().to_string()
        };

        {
            let mut afk = self.afk_users.lock().unwrap();
            afk.insert(cmd.author.to_lowercase(), (reason.clone(), Instant::now()));
        }

        Ok(Some(format!("💤 {} is nu AFK: {}", cmd.author, reason)))
    }

    async fn on_message(&self, _ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let author_key = msg.author.to_lowercase();

        // 1. Als de gebruiker zelf weer typt, verwijder AFK status
        {
            let mut afk = self.afk_users.lock().unwrap();
            if let Some((_, start)) = afk.remove(&author_key) {
                let mins = start.elapsed().as_secs() / 60;
                return Ok(Some(format!("👋 Welkom terug {}, je was {} minuten AFK.", msg.author, mins)));
            }
        }

        // 2. Controleer of iemand een AFK-gebruiker mentiont
        let words: Vec<&str> = msg.content.split_whitespace().collect();
        let afk = self.afk_users.lock().unwrap();
        for word in words {
            let clean_word = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            if let Some((reason, start)) = afk.get(&clean_word) {
                let mins = start.elapsed().as_secs() / 60;
                return Ok(Some(format!("💤 [{}] is AFK sinds {} minuten geleden (Reden: {})", word, mins, reason)));
            }
        }

        Ok(None)
    }
}
