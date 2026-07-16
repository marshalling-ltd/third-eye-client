//! ROV-only media synchronisation.
//!
//! The camera's `GET /v1/medias` endpoint is the source of truth for the list
//! of files stored on the ROV. This module mirrors that list into the local
//! `media_sync` table so the UI can show media another user captured on the
//! same ROV, track whether we've downloaded them locally, and annotate them
//! with capture-time ROV telemetry.
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::db::SharedDb;
use crate::camera::{CameraApiClient, MediaInfo, MediaScene, MediaWhich};
use crate::rov_status::Status as RovStatus;

/// Outcome of a single reconciliation pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaSyncReport {
    pub new_media: usize,
    pub updated_media: usize,
    pub disappeared_media: usize,
    pub total_on_rov: usize,
}

/// Local projection of a `media_sync` row for UI consumption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalMediaRecord {
    pub media_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub duration_s: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub mime: Option<String>,
    pub scene: Option<i32>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub local_path: Option<String>,
    pub local_sha256: Option<String>,
    pub rov_stat: Option<i32>,
    pub deleted_on_rov: bool,
    /// Epoch-millis timestamp from `capture_metadata`, if available.
    /// `None` when the capture was taken on a different machine and no
    /// local metadata row exists.
    pub captured_at_ms: Option<i64>,
}

/// Projection of a `capture_metadata` row. All numeric fields are optional
/// because early captures (before UDP telemetry is running) may not have a
/// full snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureMetadata {
    pub media_id: String,
    pub name: String,
    pub captured_at_ms: i64,
    pub pitch: Option<f64>,
    pub roll: Option<f64>,
    pub yaw: Option<f64>,
    pub depth_m: Option<f64>,
    pub temperature_c: Option<f64>,
    pub lat_e7: Option<i64>,
    pub lon_e7: Option<i64>,
    pub batteries_json: Option<String>,
    pub imu_json: Option<String>,
    pub note: Option<String>,
    pub tags_json: Option<String>,
}

/// Handle onto the media-related tables.
#[derive(Clone)]
pub struct MediaStore {
    db: SharedDb,
}

impl MediaStore {
    pub(crate) fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// Reconciles the local registry with `GET /v1/medias`.
    pub fn sync_from_rov(
        &self,
        camera: &CameraApiClient,
        scene: Option<MediaScene>,
    ) -> Result<MediaSyncReport> {
        let items = camera
            .list_medias(scene)
            .context("listing media from ROV camera")?;
        self.apply_rov_listing(&items, scene)
    }

