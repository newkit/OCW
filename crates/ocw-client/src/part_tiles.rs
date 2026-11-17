//! The bike stand's part tiles (spec §14 Phase 4, 4B and 4G *Tiles*):
//! every selectable item of a slot – each part and each paint – as a tile
//! with a small rendering of the part (`ocw_render::thumbs`) and its gain
//! in %, on a grid the focus moves over with the keyboard, the gamepad,
//! the mouse and touch. Bevy-free and wasm-safe: the grid's navigation,
//! what a tile says and the bike's line, shared by both apps.

use ocw_sim::PhysicsMode;
use ocw_sim::build::{Build, PartGain, gain_pair};

/// Tiles per row in the picker panel (40 % of the width: three tiles of
/// about 21 % of the height each, ten-foot sizes).
pub const COLUMNS: usize = 3;
/// Tile rows shown at once (the detail card below them).
pub const VISIBLE_ROWS: usize = 2;

/// A direction on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The focus after a move on a grid of `len` tiles in `cols` columns:
/// ← / → one tile (on to the previous / next row at a row's end), ↑ / ↓
/// one row in the same column (to the last tile when the last row is
/// short); the focus stops at the first and the last tile.
pub fn step(focus: usize, len: usize, cols: usize, dir: Dir) -> usize {
    if len == 0 {
        return 0;
    }
    let cols = cols.max(1);
    let focus = focus.min(len - 1);
    match dir {
        Dir::Left => focus.saturating_sub(1),
        Dir::Right => (focus + 1).min(len - 1),
        Dir::Up => focus.checked_sub(cols).unwrap_or(focus),
        Dir::Down => {
            let last_row = (len - 1) / cols;
            if focus / cols == last_row {
                focus
            } else {
                (focus + cols).min(len - 1)
            }
        }
    }
}

/// The first tile row shown so that `focus` is among the `rows` rows on
/// screen (the grid scrolls by whole rows; the last rows fill the view).
pub fn first_row(focus: usize, len: usize, cols: usize, rows: usize) -> usize {
    let cols = cols.max(1);
    let total = len.div_ceil(cols);
    let at = focus.min(len.saturating_sub(1)) / cols;
    let rows = rows.max(1);
    at.saturating_sub(rows - 1)
        .min(total.saturating_sub(rows))
        .min(at)
}

/// Where parts count (spec 4G): the race modes where they are cosmetic.
pub const COSMETIC_IN_RACES: &str = "no effect in Profile / Equal races";

/// What the tiles say about parts in the race mode in force: `None` in
/// *Open* (free ride, workouts, Open races), else the words for the card
/// ("no effect in this Profile race").
pub fn cosmetic_now(mode: PhysicsMode) -> Option<String> {
    match mode {
        PhysicsMode::Open => None,
        m => Some(format!("no effect in this {} race", m.name())),
    }
}

/// A part tile's text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TileText {
    /// The part's name.
    pub title: String,
    /// "equipped", "owned", "164 ♥", "base part", "free" …
    pub badge: String,
    /// "+0.8 % flat · −0.2 % climb" (· capped), or the race note.
    pub gain: String,
    /// "−240 g" against the slot's base part.
    pub weight: String,
    /// The caps cut this part's effect on the bike (the line says it).
    pub capped: bool,
    /// The shop's lock: the balance is too low.
    pub locked: bool,
}

/// The text of a part tile: its gain against the slot's base part
/// (`None`: no physics, e.g. a paint), in the race mode in force.
pub fn tile_text(
    title: &str,
    badge: &str,
    locked: bool,
    gain: Option<&PartGain>,
    mode: PhysicsMode,
) -> TileText {
    let (gain_line, weight, capped) = match gain {
        Some(g) => (
            cosmetic_now(mode).unwrap_or_else(|| g.line()),
            g.grams(),
            g.capped && mode == PhysicsMode::Open,
        ),
        None => ("looks only".to_string(), String::new(), false),
    };
    TileText {
        title: title.to_string(),
        badge: badge.to_string(),
        gain: gain_line,
        weight,
        capped,
        locked,
    }
}

