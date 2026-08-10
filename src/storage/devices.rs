//! Devices REST client against the third-eye server.
//!
//! Endpoints (from `/api/v1/api-doc/openapi.json`):
//!   * `GET    /api/v1/devices`      -> paged list of the signed-in user's devices
//!   * `POST   /api/v1/devices`      -> create a device (requires the user id)
//!   * `GET    /api/v1/devices/{id}` -> single device
//!   * `PATCH  /api/v1/devices/{id}` -> edit a device (optimistic-locked via `concurrency`)
//!   * `DELETE /api/v1/devices/{id}` -> delete a device
//!
//! This is the first resource domain (besides accounts) wired up from
//! `generated/` (the `third-eye-openapi` crate), proving the List+Detail
//! pattern used by `ui/pages/devices/device_list.slint`.
//!
//! Every call here goes through [`super::api::ApiSession`], so authentication
//! is entirely implicit: callers never see an access token, and a signed-in
//! user's session refreshes itself for as long as the refresh cookie holds.
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use third_eye_openapi::apis::device_handler_api;
use third_eye_openapi::apis::profile_handler_api;
use third_eye_openapi::models::{
    CreateDeviceSchema, DeviceCategory, DeviceModel, DeviceType,
    PagedResponseDeviceModelItemsInner, UpdateDeviceSchema,
};

use super::api::{ApiError, ApiSession};
use super::db::SharedDb;

/// Flattened view of a `DeviceModel`/`PagedResponseDeviceModelItemsInner` for
/// UI consumption: plain `String`s instead of `uuid::Uuid`/enum types, plus
/// the `concurrency` token callers must pass back in to `rename`.
#[derive(Clone, Debug)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub category: String,
    pub device_type: String,
    pub created_at: String,
    pub updated_at: String,
    pub concurrency: uuid::Uuid,
    /// Raw `configuration` JSON blob from the server, if any. Best-effort
    /// source for RTSP/ROV HTTP settings when this device is marked active
    /// (see `apply_device_configuration_to_client_config` in `main.rs`).
    pub configuration: Option<String>,
}

impl From<DeviceModel> for DeviceSummary {
    fn from(model: DeviceModel) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.name,
            category: model.device_category.to_string(),
            device_type: model.device_type.to_string(),
            created_at: model.created_at,
            updated_at: model.updated_at,
            concurrency: model.concurrency,
            configuration: model.configuration.map(|value| value.to_string()),
        }
    }
}

impl From<PagedResponseDeviceModelItemsInner> for DeviceSummary {
    fn from(model: PagedResponseDeviceModelItemsInner) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.name,
            category: model.device_category.to_string(),
            device_type: model.device_type.to_string(),
            created_at: model.created_at,
            updated_at: model.updated_at,
            concurrency: model.concurrency,
            configuration: model.configuration.map(|value| value.to_string()),
        }
    }
}

/// Facade held by `AppStore`. Stateless beyond the underlying HTTP client and
/// the shared [`ApiSession`]: the server base URL is supplied per call, since
/// it can change at runtime (edited in Settings), and authentication is
/// handled entirely by `ApiSession`.
pub struct DevicesClient {
    http: reqwest::Client,
    api: ApiSession,
}

impl DevicesClient {
    pub(crate) fn new(api: ApiSession) -> Self {
        Self {
            http: reqwest::Client::new(),
            api,
        }
    }

    /// `GET /api/v1/devices` (first page, default server ordering).
    pub fn list(&self, server_base: &str) -> Result<Vec<DeviceSummary>, ApiError> {
        let page = self
            .api
            .call(server_base, &self.http, |configuration| async move {
                device_handler_api::device_list_handler(
                    &configuration,
                    None,
                    Some(200),
                    Some("name"),
                    Some(false),
                    None,
                )
                .await
            })?;
        Ok(page.items.into_iter().map(DeviceSummary::from).collect())
    }

    /// `GET /api/v1/profile/info`, i.e. the signed-in user. Also doubles as the
    /// cheapest way to verify a restored session actually still works.
    pub fn me_id(&self, server_base: &str) -> Result<uuid::Uuid, ApiError> {
        let me = self
            .api
            .call(server_base, &self.http, |configuration| async move {
                profile_handler_api::get_me_handler(&configuration).await
            })?;
        Ok(me.id)
    }

