use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct SlapPlugin;

#[async_trait]
impl Plugin for SlapPlugin {
    fn name(&self) -> &'static str { "slap" }
    fn triggers(&self) -> &[&'static str] { &["slap", "mep"] }
    fn help(&self) -> &'static str { "!slap <user> / !mep <gebruiker> - Delivers a classic trout slap" }

    async fn on_command(&self, _ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let target = cmd.args.trim();
        let victim = if target.is_empty() {
            cmd.author.as_str()
        } else {
            target
        };

        if cmd.platform == "irc" {
            // IRC CTCP ACTION formaat
            Ok(Some(format!("\x01ACTION slaps {} around a bit with a large trout\x01", victim)))
        } else {
            // Discord markdown cursief
            Ok(Some(format!("*slaps {} around a bit with a large trout*", victim)))
        }
    }
}
