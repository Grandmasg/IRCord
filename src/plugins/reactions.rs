use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ReactionsPlugin {
    cooldowns: Mutex<HashMap<String, Instant>>, // "channel:category" -> Instant
}

impl ReactionsPlugin {
    pub fn new() -> Self {
        Self {
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    fn check_and_set_cooldown(&self, channel: &str, category: &str, cooldown_secs: u64) -> bool {
        let key = format!("{}:{}", channel, category);
        let mut lock = self.cooldowns.lock().unwrap();
        let now = Instant::now();

        if let Some(last) = lock.get(&key) {
            if now.duration_since(*last) < Duration::from_secs(cooldown_secs) {
                return false; // Still on cooldown
            }
        }

        lock.insert(key, now);
        true
    }
}

impl Default for ReactionsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ReactionsPlugin {
    fn name(&self) -> &'static str {
        "reactions"
    }

    fn help(&self) -> &'static str {
        "Reageert passief op gemeenschapsbegroetingen, juichen (\\o/, WOEI!), zwaaien en emoticons"
    }

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Ignore bot's own messages or commands
        let trimmed = msg.content.trim();
        let bot_nick = std::env::var("IRC_NICK").unwrap_or_else(|_| "IRCordBot".to_string());
        if ctx.config.general.is_command_trigger(trimmed)
            || msg.author.eq_ignore_ascii_case("IRCord")
            || msg.author.eq_ignore_ascii_case(&bot_nick)
        {
            return Ok(None);
        }

        let is_dutch = ctx.config.general.language == "nl";

        // 1. Begroetingen (Ochtend, Avond, Nacht)
        // Checkt of het gericht is aan iedereen / de bot, of specifiek aan een andere nick (zoals "Mogge Huub!")
        if let Some(category) = detect_greeting(trimmed, &bot_nick) {
            match category {
                GreetingCategory::Morning => {
                    if self.check_and_set_cooldown(&msg.channel, "morning", 120) {
                        let reply = if is_dutch {
                            format!("Goedemorgen \x02{}\x02! ☕ Fijne dag gewenst!", msg.author)
                        } else {
                            format!("Good morning \x02{}\x02! ☕ Have a great day!", msg.author)
                        };
                        return Ok(Some(reply));
                    }
                }
                GreetingCategory::Evening => {
                    if self.check_and_set_cooldown(&msg.channel, "evening", 120) {
                        let reply = if is_dutch {
                            format!("Goedenavond \x02{}\x02! 🌆 Gezellige avond gewenst.", msg.author)
                        } else {
                            format!("Good evening \x02{}\x02! 🌆 Have a pleasant evening.", msg.author)
                        };
                        return Ok(Some(reply));
                    }
                }
                GreetingCategory::Night => {
                    if self.check_and_set_cooldown(&msg.channel, "night", 120) {
                        let reply = if is_dutch {
                            format!("Welterusten \x02{}\x02! 🌙 Slaap lekker.", msg.author)
                        } else {
                            format!("Good night \x02{}\x02! 🌙 Sleep well.", msg.author)
                        };
                        return Ok(Some(reply));
                    }
                }
            }
        }

        let lower = trimmed.to_lowercase();

        // 4. Directe begroeting aan de bot (bijv. "hallo bot", "hoi ircord", "hey monkeybot")
        let bot_clean = bot_nick.to_lowercase();
        if (lower.contains("bot") || lower.contains("ircord") || (!bot_clean.is_empty() && lower.contains(&bot_clean)))
            && (lower.starts_with("hallo") || lower.starts_with("hoi") || lower.starts_with("hey") || lower.starts_with("hi "))
            && self.check_and_set_cooldown(&msg.channel, "hello", 60) {
                let reply = if is_dutch {
                    format!("Hoi \x02{}\x02! 👋 Alles goed?", msg.author)
                } else {
                    format!("Hello \x02{}\x02! 👋 How are you doing?", msg.author)
                };
                return Ok(Some(reply));
            }

