<div align="center">

<img src="assets/logo.png" alt="third-eye-client logo" width="120">

# third-eye-client

**Cross-platform desktop client for Chasing underwater ROVs**

Built with Rust 🦀 and [Slint](https://slint.dev/) — native GUI on macOS, Windows and Linux.

[![codecov](https://codecov.io/gh/marshalling-ltd/third-eye-client/graph/badge.svg?token=W0Ys8TfmQA)](https://codecov.io/gh/marshalling-ltd/third-eye-client)
[![CI](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/ci.yml/badge.svg)](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/ci.yml)
[![Release](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/release.yml/badge.svg)](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/release.yml)
![Rust edition](https://img.shields.io/badge/rust-edition%202024-orange)
![License](https://img.shields.io/badge/license-GPL--3.0--only-blue)

</div>

> [!TIP]
> **New here?** Read the **[Operating Guide](OPERATIONS.md)** first — a plain-English, step-by-step manual for connecting the Chasing M2S, streaming video, and getting GPS on the map.

## 📑 Table of contents

- [✨ Features](#-features)
- [🧱 Architecture](#-architecture)
- [📥 Installing](#-installing)
- [🌐 Network setup (USB Ethernet to ROV)](#-network-setup-usb-ethernet-to-rov)
- [🛠️ Development](#️-development)
  - [Build targets](#build-targets)
  - [Testing](#testing)
  - [ROV simulator](#rov-simulator)
- [🚀 Release process](#-release-process)

---

## ✨ Features

| | Feature | Summary |
|---|---|---|
| 🎥 | [Live video](#-live-video-stream) | RTSP stream with a telemetry heads-up display |
| 📡 | [Telemetry](#-rov-telemetry) | Attitude, depth, temperature, battery, GPS over UDP |
| 📸 | [Capture](#-photo--video-capture) | Remote shutter with telemetry metadata attached |
| 🗂️ | [Media library](#️-media-library) | Sync, browse, download, preview and delete ROV files |
| 🗺️ | [Map](#️-interactive-map) | OpenStreetMap slippy map with location pin |
| 📍 | [GPS](#-gps--location-detection) | Native OS location or external NMEA sources |
| 🔌 | [Interface binding](#-rov-network-interface-binding) | Pins traffic to the USB-ethernet adapter |
| 🔐 | [Server auth](#-server-authentication) | Sign in to the third-eye backend |
| 💾 | [Storage](#-persistent-storage) | SQLite for config, sessions, media and outbox |
| 🔄 | [Updates](#-update-checker) | In-app check against GitHub releases |

### 🎥 Live video stream

- Real-time RTSP video from the ROV camera, decoded via a **bundled ffmpeg**
- Full-screen stream view with a heads-up telemetry overlay (depth, temperature, heading, attitude, GPS coordinates, battery)
- Auto-starts telemetry and stream when you navigate to the Stream screen

### 📡 ROV telemetry

- Receives ROV status broadcasts over UDP (configurable port, default `8500`)
- Displays attitude (pitch / roll / yaw), depth, water temperature, IMU gyroscope, battery levels and GPS coordinates
- Binds the UDP socket to a specific network interface when configured

### 📸 Photo & video capture

- Triggers the ROV camera shutter remotely (JPEG, DNG, or JPEG+DNG; burst 1–5)
- Automatically attaches a telemetry snapshot (depth, attitude, coordinates, battery state) to each capture as metadata
- Refreshes the device GPS fix before every capture so coordinates are as fresh as possible

### 🗂️ Media library

- Syncs the ROV's on-device file list into a local SQLite registry
- Browse, download, preview (images with thumbnails) and stream (video via ffmpeg) any file on the ROV
- Delete media from the ROV directly from the UI
- Tracks local download state, SHA-256 checksums and capture metadata per file
- Auto-downloads image previews on selection
- Opens the local media directory in the OS-native file explorer

### 🗺️ Interactive map

- Slippy-map viewer backed by OpenStreetMap tiles with zoom (3–19), pan and a location pin
- Mouse-wheel and trackpad gesture support
- Scale bar and coordinate readout
- Animated viewport transitions when re-centering
- Tiles are cached locally in the SQLite database

### 📍 GPS / location detection

| Source | How it works |
|---|---|
| 🍎 **macOS** | Native CoreLocation (non-blocking; permission prompt on first launch) |
| 🪟 **Windows** | Windows Location Services (background thread with 30 s timeout) |
| 📶 **External GPS (NMEA)** | Reads standard NMEA-0183 sentences from any GPS source — three modes below |

**NMEA modes**

| Mode | Description | Typical sources |
|---|---|---|
| **TCP Listen** | The app listens on a TCP port; a phone app or network GPS device connects as a client | GPS2IP, GPSd Forwarder, ShareGPS |
| **TCP Client** | The app dials an NMEA server running on a phone or remote host | Phone / remote host |
| **Serial / Bluetooth** | Reads from any serial port (`/dev/cu.*`, `/dev/rfcomm*`, `COM*`) | Bluetooth GPS receivers, USB GPS dongles, phone apps exposing an SPP channel |

The stale-timeout is configurable per session; the map auto-centers on the latest fix.

### 🔌 ROV network interface binding

- Auto-detects the wired USB-ethernet adapter on the same subnet as the ROV
- Binds HTTP, UDP and (on Unix) socket-level traffic to that interface via `IP_BOUND_IF` / `SO_BINDTODEVICE`
- Sets up an OS-level host route so ffmpeg (an external process that can't use `IP_BOUND_IF`) reaches the ROV through the correct adapter:
  - **macOS** — `osascript` with administrator privileges (one-time password prompt)
  - **Windows** — ARP cache pre-population via an HTTP probe before launching ffmpeg

### 🔐 Server authentication

- Sign in / out against the third-eye backend (`POST /api/v1/account/login`, refresh-token cookie)
- JWT access token with automatic expiry tracking
- Persistent cookie jar stored in SQLite so sessions survive restarts
- Typed API client generated from the backend's OpenAPI spec (`make open-api`, output in `generated/`)

### 💾 Persistent storage

- Single SQLite database (via `rusqlite`, versioned with `rusqlite_migration`) stores configuration, auth sessions, media sync state, capture metadata, map tile cache and a durable REST outbox
- Background outbox worker retries failed server requests with exponential backoff

### 🔄 Update checker

- Checks the GitHub releases for a newer semantic version (on startup, or via **Check for updates** in Configuration)
- **Download update** opens the correct installer for your platform

---

## 🧱 Architecture

A native desktop app: one Rust binary with a [Slint](https://slint.dev/) UI, no embedded server. It talks to three kinds of peers — the ROV on the local link, the
[third-eye](https://github.com/marshalling-ltd/third-eye-platform) backend, and a few public services — and keeps everything it needs offline in a single SQLite file.

| Peer | Protocol | Used for |
|---|---|---|
| **ROV** (`192.168.1.88`) | UDP `8500`, RTSP `8554`, HTTP `80` | Telemetry, live video, capture and media files |
| **third-eye backend** (`https://third-eye.marshalling.eu`) | HTTPS REST | Sign-in, devices, nearby AOI / POI search |
| **GPS source** | CoreLocation, Windows Location, NMEA over TCP / serial | Device position |
| **OpenStreetMap, GitHub, IP geolocation** | HTTPS | Map tiles, update check, Linux location fallback |

```mermaid
flowchart LR
    subgraph App[third-eye-client process]
        UI[Slint UI<br/><code>ui/</code>]
        MAIN[main.rs<br/>state + callbacks<br/>16 ms poll timer]
        W[Worker threads<br/>UDP, NMEA, tiles,<br/>downloads, server calls]
        ST[(SQLite<br/><code>state.db</code>)]
        OB[Outbox worker]
    end

    FF[ffmpeg<br/>child process]
    ROV[Chasing ROV]
    API[third-eye backend]
    EXT[OSM · GitHub · GPS]

    UI <-->|properties, callbacks| MAIN
    MAIN <-->|mpsc channels| W
    MAIN <--> ST
    ST --> OB
    OB -->|retry with backoff| API
    W -->|UDP telemetry| ROV
    W -->|HTTP camera API| ROV
    W -->|REST| API
    W --> EXT
    MAIN -->|spawns| FF
    FF -->|RTSP in, MJPEG out| ROV
    FF -->|frames on stdout| W
```

**Threading model.** Slint is single-threaded, so all UI state lives in one `ThirdEyeState` on the main thread. Anything slow (UDP receive, NMEA, tile fetches, media
downloads, server calls, the ffmpeg pipe) runs on its own thread and reports back over an `mpsc` channel. A 16 ms Slint timer drains those channels and pushes the results
into the UI.

**Live video.** ffmpeg is launched as a child process that pulls RTSP over TCP and writes MJPEG frames to stdout; a reader thread splits the stream into JPEGs and hands them
to the UI. Because ffmpeg can't bind to a network interface itself, the app first installs an OS-level host route to the ROV (see [Network setup](#-network-setup-usb-ethernet-to-rov)).

**Server session.** Every backend call goes through `ApiSession`, which refreshes the access token from the persisted refresh cookie before it expires, and once more on a
401/403. Only a rejected refresh signs the user out; transport failures don't, so the app stays usable offshore without internet.

**Talking to the platform.** Everything below goes through `ApiSession`; the UI thread never blocks on the network.

```mermaid
sequenceDiagram
    autonumber
    participant UI as UI thread
    participant W as Worker thread
    participant S as ApiSession / AuthClient
    participant DB as SQLite
    participant P as third-eye platform

    Note over UI,P: Sign in
    UI->>W: Sign in (email, password)
    W->>S: login
    S->>P: POST /api/v1/account/login
    P-->>S: access token + HttpOnly refresh cookie
    S->>DB: save auth_session + http_cookies

    Note over UI,P: Authenticated call (devices, nearby search)
    UI->>W: refresh devices / open Device Map
    W->>S: call(endpoint)
    opt token expired or unknown
        S->>P: POST /api/v1/account/refresh-access-token (cookie)
        P-->>S: new access token + rotated cookie
        S->>DB: persist both
    end
    S->>P: GET /api/v1/devices, GET /api/v1/profile/info, POST /api/v1/search
    alt 401 / 403
        S->>P: refresh once, retry once
    end
    P-->>S: response
    S->>DB: cache devices (devices_cache)
    S-->>W: result
    W-->>UI: mpsc event, applied on next 16 ms tick

    Note over UI,P: Keepalive and failure handling
    loop every 15 min while signed in
        W->>S: refresh
        S->>P: POST /api/v1/account/refresh-access-token
    end
    alt refresh rejected
        S->>DB: clear session
        S-->>UI: SessionExpired, back to sign-in form
    else network unreachable (offshore)
        S-->>UI: error shown, session kept, cached devices still used
    end
```

| Area | Endpoints | Notes |
|---|---|---|
| Account | `POST /api/v1/account/login`, `POST /api/v1/account/refresh-access-token`, `GET /api/v1/account/logout` | Refresh token lives only in the `HttpOnly` cookie |
| Devices | `GET` / `POST /api/v1/devices`, `GET` / `PATCH` / `DELETE /api/v1/devices/{id}`, `GET /api/v1/profile/info` | Typed client from `generated/`; edits are optimistic-locked via `concurrency` |
| Nearby | `POST /api/v1/search` (`aoi`, `poi`, `intermagnet_analysis`) | Hand-written client; re-fetched while the Device Map is open |

The `rest_outbox` table and its retry worker are in place for writes that must survive a crash or restart, but no feature enqueues into it yet.

**Storage.** One SQLite database in the OS data directory (WAL mode, embedded migrations) holds:

| Table | Contents |
|---|---|
| `settings` | Configuration key/value pairs |
| `auth_session`, `http_cookies` | Signed-in user and the persistent cookie jar |
| `devices_cache` | Last known devices, for offline use |
| `media_sync`, `capture_metadata` | Mirror of the ROV file list, download state, per-capture telemetry |
| `map_tile_cache` | OSM tiles as PNG blobs, evicted least-recently-used |
| `rest_outbox` | Durable queue of server writes, replayed with exponential backoff (max 5 min) |

Project layout:

| Path | Contents |
|---|---|
| `src/main.rs` | App entry point: state, UI bindings, callbacks, stream pipeline, ROV route setup |
| `src/camera.rs` | ROV camera HTTP client (capture, lamp, media list / download / delete) |
| `src/rov_status.rs` | UDP status receiver and packet decoding |
| `src/nmea.rs` | NMEA-0183 GPS over TCP listen, TCP client and serial / Bluetooth |
| `src/map.rs` | Slippy-map viewport, tile loading, native location (CoreLocation / Windows) |
| `src/network.rs` | ROV interface detection and recalibration |
| `src/ip_location.rs`, `src/update_check.rs`, `src/formatting.rs` | IP geolocation fallback, release-version selection, display helpers |
| `src/storage/` | `AppStore` facade: config, auth, API session, devices, media, search, tile cache, outbox, migrations |
| `src/simulator/`, `src/bin/` | ROV simulator and test UDP server (feature `test-tools`) |
| `ui/` | Slint UI: `app.slint` window, `shell/` top bar, `pages/` stream, map, media, devices, profile |
| `generated/` | Backend API client generated from the OpenAPI spec (`make open-api`) |
| `specs/`, `tests/` | Feature specs and integration tests |
| `scripts/`, `installer/`, `macos/` | Per-platform packaging |

---

## 📥 Installing

Grab the latest build from the [Releases page](https://github.com/marshalling-ltd/third-eye-client/releases).

| Platform | Artifact | Notes |
|---|---|---|
| 🍎 macOS | `.dmg` | Universal app (arm64 + x86_64), ad-hoc signed, ffmpeg bundled per architecture |
| 🪟 Windows | Installer (NSIS) | Built natively with MSVC, `ffmpeg.exe` included |
| 🐧 Linux | `.AppImage` | ffmpeg included |

> [!NOTE]
> Releases tagged `vX.Y.Z` are the stable channel. The rolling **`latest`** pre-release mirrors the most recent build of the `release` branch.

---

## 🌐 Network setup (USB Ethernet to ROV)

The ROV communicates over a local ethernet link. UDP discovery uses broadcast, but RTSP/TCP require proper L2 (ARP) reachability between your machine and the ROV.

> [!TIP]
> Don't have a USB adapter? The ROV's Wi-Fi works too — see the [Operating Guide](OPERATIONS.md).

### Prerequisites

- USB 10/100 ethernet adapter connected to the ROV
- ROV default IP: `192.168.1.88`
- Required client IP: `192.168.1.103` (the ROV expects its client at this address)
- ROV MAC address: find it via Wireshark on the USB adapter or from the ROV documentation (e.g. `32:d7:c8:a8:ed:6a`)

### 1. Set a static IP on the USB adapter

<details open>
<summary><b>macOS (GUI)</b></summary>

System Settings → Network → USB 10/100 LAN → Details → TCP/IP → Configure IPv4: **Manually**

- IP Address: `192.168.1.103`
- Subnet Mask: `255.255.255.0`
- Router: *(leave blank)*

</details>

<details>
<summary><b>macOS (CLI)</b></summary>

```sh
# Find your USB adapter name (e.g. en10)
ifconfig | grep -B2 "status: active"

# Set the static IP (replace en10 with your adapter name)
sudo ifconfig en10 inet 192.168.1.103 netmask 255.255.255.0
```

</details>

### 2. Configure the ROV network interface in the app

In the **Configuration** screen, set **ROV network interface** to your USB adapter name (e.g. `en10`). Find it with `ifconfig | grep -B2 "status: active"`.

When set, the app binds all connections to that interface at the socket level:

| Traffic | Mechanism |
|---|---|
| **HTTP/TCP** (camera API) | `IP_BOUND_IF` via reqwest's `interface()` method |
| **UDP** (telemetry) | `IP_BOUND_IF` via `socket2::bind_device_by_index_v4()` |
| **RTSP** (video via ffmpeg) | ffmpeg is an external process and can't use `IP_BOUND_IF`, so the app sets up an OS-level host route before launching it. On macOS this triggers a **one-time admin password prompt** (via `osascript`); the route persists for the session. |

Leave the field empty to use default OS routing (no interface binding).

### Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| UDP works but no TCP/RTSP | ARP not resolving — ROV can't find client | Verify static IP is `192.168.1.103` |
| ARP requests visible in Wireshark but no replies | Wrong IP on USB adapter | Set IP to `192.168.1.103` |
| HTTP works but RTSP doesn't | Admin password not entered for route setup | Restart the stream, enter password when prompted |
| Works on hotspot but not home Wi-Fi | Subnet conflict — set the interface in the app | Enter adapter name in Configuration screen |

### Verifying connectivity

```sh
# Check ARP resolves (should show ROV's real MAC, not adapter MAC)
arp -an | grep 192.168.1.88

# Test HTTP API
nc -vz -w 3 192.168.1.88 80

# Test RTSP
nc -vz -w 3 192.168.1.88 8554
```

---

## 🛠️ Development

One-time tool setup: `make requirements`. Everyday commands:

| Command | What it does |
|---|---|
| `make code` | Update deps, audit, deny, typos, fmt, clippy (pedantic) |
| `make check` | `code` + full `nextest` run |
| `make test` / `make nextest` | Run the test suite (see [Testing](#testing)) |
| `make coverage` | Write `lcov.info` |
| `make open-api` | Regenerate the backend API client in `generated/` |
| `make bump-patch` | Bump the patch version in `Cargo.toml` |

### Build targets

| Platform | How | Output |
|---|---|---|
| 🍎 macOS | `scripts/build_macos_app.sh` | Universal (arm64 + x86_64) `.app` bundle, ad-hoc code signed |
| 🪟 Windows | `scripts/build_windows.sh` (cross-compiled from macOS via MinGW) | Zip package |
| 🐧 Linux | `scripts/build_linux.sh` (must run on Linux) | AppImage |

> [!NOTE]
> Official release installers (DMG, NSIS installer, AppImage) are built by the [`Release` workflow](.github/workflows/release.yml), which also bundles ffmpeg. On macOS each architecture gets its own self-contained ffmpeg + dylibs directory; the x86_64 one is built on a native Intel runner (`macos-15-intel`).

### Testing

| Use case | Command | Why |
|---|---|---|
| **Local iteration** | `cargo test` | Runs the whole suite in one process, so the ~2–4 s per-process loader overhead of this large, statically-linked binary is paid once instead of once per test |
| **CI / pre-push gate** | `make nextest` | Isolates each test in its own process — slower, but authoritative |
| **Coverage** | `make coverage` | Writes `lcov.info` |
| **HTML coverage** | `make test-cov` / `make nextest-cov` | Opens an HTML report |

### ROV simulator

`chasing-simulator` is a CLI that stands in for a real ROV so the client can be exercised without hardware. It runs three servers at once:

| Server | Endpoint | Details |
|---|---|---|
| 📡 **UDP telemetry** | `127.0.0.1:8500` | Synthetic status packets (same generator as `test-udp-server`) |
| 🎥 **RTSP video** | `rtsp://admin:admin@127.0.0.1:8554/stream/0/0` | Loops the first `.mp4` in `samples/` (credentials are not checked) |
| 🌐 **Camera HTTP API** | `http://127.0.0.1:8080` | `/v1/capture`, `/v1/lamp`, `/v1/medias` (list, info, download with Range support, delete), backed by the files in `samples/` |

> [!NOTE]
> Deleting only hides a file for the session; nothing is removed from disk.

**Prerequisites:** `ffmpeg` and [`mediamtx`](https://github.com/bluenviron/mediamtx) on `PATH` (mediamtx is the RTSP server, since ffmpeg cannot serve RTSP on its own).

```sh
brew install ffmpeg mediamtx
```

**Run it:**

```sh
cargo run --features test-tools --bin chasing-simulator
```

Then point the client at `rtsp://admin:admin@127.0.0.1:8554/stream/0/0` and `http://127.0.0.1:8080`. Stop with <kbd>Ctrl</kbd>+<kbd>C</kbd>.

<details>
<summary><b>Options</b> (see <code>--help</code> for all)</summary>

| Option | Default | Purpose |
|--------|---------|---------|
| `--udp-target <HOST>` / `--udp-port <PORT>` | `127.0.0.1` / `8500` | Where telemetry is sent |
| `--rtsp-port <PORT>` / `--http-port <PORT>` | `8554` / `8080` | Server ports |
| `--samples <DIR>` | `samples` | Media served over HTTP |
| `--video <FILE>` | first `.mp4` in samples | Video looped over RTSP |
| `--ffmpeg <PATH>` / `--mediamtx <PATH>` | from `PATH` | Binary locations |

</details>

---

## 🚀 Release process

```mermaid
flowchart LR
    A[Cherry-pick to<br/><code>release</code>] --> B[make bump-patch]
    B --> C[Push to <code>release</code>]
    C --> D[Release workflow<br/>3 platform builds]
    D --> E[<code>latest</code><br/>pre-release]
    E --> F[Tag <code>vX.Y.Z</code>]
    F --> G[Tagged GitHub Release<br/>+ refreshed <code>latest</code>]
    G --> H[Smoke-test updater]
```

1. Merge or cherry-pick only release-ready commits into the `release` branch.
2. Bump the app version (patch bump helper): `make bump-patch`
3. Commit and push the version bump to `release`.
4. Wait for the `Release` workflow to finish all three platform builds on `release`. This run refreshes the rolling `latest` **pre-release** with all three installers.
5. Verify the `latest` pre-release assets (macOS DMG, Windows installer, Linux AppImage).
6. Create and push a semantic version tag that **matches `Cargo.toml` exactly**:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

7. Confirm the `publish` job creates:
   - the tagged GitHub Release (`vX.Y.Z`) with all three artifacts, and
   - the refreshed `latest` prerelease mirror with the same artifacts.
8. Smoke-test the updater flow in the app:
   - Restart the app (or click **Check for updates** in Configuration).
   - Confirm it detects the new tag and opens the correct platform download when **Download update** is clicked.

> [!WARNING]
> The `publish` job fails if the pushed tag does not match the version in `Cargo.toml`.
