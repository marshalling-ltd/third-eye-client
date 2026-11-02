# ROV simulator

`chasing-simulator` is a CLI that stands in for a real ROV so the client can be exercised without hardware. It runs three servers at once:

| Server | Endpoint | Details |
|---|---|---|
| 📡 **UDP telemetry** | `127.0.0.1:8500` | Synthetic status packets (same generator as `test-udp-server`) |
| 🎥 **RTSP video** | `rtsp://admin:admin@127.0.0.1:8554/stream/0/0` | Loops the first `.mp4` in `samples/` (credentials are not checked) |
| 🌐 **Camera HTTP API** | `http://127.0.0.1:8080` | `/v1/capture`, `/v1/lamp`, `/v1/medias` (list, info, download with Range support, delete), backed by the files in `samples/` |

!!! note
    Deleting only hides a file for the session; nothing is removed from disk.

**Prerequisites:** `ffmpeg` and [`mediamtx`](https://github.com/bluenviron/mediamtx) on `PATH` (mediamtx is the RTSP server, since ffmpeg cannot serve RTSP on its own).

```sh
brew install ffmpeg mediamtx
```

**Run it:**

```sh
cargo run --features test-tools --bin chasing-simulator
```

Then point the client at `rtsp://admin:admin@127.0.0.1:8554/stream/0/0` and `http://127.0.0.1:8080`. Stop with <kbd>Ctrl</kbd>+<kbd>C</kbd>.

??? info "Options (see `--help` for all)"

    | Option | Default | Purpose |
    |--------|---------|---------|
    | `--udp-target <HOST>` / `--udp-port <PORT>` | `127.0.0.1` / `8500` | Where telemetry is sent |
    | `--bind <ADDR>` | `0.0.0.0` | Address to act as the ROV on (e.g. `192.168.1.88`); RTSP, HTTP and the telemetry source all use it |
    | `-v`, `--verbose` | off | Log each RTSP and HTTP client with its source address |
    | `--rtsp-port <PORT>` / `--http-port <PORT>` | `8554` / `8080` | Server ports |
    | `--samples <DIR>` | `samples` | Media served over HTTP |
    | `--video <FILE>` | first `.mp4` in samples | Video looped over RTSP |
    | `--ffmpeg <PATH>` / `--mediamtx <PATH>` | from `PATH` | Binary locations |

**Simulating the ROV's network.** The default loopback setup never leaves the machine, so it cannot exercise interface selection. To test that, run the simulator on a second machine (or a Raspberry Pi) that is connected to the client's laptop the way a ROV would be: over the ROV's WiFi, or over a USB Ethernet cable. Give that machine's adapter the ROV address and bind to it:

```sh
chasing-simulator --bind 192.168.1.88 --udp-target 192.168.1.103 --verbose
```

To check what the client would detect on a given machine without starting the GUI (useful on Windows and Linux), run `ROV_HOST=192.168.1.88 cargo test --test rov_link_diagnostic -- --nocapture`; it lists the interfaces, how each was classified (wired or wireless) and which one it would use for the ROV.

With `--verbose` the log shows the source address of every client connection, which shows which laptop interface the video actually used.

**Measuring the forwarder.** `forwarder-probe` runs the same RTSP session directly and through the interface-bound forwarder (alternating, several runs) and prints time to first frame, frame rate, stalls and RTSP round-trip times, plus the difference. It works against the simulator or a real ROV and needs no GUI:

```sh
cargo run --release --features test-tools --bin forwarder-probe -- \
  --url rtsp://admin:admin@192.168.1.88:8554/stream/0/0 --interface en0 --json-out probe.json
```

Run it on each OS and connection scenario and attach the output to the TE-2 ticket. `--mode direct` measures the baseline only; credentials are redacted in the report.

Video goes through the interface-bound RTSP forwarder by default on every OS (on Windows it forces the outgoing adapter with `IP_UNICAST_IF`), so no host route, ARP entry or administrator prompt is needed.
