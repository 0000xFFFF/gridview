#!/usr/bin/env bash
set -euo pipefail

APP_NAME="gridview"
BINARY="target/release/$APP_NAME"
INSTALL_DIR="/usr/local/bin"
INSTALL_PATH="$INSTALL_DIR/$APP_NAME"

if [[ ! -f "$BINARY" ]]; then
    echo "Error: $BINARY not found. Build the project first."
    exit 1
fi

echo "Installing $APP_NAME..."

sudo install -Dm755 "$BINARY" "$INSTALL_PATH"

echo "Installed successfully: $INSTALL_PATH"
