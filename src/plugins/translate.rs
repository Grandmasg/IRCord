use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
struct ChannelSettingsCache {
    language_code: String,
    language_name: String,
    auto_translate: bool,
    last_triggered: Instant,
}

pub struct TranslatePlugin {
    channel_cache: Mutex<HashMap<String, ChannelSettingsCache>>,
}

impl TranslatePlugin {
    pub fn new() -> Self {
        Self {
            channel_cache: Mutex::new(HashMap::new()),
        }
    }

    async fn get_channel_settings(&self, ctx: &PluginContext, platform: &str, channel: &str) -> ChannelSettingsCache {
        let key = channel.to_lowercase();
        {
            let lock = self.channel_cache.lock().unwrap();
            if let Some(cached) = lock.get(&key) {
                return cached.clone();
            }
        }

        // Query database
        let row: Option<(String, bool)> = sqlx::query_as(
            "SELECT language, auto_translate FROM channel_settings WHERE LOWER(channel) = LOWER(?)"
        )
        .bind(channel)
        .fetch_optional(&ctx.db)
        .await
        .unwrap_or(None);

        let (code, auto_tr) = if let Some((lang, autotr)) = row {
            (lang, autotr)
        } else {
            let default_lang = ctx.config.channel_language(platform, channel);
            (default_lang.to_string(), false)
        };

        let (lang_code, lang_name) = resolve_lang(&code).unwrap_or(("NL".into(), "Dutch".into()));

        let entry = ChannelSettingsCache {
            language_code: lang_code,
            language_name: lang_name,
            auto_translate: auto_tr,
            last_triggered: Instant::now() - Duration::from_secs(60),
        };

        let mut lock = self.channel_cache.lock().unwrap();
        lock.insert(key, entry.clone());
        entry
    }

    fn update_channel_settings(&self, channel: &str, lang_code: String, lang_name: String, auto_translate: bool) {
        let key = channel.to_lowercase();
        let mut lock = self.channel_cache.lock().unwrap();
        lock.insert(key, ChannelSettingsCache {
            language_code: lang_code,
            language_name: lang_name,
            auto_translate,
            last_triggered: Instant::now() - Duration::from_secs(60),
        });
    }

    fn check_and_set_cooldown(&self, channel: &str, cooldown_secs: u64) -> bool {
        let key = channel.to_lowercase();
        let mut lock = self.channel_cache.lock().unwrap();
        let now = Instant::now();

        if let Some(entry) = lock.get_mut(&key) {
            if now.duration_since(entry.last_triggered) < Duration::from_secs(cooldown_secs) {
                return false;
            }
            entry.last_triggered = now;
            return true;
        }

        true
    }
}

