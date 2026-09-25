use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use scraper::{Html, Selector};
use tracing::info;

pub struct AiPlugin;

impl AiPlugin {
    /// Helper function to split AI responses for IRC on logical sentence boundaries
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
                // Split on sentence boundaries
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
        "!ai <question> | !catchup [count] | !vibe | !tldr [url] | !topic | !roast <nick> | !whatis <term>"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();

        match cmd.trigger.as_str() {
            "ai" => {
                if args.is_empty() {
                    return Ok(Some(ctx.locale.t("ai_usage").to_string()));
                }

                // 1. Model inspection: !ai models or !ai model
                if args.eq_ignore_ascii_case("models") || args.eq_ignore_ascii_case("model") {
                    let current = ctx.ai_manager.get_model();
                    return match ctx.ai_client.list_models().await {
                        Ok(list) if !list.is_empty() => {
                            let msg = ctx.locale.tf("ai_models_active", &[("model", &current), ("list", &list.join(", "))]);
                            Ok(Some(format!("🧠 [AI] {}", msg)))
                        }
                        _ => {
                            let msg = ctx.locale.tf("ai_models_err", &[("model", &current)]);
                            Ok(Some(format!("🧠 [AI] {}", msg)))
                        }
                    };
                }

                // 2. Model switch (strict Admin/Operator authorization): !ai model <new_model>
                if args.to_lowercase().starts_with("model ") {
                    if !cmd.is_owner && !cmd.is_operator {
                        return Ok(Some(format!("⛔ {}", ctx.locale.t("ai_admin_only"))));
                    }

                    let new_model = args[6..].trim();
                    if new_model.is_empty() {
                        return Ok(Some(ctx.locale.t("ai_model_usage").to_string()));
                    }

                    ctx.ai_manager.set_model(new_model.to_string());
                    return Ok(Some(format!("✅ {}", ctx.locale.tf("ai_model_changed", &[("model", new_model)]))));
                }

                // 3. Regular AI prompt
                if args.len() > 1200 {
                    return Ok(Some(format!("⚠️ {}", ctx.locale.t("ai_too_long"))));
                }

                if !ctx.ai_manager.can_consume(300) {
                    return Ok(Some(format!("⚠️ {}", ctx.locale.t("ai_budget_exceeded"))));
                }

                info!("Calling FreeToken model for {} on channel {}", cmd.author, cmd.channel);
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
                // A. Webpage URL summary: !tldr https://...
                if args.starts_with("http://") || args.starts_with("https://") {
                    let url = args.split_whitespace().next().unwrap_or(args);

                    // SSRF protection against internal network addresses
                    if !crate::plugins::url_titler::UrlTitlerPlugin::is_safe_public_url(url) {
                        return Ok(Some(format!("⛔ {}", ctx.locale.t("ai_url_blocked"))));
                    }

                    let resp = match ctx.http.get(url)
                        .timeout(std::time::Duration::from_secs(6))
                        .header("User-Agent", "Mozilla/5.0 (compatible; IRCordBot/1.0)")
                        .send()
                        .await
                    {
                        Ok(r) if r.status().is_success() => r,
                        _ => return Ok(Some(format!("📝 {}", ctx.locale.tf("ai_url_failed", &[("url", url)])))),
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
                        return Ok(Some(format!("📝 {}", ctx.locale.t("ai_no_article_text"))));
                    }

                    let prompt = match ctx.locale.language() {
                        "nl" => format!(
                            "Vat de volgende artikeltekst beknopt samen in 2 feitelijke, to-the-point zinnen (maximaal 250 tekens in totaal):\n{}",
                            extracted_text.chars().take(2000).collect::<String>()
                        ),
                        "de" => format!(
                            "Fasse den folgenden Artikeltext prägnant in 2 sachlichen Sätzen zusammen (maximal 250 Zeichen insgesamt):\n{}",
                            extracted_text.chars().take(2000).collect::<String>()
                        ),
                        _ => format!(
                            "Summarize the following article text concisely in 2 factual, to-the-point sentences (maximum 250 characters in total):\n{}",
                            extracted_text.chars().take(2000).collect::<String>()
                        ),
                    };

                    let current_model = ctx.ai_manager.get_model();
                    let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                    let clean = summary.replace('\n', " • ");

                    return Ok(Some(format!("📝 [{}]: {}", ctx.locale.t("ai_tldr_web"), clean)));
                }

                // B. Channel history RAG summary
                let recent_logs = ctx.rag.search_history(&cmd.channel, 20).await?;
                if recent_logs.is_empty() {
                    return Ok(Some(ctx.locale.t("ai_no_history").to_string()));
                }

                let context_text = recent_logs.join("\n");
                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Vat de volgende recente chatgesprekken samen in 3 zeer korte, to-the-point bullet points:\n{}",
                        context_text
                    ),
                    "de" => format!(
                        "Fasse die folgenden aktuellen Chat-Gespräche in 3 sehr kurzen, prägnanten Stichpunkten zusammen:\n{}",
                        context_text
                    ),
                    _ => format!(
                        "Summarize the following recent chat conversations in 3 very short, to-the-point bullet points:\n{}",
                        context_text
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;

                if cmd.platform == "irc" {
                    let clean = summary.replace('\n', " • ");
                    Ok(Some(format!("📝 [{}]: {}", ctx.locale.t("ai_tldr_chat"), clean)))
                } else {
                    Ok(Some(format!("📝 **[{}]**:\n{}", ctx.locale.t("ai_tldr_chat"), summary)))
                }
            }

            "topic" => {
                let recent_logs = ctx.rag.search_history(&cmd.channel, 15).await?;
                let context_text = recent_logs.join("\n");
                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Bedenk op basis van deze recente chat een spitsvondig, kort en relevant kanaaltopic (max 1 regel):\n{}",
                        context_text
                    ),
                    "de" => format!(
                        "Erstelle basierend auf diesem aktuellen Chat ein witziges, kurzes und relevantes Kanalthema (max. 1 Zeile):\n{}",
                        context_text
                    ),
                    _ => format!(
                        "Based on this recent chat, suggest a witty, concise, and relevant channel topic (max 1 line):\n{}",
                        context_text
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let topic_suggestion = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;

                Ok(Some(format!("💡 [{}]: {}", ctx.locale.t("ai_topic_title"), topic_suggestion.replace('\n', " "))))
            }

            "roast" => {
                let target = if args.is_empty() { cmd.author.as_str() } else { args };
                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Bedenk een gevatte, humoristische en spitsvondige roast over de gebruiker '{}' in klassieke nerdy IRC-stijl. Maximaal 1 à 2 zinnen, geen haatzaaien of grove beledigingen, puur speelse en gevatte nerd/hacker-humor.",
                        target
                    ),
                    "de" => format!(
                        "Erstelle einen witzigen, humorvollen und schlagfertigen Roast über den Benutzer '{}' im klassischen Nerd-IRC-Stil. Maximal 1-2 Sätze, kein Hass, rein spielerischer Hacker-Humor.",
                        target
                    ),
                    _ => format!(
                        "Devise a witty, humorous, and sharp roast about user '{}' in classic nerdy IRC style. Maximum 1-2 sentences, no hate speech or slurs, purely playful nerd/hacker humor.",
                        target
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let roast = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                Ok(Some(format!("🔥 [{}]: {}", ctx.locale.t("ai_roast_title"), roast.replace('\n', " "))))
            }

            "whatis" | "def" => {
                if args.is_empty() {
                    return Ok(Some(ctx.locale.t("ai_whatis_usage").to_string()));
                }

                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Geef een vlijmscherpe, feitelijke en nuchtere definitie van exact 1 regel voor het volgende begrip: {}",
                        args
                    ),
                    "de" => format!(
                        "Gib eine präzise, sachliche und nüchterne Definition in genau 1 Zeile für folgenden Begriff: {}",
                        args
                    ),
                    _ => format!(
                        "Provide a sharp, factual, and concise definition in exactly 1 line for the following term: {}",
                        args
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let def = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                Ok(Some(format!("💡 [{}]: {}", ctx.locale.t("ai_def_title"), def.replace('\n', " "))))
            }

            "catchup" | "digest" => {
                if !ctx.ai_manager.can_consume(300) {
                    return Ok(Some(format!("⚠️ {}", ctx.locale.t("ai_budget_exceeded"))));
                }

                let count = if let Ok(n) = args.parse::<i64>() {
                    n.clamp(10, 60)
                } else {
                    35
                };

                let recent_logs = ctx.rag.search_history(&cmd.channel, count).await?;
                if recent_logs.is_empty() {
                    return Ok(Some(format!("ℹ️ {}", ctx.locale.t("ai_no_history"))));
                }

                let logs_text = recent_logs.join("\n");
                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Vat de recente chatgesprekken in kanaal '{}' beknopt samen voor de terugkerende gebruiker '{}' in maximaal 3 korte bullet points in het Nederlands. Noem expliciet of '{}' werd genoemd of gezocht, en of er concrete afspraken zijn gemaakt:\n\n{}",
                        cmd.channel, cmd.author, cmd.author, logs_text
                    ),
                    "de" => format!(
                        "Fasse die aktuellen Chat-Gespräche im Kanal '{}' kurz für den zurückkehrenden Benutzer '{}' in maximal 3 kurzen Stichpunkten auf Deutsch zusammen. Erwähne ausdrücklich, ob '{}' erwähnt oder gesucht wurde und ob konkrete Absprachen getroffen wurden:\n\n{}",
                        cmd.channel, cmd.author, cmd.author, logs_text
                    ),
                    _ => format!(
                        "Summarize recent chat in channel '{}' concisely for returning user '{}' in at most 3 short bullet points in English. Explicitly mention if '{}' was mentioned or looked for, and if any agreements were made:\n\n{}",
                        cmd.channel, cmd.author, cmd.author, logs_text
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let summary = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                ctx.ai_manager.record_consumption(200);

                let title = ctx.locale.t("ai_catchup_title");
                if cmd.platform == "irc" {
                    let lines = Self::format_for_irc(&summary);
                    Ok(Some(format!("📰 [{} {}]: {}", title, cmd.author, lines.join(" | "))))
                } else {
                    Ok(Some(format!("📰 **[{} {}]**:\n{}", title, cmd.author, summary)))
                }
            }

            "vibe" | "sentiment" => {
                if !ctx.ai_manager.can_consume(200) {
                    return Ok(Some(format!("⚠️ {}", ctx.locale.t("ai_budget_exceeded"))));
                }

                let recent_logs = ctx.rag.search_history(&cmd.channel, 30).await?;
                if recent_logs.is_empty() {
                    return Ok(Some(format!("ℹ️ {}", ctx.locale.t("ai_no_history"))));
                }

                let logs_text = recent_logs.join("\n");
                let prompt = match ctx.locale.language() {
                    "nl" => format!(
                        "Peil in maximaal 1 of 2 humoristische, opgewekte zinnen de actuele sfeer en stemming in dit chatkanaal op basis van deze recente berichten. Noem een percentage gezelligheid/vrolijkheid en de 2 heetste onderwerpen in het Nederlands:\n\n{}",
                        logs_text
                    ),
                    "de" => format!(
                        "Ermittle in maximal 1 bis 2 humorvollen Sätzen die aktuelle Stimmung in diesem Chat-Kanal basierend auf diesen Nachrichten. Nenne einen Gemütlichkeits-Prozentsatz und die 2 heißesten Themen auf Deutsch:\n\n{}",
                        logs_text
                    ),
                    _ => format!(
                        "Gauge the current mood and atmosphere in this chat channel based on recent messages in 1 or 2 humorous, upbeat sentences. Mention a cheerfulness percentage and the 2 hottest topics in English:\n\n{}",
                        logs_text
                    ),
                };

                let current_model = ctx.ai_manager.get_model();
                let vibe = ctx.ai_client.ask(&cmd.author, &prompt, Some(&current_model)).await?;
                ctx.ai_manager.record_consumption(120);

                Ok(Some(format!("🌡️ [{}]: {}", ctx.locale.t("ai_vibe_title"), vibe.replace('\n', " "))))
            }

            _ => Ok(None),
        }
    }
}
