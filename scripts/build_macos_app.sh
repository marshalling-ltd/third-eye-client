#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Error: scripts/build_macos_app.sh can only run on macOS." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Third Eye Client"
BUNDLE_NAME="${APP_NAME}.app"
EXECUTABLE_NAME="third-eye-client"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
FFMPEG_SOURCE="${FFMPEG_SOURCE:-}"

if [[ "$BUILD_PROFILE" == "release" ]]; then
  CARGO_BUILD_ARGS=(--release)
  PROFILE_DIR="release"
elif [[ "$BUILD_PROFILE" == "debug" ]]; then
  CARGO_BUILD_ARGS=()
  PROFILE_DIR="debug"
else
  echo "Error: BUILD_PROFILE must be either 'release' or 'debug' (got '$BUILD_PROFILE')." >&2
  exit 1
fi

TARGET_DIR="$ROOT_DIR/target/$PROFILE_DIR"
APP_STAGING_DIR="$ROOT_DIR/target/macos-app"
APP_BUNDLE_DIR="$APP_STAGING_DIR/$BUNDLE_NAME"
CONTENTS_DIR="$APP_BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
INFO_PLIST_SOURCE="$ROOT_DIR/macos/Info.plist"
APP_ICON_SOURCE="$ROOT_DIR/assets/logo.icns"
APP_BINARY_SOURCE="$TARGET_DIR/$EXECUTABLE_NAME"
APP_BINARY_DEST="$MACOS_DIR/$EXECUTABLE_NAME"
INSTALL_DEST_DIR="/Applications"
INSTALL_DEST_APP="$INSTALL_DEST_DIR/$BUNDLE_NAME"

echo "Building $EXECUTABLE_NAME ($BUILD_PROFILE)..."
cargo build "${CARGO_BUILD_ARGS[@]}" --bin "$EXECUTABLE_NAME"

if [[ ! -f "$APP_BINARY_SOURCE" ]]; then
  echo "Error: expected binary not found at $APP_BINARY_SOURCE" >&2
  exit 1
fi

if [[ ! -f "$INFO_PLIST_SOURCE" ]]; then
  echo "Error: Info.plist not found at $INFO_PLIST_SOURCE" >&2
  exit 1
fi

rm -rf "$APP_BUNDLE_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$APP_BINARY_SOURCE" "$APP_BINARY_DEST"
chmod 755 "$APP_BINARY_DEST"
cp "$INFO_PLIST_SOURCE" "$CONTENTS_DIR/Info.plist"
if [[ -f "$APP_ICON_SOURCE" ]]; then
  cp "$APP_ICON_SOURCE" "$RESOURCES_DIR/logo.icns"
else
  echo "Warning: app icon file not found at $APP_ICON_SOURCE; bundle icon will fall back to default."
fi

if [[ -z "$FFMPEG_SOURCE" ]]; then
  if [[ -x "$ROOT_DIR/bin/ffmpeg" ]]; then
    FFMPEG_SOURCE="$ROOT_DIR/bin/ffmpeg"
  elif command -v ffmpeg >/dev/null 2>&1; then
    FFMPEG_SOURCE="$(command -v ffmpeg)"
  fi
fi

if [[ -n "$FFMPEG_SOURCE" ]]; then
  if [[ ! -x "$FFMPEG_SOURCE" ]]; then
    echo "Error: FFMPEG_SOURCE is not executable: $FFMPEG_SOURCE" >&2
    exit 1
  fi
  FFMPEG_DEST_DIR="$MACOS_DIR/bin"
  mkdir -p "$FFMPEG_DEST_DIR"
  cp "$FFMPEG_SOURCE" "$FFMPEG_DEST_DIR/ffmpeg"
  chmod 755 "$FFMPEG_DEST_DIR/ffmpeg"
  echo "Bundled ffmpeg from: $FFMPEG_SOURCE"
else
  echo "Warning: ffmpeg was not found. Stream feature may fail inside the .app bundle."
fi

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_BUNDLE_DIR" >/dev/null
fi

echo "Installing bundle to $INSTALL_DEST_DIR..."
if rm -rf "$INSTALL_DEST_APP" 2>/dev/null && cp -R "$APP_BUNDLE_DIR" "$INSTALL_DEST_DIR/" 2>/dev/null; then
  :
else
  echo "Requesting administrator privileges to install into /Applications..."
  sudo rm -rf "$INSTALL_DEST_APP"
  sudo cp -R "$APP_BUNDLE_DIR" "$INSTALL_DEST_DIR/"
fi

echo "Done."
echo "Installed app: $INSTALL_DEST_APP"
echo "Launch with: open \"$INSTALL_DEST_APP\""
