//! The UI font: Fira Mono Regular (SIL OFL 1.1, see
//! `assets/fonts/README.md`), compiled into the binary.
//!
//! Bevy's built-in default font is an ASCII-only subset of Fira Mono, so
//! street names like "Bühelstrasse" showed as "B?helstrasse". The full font
//! replaces it under the default font handle: every `TextFont` that does not
//! name a font (HUD, junction overlay, menu) uses it, with the same look and
//! metrics as before. The egui console has its own fonts (Hack, Ubuntu),
//! which already cover Latin Extended-A.

use bevy::prelude::*;

/// The bundled font file (about 170 KB).
pub const UI_FONT: &[u8] = include_bytes!("../assets/fonts/FiraMono-Regular.ttf");

/// Puts [`UI_FONT`] under the default font handle.
pub struct UiFontPlugin;

impl Plugin for UiFontPlugin {
    fn build(&self, _app: &mut App) {}

    // `finish` runs after every plugin's `build`, so this overwrites the font
    // Bevy's `TextPlugin` inserted, and before the first update, when the
    // text pipeline registers the font assets (it does not pick up a later
    // replacement under the same id).
    fn finish(&self, app: &mut App) {
        if let Some(mut fonts) = app.world_mut().get_resource_mut::<Assets<Font>>() {
            fonts
                .insert(AssetId::default(), Font::from_bytes(UI_FONT.to_vec()))
                .expect("the default font handle is a valid asset id");
        }
    }
}

/// Whether the bundled font has a glyph for every character of `text`
/// (line breaks need none).
pub fn covers(text: &str) -> bool {
    missing(text).is_empty()
}

/// The characters of `text` the bundled font has no glyph for.
pub fn missing(text: &str) -> Vec<char> {
    let face = ttf_parser::Face::parse(UI_FONT, 0).expect("bundled font parses");
    text.chars()
        .filter(|&c| c != '\n' && face.glyph_index(c).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_street_names_and_ui_symbols() {
        for s in [
            "Bühelstrasse",
            "Schaan–Vaduz",
            "47.1° N",
            "← ↑ → ↓",
            "• ● ·",
            "ÄÖÜäöüß àáâãåæçèéêëìíîïñòóôõøùúûýÿ",
            // Latin Extended-A: Czech, Polish, Hungarian, Croatian, Turkish.
            "Łódź Kraków Žižkov Šibenik Đakovo Győr İstanbul Ćevapi œ Œ",
            "Free ride: Landstrasse | 12.3 km/h",
        ] {
            assert!(covers(s), "{s}: no glyph for {:?}", missing(s));
        }
        // All of Latin-1 and Latin Extended-A (except U+017F long s, which
        // Fira Mono lacks and no current street name uses).
        for c in ('\u{a0}'..='\u{17e}').chain(' '..='~') {
            assert!(covers(&c.to_string()), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn reports_missing_glyphs() {
        // CJK is not in Fira Mono.
        assert_eq!(missing("Vaduz 東京"), vec!['東', '京']);
        assert!(covers("line 1\nline 2"));
    }

    #[test]
    fn plugin_replaces_the_default_font() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Font>()
            .add_plugins(UiFontPlugin);
        app.finish();
        let fonts = app.world().resource::<Assets<Font>>();
        let font = fonts.get(AssetId::default()).expect("default font");
        assert_eq!(font.data.data(), UI_FONT);
    }
}
