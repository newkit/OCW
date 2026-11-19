//! Place names on the map (spec §3.2 *Navigation map*, §4.7 *Places
//! section*): the names of cities, towns, villages and smaller places
//! come from the world packages' section `PLCE` ([`labels_of`], once per
//! loaded set of packages). The HUD's navigation map draws what
//! [`navi_labels`] keeps: by the navigation map's zoom (cities and towns
//! always, villages and hamlets as it zooms in), decluttered with the
//! studio map's pass ([`ocw_world::places::layout_labels`]: rank,
//! population, the last frame's labels as hysteresis), clear of the
//! map's own marks (compass, scale, zoom, the rider's arrow).
//! [`visible_labels`] is the plain list (inside the map, most important
//! first).
//!
//! **Peaks and passes** (spec 3.2 Q8, section `LMRK`) are labels too:
//! "▲ Teide 3715 m", ")( Boca de Tauce 2055 m" ([`PlaceLabel::text`]).
//! They are named from 6.5 m per point in, with the villages
//! ([`PEAK_MAX_M_PER_PX`]: peaks lie 1 – 3 km off the roads), behind
//! the towns and villages in the decluttering ([`layout_rank`]: towns keep
//! their priority, higher peaks before lower ones), except a pass the
//! route crosses ([`route_crosses`]): named at every zoom whenever it is
//! on the map, right after the towns.

use std::collections::{BTreeSet, HashSet};

use ocw_world::landmarks::LandmarkKind;
use ocw_world::places::{LabelCandidate, layout_labels};

use crate::source::MapSource;
use crate::view::NaviView;

/// What a label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelKind {
    /// A settlement (section `PLCE`).
    #[default]
    Place,
    /// A named peak (section `LMRK`).
    Peak,
    /// A mountain pass (section `LMRK`).
    Pass,
}

/// The rank peaks and passes are given (their font; the decluttering
/// orders them by [`layout_rank`]).
pub const PEAK_RANK: u8 = 3;
/// The coarsest navigation map zoom (metres per map point) peaks and
/// passes are named at: 6.5 (levels 1 – 6 of 7, with the villages; in
/// the mountains a peak is mostly 1 – 3 km from the road, out of a
/// closer map).
pub const PEAK_MAX_M_PER_PX: f64 = 6.5;
/// A pass this close to the route (m) is one the route crosses.
pub const ROUTE_PASS_M: f64 = 80.0;

/// A place name on the map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaceLabel {
    /// Where (home metres, x z).
    pub pos: [f64; 2],
    pub name: String,
    /// 0 = most important (city) … higher = smaller places; the map
    /// shows the important ones first when room is short. Peaks and
    /// passes: [`PEAK_RANK`].
    pub rank: u8,
    /// Inhabitants (0: not tagged), the next priority after the rank;
    /// for peaks and passes their elevation in metres.
    pub population: u32,
    pub kind: LabelKind,
    /// A peak's or pass's elevation (m).
    pub ele: Option<f32>,
}

impl PlaceLabel {
    /// The text on the map: the name; a peak "▲ Name 3715 m", a pass
    /// ")( Name 2055 m" (without a name: ")( 2055 m").
    pub fn text(&self) -> String {
        let sign = match self.kind {
            LabelKind::Place => return self.name.clone(),
            LabelKind::Peak => "▲",
            LabelKind::Pass => ")(",
        };
        let mut t = String::from(sign);
        if !self.name.is_empty() {
            t.push(' ');
            t.push_str(&self.name);
        }
        if let Some(e) = self.ele {
            t.push_str(&format!(" {} m", e.round() as i32));
        }
        t
    }
}

/// The decluttering priority of a label (smaller first): settlements by
/// rank in steps of two (cities 0, towns 2, villages 4, hamlets 6, …), a
/// pass the route crosses 3 (after the towns), other peaks and passes 5
/// (after the villages).
pub fn layout_rank(l: &PlaceLabel, crossed: bool) -> u8 {
    match l.kind {
        LabelKind::Place => l.rank.saturating_mul(2),
        LabelKind::Pass if crossed => 3,
        LabelKind::Peak | LabelKind::Pass => 5,
    }
}

