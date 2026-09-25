use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct MinecraftPlugin;

#[derive(Deserialize)]
struct McStatusResponse {
    online: bool,
    host: Option<String>,
    port: Option<u16>,
    version: Option<McVersion>,
    players: Option<McPlayers>,
    motd: Option<McMotd>,
}

#[derive(Deserialize)]
struct McVersion {
    name_clean: Option<String>,
    name_raw: Option<String>,
}

#[derive(Deserialize)]
struct McPlayers {
    online: Option<u64>,
    max: Option<u64>,
}

#[derive(Deserialize)]
struct McMotd {
    clean: Option<String>,
}

#[async_trait]
impl Plugin for MinecraftPlugin {
    fn name(&self) -> &'static str {
        "minecraft"
    }

    fn triggers(&self) -> &[&'static str] {
        &["mc", "minecraft", "mcserver"]
    }

    fn help(&self) -> &'static str {
        "!mc <server-adres> - Controleert de status en online spelers van een Minecraft server"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let server = cmd.args.trim();
        if server.is_empty() {
            return Ok(Some(ctx.locale.t("minecraft_usage").into()));
        }

        let clean_address = server.replace("http://", "").replace("https://", "").replace('/', "");
        let url = format!("https://api.mcstatus.io/v2/status/java/{}", clean_address);

        let resp = ctx
            .http
            .get(&url)
            .header("User-Agent", "IRCordBot/1.0 (minecraft monitor)")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Some(format!(
                "🎮 {}",
                ctx.locale.tf("minecraft_error", &[("server", &clean_address)])
            )));
        }

        let data: McStatusResponse = resp.json().await?;
        let mc_title = ctx.locale.t("minecraft_title");

        if !data.online {
            return Ok(Some(format!(
                "🎮 [{}] {} is {}",
                mc_title,
                clean_address,
                ctx.locale.t("minecraft_offline")
            )));
        }

        let host_display = data.host.unwrap_or(clean_address);
        let players_online = data.players.as_ref().and_then(|p| p.online).unwrap_or(0);
        let players_max = data.players.as_ref().and_then(|p| p.max).unwrap_or(0);

        let version = data
            .version
            .and_then(|v| v.name_clean.or(v.name_raw))
            .unwrap_or_else(|| ctx.locale.t("minecraft_unknown").to_string());

        let clean_motd = data
            .motd
            .and_then(|m| m.clean)
            .map(|m| {
                let single = m.replace('\n', " ").trim().to_string();
                if single.chars().count() > 80 {
                    let mut cut: String = single.chars().take(77).collect();
                    cut.push_str("...");
                    cut
                } else {
                    single
                }
            })
            .unwrap_or_default();

        let motd_part = if !clean_motd.is_empty() {
            format!(" | MOTD: {}", clean_motd)
        } else {
            String::new()
        };

        let online_label = ctx.locale.t("minecraft_online");
        let players_label = ctx.locale.t("minecraft_players");
        let version_label = ctx.locale.t("minecraft_version");

        Ok(Some(format!(
            "🎮 [{}] {} is {} | {}: {}/{} | {}: {}{}",
            mc_title, host_display, online_label, players_label, players_online, players_max, version_label, version, motd_part
        )))
    }
}
