#!/usr/bin/env bash
# Removes everything install.sh put in place.
set -euo pipefail

APP="gridview"

SUDO=""
if [[ $EUID -ne 0 ]]; then
    SUDO="sudo"
fi

set -x
$SUDO rm -f "/usr/local/bin/$APP"
rm -f "$HOME/.local/share/applications/$APP.desktop"
rm -f "$HOME/.local/share/icons/hicolor/512x512/apps/$APP.png"
set +x

if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications"
fi

echo "$APP uninstalled."
