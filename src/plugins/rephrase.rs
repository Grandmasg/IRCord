use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RephraseStyle {
    Clear,    // Klaartaal / B1 niveau / Begrijpelijk
    Eli5,     // Explain Like I'm 5 / Analogie & metafoor
    Business, // Zakelijk / Professioneel / Beleefd
    Concise,  // Beknopt / Kernboodschap in 1 korte regel
}

impl RephraseStyle {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Clear => "✨ [Klaartaal]",
            Self::Eli5 => "🧸 [ELI5]",
            Self::Business => "👔 [Zakelijk]",
            Self::Concise => "✂️ [Beknopt]",
        }
    }

    pub fn prompt(&self, text: &str, lang: &str) -> String {
        match (self, lang) {
            // 1. Klaartaal / B1
            (Self::Clear, "de") => format!(
                "Formuliere den folgenden Text in einfachem, klarem und natürlichem Deutsch um (Sprachniveau B1). \
                Entferne Fachchinesisch, bürokratische Floskeln und unnötige Schachtelsätze, behalte aber den Kern bei. \
                Gib AUSSCHLIESSLICH den umformulierten Satz zurück. Keine Einleitung, keine Anführungszeichen, keine Erklärungen.\n\nText:\n{}",
                text
            ),
            (Self::Clear, "en") => format!(
                "Rewrite the following text into simple, clear, and direct English (B1 level). \
                Remove bureaucratic jargon, buzzwords, and convoluted phrasing while preserving the core meaning. \
                Output ONLY the rewritten sentence. No preamble, no quotes, no explanations.\n\nText:\n{}",
                text
            ),
            (Self::Clear, _) => format!(
                "Herschrijf de volgende tekst in eenvoudig, glashelder en natuurlijk Nederlands (taalniveau B1). \
                Verwijder ambtelijk jargon, wollige formuleringen en nodeloos ingewikkelde zinnen, maar behoud de oorspronkelijke betekenis. \
                Geef UITSLUITEND de herschreven zin terug. Geen inleiding, geen aanhalingstekens, geen uitleg achteraf.\n\nTekst:\n{}",
                text
            ),

            // 2. ELI5 (Explain Like I'm 5)
            (Self::Eli5, "de") => format!(
                "Erkläre die Kernaussage des folgenden Textes so, als würdest du es einem 5-jährigen Kind erklären (ELI5). \
                Nutze eine anschauliche Metapher oder einen einfachen Vergleich auf Deutsch. Maximal 1-2 kurze Sätze. \
                Gib AUSSCHLIESSLICH die Erklärung zurück. Keine Einleitung, keine Begrüßung, keine Anführungszeichen.\n\nText:\n{}",
                text
            ),
            (Self::Eli5, "en") => format!(
                "Explain the core message of the following text like I am 5 years old (ELI5). \
                Use a simple analogy or metaphor in clear English. Maximum 1-2 short sentences. \
                Output ONLY the explanation. No preamble, no greetings, no quotation marks.\n\nText:\n{}",
                text
            ),
            (Self::Eli5, _) => format!(
                "Leg de strekking van de volgende tekst uit alsof je het aan een 5-jarige uitlegt (ELI5). \
                Gebruik een simpele analogie of metafoor en eenvoudige bewoordingen in het Nederlands. Maximaal 1 of 2 korte zinnen. \
                Geef UITSLUITEND de uitleg terug. Geen inleiding, geen begroeting, geen aanhalingstekens.\n\nTekst:\n{}",
                text
            ),

            // 3. Zakelijk / Professioneel
            (Self::Business, "de") => format!(
                "Formuliere folgende Chatnachricht in einen professionellen, höflichen, diplomatischen und geschäftsmäßigen Ton um (geeignet für E-Mails oder Arbeitskontext) auf Deutsch. \
                Gib AUSSCHLIESSLICH den umformulierten Text zurück. Keine Einleitung, keine Anführungszeichen.\n\nText:\n{}",
                text
            ),
            (Self::Business, "en") => format!(
                "Rephrase the following message into a professional, polite, diplomatic, and business-appropriate tone (suitable for work or corporate email) in English. \
                Output ONLY the rewritten text. No preamble, no quotation marks.\n\nText:\n{}",
                text
            ),
            (Self::Business, _) => format!(
                "Herschrijf de volgende chatboodschap in een professionele, vriendelijke, diplomatieke en zakelijke stijl in het Nederlands (geschikt voor een formele werkcontext of e-mail). \
                Geef UITSLUITEND de herschreven tekst terug. Geen inleiding, geen aanhalingstekens.\n\nTekst:\n{}",
                text
            ),

            // 4. Beknopt / Essentie
            (Self::Concise, "de") => format!(
                "Kürze den folgenden Text auf die absolute Kernaussage in genau 1 prägnanten, starken deutschen Satz. \
                Gib AUSSCHLIESSLICH diesen einen Satz zurück. Keine Einleitung, keine Anführungszeichen.\n\nText:\n{}",
                text
            ),
            (Self::Concise, "en") => format!(
                "Strip away all noise and fluff from the following text and state the absolute core takeaway in exactly 1 concise, punchy English sentence. \
                Output ONLY that single sentence. No preamble, no quotation marks.\n\nText:\n{}",
                text
            ),
            (Self::Concise, _) => format!(
                "Snijd alle ruis en bijzaken weg uit de volgende tekst en formuleer de absolute kernboodschap in exact 1 korte, krachtige zin in het Nederlands. \
                Geef UITSLUITEND de beknopte zin terug. Geen inleiding, geen aanhalingstekens.\n\nTekst:\n{}",
                text
            ),
        }
    }
}

