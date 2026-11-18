//! How the maps look: the studio's road styles, the navigation map's day
//! and night palettes (a bike computer's map screen: soft land-use fills,
//! roads by class with a casing, the main roads wider, the route bold),
//! and the scale bar's round lengths.

use ocw_world::graph::RoadClass;
use ocw_world::landuse::LandClass;

/// How a road class is drawn on the studio map: the narrowest stroke
/// (points), its colour, and from which zoom on (metres per point) it is
/// drawn at all.
pub fn road_style(class: RoadClass) -> (f32, [u8; 3], f64) {
    match class {
        RoadClass::Trunk => (3.0, [232, 146, 162], f64::INFINITY),
        RoadClass::Primary => (2.6, [252, 214, 164], f64::INFINITY),
        RoadClass::Secondary => (2.2, [246, 250, 190], f64::INFINITY),
        RoadClass::Tertiary => (1.8, [255, 255, 255], 40.0),
        RoadClass::Unclassified | RoadClass::Residential => (1.3, [250, 250, 250], 15.0),
        RoadClass::Service => (1.0, [235, 235, 235], 6.0),
        RoadClass::Track => (1.0, [172, 131, 62], 10.0),
        RoadClass::Cycleway => (1.0, [80, 80, 250], 10.0),
        RoadClass::Path => (0.8, [250, 128, 114], 6.0),
    }
}

/// The largest 1, 2 or 5 × 10ⁿ metres not above `m`.
pub fn nice_length(m: f64) -> f64 {
    if m.is_nan() || m <= 0.0 {
        return 1.0;
    }
    let p = 10f64.powf(m.log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|k| k * p)
        .find(|v| *v <= m)
        .unwrap_or(p)
}

/// "500 m", "2 km".
pub fn format_length(m: f64) -> String {
    if m >= 1000.0 {
        format!("{} km", m / 1000.0)
    } else {
        format!("{m} m")
    }
}

/// Metres in a mile and in a foot.
pub const METRES_PER_MILE: f64 = 1609.344;
pub const METRES_PER_FOOT: f64 = 0.3048;

/// A scale bar for a map of `m_per_px` at most `max_px` points long: its
/// length in points and its label ("200 m", "0.5 mi", "500 ft").
pub fn scale_bar(m_per_px: f64, max_px: f64, imperial: bool) -> (f64, String) {
    let max_m = m_per_px * max_px;
    if imperial {
        let mi = max_m / METRES_PER_MILE;
        if mi >= 0.1 {
            let v = nice_length(mi);
            return (v * METRES_PER_MILE / m_per_px, format!("{v} mi"));
        }
        let ft = nice_length(max_m / METRES_PER_FOOT);
        return (ft * METRES_PER_FOOT / m_per_px, format!("{ft} ft"));
    }
    let m = nice_length(max_m);
    (m / m_per_px, format_length(m))
}

/// Day or night look of the navigation map (decided by the light at the
/// rider, see [`crate::navi::Daylight`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Shade {
    #[default]
    Day,
    Night,
}

/// An RGBA colour (unmultiplied, sRGB).
pub type Rgba = [u8; 4];

/// The navigation map's colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Open ground (no land use) and outside the world.
    pub ground: Rgba,
    pub outside: Rgba,
    pub forest: Rgba,
    pub meadow: Rgba,
    pub farmland: Rgba,
    pub settled: Rgba,
    pub industrial: Rgba,
    pub park: Rgba,
    pub water: Rgba,
    pub rock: Rgba,
    pub glacier: Rgba,
    pub building: Rgba,
    pub building_edge: Rgba,
    /// Road fill: main roads (trunk, primary, secondary), minor roads,
    /// tracks and paths; the casing around them.
    pub road_main: Rgba,
    pub road_minor: Rgba,
    pub road_track: Rgba,
    pub cycleway: Rgba,
    pub casing: Rgba,
    /// The route ahead (the planned path), its casing, the route behind,
    /// the other exits of the next junction.
    pub route: Rgba,
    pub route_casing: Rgba,
    pub route_behind: Rgba,
    pub exit: Rgba,
    /// Other riders' dots and their ring.
    pub rider_dot: Rgba,
    pub rider_ring: Rgba,
}

impl Palette {
    pub fn of(shade: Shade) -> Self {
        match shade {
            Shade::Day => Self {
                ground: [238, 236, 226, 255],
                outside: [214, 212, 204, 255],
                forest: [190, 218, 172, 255],
                meadow: [222, 236, 202, 255],
                farmland: [236, 234, 206, 255],
                settled: [228, 222, 216, 255],
                industrial: [222, 214, 222, 255],
                park: [204, 232, 186, 255],
                water: [160, 200, 236, 255],
                rock: [222, 216, 206, 255],
                glacier: [236, 244, 250, 255],
                building: [206, 196, 186, 255],
                building_edge: [184, 172, 162, 255],
                road_main: [255, 214, 120, 255],
                road_minor: [255, 255, 255, 255],
                road_track: [200, 176, 140, 255],
                cycleway: [150, 170, 230, 255],
                casing: [150, 146, 140, 255],
                route: [214, 38, 160, 255],
                route_casing: [255, 255, 255, 255],
                route_behind: [214, 38, 160, 110],
                exit: [92, 112, 140, 255],
                rider_dot: [30, 140, 230, 255],
                rider_ring: [255, 255, 255, 255],
            },
            Shade::Night => Self {
                ground: [34, 38, 44, 255],
                outside: [22, 24, 28, 255],
                forest: [30, 50, 38, 255],
                meadow: [38, 46, 40, 255],
                farmland: [44, 46, 40, 255],
                settled: [46, 46, 52, 255],
                industrial: [50, 44, 54, 255],
                park: [32, 54, 40, 255],
                water: [24, 50, 82, 255],
                rock: [52, 50, 48, 255],
                glacier: [70, 80, 90, 255],
                building: [70, 70, 78, 255],
                building_edge: [86, 86, 96, 255],
                road_main: [214, 160, 70, 255],
                road_minor: [150, 154, 162, 255],
                road_track: [110, 98, 80, 255],
                cycleway: [90, 110, 170, 255],
                casing: [16, 18, 22, 255],
                route: [255, 80, 200, 255],
                route_casing: [20, 20, 26, 255],
                route_behind: [255, 80, 200, 110],
                exit: [150, 170, 200, 255],
                rider_dot: [90, 190, 255, 255],
                rider_ring: [16, 18, 22, 255],
            },
        }
    }

