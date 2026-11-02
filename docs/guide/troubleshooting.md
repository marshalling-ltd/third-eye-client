# Troubleshooting

Try these first, in order:

1. Is the **ROV powered on**? (Check its own manual.)
2. Press **Recalibrate ROV network** again (Profile → ROV network, or on the Live Stream screen).
3. **Cable users:** double-check your adapter's address is `192.168.1.103`.
4. Restart the stream.

## Symptom, cause, fix

| Symptom | Likely cause | Fix |
|---|---|---|
| UDP telemetry works but no TCP/RTSP | ARP not resolving, so the ROV cannot find the client | Verify the static IP is `192.168.1.103` |
| ARP requests visible in Wireshark but no replies | Wrong IP on the USB adapter | Set the IP to `192.168.1.103` |
| Stream screen says the ROV cannot be found | The laptop has no link with an address on the ROV's subnet | Join the ROV's Wi-Fi, or follow the guide the app shows for the USB adapter, then check the link shown on the stream screen |
| The app picked the wrong link (for example a phone hotspot is up) | Another connection was active when the app last looked | Connect to the ROV's Wi-Fi or plug in the cable, then press **Recalibrate ROV network** |
| Video is garbled or blocky | Decode corruption in the running stream | Press **Recalibrate ROV network**; a running stream is restarted with a fresh connection |
| GPS port fails with *Permission denied* (Linux) | Account not in the `dialout` group | `sudo usermod -aG dialout $USER`, then log out and back in |
| GPS port connects but never gets a position | Receiver exposes several USB ports and this one carries no position data | Try the other ports, or use Bluetooth |
| No GPS fix although the receiver is connected | Receiver cannot see the satellites (for example indoors) | Move it outdoors or near a window |

See [Verifying connectivity](connect.md#verifying-connectivity) for commands that check ARP, HTTP and RTSP reachability.