    /// `POST /api/v1/devices`. Resolves the current user id via
    /// `GET /api/v1/profile/info` first, since device creation requires it
    /// and the local session doesn't otherwise track it. `device_configuration`
    /// should be a `models::ChasingM2SConfiguration` serialized with
    /// `serde_json::to_value` (see `main.rs::build_device_configuration`),
    /// defaulted from whatever is currently set on the Configuration page.
    pub fn create(
        &self,
        server_base: &str,
        name: String,
        device_configuration: Option<serde_json::Value>,
    ) -> Result<DeviceSummary, ApiError> {
        // Two separate calls rather than one: the generated client gives each
        // endpoint its own error type, so they can't share a single future.
        let user_id = self.me_id(server_base)?;
        self.api
            .call(server_base, &self.http, |configuration| {
                // Cloned per attempt: `ApiSession::call` may retry the closure
                // after refreshing the access token.
                let schema = CreateDeviceSchema::new(
                    device_configuration.clone(),
                    DeviceCategory::Underwater,
                    DeviceType::ChasingM2S,
                    name.clone(),
                    user_id,
                );
                async move {
                    device_handler_api::create_device_handler(&configuration, schema).await
                }
            })
            .map(DeviceSummary::from)
    }

    /// `PATCH /api/v1/devices/{id}`. `concurrency` must be the value from the
    /// last-fetched `DeviceSummary` (optimistic locking).
    pub fn rename(
        &self,
        server_base: &str,
        id: &str,
        concurrency: uuid::Uuid,
        name: String,
    ) -> Result<DeviceSummary, ApiError> {
        self.edit(server_base, id, concurrency, name, None)
    }

    /// `PATCH /api/v1/devices/{id}`. Updates both `name` and `configuration`
    /// in one request (optimistic-locked via `concurrency`). Used by the
    /// Device Detail page's combined Save action.
    pub fn update(
        &self,
        server_base: &str,
        id: &str,
        concurrency: uuid::Uuid,
        name: String,
        configuration: Option<serde_json::Value>,
    ) -> Result<DeviceSummary, ApiError> {
        self.edit(server_base, id, concurrency, name, configuration)
    }

    /// Shared `PATCH /api/v1/devices/{id}` body. `configuration` is
    /// double-`Option` on purpose, matching the generated schema: the outer
    /// `None` means "don't touch the configuration at all" (rename-only), while
    /// `Some(None)` explicitly clears it.
    fn edit(
        &self,
        server_base: &str,
        id: &str,
        concurrency: uuid::Uuid,
        name: String,
        configuration: Option<serde_json::Value>,
    ) -> Result<DeviceSummary, ApiError> {
        self.api
            .call(server_base, &self.http, |generated_config| {
                let mut schema = UpdateDeviceSchema::new(concurrency);
                schema.name = Some(Some(name.clone()));
                schema.configuration = configuration.as_ref().map(|c| Some(c.clone())); // Explicitly clone the `serde_json::Value`
                async move {
                    device_handler_api::edit_device_handler(&generated_config, id, schema).await
                }
            })
            .map(DeviceSummary::from)
    }

    /// `DELETE /api/v1/devices/{id}`.
    pub fn delete(&self, server_base: &str, id: &str) -> Result<(), ApiError> {
        self.api
            .call(server_base, &self.http, |configuration| async move {
                device_handler_api::delete_device_handler(&configuration, id).await
            })
    }
}

/// Local SQLite cache of devices (table `devices_cache`), plus which one the
/// user has marked as their *active* device.
///
/// This is deliberately separate from `DevicesClient`: the server
/// (`device_handler_api`) is the source of truth for the device list, but
/// the local cache (a) lets Profile > Devices show something before the
/// first network refresh completes and (b) durably remembers the user's
/// active-device choice, which the Live Stream / Device Map screens depend
/// on even if the server is briefly unreachable.
#[derive(Clone)]
pub struct DeviceCacheStore {
    db: SharedDb,
}