    /// The fill of a land-use class (`None`: the open ground).
    pub fn land(&self, class: LandClass) -> Option<Rgba> {
        Some(match class {
            LandClass::Default => return None,
            LandClass::Meadow | LandClass::Scrub => self.meadow,
            LandClass::Farmland | LandClass::Orchard | LandClass::Vineyard => self.farmland,
            LandClass::Forest => self.forest,
            LandClass::Residential | LandClass::Commercial => self.settled,
            LandClass::Industrial | LandClass::Bare => self.industrial,
            LandClass::Park => self.park,
            LandClass::Water | LandClass::Wetland => self.water,
            LandClass::Rock | LandClass::Scree => self.rock,
            LandClass::Glacier => self.glacier,
        })
    }
}

/// How the navigation map draws a road class at `m_per_px` (metres per
/// map point): `None` when it is left out at this zoom, else the fill
/// width (points), the fill colour and the draw order (higher on top).
pub fn navi_road(class: RoadClass, m_per_px: f64, p: &Palette) -> Option<(f32, Rgba, u8)> {
    // The widths grow a little closer in, as on a bike computer.
    let near = (2.5 / m_per_px).clamp(0.6, 2.0) as f32;
    let (w, c, order, far) = match class {
        RoadClass::Trunk => (5.5, p.road_main, 9, f64::INFINITY),
        RoadClass::Primary => (5.0, p.road_main, 8, f64::INFINITY),
        RoadClass::Secondary => (4.4, p.road_main, 7, f64::INFINITY),
        RoadClass::Tertiary => (3.8, p.road_minor, 6, 12.0),
        RoadClass::Unclassified | RoadClass::Residential => (3.2, p.road_minor, 5, 6.0),
        RoadClass::Service => (2.2, p.road_minor, 4, 2.5),
        RoadClass::Cycleway => (2.2, p.cycleway, 3, 4.0),
        RoadClass::Track => (1.8, p.road_track, 2, 4.0),
        RoadClass::Path => (1.4, p.road_track, 1, 2.0),
    };
    (m_per_px <= far).then_some((w * near, c, order))
}

/// Buildings are drawn from this zoom in (metres per map point).
pub const NAVI_BUILDINGS_BELOW_M_PER_PX: f64 = 2.2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_bar_lengths_are_round() {
        assert_eq!(nice_length(730.0), 500.0);
        assert_eq!(nice_length(2100.0), 2000.0);
        assert_eq!(nice_length(1.0), 1.0);
        assert_eq!(nice_length(0.0), 1.0);
        assert_eq!(format_length(500.0), "500 m");
        assert_eq!(format_length(2000.0), "2 km");
        assert!(road_style(RoadClass::Primary).2.is_infinite());
        let (px, label) = scale_bar(2.0, 80.0, false);
        assert_eq!((px, label.as_str()), (50.0, "100 m"));
        let (px, label) = scale_bar(20.0, 80.0, true);
        assert_eq!(label, "0.5 mi");
        assert!((px - 0.5 * METRES_PER_MILE / 20.0).abs() < 1e-9);
        assert_eq!(scale_bar(1.0, 80.0, true).1, "200 ft");
    }

    #[test]
    fn main_roads_are_wider_and_minor_ones_leave_far_out() {
        let p = Palette::of(Shade::Day);
        let w = |c, m| navi_road(c, m, &p).map(|r| r.0);
        assert!(w(RoadClass::Primary, 2.0) > w(RoadClass::Residential, 2.0));
        assert!(w(RoadClass::Residential, 2.0) > w(RoadClass::Path, 2.0));
        assert!(w(RoadClass::Residential, 10.0).is_none());
        assert!(w(RoadClass::Primary, 40.0).is_some());
        let (_, _, o1) = navi_road(RoadClass::Trunk, 1.0, &p).unwrap();
        let (_, _, o2) = navi_road(RoadClass::Track, 1.0, &p).unwrap();
        assert!(o1 > o2, "main roads on top");
        // The night palette is dark.
        let n = Palette::of(Shade::Night);
        let lum = |c: Rgba| u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]);
        assert!(lum(n.ground) * 4 < lum(p.ground));
        assert_eq!(p.land(LandClass::Default), None);
        assert_eq!(p.land(LandClass::Water), Some(p.water));
    }
}
