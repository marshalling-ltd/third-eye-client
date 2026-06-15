# third-eye-client — Review Queue

This is a desktop app that must run on **macOS, Windows, and Linux**. The items
below are cross-platform logic issues / gaps spotted while expanding the unit
tests for `camera`, `nmea`, and `network`. Severity is a rough triage guide.

## Cross-platform correctness

- [ ] **(High) GPS parsers can panic on non-ASCII byte boundaries.**
  - The NMEA/TAIP parsers slice `&str` by byte indices derived from
    `find('.')` / fixed offsets. A line that is valid UTF-8 but contains a
    multi-byte char can make a slice land off a char boundary and **panic**,
    which kills the GPS worker thread (TCP listener, TCP client, or serial).
    The UI then only sees a generic "connection ended".
  - Locations: `src/nmea.rs:882` and `:883` (`value[..degree_digits]` /
    `value[degree_digits..]`), `src/nmea.rs:935`–`:936`
    (`data[5..13]` / `data[13..22]`), `src/nmea.rs:963`–`:964`
    (`s[1..=degree_digits]` / `s[1 + degree_digits..]`).
  - Fix: parse via `.as_bytes()` with explicit ASCII checks, or guard ranges
    with `str::is_char_boundary` / `str::get(..)`, and reject non-ASCII input
    before slicing.

- [ ] **(Medium) Hardcoded `en0 == Wi-Fi` assumption (macOS).**
  - `detect_rov_interface` excludes `en0` to prefer a wired adapter, and the
    Bluetooth filters special-case device names. On Macs where `en0` is wired
    (desktops, Thunderbolt docks, Hackintosh), this misclassifies interfaces.
  - Locations: `src/network.rs:57`–`:69`; `src/nmea.rs:99`–`:130`.
  - Fix: classify by media type / interface flags instead of name, or make the
    excluded interface configurable.

- [ ] **(Medium) Non-macOS interface detection may pick Wi-Fi.**
  - On Linux/Windows `detect_rov_interface` returns the first subnet match with
    no wired-preference, so the ROV route can be bound to a Wi-Fi adapter.
  - Location: `src/network.rs:71`–`:72`.
  - Fix: add a per-platform wired-preference heuristic (mirror the macOS path).

- [ ] **(Medium) ROV interface binding is a silent no-op on Windows.**
  - `CameraApiClient::new_bound` only applies `.interface()` under
    `#[cfg(unix)]`; on Windows the chosen NIC is ignored and OS routing is used,
    so multi-NIC ROV setups may reach the camera via the wrong interface.
  - Location: `src/camera.rs:303`–`:314`.
  - Fix: implement Windows binding (e.g. bind to the chosen adapter's local
    address) or surface the limitation in the UI.

## Platform feature gaps

- [ ] **(Low) Guided Bluetooth pairing is macOS-only.**
  - `prepare_bluetooth` shells out to `blueutil` / `system_profiler` / `open`
    on macOS; Linux and Windows just return an informational string. On Linux,
    `/dev/rfcomm*` only appears after a manual `rfcomm bind`.
  - Locations: `src/nmea.rs:266`–`:328`; Linux port filter `src/nmea.rs:110`–`:111`.
  - Fix: add Linux (`bluetoothctl`/`rfcomm`) and Windows pairing flows, or
    in-UI guidance for those platforms.

- [ ] **(Low) macOS external CLI dependencies assumed present.**
  - `blueutil`, `ifconfig`, `system_profiler`, and `open` are invoked directly;
    if absent, features degrade with limited feedback.
  - Locations: `src/nmea.rs:438`–`:495`; `src/network.rs:76`–`:83`.
  - Fix: detect-and-report missing tools (blueutil bundling is already partly
    handled).

## Testing follow-ups

- [ ] **Serial/Bluetooth worker (`nmea_serial_worker`) is not unit-tested.**
  - It requires a real serial device, so it stays uncovered. Consider extracting
    the read/parse loop behind a `Read`-trait seam so it can be driven by an
    in-memory pipe in tests (covers reconnect/parse paths on all platforms).
  - Location: `src/nmea.rs:692`–`:789`.
