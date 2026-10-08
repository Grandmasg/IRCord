use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::calc::format_number;
use crate::utils::units::{convert, parse_request, round_sig};
use async_trait::async_trait;

/// `!convert <getal> <van> <naar>`: eenheden omrekenen (zie utils::units).
pub struct ConvertPlugin;

#[async_trait]
impl Plugin for ConvertPlugin {
    fn name(&self) -> &'static str { "convert" }
    fn triggers(&self) -> &[&'static str] { &["convert", "omrekenen", "conv"] }
    fn help(&self) -> &'static str {
        "!convert <getal> <van> <naar> - bijv. !convert 5 km mi, !convert 100 c f, !convert 3 ons g | lengte, gewicht, inhoud, snelheid, oppervlakte, data, tijd, druk, temperatuur"
    }

    async fn on_command(&self, _ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let result = parse_request(&cmd.args).and_then(|(v, from, to)| {
            convert(v, &from, &to).map(|r| format!("{} {} = \x02{} {}\x02", format_number(v), from, format_number(round_sig(r, 6)), to))
        });
        Ok(Some(match result {
            Ok(s) => format!("📐 {}", s),
            Err(e) => format!("📐 ⚠️ {}", e),
        }))
    }
}
