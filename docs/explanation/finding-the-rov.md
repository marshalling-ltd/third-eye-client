# How the app finds the ROV

The ROV is reached over a local network link, either its own Wi-Fi or a USB-Ethernet cable. This page explains what the app does so you do not have to configure anything.

## The addresses involved

The ROV sits at `192.168.1.88` and expects its client at `192.168.1.103`. The client reaches three services on the ROV: telemetry broadcasts over UDP (`8500`), live video over RTSP (`8554`) and the camera API over HTTP (`80`).

## Picking the link

The app looks at the computer's network interfaces and chooses the one that has an address on the ROV's subnet.

- A **cable wins over Wi-Fi** when both qualify.
- A link that is not on the ROV's subnet, such as a phone hotspot, is ignored.
- The check repeats every 2 seconds, so plugging a cable in or joining the ROV's Wi-Fi is noticed within a few seconds and video and telemetry move to the new link. **Recalibrate ROV network** runs the same detection on demand.
- If a cable is connected but its adapter has no address on the ROV's subnet, the app cannot fix that itself, because setting an address needs administrator rights. It shows the exact steps for your OS instead, and carries on by itself once the address is set.

## Why traffic is pinned to the interface

A laptop usually has several connections at once: the ROV, a hotspot, an office network. Without help the operating system may send ROV traffic out of the wrong one. The app binds its HTTP and UDP sockets to the chosen interface (`IP_BOUND_IF` on macOS, `SO_BINDTODEVICE` on Linux).

## Why video goes through a forwarder

Video is decoded by **ffmpeg**, which runs as a separate process and offers no way to bind to an interface. Older versions solved that by changing the system routing table, which needed an administrator password each time.

Now ffmpeg connects to a small forwarder inside the app, listening on `127.0.0.1`. The forwarder opens the real connection to the ROV from a socket pinned to the chosen interface (`IP_BOUND_IF` on macOS, `SO_BINDTODEVICE` on Linux, and `IP_UNICAST_IF` plus a source-address bind on Windows) and relays the bytes. ffmpeg never needs to know which link is in use, and no host route, ARP entry or administrator prompt is needed.

## Related

- [Connect to the ROV](../guide/connect.md) for the steps
- [Architecture](../reference/architecture.md) for how the pieces fit together
