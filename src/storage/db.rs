//! `SQLite` connection management for [`AppStore`](super::AppStore).
//!
//! A single connection is held under an `Arc<Mutex<_>>`; WAL mode is enabled
//! so readers (the outbox worker, the media reconciliation job) do not block
//! the UI thread even though every write serialises through the mutex.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

/// SQL for migration v1. The file is embedded at compile time so the binary
/// is self-contained.
const MIGRATION_V1: &str = include_str!("migrations/001_initial.sql");

/// Shared, thread-safe `SQLite` handle. Every storage submodule takes one of
/// these by `Arc::clone`.
pub type SharedDb = Arc<Mutex<Connection>>;

/// Opens (or creates) a persistent `SQLite` database at `path` and runs all
/// pending schema migrations.
pub fn open_persistent(path: &Path) -> Result<SharedDb> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating storage dir {}", parent.display()))?;
    }
    let mut connection = Connection::open(path)
        .with_context(|| format!("opening SQLite database at {}", path.display()))?;
    apply_pragmas(&connection, true)?;
    apply_migrations(&mut connection)?;
    Ok(Arc::new(Mutex::new(connection)))
}

/// Opens an in-memory database. Used from tests and as a fallback for
/// environments where the data directory cannot be resolved.
pub fn open_in_memory() -> Result<SharedDb> {
    let mut connection = Connection::open_in_memory().context("opening in-memory SQLite")?;
    apply_pragmas(&connection, false)?;
    apply_migrations(&mut connection)?;
    Ok(Arc::new(Mutex::new(connection)))
}

fn apply_pragmas(connection: &Connection, persistent: bool) -> Result<()> {
    // `journal_mode` is a no-op (and would fail) for :memory: databases.
    if persistent {
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .context("enabling WAL journal mode")?;
        if !mode.eq_ignore_ascii_case("wal") {
            // Falling back to the default is fine; we just lose the
            // concurrent-reader property.
            log_fallback_journal_mode(&mode);
        }
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .context("setting PRAGMA synchronous")?;
    }
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .context("enabling PRAGMA foreign_keys")?;
    connection
        .pragma_update(None, "busy_timeout", 5000)
        .context("setting PRAGMA busy_timeout")?;
    Ok(())
}

fn apply_migrations(connection: &mut Connection) -> Result<()> {
    let migrations = Migrations::new(vec![M::up(MIGRATION_V1)]);
    migrations
        .to_latest(connection)
        .context("running schema migrations")?;
    Ok(())
}

#[cfg(not(test))]
fn log_fallback_journal_mode(mode: &str) {
    eprintln!("third-eye-client: warning: SQLite journal_mode fell back to {mode}");
}

#[cfg(test)]
fn log_fallback_journal_mode(_mode: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_applies_schema() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for expected in [
            "auth_session",
            "capture_metadata",
            "http_cookies",
            "media_sync",
            "rest_outbox",
            "settings",
            "user_status_events",
        ] {
            assert!(
                tables.iter().any(|name| name == expected),
                "missing table {expected} (got {tables:?})",
            );
        }
    }

    #[test]
    fn foreign_keys_are_enabled() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }
}
