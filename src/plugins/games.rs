use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use rand::Rng;
use std::collections::HashMap;
use std::sync::Mutex;

struct RouletteGame {
    bullet_chamber: u8,
    current_chamber: u8,
}

impl RouletteGame {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            bullet_chamber: rng.gen_range(1..=6),
            current_chamber: 0,
        }
    }
}

pub struct GamesPlugin {
    roulette: Mutex<HashMap<String, RouletteGame>>,
}

impl GamesPlugin {
    pub fn new() -> Self {
        Self {
            roulette: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Plugin for GamesPlugin {
    fn name(&self) -> &'static str { "games" }
    fn triggers(&self) -> &[&'static str] {
        &["roulette", "8ball", "roll", "dobbel", "flip", "munt", "choose", "kies"]
    }
    fn help(&self) -> &'static str {
        "!roulette - Russisch roulette (kick bij bang!) | !8ball <vraag> | !roll [NdM] | !flip - Kop of munt | !choose <a | b | c>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        match cmd.trigger.as_str() {
            "roulette" => {
                let chan_key = cmd.channel.to_lowercase();
                let (is_bang, chamber, left) = {
                    let mut games = self.roulette.lock().unwrap();
                    let game = games.entry(chan_key.clone()).or_insert_with(RouletteGame::new);
                    game.current_chamber += 1;
                    let chamber = game.current_chamber;
                    if chamber == game.bullet_chamber {
                        *game = RouletteGame::new();
                        (true, chamber, 0)
                    } else {
                        (false, chamber, 6 - chamber)
                    }
                };

                if is_bang {
                    // BANG!
                    let kick_reason = "*BANG!* 💥 Verloren met Russisch Roulette!";
                    if cmd.platform == "irc" && cmd.channel.starts_with('#') {
                        ctx.send_irc_raw(format!("KICK {} {} :{}", cmd.channel, cmd.author, kick_reason)).await;
                    }
                    Ok(Some(format!(
                        "💥 \x02*BANG!*\x02 De kogel treft \x02{}\x02 vol tussen de ogen! (Kamer {}/6) De revolver is opnieuw geladen.",
                        cmd.author, chamber
                    )))
                } else {
                    Ok(Some(format!(
                        "🔫 \x02*klik*\x02 ... Niets! De kamer was leeg. \x02{}\x02 overleeft! (Kamer {}/6, nog {} over)",
                        cmd.author, chamber, left
                    )))
                }
            }
            "8ball" => {
                let question = cmd.args.trim();
                if question.is_empty() {
                    return Ok(Some("🎱 Stel een vraag aan de Magic 8-Ball: !8ball <jouw vraag>".into()));
                }

                let answers = [
                    // Positief
                    "Zonder enige twijfel! ✨",
                    "Het ziet er bijzonder rooskleurig uit! 🌟",
                    "Ja, absoluut zeker! 👍",
                    "Je kunt erop rekenen! 🔮",
                    "Zeer waarschijnlijk! 🎯",
                    // Neutraal
                    "Antwoord is wazig, probeer het later nog eens... ⏳",
                    "Beter als ik het je nu niet vertel... 🤐",
                    "Concentreer je goed en vraag opnieuw. 🤔",
                    "Nu niet te voorspellen. 🌫️",
                    // Negatief
                    "Reken er maar niet op. ❌",
                    "Mijn antwoord is een stellige nee. 🛑",
                    "Zeer twijfelachtig... 🌧️",
                    "Mijn bronnen zeggen van niet! 🙅‍♂️",
                    "Vergeet het maar! 💥",
                ];

                let mut rng = rand::thread_rng();
                let answer = answers[rng.gen_range(0..answers.len())];

                Ok(Some(format!("🎱 \x02[8-Ball: {}]\x02 {}", cmd.author, answer)))
            }
            "roll" | "dobbel" => {
                let input = cmd.args.trim().to_lowercase();
                let (count, sides) = if input.is_empty() {
                    (1, 6)
                } else if input.starts_with('d') {
                    let s: u32 = input[1..].parse().unwrap_or(6).clamp(2, 1000);
                    (1, s)
                } else if let Some((c_str, s_str)) = input.split_once('d') {
                    let c: u32 = c_str.parse().unwrap_or(1).clamp(1, 50);
                    let s: u32 = s_str.parse().unwrap_or(6).clamp(2, 1000);
                    (c, s)
                } else {
                    let s: u32 = input.parse().unwrap_or(6).clamp(2, 1000);
                    (1, s)
                };

                let mut rng = rand::thread_rng();
                let mut rolls = Vec::new();
                let mut sum: u64 = 0;
                for _ in 0..count {
                    let val = rng.gen_range(1..=sides);
                    rolls.push(val.to_string());
                    sum += val as u64;
                }

                if count == 1 {
                    Ok(Some(format!("🎲 \x02[Roll: {}]\x02 Gooide 1d{}: \x02{}\x02", cmd.author, sides, sum)))
                } else {
                    Ok(Some(format!(
                        "🎲 \x02[Roll: {}]\x02 Gooide {}d{}: [{}] = Totaal: \x02{}\x02",
                        cmd.author, count, sides, rolls.join(", "), sum
                    )))
                }
            }
            "flip" | "munt" => {
                let mut rng = rand::thread_rng();
                let is_heads = rng.gen_bool(0.5);
                let result = if is_heads { "🪙 \x02KOP\x02!" } else { "🪙 \x02MUNT\x02!" };
                Ok(Some(format!("🪙 [Kop of Munt: {}] De munt landt op: {}", cmd.author, result)))
            }
            "choose" | "kies" => {
                let input = cmd.args.trim();
                if input.is_empty() {
                    return Ok(Some("Gebruik: !choose <optie 1> | <optie 2> | <optie 3>".into()));
                }

                let delimiter = if input.contains('|') { '|' } else { ',' };
                let options: Vec<&str> = input.split(delimiter).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

                if options.is_empty() {
                    return Ok(Some("Geen geldige opties gevonden om uit te kiezen.".into()));
                }

                let mut rng = rand::thread_rng();
                let choice = options[rng.gen_range(0..options.len())];

                Ok(Some(format!("🤔 \x02[Keuze voor {}]\x02 Ik kies voor: \x02{}\x02!", cmd.author, choice)))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roulette_game_init() {
        let game = RouletteGame::new();
        assert!(game.bullet_chamber >= 1 && game.bullet_chamber <= 6);
        assert_eq!(game.current_chamber, 0);
    }
}

