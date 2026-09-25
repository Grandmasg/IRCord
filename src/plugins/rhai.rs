use async_trait::async_trait;
use rhai::{Engine, Scope, Dynamic, Array};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tracing::{info, warn};

use super::{CommandEvent, Plugin, PluginContext};

pub struct RhaiPlugin {
    scripts_dir: PathBuf,
}

impl RhaiPlugin {
    pub fn new() -> Self {
        Self {
            scripts_dir: PathBuf::from("scripts"),
        }
    }

    pub fn with_dir<P: Into<PathBuf>>(dir: P) -> Self {
        Self {
            scripts_dir: dir.into(),
        }
    }

    fn create_sandboxed_engine() -> Engine {
        let mut engine = Engine::new();
        // Strikte resource limieten volgens beveiligingsspecificatie
        engine.set_max_operations(50_000);
        engine.set_max_string_size(1_000);
        engine.set_max_array_size(100);
        engine.set_max_map_size(100);
        engine.set_max_call_levels(16);

        // Veilige ingebouwde hulpfuncties
        engine.register_fn("rand", || -> i64 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            nanos as i64
        });

        engine
    }

    fn sanitize_script_name(name: &str) -> Option<String> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.contains("..") || trimmed.contains('/') || trimmed.contains('\\') {
            return None;
        }

        let clean = trimmed.strip_suffix(".rhai").unwrap_or(trimmed);
        if clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            Some(format!("{}.rhai", clean))
        } else {
            None
        }
    }
}

