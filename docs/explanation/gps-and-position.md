# How position gets onto the map

## Why the ROV does not provide it

GPS needs a radio signal from satellites, and that signal does not travel through water. The ROV can report depth and attitude, but not where it is. The position on the map is therefore the position of **the operator at the surface**, not of the ROV itself.

## Where the position comes from

The app takes a position from whichever source you choose and uses it for three things: centring the map, the coordinates in the telemetry overlay, and the metadata attached to each photo. The sources are:

| Source | Used when |
|---|---|
| **OS location** (CoreLocation on macOS, Windows Location Services) | The laptop has usable location services |
| **IP geolocation** | Fallback on Linux, which is accurate to a city rather than a boat |
| **NMEA over TCP** | A phone app sends or serves positions over the network |
| **NMEA over Bluetooth or USB serial** | A dedicated GPS receiver, or a phone app that offers Bluetooth |

NMEA is the standard sentence format GPS receivers speak. The app also accepts TAIP lines. It only listens: it never sends commands to a receiver.

## Why a fix can be missing

Three separate things have to work, and they fail independently:

1. **The connection** between the receiver and the laptop. The Bluetooth or USB port opens, or the TCP link is up.
2. **Data** arriving over it. Some receivers expose several USB ports and only one carries position sentences; the app warns after about 10 seconds if a port sends nothing useful.
3. **A fix**. A receiver indoors can be connected, sending data, and still have no position because it cannot see the satellites.

The **GPS** badge on the map shows the result: amber `..` means running without a position yet, green **OK** means a fix, red `--` means off. Positions that stop updating are treated as stale after a timeout (default 10 s).

## Fresh coordinates for photos

Before each capture the app refreshes the device position, so the coordinates stored with the photo are as recent as possible.

## Related

- [GPS on the map](../guide/gps.md) for setup per OS and device
