-- Devices created (or edited) while signed out / offline (v4).
--
-- A row with pending_sync = 1 exists only locally: its id is a client-generated
-- UUID and the server has never seen it. On the next successful devices refresh
-- the client creates it server-side and swaps the row for the server's copy
-- (keeping the active-device flag). Server refreshes never delete these rows.

ALTER TABLE devices_cache ADD COLUMN pending_sync INTEGER NOT NULL DEFAULT 0;
