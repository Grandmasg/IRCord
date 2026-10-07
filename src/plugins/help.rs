use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct HelpPlugin;

#[async_trait]
impl Plugin for HelpPlugin {
    fn name(&self) -> &'static str { "help" }
    fn triggers(&self) -> &[&'static str] { &["help", "commands", "cmd", "cmds", "commando", "commando's"] }
    fn help(&self) -> &'static str { "!help [commando] - Toont het commandolijstje of gedetailleerde help over een specifiek commando" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let arg = cmd.args.trim().trim_start_matches('!').trim_start_matches('.').to_lowercase();

        if arg.is_empty() {
            // Overzicht per categorie
            return Ok(Some(
                "📖 \x02[IRCord Help]\x02 Beschikbare categorieën & commando's:\n\
                • 🧠 \x02AI & Taal:\x02 !ai, !rephrase, !tr, !lang\n\
                • 🌦️ \x02Weer & Tijd:\x02 !weer, !tijd, !countdown, !kerst\n\
                • 🎮 \x02Spel & Fun:\x02 !roulette, !8ball, !roll, !flip, !choose, !slap\n\
                • 👤 \x02Profiel & Stats:\x02 !profiel, !top, !karma, !whatpulse, !birthday, !seen, !afk\n\
                • 🛡️ \x02Moderatie:\x02 !kick, !ban, !kb, !op, !deop, !voice, !devoice, !topic\n\
                • ℹ️ \x02Info & Web:\x02 !wiki, !google, !youtube, !crypto, !urban, !mc\n\
                • ⚙️ \x02Systeem:\x02 !status, !uptime, !peak, !errors, !sysadmin, !tech, !poll, !remind, !tell, !track\n\
                \x0314Typ '!help <commando>' voor gedetailleerde syntax (bijv. !help weer of !help roulette).\x03".into()
            ));
        }

        // Zoek specifiek commando op in geregistreerde plugins
        if let Ok(plugins) = ctx.plugins_info.read() {
            for p in plugins.iter() {
                if p.triggers.iter().any(|t| t.eq_ignore_ascii_case(&arg)) {
                    return Ok(Some(format!(
                        "📖 \x02[Help: !{}]\x02 {}\n• Aliassen: {}",
                        arg,
                        p.help,
                        p.triggers.join(", ")
                    )));
                }
            }
        }

        Ok(Some(format!(
            "❓ Commando '!{}' niet gevonden. Typ \x02!help\x02 voor het complete categorie-overzicht.",
            arg
        )))
    }
}
