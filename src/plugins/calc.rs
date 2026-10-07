use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::calc::{evaluate, format_number};
use async_trait::async_trait;

/// `!calc <som>`: veilige rekenmachine (zie utils::calc).
pub struct CalcPlugin;

#[async_trait]
impl Plugin for CalcPlugin {
    fn name(&self) -> &'static str { "calc" }
    fn triggers(&self) -> &[&'static str] { &["calc", "bereken", "calculate"] }
    fn help(&self) -> &'static str {
        "!calc <som> - bijv. !calc (12+3)*4 | !calc sqrt(2)*pi | functies: sqrt, sin, cos, tan, ln, log, abs, round, floor, ceil | + - * / % ^"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let expr = cmd.args.trim();
        if expr.is_empty() {
            return Ok(Some(if ctx.locale.is_dutch() {
                "🧮 Gebruik: !calc <som>, bijvoorbeeld !calc (12+3)*4 of !calc sqrt(2)*pi".into()
            } else {
                "🧮 Usage: !calc <expression>, e.g. !calc (12+3)*4 or !calc sqrt(2)*pi".into()
            }));
        }
        Ok(Some(match evaluate(expr) {
            Ok(v) => format!("🧮 {} = \x02{}\x02", expr, format_number(v)),
            Err(e) => format!("🧮 ⚠️ {}", e),
        }))
    }
}
