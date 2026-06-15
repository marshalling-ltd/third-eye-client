# third-eye-client — Review Queue

This is a desktop app that must run on **macOS, Windows, and Linux**. This queue
tracks cross-platform logic issues / gaps spotted while expanding the unit tests
for `camera`, `nmea`, and `network`.

## Status

_No outstanding items._ All previously tracked cross-platform issues have been
resolved and covered by unit tests:

- **macOS interface detection by media type** — a wired `en0` (desktop /
  Thunderbolt dock) is now selectable; Wi-Fi is excluded by its media type
  rather than the hardcoded `en0` name. (`src/network.rs`)
- **Wired-interface preference on Linux/Windows** — `detect_rov_interface`
  prefers a non-wireless adapter so the ROV route is not bound to Wi-Fi.
  (`src/network.rs`)
- **Windows ROV interface binding** — `CameraApiClient::new_bound` resolves the
  chosen adapter's local IPv4 and binds via `local_address`, since reqwest
  cannot bind by interface name on Windows. (`src/camera.rs`)
- **Linux/Windows Bluetooth pairing guidance** — `prepare_bluetooth` returns
  platform-specific in-UI guidance (`bluetoothctl`/`rfcomm` on Linux; Settings
  > Bluetooth + outgoing COM port on Windows). (`src/nmea.rs`)
- **Missing macOS CLI tools reported precisely** — a `system_profiler`/`open`
  availability check yields a clear "tool not found" message instead of a
  misleading failure. (`src/nmea.rs`)
- **Serial worker read loop is unit-tested** — the read/parse/dedup loop is
  extracted behind a `BufRead` seam (`pump_nmea_lines`) and driven by an
  in-memory reader in tests (fix emission, dedup, EOF/disconnect, stop flag).
  (`src/nmea.rs`)