#[derive(Clone, Debug)]
struct CachedMessage {
    author: String,
    content: String,
}

pub struct RephrasePlugin {
    recent_messages: Mutex<HashMap<String, VecDeque<CachedMessage>>>,
}

impl RephrasePlugin {
    pub fn new() -> Self {
        Self {
            recent_messages: Mutex::new(HashMap::new()),
        }
    }

    /// Analyseert trigger en argumenten om de stijl en doellading te bepalen
    pub fn parse_invocation(trigger: &str, raw_args: &str) -> (RephraseStyle, String) {
        let trimmed_args = raw_args.trim();

        // 1. Directe stijl via commando-trigger
        match trigger {
            "eli5" => return (RephraseStyle::Eli5, trimmed_args.to_string()),
            "zakelijk" | "corporate" | "prof" => return (RephraseStyle::Business, trimmed_args.to_string()),
            "beknopt" | "kort" => return (RephraseStyle::Concise, trimmed_args.to_string()),
            _ => {}
        }

        // 2. Trigger was !rephrase / !herschrijf / !klaartaal -> controleer eventueel stijlprefix als eerste woord
        let mut words = trimmed_args.splitn(2, ' ');
        let first = words.next().unwrap_or("").to_lowercase();
        let rest = words.next().unwrap_or("").trim();

        match first.as_str() {
            "eli5" => (RephraseStyle::Eli5, rest.to_string()),
            "zakelijk" | "corporate" | "prof" => (RephraseStyle::Business, rest.to_string()),
            "beknopt" | "kort" => (RephraseStyle::Concise, rest.to_string()),
            "klaartaal" | "simpel" | "b1" => (RephraseStyle::Clear, rest.to_string()),
            _ => (RephraseStyle::Clear, trimmed_args.to_string()),
        }
    }
}

