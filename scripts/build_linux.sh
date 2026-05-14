#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Error: scripts/build_linux.sh must be run on Linux." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
  echo "Error: BUILD_PROFILE must be 'release' or 'debug' (got '$BUILD_PROFILE')." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# System dependencies
# ---------------------------------------------------------------------------

MISSING_PKGS=()
for pkg in libwayland-dev libxkbcommon-dev libx11-dev libfontconfig-dev pkg-config; do
  if ! dpkg -s "$pkg" >/dev/null 2>&1; then
    MISSING_PKGS+=("$pkg")
  fi
done

if [[ ${#MISSING_PKGS[@]} -gt 0 ]]; then
  echo "Installing missing system packages: ${MISSING_PKGS[*]}"
  sudo apt-get update -qq
  sudo apt-get install -y \
    libwayland-dev \
    libxkbcommon-dev \
    libx11-dev \
    libxcb1-dev \
    libxcb-render0-dev \
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libfontconfig-dev \
    libegl1-mesa-dev \
    libgl1-mesa-dev \
    pkg-config \
    build-essential
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

echo "Building $EXECUTABLE_NAME ($BUILD_PROFILE)..."
cargo build "${CARGO_BUILD_ARGS[@]}" --bin "$EXECUTABLE_NAME"

BINARY_SOURCE="$ROOT_DIR/target/$PROFILE_DIR/$EXECUTABLE_NAME"
if [[ ! -f "$BINARY_SOURCE" ]]; then
  echo "Error: expected binary not found at $BINARY_SOURCE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Assemble AppDir
# ---------------------------------------------------------------------------

APPDIR="$ROOT_DIR/target/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"

cp "$BINARY_SOURCE" "$APPDIR/usr/bin/$EXECUTABLE_NAME"
chmod +x "$APPDIR/usr/bin/$EXECUTABLE_NAME"

# ffmpeg — place beside the binary so locate_ffmpeg_binary() finds it
if [[ -z "$FFMPEG_SOURCE" ]]; then
  if [[ -x "$ROOT_DIR/bin/ffmpeg" ]]; then
    FFMPEG_SOURCE="$ROOT_DIR/bin/ffmpeg"
  elif command -v ffmpeg >/dev/null 2>&1; then
    FFMPEG_SOURCE="$(command -v ffmpeg)"
  fi
fi

if [[ -n "$FFMPEG_SOURCE" ]]; then
  cp "$FFMPEG_SOURCE" "$APPDIR/usr/bin/ffmpeg"
  chmod +x "$APPDIR/usr/bin/ffmpeg"
  echo "Bundled ffmpeg from: $FFMPEG_SOURCE"
else
  echo "Warning: ffmpeg not bundled. The stream feature requires ffmpeg in PATH at runtime."
  echo "         Set FFMPEG_SOURCE=/path/to/ffmpeg or install ffmpeg system-wide."
fi

# AppRun
cat > "$APPDIR/AppRun" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/third-eye-client" "$@"
EOF
chmod +x "$APPDIR/AppRun"

# .desktop file (required by appimagetool)
cat > "$APPDIR/$EXECUTABLE_NAME.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=Third Eye Client
Exec=third-eye-client
Icon=logo
Categories=Utility;
EOF

# Icon (required by appimagetool)
cp "$ROOT_DIR/assets/logo.png" "$APPDIR/logo.png"

# ---------------------------------------------------------------------------
# Create AppImage
# ---------------------------------------------------------------------------

APPIMAGETOOL="$ROOT_DIR/target/appimagetool"
if [[ ! -x "$APPIMAGETOOL" ]]; then
  echo "Downloading appimagetool..."
  curl -fsSL \
    "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage" \
    -o "$APPIMAGETOOL"
  chmod +x "$APPIMAGETOOL"
fi

APPIMAGE_PATH="$ROOT_DIR/target/$EXECUTABLE_NAME-linux-x64.AppImage"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" "$APPDIR" "$APPIMAGE_PATH"
chmod +x "$APPIMAGE_PATH"

echo ""
echo "Done."
echo "  AppImage: $APPIMAGE_PATH"
echo ""
echo "Make it executable and run: chmod +x $APPIMAGE_PATH && $APPIMAGE_PATH"
