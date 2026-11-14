//! Accessibility of what is drawn (spec §3.2.1 M9, §14 Phase 7): the
//! status colours (standard or colour-safe), a high-contrast HUD and
//! reduced motion. The app sets [`Accessibility`] (`ocw_ui` from the
//! settings `display.palette`, `display.high_contrast`,
//! `display.reduced_motion`); the HUD size (`display.ui_scale`) is
//! Bevy's `UiScale`, which scales the HUD's pixel sizes and leaves the
//! menu (sized in % of the window height) as it is.
//!
//! The colours are applied in one place, after every HUD system
//! ([`apply_access`], in `PostUpdate`): each status colour of the
//! standard palette (the damage levels, the voice indicator, the workout
//! zones) is replaced by its colour-safe twin ([`SAFE`]: blue – yellow –
//! orange – reddish purple, after Okabe & Ito, told apart with red-green
//! colour blindness); with high contrast the HUD's dark see-through
//! panels become opaque and its grey text white. The original colours
//! are kept on the entity and come back when the option goes off. The
//! status colours never carry meaning alone: icons and words stay
//! (spec §14 Phase 12F).

use bevy::prelude::*;

/// Which status colours.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Palette {
    #[default]
    Standard,
    /// Distinguishable with protanopia, deuteranopia and tritanopia.
    ColourSafe,
}

impl Palette {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "standard" => Some(Self::Standard),
            "colour-safe" | "color-safe" | "safe" => Some(Self::ColourSafe),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::ColourSafe => "colour-safe",
        }
    }
}

/// The accessibility options in effect.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct Accessibility {
    pub palette: Palette,
    pub high_contrast: bool,
    /// No camera shake, no flashing HUD warnings.
    pub reduced_motion: bool,
}

/// The standard palette's status colours and their colour-safe twins
/// (sRGB 0–1).
pub const SAFE: &[([f32; 3], [f32; 3])] = &[
    // Damage levels (`ocw_sim::damage::Status`): good, warning, serious,
    // critical.
    ([0.42, 0.88, 0.45], [0.34, 0.71, 0.91]),
    ([0.98, 0.76, 0.18], [0.94, 0.89, 0.26]),
    ([0.98, 0.52, 0.12], [0.90, 0.62, 0.0]),
    ([0.92, 0.22, 0.20], [0.80, 0.47, 0.65]),
    // The voice indicator: live, muted.
    ([0.45, 1.0, 0.55], [0.34, 0.71, 0.91]),
    ([1.0, 0.35, 0.3], [0.80, 0.47, 0.65]),
    // Workout zones: endurance, tempo, threshold, VO2max (anaerobic is
    // the critical red above).
    ([0.25, 0.55, 0.95], [0.0, 0.45, 0.70]),
    ([0.25, 0.78, 0.40], [0.34, 0.71, 0.91]),
    ([0.98, 0.80, 0.20], [0.94, 0.89, 0.26]),
    ([0.98, 0.52, 0.15], [0.90, 0.62, 0.0]),
];

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 2e-3)
}

/// `rgb` in `palette`: a standard status colour's twin, else itself.
pub fn status_rgb(rgb: [f32; 3], palette: Palette) -> [f32; 3] {
    match palette {
        Palette::Standard => rgb,
        Palette::ColourSafe => SAFE
            .iter()
            .find(|(std, _)| close(*std, rgb))
            .map_or(rgb, |(_, safe)| *safe),
    }
}

/// A colour as the options draw it (pure; `text`: a text colour).
pub fn transform(c: Color, a: &Accessibility, text: bool) -> Color {
    let s = c.to_srgba();
    let rgb = status_rgb([s.red, s.green, s.blue], a.palette);
    let mut out = Srgba::new(rgb[0], rgb[1], rgb[2], s.alpha);
    if a.high_contrast {
        let grey = (s.red - s.green).abs() < 1e-3 && (s.green - s.blue).abs() < 1e-3;
        if text && grey && (0.55..0.97).contains(&s.red) {
            // Grey text: white.
            out = Srgba::new(1.0, 1.0, 1.0, s.alpha.max(0.95));
        } else if !text && s.red.max(s.green).max(s.blue) < 0.12 && (0.2..0.85).contains(&s.alpha)
        {
            // A dark see-through panel: opaque.
            out.alpha = 0.92;
        }
    }
    Color::Srgba(out)
}

/// An entity the options leave alone (the menu's dimming veil).
#[derive(Component)]
pub struct AccessExempt;

/// The colours an entity had before the options changed them.
#[derive(Component)]
struct BaseBg(Color);
#[derive(Component)]
struct BaseText(Color);
#[derive(Component)]
struct BaseBorder(BorderColor);

pub struct AccessPlugin;

impl Plugin for AccessPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Accessibility>()
            .add_systems(PostUpdate, apply_access);
    }
}

fn border_map(b: &BorderColor, f: impl Fn(Color) -> Color) -> BorderColor {
    BorderColor {
        top: f(b.top),
        right: f(b.right),
        bottom: f(b.bottom),
        left: f(b.left),
    }
}

