//! Offline taaldetectie (lingua, n-gram gebaseerd) en hulpfuncties voor de auto-vertaling.
//! Geen API-sleutels of kosten: alle modellen zitten in de binary.

use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};
use regex::Regex;
use std::sync::OnceLock;

const SUPPORTED: [Language; 7] = [
    Language::Dutch,
    Language::English,
    Language::German,
    Language::French,
    Language::Spanish,
    Language::Italian,
    Language::Portuguese,
];

/// Minimale zekerheid van de detector voordat een bericht als "vreemd" geldt.
pub const MIN_FOREIGN_CONFIDENCE: f64 = 0.85;
/// Strengere drempel voor nicks die recent vrijwel alleen in de kanaaltaal schreven.
pub const MIN_FOREIGN_CONFIDENCE_RESIDENT: f64 = 0.95;
/// Is de kanaaltaal minstens zo waarschijnlijk, dan vertalen we nooit.
pub const CHANNEL_LANG_VETO: f64 = 0.15;

fn detector() -> &'static LanguageDetector {
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| LanguageDetectorBuilder::from_languages(&SUPPORTED).build())
}

pub fn language_from_code(code: &str) -> Option<Language> {
    match code.to_ascii_uppercase().as_str() {
        "NL" => Some(Language::Dutch),
        "EN" => Some(Language::English),
        "DE" => Some(Language::German),
        "FR" => Some(Language::French),
        "ES" => Some(Language::Spanish),
        "IT" => Some(Language::Italian),
        "PT" => Some(Language::Portuguese),
        _ => None,
    }
}

pub fn code_of(lang: Language) -> &'static str {
    match lang {
        Language::Dutch => "NL",
        Language::English => "EN",
        Language::German => "DE",
        Language::French => "FR",
        Language::Spanish => "ES",
        Language::Italian => "IT",
        Language::Portuguese => "PT",
    }
}

pub fn name_of(lang: Language) -> &'static str {
    match lang {
        Language::Dutch => "Dutch",
        Language::English => "English",
        Language::German => "German",
        Language::French => "French",
        Language::Spanish => "Spanish",
        Language::Italian => "Italian",
        Language::Portuguese => "Portuguese",
    }
}

/// Uitkomst van de detectie voor één bericht.
#[derive(Debug, Clone)]
pub struct Detection {
    pub top: Language,
    pub top_confidence: f64,
    /// Zekerheid voor de kanaaltaal (0.0 als die niet ondersteund wordt).
    pub channel_confidence: f64,
}

impl Detection {
    /// Is de top-taal de kanaaltaal, of is de kanaaltaal ook maar enigszins waarschijnlijk?
    pub fn looks_like_channel_language(&self, channel: Option<Language>) -> bool {
        Some(self.top) == channel || self.channel_confidence >= CHANNEL_LANG_VETO
    }

    /// Overtuigend een andere taal dan de kanaaltaal?
    pub fn is_confidently_foreign(&self, channel: Option<Language>, min_confidence: f64) -> bool {
        !self.looks_like_channel_language(channel) && self.top_confidence >= min_confidence
    }
}

pub fn detect(text: &str, channel: Option<Language>) -> Option<Detection> {
    let values = detector().compute_language_confidence_values(text);
    let (top, top_confidence) = *values.first()?;
    let channel_confidence = channel
        .and_then(|c| values.iter().find(|(l, _)| *l == c).map(|(_, v)| *v))
        .unwrap_or(0.0);
    Some(Detection { top, top_confidence, channel_confidence })
}

fn noise_regexes() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            r"\x01(?:ACTION)?",                          // CTCP ACTION
            r"^\s*[\w\[\]\\`^{}|-]{2,32}[:,]\s+",         // "nick: " aan het begin
            r"(?:^|\s)@\w+",                              // @mentions
            r"(?:^|\s)[:;=8xX][-^o']?[)(DPpOo/\\|*]+(?:\s|$)", // emoticons
            r"\b[A-Z]{2,}\b",                             // afkortingen (USB, NAS, DNS)
            r"\b\w*\d\w*\b",                              // tokens met cijfers (S03E10, 1080p)
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid noise regex"))
        .collect()
    })
}

/// Haalt ruis weg (releasecodes, nicks, emoticons, afkortingen) voordat we taal bepalen.
pub fn clean_for_detection(text: &str) -> String {
    let mut out = text.to_string();
    for re in noise_regexes() {
        out = re.replace_all(&out, " ").into_owned();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Aantal echte woorden (minstens 2 letters) in een tekst.
pub fn word_count(text: &str) -> usize {
    text.split(|c: char| !c.is_alphabetic()).filter(|w| w.chars().count() >= 2).count()
}

fn normalize(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Gelijkenis 0.0..=1.0 tussen twee teksten, ongevoelig voor leestekens, hoofdletters en spaties.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (normalize(a), normalize(b));
    let max = a.len().max(b.len());
    if max == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / max as f64
}

/// Is de "vertaling" in feite hetzelfde als het origineel?
pub fn is_near_identical(original: &str, translation: &str) -> bool {
    similarity(original, translation) >= 0.85
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreign_nl(text: &str) -> bool {
        let cleaned = clean_for_detection(text);
        match detect(&cleaned, Some(Language::Dutch)) {
            Some(d) => d.is_confidently_foreign(Some(Language::Dutch), MIN_FOREIGN_CONFIDENCE),
            None => false,
        }
    }

    #[test]
    fn dutch_chat_is_never_foreign() {
        for t in [
            "en rendier knuffelen",
            "Uw dagelijkse portie bananen wordt dadelijk geserveerd",
            "grijnz in ouderwetse kleuren :)",
            "eet en kookze",
            "verder insta/facebook/threads/linkedin/mastadon/die blauwe",
            "Cookies voor één specifieke website verwijderen",
            "The Ark S03E10 staat op usenet",
        ] {
            assert!(!foreign_nl(t), "ten onrechte vreemd: {t}");
        }
    }

    #[test]
    fn real_foreign_is_detected() {
        for t in [
            "Hello everyone, does someone know how to configure the bridge in docker?",
            "Guten Tag, ich suche Hilfe mit meinem Linux Server bitte",
            "Bonjour tout le monde, comment allez-vous aujourd'hui?",
            "Hola amigos, alguien me puede ayudar con este problema por favor?",
        ] {
            assert!(foreign_nl(t), "niet als vreemd herkend: {t}");
        }
    }

    #[test]
    fn near_identical_translations_are_dropped() {
        assert!(is_near_identical("The Ark S03E10 staat op usenet", "The Ark S03E10 staat op usenet."));
        assert!(is_near_identical("grijnz in ouderwetse kleuren :)", "grijnz in oudewetse kleuren :)"));
        assert!(is_near_identical("eet en kookze", "Eet en kook ze."));
        assert!(!is_near_identical("Hello everyone, how are you?", "Hallo allemaal, hoe gaat het?"));
    }

    #[test]
    fn noise_is_stripped() {
        let c = clean_for_detection("PjoT: The Ark S03E10 staat op usenet :)");
        assert!(!c.contains("S03E10") && !c.contains("PjoT") && !c.contains(":)"), "{c}");
    }
}
