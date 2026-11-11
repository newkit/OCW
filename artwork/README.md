# Artwork

Original artwork made for Open Cycling World, licensed like the code
(MIT OR Apache-2.0). Third-party assets do not go here without a licence
entry (CLAUDE.md, spec §4.6).

## logos/

- `ocw.svg` – the app logo: a unicorn riding a bike over a rainbow (the
  source; edit this one).
- `ocw-<size>.png` – renders of it for icons (16 … 512 px), made with
  `for s in 16 32 48 64 128 256 512; do rsvg-convert -w $s -h $s ocw.svg -o ocw-$s.png; done`.

Used as the window icon (`apps/desktop/src/icon.rs`, X11 and Windows), the
desktop entry icon (`packaging/linux/ocw-desktop.desktop`, installed by
`scripts/deploy-test.sh`; Wayland compositors take the icon from it via the
app id `ocw-desktop`) and the AppImage icon (`scripts/build-appimage.sh`).
