#!/bin/bash -x
npm install
npm run pack

# Define the relative path to the icon
relative_icon_path="assets/icon.png"

# Get the absolute path to the icon
full_icon_path="$(realpath "$relative_icon_path")"

# Create the symbolic link to the application
sudo ln -sfr release-builds/gridview-linux-x64/gridview /usr/local/bin/gridview

mkdir -p ~/.local/share/applications
# Generate the gridview.desktop file
cat << EOF > ~/.local/share/applications/gridview.desktop
[Desktop Entry]
Version=1.0
Name=GridView
Exec=/usr/local/bin/gridview
Icon=$full_icon_path
Type=Application
Terminal=false
EOF

# Update the desktop database (optional)
update-desktop-database ~/.local/share/applications/
