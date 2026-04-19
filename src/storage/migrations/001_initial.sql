-- Third Eye Client persistent storage (v1).
--
-- A single SQLite database holds:
--   * client-side settings (the configuration screen's fields),
--   * the authenticated session against the third-eye server (JWT + refresh
--     cookie jar),
--   * local bookkeeping for ROV-resident media (the list source of truth is
--     the camera; this is our local index),
--   * per-capture enrichment with the ROV telemetry snapshot taken at the
--     moment of capture,
--   * a durable retry queue for any outbound REST call that should survive
--     an app restart.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE auth_session (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    server_base   TEXT NOT NULL,
    email         TEXT,
    user_id       TEXT,
    access_token  TEXT,
    access_exp_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

-- Persisted cookie jar so the HttpOnly refresh cookie survives restarts.
CREATE TABLE http_cookies (
    domain     TEXT NOT NULL,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    value      TEXT NOT NULL,
    expires_ms INTEGER,
    secure     INTEGER NOT NULL DEFAULT 0,
    http_only  INTEGER NOT NULL DEFAULT 0,
    same_site  TEXT,
    PRIMARY KEY (domain, path, name)
);

-- ROV-only media registry; the ROV camera is source of truth for the list.
CREATE TABLE media_sync (
    media_id       TEXT NOT NULL,
    name           TEXT NOT NULL,
    size_bytes     INTEGER NOT NULL,
    duration_s     INTEGER,
    width          INTEGER,
    height         INTEGER,
    mime           TEXT,
    scene          INTEGER,
    first_seen_ms  INTEGER NOT NULL,
    last_seen_ms   INTEGER NOT NULL,
    local_path     TEXT,
    local_sha256   TEXT,
    rov_stat       INTEGER,
    deleted_on_rov INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (media_id, name)
);

CREATE INDEX media_sync_deleted_idx ON media_sync(deleted_on_rov);
CREATE INDEX media_sync_last_seen_idx ON media_sync(last_seen_ms);

-- Per-capture ROV telemetry snapshot, attached as metadata to the picture
-- produced by a POST /v1/capture call we made locally.
CREATE TABLE capture_metadata (
    media_id       TEXT NOT NULL,
    name           TEXT NOT NULL,
    captured_at_ms INTEGER NOT NULL,
    pitch          REAL,
    roll           REAL,
    yaw            REAL,
    depth_m        REAL,
    temperature_c  REAL,
    lat_e7         INTEGER,
    lon_e7         INTEGER,
    batteries_json TEXT,
    imu_json       TEXT,
    note           TEXT,
    tags_json      TEXT,
    PRIMARY KEY (media_id, name),
    FOREIGN KEY (media_id, name)
        REFERENCES media_sync(media_id, name) ON DELETE CASCADE
);

-- Freeform user annotation events (many per media, or unattached).
CREATE TABLE user_status_events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms         INTEGER NOT NULL,
    media_id      TEXT,
    name          TEXT,
    pitch         REAL,
    roll          REAL,
    yaw           REAL,
    depth_m       REAL,
    temperature_c REAL,
    lat_e7        INTEGER,
    lon_e7        INTEGER,
    note          TEXT,
    tags_json     TEXT
);

CREATE INDEX user_status_events_ts_idx ON user_status_events(ts_ms);
CREATE INDEX user_status_events_media_idx ON user_status_events(media_id, name);

-- Durable outbound REST queue. Commit = accepted by the app.
CREATE TABLE rest_outbox (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    method           TEXT NOT NULL,
    endpoint         TEXT NOT NULL,
    body_json        TEXT,
    headers_json     TEXT,
    created_at_ms    INTEGER NOT NULL,
    next_retry_at_ms INTEGER NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT,
    idempotency_key  TEXT UNIQUE
);

CREATE INDEX rest_outbox_due_idx ON rest_outbox(next_retry_at_ms);
