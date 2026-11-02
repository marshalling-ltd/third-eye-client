# Video and photos

Once you are [connected](connect.md):

| Screen | What it does |
|---|---|
| **Live Stream** | Shows the camera full-screen, with depth, heading, battery, attitude and GPS coordinates overlaid on top |
| **Media** | The ROV's photos and videos: preview, download to your laptop, or delete them |

## Live video

- Real-time RTSP video from the ROV camera, decoded by a **bundled ffmpeg**.
- Telemetry and the stream start automatically when you open the Stream screen.
- Telemetry arrives as UDP broadcasts from the ROV (port `8500` by default): attitude (pitch / roll / yaw), depth, water temperature, IMU gyroscope, battery levels and GPS coordinates.

## Taking photos

- The remote shutter takes JPEG, DNG, or JPEG+DNG, with a burst of 1 to 5.
- Each capture gets a telemetry snapshot (depth, attitude, coordinates, battery state) attached as metadata.
- The device GPS fix is refreshed before every capture so the coordinates are as fresh as possible. That position comes from [GPS](gps.md).

## Media library

- Syncs the ROV's on-device file list into a local SQLite registry.
- Browse, download, preview (images with thumbnails) and stream (video via ffmpeg) any file on the ROV.
- Delete media from the ROV directly from the UI.
- Tracks local download state, SHA-256 checksums and capture metadata per file.
- Opens the local media directory in the OS-native file explorer.

Next: [GPS on the map](gps.md).
