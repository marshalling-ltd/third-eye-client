# Release notes

Releases are published on the [Releases page](https://github.com/marshalling-ltd/third-eye-client/releases). See the [release process](development/release.md) for how a version is cut.

## Unreleased

Changes since `v0.1.12`.

### Connecting to the ROV

- The app detects automatically whether the ROV is reached over Wi-Fi or a USB-Ethernet cable, and follows the link when it changes.
- Video goes through an interface-bound forwarder on every OS. **No administrator password prompt, host route or ARP entry is needed any more.**
- When a USB-Ethernet adapter has no address on the ROV's network, the app shows the exact steps for your OS.

### Live video

- Live video starts faster. ffmpeg now probes the stream for at most 0.5 s instead of its default several seconds. Against the simulator, time to first frame fell from about 3 s to about 1 s; the remaining wait is mostly for the next keyframe. Measured on macOS with the simulator only, not yet on a real ROV, Windows or Linux.

### GPS

- Paired Bluetooth GPS receivers are listed as `bt://…` on macOS and Linux and connect without extra setup (`rfcomm bind` is not needed).
- USB GPS receivers are listed alongside them, with a hint when a port sends nothing useful.
- Bluetooth and USB GPS support is generic for NMEA receivers and has been tested on hardware with one receiver, the Geoastra X8-M1.

### Tooling

- New `chasing-simulator` CLI that stands in for a ROV, and `forwarder-probe` for measuring the forwarder.
- Documentation moved to this site.
