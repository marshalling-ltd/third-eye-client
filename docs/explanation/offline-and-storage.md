# Working offline and what is stored

The app is designed for use at sea, where the internet may not be available. The ROV link is local, so video, telemetry and capture never need internet. Only a few features reach out to the network.

## What needs internet

| Feature | Service |
|---|---|
| Map tiles | OpenStreetMap |
| Update check | GitHub releases |
| Sign-in, devices, nearby search | The third-eye backend |
| Location fallback on Linux | An IP geolocation service |

## Staying signed in

Signing in gives the app a short-lived access token and a refresh cookie, both kept in the local database, so a restart does not sign you out. The app refreshes the token before it expires and every 15 minutes while you are signed in. It signs you out only when the server **rejects** a refresh. If the network is simply unreachable, an error is shown, the session is kept and the last known devices are still used.

## Map tiles

Tiles you have viewed are cached in the local database. In **Profile** → **Offline map tiles** you can choose to use saved tiles and set a size limit (default 1 GB). The least recently used tiles are deleted first when the cache is over the limit.

## The local database

Everything lives in one SQLite file in the operating system's data directory: settings, the signed-in session, the cached device list, the mirror of the ROV's file list with download state and capture metadata, the tile cache, and a durable queue for server writes that retries with exponential backoff (up to 5 minutes). No feature adds to that queue yet; it is in place for writes that must survive a restart.

## Related

- [Architecture](../reference/architecture.md) for the table list
