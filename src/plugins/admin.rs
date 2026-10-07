use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::time::Instant;
use std::sync::OnceLock;

static START_TIME: OnceLock<Instant> = OnceLock::new();

pub struct AdminPlugin;

impl AdminPlugin {
    /// Legt het starttijdstip van de daemon vast. Moet bij het opstarten worden aangeroepen,
    /// anders telt de uptime pas vanaf het eerste statuscommando.
    pub fn init_start_time() {
        START_TIME.get_or_init(Instant::now);
    }
}

/// Formatteert seconden als "3d 4u 12m 5s" (nul-eenheden vooraan worden weggelaten).
fn format_duration(total: u64) -> String {
    let (d, h, m, s) = (total / 86400, (total % 86400) / 3600, (total % 3600) / 60, total % 60);
    if d > 0 {
        format!("{}d {}u {}m {}s", d, h, m, s)
    } else if h > 0 {
        format!("{}u {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// Resident geheugen van dit proces in MB (alleen Linux/containers).
fn resident_memory_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.split_whitespace().next()?.parse::<u64>().ok()?;
    Some(kb / 1024)
}

#[async_trait]
impl Plugin for AdminPlugin {
    fn name(&self) -> &'static str { "admin" }
    fn triggers(&self) -> &[&'static str] { &["status", "stats", "ping", "uptime", "errors", "errorlog", "logs"] }
    fn help(&self) -> &'static str { "!status - Toont status | !uptime - Draaitijd en geheugen | !errors [aantal|clear] - Toont recente fouten" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if cmd.trigger == "ping" {
            return Ok(Some("Pong! 🏓 Daemon actief.".into()));
        }

        if cmd.trigger == "uptime" {
            let uptime = format_duration(START_TIME.get_or_init(Instant::now).elapsed().as_secs());
            let plugin_count = ctx.plugins_info.read().map(|p| p.len()).unwrap_or(0);
            let mem = resident_memory_mb().map(|m| format!(" | RAM: {} MB", m)).unwrap_or_default();
            return Ok(Some(format!("⏱️ [Uptime] {} | Plugins: {} | Fouten in buffer: {}{}", uptime, plugin_count, ctx.error_logger.count(), mem)));
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
        let uptime = format_duration(start.elapsed().as_secs());

        let ai_online = ctx.ai_client.ping().await;
        let ai_status_str = if ai_online { "Online ✅" } else { "Offline ❌" };

        let channels_count = ctx.config.channels.len();
        let current_model = ctx.ai_manager.get_model();

        Ok(Some(format!(
            "🤖 [IRCord Status] Uptime: {} | Kanalen: {} | AI: {} ({}) | Team: {}",
            uptime, channels_count, ai_status_str, current_model, ctx.config.whatpulse.team_name
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3 * 3600 + 61), "3u 1m 1s");
        assert_eq!(format_duration(2 * 86400 + 3600), "2d 1u 0m 0s");
    }
}
