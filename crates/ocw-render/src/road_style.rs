//! Regional road marking styles (spec §14 Phase 8 T5):
//! `assets/road-styles.toml` (built in), chosen per package by its time zone
//! like the material sets and house styles. The layout itself is
//! `ocw_world::road_dress::marking_lines`; the shader gets a style as its
//! [`crate::road_material::RoadLook`].

use std::sync::OnceLock;

use ocw_world::road_dress::{CycleLane, LineSpec, MarkingStyle};
use serde::Deserialize;

/// The built-in style file.
pub const STYLES_TOML: &str = include_str!("../../../assets/road-styles.toml");

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct LineFile {
    width: f32,
    dash: f32,
    gap: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleFile {
    id: String,
    #[allow(dead_code)] // documentation in the file
    name: String,
    #[serde(default)]
    time_zones: Vec<String>,
    centre: Option<LineFile>,
    centre_town: Option<LineFile>,
    double_gap: Option<f32>,
    lane: Option<LineFile>,
    lane_town: Option<LineFile>,
    edge_width: Option<f32>,
    edge_inset: Option<f32>,
    cycle_lane: Option<f32>,
    cycle_advisory: Option<LineFile>,
    cycle_exclusive: Option<f32>,
    cycle_default: Option<String>,
    zebra_bar: Option<f32>,
    zebra_gap: Option<f32>,
    zebra_length: Option<f32>,
    stop: Option<f32>,
    give_way: Option<LineFile>,
    signals: Option<LineFile>,
    signals_spacing: Option<f32>,
    stop_before: Option<f32>,
    line_colour: Option<String>,
    cycle_colour: Option<String>,
    crossing_colour: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    style: Vec<StyleFile>,
}

fn line(l: Option<LineFile>, base: LineSpec, errors: &mut Vec<String>, what: &str) -> LineSpec {
    let Some(l) = l else {
        return base;
    };
    if !(l.width > 0.0 && l.width < 1.0 && l.dash > 0.0 && l.gap >= 0.0) {
        errors.push(format!("{what}: bad line {l:?}"));
    }
    LineSpec::dashed(l.width, l.dash, l.gap)
}

fn colour(c: Option<&String>, base: [f32; 3], errors: &mut Vec<String>, what: &str) -> [f32; 3] {
    match c {
        None => base,
        Some(c) => match crate::house_style::parse_hex(c) {
            Some(rgb) => crate::house_style::linear(rgb),
            None => {
                errors.push(format!("{what}: bad colour {c}"));
                base
            }
        },
    }
}

/// Parses and checks a style file; later styles fall back to the first,
/// the first to [`MarkingStyle::default`].
pub fn parse(text: &str) -> Result<Vec<MarkingStyle>, Vec<String>> {
    let file: File = toml::from_str(text).map_err(|e| vec![e.to_string()])?;
    let mut errors = Vec::new();
    let mut out: Vec<MarkingStyle> = Vec::new();
    for (i, f) in file.style.into_iter().enumerate() {
        let base = out.first().cloned().unwrap_or_default();
        if i > 0 && f.time_zones.is_empty() {
            errors.push(format!("style {}: needs time_zones", f.id));
        }
        let e = &mut errors;
        let num = |v: Option<f32>, b: f32, e: &mut Vec<String>, what: &str| {
            let v = v.unwrap_or(b);
            if !(0.0..=50.0).contains(&v) {
                e.push(format!("{}: {what} {v} out of range", f.id));
            }
            v
        };
        let cycle_default = match f.cycle_default.as_deref() {
            None => base.cycle_default,
            Some("advisory") => CycleLane::Advisory,
            Some("exclusive") => CycleLane::Exclusive,
            Some(x) => {
                e.push(format!("{}: unknown cycle_default {x}", f.id));
                base.cycle_default
            }
        };
        let s = MarkingStyle {
            id: f.id.clone(),
            time_zones: f.time_zones.clone(),
            centre: line(f.centre, base.centre, e, "centre"),
            centre_town: line(f.centre_town, base.centre_town, e, "centre_town"),
            double_gap_m: num(f.double_gap, base.double_gap_m, e, "double_gap"),
            lane: line(f.lane, base.lane, e, "lane"),
            lane_town: line(f.lane_town, base.lane_town, e, "lane_town"),
            edge_width_m: num(f.edge_width, base.edge_width_m, e, "edge_width"),
            edge_inset_m: num(f.edge_inset, base.edge_inset_m, e, "edge_inset"),
            cycle_lane_m: num(f.cycle_lane, base.cycle_lane_m, e, "cycle_lane"),
            cycle_advisory: line(f.cycle_advisory, base.cycle_advisory, e, "cycle_advisory"),
            cycle_exclusive_m: num(
                f.cycle_exclusive,
                base.cycle_exclusive_m,
                e,
                "cycle_exclusive",
            ),
            cycle_default,
            zebra_bar_m: num(f.zebra_bar, base.zebra_bar_m, e, "zebra_bar"),
            zebra_gap_m: num(f.zebra_gap, base.zebra_gap_m, e, "zebra_gap"),
            zebra_length_m: num(f.zebra_length, base.zebra_length_m, e, "zebra_length"),
            stop_m: num(f.stop, base.stop_m, e, "stop"),
            give_way: line(f.give_way, base.give_way, e, "give_way"),
            signals: line(f.signals, base.signals, e, "signals"),
            signals_spacing_m: num(
                f.signals_spacing,
                base.signals_spacing_m,
                e,
                "signals_spacing",
            ),
            stop_before_m: num(f.stop_before, base.stop_before_m, e, "stop_before"),
            line_colour: colour(f.line_colour.as_ref(), base.line_colour, e, "line_colour"),
            cycle_colour: colour(
                f.cycle_colour.as_ref(),
                base.cycle_colour,
                e,
                "cycle_colour",
            ),
            crossing_colour: colour(
                f.crossing_colour.as_ref(),
                base.crossing_colour,
                e,
                "crossing_colour",
            ),
        };
        out.push(s);
    }
    if out.is_empty() {
        errors.push("no styles".into());
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// The built-in styles (the first is the default).
pub fn styles() -> &'static [MarkingStyle] {
    static STYLES: OnceLock<Vec<MarkingStyle>> = OnceLock::new();
    STYLES.get_or_init(|| parse(STYLES_TOML).unwrap_or_else(|e| panic!("road-styles.toml: {e:?}")))
}

/// The index of the style for a package's time zone.
pub fn style_index(time_zone: Option<&str>) -> usize {
    let Some(tz) = time_zone else {
        return 0;
    };
    styles()
        .iter()
        .position(|s| s.time_zones.iter().any(|z| z == tz))
        .unwrap_or(0)
}

/// The style for a package's time zone.
pub fn style_for(time_zone: Option<&str>) -> &'static MarkingStyle {
    &styles()[style_index(time_zone)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_styles_parse_and_are_chosen_by_region() {
        let s = styles();
        assert_eq!(s[0].id, "de");
        assert_eq!(style_for(None).id, "de");
        assert_eq!(style_for(Some("Europe/Berlin")).id, "de");
        assert_eq!(style_for(Some("Europe/Vaduz")).id, "ch");
        assert_eq!(style_for(Some("Europe/Zurich")).id, "ch");
        assert_eq!(style_for(Some("Atlantic/Canary")).id, "es");
        assert_eq!(style_for(Some("Pacific/Auckland")).id, "de");
        // German rules: 12 cm lines, 6 / 12 m outside, 3 / 6 m in towns.
        assert_eq!(s[0].centre, LineSpec::dashed(0.12, 6.0, 12.0));
        assert_eq!(s[0].centre_town, LineSpec::dashed(0.12, 3.0, 6.0));
        // Swiss zebras are yellow (more red and green than blue), the
        // German ones white; fields not given fall back to the first style.
        let ch = style_for(Some("Europe/Vaduz"));
        let y = ch.crossing_colour;
        assert!(y[0] > 0.5 && y[1] > 0.3 && y[2] < 0.1, "{y:?}");
        let w = s[0].crossing_colour;
        assert!((w[0] - w[2]).abs() < 0.05);
        assert_eq!(ch.stop_m, s[0].stop_m);
        assert_eq!(ch.cycle_default, CycleLane::Advisory);
        let es = style_for(Some("Atlantic/Canary"));
        assert!(es.edge_width_m > es.centre.width);
    }

    #[test]
    fn bad_style_files_are_refused() {
        assert!(parse("").is_err());
        let bad = "[[style]]\nid = \"a\"\nname = \"A\"\nline_colour = \"#zz\"\n";
        assert!(parse(bad).unwrap_err()[0].contains("colour"));
        let two = "[[style]]\nid = \"a\"\nname = \"A\"\n[[style]]\nid = \"b\"\nname = \"B\"\n";
        assert!(parse(two).unwrap_err()[0].contains("time_zones"));
        let line = "[[style]]\nid = \"a\"\nname = \"A\"\ncentre = { width = 0.0, dash = 1.0, gap = 1.0 }\n";
        assert!(parse(line).is_err());
        let unknown = "[[style]]\nid = \"a\"\nname = \"A\"\ncolour = \"#fff\"\n";
        assert!(parse(unknown).is_err());
    }
}