        // 5. Juichen / Blijdschap (\o/, woei, hoera, yay, etc.)
        let is_cheer = trimmed.contains("\\o/")
            || trimmed.contains("\\O/")
            || trimmed.contains("\\0/")
            || lower.contains("woei")
            || lower.contains("hieperdepiep")
            || lower.contains("hoera")
            || lower.contains("woohoo")
            || lower == "yay"
            || lower == "yay!"
            || lower.contains("*juicht*")
            || lower.contains("*feest*");

        if is_cheer
            && self.check_and_set_cooldown(&msg.channel, "cheer", 8) {
                // Probeer eerst AI voor een grappige, dynamische reactie
                let current_model = ctx.ai_manager.get_model();
                if ctx.ai_manager.can_consume(40) {
                    let system_prompt = r#"Je bent een gevatte, vrolijke bot in een gezellig Nederlands IRC-kanaal. Iemand juicht (bijv. met '\o/' of 'WOEI!') over iets in zijn bericht. Bedenk een korte, grappige reactie van maximaal 8 woorden die BIJ HET ONDERWERP van het bericht past (bijv. bij onweer: 'Bliksemsnel feestje! \o/ ⚡', bij een nieuwe release: 'Eindelijk! Taart erbij! 🍰'). Herhaal het bericht NOOIT letterlijk en citeer het niet. Geef UITSLUITEND de reactie, zonder aanhalingstekens of uitleg."#;
                    let user_prompt = format!("Bericht van {}: {}", msg.author, msg.content);

                    let ask_fut = ctx.ai_client.ask_with_system(system_prompt, "CheerBot", &user_prompt, Some(&current_model));
                    if let Ok(Ok(ai_reply)) = tokio::time::timeout(Duration::from_secs(6), ask_fut).await {
                        let clean = ai_reply.trim().trim_matches('"').trim();
                        // Echo of bijna-echo van het originele bericht is irritant: dan liever de fallback.
                        let is_echo = crate::utils::langdetect::similarity(clean, trimmed) >= 0.6
                            || lower.contains(&clean.to_lowercase());
                        if !clean.is_empty() && clean.chars().count() <= 80 && !is_echo {
                            ctx.ai_manager.record_consumption(30);
                            return Ok(Some(clean.to_string()));
                        }
                    }
                }

                // Snelle fallback lijst met klassieke IRC juich-teksten
                let fallbacks = [
                    "WOEI! \\o/",
                    "\\o/ Jaaaaa! Hype!",
                    "\\o/ 🎉 WOEI! Hieperdepiep!",
                    "\\o/ Biertje erbij! 🍻",
                    "\\o/ *confetti strooit* 🎉",
                    "\\o/ Feestjeee!",
                    "\\o/ *juicht luidkeels mee*",
                    "(ﾉ◕ヮ◕)ﾉ*:･ﾟ✧ WOEI! \\o/",
                ];
                let idx = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as usize)
                    % fallbacks.len();
                return Ok(Some(fallbacks[idx].to_string()));
            }

        // 6. High-five / Zwaaien (o/ of \o)
        if trimmed == "o/" || trimmed == "O/" {
            if self.check_and_set_cooldown(&msg.channel, "wave", 10) {
                return Ok(Some("\\o".to_string()));
            }
        } else if (trimmed == "\\o" || trimmed == "\\O")
            && self.check_and_set_cooldown(&msg.channel, "wave", 10) {
                return Ok(Some("o/".to_string()));
            }

        // 7. Table flip: (╯°□°)╯︵ ┻━┻ of ┻━┻
        if trimmed.contains("┻━┻")
            && self.check_and_set_cooldown(&msg.channel, "tableflip", 15) {
                return Ok(Some(format!(
                    "┬─┬ノ( º _ ºノ) Rustig maar \x02{}\x02, niet met de meubels gooien!",
                    msg.author
                )));
            }

        // 8. Shrug: ¯\_(ツ)_/¯
        if trimmed.contains("¯\\_(ツ)_/¯")
            && self.check_and_set_cooldown(&msg.channel, "shrug", 15) {
                return Ok(Some("¯\\_(ツ)_/¯ Het is wat het is!".to_string()));
            }

        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GreetingCategory {
    Morning,
    Evening,
    Night,
}

