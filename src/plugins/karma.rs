use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use std::sync::OnceLock;

static KARMA_REGEX: OnceLock<Regex> = OnceLock::new();

/// Karma telt alleen als het HELE bericht één `nick++` of `nick--` is, zoals bij de meeste IRC-bots:
/// "koffie++" telt, "ik wil koffie++" of "goed gedaan PjoT++" niet. De nick moet met een letter of cijfer
/// beginnen, zodat balkjes zoals `[||||||--` of `a--b` ook niet meetellen.
fn parse_karma(content: &str) -> Option<(&str, &str)> {
    let re = KARMA_REGEX.get_or_init(|| Regex::new(r"^([\p{L}\p{N}_][\p{L}\p{N}_\-\[\]\\`^{}|]{0,31})(\+\+|--)$").unwrap());
    let caps = re.captures(content.trim())?;
    Some((caps.get(1)?.as_str(), caps.get(2)?.as_str()))
}

pub struct KarmaPlugin;

#[async_trait]
impl Plugin for KarmaPlugin {
    fn name(&self) -> &'static str { "karma" }
    fn triggers(&self) -> &[&'static str] { &["karma"] }
    fn help(&self) -> &'static str { "!karma [nick] - Toont de reputatie van een gebruiker | nick++ of nick--" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let target = if cmd.args.trim().is_empty() {
            cmd.author.as_str()
        } else {
            cmd.args.trim()
        };

        let row = sqlx::query!(
            r#"
            SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
            FROM audit_log
            WHERE details = ?
            "#,
            target
        )
        .fetch_one(&ctx.db)
        .await?;

        let score = row.total.unwrap_or(0);
        let score_str = score.to_string();
        Ok(Some(ctx.locale.tf("karma_score", &[("target", target), ("score", &score_str)])))
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Bots (ook onszelf) geven en krijgen geen karma; voorkomt reacties op eigen output.
        if msg.author.eq_ignore_ascii_case("IRCord") || msg.author.eq_ignore_ascii_case("Monkeybot") {
            return Ok(None);
        }

        let Some((target, op)) = parse_karma(&msg.content) else {
            return Ok(None);
        };
        // Self-voting not allowed
        if target.eq_ignore_ascii_case(&msg.author) {
            return Ok(Some(ctx.locale.tf("karma_self", &[("author", &msg.author)])));
        }

        let action = if op == "++" { "karma_up" } else { "karma_down" };

        sqlx::query!(
            r#"
            INSERT INTO audit_log (operator, platform, action, details)
            VALUES (?, ?, ?, ?)
            "#,
            msg.author,
            msg.platform,
            action,
            target
        )
        .execute(&ctx.db)
        .await?;

        // Fetch new score
        let row = sqlx::query!(
            r#"
            SELECT SUM(CASE WHEN action = 'karma_up' THEN 1 WHEN action = 'karma_down' THEN -1 ELSE 0 END) as total
            FROM audit_log
            WHERE details = ?
            "#,
            target
        )
        .fetch_one(&ctx.db)
        .await?;

        let score = row.total.unwrap_or(0);
        let score_str = score.to_string();
        Ok(Some(ctx.locale.tf("karma_new_score", &[("target", target), ("score", &score_str)])))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_karma;

    #[test]
    fn parses_only_a_lone_karma_token() {
        assert_eq!(parse_karma("koffie++"), Some(("koffie", "++")));
        assert_eq!(parse_karma("  koffie++  "), Some(("koffie", "++")));
        assert_eq!(parse_karma("henk--"), Some(("henk", "--")));
        assert_eq!(parse_karma("c++"), Some(("c", "++")));
        assert_eq!(parse_karma("Pj[o]T++"), Some(("Pj[o]T", "++")));
    }

    #[test]
    fn sentences_with_a_karma_token_do_not_count() {
        assert_eq!(parse_karma("ik wil koffie++"), None);
        assert_eq!(parse_karma("goed gedaan PjoT++ !"), None);
        assert_eq!(parse_karma("henk-- haha"), None);
        assert_eq!(parse_karma("koffie++ thee++"), None);
        assert_eq!(parse_karma("koffie++."), None);
    }

    #[test]
    fn ignores_noise() {
        assert_eq!(parse_karma("[||||||-- heeft nu een score van -1"), None);
        assert_eq!(parse_karma("||||||--"), None);
        assert_eq!(parse_karma("a--b"), None);
        assert_eq!(parse_karma("ls --help"), None);
        assert_eq!(parse_karma("zin -- met streepjes"), None);
    }
}