#[allow(clippy::type_complexity)] // a Bevy system
fn apply_access(
    mut commands: Commands,
    access: Res<Accessibility>,
    mut last: Local<Accessibility>,
    mut bgs: Query<(Entity, &mut BackgroundColor, Option<&BaseBg>), Without<AccessExempt>>,
    mut texts: Query<(Entity, &mut TextColor, Option<&BaseText>), Without<AccessExempt>>,
    mut borders: Query<(Entity, &mut BorderColor, Option<&BaseBorder>), Without<AccessExempt>>,
) {
    let changed = *access != *last;
    *last = *access;
    if changed {
        // Back to the original colours, then the new options.
        for (e, mut bg, base) in &mut bgs {
            if let Some(b) = base {
                bg.0 = b.0;
                commands.entity(e).remove::<BaseBg>();
            }
        }
        for (e, mut t, base) in &mut texts {
            if let Some(b) = base {
                t.0 = b.0;
                commands.entity(e).remove::<BaseText>();
            }
        }
        for (e, mut c, base) in &mut borders {
            if let Some(b) = base {
                *c = b.0;
                commands.entity(e).remove::<BaseBorder>();
            }
        }
    }
    if *access == Accessibility::default() {
        return;
    }
    let a = *access;
    for (e, mut bg, base) in &mut bgs {
        let to = transform(bg.0, &a, false);
        if to != bg.0 {
            if base.is_none() || changed {
                commands.entity(e).insert(BaseBg(bg.0));
            }
            bg.0 = to;
        }
    }
    for (e, mut t, base) in &mut texts {
        let to = transform(t.0, &a, true);
        if to != t.0 {
            if base.is_none() || changed {
                commands.entity(e).insert(BaseText(t.0));
            }
            t.0 = to;
        }
    }
    if a.palette != Palette::Standard {
        for (e, mut c, base) in &mut borders {
            let to = border_map(&c, |x| transform(x, &Accessibility { high_contrast: false, ..a }, false));
            if to != *c {
                if base.is_none() || changed {
                    commands.entity(e).insert(BaseBorder(*c));
                }
                *c = to;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_safe_twins_replace_the_standard_status_colours() {
        use ocw_sim::damage::Status;
        let all = [Status::Good, Status::Warning, Status::Serious, Status::Critical];
        let safe: Vec<[f32; 3]> = all
            .iter()
            .map(|s| status_rgb(s.rgb(), Palette::ColourSafe))
            .collect();
        for (s, c) in all.iter().zip(&safe) {
            assert!(!close(s.rgb(), *c), "{s:?} has a twin");
            assert_eq!(status_rgb(s.rgb(), Palette::Standard), s.rgb());
        }
        // Good and critical (green and red before) differ in the blue
        // channel and in lightness now: told apart without red-green.
        assert!((safe[0][2] - safe[3][2]).abs() > 0.2);
        // Twins are not standard colours themselves (idempotent).
        for (_, twin) in SAFE {
            assert_eq!(status_rgb(*twin, Palette::ColourSafe), *twin);
        }
        // Other colours stay.
        assert_eq!(status_rgb([0.1, 0.2, 0.3], Palette::ColourSafe), [0.1, 0.2, 0.3]);
        assert_eq!(Palette::parse("Colour-Safe"), Some(Palette::ColourSafe));
        assert_eq!(Palette::parse("color-safe"), Some(Palette::ColourSafe));
        assert_eq!(Palette::parse("x"), None);
    }

    #[test]
    fn high_contrast_makes_panels_opaque_and_grey_text_white() {
        let hc = Accessibility {
            high_contrast: true,
            ..Default::default()
        };
        let panel = Color::srgba(0.0, 0.0, 0.0, 0.55);
        assert_eq!(transform(panel, &hc, false).alpha(), 0.92);
        assert_eq!(transform(transform(panel, &hc, false), &hc, false), transform(panel, &hc, false));
        let grey = Color::srgb(0.85, 0.85, 0.85);
        assert_eq!(transform(grey, &hc, true), Color::srgba(1.0, 1.0, 1.0, 1.0));
        // Light rows, coloured text and the opaque menu panel stay.
        let row = Color::srgba(1.0, 1.0, 1.0, 0.07);
        assert_eq!(transform(row, &hc, false), row);
        let gold = Color::srgb(1.0, 0.78, 0.24);
        assert_eq!(transform(gold, &hc, true), gold);
        let menu = Color::srgba(0.02, 0.03, 0.05, 0.88);
        assert_eq!(transform(menu, &hc, false), menu);
        // Off: nothing changes.
        assert_eq!(transform(panel, &Accessibility::default(), false), panel);
    }

    #[test]
    fn the_options_apply_and_come_back_off() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AccessPlugin));
        let red = Color::srgb(0.92, 0.22, 0.20);
        let panel = app
            .world_mut()
            .spawn((BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)), TextColor(red)))
            .id();
        let veil = app
            .world_mut()
            .spawn((BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)), AccessExempt))
            .id();
        app.update();
        let get = |app: &App, e| {
            (
                app.world().get::<BackgroundColor>(e).unwrap().0,
                app.world().get::<TextColor>(e).map(|t| t.0),
            )
        };
        assert_eq!(get(&app, panel).0.alpha(), 0.55);
        *app.world_mut().resource_mut::<Accessibility>() = Accessibility {
            palette: Palette::ColourSafe,
            high_contrast: true,
            reduced_motion: false,
        };
        app.update();
        let (bg, text) = get(&app, panel);
        assert_eq!(bg.alpha(), 0.92);
        let t = text.unwrap().to_srgba();
        assert!(close([t.red, t.green, t.blue], [0.80, 0.47, 0.65]));
        assert_eq!(get(&app, veil).0.alpha(), 0.45, "the menu's veil stays");
        app.update();
        assert_eq!(get(&app, panel).0.alpha(), 0.92, "stable");
        *app.world_mut().resource_mut::<Accessibility>() = Accessibility::default();
        app.update();
        let (bg, text) = get(&app, panel);
        assert_eq!(bg.alpha(), 0.55);
        assert_eq!(text, Some(red));
    }
}
