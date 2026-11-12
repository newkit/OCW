# Release builds

Prebuilt binaries of Open Cycling World are published as **GitHub Releases**:
https://github.com/newkit/OCW/releases (with the world packages, which are
ODbL data and are not part of this repository). They are development builds
(spec §14), not signed or notarised. This file keeps the build notes.

Binaries are no longer committed to git: on 2026-11-12 the history was
rewritten twice, to remove the former `release/<version>/<platform>/` folders
and then the build-history media (`artwork/build-history/YYYY-MM-DD/`, now zip
files on the GitHub release `build-history`), which changed the commit ids.
`commit-map.txt` maps the original ids and those after the first rewrite to
the current ones; the ids below are the current ones. The builds
`0.1.0-15d89d0`, `-c3187a3` and `-310360b` (old ids) are not downloadable any
more; the latest build is on the Releases page. Its files and the apps'
`--version` still show the old commit ids `fc5947f` (Linux, Windows) and
`dddee37` (macOS), because they were built before the rewrite.

## 0.1.0-c5255a1 – Linux and Windows

Built from commit `c5255a1` (same code as the macOS app `8613029` below; the
later commits only add docs and the DMG) with
`cargo build --release -p ocw-desktop -p ocw-server -p world-builder`.
726 tests passed on Linux, macOS and Windows for this code (spec 2.8,
Appendix B.4).

| Platform | Built on | File | Size |
|---|---|---|---|
| linux-x86_64 | Linux build machine (`marge`) | `OpenCyclingWorld-0.1.0-fc5947f-x86_64.AppImage` | 44.6 MB |
| linux-x86_64 | | `ocw-desktop.xz` (stripped, 123 MB unpacked) | 22.8 MB |
| linux-x86_64 | | `ocw-server` | 6.2 MB |
| linux-x86_64 | | `world-builder` | 2.7 MB |
| windows-x86_64 | Windows 11 build machine | `OpenCyclingWorld-0.1.0-fc5947f-windows-x86_64.zip` (`ocw-desktop.exe`, `ocw-server.exe`, `world-builder.exe`) | 39.1 MB |

- **Linux AppImage:** `chmod +x OpenCyclingWorld-*.AppImage && ./OpenCyclingWorld-*.AppImage`.
  Without FUSE 2 (`libfuse2t64` / `libfuse2`) start it with
  `APPIMAGE_EXTRACT_AND_RUN=1`. Packed by `scripts/build-appimage.sh`.
- **Windows:** unzip and start `ocw-desktop.exe`. The binaries are not
  signed, so SmartScreen warns on the first start (*More info* → *Run
  anyway*). Needs a DirectX 12 or Vulkan GPU driver. No installer yet.
- Checksums: `SHA256SUMS` in each platform folder (Linux also
  `SHA256SUMS.uncompressed` for the unpacked `ocw-desktop`).

## 0.1.0-8613029 – macOS app

Built from commit `8613029` on the Apple Silicon Mac by
`scripts/build-macos-app.sh` at the end of the autonomous night (spec 2.8,
Appendix B). New since `b17141a`: in-game menu (Esc / Start) with settings,
rider profiles, bikes and looks, ride switching without restart, Navi pages
and GPX import; structured workouts with ERG; local events (race, group ride,
time trial against your ghost); gradient average, elevation profile and
camera tilt; trees fade instead of moving the camera; tunnels; neighbour world
packages; QUIC networking (`--server`); full-Latin UI font; U-turn and
steep-ramp fixes. 726 tests passed on macOS for this commit.

| Platform | File | Size |
|---|---|---|
| macos-arm64 | `OpenCyclingWorld-0.1.0-dddee37-macos-arm64.dmg` | 39.6 MB |

## 0.1.0-b17141a – macOS app

Built from commit `b17141a` on the Apple Silicon Mac by
`scripts/build-macos-app.sh`. New since `532c2d3`: routes from GPX files and
world packages on the road graph (route arrows, leaving and rejoining the
route), signed and versioned world packages, a missing `--devices-file`
falls back to the default file, gamepad triggers calibrated to their resting
value, and ffmpeg for video recording (V) is found when the app is started
from Finder (Homebrew's `/opt/homebrew/bin`, `/usr/local/bin`,
`~/.local/bin`).

| Platform | File | Size |
|---|---|---|
| macos-arm64 | `OpenCyclingWorld-0.1.0-310360b-macos-arm64.dmg` | 37.9 MB |

