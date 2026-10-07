use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate, Weekday};

pub struct CountdownPlugin;

const WEEKEND_QUOTES_NL: [&str; 5] = [
    "Weekend vibes 😎",
    "Bier, rust en chaos 🍻",
    "Geen wekker = geluk ⏰",
    "Vrijheid loading... 🔓",
    "Slapen > alles 💤",
];
const WEEKEND_QUOTES_EN: [&str; 5] = [
    "Weekend vibes 😎",
    "Beer, rest and chaos 🍻",
    "No alarm clock = happiness ⏰",
    "Freedom loading... 🔓",
    "Sleep > everything 💤",
];

/// Antwoord op "is het al weekend?": zaterdag en zondag zijn weekend; anders het aantal dagen tot zaterdag.
/// `quote_idx` kiest een spreuk (modulo het aantal spreuken) zodat de functie testbaar blijft.
pub fn weekend_message(weekday: Weekday, quote_idx: usize, dutch: bool) -> String {
    let quotes = if dutch { &WEEKEND_QUOTES_NL } else { &WEEKEND_QUOTES_EN };
    let quote = quotes[quote_idx % quotes.len()];
    let days_to_saturday = 5 - weekday.num_days_from_monday() as i64; // ma=0 .. za=5, zo=6
    let headline = match (weekday, dutch) {
        (Weekday::Sat | Weekday::Sun, true) => "Ja! Het is weekend 😎".to_string(),
        (Weekday::Sat | Weekday::Sun, false) => "Yes! It's the weekend 😎".to_string(),
        (Weekday::Mon, true) => format!("Maandag-trauma 💀 nog {} dagen tot het weekend", days_to_saturday),
        (Weekday::Mon, false) => format!("Monday trauma 💀 {} days until the weekend", days_to_saturday),
        (_, true) if days_to_saturday == 1 => "Nee 😭 nog 1 dag: morgen is het zover!".to_string(),
        (_, false) if days_to_saturday == 1 => "Not yet 😭 just 1 more day: tomorrow!".to_string(),
        (_, true) => format!("Nee 😭 nog {} dagen tot het weekend", days_to_saturday),
        (_, false) => format!("Not yet 😭 {} days until the weekend", days_to_saturday),
    };
    format!("{} {}", headline, quote)
}

impl CountdownPlugin {
    pub fn new() -> Self {
        Self
    }

    /// Berekent resterende dagen tot Kerstmis (25 december)
    pub fn days_until_christmas(today: NaiveDate) -> (i64, i64, bool) {
        let (month, day) = (today.month(), today.day());

        // Check of het vandaag Kerstmis is (24, 25 of 26 dec)
        if month == 12 && (day == 24 || day == 25 || day == 26) {
            return (0, 0, true);
        }

        let target_year = if month == 12 && day > 26 {
            today.year() + 1
        } else {
            today.year()
        };

        let christmas = NaiveDate::from_ymd_opt(target_year, 12, 25).unwrap();
        let days = (christmas - today).num_days();
        let eve_days = days - 1;

        (days, eve_days, false)
    }

    /// Berekent resterende dagen tot Sinterklaas / Pakjesavond (5 december)
    pub fn days_until_sinterklaas(today: NaiveDate) -> (i64, bool) {
        let (month, day) = (today.month(), today.day());

        if month == 12 && day == 5 {
            return (0, true);
        }

        let target_year = if month == 12 && day > 5 {
            today.year() + 1
        } else {
            today.year()
        };

        let sint = NaiveDate::from_ymd_opt(target_year, 12, 5).unwrap();
        let days = (sint - today).num_days();

        (days, false)
    }

    /// Berekent resterende dagen tot Oud & Nieuw (1 januari)
    pub fn days_until_new_year(today: NaiveDate) -> (i64, bool) {
        let (month, day) = (today.month(), today.day());

        if month == 1 && day == 1 {
            return (0, true);
        }

        let target_year = today.year() + 1;
        let new_year = NaiveDate::from_ymd_opt(target_year, 1, 1).unwrap();
        let days = (new_year - today).num_days();

        (days, false)
    }
}

