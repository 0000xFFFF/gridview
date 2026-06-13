
#!/usr/bin/env bash
set -xe

# Install dependencies (including devDependencies required for packaging)
npm install || { echo "npm install failed"; exit 1; }

# Run packaging (uses script which invokes npx electron-packager)
npm run pack || { echo "npm run pack failed"; ls -la release-builds || true; exit 1; }

# Define the relative path to the icon
relative_icon_path="assets/icon.png"

# Get the absolute path to the icon
full_icon_path="$(realpath "$relative_icon_path")"

BUILD_DIR="release-builds/gridview-linux-x64"
if [ ! -d "$BUILD_DIR" ]; then
	echo "Expected build directory $BUILD_DIR not found. Contents of release-builds:"
	ls -la release-builds || true
	exit 1
fi

BIN_PATH="$BUILD_DIR/gridview"
if [ ! -f "$BIN_PATH" ]; then
	echo "Expected binary $BIN_PATH not found. Listing $BUILD_DIR:"
	ls -la "$BUILD_DIR" || true
	exit 1
fi

# Create or replace the symbolic link to the application
sudo ln -sfr "$BIN_PATH" /usr/local/bin/gridview

mkdir -p ~/.local/share/applications
# Generate the gridview.desktop file
cat << EOF > ~/.local/share/applications/gridview.desktop
[Desktop Entry]
Version=1.4.0
Name=GridView
Exec=/usr/local/bin/gridview
Icon=$full_icon_path
Type=Application
Terminal=false
EOF

# Update the desktop database (optional)
update-desktop-database ~/.local/share/applications/
