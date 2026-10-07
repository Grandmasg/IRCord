use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use crate::utils::i18n::LocaleManager;
use crate::utils::langdetect;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
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
    /// Per kanaal+nick: laatste berichten, true = in de kanaaltaal geschreven.
    nick_history: Mutex<HashMap<String, VecDeque<bool>>>,
}

impl TranslatePlugin {
    pub fn new() -> Self {
        Self {
            channel_cache: Mutex::new(HashMap::new()),
            nick_history: Mutex::new(HashMap::new()),
        }
    }

    /// Registreert of dit bericht in de kanaaltaal was en geeft terug of de nick tot dan toe
    /// vrijwel altijd in de kanaaltaal schreef (dan hanteren we een strengere drempel).
    fn note_and_check_resident(&self, channel: &str, nick: &str, in_channel_lang: bool) -> bool {
        const WINDOW: usize = 5;
        const RESIDENT_MIN: usize = 3;
        let key = format!("{}/{}", channel.to_lowercase(), nick.to_lowercase());
        let mut map = self.nick_history.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 2000 {
            map.clear();
        }
        let hist = map.entry(key).or_default();
        let resident = hist.iter().filter(|b| **b).count() >= RESIDENT_MIN;
        hist.push_back(in_channel_lang);
        if hist.len() > WINDOW {
            hist.pop_front();
        }
        resident
    }

