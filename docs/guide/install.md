# Install

Grab the latest build from the [Releases page](https://github.com/marshalling-ltd/third-eye-client/releases).

| Platform | Artifact | Notes |
|---|---|---|
| macOS | `.dmg` | Universal app (arm64 + x86_64), ad-hoc signed, ffmpeg bundled per architecture |
| Windows | Installer (NSIS) | Built natively with MSVC, `ffmpeg.exe` included |
| Linux | `.AppImage` | ffmpeg included |

!!! note
    Releases tagged `vX.Y.Z` are the stable channel. The rolling **`latest`** pre-release mirrors the most recent build of the `release` branch.

## What you need before you start

- A laptop running **macOS**, **Windows** or **Linux**.
- Your **Chasing M2S ROV** and its cable reel / controller box.
- **One of these** to connect the laptop to the ROV (either works on any operating system):
    - the ROV's **Wi-Fi** (easiest), or
    - a **USB-to-Ethernet adapter** and the ROV's network cable.
- *Optional, for GPS on the map:* your **phone** with a free GPS app, or a Bluetooth or USB GPS device.

## Updates

The app checks the GitHub releases for a newer version on startup, or when you click **Check for updates** in **Profile** → **Application updates**. **Download update** opens the correct installer for your platform.

Next: [Connect to the ROV](connect.md).