Install as for `532c2d3` below (drag to *Applications*, right-click →
*Open* on the first start). Video recording needs ffmpeg: install Homebrew
(`/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"`,
asks for an admin password), then `brew install ffmpeg`.
Checksum: `shasum -a 256 -c SHA256SUMS`.

## 0.1.0-532c2d3 – macOS app

Built from commit `532c2d3` (with free ride, Strava, video recording, R/C/V
keys) on the Apple Silicon Mac by `scripts/build-macos-app.sh`:

| Platform | File | Size |
|---|---|---|
| macos-arm64 | `OpenCyclingWorld-0.1.0-c3187a3-macos-arm64.dmg` | 37.7 MB |

The DMG holds `Open Cycling World.app` (icon, Info.plist with the Bluetooth
permission text, ad-hoc signature). To install: open the DMG and drag the app
into *Applications*. The app is not notarised, so the first start needs
right-click → *Open* (or *System Settings → Privacy & Security → Open
Anyway*); a downloaded DMG may also need
`xattr -dr com.apple.quarantine "/Applications/Open Cycling World.app"`.
Checksum: `shasum -a 256 -c SHA256SUMS`.

## 0.1.0-15d89d0

Built from commit `15d89d0` (workspace version 0.1.0) with
`cargo build --release -p ocw-desktop -p ocw-server -p world-builder`
(Rust 1.99.1 from `rust-toolchain.toml`, default release profile), then
`strip`ped on the build machine. Built via `scripts/remote.sh <machine> run ...`.

| Platform | Built on | File | Size | Uncompressed |
|---|---|---|---|---|
| linux-x86_64 | Linux build machine (`marge`) | `ocw-desktop.xz` | 21.4 MB | 123.8 MB |
| linux-x86_64 | | `ocw-server` | 2.4 MB | |
| linux-x86_64 | | `world-builder` | 1.6 MB | |
| macos-arm64 | Apple Silicon Mac (`macos`) | `ocw-desktop.xz` | 17.4 MB | 96.2 MB |
| macos-arm64 | | `ocw-server` | 2.0 MB | |
| macos-arm64 | | `world-builder` | 1.4 MB | |

- `ocw-desktop` is xz-compressed because the stripped binary exceeds GitHub's
  50 MB warning / 100 MB hard limit and Git LFS is not set up. Unpack with
  `xz -dk ocw-desktop.xz && chmod +x ocw-desktop`.
- Each platform folder has `SHA256SUMS` (files as committed) and
  `SHA256SUMS.uncompressed` (the unpacked binaries); check with
  `sha256sum -c SHA256SUMS` (`shasum -a 256 -c` on macOS).
- **Windows:** no build. A Windows build machine is not available yet (the
  configured one was not reachable), so there is no Windows binary.
- **AppImage:** none. `scripts/build-appimage.sh` assembled the AppDir on the
  Linux machine, but `appimagetool` is not installed there, so no AppImage
  was packed.

## Runtime requirements

- **Linux x86_64:** glibc 2.39 or newer; a Vulkan driver (Mesa RADV/ANV,
  NVIDIA, or lavapipe for software rendering); X11 or Wayland; libudev,
  libdbus (BlueZ for Bluetooth trainers) and libasound (ALSA) are linked
  dynamically.
- **macOS arm64:** Apple Silicon, Metal. The binary is not signed or
  notarised: remove the quarantine flag
  (`xattr -d com.apple.quarantine ocw-desktop`) or allow it in System
  Settings. Bluetooth access needs the system permission prompt.
  For an app that starts from Finder (with icon and the Bluetooth usage
  text macOS requires), build `Open Cycling World.app` and its DMG with
  `scripts/build-macos-app.sh` on a Mac, or take the `macos-app` artifact of
  a CI run on `main`.
- `ocw-desktop` runs the built-in test routes and the virtual trainer with
  no extra files (`ocw-desktop --virtual`). `ocw-server` is the headless
  game server (`ocw-server --bots 3 --seconds 10 --fast`).

## Data files are not included

Routes and world packages for `ocw-desktop --route-file ... --world ...` are
**not** part of this release: `data/` is git-ignored because it holds
OpenStreetMap extracts, elevation tiles and files derived from them. Build
them yourself with `world-builder` (see `scripts/fetch-pilot.sh`).

Licence notes for such data: the road graph in a world package is a
Derivative Database of OpenStreetMap and must be offered under the ODbL;
meshes and renders made from it need the attribution
"© OpenStreetMap contributors".

The code is dual-licensed MIT / Apache-2.0 (see `LICENSE-MIT`,
`LICENSE-APACHE`).
