# Building and testing

## Prerequisites

- The Rust **nightly** toolchain (the `Makefile` and CI both use it). `make requirements` installs it, plus `cargo-audit`, `cargo-deny`, `cargo-edit`, `cargo-llvm-cov`, `cargo-machete`, `cargo-nextest` and `typos-cli`.
- **Linux** only: the system packages that `scripts/build_linux.sh` installs when missing, notably `libdbus-1-dev` (Bluetooth GPS) and `libudev-dev` (serial ports), plus `pkg-config` and the Wayland / X11 / fontconfig / Mesa development headers.
- **ffmpeg** for live video. The installers bundle it; when running from source it must be available on your machine.

## Everyday commands

| Command | What it does |
|---|---|
| `make code` | Update deps, machete, audit, deny, typos, fmt, fix, clippy (pedantic), doc tests |
| `make check` | `code` + full `nextest` run |
| `make test` / `make nextest` | Run the test suite (see [Testing](building.md#testing)) |
| `make coverage` | Write `lcov.info` |
| `make open-api` | Regenerate the backend API client in `generated/` |
| `make bump-patch` | Bump the patch version in `Cargo.toml` |

## Build targets

| Platform | How | Output |
|---|---|---|
| 🍎 macOS | `scripts/build_macos_app.sh` | Universal (arm64 + x86_64) `.app` bundle, ad-hoc code signed |
| 🪟 Windows | `scripts/build_windows.sh` (cross-compiled from macOS via MinGW) | Zip package |
| 🐧 Linux | `scripts/build_linux.sh` (must run on Linux) | AppImage |

!!! note
    Official release installers (DMG, NSIS installer, AppImage) are built by the [`Release` workflow](https://github.com/marshalling-ltd/third-eye-client/blob/master/.github/workflows/release.yml), which also bundles ffmpeg. On macOS each architecture gets its own self-contained ffmpeg + dylibs directory; the x86_64 one is built on a native Intel runner (`macos-15-intel`).

## Testing

| Use case | Command | Why |
|---|---|---|
| **Local iteration** | `cargo test` | Runs the whole suite in one process, so the ~2–4 s per-process loader overhead of this large, statically-linked binary is paid once instead of once per test |
| **CI / pre-push gate** | `make nextest` | Isolates each test in its own process — slower, but authoritative |
| **Coverage** | `make coverage` | Writes `lcov.info` |
| **HTML coverage** | `make test-cov` / `make nextest-cov` | Opens an HTML report |
