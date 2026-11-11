# Release builds

Prebuilt binaries of Open Cycling World for the platforms we can build on.
These are development builds (spec §14 Phase 1/2), not signed or notarised.

## 0.1.0-c3187a3 – macOS app

Built from commit `c3187a3` (with free ride, Strava, video recording, R/C/V
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
