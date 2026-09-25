use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use scraper::{Html, Selector};
use tracing::info;

pub struct AiPlugin;

impl AiPlugin {
    /// Hulpfunctie om AI antwoorden voor IRC op te knippen in logische zinsgrenzen
    fn format_for_irc(text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for raw_line in text.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() <= 350 {
                lines.push(trimmed.to_string());
            } else {
                // Splits op zinsgrenzen
                let mut current = String::new();
                for sentence in trimmed.split_inclusive(&['.', '!', '?'][..]) {
                    if current.len() + sentence.len() > 350 {
                        if !current.is_empty() {
                            lines.push(current.trim().to_string());
                            current.clear();
                        }
                    }
                    current.push_str(sentence);
                }
                if !current.is_empty() {
                    lines.push(current.trim().to_string());
                }
            }
        }
        lines
    }
}

#[async_trait]
impl Plugin for AiPlugin {
    fn name(&self) -> &'static str { "ai" }
    fn triggers(&self) -> &[&'static str] { &["ai", "tldr", "summary", "topic", "roast", "whatis", "def", "catchup", "digest", "vibe", "sentiment"] }
    fn help(&self) -> &'static str {
        "!ai <vraag> | !catchup [aantal] | !vibe | !tldr [url] | !topic suggest | !roast <nick> | !whatis <begrip>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();

        match cmd.trigger.as_str() {
            "ai" => {
                if args.is_empty() {
                    return Ok(Some("Gebruik: !ai <jouw vraag> | !ai models | !ai model <naam> (admin)".into()));
                }

                // 1. Model inspectie: !ai models of !ai model
                if args.eq_ignore_ascii_case("models") || args.eq_ignore_ascii_case("model") {
                    let current = ctx.ai_manager.get_model();
                    return match ctx.ai_client.list_models().await {
                        Ok(list) if !list.is_empty() => {
                            Ok(Some(format!("🧠 [AI Modellen] Actief: '{}' | Beschikbaar op server: [{}]", current, list.join(", "))))
                        }
                        _ => Ok(Some(format!("🧠 [AI Modellen] Actief model: '{}' (kon server-modellenlijst niet ophalen)", current))),
                    };
                }

                // 2. Model wisselen (strikte Admin/Operator autorisatie): !ai model <nieuw_model>
                if args.to_lowercase().starts_with("model ") {
                    if !cmd.is_owner && !cmd.is_operator {
                        return Ok(Some("⛔ Alleen de bot eigenaar of operators mogen het AI model wijzigen.".into()));
                    }

                    let new_model = args[6..].trim();
                    if new_model.is_empty() {
                        return Ok(Some("Gebruik: !ai model <naam> (bijv: !ai model qwen2.5-coder)".into()));
                    }

                    ctx.ai_manager.set_model(new_model.to_string());
                    return Ok(Some(format!("✅ [AI Model] Actief model gewijzigd naar '{}'", new_model)));
                }

                // 3. Reguliere AI prompt
                if args.len() > 1200 {
                    return Ok(Some("⚠️ Je vraag is te lang (maximaal 1200 tekens toegestaan).".into()));
                }

                if !ctx.ai_manager.can_consume(300) {
                    return Ok(Some("⚠️ Het tokenbudget voor dit uur is bereikt. Probeer het later nog eens.".into()));
                }

                info!("Aanroepen FreeToken model voor {} op kanaal {}", cmd.author, cmd.channel);
                let current_model = ctx.ai_manager.get_model();
                let answer = ctx.ai_client.ask(&cmd.author, args, Some(&current_model)).await?;
                ctx.ai_manager.record_consumption(150);

                if cmd.platform == "irc" {
                    let lines = Self::format_for_irc(&answer);
                    let formatted = if lines.len() == 1 {
                        format!("🧠 [AI]: {}", lines[0])
                    } else {
                        lines.join(" | ")
                    };
                    Ok(Some(formatted))
                } else {
                    Ok(Some(format!("🧠 **[AI]**: {}", answer)))
                }
            }

            "tldr" | "summary" => {
                // A. Webpagina URL samenvatting: !tldr https://...
                if args.starts_with("http://") || args.starts_with("https://") {
                    let url = args.split_whitespace().next().unwrap_or(args);

                    // SSRF-beveiliging tegen interne netwerkadressen
                    if !crate::plugins::url_titler::UrlTitlerPlugin::is_safe_public_url(url) {
                        return Ok(Some("⛔ Deze URL is niet toegestaan (interne en beveiligde adressen worden geblokkeerd).".into()));
                    }

                    let resp = match ctx.http.get(url)
                        .timeout(std::time::Duration::from_secs(6))
                        .header("User-Agent", "Mozilla/5.0 (compatible; IRCordBot/1.0)")
                        .send()
                        .await
                    {
                        Ok(r) if r.status().is_success() => r,
                        _ => return Ok(Some(format!("📝 Kon de pagina op '{}' niet ophalen voor een samenvatting.", url))),
                    };

                    let body = resp.text().await.unwrap_or_default();
                    let extracted_text = {
                        let document = Html::parse_document(&body);
                        let p_selector = Selector::parse("p, article p").unwrap();
                        let mut text = String::new();

                        for el in document.select(&p_selector).take(12) {
                            let t = el.text().collect::<Vec<_>>().join(" ");
                            if t.trim().len() > 20 {
                                text.push_str(t.trim());
                                text.push(' ');
                            }
                        }
                        text
                    };

                    if extracted_text.trim().is_empty() {
                        return Ok(Some("📝 Kon geen leesbare artikeltekst vinden op de pagina om samen te vatten.".into()));
                    }

                    let prompt = format!(
                        "Vat de volgende artikeltekst beknopt samen in 2 feitelijke, to-the-point zinnen (maximaal 250 tekens in totaal):\n{}",
                        extracted_text.chars().take(2000).collect::<String>()
                    );

                    let current_model = ctx.ai_manager.get_model();
                    let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                    let clean = summary.replace('\n', " • ");

                    return Ok(Some(format!("📝 [TL;DR Web]: {}", clean)));
                }

                // B. Kanaalgeschiedenis RAG samenvatting
                let recent_logs = ctx.rag.search_history(&cmd.channel, 20).await?;
                if recent_logs.is_empty() {
                    return Ok(Some("Er zijn nog niet genoeg recente chatberichten gelogd voor een samenvatting.".into()));
                }

                let context_text = recent_logs.join("\n");
                let prompt = format!(
                    "Vat de volgende recente chatgesprekken samen in 3 zeer korte, to-the-point bullet points:\n{}",
                    context_text
                );

                let current_model = ctx.ai_manager.get_model();
                let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;

                if cmd.platform == "irc" {
                    let clean = summary.replace('\n', " • ");
                    Ok(Some(format!("📝 [TL;DR]: {}", clean)))
                } else {
                    Ok(Some(format!("📝 **[TL;DR Samenvatting]**:\n{}", summary)))
                }
            }

            "topic" => {
                let recent_logs = ctx.rag.search_history(&cmd.channel, 15).await?;
                let context_text = recent_logs.join("\n");
                let prompt = format!(
                    "Bedenk op basis van deze recente chat een spitsvondig, kort en relevant kanaaltopic (max 1 regel):\n{}",
                    context_text
                );

                let current_model = ctx.ai_manager.get_model();
                let topic_suggestion = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;

                Ok(Some(format!("💡 [Topic Suggestie]: {}", topic_suggestion.replace('\n', " "))))
            }

            "roast" => {
                let target = if args.is_empty() { cmd.author.as_str() } else { args };
                let prompt = format!(
                    "Bedenk een gevatte, humoristische en spitsvondige roast over de gebruiker '{}' in klassieke nerdy IRC-stijl. Maximaal 1 à 2 zinnen, geen haatzaaien of grove beledigingen, puur speelse en gevatte nerd/hacker-humor.",
                    target
                );

                let current_model = ctx.ai_manager.get_model();
                let roast = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                Ok(Some(format!("🔥 [Roast]: {}", roast.replace('\n', " "))))
            }

            "whatis" | "def" => {
                if args.is_empty() {
                    return Ok(Some("Gebruik: !whatis <begrip> (bijv: !whatis BGP of !whatis Docker)".into()));
                }

                let prompt = format!(
                    "Geef een vlijmscherpe, feitelijke en nuchtere definitie van exact 1 regel voor het volgende begrip: {}",
                    args
                );

                let current_model = ctx.ai_manager.get_model();
                let def = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                Ok(Some(format!("💡 [Definitie]: {}", def.replace('\n', " "))))
            }

            "catchup" | "digest" => {
                if !ctx.ai_manager.can_consume(300) {
                    return Ok(Some("⚠️ Het tokenbudget voor dit uur is bereikt. Probeer het later nog eens.".into()));
                }

                let count = if let Ok(n) = args.parse::<i64>() {
                    n.clamp(10, 60)
                } else {
                    35
                };

                let recent_logs = ctx.rag.search_history(&cmd.channel, count).await?;
                if recent_logs.is_empty() {
                    return Ok(Some("ℹ️ Er is nog niet genoeg recente chatgeschiedenis om een samenvatting te maken.".into()));
                }

                let logs_text = recent_logs.join("\n");
                let prompt = format!(
                    "Vat de recente chatgesprekken in kanaal '{}' beknopt samen voor de terugkerende gebruiker '{}' in maximaal 3 korte bullet points in het Nederlands. Noem expliciet of '{}' werd genoemd of gezocht, en of er concrete afspraken zijn gemaakt:\n\n{}",
                    cmd.channel, cmd.author, cmd.author, logs_text
                );

                let current_model = ctx.ai_manager.get_model();
                let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                ctx.ai_manager.record_consumption(200);

                if cmd.platform == "irc" {
                    let lines = Self::format_for_irc(&summary);
                    Ok(Some(format!("📰 [Catch-up {}]: {}", cmd.author, lines.join(" | "))))
                } else {
                    Ok(Some(format!("📰 **[Catch-up voor {}]**:\n{}", cmd.author, summary)))
                }
            }

            "vibe" | "sentiment" => {
                if !ctx.ai_manager.can_consume(200) {
                    return Ok(Some("⚠️ Het tokenbudget voor dit uur is bereikt. Probeer het later nog eens.".into()));
                }

                let recent_logs = ctx.rag.search_history(&cmd.channel, 30).await?;
                if recent_logs.is_empty() {
                    return Ok(Some("ℹ️ Er is nog niet genoeg chatgeschiedenis om de sfeer te peilen.".into()));
                }

                let logs_text = recent_logs.join("\n");
                let prompt = format!(
                    "Peil in maximaal 1 of 2 humoristische, opgewekte zinnen de actuele sfeer en stemming in dit chatkanaal op basis van deze recente berichten. Noem een percentage gezelligheid/vrolijkheid en de 2 heetste onderwerpen in het Nederlands:\n\n{}",
                    logs_text
                );

                let current_model = ctx.ai_manager.get_model();
                let vibe = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                ctx.ai_manager.record_consumption(120);

                Ok(Some(format!("🌡️ [Kanaal Vibe]: {}", vibe.replace('\n', " "))))
            }

            _ => Ok(None),
        }
    }
}
