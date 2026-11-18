//! What the navigation map shows and how it zooms (pure, tested): the
//! zoom levels and the auto zoom (by speed, closer at junctions), the
//! next-turn banner ("↱ 250 m Street name"), the day / night look by the
//! light at the rider, and the route layer's lines (the planned path
//! ahead bold, the path behind dimmer, the next junction's other exits
//! thin).

use ocw_world::Track;

use crate::raster::MapLine;
use crate::style::{METRES_PER_FOOT, METRES_PER_MILE, Palette, Shade};

/// The zoom levels: metres per map point (a 240-point map shows from
/// about 150 m to 2.4 km across).
pub const ZOOM_LEVELS: [f64; 7] = [0.6, 1.0, 1.6, 2.5, 4.0, 6.5, 10.0];

/// Auto zoom shows at least this far ahead (m) …
pub const MIN_AHEAD_M: f64 = 120.0;
/// … and the road of this many seconds at the rider's speed.
pub const AHEAD_S: f64 = 25.0;
/// A junction closer than this (m) zooms in on it.
pub const JUNCTION_ZOOM_M: f64 = 250.0;
/// Room beyond the junction (m) kept in view.
pub const JUNCTION_BEYOND_M: f64 = 80.0;
/// Auto zoom zooms out only after wanting to for this long (s); it zooms
/// in at once.
pub const ZOOM_OUT_HOLD_S: f64 = 2.5;

/// The level that shows `need_m` over `ahead_px` map points (the room
/// from the rider to the top): the closest one that fits, else the
/// farthest.
pub fn level_for(need_m: f64, ahead_px: f64) -> usize {
    ZOOM_LEVELS
        .iter()
        .position(|m| m * ahead_px.max(1.0) >= need_m)
        .unwrap_or(ZOOM_LEVELS.len() - 1)
}

/// The level by speed alone.
pub fn speed_level(speed_mps: f64, ahead_px: f64) -> usize {
    level_for(MIN_AHEAD_M.max(speed_mps.max(0.0) * AHEAD_S), ahead_px)
}

/// The level for a junction `distance_m` ahead (negative: just passed),
/// if it is close enough to zoom in on.
pub fn junction_level(distance_m: f64, ahead_px: f64) -> Option<usize> {
    (-20.0..=JUNCTION_ZOOM_M)
        .contains(&distance_m)
        .then(|| level_for(distance_m.max(0.0) + JUNCTION_BEYOND_M, ahead_px))
}

/// How the zoom is chosen (setting `display.map_zoom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoomMode {
    #[default]
    Auto,
    /// A fixed level (index into [`ZOOM_LEVELS`]).
    Fixed(usize),
}

impl ZoomMode {
    /// "auto", "1" … "7".
    pub fn parse(s: &str) -> Option<Self> {
        if s == "auto" {
            return Some(Self::Auto);
        }
        let n: usize = s.parse().ok()?;
        (1..=ZOOM_LEVELS.len())
            .contains(&n)
            .then(|| Self::Fixed(n - 1))
    }

    pub fn label(self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Fixed(i) => (i + 1).to_string(),
        }
    }

    /// The next mode of the map's zoom key: auto, then 1 (closest) … 7,
    /// then auto again.
    pub fn cycle(self) -> Self {
        match self {
            Self::Auto => Self::Fixed(0),
            Self::Fixed(i) if i + 1 < ZOOM_LEVELS.len() => Self::Fixed(i + 1),
            Self::Fixed(_) => Self::Auto,
        }
    }

    /// One step closer (`in_`) or farther; from auto it starts at `now`.
    pub fn step(self, in_: bool, now: usize) -> Self {
        let i = match self {
            Self::Auto => now,
            Self::Fixed(i) => i,
        };
        let j = if in_ {
            i.saturating_sub(1)
        } else {
            (i + 1).min(ZOOM_LEVELS.len() - 1)
        };
        Self::Fixed(j)
    }
}

/// The auto zoom with its hold before zooming out.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AutoZoom {
    pub level: usize,
    /// The farther level wanted, and since how long (s).
    wanted: Option<(usize, f64)>,
    started: bool,
}

impl AutoZoom {
    /// The level after `dt` s at `speed_mps` with a junction `junction_m`
    /// ahead (if any).
    pub fn update(
        &mut self,
        speed_mps: f64,
        junction_m: Option<f64>,
        ahead_px: f64,
        dt: f64,
    ) -> usize {
        let by_speed = speed_level(speed_mps, ahead_px);
        let target = junction_m
            .and_then(|d| junction_level(d, ahead_px))
            .map_or(by_speed, |j| j.min(by_speed));
        if !self.started {
            self.started = true;
            self.level = target;
        } else if target < self.level {
            self.level = target;
            self.wanted = None;
        } else if target > self.level {
            let held = match self.wanted {
                Some((l, t)) if l == target => t + dt,
                _ => dt,
            };
            if held >= ZOOM_OUT_HOLD_S {
                self.level = target;
                self.wanted = None;
            } else {
                self.wanted = Some((target, held));
            }
        } else {
            self.wanted = None;
        }
        self.level
    }
}

