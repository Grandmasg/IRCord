//! Database-back-up en chatlog-export: `!backup`, `!export [aantal]` (alleen eigenaar) en de dagelijkse automatische back-up.

use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};

const EXPORT_DIR: &str = "data/exports";
const EXPORT_DEFAULT: i64 = 1000;
const EXPORT_MAX: i64 = 20_000;

pub struct BackupPlugin;

/// Maakt een consistente kopie van de database (veilig tijdens gebruik) en ruimt oude back-ups op.
pub async fn backup_database(db: &SqlitePool, dir: &Path, keep: usize) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(dir)?;
    let name = format!("ircord-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let target = dir.join(&name);
    sqlx::query("VACUUM INTO ?").bind(target.to_string_lossy().to_string()).execute(db).await?;
    prune_backups(dir, keep);
    Ok(target)
}

/// Houdt de `keep` nieuwste back-ups (op bestandsnaam, die de tijd bevat) en verwijdert de rest.
pub fn prune_backups(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ircord-") && n.ends_with(".db"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep.max(1));
    for old in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

fn safe_filename(channel: &str) -> String {
    let s: String = channel.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    if s.is_empty() { "kanaal".to_string() } else { s }
}

#[async_trait]
impl Plugin for BackupPlugin {
    fn name(&self) -> &'static str { "backup" }
    fn triggers(&self) -> &[&'static str] { &["backup", "export"] }
    fn help(&self) -> &'static str {
        "!backup - maakt nu een databaseback-up (eigenaar) | !export [aantal] - schrijft het chatlog van dit kanaal naar een bestand (eigenaar)"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if !cmd.is_owner {
            return Ok(Some("⛔ Alleen de bot-eigenaar kan back-ups en exports maken.".into()));
        }
        if cmd.trigger == "backup" {
            let dir = PathBuf::from(&ctx.config.general.backup_dir);
            return Ok(Some(match backup_database(&ctx.db, &dir, ctx.config.general.backup_keep).await {
                Ok(path) => {
                    let kb = std::fs::metadata(&path).map(|m| m.len() / 1024).unwrap_or(0);
                    format!("💾 Back-up gemaakt: {} ({} KB)", path.display(), kb)
                }
                Err(e) => format!("⚠️ Back-up mislukt: {}", e),
            }));
        }

        // !export
        let count: i64 = cmd.args.trim().parse().unwrap_or(EXPORT_DEFAULT).clamp(1, EXPORT_MAX);
        let rows = sqlx::query("SELECT timestamp, author, message FROM chat_history WHERE channel = ? ORDER BY rowid DESC LIMIT ?")
            .bind(&cmd.channel)
            .bind(count)
            .fetch_all(&ctx.db)
            .await?;
        if rows.is_empty() {
            return Ok(Some("📄 Geen chatgeschiedenis om te exporteren voor dit kanaal.".into()));
        }
        let mut out = String::new();
        for r in rows.iter().rev() {
            let ts: String = r.try_get("timestamp").unwrap_or_default();
            let author: String = r.try_get("author").unwrap_or_default();
            let msg: String = r.try_get("message").unwrap_or_default();
            out.push_str(&format!("[{}] <{}> {}\n", ts, author, msg.replace('\n', " ")));
        }
        std::fs::create_dir_all(EXPORT_DIR)?;
        let path = Path::new(EXPORT_DIR).join(format!("{}-{}.txt", safe_filename(&cmd.channel), chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::write(&path, out)?;
        Ok(Some(format!("📄 {} berichten geëxporteerd naar {}", rows.len(), path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ircord-test-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn prune_keeps_newest_only() {
        let d = tempdir("prune");
        for n in ["ircord-20260101-000000.db", "ircord-20260102-000000.db", "ircord-20260103-000000.db", "andere.txt"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        prune_backups(&d, 2);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, vec!["andere.txt", "ircord-20260102-000000.db", "ircord-20260103-000000.db"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn backup_creates_a_readable_copy() {
        // VACUUM INTO werkt op een bestandsdatabase; de in-memory pool dekt de query af
        let src_dir = tempdir("src");
        let src = src_dir.join("bron.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&src).create_if_missing(true))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (x INTEGER)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO t VALUES (42)").execute(&pool).await.unwrap();

        let dir = tempdir("dst");
        let path = backup_database(&pool, &dir, 3).await.unwrap();
        assert!(path.exists());

        let copy = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
        let x: i64 = sqlx::query("SELECT x FROM t").fetch_one(&copy).await.unwrap().get("x");
        assert_eq!(x, 42);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn filenames_are_safe() {
        assert_eq!(safe_filename("#gzrbot"), "gzrbot");
        assert_eq!(safe_filename("../../etc"), "etc");
        assert_eq!(safe_filename("###"), "kanaal");
    }
}