impl DeviceCacheStore {
    pub(crate) fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// All cached devices, ordered by name. Reflects the last successful
    /// `DevicesClient::list` call merged with the locally-tracked active
    /// device flag.
    pub fn list_cached(&self) -> Result<Vec<DeviceSummary>> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, category, device_type, created_at, updated_at, concurrency, configuration_json
             FROM devices_cache ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], row_to_device_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading cached devices")?;
        Ok(rows)
    }

    /// The device currently marked active (`is_selected = 1`), if any.
    pub fn selected(&self) -> Result<Option<DeviceSummary>> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        conn.query_row(
            "SELECT id, name, category, device_type, created_at, updated_at, concurrency, configuration_json
             FROM devices_cache WHERE is_selected = 1 LIMIT 1",
            [],
            row_to_device_summary,
        )
        .optional()
        .context("reading active device")
    }

    /// Upserts the full device list from a server refresh. Preserves the
    /// existing `is_selected` flag for devices that already existed (the
    /// active-device choice is a local preference, independent of what the
    /// server returns) and drops cached rows for devices no longer present
    /// server-side.
    pub fn replace_all(&self, devices: &[DeviceSummary]) -> Result<()> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        let tx = conn
            .unchecked_transaction()
            .context("starting devices_cache transaction")?;
        let now_ms = current_unix_ms();
        {
            let mut upsert = tx.prepare(
                "INSERT INTO devices_cache(id, name, category, device_type, created_at, updated_at, concurrency, configuration_json, is_selected, cached_at_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name,
                     category = excluded.category,
                     device_type = excluded.device_type,
                     created_at = excluded.created_at,
                     updated_at = excluded.updated_at,
                     concurrency = excluded.concurrency,
                     configuration_json = excluded.configuration_json,
                     cached_at_ms = excluded.cached_at_ms",
            )?;
            for device in devices {
                upsert.execute(params![
                    device.id,
                    device.name,
                    device.category,
                    device.device_type,
                    device.created_at,
                    device.updated_at,
                    device.concurrency.to_string(),
                    device.configuration,
                    now_ms,
                ])?;
            }
            let placeholders = devices.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            if devices.is_empty() {
                tx.execute("DELETE FROM devices_cache", [])?;
            } else {
                let sql = format!("DELETE FROM devices_cache WHERE id NOT IN ({placeholders})");
                let ids: Vec<&str> = devices.iter().map(|device| device.id.as_str()).collect();
                tx.execute(&sql, rusqlite::params_from_iter(ids))?;
            }
        }
        tx.commit().context("committing devices_cache refresh")?;
        Ok(())
    }

    /// Upserts a single device (used after create/rename), preserving
    /// `is_selected` for existing rows and defaulting new rows to unselected.
    pub fn upsert(&self, device: &DeviceSummary) -> Result<()> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        conn.execute(
            "INSERT INTO devices_cache(id, name, category, device_type, created_at, updated_at, concurrency, configuration_json, is_selected, cached_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 category = excluded.category,
                 device_type = excluded.device_type,
                 created_at = excluded.created_at,
                 updated_at = excluded.updated_at,
                 concurrency = excluded.concurrency,
                 configuration_json = excluded.configuration_json,
                 cached_at_ms = excluded.cached_at_ms",
            params![
                device.id,
                device.name,
                device.category,
                device.device_type,
                device.created_at,
                device.updated_at,
                device.concurrency.to_string(),
                device.configuration,
                current_unix_ms(),
            ],
        )
        .context("upserting cached device")?;
        Ok(())
    }

    /// Removes a device from the cache (used after a successful delete).
    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        conn.execute("DELETE FROM devices_cache WHERE id = ?1", params![id])
            .context("removing cached device")?;
        Ok(())
    }

    /// Marks `id` as the active device, clearing the flag on every other
    /// row in the same transaction.
    pub fn set_selected(&self, id: &str) -> Result<()> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        let tx = conn
            .unchecked_transaction()
            .context("starting set_selected transaction")?;
        tx.execute("UPDATE devices_cache SET is_selected = 0", [])?;
        let updated = tx.execute(
            "UPDATE devices_cache SET is_selected = 1 WHERE id = ?1",
            params![id],
        )?;
        tx.commit().context("committing active device selection")?;
        if updated == 0 {
            anyhow::bail!("device {id} is not in the local cache; refresh devices first");
        }
        Ok(())
    }

    /// Clears the active-device flag entirely (e.g. after the active device
    /// was deleted).
    pub fn clear_selection(&self) -> Result<()> {
        let conn = self.db.lock().expect("devices_cache mutex poisoned");
        conn.execute("UPDATE devices_cache SET is_selected = 0", [])
            .context("clearing active device")?;
        Ok(())
    }
}

fn row_to_device_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceSummary> {
    let concurrency_text: String = row.get(6)?;
    let concurrency =
        uuid::Uuid::parse_str(&concurrency_text).unwrap_or_else(|_| uuid::Uuid::nil());
    Ok(DeviceSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        category: row.get(2)?,
        device_type: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        concurrency,
        configuration: row.get(7)?,
    })
}