impl Default for CountdownPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for CountdownPlugin {
    fn name(&self) -> &'static str {
        "countdown"
    }

    fn triggers(&self) -> &[&'static str] {
        &[
            "kerst",
            "kerts",
            "xmas",
            "christmas",
            "kerstmis",
            "kerstavond",
            "sint",
            "sinterklaas",
            "pakjesavond",
            "nieuwjaar",
            "newyear",
            "oudennieuw",
            "nye",
            "countdown",
            "aftellen",
            "weekend",
        ]
    }

    fn help(&self) -> &'static str {
        "!kerst (of !kerts/!xmas) | !sint | !nieuwjaar | !weekend | !countdown - Feestdagen countdown en aftellen naar Kerstmis en het weekend"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let now = Local::now();
        let today = now.date_naive();

        if cmd.trigger.eq_ignore_ascii_case("weekend") {
            let idx = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;
            return Ok(Some(weekend_message(today.weekday(), idx, ctx.locale.is_dutch())));
        }
        let trigger = cmd.trigger.to_lowercase();

        // 1. Kerstmis Countdown: !kerst / !kerts / !xmas / !christmas / !kerstmis / !kerstavond
        if trigger == "kerst"
            || trigger == "kerts"
            || trigger == "xmas"
            || trigger == "christmas"
            || trigger == "kerstmis"
            || trigger == "kerstavond"
        {
            let (month, day) = (today.month(), today.day());

            if month == 12 && day == 24 {
                return Ok(Some("🎄 Vanavond is het Kerstavond! 🎅 Morgen is het Eerste Kerstdag! Gezellige feestdagen gewenst! ✨".into()));
            } else if month == 12 && day == 25 {
                return Ok(Some("🎄 Vrolijk Kerstfeest! 🎅 Vandaag is het Eerste Kerstdag! Geniet van het samenzijn en lekker eten! ✨🍗".into()));
            } else if month == 12 && day == 26 {
                return Ok(Some("🎄 Fijne Tweede Kerstdag! 🎅 Geniet nog even heerlijk na van het kerstfeest! ✨🎁".into()));
            }

            let (days, eve_days, _) = Self::days_until_christmas(today);

            if days == 1 {
                return Ok(Some("🎄 Nog maar 1 nachtje slapen tot Kerstmis! 🎅 (Vanavond is het Kerstavond! ✨)".into()));
            }

            return Ok(Some(format!(
                "🎄 Nog \x02{} dagen\x02 tot Kerstmis! 🎅 (en \x02{} dagen\x02 tot Kerstavond ✨)",
                days, eve_days
            )));
        }

        // 2. Sinterklaas Countdown: !sint / !sinterklaas / !pakjesavond
        if trigger == "sint" || trigger == "sinterklaas" || trigger == "pakjesavond" {
            let (month, day) = (today.month(), today.day());

            if month == 12 && day == 5 {
                return Ok(Some("🎁 Vandaag is het Pakjesavond! 🐴 Heel veel plezier met surprises, gedichten en pepernoten! 🍫✨".into()));
            }

            let (days, _) = Self::days_until_sinterklaas(today);

            if days == 1 {
                return Ok(Some("🎁 Morgen is het Pakjesavond! 🐴 Zet vanavond je schoen nog maar even klaar! 🍫✨".into()));
            }

            return Ok(Some(format!(
                "🎁 Nog \x02{} dagen\x02 tot Pakjesavond (Sinterklaas)! 🐴🍫",
                days
            )));
        }

        // 3. Oud & Nieuw / Nieuwjaar Countdown: !nieuwjaar / !newyear / !oudennieuw / !nye
        if trigger == "nieuwjaar" || trigger == "newyear" || trigger == "oudennieuw" || trigger == "nye" {
            let (month, day) = (today.month(), today.day());

            if month == 1 && day == 1 {
                return Ok(Some(format!(
                    "🎆 Gelukkig Nieuwjaar! 🥂 Beste wensen voor \x02{}\x02! Maak er een prachtig en gezond jaar van! 🍾✨",
                    today.year()
                )));
            } else if month == 12 && day == 31 {
                let midnight = NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap();
                let diff = midnight.signed_duration_since(now.naive_local());
                let hours = diff.num_hours();
                let mins = diff.num_minutes() % 60;
                return Ok(Some(format!(
                    "🥂 Vandaag is Oudejaarsavond! Nog \x02{} uur en {} minuten\x02 tot de jaarwisseling! 🎆🍾",
                    hours, mins
                )));
            }

            let (days, _) = Self::days_until_new_year(today);
            return Ok(Some(format!(
                "🎆 Nog \x02{} dagen\x02 tot de jaarwisseling (Oud & Nieuw)! 🥂🍾",
                days
            )));
        }

        // 4. Algemeen feestdagen overzicht of custom datum: !countdown / !aftellen
        let args = cmd.args.trim();
        if args.is_empty() {
            let (kerst_days, _, _) = Self::days_until_christmas(today);
            let (sint_days, _) = Self::days_until_sinterklaas(today);
            let (nye_days, _) = Self::days_until_new_year(today);

            return Ok(Some(format!(
                "🎉 [Feestdagen Aftellen] 🎁 Sinterklaas: nog \x02{}d\x02 | 🎄 Kerst: nog \x02{}d\x02 | 🎆 Oud & Nieuw: nog \x02{}d\x02",
                sint_days, kerst_days, nye_days
            )));
        }

        // Custom datum parseren (bijv. !countdown 2026-10-15 of !countdown 15-10)
        let clean_arg = args.replace(['/', '.'], "-");
        let parts: Vec<&str> = clean_arg.split_whitespace().next().unwrap_or("").split('-').collect();

        if parts.len() >= 2 {
            let (day, month, year) = if parts[0].len() == 4 {
                // YYYY-MM-DD
                let y: i32 = parts[0].parse().unwrap_or(today.year());
                let m: u32 = parts[1].parse().unwrap_or(0);
                let d: u32 = parts[2].parse().unwrap_or(0);
                (d, m, y)
            } else {
                // DD-MM of DD-MM-YYYY
                let d: u32 = parts[0].parse().unwrap_or(0);
                let m: u32 = parts[1].parse().unwrap_or(0);
                let y: i32 = if parts.len() >= 3 {
                    parts[2].parse().unwrap_or(today.year())
                } else if m < today.month() || (m == today.month() && d < today.day()) {
                    today.year() + 1
                } else {
                    today.year()
                };
                (d, m, y)
            };

            if let Some(target) = NaiveDate::from_ymd_opt(year, month, day) {
                let diff = (target - today).num_days();
                return if diff > 0 {
                    Ok(Some(format!(
                        "⏳ Nog \x02{} dagen\x02 tot {}!",
                        diff,
                        target.format("%d-%m-%Y")
                    )))
                } else if diff == 0 {
                    Ok(Some(format!("🎉 Vandaag is de dag: {}!", target.format("%d-%m-%Y"))))
                } else {
                    Ok(Some(format!(
                        "⏳ Die datum ({}) was al \x02{} dagen\x02 geleden!",
                        target.format("%d-%m-%Y"),
                        diff.abs()
                    )))
                };
            }
        }

        Ok(Some(
            "ℹ️ Gebruik: !kerst (of !kerts/!xmas) | !sint | !nieuwjaar | !countdown [dd-mm(-jjjj)]".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_christmas_countdown() {
        let sep28 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let (days, eve_days, is_today) = CountdownPlugin::days_until_christmas(sep28);
        assert_eq!(days, 88);
        assert_eq!(eve_days, 87);
        assert!(!is_today);

        let dec24 = NaiveDate::from_ymd_opt(2026, 12, 24).unwrap();
        let (_, _, is_today_24) = CountdownPlugin::days_until_christmas(dec24);
        assert!(is_today_24);

        let dec25 = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        let (_, _, is_today_25) = CountdownPlugin::days_until_christmas(dec25);
        assert!(is_today_25);

        let dec28 = NaiveDate::from_ymd_opt(2026, 12, 28).unwrap();
        let (next_year_days, _, _) = CountdownPlugin::days_until_christmas(dec28);
        assert_eq!(next_year_days, 362);
    }

    #[test]
    fn test_sinterklaas_countdown() {
        let sep28 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let (days, is_today) = CountdownPlugin::days_until_sinterklaas(sep28);
        assert_eq!(days, 68);
        assert!(!is_today);

        let dec5 = NaiveDate::from_ymd_opt(2026, 12, 5).unwrap();
        let (_, is_sint_today) = CountdownPlugin::days_until_sinterklaas(dec5);
        assert!(is_sint_today);
    }

    #[test]
    fn test_new_year_countdown() {
        let sep28 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let (days, is_today) = CountdownPlugin::days_until_new_year(sep28);
        assert_eq!(days, 95);
        assert!(!is_today);

        let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let (_, is_nye_today) = CountdownPlugin::days_until_new_year(jan1);
        assert!(is_nye_today);
    }

    #[test]
    fn weekend_messages() {
        assert!(weekend_message(Weekday::Sat, 0, true).starts_with("Ja! Het is weekend"));
        assert!(weekend_message(Weekday::Sun, 0, false).starts_with("Yes! It's the weekend"));
        assert!(weekend_message(Weekday::Mon, 0, true).contains("nog 5 dagen"));
        assert!(weekend_message(Weekday::Wed, 0, true).contains("nog 3 dagen"));
        assert!(weekend_message(Weekday::Fri, 0, true).contains("morgen is het zover"));
        assert!(weekend_message(Weekday::Fri, 0, false).contains("tomorrow"));
        // spreukindex valt netjes rond
        assert_eq!(weekend_message(Weekday::Tue, 1, true), weekend_message(Weekday::Tue, 6, true));
    }
}