/// Detecteert begroetingen (ochtend, avond, nacht).
///
/// Geeft `None` terug wanneer de begroeting specifiek gericht is aan een andere nickname
/// (bijv. "Mogge Huub!" of "Goedemorgen Peter").
///
/// Geeft `Some(category)` terug wanneer het gericht is aan iedereen ("Mogge allemaal", "Mogge!"),
/// of specifiek aan de bot zelf ("Mogge Monkeybot", "Mogge botje").
pub(crate) fn detect_greeting(text: &str, bot_nick: &str) -> Option<GreetingCategory> {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }

    const MORNING_PREFIXES: &[&str] = &[
        "goedemorgen",
        "goeiemorgen",
        "goede morgen",
        "goeie morgen",
        "good morning",
        "mogguh",
        "mogge",
        "mornin",
        "morning",
        "gm",
    ];

    const EVENING_PREFIXES: &[&str] = &[
        "goedenavond",
        "goeonavond",
        "goede avond",
        "goeie avond",
        "good evening",
        "fijne avond",
    ];

    const NIGHT_PREFIXES: &[&str] = &[
        "welterusten",
        "slaap lekker",
        "slaapwel",
        "trusten",
        "good night",
        "gn",
    ];

    let check_list = |prefixes: &[&'static str], cat: GreetingCategory| -> Option<(GreetingCategory, &str)> {
        for &prefix in prefixes {
            if let Some(rest) = lower.strip_prefix(prefix) {
                // Word boundary check: volgend karakter mag geen alfanumeriek teken zijn
                if let Some(ch) = rest.chars().next() {
                    if ch.is_alphanumeric() {
                        continue;
                    }
                }
                return Some((cat, rest));
            }
        }
        None
    };

    let (category, remainder) = check_list(MORNING_PREFIXES, GreetingCategory::Morning)
        .or_else(|| check_list(EVENING_PREFIXES, GreetingCategory::Evening))
        .or_else(|| check_list(NIGHT_PREFIXES, GreetingCategory::Night))?;

    let trimmed_rest = remainder.trim();
    if trimmed_rest.is_empty() {
        // Alleen de begroeting (bijv. "mogge", "goedemorgen")
        return Some(category);
    }

    // Strip voorloop-leestekens (zoals ", huub!" -> "huub!", "@huub" -> "huub")
    let trimmed_tokens = trimmed_rest.trim_start_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    if trimmed_tokens.is_empty() {
        // Enkel leestekens/emojis (bijv. "mogge!", "mogge...", "mogge : )")
        return Some(category);
    }

    // Zoek het eerste woord dat alfanumerieke karakters bevat (sla emojis/smileys zoals ":-)" of "☕" over)
    let mut target = None;
    for word in trimmed_tokens.split_whitespace() {
        let clean = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if !clean.is_empty() {
            target = Some(clean);
            break;
        }
    }

    let target_nick = match target {
        Some(t) => t,
        None => {
            // Geen alfanumerieke naam gevonden (enkel emojis/smileys) -> algemene kanaalbegroeting
            return Some(category);
        }
    };

    // 1. Is de begroeting gericht aan de bot zelf?
    let bot_clean = bot_nick.trim().to_lowercase();
    if target_nick == "bot"
        || target_nick == "botje"
        || target_nick == "ircord"
        || (!bot_clean.is_empty() && target_nick == bot_clean)
    {
        return Some(category);
    }

    // 2. Is de begroeting gericht aan het gehele kanaal / iedereen?
    const COLLECTIVE_TARGETS: &[&str] = &[
        "allemaal",
        "allen",
        "all",
        "iedereen",
        "everyone",
        "everybody",
        "folks",
        "guys",
        "peeps",
        "peepz",
        "mensen",
        "lui",
        "lieden",
        "luisteraars",
        "kanaal",
        "channel",
        "chan",
        "chat",
        "room",
        "wereld",
        "world",
        "samen",
        "tezamen",
        "tesamen",
        "dames",
        "heren",
        "vrienden",
        "kanjers",
        "toppers",
    ];

    if COLLECTIVE_TARGETS.contains(&target_nick.as_str()) {
        return Some(category);
    }

    // 3. De begroeting is gericht aan een specifieke andere persoon/nick (bijv. "Huub", "Peter")
    // In dat geval: zeg niks ("zeg maar niks")!
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooldown_mechanism() {
        let plugin = ReactionsPlugin::new();
        assert!(plugin.check_and_set_cooldown("#test", "morning", 60));
        // Immediate second call should be blocked by cooldown
        assert!(!plugin.check_and_set_cooldown("#test", "morning", 60));
        // Different category or channel should succeed
        assert!(plugin.check_and_set_cooldown("#test", "evening", 60));
        assert!(plugin.check_and_set_cooldown("#other", "morning", 60));
    }

    #[test]
    fn test_greeting_other_nick_ignored() {
        let bot_nick = "Monkeybot";

        // Gerichte begroeting aan een andere nick: bot moet ZWIJGEN
        assert_eq!(detect_greeting("Mogge Huub!", bot_nick), None);
        assert_eq!(detect_greeting("Mogge Huub", bot_nick), None);
        assert_eq!(detect_greeting("mogge @Huub", bot_nick), None);
        assert_eq!(detect_greeting("Mogge, Huub!", bot_nick), None);
        assert_eq!(detect_greeting("Mogge: Huub", bot_nick), None);
        assert_eq!(detect_greeting("Mogge :) Huub", bot_nick), None);
        assert_eq!(detect_greeting("Goedemorgen Peter", bot_nick), None);
        assert_eq!(detect_greeting("Goeiemorgen Anita!", bot_nick), None);
        assert_eq!(detect_greeting("Goedenavond Huub!", bot_nick), None);
        assert_eq!(detect_greeting("Welterusten Huub", bot_nick), None);
        assert_eq!(detect_greeting("Trusten Huub!", bot_nick), None);
        assert_eq!(detect_greeting("Slaap lekker Huub", bot_nick), None);
        assert_eq!(detect_greeting("GM Huub", bot_nick), None);
    }

    #[test]
    fn test_greeting_general_and_bot_acknowledged() {
        let bot_nick = "Monkeybot";

        // Algemene begroetingen (zonder specifieke nick) -> bot mag reageren
        assert_eq!(detect_greeting("Mogge", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge!", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogguh!", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Goedemorgen", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Goeiemorgen!", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge :)", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge ☕", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("GM", bot_nick), Some(GreetingCategory::Morning));

        // Collectieve kanaalbegroetingen -> bot mag reageren
        assert_eq!(detect_greeting("Mogge allemaal!", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge allen", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge iedereen", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("GM all", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Goedenavond allemaal", bot_nick), Some(GreetingCategory::Evening));
        assert_eq!(detect_greeting("Goedenavond!", bot_nick), Some(GreetingCategory::Evening));
        assert_eq!(detect_greeting("Welterusten iedereen", bot_nick), Some(GreetingCategory::Night));
        assert_eq!(detect_greeting("Welterusten", bot_nick), Some(GreetingCategory::Night));
        assert_eq!(detect_greeting("Slaap lekker!", bot_nick), Some(GreetingCategory::Night));

        // Direct aan de bot gericht -> bot mag reageren
        assert_eq!(detect_greeting("Mogge Monkeybot!", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge bot", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Mogge botje", bot_nick), Some(GreetingCategory::Morning));
        assert_eq!(detect_greeting("Goedemorgen ircord", bot_nick), Some(GreetingCategory::Morning));

        // Woorden die toevallig beginnen met begroetingstekst
        assert_eq!(detect_greeting("mogged", bot_nick), None);
        assert_eq!(detect_greeting("gmail", bot_nick), None);
    }
}