/// Eases the shown scale towards `target` (metres per point) with time
/// constant `tau` (s), in the logarithm (zooms feel even).
pub fn ease_zoom(current: f64, target: f64, dt: f64, tau: f64) -> f64 {
    if current <= 0.0 || tau <= 0.0 {
        return target;
    }
    let k = 1.0 - (-dt.max(0.0) / tau).exp();
    let l = current.ln() + (target.ln() - current.ln()) * k;
    let v = l.exp();
    if (v / target - 1.0).abs() < 1e-3 {
        target
    } else {
        v
    }
}

/// Day or night by the sun's elevation at the rider (°): night below −4°
/// (civil twilight), day again above −2° (no flicker at dusk); no sky
/// known: day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Daylight(pub Shade);

impl Daylight {
    pub fn update(&mut self, sun_elevation_deg: Option<f64>) -> Shade {
        self.0 = match (self.0, sun_elevation_deg) {
            (_, None) => Shade::Day,
            (Shade::Day, Some(e)) if e < -4.0 => Shade::Night,
            (Shade::Night, Some(e)) if e > -2.0 => Shade::Day,
            (s, _) => s,
        };
        self.0
    }
}

/// The next turn as the banner shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    /// The turn (degrees, positive right); `None`: no junction ahead (the
    /// arrow points straight on).
    pub angle_deg: Option<f64>,
    /// "250 m"; `None` without a junction.
    pub distance: Option<String>,
    /// The street after the turn (or the one ridden now).
    pub street: String,
}

impl Banner {
    /// The banner's text: "↱ 250 m Street name".
    pub fn text(&self) -> String {
        let glyph = turn_glyph(self.angle_deg.unwrap_or(0.0));
        match &self.distance {
            Some(d) if self.street.is_empty() => format!("{glyph} {d}"),
            Some(d) => format!("{glyph} {d} {}", self.street),
            None => format!("{glyph} {}", self.street).trim_end().to_string(),
        }
    }
}

/// The arrow character of a turn (the banner draws an icon; this is its
/// text form for the logs, hooks and tests).
pub fn turn_glyph(angle_deg: f64) -> &'static str {
    let a = angle_deg;
    if a.abs() > 150.0 {
        if a > 0.0 { "↷" } else { "↶" }
    } else if a > 60.0 {
        "↱"
    } else if a > 20.0 {
        "↗"
    } else if a < -60.0 {
        "↰"
    } else if a < -20.0 {
        "↖"
    } else {
        "↑"
    }
}

/// The banner's distance, rounded as a navi does: 5 m steps below 100 m,
/// 10 m below 300 m, 50 m below 1 km, then 0.1 km ("now" when there);
/// imperial: feet in 50 ft steps below 0.1 mi, then 0.1 mi.
pub fn banner_distance(m: f64, imperial: bool) -> String {
    if m < 5.0 {
        return "now".into();
    }
    if imperial {
        if m < 0.1 * METRES_PER_MILE {
            let ft = (m / METRES_PER_FOOT / 50.0).round() * 50.0;
            return format!("{ft:.0} ft");
        }
        return format!("{:.1} mi", m / METRES_PER_MILE);
    }
    let step = if m < 100.0 {
        5.0
    } else if m < 300.0 {
        10.0
    } else if m < 1000.0 {
        50.0
    } else {
        return format!("{:.1} km", m / 1000.0);
    };
    format!("{:.0} m", (m / step).round() * step)
}

/// The banner for a junction `(distance m, turn °, street after it)`
/// ahead, or none (then the street ridden now, straight on).
pub fn banner(next: Option<(f64, f64, &str)>, street_now: &str, imperial: bool) -> Banner {
    match next {
        Some((d, angle, name)) => Banner {
            angle_deg: Some(angle),
            distance: Some(banner_distance(d.max(0.0), imperial)),
            street: name.to_string(),
        },
        None => Banner {
            angle_deg: None,
            distance: None,
            street: street_now.to_string(),
        },
    }
}

