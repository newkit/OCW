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
use ocw_world::graph::RoadClass;
use ocw_world::landuse::LandClass;
use ocw_world::package::{TileKey, WorldPackage};

/// The land-use raster is at most this many texels per side (egui's
/// largest texture side).
pub const MAX_RASTER: usize = 2048;
/// Zoom limits (metres per screen point).
pub const MIN_M_PER_PX: f64 = 0.05;
pub const MAX_M_PER_PX: f64 = 500.0;
/// Buildings are drawn from this zoom in (metres per point).
pub const BUILDINGS_BELOW_M_PER_PX: f64 = 6.0;

/// The map's pan and zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    /// The package point at the centre of the map (x, z).
    pub centre: [f64; 2],
    /// Metres per screen point.
    pub m_per_px: f64,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            centre: [0.0; 2],
            m_per_px: 10.0,
        }
    }
}

impl MapView {
    /// The view that shows `min`…`max` whole in a map of `size` points.
    pub fn fit(min: [f64; 2], max: [f64; 2], size: [f32; 2]) -> Self {
        let w = (max[0] - min[0]).max(1.0);
        let h = (max[1] - min[1]).max(1.0);
        let m = (w / f64::from(size[0].max(1.0))).max(h / f64::from(size[1].max(1.0))) * 1.05;
        Self {
            centre: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
            m_per_px: m.clamp(MIN_M_PER_PX, MAX_M_PER_PX),
        }
    }

    /// A package point on the screen (`mid`: the map's centre on screen).
    pub fn to_screen(self, p: [f64; 2], mid: [f32; 2]) -> [f32; 2] {
        [
            mid[0] + ((p[0] - self.centre[0]) / self.m_per_px) as f32,
            mid[1] + ((p[1] - self.centre[1]) / self.m_per_px) as f32,
        ]
    }

    /// The package point under a screen point.
    pub fn to_world(self, s: [f32; 2], mid: [f32; 2]) -> [f64; 2] {
        [
            self.centre[0] + f64::from(s[0] - mid[0]) * self.m_per_px,
            self.centre[1] + f64::from(s[1] - mid[1]) * self.m_per_px,
        ]
    }

    /// Zooms by `factor` (> 1: closer) keeping the point under `s` still.
    pub fn zoom_at(&mut self, factor: f64, s: [f32; 2], mid: [f32; 2]) {
        let before = self.to_world(s, mid);
        self.m_per_px = (self.m_per_px / factor).clamp(MIN_M_PER_PX, MAX_M_PER_PX);
        let after = self.to_world(s, mid);
        self.centre[0] += before[0] - after[0];
        self.centre[1] += before[1] - after[1];
    }

    /// Moves the map with a drag of `d` points (the ground follows the
    /// pointer).
    pub fn pan(&mut self, d: [f32; 2]) {
        self.centre[0] -= f64::from(d[0]) * self.m_per_px;
        self.centre[1] -= f64::from(d[1]) * self.m_per_px;
    }

    /// The package area a map of `size` points shows: (min, max).
    pub fn visible(&self, size: [f32; 2]) -> ([f64; 2], [f64; 2]) {
        let hx = f64::from(size[0]) * 0.5 * self.m_per_px;
        let hz = f64::from(size[1]) * 0.5 * self.m_per_px;
        (
            [self.centre[0] - hx, self.centre[1] - hz],
            [self.centre[0] + hx, self.centre[1] + hz],
        )
    }
}

/// The land use as colours, row-major along +z.
#[derive(Debug, Clone, PartialEq)]
pub struct LandRaster {
    /// The package point of texel (0, 0)'s corner.
    pub origin: [f64; 2],
    pub m_per_texel: f64,
    pub width: usize,
    pub height: usize,
    /// RGBA, unmultiplied; transparent outside the package's tiles.
    pub rgba: Vec<u8>,
}

/// A road of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRoad {
    pub class: RoadClass,
    pub width_m: f32,
    pub points: Vec<[f32; 2]>,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// A building of the map: its footprint as triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct MapBuilding {
    pub points: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// What the map draws of a package.
#[derive(Debug, Clone, PartialEq)]
pub struct MapData {
    /// The tiles' extent (x, z).
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub land: Option<LandRaster>,
    pub roads: Vec<MapRoad>,
    pub buildings: Vec<MapBuilding>,
}

