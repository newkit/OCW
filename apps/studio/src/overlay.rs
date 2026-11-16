//! The world patch over the views (spec §14 Phase 16, 16C *Preview*, E5
//! step 2): what the patch does, drawn over the 2D map and as lines in the
//! 3D view (Bevy gizmos) – the selected road highlighted, changed roads
//! tinted, removed (or closed) roads struck through, height points as
//! markers with their radius, and the road profiles the height points
//! change as the new line. No tile is re-meshed (the live tile preview of
//! 16C is a later step).
//!
//! [`marks`] says what to draw (pure, tested); the map
//! ([`crate::map::paint`]) and [`draw_3d`] draw it.

use std::f32::consts::FRAC_PI_2;

use bevy::color::Color;
use bevy::prelude::*;
use ocw_edit::world::RoadTarget;

use crate::edit::Studio;
use crate::patch::PatchPreview;

/// The colours of the marks (sRGB).
pub const SELECTED: [u8; 3] = [40, 220, 255];
pub const CHANGED: [u8; 3] = [255, 150, 30];
pub const REMOVED: [u8; 3] = [240, 40, 40];
pub const POINT: [u8; 3] = [255, 60, 220];
pub const PROFILE: [u8; 3] = [255, 240, 60];

/// A height point to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct PointMark {
    pub id: String,
    /// Package metres (x, z).
    pub at: [f64; 2],
    pub radius_m: f64,
    pub selected: bool,
    /// The edge it snaps to, if it names one (stable id).
    pub edge: Option<u64>,
    /// "+3.0 m" or "455.0 m".
    pub label: String,
}

/// What the views draw of the patch: edge indices of the package's road
/// graph, and the height points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marks {
    /// The selected edge first, then the rest of its way when road edits
    /// act on the way.
    pub selected: Vec<usize>,
    pub changed: Vec<usize>,
    pub removed: Vec<usize>,
    pub points: Vec<PointMark>,
}

/// The marks of the open patch.
pub fn marks(studio: &Studio) -> Marks {
    let (Some(doc), Some(pkg)) = (studio.world(), studio.package.as_ref()) else {
        return Marks::default();
    };
    let mut m = Marks::default();
    if let Some(e) = studio.selected_edge()
        && let Some(&i) = pkg.edges.get(&e)
    {
        m.selected.push(i);
        if let Some(RoadTarget::Way(w)) = studio.selected_target() {
            m.selected.extend(
                pkg.target_edges(RoadTarget::Way(w))
                    .into_iter()
                    .filter(|&k| k != i),
            );
        }
    }
    let (changed, removed) = doc.content().touched_roads();
    for t in &removed {
        m.removed.extend(pkg.target_edges(*t));
    }
    for t in &changed {
        m.changed.extend(
            pkg.target_edges(*t)
                .into_iter()
                .filter(|i| !m.removed.contains(i)),
        );
    }
    m.removed.sort_unstable();
    m.removed.dedup();
    m.changed.sort_unstable();
    m.changed.dedup();
    let sel = studio.selected_point();
    for (_, h) in doc.content().height_points() {
        let Some(at) = studio.point_xz(h) else {
            continue;
        };
        let label = match (h.height_m, h.raise_m) {
            (Some(v), _) => format!("{v:.1} m"),
            (_, Some(r)) => format!("{r:+.1} m"),
            _ => String::new(),
        };
        m.points.push(PointMark {
            id: h.id.clone(),
            at,
            radius_m: h.radius_m.unwrap_or(ocw_world::patch::DEFAULT_RADIUS_M),
            selected: sel.as_deref() == Some(h.id.as_str()),
            edge: h.edge.map(|e| e.0),
            label,
        });
    }
    m
}

