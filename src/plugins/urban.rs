use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct UrbanDictionaryPlugin;

#[derive(Deserialize)]
struct UrbanResponse {
    list: Vec<UrbanEntry>,
}

#[derive(Deserialize)]
struct UrbanEntry {
    word: String,
    definition: String,
    example: Option<String>,
    thumbs_up: Option<i64>,
    thumbs_down: Option<i64>,
}

#[async_trait]
impl Plugin for UrbanDictionaryPlugin {
    fn name(&self) -> &'static str {
        "urban"
    }

    fn triggers(&self) -> &[&'static str] {
        &["ud", "urban", "slang"]
    }

    fn help(&self) -> &'static str {
        "!ud <zoekterm> - Zoekt definities en slang op Urban Dictionary"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let term = cmd.args.trim();
        if term.is_empty() {
            return Ok(Some("Gebruik: !ud <zoekterm>".into()));
        }

        let url = format!(
            "https://api.urbandictionary.com/v0/define?term={}",
            urlencoding_simple(term)
        );

        let resp = ctx
            .http
            .get(&url)
            .header("User-Agent", "IRCordBot/1.0")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Some(format!(
                "📚 Kon Urban Dictionary niet bereiken voor '{}'.",
                term
            )));
        }

        let data: UrbanResponse = resp.json().await?;
        if data.list.is_empty() {
            return Ok(Some(format!(
                "📚 Geen definitie gevonden op Urban Dictionary voor '{}'.",
                term
            )));
        }

        let entry = &data.list[0];
        let clean_def = clean_text(&entry.definition, 200);
        let up = entry.thumbs_up.unwrap_or(0);
        let down = entry.thumbs_down.unwrap_or(0);

        let reply = if let Some(ref ex) = entry.example {
            let clean_ex = clean_text(ex, 120);
            if !clean_ex.is_empty() {
                format!(
                    "📚 [Urban] {}: {} | Vb: \"{}\" (👍 {} / 👎 {})",
                    entry.word, clean_def, clean_ex, up, down
                )
            } else {
                format!(
                    "📚 [Urban] {}: {} (👍 {} / 👎 {})",
                    entry.word, clean_def, up, down
                )
            }
        } else {
            format!(
                "📚 [Urban] {}: {} (👍 {} / 👎 {})",
                entry.word, clean_def, up, down
            )
        };

        Ok(Some(reply))
    }
}

fn clean_text(s: &str, max_len: usize) -> String {
    let unbracketed = s.replace('[', "").replace(']', "");
    let single_line = unbracketed.replace("\r\n", " ").replace('\n', " ").replace('\r', " ");
    let trimmed = single_line.trim();

    if trimmed.chars().count() > max_len {
        let mut cut: String = trimmed.chars().take(max_len.saturating_sub(3)).collect();
        cut.push_str("...");
        cut
    } else {
        trimmed.to_string()
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.trim().replace(' ', "%20")
}
