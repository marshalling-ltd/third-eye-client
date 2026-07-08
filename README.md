[![codecov](https://codecov.io/gh/marshalling-ltd/third-eye-client/graph/badge.svg?token=W0Ys8TfmQA)](https://codecov.io/gh/marshalling-ltd/third-eye-client)![CI](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/ci.yml/badge.svg)![Release](https://github.com/marshalling-ltd/third-eye-client/actions/workflows/release.yml/badge.svg)

# third-eye-client

Cross-platform desktop client for controlling and interacting with Chasing underwater ROVs. Built with Rust and [Slint](https://slint.dev/) for a native GUI on macOS, Windows, and Linux.

> **New here?** Read the **[Operating Guide](OPERATIONS.md)** first — a plain-English, step-by-step manual for connecting the Chasing M2S, streaming video, and getting GPS on the map.

## Features

### Live Video Stream
- Real-time RTSP video from the ROV camera decoded via a bundled ffmpeg
- Full-screen stream view with a heads-up telemetry overlay (depth, temperature, heading, attitude, GPS coordinates, battery)
- Auto-starts telemetry and stream when navigating to the Stream screen

### ROV Telemetry
- Receives ROV status broadcasts over UDP (configurable port, default 8500)
- Displays attitude (pitch / roll / yaw), depth, water temperature, IMU gyroscope, battery levels, and GPS coordinates
- Binds the UDP socket to a specific network interface when configured

### Photo & Video Capture
- Triggers the ROV camera shutter remotely (JPEG, DNG, or JPEG+DNG; burst 1–5)
- Automatically attaches a telemetry snapshot (depth, attitude, coordinates, battery state) to each capture as metadata
- Refreshes the device GPS fix before every capture so coordinates are as fresh as possible

### Media Library
- Syncs the ROV's on-device file list into a local SQLite registry
- Browse, download, preview (images with thumbnails), and stream (video via ffmpeg) any file on the ROV
- Delete media from the ROV directly from the UI
- Tracks local download state, SHA-256 checksums, and capture metadata per file
- Auto-downloads image previews on selection

### Interactive Map
- Slippy-map viewer backed by OpenStreetMap tiles with zoom (3–19), pan, and a location pin
- Scale bar and coordinate readout
- Animated viewport transitions when re-centering

### GPS / Location Detection
- **macOS**: native CoreLocation (non-blocking; permission prompt on first launch)
- **Windows**: Windows Location Services (background thread with 30 s timeout)
- **External GPS (NMEA)**: reads standard NMEA-0183 sentences from any GPS source in three modes:
  - **TCP Listen** – the app listens on a TCP port; a phone app or network GPS device connects as a client (GPS2IP, GPSd Forwarder, ShareGPS)
  - **TCP Client** – the app dials an NMEA server running on a phone or remote host
  - **Serial / Bluetooth** – reads from any serial port (`/dev/cu.*`, `/dev/rfcomm*`, `COM*`), covering standalone Bluetooth GPS receivers, USB GPS dongles, and phone GPS apps that expose an SPP channel
- Stale-timeout configurable per session; the map auto-centers on the latest fix

### ROV Network Interface Binding
- Auto-detects the wired USB-ethernet adapter on the same subnet as the ROV
- Binds HTTP, UDP, and (on Unix) socket-level traffic to that interface via `IP_BOUND_IF` / `SO_BINDTODEVICE`
- Sets up an OS-level host route so ffmpeg (an external process that can't use `IP_BOUND_IF`) reaches the ROV through the correct adapter
  - macOS: `osascript` with administrator privileges (one-time password prompt)
  - Windows: ARP cache pre-population via an HTTP probe before launching ffmpeg

### Server Authentication
- Sign in / out against the third-eye backend (`POST /api/v1/account/login`, refresh-token cookie)
- JWT access-token with automatic expiry tracking
- Persistent cookie jar stored in SQLite so sessions survive restarts

### Persistent Storage
- Single SQLite database (via `rusqlite`) stores configuration, auth sessions, media sync state, capture metadata, and a durable REST outbox
- Background outbox worker retries failed server requests with exponential backoff

### Build Targets
- **macOS** – universal binary (arm64 + x86_64) `.app` bundle with ad-hoc code signing (`scripts/build_macos_app.sh`)
- **Windows** – cross-compiled from macOS via MinGW, packaged as a zip (`scripts/build_windows.sh`)
- **Linux** – native build packaged as an AppImage (`scripts/build_linux.sh`, must run on Linux)
## Release checklist

1. Merge or cherry-pick only release-ready commits into the `release` branch.
2. Bump app version (patch bump helper):
   - `make bump-patch`
3. Commit and push the version bump to `release`.
4. Wait for the `Release` workflow to finish all three platform builds on `release`.
   - This run refreshes the rolling `latest` **pre-release** with all three installers.
5. Verify the `latest` pre-release assets (macOS DMG, Windows installer, Linux AppImage).
6. Create and push a semantic version tag that matches `Cargo.toml` exactly:
   - `git tag vX.Y.Z`
   - `git push origin vX.Y.Z`
7. Confirm the `publish` job creates:
   - the tagged GitHub Release (`vX.Y.Z`) with all three artifacts, and
   - the refreshed `latest` prerelease mirror with the same artifacts.
8. Smoke-test updater flow in the app:
   - Restart app (or click **Check for updates** in Configuration).
   - Confirm it detects the new tag and opens the correct platform download when **Download update** is clicked.

## Network Setup (USB Ethernet to ROV)

The ROV communicates over a local ethernet link. UDP discovery uses broadcast,
but RTSP/TCP require proper L2 (ARP) reachability between your machine and
the ROV.

### Prerequisites

- USB 10/100 ethernet adapter connected to the ROV
- ROV default IP: `192.168.1.88`
- Required client IP: `192.168.1.103` (the ROV expects its client at this address)
- ROV MAC address: find it via Wireshark on the USB adapter or from the ROV
  documentation (e.g. `32:d7:c8:a8:ed:6a`)

### 1. Set a static IP on the USB adapter

**macOS (GUI):**
System Settings → Network → USB 10/100 LAN → Details → TCP/IP → Configure IPv4: **Manually**
- IP Address: `192.168.1.103`
- Subnet Mask: `255.255.255.0`
- Router: *(leave blank)*

**macOS (CLI):**
```sh
# Find your USB adapter name (e.g. en10)
ifconfig | grep -B2 "status: active"

# Set the static IP (replace en10 with your adapter name)
sudo ifconfig en10 inet 192.168.1.103 netmask 255.255.255.0
```

### 2. Configure the ROV network interface in the app

In the **Configuration** screen, set **ROV network interface** to your USB
adapter name (e.g. `en10`). Find it with `ifconfig | grep -B2 "status: active"`.

When set, the app binds all connections to that interface at the socket level:

- **HTTP/TCP** (camera API): uses `IP_BOUND_IF` via reqwest's `interface()` method
- **UDP** (telemetry): uses `IP_BOUND_IF` via `socket2::bind_device_by_index_v4()`
- **RTSP** (video stream via ffmpeg): ffmpeg is an external process and can't use
  `IP_BOUND_IF` directly. The app automatically sets up an OS-level host route
  before launching ffmpeg. On macOS this triggers a **one-time admin password
  prompt** (via `osascript`). The route persists for the session.

Leave the field empty to use default OS routing (no interface binding).

### Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| UDP works but no TCP/RTSP | ARP not resolving — ROV can't find client | Verify static IP is `192.168.1.103` |
| ARP requests visible in Wireshark but no replies | Wrong IP on USB adapter | Set IP to `192.168.1.103` |
| HTTP works but RTSP doesn't | Admin password not entered for route setup | Restart the stream, enter password when prompted |
| Works on hotspot but not home WiFi | Subnet conflict — set the interface in the app | Enter adapter name in Configuration screen |

### Verifying connectivity

```sh
# Check ARP resolves (should show ROV's real MAC, not adapter MAC)
arp -an | grep 192.168.1.88

# Test HTTP API
nc -vz -w 3 192.168.1.88 80

# Test RTSP
nc -vz -w 3 192.168.1.88 8554
```