#[async_trait]
impl Plugin for RhaiPlugin {
    fn name(&self) -> &'static str {
        "rhai"
    }

    fn triggers(&self) -> &[&'static str] {
        &["script", "rhai", "scriptje"]
    }

    fn help(&self) -> &'static str {
        "Voert een dynamisch Rhai script uit: !script <naam> [args...]"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args_trimmed = cmd.args.trim();
        if args_trimmed.is_empty() {
            let lang = ctx.locale.language();
            let msg = if lang == "nl" {
                "Gebruik: !script <scriptnaam> [argumenten...]. Voorbeeld: !script hello of !script hallo"
            } else {
                "Usage: !script <script_name> [args...]. Example: !script hello or !script hallo"
            };
            return Ok(Some(msg.to_string()));
        }

        let mut parts = args_trimmed.split_whitespace();
        let script_arg = parts.next().unwrap_or("");
        let script_file = match Self::sanitize_script_name(script_arg) {
            Some(name) => name,
            None => {
                let err_msg = if ctx.locale.is_dutch() {
                    "Ongeldige scriptnaam. Gebruik alleen alfanumerieke tekens, underscores of koppeltekens."
                } else {
                    "Invalid script name. Use only alphanumeric characters, underscores, or hyphens."
                };
                return Ok(Some(err_msg.to_string()));
            }
        };

        let path = self.scripts_dir.join(&script_file);
        if !path.exists() {
            let err_msg = if ctx.locale.is_dutch() {
                format!("Script '{}' niet gevonden in scripts/.", script_file)
            } else {
                format!("Script '{}' not found in scripts/.", script_file)
            };
            return Ok(Some(err_msg));
        }

        let script_content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                warn!("Fout bij lezen van script {}: {}", path.display(), e);
                return Ok(Some(format!("Fout bij openen van script: {}", e)));
            }
        };

        // Bouw parameters array
        let raw_args: Vec<Dynamic> = parts.map(|s| Dynamic::from(s.to_string())).collect();
        let params_array = Array::from(raw_args);

        let lang = ctx.locale.language().to_string();
        let author = cmd.author.clone();
        let channel = cmd.channel.clone();
        let platform = cmd.platform.clone();
        let is_op = cmd.is_operator;

        // Voer uit in een blocking threadpool met timeout ter preventie van infinite loops
        let result = tokio::task::spawn_blocking(move || {
            let engine = Self::create_sandboxed_engine();
            let mut scope = Scope::new();

            scope.push("params", params_array);
            scope.push("author", author);
            scope.push("channel", channel);
            scope.push("platform", platform);
            scope.push("lang", lang);
            scope.push("is_operator", is_op);

            engine.eval_with_scope::<Dynamic>(&mut scope, &script_content)
        });

        match timeout(Duration::from_millis(250), result).await {
            Ok(Ok(Ok(val))) => {
                let out = val.to_string();
                if out.is_empty() || out == "()" {
                    Ok(None)
                } else {
                    Ok(Some(out))
                }
            }
            Ok(Ok(Err(eval_err))) => {
                warn!("Rhai script runtime fout in {}: {}", script_file, eval_err);
                Ok(Some(format!("Script error: {}", eval_err)))
            }
            Ok(Err(join_err)) => {
                warn!("Rhai script task panic in {}: {}", script_file, join_err);
                Ok(Some("Script execution panicked.".to_string()))
            }
            Err(_) => {
                warn!("Rhai script timeout (>250ms) in {}", script_file);
                let timeout_msg = if ctx.locale.is_dutch() {
                    "Script time-out: uitvoering duurde te lang (>250ms)."
                } else {
                    "Script execution timed out (>250ms)."
                };
                Ok(Some(timeout_msg.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sandboxed_engine_basic() {
        let engine = RhaiPlugin::create_sandboxed_engine();
        let mut scope = Scope::new();
        scope.push("lang", "nl".to_string());
        let res: String = engine.eval_with_scope(&mut scope, r#"if lang == "nl" { "Hallo wereld" } else { "Hello world" }"#).unwrap();
        assert_eq!(res, "Hallo wereld");
    }

    #[tokio::test]
    async fn test_script_name_sanitizer() {
        assert_eq!(RhaiPlugin::sanitize_script_name("hello"), Some("hello.rhai".to_string()));
        assert_eq!(RhaiPlugin::sanitize_script_name("hallo.rhai"), Some("hallo.rhai".to_string()));
        assert_eq!(RhaiPlugin::sanitize_script_name("../secret"), None);
        assert_eq!(RhaiPlugin::sanitize_script_name("etc/passwd"), None);
        assert_eq!(RhaiPlugin::sanitize_script_name("test script"), None);
    }

    #[tokio::test]
    async fn test_multilingual_hello_script() {
        let script = std::fs::read_to_string("scripts/hello.rhai").expect("scripts/hello.rhai must exist");
        let engine = RhaiPlugin::create_sandboxed_engine();

        // 1. Test Dutch output
        let mut scope_nl = Scope::new();
        let params_nl: Array = vec![Dynamic::from("Klaas".to_string())];
        scope_nl.push("params", params_nl);
        scope_nl.push("author", "Klaas".to_string());
        scope_nl.push("lang", "nl".to_string());
        let res_nl: String = engine.eval_with_scope(&mut scope_nl, &script).unwrap();
        assert!(res_nl.contains("Klaas"), "Expected user name in Dutch output");
        assert!(
            res_nl.contains("Welkom") || res_nl.contains("Hoi") || res_nl.contains("Gegroet"),
            "Expected Dutch greeting, got: {}", res_nl
        );

        // 2. Test English output
        let mut scope_en = Scope::new();
        let params_en: Array = vec![Dynamic::from("John".to_string())];
        scope_en.push("params", params_en);
        scope_en.push("author", "John".to_string());
        scope_en.push("lang", "en".to_string());
        let res_en: String = engine.eval_with_scope(&mut scope_en, &script).unwrap();
        assert!(res_en.contains("John"), "Expected user name in English output");
        assert!(
            res_en.contains("Welcome") || res_en.contains("Hey") || res_en.contains("Greetings"),
            "Expected English greeting, got: {}", res_en
        );
    }

    #[tokio::test]
    async fn test_hallo_script_dutch() {
        let script = std::fs::read_to_string("scripts/hallo.rhai").expect("scripts/hallo.rhai must exist");
        let engine = RhaiPlugin::create_sandboxed_engine();

        let mut scope = Scope::new();
        let params: Array = vec![Dynamic::from("Piet".to_string())];
        scope.push("params", params);
        scope.push("author", "Piet".to_string());
        scope.push("lang", "nl".to_string());
        let res: String = engine.eval_with_scope(&mut scope, &script).unwrap();
        assert!(res.contains("Piet"));
        assert!(res.contains("Welkom") || res.contains("Hoi") || res.contains("Gegroet"));
    }
}
