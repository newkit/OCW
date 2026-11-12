# Fonts

Third-party fonts bundled into `ocw-desktop` (compiled in with
`include_bytes!` in `crates/ocw-render/src/font.rs`, so the binary needs no
font files at run time). Each font keeps its own licence; it is not covered
by the MIT/Apache-2.0 licence of the code.

| File | Font | Version | Licence | Source | SHA-256 |
|---|---|---|---|---|---|
| `FiraMono-Regular.ttf` | Fira Mono Regular, © 2012-2015 The Mozilla Foundation and Telefonica S.A. | 3.206 (unmodified, not subset) | SIL Open Font License 1.1, [`OFL.txt`](OFL.txt) | [google/fonts `ofl/firamono`](https://github.com/google/fonts/tree/main/ofl/firamono) | `2e00b0cf3106a3d792f35711b7722740b76c02ee7300ea8c60f940584b7a8ac9` |

Fira Mono is the font Bevy's built-in default is a subset of (ASCII only), so
the UI looks and measures the same (glyphs 0.6 em wide), but now covers
Latin-1, Latin Extended-A (all but U+017F long s), Greek, Cyrillic, the degree sign, arrows
(← ↑ → ↓), bullets (• ●), dashes and the middle dot. The test
`font::tests` checks the characters the UI relies on.