fn bbox(points: impl Iterator<Item = [f32; 2]>) -> ([f32; 2], [f32; 2]) {
    points.fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    )
}

/// The colour of a land-use class on the map (the renderer's ground
/// colours, a little lighter).
pub fn land_rgb(class: LandClass) -> [u8; 3] {
    let c = ocw_render::package::land_colour(class);
    c.map(|v| ((v * 1.15).clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl MapData {
    /// The map of `pkg` (its own tiles; neighbours are not drawn yet).
    pub fn of(pkg: &WorldPackage) -> Self {
        let size = f64::from(pkg.tile_size_m);
        let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
        let (min, max) = if keys.is_empty() {
            ([0.0; 2], [0.0; 2])
        } else {
            let kx0 = keys.iter().map(|k| k.x).min().unwrap_or(0);
            let kx1 = keys.iter().map(|k| k.x).max().unwrap_or(0);
            let kz0 = keys.iter().map(|k| k.z).min().unwrap_or(0);
            let kz1 = keys.iter().map(|k| k.z).max().unwrap_or(0);
            (
                [f64::from(kx0) * size, f64::from(kz0) * size],
                [f64::from(kx1 + 1) * size, f64::from(kz1 + 1) * size],
            )
        };
        let land = (!keys.is_empty()).then(|| land_raster(pkg, min, max));
        let roads = match &pkg.graph {
            Some(g) => g
                .edges
                .iter()
                .filter(|e| e.points.len() >= 2)
                .map(|e| {
                    let points: Vec<[f32; 2]> = e
                        .points
                        .iter()
                        .map(|p| [p[0] as f32, p[1] as f32])
                        .collect();
                    let (min, max) = bbox(points.iter().copied());
                    MapRoad {
                        class: e.class,
                        width_m: e.width_m as f32,
                        points,
                        min,
                        max,
                    }
                })
                .collect(),
            // Version 1 packages: the tiles' road strips.
            None => pkg
                .tiles
                .values()
                .flat_map(|t| &t.roads)
                .filter(|s| s.points.len() >= 2)
                .map(|s| {
                    let points: Vec<[f32; 2]> = s.points.iter().map(|p| [p[0], p[2]]).collect();
                    let (min, max) = bbox(points.iter().copied());
                    MapRoad {
                        class: s.class,
                        width_m: s.width_m,
                        points,
                        min,
                        max,
                    }
                })
                .collect(),
        };
        let buildings = pkg
            .buildings
            .iter()
            .flat_map(|b| b.values())
            .flatten()
            .filter(|b| b.outline.len() >= 3)
            .map(|b| {
                let mut points = b.outline.clone();
                for h in &b.holes {
                    points.extend_from_slice(h);
                }
                let triangles = ocw_world::buildings::triangulate(&b.outline, &b.holes);
                let (min, max) = bbox(b.outline.iter().copied());
                MapBuilding {
                    points,
                    triangles,
                    min,
                    max,
                }
            })
            .collect();
        Self {
            min,
            max,
            land,
            roads,
            buildings,
        }
    }
}

/// The land-use raster over the tile range `min`…`max`: one texel per
/// land-use cell (every `stride`-th for large packages), tiles without
/// land use in the default ground colour, no tile transparent.
fn land_raster(pkg: &WorldPackage, min: [f64; 2], max: [f64; 2]) -> LandRaster {
    let size = f64::from(pkg.tile_size_m);
    let cells = pkg.landuse.as_ref().map_or(1, |l| l.cells.max(1)) as usize;
    let tiles_x = ((max[0] - min[0]) / size).round() as usize;
    let tiles_z = ((max[1] - min[1]) / size).round() as usize;
    let total = tiles_x.max(tiles_z) * cells;
    let stride = total.div_ceil(MAX_RASTER).max(1);
    let width = (tiles_x * cells).div_ceil(stride).max(1);
    let height = (tiles_z * cells).div_ceil(stride).max(1);
    let k0 = TileKey::containing([min[0] + 0.5, min[1] + 0.5], size);
    let mut rgba = vec![0u8; width * height * 4];
    let default = land_rgb(LandClass::Default);
    for v in 0..height {
        for u in 0..width {
            let (gx, gz) = (u * stride, v * stride);
            let key = TileKey {
                x: k0.x + (gx / cells) as i32,
                z: k0.z + (gz / cells) as i32,
            };
            if !pkg.tiles.contains_key(&key) {
                continue;
            }
            let class = pkg
                .landuse
                .as_ref()
                .and_then(|l| l.tiles.get(&key))
                .and_then(|t| t.classes.get((gz % cells) * cells + gx % cells))
                .copied();
            let rgb = class.map_or(default, land_rgb);
            let i = (v * width + u) * 4;
            rgba[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    LandRaster {
        origin: min,
        m_per_texel: size / cells as f64 * stride as f64,
        width,
        height,
        rgba,
    }
}

/// How a road class is drawn: the narrowest stroke (points), its colour,
/// and from which zoom on (metres per point) it is drawn at all.
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

fn overlaps(min: [f32; 2], max: [f32; 2], vis: &([f64; 2], [f64; 2])) -> bool {
    f64::from(max[0]) >= vis.0[0]
        && f64::from(min[0]) <= vis.1[0]
        && f64::from(max[1]) >= vis.0[1]
        && f64::from(min[1]) <= vis.1[1]
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
            egui::Stroke::new(6.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
        ));
        for (k, w) in pts.windows(2).enumerate() {
            let g = r.grades.get(k).copied().unwrap_or(0.0);
            painter.line_segment(
                [w[0], w[1]],
                egui::Stroke::new(3.5, rgb(crate::routes::grade_rgb(g))),
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
        painter.circle_filled(c, 8.0, fill);
        painter.circle_stroke(c, 8.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
        if r.selected == Some(i) {
            painter.circle_stroke(
                c,
                12.0,
                egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 230, 60)),
            );
        }
        painter.text(
            c,
            egui::Align2::CENTER_CENTER,
            format!("{}", i + 1),
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
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

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;

    use ocw_world::buildings::{Building, BuildingKind, Roof};
    use ocw_world::landuse::{LandUse, LandUseTile};
    use ocw_world::package::Tile;

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
        let m = MapData::of(&package());
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
        let r = MapData::of(&p).land.unwrap();
        assert!(r.width <= MAX_RASTER && r.height <= MAX_RASTER);
        assert_eq!(r.width, 2000, "6000 cells at stride 3");
        assert!((r.width as f64 * r.m_per_texel - 200.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer_and_pan_follows_it() {
        let mut v = MapView::fit([0.0, 0.0], [2000.0, 1000.0], [400.0, 400.0]);
        assert_eq!(v.centre, [1000.0, 500.0]);
        assert!((v.m_per_px - 2000.0 / 400.0 * 1.05).abs() < 1e-9);
        let mid = [200.0, 200.0];
        let s = [300.0, 120.0];
        let under = v.to_world(s, mid);
        v.zoom_at(2.0, s, mid);
        let again = v.to_world(s, mid);
        assert!((under[0] - again[0]).abs() < 1e-6 && (under[1] - again[1]).abs() < 1e-6);
        let p = v.to_world([250.0, 250.0], mid);
        v.pan([10.0, -5.0]);
        let moved = v.to_screen(p, mid);
        assert!((moved[0] - 260.0).abs() < 1e-3 && (moved[1] - 245.0).abs() < 1e-3);
        v.zoom_at(1e9, s, mid);
        assert_eq!(v.m_per_px, MIN_M_PER_PX);
        let (lo, hi) = v.visible([100.0, 50.0]);
        assert!((hi[0] - lo[0] - 100.0 * MIN_M_PER_PX).abs() < 1e-9);
    }

    #[test]
    fn scale_bar_lengths_are_round() {
        assert_eq!(nice_length(730.0), 500.0);
        assert_eq!(nice_length(2100.0), 2000.0);
        assert_eq!(nice_length(1.0), 1.0);
        assert_eq!(nice_length(0.0), 1.0);
        assert_eq!(format_length(500.0), "500 m");
        assert_eq!(format_length(2000.0), "2 km");
        assert!(road_style(RoadClass::Primary).2.is_infinite());
    }
}
