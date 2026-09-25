use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::time::Instant;
use std::sync::OnceLock;

static START_TIME: OnceLock<Instant> = OnceLock::new();

pub struct AdminPlugin;

impl AdminPlugin {
    pub fn new() -> Self {
        START_TIME.get_or_init(Instant::now);
        Self
    }
}

#[async_trait]
impl Plugin for AdminPlugin {
    fn name(&self) -> &'static str { "admin" }
    fn triggers(&self) -> &[&'static str] { &["status", "stats", "ping", "errors", "errorlog", "logs"] }
    fn help(&self) -> &'static str { "!status - Toont uptime en status | !errors [aantal|clear] - Toont recente fouten" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if cmd.trigger == "ping" {
            return Ok(Some("Pong! 🏓 Daemon actief.".into()));
        }

        // Foutendiagnose & error logging
        if cmd.trigger == "errors" || cmd.trigger == "errorlog" || cmd.trigger == "logs" {
            if !cmd.is_owner && !cmd.is_operator {
                return Ok(Some("⛔ Geen toegang. Alleen bot operators kunnen het foutenlogboek inzien.".into()));
            }

            // Controleer of de aanroep plaatsvindt in een PM of in het geconfigureerde admin-kanaal
            let is_pm = !cmd.channel.starts_with('#');
            let is_admin_channel = if cmd.platform == "irc" {
                cmd.channel.eq_ignore_ascii_case(&ctx.config.general.admin_channel_irc)
            } else {
                cmd.channel == ctx.config.general.admin_channel_discord_id.to_string()
            };

            if !is_pm && !is_admin_channel {
                return Ok(Some(format!(
                    "🔒 [Admin] Om kanaalrust te bewaren is het foutenlogboek alleen direct in te zien in \x02{}\x02 of stuur mij een privébericht (PM): \x02!errors\x02.",
                    ctx.config.general.admin_channel_irc
                )));
            }

            let arg = cmd.args.trim().to_lowercase();
            if arg == "clear" {
                ctx.error_logger.clear();
                return Ok(Some("🧹 Foutenlogboek succesvol geleegd.".into()));
            }

            let count: usize = arg.parse().unwrap_or(3).clamp(1, 10);
            let entries = ctx.error_logger.recent(count);

            if entries.is_empty() {
                return Ok(Some("✅ Geen recente fouten of waarschuwingen in het logboek.".into()));
            }

            let mut lines = Vec::new();
            lines.push(format!("📋 [Foutenlogboek] Laatste {} fouten (totaal: {} in buffer):", entries.len(), ctx.error_logger.count()));
            for e in entries {
                lines.push(format!(
                    "• [{}] {} ({}): {}",
                    e.level,
                    e.source,
                    e.timestamp.format("%H:%M:%S"),
                    e.message
                ));
            }

            return Ok(Some(lines.join("\n")));
        }

        let start = START_TIME.get_or_init(Instant::now);
        let uptime_secs = start.elapsed().as_secs();
        let hours = uptime_secs / 3600;
        let mins = (uptime_secs % 3600) / 60;
        let secs = uptime_secs % 60;

        let ai_online = ctx.ai_client.ping().await;
        let ai_status_str = if ai_online { "Online ✅" } else { "Offline ❌" };

        let channels_count = ctx.config.channels.len();
        let current_model = ctx.ai_manager.get_model();

        Ok(Some(format!(
            "🤖 [IRCord Status] Uptime: {}u {}m {}s | Kanalen: {} | AI: {} ({}) | Team: {}",
            hours, mins, secs, channels_count, ai_status_str, current_model, ctx.config.whatpulse.team_name
        )))
    }
}
