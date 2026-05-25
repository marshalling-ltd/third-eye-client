-- Persistent OSM tile cache for offline map support (v2).
--
-- Tiles are stored as raw PNG blobs keyed by (z, x, y). The
-- `last_accessed_ms` column tracks when a tile was last read so an
-- LRU eviction policy can keep the total size under the configured cap.

CREATE TABLE map_tile_cache (
    z               INTEGER NOT NULL,
    x               INTEGER NOT NULL,
    y               INTEGER NOT NULL,
    png_data        BLOB    NOT NULL,
    size_bytes      INTEGER NOT NULL,
    last_accessed_ms INTEGER NOT NULL,
    PRIMARY KEY (z, x, y)
);

CREATE INDEX map_tile_cache_lru_idx ON map_tile_cache(last_accessed_ms);
