use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct TranslatePlugin;

#[derive(Deserialize)]
struct MyMemoryResponse {
    #[serde(rename = "responseData")]
    response_data: Option<MyMemoryData>,
}

#[derive(Deserialize)]
struct MyMemoryData {
    #[serde(rename = "translatedText")]
    translated_text: Option<String>,
}

#[async_trait]
impl Plugin for TranslatePlugin {
    fn name(&self) -> &'static str {
        "translate"
    }

    fn triggers(&self) -> &[&'static str] {
        &["translate", "tr", "vertaal"]
    }

    fn help(&self) -> &'static str {
        "!translate [langpair] <text> / !tr [taalpaar] <tekst> - Translates text"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let is_dutch = ctx.locale.is_dutch();
        let default_target = ctx.locale.language();

        if args.is_empty() {
            let usage = if is_dutch {
                "Gebruik: !tr [van:naar] <tekst> (bijv: !tr en:nl Hello world)"
            } else {
                "Usage: !translate [from:to] <text> (e.g. !translate en:nl Hello world)"
            };
            return Ok(Some(usage.into()));
        }

        // Parse optional language pair: "en:nl", "nl:en", "de:nl", "es:en", etc.
        let (lang_from, lang_to, text_to_translate) = if let Some((first, rest)) = args.split_once(' ') {
            if first.contains(':') {
                let mut parts = first.splitn(2, ':');
                let from = parts.next().unwrap_or("auto").to_lowercase();
                let to = parts.next().unwrap_or(default_target).to_lowercase();
                (from, to, rest.trim())
            } else if first.len() == 2 && (first == "en" || first == "nl" || first == "de" || first == "fr" || first == "es") {
                ("auto".to_string(), first.to_lowercase(), rest.trim())
            } else {
                ("auto".to_string(), default_target.to_string(), args)
            }
        } else {
            ("auto".to_string(), default_target.to_string(), args)
        };

        if text_to_translate.is_empty() {
            let usage_sub = if is_dutch { "Gebruik: !tr [van:naar] <tekst>" } else { "Usage: !tr [from:to] <text>" };
            return Ok(Some(usage_sub.into()));
        }

        let ai_label = ctx.locale.t("ai_translation_title");
        let deepl_label = "DeepL";
        let web_label = ctx.locale.t("web_translation_title");

        // 1. Check officiële DeepL API v2 indien DEEPL_API_KEY is ingesteld
        if let Ok(key) = std::env::var("DEEPL_API_KEY") {
            let key = key.trim();
            if !key.is_empty() {
                let endpoint = if key.ends_with(":fx") {
                    "https://api-free.deepl.com/v2/translate"
                } else {
                    "https://api.deepl.com/v2/translate"
                };

                let mut body = serde_json::json!({
                    "text": [text_to_translate],
                    "target_lang": lang_to.to_uppercase(),
                });

                if lang_from != "auto" {
                    body["source_lang"] = serde_json::json!(lang_from.to_uppercase());
                }

                if let Ok(resp) = ctx
                    .http
                    .post(endpoint)
                    .header("Authorization", format!("DeepL-Auth-Key {}", key))
                    .header("User-Agent", "IRCordBot/1.0 (translation client)")
                    .json(&body)
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        #[derive(Deserialize)]
                        struct DeepLResponse {
                            translations: Vec<DeepLItem>,
                        }
                        #[derive(Deserialize)]
                        struct DeepLItem {
                            text: String,
                        }

                        if let Ok(data) = resp.json::<DeepLResponse>().await {
                            if let Some(item) = data.translations.first() {
                                return Ok(Some(format!(
                                    "🌐 [{} -> {}] {}",
                                    deepl_label,
                                    lang_to.to_uppercase(),
                                    item.text.trim()
                                )));
                            }
                        }
                    }
                }
            }
        }

        // 2. Probeer de lokale FlashML FreeToken AI indien beschikbaar en budget toereikend
        let current_model = ctx.ai_manager.get_model();
        if ctx.ai_manager.can_consume(80) {
            let prompt = format!(
                "You are a professional translator. Translate the following text into {} (source language: {}). Output ONLY the clean translated sentence without any introduction, explanations, quotes, or conversational remarks:\n{}",
                lang_to, lang_from, text_to_translate
            );

            match ctx.ai_client.ask("Translator", &prompt, Some(&current_model)).await {
                Ok(reply) => {
                    let clean = reply.trim().trim_matches('"').to_string();
                    if !clean.is_empty() {
                        ctx.ai_manager.record_consumption(60);
                        return Ok(Some(format!("🌐 [{} -> {}] {}", ai_label, lang_to.to_uppercase(), clean)));
                    }
                }
                Err(_) => {
                    // Fallback to web translation
                }
            }
        }

        // 3. Fallback naar MyMemory Translation API
        let pair = format!("{}|{}", if lang_from == "auto" { "en" } else { &lang_from }, lang_to);
        let encoded_text = urlencoding_simple(text_to_translate);
        let url = format!(
            "https://api.mymemory.translated.net/get?q={}&langpair={}",
            encoded_text, pair
        );

        let resp = ctx
            .http
            .get(&url)
            .header("User-Agent", "IRCordBot/1.0 (translation client)")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Some("🌐 Kon de vertaling op dit moment niet uitvoeren.".into()));
        }

        let data: MyMemoryResponse = resp.json().await?;
        if let Some(res) = data.response_data {
            if let Some(trans) = res.translated_text {
                let clean = trans.replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
                return Ok(Some(format!(
                    "🌐 [{} -> {}] {}",
                    web_label,
                    lang_to.to_uppercase(),
                    clean
                )));
            }
        }

        Ok(Some("🌐 Geen vertaling kunnen vinden voor deze invoer.".into()))
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.trim().replace(' ', "%20")
}
