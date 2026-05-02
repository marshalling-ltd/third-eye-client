# third-eye-client

Rust client for connecting to and controlling an ROV via USB ethernet.

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
