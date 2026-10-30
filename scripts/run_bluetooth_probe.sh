#!/usr/bin/env bash
# Builds examples/iobt_link_probe into a throwaway .app (macOS only grants
# Bluetooth access to bundles with NSBluetoothAlwaysUsageDescription) and runs
# it against a paired GPS, streaming its output. Usage:
#   scripts/run_bluetooth_probe.sh                       (list candidates)
#   scripts/run_bluetooth_probe.sh <name-or-MAC> [seconds]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
EX=iobt_link_probe
cargo build --example "$EX"
# The bundle keeps its original "rfcomm-spike" name/identifier so the existing
# macOS Bluetooth permission grant still applies.
APP="$ROOT/target/rfcomm-spike.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "target/debug/examples/$EX" "$APP/Contents/MacOS/rfcomm_spike"
sed -e 's#<string>third-eye-client</string>#<string>rfcomm_spike</string>#' \
    -e 's#dev.warp.third-eye-client#dev.warp.third-eye-rfcomm-spike#' \
    -e '/CFBundleIconFile/,+1d' macos/Info.plist > "$APP/Contents/Info.plist"
codesign --force --sign - "$APP"
OUT="$(mktemp)"
open -W -n --stdout "$OUT" --stderr "$OUT" "$APP" --args "$@" &
OPEN_PID=$!
tail -f "$OUT" &
TAIL_PID=$!
wait "$OPEN_PID" || true
sleep 0.5
kill "$TAIL_PID" 2>/dev/null || true
