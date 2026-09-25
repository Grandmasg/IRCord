use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;

pub struct LangPlugin;

#[async_trait]
impl Plugin for LangPlugin {
    fn name(&self) -> &'static str {
        "lang"
    }

    fn triggers(&self) -> &[&'static str] {
        &["lang", "taal", "language", "sprache", "setlang"]
    }

    fn help(&self) -> &'static str {
        "!lang | !lang <code|name> (e.g. !lang nl, !lang en, !lang de) | !lang reset"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let title = ctx.locale.t("lang_title");
        let available = ctx.locale.available_languages().join(", ");

        if args.is_empty() {
            let current = ctx.locale.user_lang(&cmd.platform, &cmd.author);
            let default_lang = ctx.locale.default_language();
            let msg = ctx.locale.tf(
                "lang_current",
                &[
                    ("title", title),
                    ("lang", &current),
                    ("default", default_lang),
                    ("available", &available),
                ],
            );
            return Ok(Some(msg));
        }

        if args.eq_ignore_ascii_case("reset") || args.eq_ignore_ascii_case("default") {
            ctx.locale
                .remove_user_language(&ctx.db, &cmd.platform, &cmd.author)
                .await?;
            let default_lang = ctx.locale.default_language();
            let msg = ctx.locale.tf(
                "lang_reset",
                &[
                    ("title", title),
                    ("default", default_lang),
                ],
            );
            return Ok(Some(msg));
        }

        if let Some(target_code) = ctx.locale.normalize_language_code(args) {
            ctx.locale
                .persist_user_language(&ctx.db, &cmd.platform, &cmd.author, target_code)
                .await?;

            // Render confirmation in newly chosen language
            let new_locale = ctx.locale.for_language(target_code);
            let new_title = new_locale.t("lang_title");
            let msg = new_locale.tf(
                "lang_set_success",
                &[
                    ("title", new_title),
                    ("lang", target_code),
                ],
            );
            Ok(Some(msg))
        } else {
            let msg = ctx.locale.tf(
                "lang_unknown",
                &[
                    ("title", title),
                    ("input", args),
                    ("available", &available),
                ],
            );
            Ok(Some(msg))
        }
    }
}
