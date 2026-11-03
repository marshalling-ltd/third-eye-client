# Supported ROVs

| ROV | Status |
|---|---|
| **Chasing M2S** | Supported. Tested with a real ROV on macOS; Linux and Windows are tested against the simulator only so far. |
| Other Chasing models and firmware | Expected to work, not verified. |
| Other makes | Wanted. [Open an issue](#request-support-for-your-rov) and we will support it. |

Reports of what works and what does not, on any ROV or operating system, are welcome as [issues](https://github.com/marshalling-ltd/third-eye-client/issues).

## What the app needs from a ROV

The app talks to the ROV over three links. Addresses are the Chasing defaults and can be changed per device.

| Link | Default | What it carries |
|---|---|---|
| **Video** | RTSP, port `8554` | The live camera stream, read with ffmpeg |
| **Telemetry** | UDP, port `8500` | Status packets: pitch, roll, yaw, depth, temperature, batteries, IMU, position |
| **Camera API** | HTTP, port `80` | `/v1/capture`, `/v1/lamp`, and `/v1/medias` (list, info, download, delete) |

The ROV is expected at `192.168.1.88`, reached over its Wi-Fi or a wired link. See [how the app finds the ROV](../explanation/finding-the-rov.md).

A status packet is a 12-byte header (an id byte, a payload length and a type byte) followed by a JSON payload. The expected id and type are device settings (3 and 1 for Chasing).

## Simulator

For development without hardware there is a [ROV simulator](../development/simulator.md), `chasing-simulator`, that stands in for a Chasing ROV. It serves synthetic telemetry, loops a sample video over RTSP, and implements the camera API, including photo capture and media download.

It is not a real ROV. It does not reproduce real camera timing, multi-camera errors or real raw photo files, so it cannot prove that everything works on a real ROV.

## Request support for your ROV

Open a [GitHub issue](https://github.com/marshalling-ltd/third-eye-client/issues/new) and we will work on supporting your ROV. We only own a Chasing ROV, so the work goes in this order:

1. We build a **simulator** for your ROV from its documentation.
2. We add support in the app and test it against that simulator.
3. You, or anyone with the real ROV, try it and report what works and what does not.

The documentation is what makes step 1 possible, so please put it in the issue:

- the **make, model and firmware version**,
- how the **video** is served (for example an RTSP address and port),
- how **telemetry** is sent (protocol, port and packet format),
- how the **camera** is controlled (for example the HTTP API: capture, lamp, listing and downloading photos and videos),
- links to the manufacturer's manuals or API documents.

If the maker has not published documentation, say so in the issue and we will see what is possible. Without any description of how the ROV talks, we cannot build a simulator, and so we cannot start.

## Adding another ROV

The app is not yet built around a plug-in interface for ROVs: the code that talks to the ROV is written for Chasing. Supporting another make means working in these places:

| Area | Where |
|---|---|
| Camera API client (capture, lamp, media) | `src/camera.rs` |
| Telemetry packets | `src/rov_status.rs` |
| Video stream setup | `src/main.rs` (stream pipeline) |
| Device settings shown in the app | `src/main.rs` and `ui/pages/devices/` |
| A simulator for the new ROV | `src/simulator/` |

If you would rather write the support yourself, open an [issue](https://github.com/marshalling-ltd/third-eye-client/issues/new) first (see above) so the work can be shaped before code is written. Then follow [Contributing](../development/contributing.md).
