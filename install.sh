
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

APP="gridview"

BINARY="target/release/$APP"
ICON="assets/icon.png"
DESKTOP="packaging/$APP.desktop"

echo "==> Checking files..."

for file in "$BINARY" "$ICON" "$DESKTOP"; do
    if [[ ! -f "$file" ]]; then
        echo "Error: Missing file: $file"
        exit 1
    fi
done

echo "==> Installing binary..."
sudo install -Dm755 "$BINARY" "/usr/local/bin/$APP"

echo "==> Installing icon..."
install -Dm644 "$ICON" \
    "$HOME/.local/share/icons/hicolor/512x512/apps/$APP.png"

echo "==> Installing desktop entry..."
install -Dm644 "$DESKTOP" \
    "$HOME/.local/share/applications/$APP.desktop"

echo "==> Updating desktop database..."
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications"
fi

echo
echo "$APP successfully installed!"

