#!/usr/bin/env bash
# Installs gridview for the current user (binary goes to /usr/local/bin).
# Works from an unpacked release tarball and from a source checkout
# (after `cargo build --release`).
set -euo pipefail

cd "$(dirname "$0")"

APP="gridview"

if [[ -f "$APP" ]]; then
    # release tarball layout
    BINARY="$APP"
    ICON="share/icons/hicolor/256x256/apps/$APP.png"
    DESKTOP="share/applications/$APP.desktop"
else
    # source checkout layout
    BINARY="target/release/$APP"
    ICON="assets/icon.png"
    DESKTOP="packaging/$APP.desktop"
fi

SUDO=""
if [[ $EUID -ne 0 ]]; then
    SUDO="sudo"
fi

echo "==> Checking files..."

for file in "$BINARY" "$ICON" "$DESKTOP"; do
    if [[ ! -f "$file" ]]; then
        echo "Error: Missing file: $file"
        exit 1
    fi
done

echo "==> Installing binary..."
$SUDO install -Dm755 "$BINARY" "/usr/local/bin/$APP"

echo "==> Installing icon..."
install -Dm644 "$ICON" \
    "$HOME/.local/share/icons/hicolor/512x512/apps/$APP.png"

echo "==> Installing desktop entry..."
install -Dm644 "$DESKTOP" \
    "$HOME/.local/share/applications/$APP.desktop"

# Also refreshes the mime cache, so gridview shows up under "Open With"
# for the image and video types listed in the desktop entry.
echo "==> Updating desktop database..."
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications"
fi

echo
echo "$APP successfully installed!"