impl Default for TranslatePlugin {
    fn default() -> Self {
        Self::new()
    }
}

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
        &["translate", "tr", "vertaal", "chatlang", "kanaaltaal", "autotr", "autotranslate"]
    }

    fn help(&self) -> &'static str {
        "!tr [taal] <tekst> | !chatlang [nl/en/de] | !autotr [on/off/status] - Vertalingen en realtime kanaalvertaling"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trigger = cmd.trigger.to_lowercase();
        let args = cmd.args.trim();

        // 1. Kanaal-instelling voor chattaal: !chatlang / !kanaaltaal
        if trigger == "chatlang" || trigger == "kanaaltaal" {
            let current_settings = self.get_channel_settings(ctx, &cmd.platform, &cmd.channel).await;

            if args.is_empty() {
                let status_str = if current_settings.auto_translate { "AAN" } else { "UIT" };
                return Ok(Some(format!(
                    "🌐 [Kanaalinstelling] Standaard chattaal voor {} is \x02{} ({})\x02. Auto-vertaling staat \x02{}\x02 (wijzig met !chatlang <taal> of !autotr on/off).",
                    cmd.channel, current_settings.language_name, current_settings.language_code, status_str
                )));
            }

            if !cmd.is_operator && !cmd.is_owner {
                return Ok(Some("⚠️ Alleen operators en de bot owner kunnen de standaard kanaaltaal aanpassen.".into()));
            }

            if let Some((code, name)) = resolve_lang(args) {
                sqlx::query(
                    r#"
                    INSERT INTO channel_settings (channel, language, auto_translate)
                    VALUES (?, ?, 0)
                    ON CONFLICT(channel) DO UPDATE SET language = excluded.language, updated_at = CURRENT_TIMESTAMP
                    "#
                )
                .bind(&cmd.channel)
                .bind(&code)
                .execute(&ctx.db)
                .await?;

                self.update_channel_settings(&cmd.channel, code.clone(), name.clone(), current_settings.auto_translate);
                return Ok(Some(format!(
                    "✅ [Kanaalinstelling] Standaard chattaal voor {} is nu ingesteld op \x02{} ({})\x02.",
                    cmd.channel, name, code
                )));
            } else {
                return Ok(Some(format!("⚠️ Onbekende taalcode '{args}'. Gebruik bijv. nl, en, de, es, fr.")));
            }
        }

        // 2. Realtime auto-vertaling in- of uitschakelen: !autotr / !autotranslate
        if trigger == "autotr" || trigger == "autotranslate" {
            let current_settings = self.get_channel_settings(ctx, &cmd.platform, &cmd.channel).await;

            if args.eq_ignore_ascii_case("on") || args.eq_ignore_ascii_case("aan") || args.eq_ignore_ascii_case("1") {
                if !cmd.is_operator && !cmd.is_owner {
                    return Ok(Some("⚠️ Alleen operators en de bot owner kunnen auto-vertaling in- of uitschakelen.".into()));
                }

                sqlx::query(
                    r#"
                    INSERT INTO channel_settings (channel, language, auto_translate)
                    VALUES (?, ?, 1)
                    ON CONFLICT(channel) DO UPDATE SET auto_translate = 1, updated_at = CURRENT_TIMESTAMP
                    "#
                )
                .bind(&cmd.channel)
                .bind(&current_settings.language_code)
                .execute(&ctx.db)
                .await?;

                self.update_channel_settings(&cmd.channel, current_settings.language_code.clone(), current_settings.language_name.clone(), true);
                return Ok(Some(format!(
                    "✅ [Auto-Vertaling] Ingeschakeld voor {}! Berichten die afwijken van het \x02{} ({})\x02 worden automatisch in het kanaal vertaald.",
                    cmd.channel, current_settings.language_name, current_settings.language_code
                )));
            } else if args.eq_ignore_ascii_case("off") || args.eq_ignore_ascii_case("uit") || args.eq_ignore_ascii_case("0") {
                if !cmd.is_operator && !cmd.is_owner {
                    return Ok(Some("⚠️ Alleen operators en de bot owner kunnen auto-vertaling in- of uitschakelen.".into()));
                }

                sqlx::query(
                    r#"
                    INSERT INTO channel_settings (channel, language, auto_translate)
                    VALUES (?, ?, 0)
                    ON CONFLICT(channel) DO UPDATE SET auto_translate = 0, updated_at = CURRENT_TIMESTAMP
                    "#
                )
                .bind(&cmd.channel)
                .bind(&current_settings.language_code)
                .execute(&ctx.db)
                .await?;

                self.update_channel_settings(&cmd.channel, current_settings.language_code.clone(), current_settings.language_name.clone(), false);
                return Ok(Some(format!("🛑 [Auto-Vertaling] Uitgeschakeld voor {}.", cmd.channel)));
            } else {
                let status = if current_settings.auto_translate { "AAN (actief) ✅" } else { "UIT 🛑" };
                return Ok(Some(format!(
                    "ℹ️ [Auto-Vertaling] Status voor {}: {} (Standaardtaal: \x02{} ({})\x02). Gebruik: !autotr on | !autotr off",
                    cmd.channel, status, current_settings.language_name, current_settings.language_code
                )));
            }
        }

        // 3. Reguliere handmatige vertaling: !tr / !translate / !vertaal
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

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trimmed = msg.content.trim();

        // 1. Negeer commando's of berichten van de bot zelf
        if ctx.config.general.is_command_trigger(trimmed)
            || msg.author.eq_ignore_ascii_case("IRCord")
            || msg.author.eq_ignore_ascii_case("Monkeybot")
        {
            return Ok(None);
        }

        // 2. Filter ruis: minimale lengte (minstens 2 woorden en 6 tekens) en geen URLs
        if trimmed.len() < 6
            || (trimmed.split_whitespace().count() < 2 && trimmed.len() < 12)
            || trimmed.contains("http://")
            || trimmed.contains("https://")
        {
            return Ok(None);
        }

        // 3. Controleer of auto-vertaling actief is voor dit kanaal
        let settings = self.get_channel_settings(ctx, &msg.platform, &msg.channel).await;
        if !settings.auto_translate {
            return Ok(None);
        }

        // 4. Cooldown per kanaal (6 seconden) om AI-overbelasting bij snelle chat te voorkomen
        if !self.check_and_set_cooldown(&msg.channel, 6) {
            return Ok(None);
        }

        // 5. Beoordeel met lokale Ollama AI of het bericht afwijkt van de kanaaltaal
        let current_model = ctx.ai_manager.get_model();
        if !ctx.ai_manager.can_consume(50) {
            return Ok(None);
        }

        let system_prompt = format!(
            "You are an automated real-time chat translator for an IRC/Discord channel where the primary chat language is {} ({}).\n\
            TASK:\n\
            1. If the message is ALREADY written in {} (or is code/technical syntax), reply with ONLY the word: NONE\n\
            2. If the message is written in ANOTHER language, translate it directly into {}.\n\
            Output format: [SOURCE_LANG_CODE] <translated text in {}>\n\
            Example: [NL] Hello world\n\
            Never output explanations, quotes, or notes.",
            settings.language_name, settings.language_code,
            settings.language_name,
            settings.language_name,
            settings.language_name
        );

        let user_prompt = format!("Message: \"{}\"", trimmed);

        let ask_fut = ctx.ai_client.ask_with_system(&system_prompt, "AutoTranslator", &user_prompt, Some(&current_model));
        if let Ok(Ok(ai_reply)) = tokio::time::timeout(Duration::from_secs(8), ask_fut).await {
            let clean = ai_reply.trim().trim_matches('"').trim();
            tracing::info!("🌐 [Auto-Translate Evaluatie] Kanaal '{}': input='{}' ➔ AI='{}'", msg.channel, trimmed, clean);

            if clean.is_empty()
                || clean.starts_with("NONE")
                || clean.eq_ignore_ascii_case("NONE")
                || clean.ends_with("NONE")
            {
                return Ok(None);
            }

            // Haal eventuele taal-tag [XX] op
            let (orig_lang, translation) = if let (Some(open), Some(close)) = (clean.find('['), clean.find(']')) {
                if close > open && close - open <= 10 {
                    let tag = clean[open + 1..close].trim().to_uppercase();
                    let rest = clean[close + 1..].trim().trim_start_matches(':').trim();
                    (tag, rest.to_string())
                } else {
                    ("?".to_string(), clean.to_string())
                }
            } else {
                ("?".to_string(), clean.to_string())
            };

            // Als het resultaat identiek is aan het origineel, niet vertalen
            if translation.is_empty() || translation.eq_ignore_ascii_case(trimmed) {
                return Ok(None);
            }

            // Bouw de taalbadge: als de AI per ongeluk de doeltaal tagde (bijv. [EN]), toon [➔ EN]
            let badge = if orig_lang == "?" || orig_lang == settings.language_code.to_uppercase() {
                format!("🌐 [➔ {}]", settings.language_code.to_uppercase())
            } else {
                format!("🌐 [{} ➔ {}]", orig_lang, settings.language_code.to_uppercase())
            };

            ctx.ai_manager.record_consumption(40);
            return Ok(Some(format!(
                "{} \x02{}\x02: {}",
                badge, msg.author, translation
            )));
        }

        Ok(None)
    }
}