#[async_trait]
impl Plugin for RephrasePlugin {
    fn name(&self) -> &'static str {
        "rephrase"
    }

    fn triggers(&self) -> &[&'static str] {
        &[
            "rephrase",
            "herschrijf",
            "klaartaal",
            "eli5",
            "zakelijk",
            "corporate",
            "beknopt",
            "kort",
        ]
    }

    fn help(&self) -> &'static str {
        "!rephrase [tekst|nick] | !eli5 [tekst|nick] | !zakelijk [tekst] | !beknopt [tekst] - Herschrijft moeilijke of wollige zinnen in heldere taal"
    }

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trimmed = msg.content.trim();

        // Sla geen commando's of bot-antwoorden op in de recente geschiedenis
        if ctx.config.general.is_command_trigger(trimmed)
            || msg.author.eq_ignore_ascii_case("IRCord")
            || msg.author.eq_ignore_ascii_case("Monkeybot")
        {
            return Ok(None);
        }

        // Bewaar de laatste 25 berichten per kanaal in het ringbuffer
        let mut lock = self.recent_messages.lock().unwrap();
        let queue = lock.entry(msg.channel.clone()).or_default();

        if queue.len() >= 25 {
            queue.pop_front();
        }

        queue.push_back(CachedMessage {
            author: msg.author.clone(),
            content: trimmed.to_string(),
        });

        Ok(None)
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let (style, target_arg) = Self::parse_invocation(&cmd.trigger, &cmd.args);

        let mut target_author: Option<String> = None;
        let mut target_text = String::new();

        // 1. Bepaal de brontekst:
        if target_arg.is_empty() {
            // Geen argumenten -> pak het laatste bericht van een ander in het actieve kanaal
            let lock = self.recent_messages.lock().unwrap();
            if let Some(queue) = lock.get(&cmd.channel) {
                for item in queue.iter().rev() {
                    if !item.author.eq_ignore_ascii_case(&cmd.author) {
                        target_author = Some(item.author.clone());
                        target_text = item.content.clone();
                        break;
                    }
                }
            }
        } else if !target_arg.contains(' ') {
            // Enkel woord: controleer eerst of dit overeenkomt met een actieve nick in het kanaal
            let lock = self.recent_messages.lock().unwrap();
            let mut found_by_nick = false;
            if let Some(queue) = lock.get(&cmd.channel) {
                for item in queue.iter().rev() {
                    if item.author.eq_ignore_ascii_case(&target_arg) {
                        target_author = Some(item.author.clone());
                        target_text = item.content.clone();
                        found_by_nick = true;
                        break;
                    }
                }
            }

            if !found_by_nick {
                // Was geen nick, maar gewoon een enkel woord of korte kreet
                target_text = target_arg;
            }
        } else {
            // Meerdere woorden -> directe tekst van de gebruiker
            target_text = target_arg;
        }

        // 2. Als er in het geheugen niets gevonden is (bijv. vlak na een herstart), zoek in SQLite FTS5 geschiedenis
        if target_text.is_empty() {
            let row: Result<Option<(String, String)>, sqlx::Error> = sqlx::query_as(
                r#"
                SELECT author, message
                FROM chat_history
                WHERE channel = ? AND author != ? AND author != 'IRCord' AND author != 'Monkeybot'
                ORDER BY rowid DESC
                LIMIT 1
                "#,
            )
            .bind(&cmd.channel)
            .bind(&cmd.author)
            .fetch_optional(&ctx.db)
            .await;

            if let Ok(Some((author, msg))) = row {
                target_author = Some(author);
                target_text = msg;
            }
        }

        // 3. Nog steeds leeg? Toon behulpzame helptekst
        if target_text.trim().is_empty() {
            return Ok(Some(
                "ℹ️ Gebruik: !rephrase [tekst] | !rephrase <nick> | !eli5 [tekst] | !zakelijk [tekst] | !beknopt [tekst] (of typ direct na een moeilijk bericht)".into()
            ));
        }

        // 4. Token budget controle
        if !ctx.ai_manager.can_consume(150) {
            return Ok(Some("⚠️ Het AI tokenbudget voor dit uur is bereikt. Probeer het later nog eens.".into()));
        }

        // 5. Prompt genereren en uitvoeren via lokale LLM (Ollama)
        let effective_lang = ctx.locale.language();
        let prompt = style.prompt(&target_text, effective_lang);
        let current_model = ctx.ai_manager.get_model();

        let ai_result = match ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await {
            Ok(ans) => ans,
            Err(e) => {
                warn!("Fout bij herschrijven via AI: {}", e);
                return Ok(Some("⚠️ Kon de zin momenteel niet herschrijven via de AI service.".into()));
            }
        };

        ctx.ai_manager.record_consumption(120);

        // Schoon de uitvoer op van eventuele omhullende aanhalingstekens of newlines
        let cleaned = ai_result
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .replace('\n', " ");

        // 6. Formatteer de uitvoer voor IRC of Discord
        let badge = style.badge();

        if let Some(author) = target_author {
            if cmd.platform == "discord" {
                let quote_preview = if target_text.len() > 100 {
                    format!("{}...", &target_text[..97])
                } else {
                    target_text
                };
                Ok(Some(format!(
                    "> *<{}> {}*\n**{}**: {}",
                    author, quote_preview, badge, cleaned
                )))
            } else {
                Ok(Some(format!("{} <{}>: {}", badge, author, cleaned)))
            }
        } else {
            if cmd.platform == "discord" {
                Ok(Some(format!("**{}**: {}", badge, cleaned)))
            } else {
                Ok(Some(format!("{}: {}", badge, cleaned)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_invocation_styles() {
        let (s1, a1) = RephrasePlugin::parse_invocation("eli5", "Quantum computing");
        assert_eq!(s1, RephraseStyle::Eli5);
        assert_eq!(a1, "Quantum computing");

        let (s2, a2) = RephrasePlugin::parse_invocation("zakelijk", "Schiet eens op man");
        assert_eq!(s2, RephraseStyle::Business);
        assert_eq!(a2, "Schiet eens op man");

        let (s3, a3) = RephrasePlugin::parse_invocation("beknopt", "Lang verhaal kort");
        assert_eq!(s3, RephraseStyle::Concise);
        assert_eq!(a3, "Lang verhaal kort");

        let (s4, a4) = RephrasePlugin::parse_invocation("rephrase", "eli5 Het universum expandeert");
        assert_eq!(s4, RephraseStyle::Eli5);
        assert_eq!(a4, "Het universum expandeert");

        let (s5, a5) = RephrasePlugin::parse_invocation("rephrase", "Gewoon een moeilijke zin");
        assert_eq!(s5, RephraseStyle::Clear);
        assert_eq!(a5, "Gewoon een moeilijke zin");
    }
}
