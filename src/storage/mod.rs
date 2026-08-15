//! Persistent application storage.
//!
//! All per-user state - client configuration, authentication against the
//! third-eye server, ROV media bookkeeping, capture metadata, and the outbox
//! for delayed server writes - lives in a single `SQLite` database at
//! `~/Library/Application Support/eu.marshalling.third-eye-client/state.db`
//! on macOS (equivalent paths elsewhere via [`directories`]).
//!
//! The top-level facade is [`AppStore`]. It is constructed once at startup
//! in `main.rs`, cloned as an `Rc<AppStore>` into Slint callbacks, and owns
//! the background outbox worker thread.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use directories::ProjectDirs;

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod devices;
pub mod media;
pub mod outbox;
pub mod search;
pub mod tile_cache;

use crate::storage::api::ApiSession;
use crate::storage::auth::AuthClient;
use crate::storage::config::ConfigStore;
use crate::storage::db::SharedDb;
use crate::storage::devices::{DeviceCacheStore, DevicesClient};
use crate::storage::media::MediaStore;
use crate::storage::outbox::{OutboxStore, OutboxWorker};
use crate::storage::search::SearchClient;
use crate::storage::tile_cache::TileCacheStore;

/// Qualifier / organisation / application triple used with `directories`.
pub const PROJECT_QUALIFIER: &str = "eu";
pub const PROJECT_ORG: &str = "marshalling";
pub const PROJECT_APP: &str = "third-eye-client";
/// Name of the `SQLite` file inside the data dir.
pub const DB_FILE_NAME: &str = "state.db";

/// Facade over the persistent storage layer. Cheap to clone the inner stores
/// (`config()`, `media()`, `outbox()`) because they share a single
/// `Arc<Mutex<Connection>>`.
pub struct AppStore {
    db: SharedDb,
    config: ConfigStore,
    auth: Arc<AuthClient>,
    /// Shared authenticated-call facade. Every server call goes through this so
    /// a signed-in user's session refreshes itself indefinitely (see
    /// `storage::api`).
    api: ApiSession,
    devices: DevicesClient,
    device_cache: DeviceCacheStore,
    media: MediaStore,
    tile_cache: TileCacheStore,
    outbox: OutboxStore,
    search: SearchClient,
    worker: Mutex<Option<OutboxWorker>>,
    data_path: Option<PathBuf>,
}

impl AppStore {
    /// Opens the default database (creating it if needed) and starts the
    /// outbox worker.
    pub fn open() -> Result<Self> {
        let data_path = resolve_db_path()?;
        let db = db::open_persistent(&data_path)?;
        Self::from_db(db, Some(data_path), /*start_worker=*/ true)
    }

    /// Opens a database at a specific path. Used by tests and any caller that
    /// wants to point at a non-default location. The outbox worker is NOT
    /// started by this variant so tests can assert against the queue state
    /// deterministically.
    pub fn open_at(path: &std::path::Path) -> Result<Self> {
        let db = db::open_persistent(path)?;
        Self::from_db(db, Some(path.to_path_buf()), /*start_worker=*/ false)
    }

    /// Test / fallback entry point.
    pub fn open_in_memory() -> Result<Self> {
        let db = db::open_in_memory()?;
        Self::from_db(db, None, /*start_worker=*/ false)
    }

    fn from_db(db: SharedDb, data_path: Option<PathBuf>, start_worker: bool) -> Result<Self> {
        let config = ConfigStore::new(db.clone());
        let auth = Arc::new(AuthClient::new(db.clone())?);
        let api = ApiSession::new(Arc::clone(&auth));
        let devices = DevicesClient::new(api.clone());
        let device_cache = DeviceCacheStore::new(db.clone());
        let media = MediaStore::new(db.clone());
        let tile_cache = TileCacheStore::new(db.clone());
        let outbox = OutboxStore::new(db.clone());
        let search = SearchClient::new(api.clone());
        let worker = if start_worker {
            Some(OutboxWorker::spawn(outbox.clone()))
        } else {
            None
        };
        Ok(Self {
            db,
            config,
            auth,
            api,
            devices,
            device_cache,
            media,
            tile_cache,
            outbox,
            search,
            worker: Mutex::new(worker),
            data_path,
        })
    }

    pub fn config(&self) -> &ConfigStore {
        &self.config
    }

    pub fn auth(&self) -> &AuthClient {
        &self.auth
    }

    /// Shared authenticated-call facade. Use this (rather than raw access
    /// tokens) for anything that talks to the third-eye server.
    pub fn api(&self) -> &ApiSession {
        &self.api
    }

    pub fn devices(&self) -> &DevicesClient {
        &self.devices
    }

    pub fn device_cache(&self) -> &DeviceCacheStore {
        &self.device_cache
    }

    pub fn media(&self) -> &MediaStore {
        &self.media
    }

    pub fn tile_cache(&self) -> &TileCacheStore {
        &self.tile_cache
    }

    pub fn outbox(&self) -> &OutboxStore {
        &self.outbox
    }

    pub fn search(&self) -> &SearchClient {
        &self.search
    }

    /// Absolute path of the DB file on disk, if this is a persistent store.
    pub fn data_path(&self) -> Option<&PathBuf> {
        self.data_path.as_ref()
    }

    /// Access to the shared connection for rare direct-SQL cases.
    pub fn raw_db(&self) -> SharedDb {
        self.db.clone()
    }

    /// Stops the background outbox worker. Called by `main.rs` before exit.
    pub fn shutdown(&self) {
        if let Some(mut worker) = self
            .worker
            .lock()
            .expect("outbox worker mutex poisoned")
            .take()
        {
            worker.stop();
        }
    }
}

impl Drop for AppStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn resolve_db_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_ORG, PROJECT_APP)
        .context("resolving application data directory")?;
    let mut path = dirs.data_dir().to_path_buf();
    path.push(DB_FILE_NAME);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_memory_with_all_stores() {
        let store = AppStore::open_in_memory().unwrap();
        // Basic smoke: every facade can execute at least one query.
        store.config().set("x", "1").unwrap();
        assert_eq!(store.config().get("x").unwrap().as_deref(), Some("1"));
        assert_eq!(store.outbox().pending_count().unwrap(), 0);
        assert_eq!(store.media().list_recent(10).unwrap(), []);
        assert!(store.auth().current_session().unwrap().is_none());
    }

    #[test]
    fn in_memory_store_has_no_data_path() {
        let store = AppStore::open_in_memory().unwrap();
        assert!(store.data_path().is_none());
    }

    #[test]
    fn shutdown_is_idempotent() {
        let store = AppStore::open_in_memory().unwrap();
        // No outbox worker was started (open_in_memory), but shutdown must
        // still be safely callable multiple times, including via `Drop`.
        store.shutdown();
        store.shutdown();
    }
}
