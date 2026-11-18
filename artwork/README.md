# Artwork

Original artwork made for Open Cycling World, licensed like the code
(MIT OR Apache-2.0). Third-party assets do not go here without a licence
entry (CLAUDE.md, spec §4.6).

## logos/

- `ocw.svg` – the app logo: a unicorn riding a bike over a rainbow (the
  source; edit this one).
- `ocw-<size>.png` – renders of it for icons (16 … 512 px).
- `ocw-studio.svg` – OCW Studio's logo (the editor, `ocw-studio`): a
  horn-gold pencil drawing the rainbow on drafting paper, the rest of the
  rainbow still a dashed construction line. Same tile, sky, rainbow and
  clouds as `ocw.svg`, so the two read as siblings. Original artwork, MIT OR
  Apache-2.0 (stated in the file).
- `ocw-studio-small.svg` – the studio logo simplified for 16 and 32 px
  (four wider bands, a thicker pencil, no grid or sparkles).
- `ocw-studio-<size>.png` – renders for icons (16 … 512 px; 16 and 32 from
  the small variant).

The PNGs are made with `scripts/render-logos.sh [LOGO…]` (rsvg-convert; on
marge: `scripts/remote.sh marge run 'scripts/render-logos.sh ocw-studio'`):
`LOGO-<size>.png` from `LOGO.svg`, 16 and 32 px from `LOGO-small.svg` when
there is one. The game's PNGs were made the same way before the script
(`rsvg-convert -w $s -h $s ocw.svg -o ocw-$s.png`); the script reproduces them
byte for byte (rsvg-convert 2.60 on marge, 2026-11-17).

The game's logo is the window icon of `ocw-desktop`, the studio's that of
`ocw-studio` (shared code: `ocw_ui::app_icon`; X11, Windows title bar and
taskbar), the icon of each Windows exe (`crates/ocw-winres`:
`OCW` for `ocw-desktop` and `ocw-server`, `OCW_STUDIO` for `ocw-studio`),
the desktop entry icon (`packaging/linux/ocw-desktop.desktop`,
`ocw-studio.desktop`, installed by each app at start; Wayland compositors
take the icon from it via the app ids `ocw-desktop` / `ocw-studio`), the
AppImage icon (`scripts/build-appimage.sh`, the game) and the `.icns` of the
macOS bundles (`scripts/build-macos-app.sh`; `OCW_APP=studio` for
`OCW Studio.app`).

## 3dmodels/

The 3D models of the app as glTF 2.0 files (Git LFS), with their manifest
and the model contract: see [`3dmodels/README.md`](3dmodels/README.md)
(spec §14 Phase 15).