/// The whole build against the stock bike, for the picker's head line:
/// "This bike: +1.2 % flat · +1.9 % climb vs stock · 7.42 kg (−780 g),
/// capped".
pub fn bike_gain_line(build: &Build) -> String {
    let stock = Build::stock(build.profile);
    let g = ((build.params.bike_mass_kg - stock.params.bike_mass_kg) * 1000.0).round() as i64;
    let grams = match g {
        0 => "stock weight".into(),
        g => ocw_sim::build::signed_grams(g),
    };
    let mut s = format!(
        "This bike: {} vs stock · {:.2} kg ({grams})",
        gain_pair(build.flat_gain, build.climb_gain),
        build.params.bike_mass_kg
    );
    if build.capped() {
        s.push_str(", capped");
    }
    if build.mass_floored {
        s.push_str(", on the weight floor");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::ProfileId;
    use ocw_sim::PartTable;
    use ocw_sim::build::{build, part_gain};

    /// The grid with each direction: ← / → wrap rows, ↑ / ↓ keep the
    /// column, the edges hold.
    #[test]
    fn the_focus_moves_over_the_grid() {
        // 3 columns, 8 tiles: rows [0 1 2] [3 4 5] [6 7].
        let s = |f, d| step(f, 8, 3, d);
        assert_eq!(s(0, Dir::Left), 0);
        assert_eq!(s(0, Dir::Right), 1);
        assert_eq!(s(2, Dir::Right), 3);
        assert_eq!(s(3, Dir::Left), 2);
        assert_eq!(s(7, Dir::Right), 7);
        assert_eq!(s(1, Dir::Down), 4);
        assert_eq!(s(4, Dir::Down), 7);
        // The short last row: down from 5 goes to the last tile.
        assert_eq!(s(5, Dir::Down), 7);
        assert_eq!(s(7, Dir::Down), 7);
        assert_eq!(s(1, Dir::Up), 1);
        assert_eq!(s(7, Dir::Up), 4);
        // A focus beyond the end (the list got shorter) comes back.
        assert_eq!(step(12, 8, 3, Dir::Left), 6);
        assert_eq!(step(0, 0, 3, Dir::Down), 0);
        // One column behaves like a list.
        assert_eq!(step(2, 5, 1, Dir::Down), 3);
        assert_eq!(step(2, 5, 1, Dir::Up), 1);
    }

    #[test]
    fn the_grid_scrolls_by_rows() {
        // 3 columns, 11 tiles: 4 rows; 2 shown.
        assert_eq!(first_row(0, 11, 3, 2), 0);
        assert_eq!(first_row(4, 11, 3, 2), 0);
        assert_eq!(first_row(6, 11, 3, 2), 1);
        assert_eq!(first_row(10, 11, 3, 2), 2);
        assert_eq!(first_row(2, 3, 3, 2), 0);
        assert_eq!(first_row(0, 0, 3, 2), 0);
    }

    /// A tile in free ride shows the gain; in a Profile or Equal race the
    /// parts are cosmetic and the tile says so.
    #[test]
    fn tiles_per_race_mode() {
        let g = part_gain(
            PartTable::shipped(),
            ProfileId::Road,
            &[],
            "wheels.road-carbon.top",
        )
        .unwrap();
        let open = tile_text(
            "Top carbon wheels",
            "3,900 ♥",
            false,
            Some(&g),
            PhysicsMode::Open,
        );
        assert!(
            open.gain.contains(" flat · ") && open.gain.ends_with(" climb"),
            "{}",
            open.gain
        );
        assert!(open.gain.starts_with('+'), "{}", open.gain);
        assert_eq!(open.weight, "−650 g");
        let profile = tile_text("x", "", false, Some(&g), PhysicsMode::Profile);
        assert_eq!(profile.gain, "no effect in this Profile race");
        assert_eq!(profile.weight, "−650 g");
        let equal = tile_text("x", "", false, Some(&g), PhysicsMode::Equal);
        assert_eq!(equal.gain, "no effect in this Equal race");
        assert_eq!(cosmetic_now(PhysicsMode::Open), None);
        // A paint: no physics.
        let paint = tile_text("Paint", "free", false, None, PhysicsMode::Open);
        assert_eq!(paint.gain, "looks only");
        assert!(paint.weight.is_empty());
    }

    #[test]
    fn the_bike_line_against_stock() {
        let stock = Build::stock(ProfileId::Road);
        assert_eq!(
            bike_gain_line(&stock),
            format!(
                "This bike: ±0.0 % flat · ±0.0 % climb vs stock · {:.2} kg (stock weight)",
                stock.params.bike_mass_kg
            )
        );
        let t = PartTable::shipped();
        let best = ocw_sim::build::best_parts(t, ProfileId::RoadAero);
        let b = build(t, ProfileId::RoadAero, &best);
        let line = bike_gain_line(&b);
        // The capped best build: faster on both, at most the cap.
        assert!(b.capped());
        assert!(b.flat_gain > 0.0 && b.flat_gain <= ocw_sim::build::FLAT_CAP + 1e-9);
        assert!(line.starts_with("This bike: +1."), "{line}");
        assert!(line.contains(" climb vs stock · "), "{line}");
        assert!(line.contains(" g), capped"), "{line}");
    }
}
