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

fn resolve_lang(input: &str) -> Option<(String, String)> {
    let s = input.trim().to_lowercase();
    let res = match s.as_str() {
        "nl" | "ned" | "nld" | "dutch" | "nederlands" | "hollands" => ("NL", "Dutch"),
        "de" | "ger" | "deu" | "german" | "duits" | "deutsch" => ("DE", "German"),
        "en" | "eng" | "english" | "engels" => ("EN", "English"),
        "es" | "sp" | "spa" | "spanish" | "spaans" | "español" | "castellano" => ("ES", "Spanish"),
        "fr" | "fra" | "fre" | "french" | "frans" | "français" => ("FR", "French"),
        "it" | "ita" | "italian" | "italiaans" | "italiano" => ("IT", "Italian"),
        "pt" | "por" | "portuguese" | "portugees" | "português" => ("PT", "Portuguese"),
        "ru" | "rus" | "russian" | "russisch" => ("RU", "Russian"),
        "ja" | "jp" | "jpn" | "japanese" | "japans" => ("JA", "Japanese"),
        "zh" | "cn" | "chi" | "chinese" | "chinees" => ("ZH", "Chinese"),
        "pl" | "pol" | "polish" | "pools" | "polski" => ("PL", "Polish"),
        "sv" | "se" | "swe" | "swedish" | "zweeds" | "svenska" => ("SV", "Swedish"),
        "da" | "dk" | "dan" | "danish" | "deens" | "dansk" => ("DA", "Danish"),
        "fi" | "fin" | "finnish" | "fins" | "suomi" => ("FI", "Finnish"),
        "no" | "nor" | "norwegian" | "noors" | "norsk" => ("NO", "Norwegian"),
        "tr" | "tur" | "turkish" | "turks" | "türkçe" => ("TR", "Turkish"),
        "uk" | "ua" | "ukr" | "ukrainian" | "oekraïens" => ("UK", "Ukrainian"),
        "ar" | "ara" | "arabic" | "arabisch" => ("AR", "Arabic"),
        "el" | "gr" | "gre" | "greek" | "grieks" => ("EL", "Greek"),
        "cs" | "cz" | "cze" | "ces" | "czech" | "tsjechisch" => ("CS", "Czech"),
        "hu" | "hun" | "hungarian" | "hongaars" | "magyar" => ("HU", "Hungarian"),
        "ro" | "ron" | "rum" | "romanian" | "roemeens" => ("RO", "Romanian"),
        "bg" | "bul" | "bulgarian" | "bulgaars" => ("BG", "Bulgarian"),
        "hr" | "cro" | "hrv" | "croatian" | "kroatisch" => ("HR", "Croatian"),
        "sr" | "srp" | "serbian" | "servisch" => ("SR", "Serbian"),
        "sk" | "slk" | "slovak" | "slowaaks" => ("SK", "Slovak"),
        "sl" | "slv" | "slovenian" | "sloveens" => ("SL", "Slovenian"),
        "id" | "ind" | "indonesian" | "indonesisch" => ("ID", "Indonesian"),
        "hi" | "hin" | "hindi" => ("HI", "Hindi"),
        "th" | "tha" | "thai" | "thais" => ("TH", "Thai"),
        "vi" | "vie" | "vietnamese" | "vietnamees" => ("VI", "Vietnamese"),
        "he" | "heb" | "il" | "hebrew" | "hebreeuws" => ("HE", "Hebrew"),
        "la" | "lat" | "latin" | "latijn" => ("LA", "Latin"),
        "fy" | "fry" | "frisian" | "fries" => ("FY", "Frisian"),
        "af" | "afr" | "afrikaans" => ("AF", "Afrikaans"),
        "eo" | "epo" | "esperanto" => ("EO", "Esperanto"),
        "is" | "ice" | "isl" | "icelandic" | "ijslands" => ("IS", "Icelandic"),
        "et" | "est" | "estonian" | "ests" => ("ET", "Estonian"),
        "lv" | "lav" | "latvian" | "lets" => ("LV", "Latvian"),
        "lt" | "lit" | "lithuanian" | "litouws" => ("LT", "Lithuanian"),
        "ca" | "cat" | "catalan" | "catalaans" => ("CA", "Catalan"),
        _ => {
            // Als het een 2- of 3-letterige ISO code is, accepteer deze dynamisch
            if (s.len() == 2 || s.len() == 3) && s.chars().all(|c| c.is_ascii_alphabetic()) {
                return Some((s.to_uppercase(), s.to_uppercase()));
            }
            return None;
        }
    };
    Some((res.0.to_string(), res.1.to_string()))
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
        let (from_code, from_name, to_code, to_name, text_to_translate, is_auto_bilingual) = if let Some((first, rest)) = args.split_once(' ') {
            if first.contains(':') {
                let mut parts = first.splitn(2, ':');
                let raw_from = parts.next().unwrap_or("auto");
                let raw_to = parts.next().unwrap_or(default_target);

                let (f_code, f_name) = resolve_lang(raw_from).unwrap_or(("AUTO".into(), "auto-detected language".into()));
                let (t_code, t_name) = resolve_lang(raw_to).unwrap_or_else(|| {
                    resolve_lang(default_target).unwrap_or(("NL".into(), "Dutch".into()))
                });
                (f_code, f_name, t_code, t_name, rest.trim(), false)
            } else if let Some((code, name)) = resolve_lang(first) {
                ("AUTO".into(), "auto-detected language".into(), code, name, rest.trim(), false)
            } else {
                let (def_code, def_name) = resolve_lang(default_target).unwrap_or(("NL".into(), "Dutch".into()));
                ("AUTO".into(), "auto-detected language".into(), def_code, def_name, args, true)
            }
        } else {
            let (def_code, def_name) = resolve_lang(default_target).unwrap_or(("NL".into(), "Dutch".into()));
            ("AUTO".into(), "auto-detected language".into(), def_code, def_name, args, true)
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
            let prompt = if is_auto_bilingual {
                format!(
                    "You are an expert translator. Detect the language of the following text:\n- If the text is in Dutch, translate it into English.\n- If the text is in English or any other language, translate it into Dutch.\nOutput ONLY the clean translated sentence without notes, quotes, or explanations:\n\n{}",
                    text_to_translate
                )
            } else {
                format!(
                    "You are a professional translator. Translate the following text into {} (from {}). Output ONLY the direct translated text in {}, without quotes, explanations, or notes. Do NOT repeat the input sentence if it is not in {}:\n\n{}",
                    to_name, from_name, to_name, to_name, text_to_translate
                )
            };

            if let Ok(reply) = ctx.ai_client.ask("Translator", &prompt, Some(&current_model)).await {
                let clean = reply.trim().trim_matches('"').trim().to_string();
                // Weiger antwoorden die leeg zijn óf een letterlijke echo zijn van de bronsleutel
                if !clean.is_empty() && (!clean.eq_ignore_ascii_case(text_to_translate) || to_name == from_name) {
                    ctx.ai_manager.record_consumption(60);
                    let target_header = if is_auto_bilingual { "AUTO" } else { &to_code };
                    return Ok(Some(format!("🌐 [{} -> {}] {}", ai_label, target_header, clean)));
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
                        let clean = trans
                            .replace("&quot;", "\"")
                            .replace("&#39;", "'")
                            .replace("&#039;", "'")
                            .replace("&amp;", "&")
                            .replace("&lt;", "<")
                            .replace("&gt;", ">");
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
