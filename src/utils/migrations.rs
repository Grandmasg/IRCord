//! sqlx bewaart per migratie een SHA-384-controlesom en weigert te starten bij `VersionMismatch` als die afwijkt van het
//! bestand in de binary. Een afwijkende controlesom kan puur door regeleinden komen (Windows CRLF tegenover Linux LF) terwijl
//! de migratie inhoudelijk hetzelfde is. Deze module herstelt dat en biedt een expliciete noodroute voor andere gevallen.

use sha2::{Digest, Sha384};
use sqlx::migrate::Migrator;
use sqlx::SqlitePool;
use tracing::{error, info, warn};

fn sha384(bytes: &[u8]) -> Vec<u8> {
    Sha384::digest(bytes).to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

/// Uitkomst van het controleren van de opgeslagen controlesommen.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// Versies waarvan alleen de regeleinden afweken (opgelost).
    pub eol_fixed: Vec<i64>,
    /// Versies die uitdrukkelijk zijn geaccepteerd (`accept_all`).
    pub accepted: Vec<i64>,
    /// Versies met een afwijking die we niet automatisch vertrouwen.
    pub unresolved: Vec<i64>,
}

/// Vergelijkt de in de database opgeslagen controlesommen met die van de ingebouwde migraties.
/// - Verschil alleen in regeleinden (LF/CRLF): stilzwijgend herstellen.
/// - Ander verschil: alleen bijwerken als `accept_all` aan staat; anders melden (en `migrator.run` zal falen).
pub async fn reconcile_checksums(pool: &SqlitePool, migrator: &Migrator, accept_all: bool) -> Reconciled {
    let mut out = Reconciled::default();
    for m in migrator.iter() {
        let stored: Option<Vec<u8>> = match sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
            .bind(m.version)
            .fetch_optional(pool)
            .await
        {
            Ok(v) => v,
            // Tabel bestaat nog niet (verse database): niets te herstellen
            Err(_) => return out,
        };
        let Some(stored) = stored else { continue };
        if stored.as_slice() == m.checksum.as_ref() {
            continue;
        }
        let lf = m.sql.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let eol_only = stored == sha384(lf.as_bytes()) || stored == sha384(crlf.as_bytes());
        if eol_only || accept_all {
            let upd = sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(m.checksum.as_ref())
                .bind(m.version)
                .execute(pool)
                .await;
            if upd.is_ok() {
                if eol_only {
                    info!("Migratie {}: controlesom hersteld (alleen regeleinden verschilden).", m.version);
                    out.eol_fixed.push(m.version);
                } else {
                    warn!("Migratie {}: afwijkende controlesom geaccepteerd (IRCORD_ACCEPT_MIGRATION_CHECKSUMS).", m.version);
                    out.accepted.push(m.version);
                }
                continue;
            }
        }
        error!(
            "Migratie {} ({}): de database heeft een andere controlesom dan deze versie van IRCord. \
             Als je zeker weet dat dit dezelfde migratie is, zet IRCORD_ACCEPT_MIGRATION_CHECKSUMS=true (eenmalig) of voer uit: \
             UPDATE _sqlx_migrations SET checksum = X'{}' WHERE version = {};",
            m.version,
            m.description,
            hex(m.checksum.as_ref()),
            m.version
        );
        out.unresolved.push(m.version);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn migrated_pool() -> (SqlitePool, Migrator) {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        let migrator = sqlx::migrate!("./migrations");
        migrator.run(&pool).await.unwrap();
        (pool, migrator)
    }

    async fn set_checksum(pool: &SqlitePool, version: i64, bytes: Vec<u8>) {
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?").bind(bytes).bind(version).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn crlf_checksum_is_repaired_automatically() {
        let (pool, migrator) = migrated_pool().await;
        let first = migrator.iter().next().unwrap();
        let crlf = first.sql.replace("\r\n", "\n").replace('\n', "\r\n");
        set_checksum(&pool, first.version, sha384(crlf.as_bytes())).await;

        // Zonder herstel weigert sqlx te starten (de fout uit de N5-log)
        let err = migrator.run(&pool).await.unwrap_err().to_string();
        assert!(err.contains("previously applied but has been modified"), "{err}");

        let r = reconcile_checksums(&pool, &migrator, false).await;
        assert_eq!(r.eol_fixed, vec![first.version]);
        assert!(r.unresolved.is_empty());
        migrator.run(&pool).await.expect("na herstel start sqlx wel");
    }

    #[tokio::test]
    async fn unknown_mismatch_needs_explicit_acceptance() {
        let (pool, migrator) = migrated_pool().await;
        let first = migrator.iter().next().unwrap();
        set_checksum(&pool, first.version, vec![1, 2, 3]).await;

        let r = reconcile_checksums(&pool, &migrator, false).await;
        assert_eq!(r.unresolved, vec![first.version]);
        assert!(migrator.run(&pool).await.is_err(), "een onbekende afwijking mag niet stilzwijgend worden goedgekeurd");

        let r = reconcile_checksums(&pool, &migrator, true).await;
        assert_eq!(r.accepted, vec![first.version]);
        migrator.run(&pool).await.expect("na expliciete acceptatie start sqlx");
    }

    #[tokio::test]
    async fn fresh_and_consistent_databases_are_untouched() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        let migrator = sqlx::migrate!("./migrations");
        // verse database: geen _sqlx_migrations-tabel
        assert_eq!(reconcile_checksums(&pool, &migrator, false).await, Reconciled::default());
        migrator.run(&pool).await.unwrap();
        assert_eq!(reconcile_checksums(&pool, &migrator, true).await, Reconciled::default());
    }

    #[test]
    fn hex_format() {
        assert_eq!(hex(&[0x0a, 0xff]), "0AFF");
    }
}