/// The stretches of an edge whose profile `new` differs from its own (by
/// more than a centimetre), as 3D lines at the new height plus `lift`:
/// only the part of the road a height point moves is drawn.
pub fn changed_runs(edge: &ocw_world::graph::Edge, new: &[f64], lift: f64) -> Vec<Vec<[f64; 3]>> {
    let old = &edge.profile;
    if old.len() != new.len() {
        return Vec::new();
    }
    let len = edge.length_m();
    let n = new.len();
    let changed: Vec<bool> = (0..n).map(|i| (new[i] - old[i]).abs() > 0.01).collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < n {
        if !changed[i] {
            i += 1;
            continue;
        }
        let start = i.saturating_sub(1);
        while i < n && changed[i] {
            i += 1;
        }
        let end = i.min(n - 1);
        let run: Vec<[f64; 3]> = (start..=end)
            .filter_map(|k| {
                let arc = (k as f64 * ocw_world::graph::PROFILE_STEP_M).min(len);
                let p = crate::pick::point_at(&edge.points, arc)?;
                Some([p[0], new[k] + lift, p[1]])
            })
            .collect();
        if run.len() >= 2 {
            runs.push(run);
        }
    }
    runs
}

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PatchPreview>()
            .add_systems(Startup, gizmo_style)
            .add_systems(Update, (crate::patch::update_preview, draw_3d).chain());
    }
}

/// Lines a little wider than the default, and in front of the road and
/// the terrain they lie on.
fn gizmo_style(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 3.0;
    config.depth_bias = -1.0;
}