/// The centre line of `track` from `from` to `to` (m along it, clipped to
/// what it knows) every `step` m, straight runs thinned to their ends.
pub fn track_line(track: &dyn Track, from: f64, to: f64, step: f64) -> Vec<[f64; 2]> {
    let (lo, hi) = track.extent().unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
    let (a, b) = (from.max(lo), to.min(hi));
    if b <= a || step <= 0.0 {
        return Vec::new();
    }
    let n = ((b - a) / step).ceil() as usize;
    let mut pts: Vec<[f64; 2]> = (0..=n)
        .map(|i| {
            let d = (a + i as f64 * step).min(b);
            let p = track.sample(d).position;
            [p[0], p[2]]
        })
        .collect();
    pts.dedup();
    thin(&pts, 0.4)
}

/// Drops points within `tol` m of the line through their neighbours
/// (Douglas–Peucker).
pub fn thin(pts: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((i, j)) = stack.pop() {
        let (a, b) = (pts[i], pts[j]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dz * dz).sqrt();
        let mut best = (0.0, i);
        for (k, p) in pts.iter().enumerate().take(j).skip(i + 1) {
            let d = if len < 1e-9 {
                ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
            } else {
                ((p[0] - a[0]) * dz - (p[1] - a[1]) * dx).abs() / len
            };
            if d > best.0 {
                best = (d, k);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((i, best.1));
            stack.push((best.1, j));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// The route layer's parts in map points and palette colours.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteLines {
    /// The planned path ahead of the rider (route, or free ride: the path
    /// with the exits chosen and the defaults after them).
    pub ahead: Vec<[f64; 2]>,
    /// The path ridden (behind the rider).
    pub behind: Vec<[f64; 2]>,
    /// The next junction's other exits (a short way into each).
    pub exits: Vec<Vec<[f64; 2]>>,
}

/// The route's line width (map points; bold, as on a bike computer).
pub const ROUTE_WIDTH: f32 = 5.0;

impl RouteLines {
    pub fn is_empty(&self) -> bool {
        self.ahead.len() < 2 && self.behind.len() < 2 && self.exits.iter().all(|e| e.len() < 2)
    }

    /// The lines to draw: the exits thin under the route, the path behind
    /// dimmer, the path ahead bold with a casing on top.
    pub fn lines(&self, p: &Palette) -> Vec<MapLine> {
        let mut out = Vec::new();
        for e in self.exits.iter().filter(|e| e.len() >= 2) {
            out.push(MapLine {
                points: e.clone(),
                width: 3.0,
                colour: p.exit,
                casing: Some((2.0, p.route_casing)),
                layer: 0,
            });
        }
        if self.behind.len() >= 2 {
            out.push(MapLine {
                points: self.behind.clone(),
                width: ROUTE_WIDTH - 1.0,
                colour: p.route_behind,
                casing: None,
                layer: 1,
            });
        }
        if self.ahead.len() >= 2 {
            out.push(MapLine {
                points: self.ahead.clone(),
                width: ROUTE_WIDTH,
                colour: p.route,
                casing: Some((2.5, p.route_casing)),
                layer: 2,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use ocw_world::Route;

    use super::*;

    #[test]
    fn auto_zoom_follows_the_speed_and_closes_in_at_junctions() {
        let ahead = 168.0; // a 240-point map, the rider at 70 %
        // Slow: the closest level that shows 120 m.
        // Standing: 0.6 m × 168 = 101 m is too little, 1.0 m × 168 fits.
        assert_eq!(speed_level(0.0, ahead), 1);
        // Faster shows farther.
        assert!(speed_level(12.0, ahead) > speed_level(5.0, ahead));
        assert_eq!(speed_level(12.0, ahead), level_for(300.0, ahead));
        // Very fast: the farthest level, never beyond.
        assert_eq!(speed_level(1000.0, ahead), ZOOM_LEVELS.len() - 1);
        // A junction 150 m ahead zooms in to show it and 80 m beyond.
        assert_eq!(junction_level(150.0, ahead), Some(level_for(230.0, ahead)));
        assert_eq!(junction_level(400.0, ahead), None);
        let mut z = AutoZoom::default();
        assert_eq!(z.update(12.0, None, ahead, 0.0), level_for(300.0, ahead));
        let fast = z.level;
        // At the junction: at once closer.
        let near = z.update(12.0, Some(60.0), ahead, 0.016);
        assert!(near < fast);
        // After it: farther again only after the hold.
        let mut t = 0.0;
        while t < ZOOM_OUT_HOLD_S - 0.1 {
            assert_eq!(z.update(12.0, None, ahead, 0.1), near, "{t}");
            t += 0.1;
        }
        assert_eq!(z.update(12.0, None, ahead, 0.2), fast);
    }

    #[test]
    fn zoom_modes_parse_cycle_and_step() {
        assert_eq!(ZoomMode::parse("auto"), Some(ZoomMode::Auto));
        assert_eq!(ZoomMode::parse("1"), Some(ZoomMode::Fixed(0)));
        assert_eq!(ZoomMode::parse("7"), Some(ZoomMode::Fixed(6)));
        assert_eq!(ZoomMode::parse("8"), None);
        assert_eq!(ZoomMode::Fixed(3).label(), "4");
        let mut m = ZoomMode::Auto;
        let mut seen = vec![m];
        for _ in 0..ZOOM_LEVELS.len() + 1 {
            m = m.cycle();
            seen.push(m);
        }
        assert_eq!(seen.last(), Some(&ZoomMode::Auto));
        assert_eq!(ZoomMode::Auto.step(true, 3), ZoomMode::Fixed(2));
        assert_eq!(ZoomMode::Fixed(0).step(true, 3), ZoomMode::Fixed(0));
        assert_eq!(ZoomMode::Fixed(6).step(false, 0), ZoomMode::Fixed(6));
        let e = ease_zoom(1.0, 4.0, 10.0, 0.4);
        assert_eq!(e, 4.0);
        let e = ease_zoom(1.0, 4.0, 0.1, 0.4);
        assert!(e > 1.0 && e < 4.0);
    }

    #[test]
    fn the_banner_says_the_turn_the_distance_and_the_street() {
        let b = banner(Some((247.0, 85.0, "Landstrasse")), "Bahnweg", false);
        assert_eq!(b.text(), "↱ 250 m Landstrasse");
        assert_eq!(b.angle_deg, Some(85.0));
        let b = banner(Some((47.0, -40.0, "Kirchweg")), "", false);
        assert_eq!(b.text(), "↖ 45 m Kirchweg");
        let b = banner(Some((1260.0, 0.0, "")), "", false);
        assert_eq!(b.text(), "↑ 1.3 km");
        let b = banner(Some((2.0, 170.0, "Dorfplatz")), "", false);
        assert_eq!(b.text(), "↷ now Dorfplatz");
        let b = banner(None, "Rheinstrasse", false);
        assert_eq!((b.text().as_str(), b.distance), ("↑ Rheinstrasse", None));
        assert_eq!(banner_distance(640.0, false), "650 m");
        assert_eq!(banner_distance(130.0, true), "450 ft");
        assert_eq!(banner_distance(800.0, true), "0.5 mi");
        assert_eq!(turn_glyph(-95.0), "↰");
    }

    #[test]
    fn night_comes_at_civil_twilight_without_flicker() {
        let mut d = Daylight::default();
        assert_eq!(d.update(Some(10.0)), Shade::Day);
        assert_eq!(d.update(Some(-3.0)), Shade::Day);
        assert_eq!(d.update(Some(-4.5)), Shade::Night);
        assert_eq!(d.update(Some(-3.0)), Shade::Night, "hysteresis");
        assert_eq!(d.update(Some(-1.0)), Shade::Day);
        assert_eq!(d.update(None), Shade::Day);
    }

    /// Route highlighting: the path ahead bold on top, behind dimmer,
    /// exits thin under it; the line follows the track.
    #[test]
    fn the_route_ahead_is_bold_and_on_top() {
        let route = Route::test_loop();
        let ahead = track_line(&route, 100.0, 600.0, 10.0);
        assert!(ahead.len() >= 2);
        let p0 = route.sample(100.0).position;
        assert!((ahead[0][0] - p0[0]).abs() < 1e-9 && (ahead[0][1] - p0[2]).abs() < 1e-9);
        let behind = track_line(&route, 0.0, 100.0, 10.0);
        let lines = RouteLines {
            ahead,
            behind,
            exits: vec![vec![[0.0, 0.0], [10.0, 0.0]]],
        };
        let pal = Palette::of(Shade::Day);
        let l = lines.lines(&pal);
        assert_eq!(l.len(), 3);
        let top = l.iter().max_by_key(|l| l.layer).unwrap();
        assert_eq!(top.colour, pal.route);
        assert_eq!(top.width, ROUTE_WIDTH);
        assert!(top.casing.is_some());
        let exit = l.iter().min_by_key(|l| l.layer).unwrap();
        assert!(exit.width < top.width && exit.colour == pal.exit);
        assert!(l.iter().any(|l| l.colour == pal.route_behind));
        // A straight line keeps only its ends.
        let straight: Vec<[f64; 2]> = (0..20).map(|i| [f64::from(i), 2.0]).collect();
        assert_eq!(thin(&straight, 0.1), vec![[0.0, 2.0], [19.0, 2.0]]);
    }
}