fn current_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AppStore;
    use base64::Engine;

    /// Builds a minimal unsigned JWT whose payload sets `exp` to `exp_secs`,
    /// matching the pattern in `tests/auth.rs`.
    fn make_jwt(exp_secs: i64) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = serde_json::json!({"exp": exp_secs, "sub": "user"});
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{header}.{payload_b64}.")
    }

    /// Signs `store` in against `server` via a mocked `/api/v1/account/login`,
    /// so subsequent `store.devices()` calls have a valid session to work with.
    fn sign_in(store: &AppStore, server: &mut mockito::Server) {
        let token = make_jwt(2_000_000_000);
        let body =
            serde_json::json!({"access_token": token, "refresh_token": "abc", "status": "success"})
                .to_string();
        server
            .mock("POST", "/api/v1/account/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        store
            .auth()
            .login(&server.url(), "me@example.test", "secret")
            .expect("login succeeds");
    }

    fn device_model_json(id: uuid::Uuid, concurrency: uuid::Uuid, name: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": name,
            "device_category": "Underwater",
            "device_type": "ChasingM2S",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "concurrency": concurrency,
            "configuration": null,
            "user_id": uuid::Uuid::new_v4(),
        })
    }

    // ---- DeviceCacheStore (local cache, no network) ------------------------

    fn sample_summary(id: &str, name: &str) -> DeviceSummary {
        DeviceSummary {
            id: id.to_string(),
            name: name.to_string(),
            category: "Underwater".to_string(),
            device_type: "ChasingM2S".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            concurrency: uuid::Uuid::new_v4(),
            configuration: None,
        }
    }

    #[test]
    fn cache_starts_empty() {
        let store = AppStore::open_in_memory().unwrap();
        assert!(store.device_cache().list_cached().unwrap().is_empty());
        assert!(store.device_cache().selected().unwrap().is_none());
    }

    #[test]
    fn replace_all_populates_cache_ordered_by_name() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache
            .replace_all(&[
                sample_summary("id-b", "Bravo"),
                sample_summary("id-a", "Alpha"),
            ])
            .unwrap();
        let listed = cache.list_cached().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "Alpha");
        assert_eq!(listed[1].name, "Bravo");
    }

    #[test]
    fn replace_all_drops_devices_no_longer_present() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache
            .replace_all(&[sample_summary("id-a", "Alpha")])
            .unwrap();
        cache
            .replace_all(&[sample_summary("id-b", "Bravo")])
            .unwrap();
        let listed = cache.list_cached().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "id-b");
    }

    #[test]
    fn replace_all_preserves_selection_for_existing_device() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache
            .replace_all(&[sample_summary("id-a", "Alpha")])
            .unwrap();
        cache.set_selected("id-a").unwrap();
        // A refresh that still includes id-a must not clear the selection.
        cache
            .replace_all(&[
                sample_summary("id-a", "Alpha"),
                sample_summary("id-b", "Bravo"),
            ])
            .unwrap();
        let selected = cache.selected().unwrap().expect("selection preserved");
        assert_eq!(selected.id, "id-a");
    }

    #[test]
    fn upsert_inserts_and_updates() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache.upsert(&sample_summary("id-a", "Alpha")).unwrap();
        assert_eq!(cache.list_cached().unwrap().len(), 1);
        let mut renamed = sample_summary("id-a", "Alpha Renamed");
        renamed.concurrency = uuid::Uuid::new_v4();
        cache.upsert(&renamed).unwrap();
        let listed = cache.list_cached().unwrap();
        assert_eq!(listed.len(), 1, "upsert must not duplicate the row");
        assert_eq!(listed[0].name, "Alpha Renamed");
    }

    #[test]
    fn remove_deletes_the_row() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache.upsert(&sample_summary("id-a", "Alpha")).unwrap();
        cache.remove("id-a").unwrap();
        assert!(cache.list_cached().unwrap().is_empty());
    }

    #[test]
    fn set_selected_switches_the_active_device() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache
            .replace_all(&[
                sample_summary("id-a", "Alpha"),
                sample_summary("id-b", "Bravo"),
            ])
            .unwrap();
        cache.set_selected("id-a").unwrap();
        assert_eq!(cache.selected().unwrap().unwrap().id, "id-a");
        cache.set_selected("id-b").unwrap();
        assert_eq!(
            cache.selected().unwrap().unwrap().id,
            "id-b",
            "selecting a new device must clear the previous flag"
        );
    }

    #[test]
    fn set_selected_rejects_unknown_id() {
        let store = AppStore::open_in_memory().unwrap();
        let err = store.device_cache().set_selected("missing").unwrap_err();
        assert!(format!("{err}").contains("not in the local cache"));
    }

    #[test]
    fn clear_selection_unsets_the_active_device() {
        let store = AppStore::open_in_memory().unwrap();
        let cache = store.device_cache();
        cache
            .replace_all(&[sample_summary("id-a", "Alpha")])
            .unwrap();
        cache.set_selected("id-a").unwrap();
        cache.clear_selection().unwrap();
        assert!(cache.selected().unwrap().is_none());
    }

    // ---- DevicesClient (remote calls, mocked server) -----------------------

    #[test]
    fn list_returns_devices_from_server() {
        let mut server = mockito::Server::new();
        let store = AppStore::open_in_memory().unwrap();
        sign_in(&store, &mut server);

        let id = uuid::Uuid::new_v4();
        let concurrency = uuid::Uuid::new_v4();
        let body = serde_json::json!({
            "items": [device_model_json(id, concurrency, "ROV One")],
            "limit": 200,
            "page": 1,
            "total": 1,
        })
        .to_string();
        let list_mock = server
            .mock("GET", "/api/v1/devices")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();

        let devices = store.devices().list(&server.url()).expect("list succeeds");
        list_mock.assert();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, id.to_string());
        assert_eq!(devices[0].name, "ROV One");
        assert_eq!(devices[0].concurrency, concurrency);
    }

    #[test]
    fn list_surfaces_server_error() {
        let mut server = mockito::Server::new();
        let store = AppStore::open_in_memory().unwrap();
        sign_in(&store, &mut server);

        server
            .mock("GET", "/api/v1/devices")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .with_body(r#"{"message":"boom"}"#)
            .create();

        let err = store
            .devices()
            .list(&server.url())
            .expect_err("should fail");
        match err {
            ApiError::Server { status, .. } => assert_eq!(status.as_u16(), 500),
            other => panic!("expected ApiError::Server, got {other}"),
        }
    }

    #[test]
    fn me_id_returns_the_signed_in_user_id() {
        let mut server = mockito::Server::new();
        let store = AppStore::open_in_memory().unwrap();
        sign_in(&store, &mut server);

        let user_id = uuid::Uuid::new_v4();
        let body = serde_json::json!({
            "id": user_id,
            "email": "me@example.test",
            "name": "Me",
            "surname": "Example",
            "role": "User",
            "is_active": true,
            "password": "hashed",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "concurrency": uuid::Uuid::new_v4(),
        })
        .to_string();
        server
            .mock("GET", "/api/v1/profile/info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();

        let returned = store
            .devices()
            .me_id(&server.url())
            .expect("me_id succeeds");
        assert_eq!(returned, user_id);
    }

    #[test]
    fn create_sends_resolved_user_id_and_returns_device() {
        let mut server = mockito::Server::new();
        let store = AppStore::open_in_memory().unwrap();
        sign_in(&store, &mut server);

        let user_id = uuid::Uuid::new_v4();
        let me_body = serde_json::json!({
            "id": user_id,
            "email": "me@example.test",
            "name": "Me",
            "surname": "Example",
            "role": "User",
            "is_active": true,
            "password": "hashed",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "concurrency": uuid::Uuid::new_v4(),
        })
        .to_string();
        server
            .mock("GET", "/api/v1/profile/info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(me_body)
            .create();

        let device_id = uuid::Uuid::new_v4();
        let concurrency = uuid::Uuid::new_v4();
        let create_body = device_model_json(device_id, concurrency, "New ROV").to_string();
        let create_mock = server
            .mock("POST", "/api/v1/devices")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "name": "New ROV",
                "user_id": user_id,
            })))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(create_body)
            .create();

        let created = store
            .devices()
            .create(&server.url(), "New ROV".to_string(), None)
            .expect("create succeeds");
        create_mock.assert();
        assert_eq!(created.id, device_id.to_string());
        assert_eq!(created.name, "New ROV");
    }

    #[test]
    fn delete_calls_the_delete_endpoint() {
        let mut server = mockito::Server::new();
        let store = AppStore::open_in_memory().unwrap();
        sign_in(&store, &mut server);

        let delete_mock = server
            .mock("DELETE", "/api/v1/devices/id-a")
            .with_status(200)
            .create();

        store
            .devices()
            .delete(&server.url(), "id-a")
            .expect("delete succeeds");
        delete_mock.assert();
    }
}
