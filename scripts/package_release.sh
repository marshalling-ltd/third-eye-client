#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_DIR="$ROOT_DIR/target/release"
BUNDLE_DIR="$RELEASE_DIR/bin"
BUNDLED_FFMPEG="$BUNDLE_DIR/ffmpeg"

# Optional override: FFMPEG_SOURCE=/absolute/path/to/ffmpeg ./scripts/package_release.sh
FFMPEG_SOURCE="${FFMPEG_SOURCE:-}"

if [[ -z "$FFMPEG_SOURCE" ]]; then
  if [[ -x "$ROOT_DIR/bin/ffmpeg" ]]; then
    FFMPEG_SOURCE="$ROOT_DIR/bin/ffmpeg"
  elif command -v ffmpeg >/dev/null 2>&1; then
    FFMPEG_SOURCE="$(command -v ffmpeg)"
  else
    echo "Error: ffmpeg not found. Set FFMPEG_SOURCE or place ffmpeg at ./bin/ffmpeg." >&2
    exit 1
  fi
fi

if [[ ! -x "$FFMPEG_SOURCE" ]]; then
  echo "Error: FFMPEG_SOURCE is not executable: $FFMPEG_SOURCE" >&2
  exit 1
fi

echo "Building release binary..."
cargo build --release --bin third-eye-client

mkdir -p "$BUNDLE_DIR"
if [[ -e "$BUNDLED_FFMPEG" && "$FFMPEG_SOURCE" -ef "$BUNDLED_FFMPEG" ]]; then
  :
else
  rm -f "$BUNDLED_FFMPEG"
  cp "$FFMPEG_SOURCE" "$BUNDLED_FFMPEG"
fi
chmod 755 "$BUNDLED_FFMPEG"

echo "Release package ready:"
echo "  App binary: $RELEASE_DIR/third-eye-client"
echo "  Bundled ffmpeg: $BUNDLED_FFMPEG"
