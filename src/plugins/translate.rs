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

fn resolve_lang(input: &str) -> Option<(&'static str, &'static str)> {
    match input.trim().to_lowercase().as_str() {
        "nl" | "ned" | "nld" | "dutch" | "nederlands" => Some(("NL", "Dutch")),
        "de" | "ger" | "deu" | "german" | "duits" | "deutsch" => Some(("DE", "German")),
        "en" | "eng" | "english" | "engels" => Some(("EN", "English")),
        "es" | "sp" | "spa" | "spanish" | "spaans" | "español" => Some(("ES", "Spanish")),
        "fr" | "fra" | "fre" | "french" | "frans" | "français" => Some(("FR", "French")),
        "it" | "ita" | "italian" | "italiaans" | "italiano" => Some(("IT", "Italian")),
        "pt" | "por" | "portuguese" | "portugees" => Some(("PT", "Portuguese")),
        "ru" | "rus" | "russian" | "russisch" => Some(("RU", "Russian")),
        "ja" | "jp" | "jpn" | "japanese" | "japans" => Some(("JA", "Japanese")),
        "zh" | "cn" | "chi" | "chinese" | "chinees" => Some(("ZH", "Chinese")),
        "pl" | "pol" | "polish" | "pools" => Some(("PL", "Polish")),
        "sv" | "se" | "swe" | "swedish" | "zweeds" => Some(("SV", "Swedish")),
        "da" | "dk" | "dan" | "danish" | "deens" => Some(("DA", "Danish")),
        "fi" | "fin" | "finnish" | "fins" => Some(("FI", "Finnish")),
        "no" | "nor" | "norwegian" | "noors" => Some(("NO", "Norwegian")),
        "tr" | "tur" | "turkish" | "turks" => Some(("TR", "Turkish")),
        "uk" | "ukr" | "ukrainian" | "oekraïens" => Some(("UK", "Ukrainian")),
        "ar" | "ara" | "arabic" | "arabisch" => Some(("AR", "Arabic")),
        "el" | "gr" | "gre" | "greek" | "grieks" => Some(("EL", "Greek")),
        _ => None,
    }
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
        "!translate [taalcode/taalpaar] <tekst> - Vertaalt tekst (bijv. !tr de Hallo, !tr nl:en Hoi, !tr sp Buenos días)"
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
                "Gebruik: !tr [doeltaal of van:naar] <tekst> (bijv: !tr de Guten Tag, !tr en:nl Hello world, !tr sp Hola)"
            } else {
                "Usage: !translate [target or from:to] <text> (e.g. !tr de Guten Tag, !tr en:nl Hello world, !tr sp Hola)"
            };
            return Ok(Some(usage.into()));
        }

        // Parse taalopties: "nl:de", "de", "en", "sp", "es", "fr", etc.
        let (from_code, from_name, to_code, to_name, text_to_translate) = if let Some((first, rest)) = args.split_once(' ') {
            if first.contains(':') {
                let mut parts = first.splitn(2, ':');
                let raw_from = parts.next().unwrap_or("auto");
                let raw_to = parts.next().unwrap_or(default_target);

                let (f_code, f_name) = resolve_lang(raw_from).unwrap_or(("AUTO", "auto-detected language"));
                let (t_code, t_name) = resolve_lang(raw_to).unwrap_or_else(|| {
                    resolve_lang(default_target).unwrap_or(("NL", "Dutch"))
                });
                (f_code, f_name, t_code, t_name, rest.trim())
            } else if let Some((code, name)) = resolve_lang(first) {
                ("AUTO", "auto-detected language", code, name, rest.trim())
            } else {
                let (def_code, def_name) = resolve_lang(default_target).unwrap_or(("NL", "Dutch"));
                ("AUTO", "auto-detected language", def_code, def_name, args)
            }
        } else {
            let (def_code, def_name) = resolve_lang(default_target).unwrap_or(("NL", "Dutch"));
            ("AUTO", "auto-detected language", def_code, def_name, args)
        };

        if text_to_translate.is_empty() {
            let usage_sub = if is_dutch {
                "Gebruik: !tr [doeltaal] <tekst>"
            } else {
                "Usage: !tr [target] <text>"
            };
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
                    "target_lang": to_code,
                });

                if from_code != "AUTO" {
                    body["source_lang"] = serde_json::json!(from_code);
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
                                    to_code,
                                    item.text.trim()
                                )));
                            }
                        }
                    }
                }
            }
        }

        // 2. Probeer de lokale FlashML FreeToken AI (Ollama Qwen2.5) met duidelijke instructie
        let current_model = ctx.ai_manager.get_model();
        if ctx.ai_manager.can_consume(80) {
            let prompt = format!(
                "You are a professional translator. Translate the following text into {} (from {}). Output ONLY the direct translated text in {}, without quotes, explanations, or notes. Do NOT repeat the input sentence if it is not in {}:\n\n{}",
                to_name, from_name, to_name, to_name, text_to_translate
            );

            if let Ok(reply) = ctx.ai_client.ask("Translator", &prompt, Some(&current_model)).await {
                let clean = reply.trim().trim_matches('"').trim().to_string();
                // Weiger antwoorden die leeg zijn óf een letterlijke echo zijn van de bronsleutel
                if !clean.is_empty() && (!clean.eq_ignore_ascii_case(text_to_translate) || to_name == from_name) {
                    ctx.ai_manager.record_consumption(60);
                    return Ok(Some(format!("🌐 [{} -> {}] {}", ai_label, to_code, clean)));
                }
            }
        }

        // 3. Fallback naar MyMemory Translation API
        let mymemory_from = if from_code == "AUTO" {
            if to_code == "NL" { "en" } else { "nl" }
        } else {
            &from_code.to_lowercase()
        };
        let pair = format!("{}|{}", mymemory_from, to_code.to_lowercase());

        let resp = ctx
            .http
            .get("https://api.mymemory.translated.net/get")
            .query(&[("q", text_to_translate), ("langpair", &pair)])
            .header("User-Agent", "IRCordBot/1.0 (translation client)")
            .send()
            .await?;

        if resp.status().is_success() {
            if let Ok(data) = resp.json::<MyMemoryResponse>().await {
                if let Some(res) = data.response_data {
                    if let Some(trans) = res.translated_text {
                        let clean = trans.replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
                        return Ok(Some(format!(
                            "🌐 [{} -> {}] {}",
                            web_label,
                            to_code,
                            clean
                        )));
                    }
                }
            }
        }

        Ok(Some("🌐 Geen vertaling kunnen vinden voor deze invoer.".into()))
    }
}
