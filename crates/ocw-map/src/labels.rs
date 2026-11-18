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

use std::collections::{BTreeSet, HashSet};

use ocw_world::places::{LabelCandidate, layout_labels};

use crate::source::MapSource;
use crate::view::NaviView;

/// A place name on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLabel {
    /// Where (home metres, x z).
    pub pos: [f64; 2],
    pub name: String,
    /// 0 = most important (city) … higher = smaller places; the map
    /// shows the important ones first when room is short.
    pub rank: u8,
    /// Inhabitants (0: not tagged), the next priority after the rank.
    pub population: u32,
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
            });
        }
    }
    out.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
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
    let reach = f64::from(view.size[0].hypot(view.size[1])) * view.m_per_px;
    let hits = |lo: [f32; 2], hi: [f32; 2]| {
        marks
            .iter()
            .any(|(a, b)| lo[0] < b[0] && hi[0] > a[0] && lo[1] < b[1] && hi[1] > a[1])
    };
    let mut found: Vec<NaviLabel> = Vec::new();
    let mut cands: Vec<LabelCandidate> = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        if view.m_per_px > navi_rank_max_m_per_px(l.rank) {
            continue;
        }
        // Far away: not even projected.
        if (l.pos[0] - view.rider[0]).abs() > reach || (l.pos[1] - view.rider[1]).abs() > reach {
            continue;
        }
        let at = view.to_screen(l.pos);
        let (size, bold) = navi_label_font(l.rank);
        // The pill: the text and its padding.
        let w = text_width(&l.name, size, bold) + 2.0 * PILL_PAD_PX;
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
            rank: l.rank,
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
        pkg.places = Some(Places::new(vec![
            place("Ada Dorf", PlaceKind::Village, [10.0, 20.0], None),
            place("Bea Stadt", PlaceKind::Town, [50.0, 60.0], Some(9_000)),
        ]));
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
        assert_eq!(names, ["Bea Stadt", "Ada Dorf"]);
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
