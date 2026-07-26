-- Local cache of devices fetched from the third-eye server (v3).
--
-- The server (`device_handler_api`) is the source of truth for the device
-- list itself, but the client needs an offline-available local copy (so the
-- Profile > Devices screen has something to show before the first refresh
-- completes) and, more importantly, a durable record of which device the
-- user has chosen as their *active* device. The active device drives the
-- Live Stream / Device Map screens (its `configuration_json`, when present,
-- seeds the RTSP/ROV HTTP settings), so this choice must survive restarts
-- even if the server is briefly unreachable.

CREATE TABLE devices_cache (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    category           TEXT NOT NULL,
    device_type        TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    concurrency        TEXT NOT NULL,
    configuration_json TEXT,
    -- Exactly zero or one row may have is_selected = 1; enforced in
    -- application code (DeviceCacheStore::set_selected clears the others
    -- inside the same transaction), not by a SQL constraint.
    is_selected        INTEGER NOT NULL DEFAULT 0,
    cached_at_ms        INTEGER NOT NULL
);

CREATE INDEX devices_cache_selected_idx ON devices_cache(is_selected);