fn colour(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// How high over the road the lines are drawn (m).
const LIFT_M: f64 = 0.6;

/// Draws the marks in the 3D view.
pub fn draw_3d(
    studio: Res<Studio>,
    preview: Res<PatchPreview>,
    ground: Option<Res<ocw_render::Ground>>,
    mut gizmos: Gizmos,
) {
    let Some(pkg) = studio.package.as_ref() else {
        return;
    };
    let Some(graph) = pkg.graph() else {
        return;
    };
    let m = marks(&studio);
    let g = |x: f64, z: f64| ground.as_ref().and_then(|g| (g.0)(x, z));
    let line = |gizmos: &mut Gizmos, i: usize, c: Color, lift: f64| {
        let pts = crate::pick::edge_line(&graph.edges[i], None, lift, g);
        gizmos.linestrip(pts.into_iter().map(v3), c);
    };
    for &i in &m.changed {
        line(&mut gizmos, i, colour(CHANGED), LIFT_M);
    }
    for &i in &m.removed {
        let e = &graph.edges[i];
        line(&mut gizmos, i, colour(REMOVED), LIFT_M);
        // Struck through: crosses along the road every 25 m.
        let len = e.length_m();
        let mut d = 5.0f64.min(len * 0.5);
        while d < len {
            if let (Some(p), Some(q)) = (
                crate::pick::point_at(&e.points, d),
                crate::pick::point_at(&e.points, (d + 1.0).min(len)),
            ) {
                let h = e.height_at(d).or_else(|| g(p[0], p[1])).unwrap_or(0.0) + LIFT_M;
                let dir = [q[0] - p[0], q[1] - p[1]];
                let l = dir[0].hypot(dir[1]).max(1e-6);
                let (u, n) = ([dir[0] / l, dir[1] / l], [-dir[1] / l, dir[0] / l]);
                let s = (e.width_m * 0.5).max(2.0);
                for k in [-1.0, 1.0] {
                    let a = [
                        p[0] - (u[0] + n[0] * k) * s,
                        h,
                        p[1] - (u[1] + n[1] * k) * s,
                    ];
                    let b = [
                        p[0] + (u[0] + n[0] * k) * s,
                        h,
                        p[1] + (u[1] + n[1] * k) * s,
                    ];
                    gizmos.line(v3(a), v3(b), colour(REMOVED));
                }
            }
            d += 25.0;
        }
    }
    for (k, &i) in m.selected.iter().enumerate() {
        let c = if k == 0 {
            colour(SELECTED)
        } else {
            colour(SELECTED).with_alpha(0.55)
        };
        line(&mut gizmos, i, c, LIFT_M + 0.2);
    }
    // The road profiles as the height points make them.
    for (id, profile) in &preview.preview.profiles {
        if let Some(e) = pkg.edge(*id) {
            for run in changed_runs(e, profile, LIFT_M) {
                gizmos.linestrip(run.into_iter().map(v3), colour(PROFILE));
            }
        }
    }
    for p in &m.points {
        let h = p
            .edge
            .and_then(|e| pkg.edge(e))
            .and_then(|e| {
                let (_, arc, _) = crate::pick::nearest_on_polyline(&e.points, p.at);
                preview
                    .preview
                    .profiles
                    .get(&e.id.0)
                    .map(|prof| {
                        let mut c = e.clone();
                        c.profile = prof.clone();
                        c
                    })
                    .as_ref()
                    .unwrap_or(e)
                    .height_at(arc)
            })
            .or_else(|| g(p.at[0], p.at[1]))
            .unwrap_or(0.0);
        let c = if p.selected {
            Color::WHITE
        } else {
            colour(POINT)
        };
        let base = v3([p.at[0], h + LIFT_M, p.at[1]]);
        gizmos.line(base, base + Vec3::Y * 30.0, c);
        gizmos.sphere(
            Isometry3d::from_translation(base + Vec3::Y * 30.0),
            if p.selected { 3.0 } else { 2.0 },
            c,
        );
        // The radius, flat around the point.
        gizmos.circle(
            Isometry3d::new(base, Quat::from_rotation_x(FRAC_PI_2)),
            p.radius_m as f32,
            colour(POINT).with_alpha(0.8),
        );
    }
}

#[cfg(test)]
mod tests {
    use ocw_edit::Document;
    use ocw_edit::world::{PackageRef, RoadChange, WorldDoc, WorldItem};
    use ocw_world::graph::EdgeId;
    use ocw_world::patch::HeightPoint;

    use super::*;

    fn studio() -> Studio {
        let mut s = Studio::new(None, None);
        let pkg = crate::patch::tests::package();
        let id = s.session.open(|id| {
            Document::new(
                id,
                "Testland",
                WorldDoc::new(PackageRef {
                    id: "local/testland".into(),
                    ..Default::default()
                }),
            )
        });
        s.package = Some(pkg);
        s.world_doc = Some(id);
        s
    }

    #[test]
    fn only_the_changed_stretch_of_a_profile_is_drawn() {
        let e = &crate::pick::tests::graph().edges[0];
        // 100 m at 400 m: samples 0..=20; 6..=8 raised.
        let mut new = e.profile.clone();
        for v in &mut new[6..=8] {
            *v += 2.0;
        }
        let runs = changed_runs(e, &new, 0.5);
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.len(), 5, "one unchanged sample on each side");
        assert_eq!(r[0], [25.0, 400.5, 0.0]);
        assert_eq!(r[1], [30.0, 402.5, 0.0]);
        assert_eq!(r[4], [45.0, 400.5, 0.0]);
        assert!(changed_runs(e, &e.profile, 0.0).is_empty());
        assert!(changed_runs(e, &new[..3], 0.0).is_empty(), "another length");
    }

    #[test]
    fn the_marks_show_the_selection_the_changes_and_the_points() {
        let mut s = studio();
        assert_eq!(marks(&s), Marks::default());
        let id = s.world_doc.unwrap();
        let (lat, lon) = s
            .package
            .as_ref()
            .unwrap()
            .projection()
            .unproject([30.0, 0.0]);
        let ops = {
            let c = s.world().unwrap().content();
            vec![
                c.set_road(
                    RoadTarget::Way(7),
                    &RoadChange::Surface(Some("gravel".into())),
                )
                .unwrap(),
                c.remove_road(RoadTarget::Edge(300)).unwrap(),
                c.add_height_point(HeightPoint {
                    id: "p".into(),
                    at: Some([lat, lon]),
                    edge: Some(EdgeId(100)),
                    raise_m: Some(2.0),
                    ..Default::default()
                }),
            ]
        };
        for (k, op) in ops.into_iter().enumerate() {
            s.session
                .execute::<WorldDoc>(id, op, k as f64 * 5.0)
                .unwrap();
        }
        let d = s.session.get_mut::<WorldDoc>(id).unwrap();
        d.selection.select_only(WorldItem::Edge(100));
        let m = marks(&s);
        assert_eq!(m.selected, vec![0]);
        assert_eq!(m.changed, vec![0], "way 7 without the removed edge 300");
        assert_eq!(m.removed, vec![2]);
        assert_eq!(m.points.len(), 1);
        let p = &m.points[0];
        assert!((p.at[0] - 30.0).abs() < 1e-3 && p.at[1].abs() < 1e-3);
        assert_eq!(
            (p.radius_m, p.label.as_str(), p.selected),
            (100.0, "+2.0 m", false)
        );
        // Road edits on the whole way: its other edge is highlighted too.
        s.way_scope = true;
        assert_eq!(marks(&s).selected, vec![0, 2]);
        let d = s.session.get_mut::<WorldDoc>(id).unwrap();
        d.selection.select_only(WorldItem::HeightPoint("p".into()));
        let m = marks(&s);
        assert!(m.selected.is_empty() && m.points[0].selected);
    }
}
