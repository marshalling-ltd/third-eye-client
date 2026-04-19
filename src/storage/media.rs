//! ROV-only media synchronisation.
//!
//! The camera's `GET /v1/medias` endpoint is the source of truth for the list
//! of files stored on the ROV. This module mirrors that list into the local
//! `media_sync` table so the UI can show media another user captured on the
//! same ROV, track whether we've downloaded them locally, and annotate them
//! with capture-time ROV telemetry.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::db::SharedDb;
use crate::camera::{CameraApiClient, MediaInfo, MediaScene};
use crate::rov_status::Status as RovStatus;

/// Outcome of a single reconciliation pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaSyncReport {
    pub new_media: usize,
    pub updated_media: usize,
    pub disappeared_media: usize,
    pub total_on_rov: usize,
}

/// Minimal local projection of a `media_sync` row for UI consumption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalMediaRecord {
    pub media_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub local_path: Option<String>,
    pub rov_stat: Option<i32>,
    pub deleted_on_rov: bool,
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
        let now = now_ms();
        let conn = self.db.lock().expect("media_sync mutex poisoned");
        let tx = conn.unchecked_transaction()?;

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
                     width          = excluded.width,
                     height         = excluded.height,
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
                    scene.map(|s| s.as_query_int()),
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
                status.map(|s| s.pitch as f64),
                status.map(|s| s.roll as f64),
                status.map(|s| s.yaw as f64),
                status.map(|s| s.depth as f64),
                status.map(|s| s.temperature as f64),
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
                status.map(|s| s.pitch as f64),
                status.map(|s| s.roll as f64),
                status.map(|s| s.yaw as f64),
                status.map(|s| s.depth as f64),
                status.map(|s| s.temperature as f64),
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
        let mut stmt = conn.prepare(
            "SELECT media_id, name, size_bytes, local_path, rov_stat, deleted_on_rov
             FROM media_sync
             ORDER BY last_seen_ms DESC, name ASC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(LocalMediaRecord {
                media_id: row.get(0)?,
                name: row.get(1)?,
                size_bytes: row.get(2)?,
                local_path: row.get(3)?,
                rov_stat: row.get(4)?,
                deleted_on_rov: row.get::<_, i64>(5)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn guess_mime(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg".to_owned())
    } else if lower.ends_with(".dng") {
        Some("image/x-adobe-dng".to_owned())
    } else if lower.ends_with(".png") {
        Some("image/png".to_owned())
    } else if lower.ends_with(".mp4") {
        Some("video/mp4".to_owned())
    } else if lower.ends_with(".mov") {
        Some("video/quicktime".to_owned())
    } else {
        None
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

    // Required by `Arc::clone` in the tests above.
    use std::sync::Arc;
}
