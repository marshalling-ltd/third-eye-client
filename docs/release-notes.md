# Release notes

Releases are published on the [Releases page](https://github.com/marshalling-ltd/third-eye-client/releases). See the [release process](development/release.md) for how a version is cut.

## Unreleased

Changes since `v0.1.12`.

### Connecting to the ROV

- The app detects automatically whether the ROV is reached over Wi-Fi or a USB-Ethernet cable, and follows the link when it changes.
- Video goes through an interface-bound forwarder on every OS. **No administrator password prompt, host route or ARP entry is needed any more.**
- When a USB-Ethernet adapter has no address on the ROV's network, the app shows the exact steps for your OS.

### GPS

- Paired Bluetooth GPS receivers are listed as `bt://…` on macOS and Linux and connect without extra setup (`rfcomm bind` is not needed).
- USB GPS receivers are listed alongside them, with a hint when a port sends nothing useful.
- The Geoastra X8-M1 should be used over Bluetooth on macOS.

### Tooling

- New `chasing-simulator` CLI that stands in for a ROV, and `forwarder-probe` for measuring the forwarder.
- Documentation moved to this site.
