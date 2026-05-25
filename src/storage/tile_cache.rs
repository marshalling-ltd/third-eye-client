//! Persistent map tile cache backed by SQLite.
//!
//! Tiles are stored as raw PNG blobs keyed by `(z, x, y)`. Each row tracks
//! its `last_accessed_ms` timestamp so the cache can evict the least-recently
//! used tiles when the total size exceeds the configured cap.

use anyhow::{Context, Result};
use rusqlite::params;

use super::db::SharedDb;

/// Accessor for the `map_tile_cache` table.
#[derive(Clone)]
pub struct TileCacheStore {
    db: SharedDb,
}

impl TileCacheStore {
    pub(crate) fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// Returns the raw PNG bytes for a cached tile, updating its
    /// `last_accessed_ms` timestamp. Returns `None` on cache miss.
    pub fn get_tile(&self, z: u32, x: i64, y: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.db.lock().expect("tile_cache mutex poisoned");
        let now_ms = current_unix_ms();
        // Touch the access timestamp first (no-op if the row doesn't exist).
        conn.execute(
            "UPDATE map_tile_cache SET last_accessed_ms = ?4
             WHERE z = ?1 AND x = ?2 AND y = ?3",
            params![z, x, y, now_ms],
        )
        .context("updating tile last_accessed_ms")?;
        let result = conn
            .query_row(
                "SELECT png_data FROM map_tile_cache WHERE z = ?1 AND x = ?2 AND y = ?3",
                params![z, x, y],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .context("reading cached tile")?;
        Ok(result)
    }

    /// Inserts or replaces a tile in the cache.
    pub fn put_tile(&self, z: u32, x: i64, y: i64, png_data: &[u8]) -> Result<()> {
        let size_bytes = png_data.len() as i64;
        let now_ms = current_unix_ms();
        let conn = self.db.lock().expect("tile_cache mutex poisoned");
        conn.execute(
            "INSERT INTO map_tile_cache (z, x, y, png_data, size_bytes, last_accessed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(z, x, y) DO UPDATE SET
                 png_data = excluded.png_data,
                 size_bytes = excluded.size_bytes,
                 last_accessed_ms = excluded.last_accessed_ms",
            params![z, x, y, png_data, size_bytes, now_ms],
        )
        .context("inserting cached tile")?;
        Ok(())
    }

    /// Returns the total size in bytes of all cached tiles.
    pub fn total_size(&self) -> Result<u64> {
        let conn = self.db.lock().expect("tile_cache mutex poisoned");
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM map_tile_cache",
                [],
                |row| row.get(0),
            )
            .context("computing tile cache total size")?;
        Ok(total.max(0) as u64)
    }

    /// Evicts the least-recently-used tiles until the total cache size is at
    /// or below `max_bytes`.
    pub fn evict_lru(&self, max_bytes: u64) -> Result<u64> {
        let mut evicted = 0u64;
        let conn = self.db.lock().expect("tile_cache mutex poisoned");
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM map_tile_cache",
                [],
                |row| row.get(0),
            )
            .context("computing tile cache total size for eviction")?;
        let mut remaining = total.max(0) as u64;
        if remaining <= max_bytes {
            return Ok(0);
        }
        // Delete in batches ordered by oldest access time.
        let mut stmt = conn
            .prepare(
                "SELECT z, x, y, size_bytes FROM map_tile_cache
                 ORDER BY last_accessed_ms ASC",
            )
            .context("preparing LRU eviction query")?;
        let rows: Vec<(u32, i64, i64, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .context("querying LRU tiles")?
            .collect::<Result<_, _>>()
            .context("collecting LRU tiles")?;
        for (z, x, y, size) in rows {
            if remaining <= max_bytes {
                break;
            }
            conn.execute(
                "DELETE FROM map_tile_cache WHERE z = ?1 AND x = ?2 AND y = ?3",
                params![z, x, y],
            )
            .context("deleting evicted tile")?;
            let freed = size.max(0) as u64;
            remaining = remaining.saturating_sub(freed);
            evicted += freed;
        }
        Ok(evicted)
    }
}

use rusqlite::OptionalExtension;

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::open_in_memory;

    #[test]
    fn round_trip_tile() {
        let db = open_in_memory().unwrap();
        let store = TileCacheStore::new(db);
        assert!(store.get_tile(14, 100, 200).unwrap().is_none());
        let png = vec![0x89, 0x50, 0x4E, 0x47]; // fake PNG header
        store.put_tile(14, 100, 200, &png).unwrap();
        let got = store.get_tile(14, 100, 200).unwrap().unwrap();
        assert_eq!(got, png);
    }

    #[test]
    fn total_size_tracks_inserts() {
        let db = open_in_memory().unwrap();
        let store = TileCacheStore::new(db);
        assert_eq!(store.total_size().unwrap(), 0);
        store.put_tile(1, 0, 0, &[0u8; 100]).unwrap();
        store.put_tile(1, 0, 1, &[0u8; 200]).unwrap();
        assert_eq!(store.total_size().unwrap(), 300);
    }

    #[test]
    fn evict_lru_removes_oldest_first() {
        let db = open_in_memory().unwrap();
        let store = TileCacheStore::new(db);
        // Insert three tiles. Because they are inserted sequentially,
        // tile (1,0,0) has the oldest last_accessed_ms.
        store.put_tile(1, 0, 0, &[0u8; 500]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.put_tile(1, 0, 1, &[0u8; 500]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.put_tile(1, 0, 2, &[0u8; 500]).unwrap();

        assert_eq!(store.total_size().unwrap(), 1500);
        let evicted = store.evict_lru(1000).unwrap();
        assert!(evicted >= 500, "should have evicted at least 500 bytes");
        assert!(store.total_size().unwrap() <= 1000);
        // The oldest tile should be gone.
        assert!(store.get_tile(1, 0, 0).unwrap().is_none());
        // The newest should survive.
        assert!(store.get_tile(1, 0, 2).unwrap().is_some());
    }

    #[test]
    fn put_tile_replaces_existing() {
        let db = open_in_memory().unwrap();
        let store = TileCacheStore::new(db);
        store.put_tile(5, 3, 7, &[1, 2, 3]).unwrap();
        store.put_tile(5, 3, 7, &[4, 5, 6, 7]).unwrap();
        let got = store.get_tile(5, 3, 7).unwrap().unwrap();
        assert_eq!(got, vec![4, 5, 6, 7]);
        assert_eq!(store.total_size().unwrap(), 4);
    }
}
