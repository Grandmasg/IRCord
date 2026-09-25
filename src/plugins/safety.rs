use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use std::sync::OnceLock;

static SECRET_PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();

pub struct SafetyPlugin;

impl SafetyPlugin {
    fn is_suspicious(url: &str) -> bool {
        let suspicious_keywords = [
            "discord-nitro", "steam-community", "free-nitro", "airdrop-claim",
            "d1scord", "steancommunity", "dlscord", "gift-nitro"
        ];

        let lower = url.to_lowercase();
        for kw in suspicious_keywords {
            if lower.contains(kw) {
                return true;
            }
        }
        false
    }

    /// Detecteert per ongeluk geplakte tokens en API-sleutels
    pub fn detect_leaked_secret(content: &str) -> Option<&'static str> {
        let patterns = SECRET_PATTERNS.get_or_init(|| {
            vec![
                ("Discord Bot Token", Regex::new(r"[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27,}").unwrap()),
                ("OpenAI / LLM API Key", Regex::new(r"\bsk-(proj-|live-)?[a-zA-Z0-9_-]{30,}\b").unwrap()),
                ("GitHub Personal Access Token", Regex::new(r"\b(gh[pousr]_[a-zA-Z0-9]{36,}|github_pat_[a-zA-Z0-9_]{50,})\b").unwrap()),
                ("AWS Access Key", Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap()),
            ]
        });

        for (name, re) in patterns {
            if re.is_match(content) {
                return Some(name);
            }
        }
        None
    }
}

#[async_trait]
impl Plugin for SafetyPlugin {
    fn name(&self) -> &'static str { "safety" }
    fn help(&self) -> &'static str { "Detecteert phishing links en waarschuwt bij per ongeluk gelekte API keys of tokens" }

    async fn on_message(&self, _ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Controle op per ongeluk gelekte tokens of API keys
        if let Some(secret_type) = Self::detect_leaked_secret(&msg.content) {
            return Ok(Some(format!(
                "🚨 [BEVEILIGINGSALARM] \x02{}\x02, je lijkt per ongeluk een gevoelige \x02{}\x02 te hebben gedeeld in de chat! Verwijder dit bericht en roteer/herroep deze sleutel direct!",
                msg.author, secret_type
            )));
        }

        // 2. Controle op phishing of scam links
        for word in msg.content.split_whitespace() {
            if word.starts_with("http://") || word.starts_with("https://") {
                if Self::is_suspicious(word) {
                    return Ok(Some(format!(
                        "⚠️ [BEVEILIGINGSWAARSCHUWING] Mogelijke phishing/scam link gedetecteerd van {}: klik niet op verdachte links!",
                        msg.author
                    )));
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_detection() {
        let fake_discord_token = format!("{}.{}.{}", "MTA1NDk2NTc1NDM5OTU5MjU3MQ", "GqO_aB", "1234567890abcdefghijklmnopqrstuv");
        assert_eq!(SafetyPlugin::detect_leaked_secret(&fake_discord_token), Some("Discord Bot Token"));

        let fake_openai = format!("sk-{}", "abcdefghijklmnopqrstuvwxyz1234567890");
        assert_eq!(SafetyPlugin::detect_leaked_secret(&format!("Hier is mijn key: {}", fake_openai)), Some("OpenAI / LLM API Key"));

        let fake_gh = format!("ghp_{}", "123456789012345678901234567890123456");
        assert_eq!(SafetyPlugin::detect_leaked_secret(&fake_gh), Some("GitHub Personal Access Token"));

        let fake_aws = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        assert_eq!(SafetyPlugin::detect_leaked_secret(&fake_aws), Some("AWS Access Key"));

        assert_eq!(SafetyPlugin::detect_leaked_secret("Gewoon een gezellig bericht in het kanaal!"), None);
    }
}
