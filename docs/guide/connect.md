# Connect to the ROV

![Chasing M2S ROV](../assets/chasing-m2s.jpg)

!!! important
    Follow the steps **in order**.

## Step 0: Read the ROV's own manual first

Before touching the app, read the **operation manual that came with your Chasing ROV**. Learn how to:

- charge and power the ROV on and off,
- attach the tether cable safely,
- turn the ROV's Wi-Fi on.

!!! note
    The app only *talks* to the ROV. The ROV still has to be powered on and set up the way its own manual describes.

## Step 1: Connect your laptop to the ROV

**Both connection types work on every operating system.** Pick whichever is easier; you do not need both.

!!! note
    The app does not care how you connect. It talks to the ROV the same way over Wi-Fi or a wired USB link, so the connection type never changes what the app can do.

=== "Wi-Fi (easiest)"

    1. On the ROV, turn on its Wi-Fi (see the ROV manual).
    2. On your laptop, open Wi-Fi settings and **connect to the ROV's Wi-Fi network**, like joining any home or office Wi-Fi.

=== "USB-to-Ethernet cable"

    1. Plug a **USB-to-Ethernet adapter** into your laptop.
    2. Plug the **ROV's network cable** into that adapter.
    3. Give the adapter a fixed address so it can find the ROV:

        | Setting | Value |
        |---|---|
        | ROV address | `192.168.1.88` |
        | **Your adapter's IP** | `192.168.1.103` |
        | Subnet mask | `255.255.255.0` |

        The ROV expects its client at `192.168.1.103`.

    The app notices a USB-Ethernet cable that has no address on the ROV's network and shows the exact steps for your OS, with your adapter's name filled in, on the stream screen. It notices the address within a few seconds of you setting it, so there is no need to restart or press Recalibrate.

    Setting an address needs administrator rights, so you do it yourself in your system settings. The app never asks for an administrator password.

    The app shows the steps for your OS, with your adapter's name filled in:

    === "macOS"

        System Settings → Network → the USB adapter → Details → TCP/IP → Configure IPv4: **Manually**, IP `192.168.1.103`, subnet mask `255.255.255.0`, router empty. Or in a terminal:

        ```sh
        # Find your USB adapter name (e.g. en10)
        ifconfig | grep -B2 "status: active"

        # Set the static IP (replace en10 with your adapter name)
        sudo ifconfig en10 inet 192.168.1.103 netmask 255.255.255.0
        ```

    Replace the adapter names in the commands (`en10`, `Ethernet 2`, `enp3s0`) with your own. On Linux, `ip addr add` lasts until the next reboot or until the link is reset.

    === "Windows"

        Settings → Network & Internet → Ethernet → IP assignment → Edit → Manual → IPv4: IP `192.168.1.103`, subnet mask `255.255.255.0`, gateway empty. Or in an administrator terminal:

        ```sh
        netsh interface ip set address name="Ethernet 2" static 192.168.1.103 255.255.255.0
        ```

    === "Linux"

        Set a manual IPv4 address in your network settings: IP `192.168.1.103`, subnet mask `255.255.255.0`, gateway empty. Or in a terminal:

        ```sh
        sudo ip addr add 192.168.1.103/24 dev enp3s0
        ```

## Step 2: Press "Recalibrate" and let the app figure it out

You do **not** have to tell the app whether you used Wi-Fi or the cable. It checks for you.

1. Open the menu at the top right of the app and choose **Profile**.
2. In the **ROV network** section, click **Recalibrate ROV network**. The **Live Stream** screen has the same button, **Recalibrate ROV Network**.
3. Wait a few seconds and read the message it shows.

What Recalibrate does:

- It re-detects how the ROV is reached. Nothing is changed on your system and no administrator rights are involved.
- **Wi-Fi comes first, on every OS.** If your laptop is on the ROV's Wi-Fi, the app uses it.
- **A cable wins over Wi-Fi**, and an unrelated link such as a phone hotspot is ignored.
- It then moves telemetry and video onto that link. If a stream is running it is restarted with a fresh connection, which also clears garbled or blocky video.

If the message says it found your ROV / interface, you are connected.

The app also watches for cables and Wi-Fi coming and going (every 2 s). Within a few seconds it moves video and telemetry onto the new link. The stream screen shows which link is in use, or what to check when the ROV cannot be found.

## Did it work?

Open **Live Stream** from the menu. You should see live video within a few seconds. If you do not, see [Troubleshooting](troubleshooting.md).

## Verifying connectivity

```sh
# Check ARP resolves (should show the ROV's real MAC, not the adapter's MAC)
arp -an | grep 192.168.1.88

# Test HTTP API
nc -vz -w 3 192.168.1.88 80

# Test RTSP
nc -vz -w 3 192.168.1.88 8554
```

Next: [Video and photos](video-and-capture.md).
