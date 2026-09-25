use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static SED_REGEX: OnceLock<Regex> = OnceLock::new();

pub struct SedPlugin {
    last_messages: Mutex<HashMap<String, String>>, // Nick -> Laatste bericht
}

impl SedPlugin {
    pub fn new() -> Self {
        Self {
            last_messages: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Plugin for SedPlugin {
    fn name(&self) -> &'static str { "sed" }
    fn help(&self) -> &'static str { "s/oud/nieuw/ - Corrigeert een typefout in je vorige bericht" }

    async fn on_message(&self, _ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let re = SED_REGEX.get_or_init(|| {
            Regex::new(r"^s/([^/]+)/([^/]*)/?([gG]?)$").unwrap()
        });

        let trimmed = msg.content.trim();

        if let Some(caps) = re.captures(trimmed) {
            let from = caps.get(1).map_or("", |m| m.as_str());
            let to = caps.get(2).map_or("", |m| m.as_str());
            let flags = caps.get(3).map_or("", |m| m.as_str());

            let mut lock = self.last_messages.lock().unwrap();
            if let Some(last) = lock.get(&msg.author.to_lowercase()) {
                let corrected = if flags.contains('g') || flags.contains('G') {
                    last.replace(from, to)
                } else {
                    last.replacen(from, to, 1)
                };

                if &corrected != last {
                    return Ok(Some(format!("✏️ <{}> {}", msg.author, corrected)));
                }
            }
        } else if !trimmed.starts_with('!') && !trimmed.starts_with('.') {
            // Sla het laatste reguliere bericht op
            let mut lock = self.last_messages.lock().unwrap();
            lock.insert(msg.author.to_lowercase(), msg.content.clone());
        }

        Ok(None)
    }
}
