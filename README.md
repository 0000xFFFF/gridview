# <img src="assets/icon.png" width="25"> gridview - masonry image/video gallery

![preview](preview/preview.gif)

Rewrite of the Electron version, built for folders with tens of thousands of files. Made to be faster.

## Build (Linux)

### Requirements

<details><summary>Debian / Ubuntu</summary>

```
sudo apt update
sudo apt install build-essential pkg-config clang libclang-dev \
    libavcodec-dev libavformat-dev libavutil-dev \
    libswscale-dev libswresample-dev \
    libxkbcommon-dev libwayland-dev libgl1-mesa-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

</details>

<details><summary>Arch Linux</summary>

```
sudo pacman -Sy --needed base-devel pkgconf clang \
    ffmpeg libclang \
    libxkbcommon wayland mesa \
    gstreamer gst-plugins-base gst-plugins-good \
    gst-plugins-bad gst-plugins-ugly gst-libav
```

</details>

```
cargo build --release
```

No ffmpeg dev libs? `cargo build --release --no-default-features` (no video thumbnails).

## Install
From a release tarball (Linux): unpack it and run `./install.sh` (`./uninstall.sh` removes it again).

From source:
```
./install.sh
```

## Use
- Open a folder via the path box, CLI arg (e.g. ```gridview .```, ```gridview ~/Pictures```), or drag & drop a folder onto the window
- Columns slider, or Ctrl+scroll / pinch
- Grouping: by folder / date modified / date created (day or month), always recursive
- Compact checkbox (top bar) or `-C` / `--compact`: packs small groups side by side, each group gets its own gray shade, hover a tile to see its group name
- Find: `Ctrl+F` or the 🔍 Find button (top right) opens a search box. It matches the path relative to the opened folder (sub directories included); the query is split into words and every word must occur, in any order, case-insensitive. Matches are outlined, everything else is dimmed. `Enter` / `Shift+Enter` (or `F3` / `Shift+F3`, or the ▲ ▼ buttons) jump to the next / previous hit, `Esc` closes it
- Hover a tile to see its relative path
- Single file mode: `gridview photo.jpg` (or open a file with it from your file manager, the desktop entry is registered for every supported image/video type) shows just that file. `Esc` then loads its folder, scrolls to the file and highlights it for a moment
- Click a tile for the viewer: Left/Right = prev/next, Esc = close, Enter = open in system player. Mouse wheel (or pinch, or `+` / `-`) zooms towards the cursor, `0` or a double click resets; once zoomed in, drag to move around (the cursor turns into a hand)
- Right click a tile, or the viewer, for Open externally / Show in folder

Cache (thumbnails + image sizes) lives in `~/.cache/gridview`; delete it any time.

Files on an **encrypted drive** (LUKS/dm-crypt incl. LVM or RAID on top of it, VeraCrypt, eCryptfs, gocryptfs, ...) are cached on that drive instead, in `<mount point>/.gridview-cache`, so thumbnails and file names never leak to the unencrypted system drive. If the drive isn't writable nothing is cached for it. Choose the behaviour with `--cache`:

- `auto` (default): on the drive only when it is detected as encrypted (detection is Linux-only)
- `drive`: always on the drive the files are on (use this for BitLocker/fscrypt/ZFS encryption, which aren't detected)
- `central`: always in `~/.cache/gridview`
