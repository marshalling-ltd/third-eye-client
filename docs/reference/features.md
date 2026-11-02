# Features

| Feature | Summary |
|---|---|
| [Live video](../guide/video-and-capture.md#live-video) | RTSP stream with a telemetry heads-up display |
| [Telemetry](../guide/video-and-capture.md#live-video) | Attitude, depth, temperature, battery, GPS over UDP |
| [Capture](../guide/video-and-capture.md#taking-photos) | Remote shutter with telemetry metadata attached |
| [Media library](../guide/video-and-capture.md#media-library) | Sync, browse, download, preview and delete ROV files |
| [Map](#interactive-map) | OpenStreetMap slippy map with location pin |
| [GPS](../guide/gps.md) | Native OS location or external NMEA sources |
| [Interface binding](#rov-network-interface-binding) | Pins traffic to the detected Wi-Fi or USB-Ethernet link |
| [Server auth](#server-authentication) | Sign in to the third-eye backend |
| [Storage](#persistent-storage) | SQLite for config, sessions, media and outbox |
| [Updates](#update-checker) | In-app check against GitHub releases |

## Interactive map

- Slippy-map viewer backed by OpenStreetMap tiles with zoom (3 to 19), pan and a location pin.
- Mouse-wheel and trackpad gesture support.
- Scale bar and coordinate readout.
- Animated viewport transitions when re-centring.
- Tiles are cached locally in the SQLite database.

## ROV network interface binding

- Auto-detects how the computer reaches the ROV (press **Recalibrate ROV network** to re-run it), whether over its Wi-Fi or a USB-Ethernet cable: the interface with an address on the ROV's subnet is chosen (a cable wins over Wi-Fi), and an unrelated link such as a phone hotspot is ignored.
- Watches for cables and Wi-Fi coming and going (every 2 s) and, within a few seconds, moves video and telemetry onto the new link.
- Binds HTTP, UDP and (on Unix) socket-level traffic to that interface via `IP_BOUND_IF` / `SO_BINDTODEVICE`.
- ffmpeg connects to a small in-process forwarder on `127.0.0.1`, which opens the real connection to the ROV from a socket pinned to the detected interface. No host route, ARP entry or administrator prompt is needed.
- If a cable is connected but the adapter has no address on the ROV's network, the app shows a one-time guide with the exact steps for your OS.

See [Connect to the ROV](../guide/connect.md) for the user-facing steps.

## Server authentication

- Sign in / out against the third-eye backend (`POST /api/v1/account/login`, refresh-token cookie).
- JWT access token with automatic expiry tracking.
- Persistent cookie jar stored in SQLite so sessions survive restarts.
- Typed API client generated from the backend's OpenAPI spec (`make open-api`, output in `generated/`).

## Persistent storage

- Single SQLite database (via `rusqlite`, versioned with `rusqlite_migration`) stores configuration, auth sessions, media sync state, capture metadata, map tile cache and a durable REST outbox.
- Background outbox worker retries failed server requests with exponential backoff.

## Update checker

- Checks the GitHub releases for a newer semantic version (on startup, or via **Check for updates** in Profile → Application updates).
- **Download update** opens the correct installer for your platform.