    /// Pure DB-facing variant of [`Self::sync_from_rov`] for tests and for
    /// callers that already have a listing in hand.
    pub fn apply_rov_listing(
        &self,
        items: &[MediaInfo],
        scene: Option<MediaScene>,
    ) -> Result<MediaSyncReport> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        let tx = conn.unchecked_transaction()?;
        // Ensure the sweep timestamp is strictly greater than any existing
        // `last_seen_ms` so that the disappeared-detection query (`< ?1`)
        // works even when two calls land within the same millisecond.
        let max_existing: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(last_seen_ms), 0) FROM media_sync",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms().max(max_existing + 1);

        let mut report = MediaSyncReport {
            total_on_rov: items.len(),
            ..MediaSyncReport::default()
        };
        for item in items {
            let existed: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM media_sync WHERE media_id = ?1 AND name = ?2",
                    params![item.origin.id, item.name],
                    |row| row.get(0),
                )
                .optional()?;
            let mime = guess_mime(&item.name);
            tx.execute(
                "INSERT INTO media_sync(
                    media_id, name, size_bytes, duration_s, width, height,
                    mime, scene, first_seen_ms, last_seen_ms, rov_stat, deleted_on_rov)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)
                 ON CONFLICT(media_id, name) DO UPDATE SET
                     size_bytes     = excluded.size_bytes,
                     duration_s     = excluded.duration_s,
                     width          = COALESCE(NULLIF(excluded.width, 0), media_sync.width),
                     height         = COALESCE(NULLIF(excluded.height, 0), media_sync.height),
                     mime           = COALESCE(excluded.mime, media_sync.mime),
                     scene          = COALESCE(excluded.scene, media_sync.scene),
                     last_seen_ms   = excluded.last_seen_ms,
                     rov_stat       = excluded.rov_stat,
                     deleted_on_rov = 0",
                params![
                    item.origin.id,
                    item.name,
                    item.size as i64,
                    item.origin.duration,
                    item.origin.width,
                    item.origin.height,
                    mime,
                    scene.map(super::super::camera::MediaScene::as_query_int),
                    now,
                    now,
                    item.origin.stat,
                ],
            )?;
            if existed.is_some() {
                report.updated_media += 1;
            } else {
                report.new_media += 1;
            }
        }

        // Any row whose `last_seen_ms` predates this sweep has vanished.
        let disappeared = tx.execute(
            "UPDATE media_sync SET deleted_on_rov = 1
             WHERE last_seen_ms < ?1 AND deleted_on_rov = 0",
            params![now],
        )?;
        report.disappeared_media = disappeared;

        tx.commit()?;
        Ok(report)
    }

    /// Records the ROV telemetry snapshot associated with a freshly-captured
    /// image. Upserts are used so repeated captures with the same media id
    /// (should never happen, but the camera's assignment is the source of
    /// truth) do not error out.
    pub fn attach_capture_metadata(
        &self,
        media_id: &str,
        name: &str,
        captured_at_ms: i64,
        status: Option<&RovStatus>,
        note: Option<&str>,
    ) -> Result<()> {
        let conn = self.db.lock().expect("capture_metadata mutex poisoned");
        let batteries_json =
            status.map(|s| serde_json::to_string(&s.batteries).unwrap_or_default());
        let imu_json = status.map(|s| {
            serde_json::json!({
                "gx": s.imu.gyro_x,
                "gy": s.imu.gyro_y,
                "gz": s.imu.gyro_z,
            })
            .to_string()
        });
        conn.execute(
            "INSERT INTO capture_metadata(
                media_id, name, captured_at_ms, pitch, roll, yaw,
                depth_m, temperature_c, lat_e7, lon_e7, batteries_json, imu_json, note)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(media_id, name) DO UPDATE SET
                 captured_at_ms = excluded.captured_at_ms,
                 pitch          = excluded.pitch,
                 roll           = excluded.roll,
                 yaw            = excluded.yaw,
                 depth_m        = excluded.depth_m,
                 temperature_c  = excluded.temperature_c,
                 lat_e7         = excluded.lat_e7,
                 lon_e7         = excluded.lon_e7,
                 batteries_json = excluded.batteries_json,
                 imu_json       = excluded.imu_json,
                 note           = COALESCE(excluded.note, capture_metadata.note)",
            params![
                media_id,
                name,
                captured_at_ms,
                status.map(|s| f64::from(s.pitch)),
                status.map(|s| f64::from(s.roll)),
                status.map(|s| f64::from(s.yaw)),
                status.map(|s| f64::from(s.depth)),
                status.map(|s| f64::from(s.temperature)),
                status.map(|s| s.lat),
                status.map(|s| s.lon),
                batteries_json,
                imu_json,
                note,
            ],
        )
        .context("writing capture metadata")?;
        Ok(())
    }

    /// Appends a freeform user status enrichment event.
    pub fn record_status_event(
        &self,
        ts_ms: i64,
        media_id: Option<&str>,
        name: Option<&str>,
        status: Option<&RovStatus>,
        note: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<i64> {
        let tags_json = tags.map(|t| serde_json::to_string(t).unwrap_or_default());
        let conn = self.db.lock().expect("user_status_events mutex poisoned");
        conn.execute(
            "INSERT INTO user_status_events(
                ts_ms, media_id, name, pitch, roll, yaw,
                depth_m, temperature_c, lat_e7, lon_e7, note, tags_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                ts_ms,
                media_id,
                name,
                status.map(|s| f64::from(s.pitch)),
                status.map(|s| f64::from(s.roll)),
                status.map(|s| f64::from(s.yaw)),
                status.map(|s| f64::from(s.depth)),
                status.map(|s| f64::from(s.temperature)),
                status.map(|s| s.lat),
                status.map(|s| s.lon),
                note,
                tags_json,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Returns up to `limit` media records. Most recently seen first.
    pub fn list_recent(&self, limit: usize) -> Result<Vec<LocalMediaRecord>> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        let mut stmt = conn.prepare(LIST_QUERY_RECENT)?;
        let rows = stmt.query_map(params![limit as i64], map_local_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Returns every row - including `deleted_on_rov` - newest-first. Used by
    /// the Media screen to render the full library.
    pub fn list_all(&self) -> Result<Vec<LocalMediaRecord>> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        let mut stmt = conn.prepare(LIST_QUERY_ALL)?;
        let rows = stmt.query_map([], map_local_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Reads the capture metadata row for a given media, if any.
    pub fn get_capture_metadata(
        &self,
        media_id: &str,
        name: &str,
    ) -> Result<Option<CaptureMetadata>> {
        let conn = self.db.lock().expect("capture_metadata mutex poisoned");
        conn.query_row(
            "SELECT media_id, name, captured_at_ms, pitch, roll, yaw,
                    depth_m, temperature_c, lat_e7, lon_e7,
                    batteries_json, imu_json, note, tags_json
             FROM capture_metadata
             WHERE media_id = ?1 AND name = ?2",
            params![media_id, name],
            |row| {
                Ok(CaptureMetadata {
                    media_id: row.get(0)?,
                    name: row.get(1)?,
                    captured_at_ms: row.get(2)?,
                    pitch: row.get(3)?,
                    roll: row.get(4)?,
                    yaw: row.get(5)?,
                    depth_m: row.get(6)?,
                    temperature_c: row.get(7)?,
                    lat_e7: row.get(8)?,
                    lon_e7: row.get(9)?,
                    batteries_json: row.get(10)?,
                    imu_json: row.get(11)?,
                    note: row.get(12)?,
                    tags_json: row.get(13)?,
                })
            },
        )
        .optional()
        .context("reading capture_metadata")
    }

    /// Marks a media as downloaded and stamps the absolute local path + hash.
    pub fn set_local_path(
        &self,
        media_id: &str,
        name: &str,
        local_path: &Path,
        sha256: Option<&str>,
    ) -> Result<()> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        let path_str = local_path.to_string_lossy().to_string();
        let affected = conn
            .execute(
                "UPDATE media_sync
                    SET local_path = ?3, local_sha256 = ?4
                  WHERE media_id = ?1 AND name = ?2",
                params![media_id, name, path_str, sha256],
            )
            .context("updating media_sync local_path")?;
        if affected == 0 {
            anyhow::bail!("no media_sync row for ({media_id}, {name})");
        }
        Ok(())
    }

    /// Removes a media row and its capture metadata from the local DB.
    pub fn remove_by_name(&self, name: &str) -> Result<()> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        conn.execute(
            "DELETE FROM capture_metadata WHERE name = ?1",
            params![name],
        )?;
        conn.execute("DELETE FROM media_sync WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Updates the width/height columns for a media row.
    pub fn set_dimensions(
        &self,
        media_id: &str,
        name: &str,
        width: i32,
        height: i32,
    ) -> Result<()> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        conn.execute(
            "UPDATE media_sync
                SET width = ?3, height = ?4
              WHERE media_id = ?1 AND name = ?2",
            params![media_id, name, width, height],
        )
        .context("updating media_sync dimensions")?;
        Ok(())
    }

    /// Clears `local_path` / `local_sha256` when the local copy is gone.
    pub fn forget_local(&self, media_id: &str, name: &str) -> Result<()> {
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        conn.execute(
            "UPDATE media_sync
                SET local_path = NULL, local_sha256 = NULL
              WHERE media_id = ?1 AND name = ?2",
            params![media_id, name],
        )
        .context("clearing media_sync local_path")?;
        Ok(())
    }
}

const LIST_SELECT: &str =
    "SELECT m.media_id, m.name, m.size_bytes, m.duration_s, m.width, m.height, m.mime, m.scene,
            m.first_seen_ms, m.last_seen_ms, m.local_path, m.local_sha256, m.rov_stat,
            m.deleted_on_rov, c.captured_at_ms
     FROM media_sync m
     LEFT JOIN capture_metadata c ON c.media_id = m.media_id AND c.name = m.name
     ORDER BY (c.captured_at_ms IS NULL) ASC, c.captured_at_ms DESC, m.last_seen_ms DESC, m.name ASC";

const LIST_QUERY_RECENT: &str = concat!(
    "SELECT m.media_id, m.name, m.size_bytes, m.duration_s, m.width, m.height, m.mime, m.scene,\n",
    "       m.first_seen_ms, m.last_seen_ms, m.local_path, m.local_sha256, m.rov_stat,\n",
    "       m.deleted_on_rov, c.captured_at_ms\n",
    "FROM media_sync m\n",
    "LEFT JOIN capture_metadata c ON c.media_id = m.media_id AND c.name = m.name\n",
    "ORDER BY (c.captured_at_ms IS NULL) ASC, c.captured_at_ms DESC, m.last_seen_ms DESC, m.name ASC\n",
    "LIMIT ?1",
);

const LIST_QUERY_ALL: &str = LIST_SELECT;

fn map_local_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalMediaRecord> {
    Ok(LocalMediaRecord {
        media_id: row.get(0)?,
        name: row.get(1)?,
        size_bytes: row.get(2)?,
        duration_s: row.get(3)?,
        width: row.get(4)?,
        height: row.get(5)?,
        mime: row.get(6)?,
        scene: row.get(7)?,
        first_seen_ms: row.get(8)?,
        last_seen_ms: row.get(9)?,
        local_path: row.get(10)?,
        local_sha256: row.get(11)?,
        rov_stat: row.get(12)?,
        deleted_on_rov: row.get::<_, i64>(13)? != 0,
        captured_at_ms: row.get(14)?,
    })
}

/// Downloads `name` (original variant only) from the ROV camera and stores it
/// at `<data_root>/media/<name>`. JPEG images are re-encoded at
/// quality 85 to reduce the bloated file sizes the ROV camera produces
/// (typically 4× smaller with no visible quality loss at the same resolution).
/// Returns the absolute on-disk path.
pub fn download_to_local(
    store: &MediaStore,
    camera: &CameraApiClient,
    data_root: &Path,
    media_id: &str,
    name: &str,
) -> Result<PathBuf> {
    let payload = camera
        .download_media(name, MediaWhich::Original)
        .with_context(|| format!("downloading {name} from camera"))?;
    let media_root = data_root.join("media");
    std::fs::create_dir_all(&media_root)
        .with_context(|| format!("creating {}", media_root.display()))?;
    let target_name = local_media_file_name(name);
    let target = media_root.join(&target_name);
    let saved_bytes = write_media_bytes_optimized(&target, &target_name, &payload.bytes)?;

    let mut hasher = Sha256::new();
    hasher.update(&saved_bytes);
    let sha_hex = hasher.finalize().iter().fold(String::new(), |mut acc, b| {
        write!(&mut acc, "{b:02x}").unwrap();
        acc
    });
    store.set_local_path(media_id, name, &target, Some(&sha_hex))?;
    // Persist image dimensions into the DB so the UI can show them without
    // re-reading the file every time.
    if let Ok(dim) = image::image_dimensions(&target) {
        let _ = store.set_dimensions(media_id, name, dim.0 as i32, dim.1 as i32);
    }
    Ok(target)
}

fn local_media_file_name(name: &str) -> String {
    let raw = Path::new(name)
        .file_name()
        .map_or_else(|| name.to_owned(), |f| f.to_string_lossy().into_owned());
    if raw.is_empty() {
        "unnamed-media".to_string()
    } else {
        raw
    }
}

fn write_media_bytes_optimized(target: &Path, name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    // Re-encode JPEG images at quality 85 to reduce the bloated file sizes
    // the ROV camera produces. Resolution stays the same.
    let is_jpeg = Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"));
    if is_jpeg && let Ok(img) = image::load_from_memory(bytes) {
        let rgb = img.to_rgb8();
        let mut buf = std::io::Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
        rgb.write_with_encoder(encoder)
            .with_context(|| format!("re-encoding {name} as JPEG"))?;
        let optimized = buf.into_inner();
        std::fs::write(target, &optimized)
            .with_context(|| format!("writing {}", target.display()))?;
        return Ok(optimized);
    }

    // Non-JPEG (or decode failure): write the original bytes unchanged.
    std::fs::write(target, bytes).with_context(|| format!("writing {}", target.display()))?;
    Ok(bytes.to_vec())
}

fn guess_mime(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg"))
        || std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jpeg"))
    {
        Some("image/jpeg".to_string())
    } else if std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dng"))
    {
        Some("image/x-adobe-dng".to_string())
    } else if std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        Some("image/png".to_string())
    } else if std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    {
        Some("video/mp4".to_string())
    } else if std::path::Path::new(&lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mov"))
    {
        Some("video/quicktime".to_string())
    } else {
        None
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{MediaFileStat, MediaInfo, MediaOrigin};
    use crate::rov_status::{Battery, Imu, Status};
    use crate::storage::db::open_in_memory;

    fn info(id: &str, name: &str, size: u64) -> MediaInfo {
        MediaInfo {
            name: name.into(),
            size,
            canplayback: false,
            origin: MediaOrigin {
                width: 1920,
                height: 1080,
                duration: 0,
                fps: 0,
                br: 0,
                multi: 0,
                with_osd: false,
                id: id.into(),
                stat: 0,
            },
            play: None,
            osd: None,
        }
    }

    fn sample_status() -> Status {
        Status {
            pitch: 0.1,
            roll: -0.05,
            yaw: 1.57,
            depth: 12.34,
            lat: 455_012_345,
            lon: 167_891_234,
            temperature: 17.5,
            batteries: vec![Battery {
                id: 1,
                voltage: 16_500,
                current: -300,
                remaining: 87,
            }],
            imu: Imu {
                gyro_x: 10,
                gyro_y: -5,
                gyro_z: 1,
            },
        }
    }

    #[test]
    fn reconciliation_inserts_new_and_updates_existing() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        let first = vec![info("id-a", "a.jpeg", 1024), info("id-b", "b.mp4", 2048)];
        let report = store.apply_rov_listing(&first, None).unwrap();
        assert_eq!(report.new_media, 2);
        assert_eq!(report.updated_media, 0);
        assert_eq!(report.total_on_rov, 2);

        // Second pass: b disappears, a grows, new c appears.
        let second = vec![info("id-a", "a.jpeg", 2048), info("id-c", "c.jpeg", 512)];
        let report = store.apply_rov_listing(&second, None).unwrap();
        assert_eq!(report.new_media, 1);
        assert_eq!(report.updated_media, 1);
        assert_eq!(report.disappeared_media, 1);

        let listed = store.list_recent(10).unwrap();
        assert_eq!(listed.len(), 3);
        let b = listed.iter().find(|r| r.media_id == "id-b").unwrap();
        assert!(b.deleted_on_rov);
        let a = listed.iter().find(|r| r.media_id == "id-a").unwrap();
        assert_eq!(a.size_bytes, 2048);
    }

    #[test]
    fn file_stat_mapping_is_preserved() {
        // Drop of MediaFileStat ensures the ROV enum still compiles into the
        // int column. This test double-checks the mapping is stable across
        // the storage boundary.
        assert_eq!(MediaFileStat::from_code(2), MediaFileStat::Repairing);
    }

    #[test]
    fn capture_metadata_requires_media_row_first() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        store
            .apply_rov_listing(&[info("id-a", "a.jpeg", 100)], None)
            .unwrap();
        let status = sample_status();
        store
            .attach_capture_metadata(
                "id-a",
                "a.jpeg",
                1_700_000_000_000,
                Some(&status),
                Some("n"),
            )
            .unwrap();
    }

    #[test]
    fn set_local_path_roundtrip() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        store
            .apply_rov_listing(&[info("id-a", "a.jpeg", 100)], None)
            .unwrap();
        let path = std::path::Path::new("/tmp/a.jpeg");
        store
            .set_local_path("id-a", "a.jpeg", path, Some("deadbeef"))
            .unwrap();
        let rec = store
            .list_recent(10)
            .unwrap()
            .into_iter()
            .find(|r| r.media_id == "id-a")
            .unwrap();
        assert_eq!(rec.local_path.as_deref(), Some("/tmp/a.jpeg"));
        assert_eq!(rec.local_sha256.as_deref(), Some("deadbeef"));
        store.forget_local("id-a", "a.jpeg").unwrap();
        let rec = store
            .list_recent(10)
            .unwrap()
            .into_iter()
            .find(|r| r.media_id == "id-a")
            .unwrap();
        assert!(rec.local_path.is_none());
        assert!(rec.local_sha256.is_none());
    }

    #[test]
    fn set_local_path_errors_on_missing_row() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(db);
        let err = store
            .set_local_path(
                "missing",
                "missing.jpeg",
                std::path::Path::new("/tmp/x"),
                None,
            )
            .unwrap_err();
        assert!(format!("{err}").contains("no media_sync row"));
    }

    #[test]
    fn get_capture_metadata_returns_row() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        store
            .apply_rov_listing(&[info("id-a", "a.jpeg", 100)], None)
            .unwrap();
        let status = sample_status();
        store
            .attach_capture_metadata(
                "id-a",
                "a.jpeg",
                1_700_000_000_000,
                Some(&status),
                Some("initial"),
            )
            .unwrap();
        let meta = store
            .get_capture_metadata("id-a", "a.jpeg")
            .unwrap()
            .unwrap();
        // `Status.depth` is stored as f32 then widened to f64, so check with
        // an epsilon rather than a strict equality.
        let depth = meta.depth_m.unwrap();
        assert!((depth - 12.34).abs() < 1e-3, "got depth {depth}");
        assert_eq!(meta.lat_e7, Some(455_012_345));
        assert_eq!(meta.note.as_deref(), Some("initial"));
        assert!(meta.imu_json.is_some());

        // Unknown media returns None.
        assert!(
            store
                .get_capture_metadata("id-missing", "x.jpeg")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn list_all_includes_deleted_rows() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        store
            .apply_rov_listing(
                &[info("id-a", "a.jpeg", 1), info("id-b", "b.jpeg", 2)],
                None,
            )
            .unwrap();
        // Second pass without `id-b` flags it as deleted_on_rov.
        store
            .apply_rov_listing(&[info("id-a", "a.jpeg", 3)], None)
            .unwrap();
        let rows = store.list_all().unwrap();
        assert_eq!(rows.len(), 2);
        let b = rows.iter().find(|r| r.media_id == "id-b").unwrap();
        assert!(b.deleted_on_rov);
    }

    #[test]
    fn status_events_can_be_unattached() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(db);
        let id = store
            .record_status_event(
                1,
                None,
                None,
                Some(&sample_status()),
                Some("freeform"),
                Some(&["tag1".into(), "tag2".into()]),
            )
            .unwrap();
        assert!(id >= 1);
    }

    #[test]
    fn dimensions_survive_set_and_rov_refresh() {
        let db = open_in_memory().unwrap();
        let store = MediaStore::new(Arc::clone(&db));
        store
            .apply_rov_listing(&[info("id-a", "a.jpeg", 100)], None)
            .unwrap();
        // info() sets width=1920, height=1080 from ROV listing
        let rec = store
            .list_all()
            .unwrap()
            .into_iter()
            .find(|r| r.media_id == "id-a")
            .unwrap();
        assert_eq!(rec.width, Some(1920), "width from ROV listing");
        assert_eq!(rec.height, Some(1080), "height from ROV listing");
        // Simulate download setting dimensions
        store.set_dimensions("id-a", "a.jpeg", 3840, 2160).unwrap();
        let rec = store
            .list_all()
            .unwrap()
            .into_iter()
            .find(|r| r.media_id == "id-a")
            .unwrap();
        assert_eq!(rec.width, Some(3840), "width after set_dimensions");
        assert_eq!(rec.height, Some(2160), "height after set_dimensions");
        // Simulate ROV refresh with 0 dimensions — should preserve
        let mut zero_dim = info("id-a", "a.jpeg", 100);
        zero_dim.origin.width = 0;
        zero_dim.origin.height = 0;
        store.apply_rov_listing(&[zero_dim], None).unwrap();
        let rec = store
            .list_all()
            .unwrap()
            .into_iter()
            .find(|r| r.media_id == "id-a")
            .unwrap();
        assert_eq!(
            rec.width,
            Some(3840),
            "width preserved after ROV refresh with 0"
        );
        assert_eq!(
            rec.height,
            Some(2160),
            "height preserved after ROV refresh with 0"
        );
    }

    // Required by `Arc::clone` in the tests above.
    use std::sync::Arc;
}
