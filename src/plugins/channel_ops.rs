use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct ChannelOpsPlugin;

#[async_trait]
impl Plugin for ChannelOpsPlugin {
    fn name(&self) -> &'static str { "channel_ops" }
    fn triggers(&self) -> &[&'static str] {
        &["kick", "ban", "kb", "kickban", "op", "deop", "voice", "devoice", "topic"]
    }
    fn help(&self) -> &'static str {
        "!kick <nick> [reden] | !ban <nick|mask> | !kb <nick> [reden] | !op <nick> | !deop <nick> | !voice <nick> | !devoice <nick> | !topic <tekst>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Enforce permissions: Must be owner or operator
        // op/deop vereisen operator-rechten; kick/ban/voice/topic mogen ook door moderators
        let needs_operator = matches!(cmd.trigger.as_str(), "op" | "deop");
        let allowed = if needs_operator { cmd.is_owner || cmd.is_operator } else { cmd.is_owner || cmd.is_operator || cmd.is_moderator };
        if !allowed {
            return Ok(Some(if needs_operator {
                "⛔ Toegang geweigerd. Dit commando vereist operator- of bot-eigenaar-rechten.".into()
            } else {
                "⛔ Toegang geweigerd. Dit commando vereist moderator-, operator- of bot-eigenaar-rechten.".to_string()
            }));
        }

        let channel = &cmd.channel;
        if !channel.starts_with('#') {
            return Ok(Some("⛔ Kanaalbeheer kan alleen in een echt IRC-kanaal worden uitgevoerd.".into()));
        }

        let mut parts = cmd.args.trim().splitn(2, ' ');
        let target = parts.next().unwrap_or("").trim();
        let extra = parts.next().unwrap_or("").trim();

        match cmd.trigger.as_str() {
            "kick" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !kick <bijnaam> [reden]".into()));
                }
                let reason = if extra.is_empty() { "Verzocht door kanaaloperator" } else { extra };
                ctx.send_irc_raw(format!("KICK {} {} :{}", channel, target, reason)).await;
                Ok(Some(format!("👢 \x02{}\x02 is gekickt uit {}: {}", target, channel, reason)))
            }
            "ban" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !ban <bijnaam of hostmask>".into()));
                }
                let mask = if target.contains('!') || target.contains('@') {
                    target.to_string()
                } else {
                    format!("{}!*@*", target)
                };
                ctx.send_irc_raw(format!("MODE {} +b {}", channel, mask)).await;
                Ok(Some(format!("🔨 Ban geplaatst op \x02{}\x02 in {}", mask, channel)))
            }
            "kb" | "kickban" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !kb <bijnaam> [reden]".into()));
                }
                let mask = format!("{}!*@*", target);
                let reason = if extra.is_empty() { "Banned door kanaaloperator" } else { extra };
                ctx.send_irc_raw(format!("MODE {} +b {}", channel, mask)).await;
                ctx.send_irc_raw(format!("KICK {} {} :{}", channel, target, reason)).await;
                Ok(Some(format!("🔨👢 \x02{}\x02 is verbannen en gekickt uit {}: {}", target, channel, reason)))
            }
            "op" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !op <bijnaam>".into()));
                }
                ctx.send_irc_raw(format!("MODE {} +o {}", channel, target)).await;
                Ok(Some(format!("👑 Operator status (+o) verleend aan \x02{}\x02 in {}", target, channel)))
            }
            "deop" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !deop <bijnaam>".into()));
                }
                ctx.send_irc_raw(format!("MODE {} -o {}", channel, target)).await;
                Ok(Some(format!("Operator status (-o) ingetrokken voor \x02{}\x02 in {}", target, channel)))
            }
            "voice" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !voice <bijnaam>".into()));
                }
                ctx.send_irc_raw(format!("MODE {} +v {}", channel, target)).await;
                Ok(Some(format!("🎤 Voice status (+v) verleend aan \x02{}\x02 in {}", target, channel)))
            }
            "devoice" => {
                if target.is_empty() {
                    return Ok(Some("Gebruik: !devoice <bijnaam>".into()));
                }
                ctx.send_irc_raw(format!("MODE {} -v {}", channel, target)).await;
                Ok(Some(format!("Voice status (-v) ingetrokken voor \x02{}\x02 in {}", target, channel)))
            }
            "topic" => {
                let new_topic = cmd.args.trim();
                if new_topic.is_empty() {
                    return Ok(Some("Gebruik: !topic <nieuw kanaaltopic>".into()));
                }
                ctx.send_irc_raw(format!("TOPIC {} :{}", channel, new_topic)).await;
                // Log in topic_history
                let _ = sqlx::query!(
                    "INSERT INTO topic_history (channel, topic, set_by) VALUES (?, ?, ?)",
                    channel,
                    new_topic,
                    cmd.author
                )
                .execute(&ctx.db)
                .await;

                Ok(Some(format!("📌 Kanaaltopic voor {} gewijzigd naar: \x02{}\x02", channel, new_topic)))
            }
            _ => Ok(None),
        }
    }
}