    async fn get_channel_settings(&self, ctx: &PluginContext, platform: &str, channel: &str) -> ChannelSettingsCache {
        let key = channel.to_lowercase();
        {
            let lock = self.channel_cache.lock().unwrap_or_else(|e| e.into_inner());
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

        let mut lock = self.channel_cache.lock().unwrap_or_else(|e| e.into_inner());
        lock.insert(key, entry.clone());
        entry
    }

    fn update_channel_settings(&self, channel: &str, lang_code: String, lang_name: String, auto_translate: bool) {
        let key = channel.to_lowercase();
        let mut lock = self.channel_cache.lock().unwrap_or_else(|e| e.into_inner());
        lock.insert(key, ChannelSettingsCache {
            language_code: lang_code,
            language_name: lang_name,
            auto_translate,
            last_triggered: Instant::now() - Duration::from_secs(60),
        });
    }

    fn check_and_set_cooldown(&self, channel: &str, cooldown_secs: u64) -> bool {
        let key = channel.to_lowercase();
        let mut lock = self.channel_cache.lock().unwrap_or_else(|e| e.into_inner());
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

/// DeepL doeltaalcodes: "EN" en "PT" zijn als doeltaal verouderd en vragen een variant.
fn deepl_target_code(code: &str) -> String {
    match code.to_ascii_uppercase().as_str() {
        "EN" => "EN-GB".to_string(),
        "PT" => "PT-PT".to_string(),
        other => other.to_string(),
    }
}

/// Vertaalt via de officiële DeepL API v2 (https://developers.deepl.com/docs/getting-started/your-first-api-request).
/// Geeft `(gedetecteerde brontaal, vertaling)` terug, of `None` zonder sleutel, bij quota/fouten.
async fn deepl_translate(
    ctx: &PluginContext,
    text: &str,
    target: &str,
    source: Option<&str>,
) -> Option<(String, String)> {
    let key = std::env::var("DEEPL_API_KEY").ok()?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let endpoint = if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };
    let mut body = serde_json::json!({ "text": [text], "target_lang": deepl_target_code(target) });
    if let Some(src) = source {
        body["source_lang"] = serde_json::json!(src.to_ascii_uppercase());
    }

    #[derive(Deserialize)]
    struct Resp {
        translations: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        #[serde(default)]
        detected_source_language: String,
        text: String,
    }

    let resp = ctx
        .http
        .post(endpoint)
        .header("Authorization", format!("DeepL-Auth-Key {}", key))
        .header("User-Agent", "IRCordBot/1.0 (translation client)")
        .timeout(Duration::from_secs(8))
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        // 456 = quota op; 429 = te veel verzoeken. In beide gevallen terugvallen op de AI.
        tracing::warn!("DeepL gaf status {}", resp.status());
        return None;
    }
    let data = resp.json::<Resp>().await.ok()?;
    let item = data.translations.into_iter().next()?;
    Some((item.detected_source_language.to_uppercase(), item.text.trim().to_string()))
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LanguageScore {
    pub dutch_pct: f32,
    pub english_pct: f32,
    pub german_pct: f32,
    pub french_pct: f32,
    pub spanish_pct: f32,
    pub dutch_count: usize,
    pub english_count: usize,
    pub german_count: usize,
    pub french_count: usize,
    pub spanish_count: usize,
    pub total_words: usize,
}

#[allow(dead_code)]
impl LanguageScore {
    pub fn is_likely_dutch(&self) -> bool {
        if self.dutch_count == 0 {
            return false;
        }

        // Als een andere taal duidelijk meer hits heeft dan Nederlands, is het geen Nederlands!
        if self.french_count > self.dutch_count
            || self.german_count > self.dutch_count
            || self.spanish_count > self.dutch_count
            || (self.english_count > self.dutch_count && self.english_pct >= 25.0)
        {
            return false;
        }

        self.dutch_count >= 2 || (self.total_words <= 4 && self.dutch_count >= 1) || self.dutch_pct >= 25.0
    }

    pub fn is_likely_english(&self) -> bool {
        self.english_count >= 2
            && self.english_count > self.dutch_count
            && (self.english_pct >= 25.0 || (self.total_words <= 5 && self.english_count >= 2))
    }

    pub fn is_likely_german(&self) -> bool {
        (self.german_count >= 2 || (self.total_words <= 4 && self.german_count >= 1 && self.dutch_count == 0))
            && self.german_count > self.dutch_count
            && (self.german_pct >= 20.0 || (self.total_words <= 5 && self.german_count >= 1))
    }

    pub fn is_likely_french(&self) -> bool {
        (self.french_count >= 2 || (self.total_words <= 4 && self.french_count >= 1 && self.dutch_count == 0))
            && self.french_count > self.dutch_count
            && (self.french_pct >= 20.0 || (self.total_words <= 5 && self.french_count >= 1))
    }

    pub fn is_likely_spanish(&self) -> bool {
        (self.spanish_count >= 2 || (self.total_words <= 4 && self.spanish_count >= 1 && self.dutch_count == 0))
            && self.spanish_count > self.dutch_count
            && (self.spanish_pct >= 20.0 || (self.total_words <= 5 && self.spanish_count >= 1))
    }

    /// Geeft aan of de tekst overtuigend een andere taal is dan Nederlands
    pub fn is_foreign_to_dutch(&self) -> bool {
        !self.is_likely_dutch() && (self.is_likely_english() || self.is_likely_german() || self.is_likely_french() || self.is_likely_spanish())
    }

    /// Geeft aan of de tekst overtuigend een andere taal is dan Engels
    pub fn is_foreign_to_english(&self) -> bool {
        !self.is_likely_english() && (self.is_likely_dutch() || self.is_likely_german() || self.is_likely_french() || self.is_likely_spanish())
    }
}

/// Detecteert of een tekst niet-Latijnse alfabetten bevat (zoals Chinees, Japans, Koreaans, Cyrillisch, Grieks, Arabisch)
pub fn contains_non_latin_script(text: &str) -> bool {
    text.chars().any(|c| {
        ('\u{0400}'..='\u{04FF}').contains(&c) // Cyrillisch (Russisch, Oekraïens, etc.)
        || ('\u{0370}'..='\u{03FF}').contains(&c) // Grieks
        || ('\u{0600}'..='\u{06FF}').contains(&c) // Arabisch
        || ('\u{0590}'..='\u{05FF}').contains(&c) // Hebreeuws
        || ('\u{4E00}'..='\u{9FFF}').contains(&c) // Chinees / Japans Kanji (CJK Ideographs)
        || ('\u{3400}'..='\u{4DBF}').contains(&c) // CJK Extension A
        || ('\u{3040}'..='\u{309F}').contains(&c) // Japans Hiragana
        || ('\u{30A0}'..='\u{30FF}').contains(&c) // Japans Katakana
        || ('\u{AC00}'..='\u{D7AF}').contains(&c) // Koreaans Hangul
        || ('\u{0E00}'..='\u{0E7F}').contains(&c) // Thai
        || ('\u{0900}'..='\u{097F}').contains(&c) // Hindi / Devanagari
    })
}

/// Berekent de relatieve taalpercentages (Nederlands, Engels, Duits, Frans, Spaans) voor een chatbericht
#[allow(dead_code)]
pub fn calculate_language_percentages(text: &str) -> LanguageScore {
    calculate_language_percentages_with_locale(text, None)
}

/// Berekent de relatieve taalpercentages met behulp van de centrale LocaleManager
pub fn calculate_language_percentages_with_locale(text: &str, locale: Option<&LocaleManager>) -> LanguageScore {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return LanguageScore {
            dutch_pct: 0.0,
            english_pct: 0.0,
            german_pct: 0.0,
            french_pct: 0.0,
            spanish_pct: 0.0,
            dutch_count: 0,
            english_count: 0,
            german_count: 0,
            french_count: 0,
            spanish_count: 0,
            total_words: 0,
        };
    }

    let default_locale;
    let loc = match locale {
        Some(l) => l,
        None => {
            default_locale = LocaleManager::load("locales", "nl");
            &default_locale
        }
    };

    let nl_words = loc.get_distinct_words("nl").unwrap_or(&[]);
    let en_words = loc.get_distinct_words("en").unwrap_or(&[]);
    let de_words = loc.get_distinct_words("de").unwrap_or(&[]);

    // Fallbacks voor Frans en Spaans
    const DISTINCT_FRENCH: &[&str] = &[
        "les", "des", "une", "est", "sont", "que", "qui", "dans", "pour", "pas", "sur", "cette", "avec",
        "tout", "tous", "nous", "vous", "ils", "elles", "mais", "notre", "votre", "leur", "comme",
        "aussi", "bonjour", "merci", "salut", "comment", "pourquoi", "quand", "toujours", "mon", "ma",
        "mes", "ton", "ta", "tes", "son", "sa", "ses", "moi", "toi", "lui", "eux", "rien", "jamais",
        "fais", "fait", "fai", "vais", "vas", "va", "suis", "veux", "peux", "sais", "bien", "très", "tres",
        "ça", "ca", "oui", "non", "bon", "bonne",
    ];

    const DISTINCT_SPANISH: &[&str] = &[
        "los", "las", "una", "unos", "unas", "por", "para", "con", "son", "como", "pero", "este", "esta",
        "estos", "estas", "todo", "todos", "toda", "todas", "muy", "hola", "gracias", "amigo", "amigos",
        "donde", "quando", "porque", "bueno", "buenos", "buenas", "también", "nosotros", "ustedes", "favor",
        "nada", "nunca", "siempre", "ahora", "quiero", "tengo", "hacer", "hace", "estoy", "está", "estan",
    ];

    let fr_custom = loc.get_distinct_words("fr");
    let es_custom = loc.get_distinct_words("es");

    let is_dutch_word = |w: &str| -> bool { nl_words.iter().any(|item| item.as_str() == w) };
    let is_english_word = |w: &str| -> bool { en_words.iter().any(|item| item.as_str() == w) };
    let is_german_word = |w: &str| -> bool { de_words.iter().any(|item| item.as_str() == w) };
    let is_french_word = |w: &str| -> bool {
        if let Some(list) = fr_custom {
            list.iter().any(|item| item.as_str() == w)
        } else {
            DISTINCT_FRENCH.contains(&w)
        }
    };
    let is_spanish_word = |w: &str| -> bool {
        if let Some(list) = es_custom {
            list.iter().any(|item| item.as_str() == w)
        } else {
            DISTINCT_SPANISH.contains(&w)
        }
    };

    let mut dutch_hits = 0;
    let mut english_hits = 0;
    let mut german_hits = 0;
    let mut french_hits = 0;
    let mut spanish_hits = 0;

    for w in &words {
        if is_dutch_word(w) {
            dutch_hits += 1;
        } else if is_english_word(w) {
            english_hits += 1;
        } else if is_german_word(w) {
            german_hits += 1;
        } else if is_french_word(w) {
            french_hits += 1;
        } else if is_spanish_word(w) {
            spanish_hits += 1;
        }
    }

    let total = words.len();
    let dutch_pct = (dutch_hits as f32 / total as f32) * 100.0;
    let english_pct = (english_hits as f32 / total as f32) * 100.0;
    let german_pct = (german_hits as f32 / total as f32) * 100.0;
    let french_pct = (french_hits as f32 / total as f32) * 100.0;
    let spanish_pct = (spanish_hits as f32 / total as f32) * 100.0;

    LanguageScore {
        dutch_pct,
        english_pct,
        german_pct,
        french_pct,
        spanish_pct,
        dutch_count: dutch_hits,
        english_count: english_hits,
        german_count: german_hits,
        french_count: french_hits,
        spanish_count: spanish_hits,
        total_words: total,
    }
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

        // 1. Officiële DeepL API v2 indien DEEPL_API_KEY is ingesteld
        let deepl_source = if from_code == "AUTO" { None } else { Some(from_code.as_str()) };
        if let Some((_, translated)) = deepl_translate(ctx, text_to_translate, &to_code, deepl_source).await {
            if !translated.is_empty() {
                return Ok(Some(format!("🌐 [{} -> {}] {}", deepl_label, to_code, translated)));
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

        // 2. Filter ruis en URLs
        if trimmed.contains("http://") || trimmed.contains("https://") {
            return Ok(None);
        }

        let has_foreign_script = contains_non_latin_script(trimmed);
        let char_count = trimmed.chars().count();

        // Ruis (nicks, releasecodes zoals S03E10, emoticons, afkortingen) telt niet mee.
        let cleaned = langdetect::clean_for_detection(trimmed);

        // Niet-Latijns schrift gebruikt geen spaties tussen woorden: 2 tekens is al genoeg.
        // Latijns schrift: minstens 4 echte woorden, anders is taaldetectie te onbetrouwbaar.
        if has_foreign_script {
            if char_count < 2 {
                return Ok(None);
            }
        } else if langdetect::word_count(&cleaned) < 4 || cleaned.chars().count() < 15 {
            return Ok(None);
        }

        // 3. Filter alledaagse IRC slang en computer-leenwoorden (centraal beheerd in locales)
        if ctx.locale.is_casual_banter(trimmed) {
            return Ok(None);
        }

        // 4. Controleer of auto-vertaling actief is voor dit kanaal
        let settings = self.get_channel_settings(ctx, &msg.platform, &msg.channel).await;
        if !settings.auto_translate {
            return Ok(None);
        }

        // 5. Taaldetectie (offline n-grammen). Alleen vertalen bij een overtuigend vreemde taal;
        // is de kanaaltaal ook maar enigszins waarschijnlijk, dan doen we niets.
        let channel_lang = langdetect::language_from_code(&settings.language_code);
        let mut detected: Option<langdetect::Detection> = None;
        if !has_foreign_script {
            let Some(d) = langdetect::detect(&cleaned, channel_lang) else {
                return Ok(None);
            };
            let resident = self.note_and_check_resident(&msg.channel, &msg.author, d.looks_like_channel_language(channel_lang));
            let min_conf = if resident {
                langdetect::MIN_FOREIGN_CONFIDENCE_RESIDENT
            } else {
                langdetect::MIN_FOREIGN_CONFIDENCE
            };
            tracing::debug!(
                "🌐 [Taaldetectie] input='{}' ➔ top={} ({:.2}), kanaaltaal={:.2}, resident={}, drempel={:.2}",
                cleaned, langdetect::code_of(d.top), d.top_confidence, d.channel_confidence, resident, min_conf
            );
            if !d.is_confidently_foreign(channel_lang, min_conf) {
                return Ok(None);
            }
            // Extra veto: de woordenlijst-heuristiek ziet het wél als Nederlands.
            if settings.language_code.eq_ignore_ascii_case("NL")
                && calculate_language_percentages_with_locale(trimmed, Some(&ctx.locale)).is_likely_dutch()
            {
                return Ok(None);
            }
            detected = Some(d);
        }

        // 6. Cooldown per kanaal (6 seconden) om AI-overbelasting bij snelle chat te voorkomen
        if !self.check_and_set_cooldown(&msg.channel, 6) {
            return Ok(None);
        }

        // 7a. Officiële DeepL API (indien sleutel): geeft ook de gedetecteerde brontaal terug.
        // Meldt DeepL dat de tekst al in de kanaaltaal is, dan doen we niets.
        if !has_foreign_script || std::env::var("DEEPL_API_KEY").map(|k| !k.trim().is_empty()).unwrap_or(false) {
            if let Some((src, translated)) = deepl_translate(ctx, trimmed, &settings.language_code, None).await {
                let same_lang = src.eq_ignore_ascii_case(&settings.language_code);
                let expected_ok = detected
                    .as_ref()
                    .map(|d| src.eq_ignore_ascii_case(langdetect::code_of(d.top)))
                    .unwrap_or(true);
                if same_lang || !expected_ok || translated.is_empty() || langdetect::is_near_identical(trimmed, &translated) {
                    return Ok(None);
                }
                return Ok(Some(format!(
                    "🌐 [{} ➔ {}] \x02{}\x02: {}",
                    src,
                    settings.language_code.to_uppercase(),
                    msg.author,
                    translated
                )));
            }
        }

        // 7b. Beoordeel met lokale Ollama AI of het bericht afwijkt van de kanaaltaal
        let current_model = ctx.ai_manager.get_model();
        if !ctx.ai_manager.can_consume(50) {
            return Ok(None);
        }

        // Verwachte brontaal volgens de detector (tweede stem: de AI moet het hiermee eens zijn)
        let (expected_lang_code, detected_lang_hint) = match &detected {
            Some(d) => (langdetect::code_of(d.top), langdetect::name_of(d.top)),
            None => ("AUTO", "non-Latin (Chinese/Japanese/Russian/etc.)"),
        };

        // System prompt:
        // Voor niet-Latijns schrift (Chinees, Japans, etc.) is het 100% zeker geen Nederlands; vraag direct om vertaling!
        let system_prompt = if has_foreign_script {
            format!(
                "You are an automated chat translator for an IRC/Discord channel.\n\
                The primary channel language is {} ({}).\n\
                The input text is written in a non-Latin script.\n\
                TASK: Detect the 2-letter language code (e.g. ZH, JA, KO, RU, AR, EL) and translate the text directly into {}.\n\
                Output format strictly: [LANG] <translated text in {}>\n\
                Example: [ZH] Dit is een testzin.\n\
                Output ONLY the formatted translation, without quotes, notes, or explanations.",
                settings.language_name, settings.language_code,
                settings.language_name,
                settings.language_name
            )
        } else {
            format!(
                "You are an automated chat translator for an IRC/Discord channel where the primary chat language is {} ({}).\n\
                RULES:\n\
                1. If the message is already written in {}, reply ONLY: NONE\n\
                2. If the message is written in a foreign language (such as {}), translate it into {}.\n\
                Output format strictly: [LANG] <translated text in {}>\n\
                Example: [FR] Dit is een testzin.\n\
                If the text is already in {}, output ONLY: NONE",
                settings.language_name, settings.language_code,
                settings.language_name,
                detected_lang_hint, settings.language_name,
                settings.language_name,
                settings.language_name
            )
        };

        let user_prompt = format!("Message: \"{}\"", trimmed);

        let ask_fut = ctx.ai_client.ask_with_system(&system_prompt, "AutoTranslator", &user_prompt, Some(&current_model));
        let mut final_result: Option<(String, String)> = None;

        if let Ok(Ok(ai_reply)) = tokio::time::timeout(Duration::from_secs(8), ask_fut).await {
            let clean = ai_reply.trim().trim_matches('"').trim();
            tracing::info!("🌐 [Auto-Translate Evaluatie] Kanaal '{}': input='{}' ➔ AI='{}'", msg.channel, trimmed, clean);

            // Parse resultaat: [XX] vertaling OF XX: vertaling OF XX vertaling
            let parsed = if let (Some(open), Some(close)) = (clean.find('['), clean.find(']')) {
                if close > open && close - open <= 10 {
                    let tag = clean[open + 1..close].trim().to_uppercase();
                    let rest = clean[close + 1..].trim().trim_start_matches(':').trim();
                    Some((tag, rest.to_string()))
                } else {
                    None
                }
            } else if let Some((first, rest)) = clean.split_once([':', ' ']) {
                let first_clean = first.trim().to_uppercase();
                if (first_clean.len() == 2 || first_clean.len() == 3) && first_clean.chars().all(|c| c.is_ascii_alphabetic()) && first_clean != "NONE" {
                    Some((first_clean, rest.trim().to_string()))
                } else {
                    None
                }
            } else {
                None
            };

            if let Some((tag, trans)) = parsed {
                // Als de vertaling niet leeg is en niet "NONE", en tag is niet de kanaaltaal:
                let agrees = expected_lang_code == "AUTO" || tag.eq_ignore_ascii_case(expected_lang_code);
                if !trans.is_empty()
                    && !trans.eq_ignore_ascii_case("NONE")
                    && !tag.eq_ignore_ascii_case(&settings.language_code)
                    && agrees
                    && !langdetect::is_near_identical(trimmed, &trans)
                {
                    final_result = Some((tag, trans));
                }
            }
        }

        // Fallback: als de classificatie-evaluatie "NONE" of "FR NONE" opleverde, maar Rust WEET dat het een buitenlands bericht is:
        if final_result.is_none() && has_foreign_script {
            tracing::info!("🌐 [Auto-Translate Fallback] Rust detecteerde vreemde taal ({}), directe vertaling aanvragen...", detected_lang_hint);
            let direct_prompt = format!(
                "You are a professional translator. Translate the following text directly into {} (from {}). Output ONLY the direct translated text in {}, without quotes, explanations, or notes:\n\n{}",
                settings.language_name, detected_lang_hint, settings.language_name, trimmed
            );
            if let Ok(Ok(reply)) = tokio::time::timeout(Duration::from_secs(8), ctx.ai_client.ask("Translator", &direct_prompt, Some(&current_model))).await {
                let clean = reply.trim().trim_matches('"').trim();
                if !clean.is_empty() && !clean.eq_ignore_ascii_case("NONE") && !langdetect::is_near_identical(trimmed, clean) {
                    let tag = "?".to_string();
                    final_result = Some((tag, clean.to_string()));
                }
            }
        }

        if let Some((orig_lang, translation)) = final_result {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deepl_target_codes() {
        assert_eq!(deepl_target_code("en"), "EN-GB");
        assert_eq!(deepl_target_code("PT"), "PT-PT");
        assert_eq!(deepl_target_code("nl"), "NL");
    }

    #[test]
    fn test_language_percentages() {
        let s1 = calculate_language_percentages("is dus overschreven of heeft nu verkeerde link");
        assert!(s1.dutch_pct > 50.0);
        assert!(s1.dutch_count >= 4);
        assert!(s1.is_likely_dutch());
        assert!(!s1.is_foreign_to_dutch());

        let s2 = calculate_language_percentages("paswoord is goed, dat vind ik in de settings bij portainer terug");
        assert!(s2.dutch_pct > 50.0);
        assert!(s2.dutch_count >= 6);
        assert!(s2.is_likely_dutch());
        assert!(!s2.is_foreign_to_dutch());

        let s3 = calculate_language_percentages("Hello everyone, does someone know how to configure the bridge in docker?");
        assert!(s3.english_pct > 40.0);
        assert!(s3.english_count >= 4);
        assert_eq!(s3.dutch_count, 0);
        assert!(s3.is_likely_english());
        assert!(s3.is_foreign_to_dutch());

        // Duits test
        let s_de = calculate_language_percentages("Guten Tag, ich suche Hilfe mit meinem Linux Server bitte");
        assert!(s_de.german_count >= 3);
        assert!(s_de.is_likely_german());
        assert!(s_de.is_foreign_to_dutch());

        // Frans test
        let s_fr = calculate_language_percentages("Bonjour tout le monde, comment allez-vous aujourd'hui?");
        assert!(s_fr.french_count >= 2);
        assert!(s_fr.is_likely_french());
        assert!(s_fr.is_foreign_to_dutch());

        // Spaans test
        let s_es = calculate_language_percentages("Hola amigos, alguien me puede ayudar con este problema por favor?");
        assert!(s_es.spanish_count >= 3);
        assert!(s_es.is_likely_spanish());
        assert!(s_es.is_foreign_to_dutch());
    }

    #[test]
    fn test_resident_nick_gets_stricter_threshold() {
        let plugin = TranslatePlugin::new();
        for _ in 0..3 {
            plugin.note_and_check_resident("#chan", "PjoT", true);
        }
        assert!(plugin.note_and_check_resident("#chan", "pjot", true));
        assert!(!plugin.note_and_check_resident("#chan", "newbie", false));
    }

    #[test]
    fn test_non_latin_script() {
        assert!(contains_non_latin_script("这是一个测试句子。"));
        assert!(contains_non_latin_script("Привет как дела"));
        assert!(contains_non_latin_script("Γειά σου κόσμε"));
        assert!(contains_non_latin_script("مرحبا كيف حالك"));
        assert!(contains_non_latin_script("こんにちは世界"));
        assert!(!contains_non_latin_script("Gewoon een normale Nederlandse zin met café en één!"));
    }

    #[test]
    fn test_is_casual_banter() {
        let locale = LocaleManager::load("locales", "nl");
        assert!(locale.is_casual_banter("yeah yeah"));
        assert!(locale.is_casual_banter("nope, firefox: nope"));
        assert!(locale.is_casual_banter("cool nice"));
        assert!(!locale.is_casual_banter("Can someone please explain this error to me?"));
    }

    #[test]
    fn test_locale_driven_language_percentages() {
        let locale = LocaleManager::load("locales", "nl");
        assert!(locale.is_casual_banter("settings portainer docker"));
        assert!(locale.is_casual_banter("yeah nope"));

        let score = calculate_language_percentages_with_locale(
            "is dus overschreven of heeft nu verkeerde link",
            Some(&locale),
        );
        assert!(score.is_likely_dutch());
        assert!(!score.is_foreign_to_dutch());

        let en_score = calculate_language_percentages_with_locale(
            "Hello everyone, can you please help with this server configuration?",
            Some(&locale),
        );
        assert!(en_score.is_likely_english());
        assert!(en_score.is_foreign_to_dutch());

        let fr_score1 = calculate_language_percentages_with_locale(
            "Ceci est une phrase de test.",
            Some(&locale),
        );
        assert!(fr_score1.is_likely_french());
        assert!(fr_score1.is_foreign_to_dutch());
        assert!(!fr_score1.is_likely_dutch());

        let fr_score2 = calculate_language_percentages_with_locale(
            "Il s'agit d'une phrase test qui doit être traduite en néerlandais.",
            Some(&locale),
        );
        assert!(fr_score2.is_likely_french());
        assert!(fr_score2.is_foreign_to_dutch());
        assert!(!fr_score2.is_likely_dutch());

        // Informele/gesproken Franse chatzinnen (zoals getest door Cjefke)
        let fr_score3 = calculate_language_percentages_with_locale(
            "sa fai rien pour moi",
            Some(&locale),
        );
        assert!(fr_score3.is_likely_french());
        assert!(fr_score3.is_foreign_to_dutch());
        assert!(!fr_score3.is_likely_dutch());

        let fr_score4 = calculate_language_percentages_with_locale(
            "tu est trés bon!",
            Some(&locale),
        );
        assert!(fr_score4.is_likely_french());
        assert!(fr_score4.is_foreign_to_dutch());
        assert!(!fr_score4.is_likely_dutch());
    }
}


