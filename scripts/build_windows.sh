#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Error: scripts/build_windows.sh must be run from macOS." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXECUTABLE_NAME="third-eye-client"
TARGET="x86_64-pc-windows-gnu"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
FFMPEG_SOURCE="${FFMPEG_SOURCE:-}"

if [[ "$BUILD_PROFILE" == "release" ]]; then
  CARGO_BUILD_ARGS=(--release)
  PROFILE_DIR="release"
elif [[ "$BUILD_PROFILE" == "debug" ]]; then
  CARGO_BUILD_ARGS=()
  PROFILE_DIR="debug"
else
  echo "Error: BUILD_PROFILE must be 'release' or 'debug' (got '$BUILD_PROFILE')." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "MinGW cross-compiler not found. Installing via Homebrew..."
  brew install mingw-w64
fi

if ! rustup target list --installed | grep -q "$TARGET"; then
  echo "Rust target $TARGET not installed. Installing..."
  rustup target add "$TARGET"
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

export CC_x86_64_pc_windows_gnu="x86_64-w64-mingw32-gcc"
export CXX_x86_64_pc_windows_gnu="x86_64-w64-mingw32-g++"
export AR_x86_64_pc_windows_gnu="x86_64-w64-mingw32-ar"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="x86_64-w64-mingw32-gcc"

echo "Building $EXECUTABLE_NAME ($BUILD_PROFILE) for $TARGET..."
cargo build "${CARGO_BUILD_ARGS[@]}" --bin "$EXECUTABLE_NAME" --target "$TARGET"

BINARY_SOURCE="$ROOT_DIR/target/$TARGET/$PROFILE_DIR/$EXECUTABLE_NAME.exe"
if [[ ! -f "$BINARY_SOURCE" ]]; then
  echo "Error: expected binary not found at $BINARY_SOURCE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Package
# ---------------------------------------------------------------------------

PACKAGE_DIR="$ROOT_DIR/target/windows-package"
BIN_DIR="$PACKAGE_DIR/bin"

rm -rf "$PACKAGE_DIR"
mkdir -p "$BIN_DIR"

cp "$BINARY_SOURCE" "$PACKAGE_DIR/$EXECUTABLE_NAME.exe"

# Bundle ffmpeg.exe
if [[ -z "$FFMPEG_SOURCE" ]]; then
  if [[ -f "$ROOT_DIR/bin/ffmpeg.exe" ]]; then
    FFMPEG_SOURCE="$ROOT_DIR/bin/ffmpeg.exe"
  fi
fi

if [[ -n "$FFMPEG_SOURCE" ]]; then
  if [[ ! -f "$FFMPEG_SOURCE" ]]; then
    echo "Error: FFMPEG_SOURCE not found: $FFMPEG_SOURCE" >&2
    exit 1
  fi
  cp "$FFMPEG_SOURCE" "$BIN_DIR/ffmpeg.exe"
  echo "Bundled ffmpeg.exe from: $FFMPEG_SOURCE"
else
  echo "Warning: ffmpeg.exe not bundled. Place a Windows ffmpeg.exe at ./bin/ffmpeg.exe"
  echo "         or set FFMPEG_SOURCE=/path/to/ffmpeg.exe before running this script."
  echo "         The stream feature will not work without it."
fi

# Zip it up
ZIP_PATH="$ROOT_DIR/target/third-eye-client-windows-x64.zip"
rm -f "$ZIP_PATH"
(cd "$PACKAGE_DIR" && zip -r "$ZIP_PATH" .)

echo ""
echo "Done."
echo "  Package dir : $PACKAGE_DIR"
echo "  Zip         : $ZIP_PATH"
echo ""
echo "Copy the zip to a Windows machine, extract, and run $EXECUTABLE_NAME.exe"
