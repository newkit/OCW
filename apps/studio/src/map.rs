//! The top-down 2D map of the open package (spec §14 Phase 16, 16A):
//! land use, buildings and roads **from the package itself** – never
//! online tiles (§4). North is up, east is right (package metres: x east,
//! z south, so the screen's y follows z).
//!
//! [`MapData`] is built once per package (pure, tested): the land use as
//! a colour raster (one texel per land-use cell, coarser for large
//! packages), the buildings as triangles, the roads of the road graph as
//! polylines, each with its bounding box for culling. [`MapView`] is the
//! map's pan and zoom (pure, tested). [`paint`] draws them with egui.

use bevy_egui::egui;
use ocw_world::landuse::LandClass;
use ocw_world::package::WorldPackage;

// The map's geometry, projection and styles are shared with the game's
// HUD navigation map (crate `ocw-map`, spec §3.2 *Navigation map*).
use ocw_map::overlaps;
pub use ocw_map::{MapData, MapView, format_length, nice_length, road_style};

/// Buildings are drawn from this zoom in (metres per point).
pub const BUILDINGS_BELOW_M_PER_PX: f64 = 6.0;

/// The colour of a land-use class on the map (the renderer's ground
/// colours, a little lighter).
pub fn land_rgb(class: LandClass) -> [u8; 3] {
    let c = ocw_render::package::land_colour(class);
    c.map(|v| ((v * 1.15).clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The map of `pkg` with the studio's land colours.
pub fn map_data(pkg: &WorldPackage) -> MapData {
    MapData::of(pkg, land_rgb)
}

/// Marks drawn over the map.
#[derive(Debug, Clone, Default)]
pub struct Overlay<'a> {
    /// The 3D camera: position (x, z) and yaw.
    pub camera: Option<([f64; 2], f64)>,
    /// Saved views: position (x, z) and name.
    pub views: Vec<([f64; 2], String)>,
    /// The package's road graph, for the patch's marks.
    pub graph: Option<&'a ocw_world::graph::RoadGraph>,
    /// What the world patch does (E5 step 2).
    pub marks: crate::overlay::Marks,
    /// The route of the Routes workspace (16D).
    pub route: Option<&'a crate::routes::RouteView>,
}

/// What the pointer did on the map this frame (package points).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MapInput {
    /// Double-clicked: fly there.
    pub fly: Option<[f64; 2]>,
    /// Clicked: select or place there.
    pub click: Option<[f64; 2]>,
    /// A drag that grabbed something (the route): where it started, where
    /// the pointer is, and how far it is.
    pub drag: Option<MapDrag>,
}

/// A drag of something on the map (not a pan).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapDrag {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub phase: DragPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    End,
}

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// Draws the map into `rect` and handles its input: drag to pan (or, where
/// `grab` says a press grabs something, a [`MapDrag`]), wheel to zoom,
/// click, double-click.
pub fn paint(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view: &mut MapView,
    data: &MapData,
    land: Option<&egui::TextureHandle>,
    overlay: &Overlay,
    grab: Option<&dyn Fn([f64; 2]) -> bool>,
) -> MapInput {
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let mid = [rect.center().x, rect.center().y];
    let grab_id = response.id.with("grab");
    let mut drag: Option<MapDrag> = None;
    let pointer = response
        .interact_pointer_pos()
        .map(|p| view.to_world([p.x, p.y], mid));
    if response.drag_started() {
        let grabbed = pointer.filter(|p| grab.is_some_and(|g| g(*p)));
        ui.memory_mut(|m| m.data.insert_temp(grab_id, grabbed));
        if let (Some(from), Some(to)) = (grabbed, pointer) {
            drag = Some(MapDrag {
                from,
                to,
                phase: DragPhase::Start,
            });
        }
    }
    let grabbed: Option<[f64; 2]> = ui
        .memory(|m| m.data.get_temp::<Option<[f64; 2]>>(grab_id))
        .flatten();
    if response.dragged() {
        match (grabbed, pointer) {
            (Some(from), Some(to)) => {
                if drag.is_none() {
                    drag = Some(MapDrag {
                        from,
                        to,
                        phase: DragPhase::Move,
                    });
                }
            }
            _ => {
                let d = response.drag_delta();
                view.pan([d.x, d.y]);
            }
        }
    }
    if response.drag_stopped() {
        if let Some(from) = grabbed {
            let to = response
                .hover_pos()
                .map(|p| view.to_world([p.x, p.y], mid))
                .or(pointer)
                .unwrap_or(from);
            drag = Some(MapDrag {
                from,
                to,
                phase: DragPhase::End,
            });
        }
        ui.memory_mut(|m| m.data.remove::<Option<[f64; 2]>>(grab_id));
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0
            && let Some(p) = response.hover_pos()
        {
            view.zoom_at(f64::from((scroll / 200.0).exp()), [p.x, p.y], mid);
        }
    }
    let at = |r: bool| {
        r.then(|| response.interact_pointer_pos())
            .flatten()
            .map(|p| view.to_world([p.x, p.y], mid))
    };
    let input = MapInput {
        fly: at(response.double_clicked()),
        click: at(response.clicked()),
        drag,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(38, 42, 46));
    let pos = |p: [f64; 2]| {
        let s = view.to_screen(p, mid);
        egui::pos2(s[0], s[1])
    };
    if let (Some(r), Some(tex)) = (&data.land, land) {
        let a = pos(r.origin);
        let b = pos([
            r.origin[0] + r.width as f64 * r.m_per_texel,
            r.origin[1] + r.height as f64 * r.m_per_texel,
        ]);
        painter.image(
            tex.id(),
            egui::Rect::from_min_max(a, b),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let vis = view.visible([rect.width(), rect.height()]);
    if view.m_per_px < BUILDINGS_BELOW_M_PER_PX {
        let mut mesh = egui::Mesh::default();
        let colour = egui::Color32::from_rgb(196, 182, 166);
        for b in data
            .buildings
            .iter()
            .filter(|b| overlaps(b.min, b.max, &vis))
        {
            let base = mesh.vertices.len() as u32;
            for p in &b.points {
                mesh.colored_vertex(pos([f64::from(p[0]), f64::from(p[1])]), colour);
            }
            for t in &b.triangles {
                mesh.add_triangle(base + t[0], base + t[1], base + t[2]);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    // Small roads first, the big ones over them.
    for pass in [false, true] {
        for r in data.roads.iter().filter(|r| overlaps(r.min, r.max, &vis)) {
            let (px, rgb, from) = road_style(r.class);
            let major = from.is_infinite();
            if major != pass || view.m_per_px > from {
                continue;
            }
            let width = px.max((f64::from(r.width_m) / view.m_per_px) as f32);
            let pts: Vec<egui::Pos2> = r
                .points
                .iter()
                .map(|p| pos([f64::from(p[0]), f64::from(p[1])]))
                .collect();
            painter.add(egui::Shape::line(
                pts,
                egui::Stroke::new(width, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])),
            ));
        }
    }
    if let Some(g) = overlay.graph {
        paint_marks(&painter, view, mid, g, &overlay.marks);
    }
    if let Some(r) = overlay.route {
        paint_route(&painter, view, mid, r);
    }
    for (p, name) in &overlay.views {
        let c = pos(*p);
        painter.circle_filled(c, 4.0, egui::Color32::from_rgb(255, 200, 40));
        painter.text(
            c + egui::vec2(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            name,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
    if let Some((p, yaw)) = overlay.camera {
        let c = pos(p);
        // Yaw counter-clockwise from north; north is up on the map.
        let dir = egui::vec2(-(yaw.sin() as f32), -(yaw.cos() as f32));
        let side = egui::vec2(-dir.y, dir.x);
        let tip = c + dir * 12.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, c - dir * 6.0 + side * 7.0, c - dir * 6.0 - side * 7.0],
            egui::Color32::from_rgb(255, 80, 60),
            egui::Stroke::new(1.0, egui::Color32::BLACK),
        ));
    }
    // The scale bar and the credit (spec §13: renders of OSM data).
    let bar_m = nice_length(view.m_per_px * 100.0);
    let bar_px = (bar_m / view.m_per_px) as f32;
    let y = rect.bottom() - 14.0;
    let x = rect.left() + 10.0;
    painter.line_segment(
        [egui::pos2(x, y), egui::pos2(x + bar_px, y)],
        egui::Stroke::new(2.0, egui::Color32::WHITE),
    );
    painter.text(
        egui::pos2(x, y - 3.0),
        egui::Align2::LEFT_BOTTOM,
        format_length(bar_m),
        egui::FontId::proportional(11.0),
        egui::Color32::WHITE,
    );
    painter.text(
        egui::pos2(rect.right() - 6.0, rect.bottom() - 4.0),
        egui::Align2::RIGHT_BOTTOM,
        "(c) OpenStreetMap contributors",
        egui::FontId::proportional(11.0),
        egui::Color32::from_gray(220),
    );
    input
}

/// The world patch's marks over the map: changed roads tinted, removed
/// ones struck through, the selection highlighted, height points with
/// their radius.
fn paint_marks(
    painter: &egui::Painter,
    view: &MapView,
    mid: [f32; 2],
    g: &ocw_world::graph::RoadGraph,
    m: &crate::overlay::Marks,
) {
    use crate::overlay::{CHANGED, POINT, REMOVED, SELECTED};
    let pos = |p: [f64; 2]| {
        let s = view.to_screen(p, mid);
        egui::pos2(s[0], s[1])
    };
    let width = |e: &ocw_world::graph::Edge, min: f32| min.max((e.width_m / view.m_per_px) as f32);
    let line = |i: usize, c: egui::Color32, min: f32| {
        let e = &g.edges[i];
        let pts: Vec<egui::Pos2> = e.points.iter().map(|p| pos(*p)).collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(width(e, min), c)));
    };
    for &i in &m.changed {
        line(i, rgb(CHANGED), 3.0);
    }
    for &i in &m.removed {
        let e = &g.edges[i];
        line(i, rgb(REMOVED).gamma_multiply(0.85), 2.5);
        // Struck through: a tick across the road every 18 points.
        let len = e.length_m();
        let step = (18.0 * view.m_per_px).max(2.0);
        let mut d = step * 0.5;
        while d < len {
            if let (Some(p), Some(q)) = (
                crate::pick::point_at(&e.points, d),
                crate::pick::point_at(&e.points, (d + 0.5).min(len)),
            ) {
                let (a, b) = (pos(p), pos(q));
                let dir = (b - a).normalized();
                let n = egui::vec2(-dir.y, dir.x) * (width(e, 2.5) * 0.5 + 4.0);
                painter.line_segment(
                    [a - n - dir * 3.0, a + n + dir * 3.0],
                    egui::Stroke::new(2.0, rgb(REMOVED)),
                );
            }
            d += step;
        }
    }
    for (k, &i) in m.selected.iter().enumerate() {
        let c = if k == 0 {
            rgb(SELECTED)
        } else {
            rgb(SELECTED).gamma_multiply(0.6)
        };
        line(i, c, 4.0);
    }
    for p in &m.points {
        let c = pos(p.at);
        let r = (p.radius_m / view.m_per_px) as f32;
        painter.circle_stroke(c, r, egui::Stroke::new(1.5, rgb(POINT)));
        painter.circle_filled(c, if p.selected { 6.0 } else { 4.5 }, rgb(POINT));
        if p.selected {
            painter.circle_stroke(c, 8.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
        }
        painter.text(
            c + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            format!("{} {}", p.id, p.label),
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
}

/// The route over the map (16D): the imported track grey with its
/// problem sections, the route coloured by gradient, the numbered
/// via-points (the start green, the finish red, the selected one ringed).
fn paint_route(
    painter: &egui::Painter,
    view: &MapView,
    mid: [f32; 2],
    r: &crate::routes::RouteView,
) {
    let pos = |p: [f64; 2]| {
        let s = view.to_screen(p, mid);
        egui::pos2(s[0], s[1])
    };
    if r.track.len() >= 2 {
        painter.add(egui::Shape::line(
            r.track.iter().map(|p| pos(*p)).collect(),
            egui::Stroke::new(1.5, egui::Color32::from_gray(150)),
        ));
    }
    for (kind, pts) in &r.problems {
        painter.add(egui::Shape::line(
            pts.iter().map(|p| pos(*p)).collect(),
            egui::Stroke::new(
                6.0,
                rgb(crate::routes_ui::problem_rgb(*kind)).gamma_multiply(0.85),
            ),
        ));
    }
    if r.line.len() >= 2 {
        let pts: Vec<egui::Pos2> = r.line.iter().map(|p| pos([p[0], p[2]])).collect();
        painter.add(egui::Shape::line(
            pts.clone(),
            egui::Stroke::new(
                7.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220),
            ),
        ));
        for (k, w) in pts.windows(2).enumerate() {
            let g = r.grades.get(k).copied().unwrap_or(0.0);
            painter.line_segment(
                [w[0], w[1]],
                egui::Stroke::new(4.0, rgb(crate::routes::grade_rgb(g))),
            );
        }
    }
    let last = r.vias.len().saturating_sub(1);
    for (i, v) in r.vias.iter().enumerate() {
        let c = pos(*v);
        let fill = if i == 0 {
            egui::Color32::from_rgb(60, 190, 90)
        } else if i == last {
            egui::Color32::from_rgb(220, 70, 60)
        } else {
            egui::Color32::from_rgb(40, 120, 230)
        };
        painter.circle_filled(c, 6.5, fill);
        painter.circle_stroke(c, 6.5, egui::Stroke::new(1.5, egui::Color32::WHITE));
        if r.selected == Some(i) {
            painter.circle_stroke(
                c,
                10.0,
                egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 230, 60)),
            );
        }
        painter.text(
            c,
            egui::Align2::CENTER_CENTER,
            format!("{}", i + 1),
            egui::FontId::proportional(9.0),
            egui::Color32::WHITE,
        );
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;

    use ocw_map::data::MAX_RASTER;
    use ocw_world::buildings::{Building, BuildingKind, Roof};
    use ocw_world::graph::RoadClass;
    use ocw_world::landuse::{LandUse, LandUseTile};
    use ocw_world::package::{Tile, TileKey};

    use super::*;

    /// A made-up package: two tiles of 100 m side by side, land use in
    /// the first (forest left, water right), one building, no graph.
    pub fn package() -> WorldPackage {
        let n = 3;
        let tile = |x| Tile {
            key: TileKey { x, z: 0 },
            heights: vec![400.0; n * n],
            roads: vec![ocw_world::package::RoadStrip {
                class: RoadClass::Secondary,
                width_m: 6.0,
                points: vec![
                    [x as f32 * 100.0, 400.0, 50.0],
                    [x as f32 * 100.0 + 100.0, 400.0, 50.0],
                ],
                surface: ocw_world::RoadSurface::Asphalt,
                dress: ocw_world::RoadDress::of_class(RoadClass::Secondary, 6.0),
                lit: ocw_world::RoadLit::Unknown,
                condition: Default::default(),
            }],
        };
        let classes = vec![
            LandClass::Forest,
            LandClass::Water,
            LandClass::Forest,
            LandClass::Water,
        ];
        WorldPackage {
            name: "testland".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: n as u32,
            tiles: BTreeMap::from([
                (TileKey { x: 0, z: 0 }, tile(0)),
                (TileKey { x: 1, z: 0 }, tile(1)),
            ]),
            graph: None,
            landuse: Some(LandUse {
                cells: 2,
                tiles: BTreeMap::from([(
                    TileKey { x: 0, z: 0 },
                    LandUseTile {
                        classes,
                        water: Vec::new(),
                    },
                )]),
            }),
            buildings: Some(BTreeMap::from([(
                TileKey { x: 0, z: 0 },
                vec![Building {
                    kind: BuildingKind::House,
                    outline: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
                    holes: Vec::new(),
                    base_y: 399.0,
                    eave_y: 406.0,
                    roof: Roof::Flat,
                    tags: Default::default(),
                    parts: Default::default(),
                }],
            )])),
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        }
    }

    #[test]
    fn the_map_holds_land_use_buildings_and_roads_of_the_package() {
        let m = map_data(&package());
        assert_eq!((m.min, m.max), ([0.0, 0.0], [200.0, 100.0]));
        let r = m.land.as_ref().unwrap();
        assert_eq!((r.width, r.height, r.m_per_texel), (4, 2, 50.0));
        let px = |u: usize, v: usize| &r.rgba[(v * r.width + u) * 4..(v * r.width + u) * 4 + 4];
        assert_eq!(px(0, 0)[..3], land_rgb(LandClass::Forest));
        assert_eq!(px(1, 1)[..3], land_rgb(LandClass::Water));
        // The second tile has no land use: the default ground, opaque.
        assert_eq!(
            px(3, 0),
            [land_rgb(LandClass::Default).as_slice(), &[255]].concat()
        );
        assert_eq!(m.roads.len(), 2, "the tiles' strips without a graph");
        assert_eq!(m.roads[1].min, [100.0, 50.0]);
        assert_eq!(m.buildings.len(), 1);
        assert_eq!(m.buildings[0].triangles.len(), 2);
    }

    #[test]
    fn a_large_package_gets_a_coarser_raster() {
        let mut p = package();
        p.landuse.as_mut().unwrap().cells = 3000;
        let t = p
            .landuse
            .as_mut()
            .unwrap()
            .tiles
            .values_mut()
            .next()
            .unwrap();
        t.classes = vec![LandClass::Meadow; 3000 * 3000];
        let r = map_data(&p).land.unwrap();
        assert!(r.width <= MAX_RASTER && r.height <= MAX_RASTER);
        assert_eq!(r.width, 2000, "6000 cells at stride 3");
        assert!((r.width as f64 * r.m_per_texel - 200.0).abs() < 1e-9);
    }
}
