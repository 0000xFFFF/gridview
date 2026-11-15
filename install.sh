
#!/usr/bin/env bash
set -euo pipefail

# Always run from the project root
cd "$(dirname "$0")"

APP_NAME="gridview"
BINARY="target/release/$APP_NAME"
ICON="assets/icon.png"

INSTALL_BIN="/usr/local/bin/$APP_NAME"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
ICON_PATH="$ICON_DIR/$APP_NAME.png"
DESKTOP_DIR="$HOME/.local/share/applications"
DESKTOP_FILE="$DESKTOP_DIR/$APP_NAME.desktop"

echo "==> Building $APP_NAME..."
cargo build --release

# Verify build output
if [[ ! -f "$BINARY" ]]; then
    echo "Error: Release binary not found: $BINARY"
    exit 1
fi

if [[ ! -f "$ICON" ]]; then
    echo "Error: Application icon not found: $ICON"
    exit 1
fi

echo "==> Installing binary..."
sudo install -Dm755 "$BINARY" "$INSTALL_BIN"

echo "==> Installing icon..."
install -Dm644 "$ICON" "$ICON_PATH"

echo "==> Creating desktop entry..."
mkdir -p "$DESKTOP_DIR"

cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Version=1.0
Name=GridView
Comment=Image and video viewer
Exec=$INSTALL_BIN
Icon=$APP_NAME
Type=Application
Terminal=false
Categories=Graphics;Viewer;
StartupNotify=true
EOF

echo "==> Updating desktop database..."
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR"
fi

echo
echo "GridView successfully installed!"
echo "Binary:  $INSTALL_BIN"
echo "Icon:    $ICON_PATH"
echo "Desktop: $DESKTOP_FILE"

