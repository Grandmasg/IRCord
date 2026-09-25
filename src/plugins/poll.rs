use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct PollPlugin;

#[async_trait]
impl Plugin for PollPlugin {
    fn name(&self) -> &'static str { "poll" }
    fn triggers(&self) -> &[&'static str] { &["poll"] }
    fn help(&self) -> &'static str { "!poll start \"Vraag?\" optie1/optie2 | !poll vote <nr> | !poll end" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let mut parts = args.splitn(2, ' ');
        let sub = parts.next().unwrap_or("").to_lowercase();
        let rest = parts.next().unwrap_or("").trim();

        match sub.as_str() {
            "start" => {
                if rest.is_empty() {
                    return Ok(Some(ctx.locale.t("poll_usage").into()));
                }

                // Simple parser: question in quotes, options separated by slash
                let (question, options_str) = if rest.starts_with('"') {
                    if let Some(end_quote) = rest[1..].find('"') {
                        let q = &rest[1..=end_quote];
                        let opt = rest[end_quote + 2..].trim();
                        (q, opt)
                    } else {
                        return Ok(Some(ctx.locale.t("poll_need_quotes").into()));
                    }
                } else {
                    let mut s = rest.splitn(2, '?');
                    let q = s.next().unwrap_or(rest);
                    let opt = s.next().unwrap_or("");
                    (q, opt)
                };

                let options: Vec<&str> = options_str.split('/').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if options.len() < 2 {
                    return Ok(Some(ctx.locale.t("poll_min_options").into()));
                }

                let options_json = serde_json::to_string(&options)?;

                // Deactivate previous active polls in this channel
                sqlx::query!("UPDATE polls SET is_active = FALSE WHERE channel = ?", cmd.channel)
                    .execute(&ctx.db)
                    .await?;

                let res = sqlx::query!(
                    r#"
                    INSERT INTO polls (channel, question, options_json, is_active, created_by)
                    VALUES (?, ?, ?, TRUE, ?)
                    "#,
                    cmd.channel,
                    question,
                    options_json,
                    cmd.author
                )
                .execute(&ctx.db)
                .await?;

                let poll_id = res.last_insert_rowid();

                let mut opt_list = Vec::new();
                for (idx, opt) in options.iter().enumerate() {
                    opt_list.push(format!("[{}] {}", idx + 1, opt));
                }

                let poll_title = if ctx.locale.is_dutch() { "Peiling" } else { "Poll" };
                let vote_hint = if ctx.locale.is_dutch() { "Stem met: !poll vote <nummer>" } else { "Vote with: !poll vote <number>" };

                Ok(Some(format!(
                    "📊 [{} #{}: \"{}\"] {} | {}",
                    poll_title, poll_id, question, opt_list.join(" | "), vote_hint
                )))
            }
            "vote" => {
                let choice_str = rest.trim();
                let choice_num: usize = match choice_str.parse() {
                    Ok(n) if n > 0 => n,
                    _ => return Ok(Some(ctx.locale.t("poll_vote_usage").into())),
                };

                let active_poll = sqlx::query!(
                    "SELECT id, options_json FROM polls WHERE channel = ? AND is_active = TRUE LIMIT 1",
                    cmd.channel
                )
                .fetch_optional(&ctx.db)
                .await?;

                if let Some(poll) = active_poll {
                    let options: Vec<String> = serde_json::from_str(&poll.options_json)?;
                    if choice_num > options.len() {
                        return Ok(Some(ctx.locale.tf("poll_vote_invalid", &[("max", &options.len().to_string())])));
                    }

                    let option_idx = (choice_num - 1) as i64;
                    sqlx::query!(
                        r#"
                        INSERT INTO poll_votes (poll_id, voter, platform, option_index)
                        VALUES (?, ?, ?, ?)
                        ON CONFLICT(poll_id, voter, platform) DO UPDATE SET option_index = excluded.option_index
                        "#,
                        poll.id,
                        cmd.author,
                        cmd.platform,
                        option_idx
                    )
                    .execute(&ctx.db)
                    .await?;

                    let voted_msg = ctx.locale.tf(
                        "poll_voted",
                        &[
                            ("choice", &choice_num.to_string()),
                            ("option", &options[choice_num - 1]),
                        ],
                    );
                    Ok(Some(format!("🗳️ {}, {}", cmd.author, voted_msg)))
                } else {
                    Ok(Some(ctx.locale.t("poll_none_active").into()))
                }
            }
            "end" => {
                let active_poll = sqlx::query!(
                    "SELECT id, question, options_json FROM polls WHERE channel = ? AND is_active = TRUE LIMIT 1",
                    cmd.channel
                )
                .fetch_optional(&ctx.db)
                .await?;

                if let Some(poll) = active_poll {
                    sqlx::query!("UPDATE polls SET is_active = FALSE WHERE id = ?", poll.id)
                        .execute(&ctx.db)
                        .await?;

                    let options: Vec<String> = serde_json::from_str(&poll.options_json)?;
                    let votes: Vec<(i64, i64)> = sqlx::query_as(
                        "SELECT option_index, COUNT(*) FROM poll_votes WHERE poll_id = ? GROUP BY option_index",
                    )
                    .bind(poll.id)
                    .fetch_all(&ctx.db)
                    .await?;

                    let mut counts = vec![0; options.len()];
                    for (opt_idx, count) in votes {
                        let idx = opt_idx as usize;
                        if idx < counts.len() {
                            counts[idx] = count;
                        }
                    }

                    let votes_word = if ctx.locale.is_dutch() { "stemmen" } else { "votes" };
                    let mut summary = Vec::new();
                    for (idx, opt) in options.iter().enumerate() {
                        summary.push(format!("{}: {} {}", opt, counts[idx], votes_word));
                    }

                    let result_title = if ctx.locale.is_dutch() { "Uitslag Peiling" } else { "Poll Results" };
                    Ok(Some(format!(
                        "🏁 [{}: \"{}\"] {}",
                        result_title, poll.question, summary.join(" | ")
                    )))
                } else {
                    Ok(Some(ctx.locale.t("poll_none_active").into()))
                }
            }
            _ => Ok(Some(ctx.locale.t("poll_usage").into())),
        }
    }
}
