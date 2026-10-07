use super::toggles::PROTECTED_PLUGINS;
use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

/// `!plugin list|enable|disable <naam>`: plugins per kanaal beheren (operators).
pub struct PluginAdminPlugin;

#[async_trait]
impl Plugin for PluginAdminPlugin {
    fn name(&self) -> &'static str { "plugins" }
    fn triggers(&self) -> &[&'static str] { &["plugin", "plugins"] }
    fn help(&self) -> &'static str {
        "!plugin list - status in dit kanaal | !plugin disable <naam> / !plugin enable <naam> - alleen operators"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut parts = cmd.args.split_whitespace();
        let action = parts.next().unwrap_or("list").to_lowercase();
        let names: Vec<String> = ctx
            .plugins_info
            .read()
            .map(|p| p.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default();

        match action.as_str() {
            "list" => {
                let off: Vec<&String> = names.iter().filter(|n| ctx.toggles.is_disabled(&cmd.channel, n)).collect();
                let off_str = if off.is_empty() { "geen".to_string() } else { off.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") };
                Ok(Some(format!(
                    "🧩 [Plugins] {} geladen | Uitgeschakeld in dit kanaal: {} | Beheer: !plugin disable|enable <naam>",
                    names.len(),
                    off_str
                )))
            }
            "enable" | "disable" => {
                if !cmd.is_owner && !cmd.is_operator {
                    return Ok(Some("⛔ Alleen operators kunnen plugins in- of uitschakelen.".into()));
                }
                let Some(name) = parts.next().map(|s| s.to_lowercase()) else {
                    return Ok(Some("Gebruik: !plugin disable <naam> | !plugin enable <naam>".into()));
                };
                if !names.iter().any(|n| n == &name) {
                    return Ok(Some(format!("⚠️ Onbekende plugin '{}'. Zie: !plugin list", name)));
                }
                if PROTECTED_PLUGINS.contains(&name.as_str()) {
                    return Ok(Some(format!("🔒 De plugin '{}' kan niet worden uitgeschakeld.", name)));
                }
                let disable = action == "disable";
                ctx.toggles.set_disabled(&ctx.db, &ctx.config, &cmd.channel, &name, disable).await;
                Ok(Some(if disable {
                    format!("🔕 Plugin '{}' is uitgeschakeld in dit kanaal.", name)
                } else {
                    format!("🔔 Plugin '{}' is weer ingeschakeld in dit kanaal.", name)
                }))
            }
            _ => Ok(Some("Gebruik: !plugin list | !plugin disable <naam> | !plugin enable <naam>".into())),
        }
    }
}