/// The passes (indices into `labels`) within [`ROUTE_PASS_M`] of one of
/// the `lines` (home metres): the passes the route crosses.
pub fn route_crosses(labels: &[PlaceLabel], lines: &[&[[f64; 2]]]) -> BTreeSet<usize> {
    let near = |p: [f64; 2]| {
        lines.iter().any(|line| {
            line.windows(2).any(|w| {
                let (a, b) = (w[0], w[1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                let len2 = d[0] * d[0] + d[1] * d[1];
                let t = if len2 > 0.0 {
                    (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = [a[0] + d[0] * t, a[1] + d[1] * t];
                (p[0] - q[0]).hypot(p[1] - q[1]) <= ROUTE_PASS_M
            })
        })
    };
    labels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LabelKind::Pass && near(l.pos))
        .map(|(i, _)| i)
        .collect()
}

/// The labels of the loaded packages, most important first (each place
/// once, also where packages overlap), in the home package's metres.
pub fn labels_of(src: &MapSource) -> Vec<PlaceLabel> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for sp in &src.packages {
        let Some(places) = &sp.pkg.places else {
            continue;
        };
        for p in &places.places {
            if !seen.insert((p.element, p.osm_id, p.name.clone())) {
                continue;
            }
            out.push(PlaceLabel {
                pos: sp.frame.apply([f64::from(p.pos[0]), f64::from(p.pos[1])]),
                name: p.name.clone(),
                rank: p.rank,
                population: p.population.unwrap_or(0),
                ..Default::default()
            });
        }
        // Peaks and passes (section `LMRK`; viewpoints are not named).
        for l in &places.landmarks {
            let kind = match l.kind {
                LandmarkKind::Peak => LabelKind::Peak,
                LandmarkKind::Pass => LabelKind::Pass,
                LandmarkKind::Viewpoint => continue,
            };
            if l.name.is_empty() && l.ele.is_none() {
                continue;
            }
            if !seen.insert((l.element, l.osm_id, format!("landmark {}", l.name))) {
                continue;
            }
            out.push(PlaceLabel {
                pos: sp.frame.apply([f64::from(l.pos[0]), f64::from(l.pos[1])]),
                name: l.name.clone(),
                rank: PEAK_RANK,
                population: l.ele.map_or(0, |e| e.max(0.0) as u32),
                kind,
                ele: l.ele,
            });
        }
    }
    out.sort_by(|a, b| {
        layout_rank(a, false)
            .cmp(&layout_rank(b, false))
            .then(b.population.cmp(&a.population))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// The labels inside the map (`inset` points from its edge) with their
/// map points, most important first, at most `max`.
pub fn visible_labels<'a>(
    labels: &'a [PlaceLabel],
    view: &NaviView,
    inset: f32,
    max: usize,
) -> Vec<([f32; 2], &'a PlaceLabel)> {
    let mut v: Vec<_> = labels
        .iter()
        .map(|l| (view.to_screen(l.pos), l))
        .filter(|(s, _)| {
            s[0] >= inset
                && s[1] >= inset
                && s[0] <= view.size[0] - inset
                && s[1] <= view.size[1] - inset
        })
        .collect();
    v.sort_by(|a, b| a.1.rank.cmp(&b.1.rank).then(a.1.name.cmp(&b.1.name)));
    v.truncate(max);
    v
}

/// The coarsest navigation map zoom (metres per map point) a rank is
/// named at: cities and towns at every level, villages from 6.5 (levels
/// 1 – 6), hamlets and suburbs from 4, neighbourhoods from 2.5,
/// localities and isolated dwellings from 1.6 (the four closest levels).
pub fn navi_rank_max_m_per_px(rank: u8) -> f64 {
    match rank {
        0 | 1 => f64::INFINITY,
        2 => 6.5,
        3 => 4.0,
        4 => 2.5,
        _ => 1.6,
    }
}

/// The coarsest zoom a label is named at: [`navi_rank_max_m_per_px`] for
/// settlements, [`PEAK_MAX_M_PER_PX`] for peaks and passes, every zoom
/// for a pass the route crosses.
pub fn navi_max_m_per_px(l: &PlaceLabel, crossed: bool) -> f64 {
    match l.kind {
        LabelKind::Place => navi_rank_max_m_per_px(l.rank),
        LabelKind::Pass if crossed => f64::INFINITY,
        LabelKind::Peak | LabelKind::Pass => PEAK_MAX_M_PER_PX,
    }
}

/// The font size of a rank's names on the navigation map (points) and
/// whether bold (cities and towns).
pub fn navi_label_font(rank: u8) -> (f32, bool) {
    match rank {
        0 => (13.5, true),
        1 => (12.5, true),
        2 => (11.5, false),
        3 => (10.5, false),
        _ => (10.0, false),
    }
}

/// The width of a name at a font size (points), for the layout: the HUD
/// font is monospaced (Fira Mono, 0.6 em per character).
pub fn text_width(name: &str, size: f32, _bold: bool) -> f32 {
    name.chars().count() as f32 * size * 0.6
}

/// The space beside a name inside its pill (points, each side).
pub const PILL_PAD_PX: f32 = 3.0;

/// A name the navigation map draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NaviLabel {
    /// Index into the labels.
    pub index: usize,
    /// The place on the map (points; the text is centred above it, so it
    /// stays upright whatever the map's heading).
    pub at: [f32; 2],
    /// The text's box (points): min, max.
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub size: f32,
    pub bold: bool,
}

/// Half the box the rider's arrow keeps free of names (points).
pub const RIDER_MARK_PX: f32 = 17.0;

/// The map's own marks names keep clear of (map points: min, max): the
/// compass (top right), the scale (bottom left), the zoom (bottom right)
/// and the rider's arrow, for a map of `size` with the rider at `anchor`.
pub fn navi_marks(size: [f32; 2], anchor: [f32; 2]) -> Vec<([f32; 2], [f32; 2])> {
    let (w, h) = (size[0], size[1]);
    vec![
        ([w - 34.0, 2.0], [w, 34.0]),
        ([0.0, h - 34.0], [90.0, h]),
        ([w - 72.0, h - 24.0], [w, h]),
        (
            [anchor[0] - RIDER_MARK_PX, anchor[1] - RIDER_MARK_PX],
            [anchor[0] + RIDER_MARK_PX, anchor[1] + RIDER_MARK_PX],
        ),
    ]
}

/// The names to draw on the navigation map, most important first, at most
/// `max`: the labels whose rank is named at the view's zoom
/// ([`navi_rank_max_m_per_px`]), whose text lies inside the map
/// (`inset` points from its edge) and off `marks`, decluttered by
/// [`layout_labels`] with `previous` (the indices shown last frame) as
/// the hysteresis.
pub fn navi_labels(
    labels: &[PlaceLabel],
    view: &NaviView,
    marks: &[([f32; 2], [f32; 2])],
    inset: f32,
    previous: &BTreeSet<u32>,
    max: usize,
) -> Vec<NaviLabel> {
    navi_labels_with(labels, view, marks, inset, previous, &BTreeSet::new(), max)
}

/// [`navi_labels`] with the passes the route crosses (`crossed`, indices
/// into `labels`, [`route_crosses`]): named at every zoom and right after
/// the towns.
pub fn navi_labels_with(
    labels: &[PlaceLabel],
    view: &NaviView,
    marks: &[([f32; 2], [f32; 2])],
    inset: f32,
    previous: &BTreeSet<u32>,
    crossed: &BTreeSet<usize>,
    max: usize,
) -> Vec<NaviLabel> {
    let reach = f64::from(view.size[0].hypot(view.size[1])) * view.m_per_px;
    let hits = |lo: [f32; 2], hi: [f32; 2]| {
        marks
            .iter()
            .any(|(a, b)| lo[0] < b[0] && hi[0] > a[0] && lo[1] < b[1] && hi[1] > a[1])
    };
    let mut found: Vec<NaviLabel> = Vec::new();
    let mut cands: Vec<LabelCandidate> = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        let on_route = crossed.contains(&i);
        if view.m_per_px > navi_max_m_per_px(l, on_route) {
            continue;
        }
        // Far away: not even projected.
        if (l.pos[0] - view.rider[0]).abs() > reach || (l.pos[1] - view.rider[1]).abs() > reach {
            continue;
        }
        let at = view.to_screen(l.pos);
        let (size, bold) = navi_label_font(l.rank);
        // The pill: the text and its padding.
        let w = text_width(&l.text(), size, bold) + 2.0 * PILL_PAD_PX;
        let h = size * 1.25;
        // Above the place; else below it; else, for a place by the rider
        // whose name would cover the arrow (the town the rider is in), just
        // above the arrow. A box is moved sideways into the map as long as
        // it stays over its place.
        let anchor = view.anchor();
        let arrow_top = anchor[1] - RIDER_MARK_PX;
        let tries = [
            (at[1] - h - 2.0, at[1] + 2.0),
            (at[1] + 3.0, at[1] + h + 5.0),
            (arrow_top - h - 2.0, arrow_top - 1.0),
        ];
        let by_rider = (at[0] - anchor[0]).abs() <= RIDER_MARK_PX + w * 0.5 + 4.0
            && (at[1] - anchor[1]).abs() <= RIDER_MARK_PX + h + 4.0;
        let tries = if by_rider { &tries[..] } else { &tries[..2] };
        let (left, right) = (inset, view.size[0] - inset);
        let place = |top: f32, bottom: f32| -> Option<([f32; 2], [f32; 2])> {
            let mut x0 = at[0] - w * 0.5;
            if x0 < left {
                x0 = left;
            }
            if x0 + w > right {
                x0 = right - w;
            }
            let (min, max) = ([x0, top], [x0 + w, bottom]);
            let over_place = at[0] >= min[0] + 2.0 && at[0] <= max[0] - 2.0;
            let inside = min[0] >= left
                && max[0] <= right
                && min[1] >= inset
                && max[1] <= view.size[1] - inset;
            (over_place && inside && !hits(min, max)).then_some((min, max))
        };
        let Some((min, max)) = tries.iter().find_map(|&(top, bottom)| place(top, bottom)) else {
            continue;
        };
        cands.push(LabelCandidate {
            id: i as u32,
            rank: layout_rank(l, on_route),
            population: l.population,
            anchor: at,
            min,
            max,
        });
        found.push(NaviLabel {
            index: i,
            at,
            min,
            max,
            size,
            bold,
        });
    }
    let centre = view.anchor();
    let shown = layout_labels(&cands, centre, [0.0, 0.0], view.size, previous);
    shown
        .into_iter()
        .take(max)
        .filter_map(|id| found.iter().find(|f| f.index == id as usize).copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ocw_world::places::{OsmElement, Place, PlaceKind, Places, place_rank};

    use super::*;
    use crate::source::SourcePackage;

    fn label(x: f64, z: f64, name: &str, rank: u8) -> PlaceLabel {
        PlaceLabel {
            pos: [x, z],
            name: name.into(),
            rank,
            population: 0,
            ..Default::default()
        }
    }

    fn peak(x: f64, z: f64, name: &str, ele: f32, kind: LabelKind) -> PlaceLabel {
        PlaceLabel {
            pos: [x, z],
            name: name.into(),
            rank: PEAK_RANK,
            population: ele as u32,
            kind,
            ele: Some(ele),
        }
    }

    fn boxes_apart(a: &NaviLabel, b: &NaviLabel) -> bool {
        a.min[0] >= b.max[0] || a.max[0] <= b.min[0] || a.min[1] >= b.max[1] || a.max[1] <= b.min[1]
    }

    #[test]
    fn peaks_and_passes_have_their_sign_name_and_elevation() {
        assert_eq!(
            peak(0.0, 0.0, "Pico Ada", 3714.6, LabelKind::Peak).text(),
            "▲ Pico Ada 3715 m"
        );
        assert_eq!(
            peak(0.0, 0.0, "Bea Pass", 2055.0, LabelKind::Pass).text(),
            ")( Bea Pass 2055 m"
        );
        assert_eq!(
            peak(0.0, 0.0, "", 1800.0, LabelKind::Pass).text(),
            ")( 1800 m"
        );
        let mut no_ele = peak(0.0, 0.0, "Cora Kopf", 0.0, LabelKind::Peak);
        no_ele.ele = None;
        assert_eq!(no_ele.text(), "▲ Cora Kopf");
        assert_eq!(label(0.0, 0.0, "Ada Town", 1).text(), "Ada Town");
    }

    /// A town, a village, two peaks and a pass near the rider; the route
    /// runs north over the pass.
    fn mountains() -> Vec<PlaceLabel> {
        vec![
            label(-300.0, -500.0, "Ada Town", 1),
            label(350.0, -250.0, "Bea Village", 2),
            peak(-250.0, -150.0, "Pico Alto", 3100.0, LabelKind::Peak),
            peak(300.0, -600.0, "Pico Bajo", 1900.0, LabelKind::Peak),
            peak(0.0, -300.0, "Paso Cora", 2050.0, LabelKind::Pass),
        ]
    }

    #[test]
    fn peaks_come_from_the_medium_zoom_in_and_a_crossed_pass_always() {
        let ls = mountains();
        let route: Vec<[f64; 2]> = vec![[0.0, 100.0], [0.0, -2000.0]];
        let crossed = route_crosses(&ls, &[&route]);
        assert_eq!(crossed, BTreeSet::from([4]));
        assert!(route_crosses(&ls, &[&[[500.0, 0.0], [500.0, -50.0]][..]]).is_empty());
        let at = |mpp: f64, crossed: &BTreeSet<usize>| {
            let v = view(mpp, 0.0);
            let marks = navi_marks(v.size, v.anchor());
            names(
                &ls,
                &navi_labels_with(&ls, &v, &marks, 6.0, &BTreeSet::new(), crossed, 8),
            )
        };
        let none = BTreeSet::new();
        // Far out (10 m per point): no peaks; the crossed pass yes.
        let far = at(10.0, &none);
        assert!(
            !far.iter()
                .any(|n| n.starts_with("Pico") || n.starts_with("Paso")),
            "{far:?}"
        );
        // (At 10 m per point the town's name lies where the pass's would:
        // the town keeps its place; without it the pass is named.)
        let far = at(10.0, &crossed);
        assert!(far.iter().any(|n| n == "Ada Town"), "{far:?}");
        let alone: Vec<PlaceLabel> = ls[2..].to_vec();
        let v = view(10.0, 0.0);
        let shown = navi_labels_with(
            &alone,
            &v,
            &navi_marks(v.size, v.anchor()),
            6.0,
            &BTreeSet::new(),
            &route_crosses(&alone, &[&route]),
            8,
        );
        assert_eq!(names(&alone, &shown), ["Paso Cora"]);
        // Closer (4 m): the peaks are named, the town first.
        let mid = at(4.0, &none);
        assert_eq!(mid[0], "Ada Town", "{mid:?}");
        assert!(mid.iter().any(|n| n == "Pico Alto"), "{mid:?}");
        // The crossed pass is laid out right after the town.
        let v = view(4.0, 0.0);
        let shown = navi_labels_with(
            &ls,
            &v,
            &navi_marks(v.size, v.anchor()),
            6.0,
            &BTreeSet::new(),
            &crossed,
            8,
        );
        let order: Vec<usize> = shown.iter().map(|l| l.index).collect();
        assert_eq!(&order[..2], &[0, 4], "{order:?}");
        // The pill fits the whole text (sign, name, elevation).
        let pass = shown.iter().find(|l| l.index == 4).unwrap();
        let w = text_width(&ls[4].text(), pass.size, pass.bold) + 2.0 * PILL_PAD_PX;
        assert!((pass.max[0] - pass.min[0] - w).abs() < 0.01);
    }

    #[test]
    fn towns_keep_their_place_when_a_peak_crowds_them() {
        // A peak right beside the town, higher than any: the town is named.
        let ls = vec![
            label(0.0, -150.0, "Ada Town", 1),
            peak(8.0, -152.0, "Pico Ada", 3700.0, LabelKind::Peak),
            label(5.0, -147.0, "Bea Village", 2),
        ];
        for heading in [0.0, 1.3, -2.2] {
            let v = view(2.5, heading);
            let marks = navi_marks(v.size, v.anchor());
            let shown = navi_labels(&ls, &v, &marks, 6.0, &BTreeSet::new(), 8);
            assert_eq!(shown[0].index, 0, "heading {heading}");
            for (i, a) in shown.iter().enumerate() {
                for b in &shown[i + 1..] {
                    assert!(boxes_apart(a, b), "heading {heading}");
                }
                for (lo, hi) in &marks {
                    let m = NaviLabel {
                        index: 0,
                        at: [0.0; 2],
                        min: *lo,
                        max: *hi,
                        size: 0.0,
                        bold: false,
                    };
                    assert!(boxes_apart(a, &m), "heading {heading}: on a mark");
                }
            }
        }
    }

    #[test]
    fn many_peaks_never_overlap_and_the_higher_go_first() {
        let mut ls = vec![label(0.0, -500.0, "Ada Town", 1)];
        for i in 0..10 {
            for k in 0..10 {
                ls.push(peak(
                    -450.0 + 100.0 * f64::from(i),
                    -700.0 + 90.0 * f64::from(k),
                    &format!("Kopf {i}{k}"),
                    1000.0 + 25.0 * f32::from(i as u8 * 10 + k as u8),
                    if (i + k) % 3 == 0 {
                        LabelKind::Pass
                    } else {
                        LabelKind::Peak
                    },
                ));
            }
        }
        for heading in [0.0, 0.8, 2.6, -1.4] {
            let v = view(2.5, heading);
            let marks = navi_marks(v.size, v.anchor());
            let shown = navi_labels(&ls, &v, &marks, 6.0, &BTreeSet::new(), 8);
            assert!(shown.len() >= 2, "heading {heading}");
            for (i, a) in shown.iter().enumerate() {
                for b in &shown[i + 1..] {
                    assert!(boxes_apart(a, b), "heading {heading}");
                }
            }
        }
    }

    fn view(m_per_px: f64, heading: f64) -> NaviView {
        NaviView {
            rider: [0.0, 0.0],
            heading,
            m_per_px,
            size: [240.0, 264.0],
        }
    }

    #[test]
    fn labels_inside_the_map_most_important_first() {
        let view = NaviView {
            rider: [0.0, 0.0],
            heading: 0.0,
            m_per_px: 1.0,
            size: [200.0, 200.0],
        };
        let labels = [
            label(0.0, -50.0, "Hamlet", 3),
            label(10.0, -100.0, "Town", 1),
            label(5000.0, 0.0, "Far away", 0),
        ];
        let v = visible_labels(&labels, &view, 8.0, 5);
        let names: Vec<&str> = v.iter().map(|(_, l)| l.name.as_str()).collect();
        assert_eq!(names, ["Town", "Hamlet"]);
        assert_eq!(v[1].0, [100.0, 90.0]);
        assert_eq!(visible_labels(&labels, &view, 8.0, 1).len(), 1);
    }

    /// A town ahead with villages, a hamlet and a locality around it.
    fn around() -> Vec<PlaceLabel> {
        vec![
            label(0.0, -400.0, "Ada Town", 1),
            label(-500.0, -1100.0, "Bea Village", 2),
            label(450.0, -300.0, "Cora Village", 2),
            label(-150.0, -250.0, "Dora Hamlet", 3),
            label(120.0, -110.0, "Elia Ried", 5),
        ]
    }

    fn names(labels: &[PlaceLabel], shown: &[NaviLabel]) -> Vec<String> {
        shown.iter().map(|l| labels[l.index].name.clone()).collect()
    }

    #[test]
    fn the_zoom_levels_name_towns_far_out_and_smaller_places_close_in() {
        let ls = around();
        let none = BTreeSet::new();
        let at = |mpp: f64| {
            let v = view(mpp, 0.0);
            names(
                &ls,
                &navi_labels(&ls, &v, &navi_marks(v.size, v.anchor()), 6.0, &none, 8),
            )
        };
        // Level 7 (10 m per point): the town only.
        assert_eq!(at(10.0), ["Ada Town"]);
        // Level 5 (4 m): villages and hamlets come in, the town first.
        let mid = at(4.0);
        assert_eq!(mid[0], "Ada Town");
        assert!(mid.iter().any(|n| n == "Dora Hamlet"), "{mid:?}");
        assert!(!mid.iter().any(|n| n == "Elia Ried"), "{mid:?}");
        // Level 3 (1.6 m): the locality next to the rider.
        let close = at(1.6);
        assert!(close.iter().any(|n| n == "Elia Ried"), "{close:?}");
    }

    #[test]
    fn names_do_not_overlap_each_other_or_the_marks_and_turn_with_the_map() {
        // A crowded square of villages around the rider.
        let mut ls = vec![label(0.0, -600.0, "Ada Town", 1)];
        for i in 0..12 {
            for k in 0..12 {
                ls.push(label(
                    -1100.0 + 200.0 * f64::from(i),
                    -1200.0 + 200.0 * f64::from(k),
                    &format!("Weiler {i}-{k}"),
                    2,
                ));
            }
        }
        for heading in [0.0, 1.0, 2.5, -2.0] {
            let v = view(5.0, heading);
            let marks = navi_marks(v.size, v.anchor());
            let shown = navi_labels(&ls, &v, &marks, 6.0, &BTreeSet::new(), 8);
            assert!(!shown.is_empty() && shown.len() <= 8);
            for (i, a) in shown.iter().enumerate() {
                for b in &shown[i + 1..] {
                    let apart = a.min[0] >= b.max[0]
                        || a.max[0] <= b.min[0]
                        || a.min[1] >= b.max[1]
                        || a.max[1] <= b.min[1];
                    assert!(apart, "heading {heading}");
                }
                for (lo, hi) in &marks {
                    let clear = a.min[0] >= hi[0]
                        || a.max[0] <= lo[0]
                        || a.min[1] >= hi[1]
                        || a.max[1] <= lo[1];
                    assert!(clear, "heading {heading}: {:?} on a mark", ls[a.index].name);
                }
                // Upright: the box is level and over its place.
                assert!(a.at[0] >= a.min[0] && a.at[0] <= a.max[0]);
                assert!(a.min[0] >= 6.0 && a.max[0] <= v.size[0] - 6.0);
            }
            // The town comes before the villages when it is named.
            if let Some(t) = shown.iter().position(|l| l.index == 0) {
                assert_eq!(t, 0, "heading {heading}");
            }
        }
    }

    #[test]
    fn the_place_the_rider_is_in_is_named_above_the_arrow() {
        let ls = vec![label(5.0, -3.0, "Ada Town", 1)];
        let v = view(10.0, 0.7);
        let marks = navi_marks(v.size, v.anchor());
        let shown = navi_labels(&ls, &v, &marks, 6.0, &BTreeSet::new(), 8);
        assert_eq!(shown.len(), 1);
        let l = shown[0];
        assert!(l.max[1] <= v.anchor()[1] - RIDER_MARK_PX, "{l:?}");
        for (lo, hi) in &marks {
            assert!(
                l.min[0] >= hi[0] || l.max[0] <= lo[0] || l.min[1] >= hi[1] || l.max[1] <= lo[1]
            );
        }
    }

    /// The cost per frame with a big package's places (2,000, as
    /// Tenerife's 1,823): printed for the record (`--nocapture`), checked
    /// loosely (debug builds).
    #[test]
    fn the_layout_per_frame_is_cheap() {
        let ls: Vec<PlaceLabel> = (0..2000)
            .map(|i| {
                let a = f64::from(i) * 0.7;
                let r = 30.0 * f64::from(i);
                label(r * a.cos(), r * a.sin(), &format!("Ort {i}"), (i % 6) as u8)
            })
            .collect();
        let v = view(2.5, 0.3);
        let marks = navi_marks(v.size, v.anchor());
        let mut prev = BTreeSet::new();
        let t = std::time::Instant::now();
        let n = 200;
        for _ in 0..n {
            let s = navi_labels(&ls, &v, &marks, 6.0, &prev, 8);
            prev = s.iter().map(|l| l.index as u32).collect();
        }
        let per = t.elapsed().as_secs_f64() / f64::from(n) * 1e6;
        println!("navi_labels: {per:.1} µs per frame for {} places", ls.len());
        assert!(per < 5_000.0, "{per} µs");
    }

    /// Regression (Puerto de la Cruz, 2026-11-18): a long name whose place
    /// lies beside the rider, a little left and behind, covered the arrow
    /// above and below its place and was dropped; it goes above the arrow.
    #[test]
    fn a_long_name_beside_the_rider_is_named_above_the_arrow() {
        // 70 m left and 20 m behind at 2.5 m per point (zoom 4), heading
        // north; then the same at other headings and offsets around.
        for (dx, dz, heading) in [
            (-70.0, 20.0, 0.0),
            (60.0, -10.0, 0.0),
            (-30.0, 30.0, 1.2),
            (20.0, 5.0, -2.0),
        ] {
            let ls = vec![label(dx, dz, "Puerto de la Cruz", 1)];
            let v = view(2.5, heading);
            let marks = navi_marks(v.size, v.anchor());
            let shown = navi_labels(&ls, &v, &marks, 6.0, &BTreeSet::new(), 8);
            assert_eq!(shown.len(), 1, "{dx} {dz} {heading}");
            let l = shown[0];
            assert!(l.min[0] >= 6.0 && l.max[0] <= v.size[0] - 6.0, "{l:?}");
            assert!(l.at[0] >= l.min[0] && l.at[0] <= l.max[0], "{l:?}");
            for (lo, hi) in &marks {
                assert!(
                    l.min[0] >= hi[0]
                        || l.max[0] <= lo[0]
                        || l.min[1] >= hi[1]
                        || l.max[1] <= lo[1],
                    "{dx} {dz} {heading}: {l:?}"
                );
            }
        }
    }

    #[test]
    fn riding_on_keeps_the_names_steady() {
        let ls = around();
        let mut prev = BTreeSet::new();
        let mut v = view(4.0, 0.0);
        let marks = navi_marks(v.size, v.anchor());
        let first = navi_labels(&ls, &v, &marks, 6.0, &prev, 8);
        prev = first.iter().map(|l| l.index as u32).collect();
        // Ride north 2 m per frame: names go only by leaving the map.
        for step in 1..=60 {
            v.rider = [0.0, -2.0 * f64::from(step)];
            let now = navi_labels(&ls, &v, &marks, 6.0, &prev, 8);
            let set: BTreeSet<u32> = now.iter().map(|l| l.index as u32).collect();
            for gone in prev.difference(&set) {
                let s = v.to_screen(ls[*gone as usize].pos);
                assert!(
                    !v.inside(s, 40.0, 0.0),
                    "step {step}: {} dropped",
                    ls[*gone as usize].name
                );
            }
            prev = set;
        }
    }

    #[test]
    fn labels_come_from_the_packages_places_in_home_metres() {
        let mut pkg = crate::fixture::package();
        let place = |name: &str, kind, pos, population| Place {
            kind,
            rank: place_rank(kind, None, population),
            pos,
            name: String::from(name),
            names: Vec::new(),
            population,
            capital: None,
            area: false,
            element: OsmElement::Node,
            osm_id: name.len() as i64,
        };
        let mark = |name: &str, kind, ele: Option<f32>| ocw_world::landmarks::Landmark {
            kind,
            pos: [30.0, 40.0],
            ele,
            ele_from_terrain: false,
            direction: None,
            name: name.into(),
            names: Vec::new(),
            element: OsmElement::Node,
            osm_id: 100 + name.len() as i64,
        };
        pkg.places = Some(
            Places::new(vec![
                place("Ada Dorf", PlaceKind::Village, [10.0, 20.0], None),
                place("Bea Stadt", PlaceKind::Town, [50.0, 60.0], Some(9_000)),
            ])
            .with_landmarks(vec![
                mark("Pico Cora", LandmarkKind::Peak, Some(2400.0)),
                mark("Dora Pass", LandmarkKind::Pass, Some(1800.0)),
                mark("Mirador Elia", LandmarkKind::Viewpoint, Some(900.0)),
                mark("", LandmarkKind::Pass, None),
            ]),
        );
        let pkg = Arc::new(pkg);
        let src = MapSource::new(
            vec![
                SourcePackage {
                    pkg: pkg.clone(),
                    frame: ocw_world::world_set::Frame {
                        scale_x: 1.0,
                        offset: [100.0, -50.0],
                    },
                },
                // The same package again (overlapping neighbours): once.
                SourcePackage {
                    pkg,
                    frame: ocw_world::world_set::Frame::IDENTITY,
                },
            ],
            None,
        );
        let ls = labels_of(&src);
        let names: Vec<&str> = ls.iter().map(|l| l.name.as_str()).collect();
        // Settlements first, then the peaks and passes (higher first); no
        // viewpoints, no pass without a name and elevation; each once.
        assert_eq!(names, ["Bea Stadt", "Ada Dorf", "Pico Cora", "Dora Pass"]);
        assert_eq!(ls[2].kind, LabelKind::Peak);
        assert_eq!(ls[3].text(), ")( Dora Pass 1800 m");
        assert_eq!(ls[0].population, 9_000);
        let f = ocw_world::world_set::Frame {
            scale_x: 1.0,
            offset: [100.0, -50.0],
        };
        assert_eq!(ls[1].pos, f.apply([10.0, 20.0]));
        // Packages without the section: no labels.
        let none = MapSource::new(
            vec![SourcePackage {
                pkg: Arc::new(crate::fixture::package()),
                frame: ocw_world::world_set::Frame::IDENTITY,
            }],
            None,
        );
        assert!(labels_of(&none).is_empty());
    }
}
