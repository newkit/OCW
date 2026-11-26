//! The Routes workspace (spec §14 Phase 16, 16D, E1; external planners
//! E2): the route document of the open package (`ocw_edit::route`), what
//! the UI and the console send to it ([`RouteCommand`]), the road graph
//! and planner it is edited with, the BRouter requests (off the UI thread,
//! one at a time, only to the server the user entered), the files in and
//! out, and *Test ride* (`ocw-desktop --world … --route …`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use ocw_core::RiderProfile;
use ocw_edit::route::{
    Editor, Imported, PartChoice, ProblemKind, RouteDoc, RouteOp, RoutePackage, RouteProfile,
    RouteSet, Track, TrackPart, TrackPoint, chosen_track, formats, geometry_of_graph_route,
    import_planned, import_track, route_profile, route_validators, track_parts,
};
use ocw_edit::{DocId, Document, Severity};
use ocw_sim::estimate::{ProfilePoint, RideEstimate, ride_time};
use ocw_world::formats::TrackFormat;
use ocw_world::gpx::Gpx;
use ocw_world::matching::MatchParams;
use ocw_world::network::RoadNetwork;
use ocw_world::plan::brouter::{BRouterClient, BRouterServer, HttpAnswer, HttpGet};
use ocw_world::plan::{EdgeCosts, PlanError, PlannedTrack, Profile, RoutePlanner, steps_points};
use ocw_world::road_surface::RoadSurface;
use serde::{Deserialize, Serialize};

use crate::edit::{Studio, shown_path};
use crate::patch::StudioPackage;

/// Which planner plans new legs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlannerKind {
    /// On the package's road graph, offline (the default).
    #[default]
    Builtin,
    /// The BRouter server the user entered (16H Q3).
    Brouter,
}

/// The planner settings (`route.planner`, `route.planner.brouter_url`,
/// `route.planner.brouter_profile`; 16H Q3: the server has no default),
/// kept in `<config dir>/ocw/studio/planner.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerSettings {
    #[serde(rename = "route.planner", default)]
    pub planner: PlannerKind,
    #[serde(rename = "route.planner.brouter_url", default)]
    pub brouter_url: String,
    #[serde(rename = "route.planner.brouter_profile", default = "trekking")]
    pub brouter_profile: String,
    /// The power the expected riding time is for (W; 16D *Views*).
    #[serde(rename = "route.estimate.power_w", default = "default_power")]
    pub power_w: f64,
}

fn trekking() -> String {
    "trekking".into()
}

/// The expected time's power until the user sets one (W).
pub const DEFAULT_POWER_W: f64 = 200.0;
/// The powers the field allows (W).
pub const POWER_RANGE_W: (f64, f64) = (50.0, 1000.0);

fn default_power() -> f64 {
    DEFAULT_POWER_W
}

impl Default for PlannerSettings {
    fn default() -> Self {
        Self {
            planner: PlannerKind::Builtin,
            brouter_url: String::new(),
            brouter_profile: trekking(),
            power_w: DEFAULT_POWER_W,
        }
    }
}

impl PlannerSettings {
    pub fn path() -> Option<PathBuf> {
        let base = match std::env::var_os("OCW_CONFIG_DIR") {
            Some(d) => PathBuf::from(d),
            None => dirs::config_dir()?.join("ocw"),
        };
        Some(base.join("studio").join("planner.json"))
    }

    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: Option<&Path>) -> Result<(), String> {
        let Some(p) = path else {
            return Ok(());
        };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(p, text + "\n").map_err(|e| format!("{}: {e}", shown_path(p)))
    }
}

/// A BRouter request on its way.
pub struct PlanJob {
    /// (A `Mutex` makes the resource `Sync`; only the UI thread polls it.)
    rx: Mutex<Receiver<Result<PlannedTrack, PlanError>>>,
    /// The via-points asked for (x, z).
    pub vias: Vec<[f64; 2]>,
    pub server: String,
}

/// What the views draw of the route (recomputed when it changes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteView {
    /// The document and history state it was made for.
    pub key: Option<(DocId, u64)>,
    /// The route's points (x, height, z) and the grade from each to the
    /// next.
    pub line: Vec<[f64; 3]>,
    pub grades: Vec<f64>,
    pub vias: Vec<[f64; 2]>,
    pub selected: Option<usize>,
    /// The imported track (x, z) and its problem sections.
    pub track: Vec<[f64; 2]>,
    pub problems: Vec<(ProblemKind, Vec<[f64; 2]>)>,
    pub profile: Option<RouteProfile>,
    /// The imported track's elevation (distance, m) for comparison.
    pub track_ele: Vec<[f64; 2]>,
    /// Why there is no line (a broken route).
    pub error: Option<String>,
}

/// The rider the expected time is for: the active local rider of the
/// game (`<config dir>/ocw/ocw.toml`, section `[riders]`; else the old
/// `riders.toml`), else the default rider (75 kg, road bike).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRider {
    pub name: String,
    pub weight_kg: f64,
    /// The curated profile of the rider's bike with their weight (2.5).
    pub physics: RiderProfile,
    /// Where it was read from (shown), `None` for the default rider.
    pub from: Option<String>,
}

impl Default for LocalRider {
    fn default() -> Self {
        let r = ocw_client::riders::Rider::default();
        Self {
            name: r.name.clone(),
            weight_kg: r.weight_kg,
            physics: r.physics(),
            from: None,
        }
    }
}

impl LocalRider {
    /// The active rider of the game's config in `dir` (`ocw.toml`, else
    /// `riders.toml`); the default rider when there is none.
    pub fn load(dir: Option<&Path>) -> Self {
        let Some(dir) = dir else {
            return Self::default();
        };
        let config = dir.join("ocw.toml");
        let section = std::fs::read_to_string(&config)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|mut t| match t.remove("riders") {
                Some(toml::Value::Table(r)) => toml::to_string(&r).ok(),
                _ => None,
            });
        let (text, from) = match section {
            Some(t) => (t, config),
            None => {
                let old = dir.join("riders.toml");
                match std::fs::read_to_string(&old) {
                    Ok(t) => (t, old),
                    Err(_) => return Self::default(),
                }
            }
        };
        match ocw_client::riders::Riders::parse(&text) {
            Ok((riders, _)) if !riders.riders.is_empty() => {
                let r = riders.active();
                Self {
                    name: r.name.clone(),
                    weight_kg: r.weight_kg,
                    physics: r.physics(),
                    from: from
                        .file_name()
                        .map(|f| format!("from {}", f.to_string_lossy())),
                }
            }
            _ => Self::default(),
        }
    }

    /// "Ada Rider, 68 kg, Road".
    pub fn summary(&self) -> String {
        format!(
            "{}, {:.0} kg, {}",
            self.name,
            self.weight_kg,
            self.physics.bike.profile().name
        )
    }
}

/// The game's config folder (`OCW_CONFIG_DIR`, else the platform's config
/// folder + `ocw`), where its riders are.
pub fn game_config_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::config_dir()?.join("ocw")),
    }
}

/// The expected riding time of the route's profile at `power_w` for
/// `rider` (`ocw_sim::estimate`, the game's physics).
pub fn estimate(
    profile: &RouteProfile,
    rider: &RiderProfile,
    power_w: f64,
) -> Option<RideEstimate> {
    let points: Vec<ProfilePoint> = profile
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| ProfilePoint {
            distance_m: s[0],
            height_m: s[1],
            surface: profile
                .sample_surfaces
                .get(i)
                .copied()
                .unwrap_or_default()
                .physics(),
        })
        .collect();
    ride_time(rider, power_w, &points)
}

/// A track file with several parts, waiting for the user to choose one
/// (or all joined) before it is imported (16D).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingImport {
    /// The file, as shown, and its stem (the route's name without one).
    pub shown: String,
    pub stem: String,
    pub format: TrackFormat,
    pub gpx: Gpx,
    pub parts: Vec<TrackPart>,
}

/// The colour of part `i` of a pending import (list and map).
pub fn part_rgb(i: usize) -> [u8; 3] {
    const C: [[u8; 3]; 6] = [
        [80, 200, 255],
        [255, 150, 60],
        [190, 120, 255],
        [120, 230, 110],
        [255, 100, 160],
        [240, 220, 80],
    ];
    C[i % C.len()]
}

/// The Routes workspace's state in the studio.
pub struct RouteTools {
    pub doc: Option<DocId>,
    /// The open package's road graph, and its id, name, origin.
    pub net: Option<Arc<RoadNetwork>>,
    pub package: RoutePackage,
    pub costs: Option<EdgeCosts>,
    pub settings: PlannerSettings,
    pub settings_path: Option<PathBuf>,
    /// The user agreed that via-points go to the BRouter server (§12).
    pub brouter_consent: bool,
    pub job: Option<PlanJob>,
    /// The via-point being dragged (a drag of the route inserts one).
    pub dragging: Option<usize>,
    pub view: RouteView,
    /// Where the route was saved / exported last.
    pub file: Option<PathBuf>,
    /// Where *Save route* writes when the package folder is not writable
    /// (`<data dir>/ocw/routes`; `None`: no data directory).
    pub fallback_dir: Option<PathBuf>,
    pub test_ride: Option<std::process::Child>,
    /// The package's road surface of every edge (by index; empty: the
    /// classes' usual ones): the planner's costs and the figures.
    pub surfaces: Arc<Vec<RoadSurface>>,
    /// The rider and the expected time of the route at the settings' power.
    pub rider: LocalRider,
    pub estimate: Option<RideEstimate>,
    /// What the estimate was made for: the view's key and the power.
    estimate_for: Option<(Option<(DocId, u64)>, u64)>,
    /// A track file with several parts, waiting for the choice.
    pub pending: Option<PendingImport>,
}

impl Default for RouteTools {
    fn default() -> Self {
        let settings_path = PlannerSettings::path();
        Self {
            doc: None,
            net: None,
            package: RoutePackage::default(),
            costs: None,
            settings: PlannerSettings::load(settings_path.as_deref()),
            settings_path,
            brouter_consent: false,
            job: None,
            dragging: None,
            view: RouteView::default(),
            file: None,
            fallback_dir: crate::edit::data_routes_dir(),
            test_ride: None,
            surfaces: Arc::new(Vec::new()),
            rider: LocalRider::load(game_config_dir().as_deref()),
            estimate: None,
            estimate_for: None,
            pending: None,
        }
    }
}

impl RouteTools {
    /// Tools for tests: no settings file.
    #[cfg(test)]
    pub fn detached() -> Self {
        Self {
            settings: PlannerSettings::default(),
            settings_path: None,
            rider: LocalRider::default(),
            fallback_dir: None,
            ..Self::default()
        }
    }

    /// The package's road graph for routes (a new package closes the old
    /// route: it is on other roads).
    pub fn set_package(&mut self, package: &StudioPackage, id: &str) {
        self.net = package
            .graph()
            .map(|g| Arc::new(RoadNetwork::new(g.clone())));
        self.package = RoutePackage {
            id: id.into(),
            name: package.pkg.name.clone(),
            fingerprint: self.net.as_ref().map_or(0, |n| n.fingerprint()),
            origin: package.pkg.origin,
        };
        self.costs = None;
        self.view = RouteView::default();
        self.dragging = None;
        self.pending = None;
        self.estimate = None;
        self.estimate_for = None;
        // The package's measured surfaces, for the costs (16D).
        self.surfaces = Arc::new(package.edge_surfaces());
    }

    /// The package's surface of edge `e` (else its class's usual one).
    pub fn surface(&self, net: &RoadNetwork, e: usize) -> RoadSurface {
        self.surfaces
            .get(e)
            .copied()
            .unwrap_or_else(|| RoadSurface::of_class(net.edge(e).class))
    }

    /// Re-makes the expected time when the route's view or the power
    /// changed.
    pub fn update_estimate(&mut self) {
        let key = (self.view.key, self.settings.power_w.to_bits());
        if self.estimate_for == Some(key) {
            return;
        }
        self.estimate = self
            .view
            .profile
            .as_ref()
            .and_then(|p| estimate(p, &self.rider.physics, self.settings.power_w));
        self.estimate_for = Some(key);
    }

    fn net(&self) -> Result<Arc<RoadNetwork>, String> {
        self.net
            .clone()
            .ok_or_else(|| "the open package has no road graph".into())
    }

    /// The built-in planner's costs for `profile` (cached).
    fn costs_for(&mut self, profile: Profile) -> Result<&EdgeCosts, String> {
        let net = self.net()?;
        if self.costs.as_ref().is_none_or(|c| c.profile != profile) {
            let surfaces = self.surfaces.clone();
            let surface = |e: usize| {
                surfaces
                    .get(e)
                    .copied()
                    .unwrap_or_else(|| RoadSurface::of_class(net.edge(e).class))
            };
            self.costs = Some(EdgeCosts::new(&net, profile, Some(&surface)));
        }
        Ok(self.costs.as_ref().expect("set"))
    }
}

/// What the Routes workspace's UI and console ask for.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteCommand {
    New(Option<String>),
    /// Imports a GPX / TCX / GeoJSON / FIT file (matched onto the graph)
    /// or opens a graph route file; a file of several parts waits for
    /// [`RouteCommand::ImportPart`].
    Import(String),
    /// Imports one part of the waiting file (or all joined).
    ImportPart(PartChoice),
    /// Forgets the waiting file.
    CancelImport,
    /// The power of the expected time (W).
    Power(f64),
    /// Opens the package's route `i`.
    PackageRoute(usize),
    /// A click: selects the via-point there, else adds one at the end.
    Click([f64; 2], f64),
    AddVia([f64; 2]),
    InsertVia(usize, [f64; 2]),
    MoveVia(usize, [f64; 2]),
    RemoveVia(usize),
    SelectVia(Option<usize>),
    /// A route through these points from scratch.
    Plan(Vec<[f64; 2]>),
    /// A drag starts at `grab` (a via-point within `tol` m is moved, else
    /// the route is grabbed there), continues to …, and ends.
    DragStart {
        grab: [f64; 2],
        to: [f64; 2],
        tol: f64,
    },
    DragTo([f64; 2]),
    DragEnd,
    Reverse,
    Loop(bool),
    OutAndBack,
    CutStart(usize),
    CutEnd(usize),
    Split(usize),
    Profile(String),
    Rename(String),
    Describe(String),
    Planner(PlannerKind),
    BRouterUrl(String),
    BRouterProfile(String),
    BRouterConsent(bool),
    /// Plans the via-points with BRouter (in the background).
    PlanExternal,
    /// Writes the route as a file (format by the extension, or given).
    Export(String, Option<TrackFormat>),
    /// Saves the graph route (`<package dir>/routes/<name>.ocwroute`, or
    /// `<data dir>/ocw/routes/` when the package folder is not writable,
    /// or here) and marks the document saved.
    Save(Option<String>),
    /// Saves and starts `ocw-desktop --world … --route …`.
    TestRide,
    Seal,
    /// Fits the 2D map to the route.
    Fit,
}

impl Studio {
    pub fn route(&self) -> Option<&Document<RouteDoc>> {
        self.session.get(self.routes.doc?)
    }

    fn route_mut(&mut self) -> Result<&mut Document<RouteDoc>, String> {
        let id = self
            .routes
            .doc
            .ok_or("no route is open (Routes → New route)")?;
        self.session
            .get_mut::<RouteDoc>(id)
            .ok_or_else(|| "no route is open".into())
    }
}

/// Opens a new route document (closing the one open).
fn open_route(
    studio: &mut Studio,
    doc: RouteDoc,
    source: Option<String>,
    now_s: f64,
) -> Result<(), String> {
    let net = studio.routes.net()?;
    if let Some(old) = studio.routes.doc.take() {
        if let Some(store) = studio.store.as_deref_mut() {
            studio.session.autosave(store, now_s, 0.0);
        }
        studio.session.close(old);
    }
    let name = doc.name.clone();
    let id = studio.session.open(|id| {
        let mut d = Document::new(id, name, doc);
        if let Some(s) = source {
            d = d.with_source(s);
        }
        for v in route_validators(net) {
            d = d.with_validator(v);
        }
        d
    });
    studio.routes.doc = Some(id);
    studio.routes.dragging = None;
    studio.routes.view = RouteView::default();
    Ok(())
}

fn execute(studio: &mut Studio, op: RouteOp, now_s: f64, seal: bool) -> Result<(), String> {
    let d = studio.route_mut()?;
    d.execute(op, now_s).map_err(|e| e.to_string())?;
    if seal {
        d.seal();
    }
    Ok(())
}

/// The editor of the open route's profile.
fn with_editor<T>(
    studio: &mut Studio,
    f: impl FnOnce(&Editor, &RouteDoc) -> Result<T, ocw_edit::EditError>,
) -> Result<T, String> {
    let net = studio.routes.net()?;
    let doc = studio.route().ok_or("no route is open")?.content().clone();
    let profile = Profile::parse(&doc.profile).unwrap_or_default();
    let costs = studio.routes.costs_for(profile)?;
    let ed = Editor { net: &net, costs };
    f(&ed, &doc).map_err(|e| e.to_string())
}

fn n(i: usize) -> usize {
    i + 1
}

/// Reads and imports a file into a new route document.
fn import_file(studio: &mut Studio, path: &str, now_s: f64) -> Result<String, String> {
    let p = crate::edit::expand_home(path.trim());
    let shown = shown_path(&p);
    let bytes = std::fs::read(&p).map_err(|e| format!("{shown}: {e}"))?;
    let net = studio.routes.net()?;
    let file_name = p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = p
        .file_stem()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported route".into());
    let format = TrackFormat::of_name(&file_name).or_else(|| TrackFormat::sniff(&bytes));
    if format == Some(TrackFormat::GraphRoute) {
        let text = String::from_utf8(bytes).map_err(|_| format!("{shown}: not text"))?;
        let (gr, _) = ocw_world::GraphRoute::parse(&text).map_err(|e| format!("{shown}: {e}"))?;
        let geometry = geometry_of_graph_route(&net, &gr).map_err(|e| format!("{shown}: {e}"))?;
        let mut doc = RouteDoc::new(studio.routes.package.clone(), gr.name.clone());
        doc.geometry = geometry;
        open_route(studio, doc, Some(p.display().to_string()), now_s)?;
        studio.routes.file = Some(p);
        return Ok(format!("opened the graph route '{}' ({shown})", gr.name));
    }
    let (gpx, format) =
        formats::read_parts(&file_name, &bytes).map_err(|e| format!("{shown}: {e}"))?;
    let parts = track_parts(&gpx, &net, &studio.routes.package, &MatchParams::default());
    let pending = PendingImport {
        shown: shown.clone(),
        stem,
        format,
        gpx,
        parts,
    };
    if pending.parts.len() > 1 {
        // Several segments: the user chooses one, or all joined (16D).
        let n = pending.parts.len();
        studio.routes.pending = Some(pending);
        return Ok(format!(
            "{shown} has {n} parts: choose one, or all joined in the Routes panel"
        ));
    }
    import_part(studio, &pending, PartChoice::All, now_s)
}

/// Imports `choice` of a read track file into a new route document.
fn import_part(
    studio: &mut Studio,
    p: &PendingImport,
    choice: PartChoice,
    now_s: f64,
) -> Result<String, String> {
    let net = studio.routes.net()?;
    let track = chosen_track(&p.gpx, choice, p.format.name())
        .ok_or_else(|| format!("{} has no part {}", p.shown, part_number(choice)))?;
    let r = import_track(
        &net,
        &studio.routes.package,
        &track,
        &MatchParams::default(),
    )?;
    let mut name = track.name.clone().unwrap_or_else(|| p.stem.clone());
    if let PartChoice::One(i) = choice
        && p.parts.len() > 1
    {
        name = format!("{name} ({}/{})", i + 1, p.parts.len());
    }
    open_route(
        studio,
        RouteDoc::new(studio.routes.package.clone(), name),
        None,
        now_s,
    )?;
    let problems = r.problems.len();
    execute(
        studio,
        RouteOp::Set(Box::new(RouteSet {
            geometry: r.geometry,
            profile: None,
            imported: Some(Imported {
                track: Some(track),
                problems: r.problems,
            }),
            edit: ocw_edit::route::Edit::Import,
        })),
        now_s,
        true,
    )?;
    let what = match choice {
        PartChoice::One(i) if p.parts.len() > 1 => format!(", part {} of {}", i + 1, p.parts.len()),
        PartChoice::All if p.parts.len() > 1 => format!(", {} parts joined", p.parts.len()),
        _ => String::new(),
    };
    Ok(format!(
        "imported {} ({}{what}): {}{}",
        p.shown,
        p.format.name(),
        r.summary.lines().next().unwrap_or(""),
        if problems > 0 {
            format!("; {problems} problem sections")
        } else {
            String::new()
        }
    ))
}

fn part_number(c: PartChoice) -> String {
    match c {
        PartChoice::All => "all".into(),
        PartChoice::One(i) => n(i).to_string(),
    }
}

/// The default file of a saved route: `<package dir>/routes/<name>.ocwroute`.
pub fn default_route_path(package_file: &Path, name: &str) -> PathBuf {
    package_file
        .parent()
        .unwrap_or(Path::new("."))
        .join("routes")
        .join(format!(
            "{}.{}",
            route_slug(name),
            TrackFormat::GraphRoute.extension()
        ))
}

/// The file of a saved route when the package folder is not writable:
/// `<dir>/<name>.ocwroute` in `<data dir>/ocw/routes` (shared by all
/// packages), with the package's name in front if that file is another
/// package's route.
pub fn fallback_route_path(dir: &Path, package: &str, name: &str) -> PathBuf {
    let ext = TrackFormat::GraphRoute.extension();
    let plain = dir.join(format!("{}.{ext}", route_slug(name)));
    let other = std::fs::read_to_string(&plain)
        .ok()
        .and_then(|t| ocw_world::GraphRoute::parse(&t).ok())
        .and_then(|(_, made_for)| made_for)
        .is_some_and(|(n, _)| n.replace('_', " ") != package.replace('_', " "));
    if other {
        dir.join(format!(
            "{}-{}.{ext}",
            route_slug(package),
            route_slug(name)
        ))
    } else {
        plain
    }
}

/// A file name for `name`: lower case, `-` between the words.
fn route_slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "route".into()
    } else {
        slug
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent()
        && !d.as_os_str().is_empty()
    {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", shown_path(d)))?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", shown_path(path)))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", shown_path(path)))
}

/// A saved route: the file, and why it is not next to the package (the
/// package folder is not writable: it went to `<data dir>/ocw/routes`).
struct Saved {
    path: PathBuf,
    fallback: Option<String>,
}

impl Saved {
    /// "route saved as …", with why it is not next to the package.
    fn message(&self) -> String {
        match &self.fallback {
            None => format!("route saved as {}", shown_path(&self.path)),
            Some(why) => format!(
                "route saved as {} (the package folder is not writable: {why}); the game lists it from there too",
                shown_path(&self.path)
            ),
        }
    }
}

fn save(studio: &mut Studio, path: Option<String>) -> Result<Saved, String> {
    let net = studio.routes.net()?;
    let doc = studio.route().ok_or("no route is open")?.content().clone();
    // The package folder's `routes/` unless a file was given or saved to.
    let (path, default) = match path {
        Some(p) => (crate::edit::expand_home(p.trim()), false),
        None => match &studio.routes.file {
            Some(f)
                if TrackFormat::of_name(&f.to_string_lossy()) == Some(TrackFormat::GraphRoute) =>
            {
                (f.clone(), false)
            }
            _ => {
                let pkg = studio.package.as_ref().ok_or("no world is open")?;
                (default_route_path(&pkg.file, &doc.name), true)
            }
        },
    };
    let bytes = formats::export(&doc, &net, TrackFormat::GraphRoute)?;
    let saved = match write_file(&path, &bytes) {
        Ok(()) => Saved {
            path,
            fallback: None,
        },
        // Not writable (an installed or read-only package): the data
        // directory, which the game reads too (spec 16H Q19).
        Err(e) if default => {
            let dir = studio
                .routes
                .fallback_dir
                .clone()
                .ok_or_else(|| format!("{e} (and there is no data directory to save to)"))?;
            let alt = fallback_route_path(&dir, &doc.package.name, &doc.name);
            write_file(&alt, &bytes).map_err(|e2| format!("{e}; {e2}"))?;
            Saved {
                path: alt,
                fallback: Some(e),
            }
        }
        Err(e) => return Err(e),
    };
    studio.route_mut()?.mark_saved();
    studio.routes.file = Some(saved.path.clone());
    Ok(saved)
}

/// The test ride's program and arguments: `ocw-desktop` next to the studio
/// (else on the PATH; `OCW_DESKTOP` overrides), the package and the route.
pub fn test_ride_command(
    studio_exe: Option<&Path>,
    package: &Path,
    route: &Path,
) -> (PathBuf, Vec<String>) {
    let name = format!("ocw-desktop{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::var_os("OCW_DESKTOP")
        .map(PathBuf::from)
        .or_else(|| {
            let p = studio_exe?.parent()?.join(&name);
            p.is_file().then_some(p)
        })
        .unwrap_or_else(|| PathBuf::from(&name));
    (
        exe,
        vec![
            "--world".into(),
            package.display().to_string(),
            "--route".into(),
            route.display().to_string(),
        ],
    )
}

/// The native HTTP GET for BRouter (`ureq`, 30 s, one at a time).
pub struct UreqGet(ureq::Agent);

impl Default for UreqGet {
    fn default() -> Self {
        Self(
            ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .http_status_as_error(false)
                .build()
                .into(),
        )
    }
}

impl HttpGet for UreqGet {
    fn get(&self, url: &str, user_agent: &str) -> Result<HttpAnswer, String> {
        let resp = self
            .0
            .get(url)
            .header("User-Agent", user_agent)
            .call()
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(16 << 20)
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(HttpAnswer { status, body })
    }
}

/// Starts a BRouter request for the route's via-points.
fn plan_external(studio: &mut Studio) -> Result<String, String> {
    if studio.routes.job.is_some() {
        return Err("a BRouter request is still running".into());
    }
    let s = studio.routes.settings.clone();
    if s.brouter_url.trim().is_empty() {
        return Err(
            "enter a BRouter server first (route.planner.brouter_url; there is no default)".into(),
        );
    }
    if !studio.routes.brouter_consent {
        return Err(format!(
            "the via-points would be sent to {}: agree first (Routes → BRouter, or `studio route brouter ok`)",
            s.brouter_url.trim()
        ));
    }
    let doc = studio.route().ok_or("no route is open")?.content();
    let vias: Vec<[f64; 2]> = doc.geometry.vias.iter().map(|v| v.at).collect();
    if vias.len() < 2 {
        return Err("BRouter needs two via-points or more".into());
    }
    let proj = doc.projection();
    let mut lonlat: Vec<(f64, f64)> = vias.iter().map(|p| proj.unproject(*p)).collect();
    if doc.geometry.closed {
        lonlat.push(lonlat[0]);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let server = BRouterServer {
        url: s.brouter_url.clone(),
        profile: s.brouter_profile.clone(),
    };
    let shown = server.url.clone();
    std::thread::Builder::new()
        .name("brouter".into())
        .spawn(move || {
            let client = BRouterClient {
                server,
                http: UreqGet::default(),
            };
            let _ = tx.send(client.plan(&lonlat, ""));
        })
        .map_err(|e| e.to_string())?;
    studio.routes.job = Some(PlanJob {
        rx: Mutex::new(rx),
        vias,
        server: shown.clone(),
    });
    Ok(format!("asking BRouter at {shown} …"))
}

/// Takes a BRouter answer into the route: matched onto the package graph
/// and split at the via-points; the planner's line stays as the
/// comparison track with its problem sections.
pub fn take_planned(
    studio: &mut Studio,
    planned: &PlannedTrack,
    vias: &[[f64; 2]],
    now_s: f64,
) -> Result<String, String> {
    let net = studio.routes.net()?;
    let doc = studio.route().ok_or("no route is open")?.content().clone();
    let track = Track {
        name: Some("BRouter".into()),
        format: "BRouter".into(),
        points: planned
            .points
            .iter()
            .map(|p| TrackPoint {
                lat: p.lat,
                lon: p.lon,
                ele: p.ele,
            })
            .collect(),
    };
    let r = import_planned(&net, &doc.package, &track, vias, &planned.credit)?;
    let mut geometry = r.geometry;
    if doc.geometry.closed && geometry.vias.len() > 2 {
        // The planned way back ends at the start: the closing leg.
        geometry.vias.pop();
        geometry.closed = true;
    }
    let problems = r.problems.len();
    execute(
        studio,
        RouteOp::Set(Box::new(RouteSet {
            geometry,
            profile: None,
            imported: Some(Imported {
                track: Some(track),
                problems: r.problems,
            }),
            edit: ocw_edit::route::Edit::Plan,
        })),
        now_s,
        true,
    )?;
    Ok(format!(
        "planned with BRouter{}: {}",
        planned
            .length_m
            .map(|l| format!(" ({:.1} km)", l / 1000.0))
            .unwrap_or_default(),
        if problems == 0 {
            "matched onto the package's roads".to_string()
        } else {
            format!("{problems} sections do not match the package's roads")
        }
    ))
}

/// Carries out a route command; the status line's message.
pub fn command(studio: &mut Studio, cmd: RouteCommand, now_s: f64) -> Result<String, String> {
    use RouteCommand as C;
    match cmd {
        C::New(name) => {
            let name = name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "New route".into());
            let doc = RouteDoc::new(studio.routes.package.clone(), name.clone());
            open_route(studio, doc, None, now_s)?;
            studio.routes.file = None;
            Ok(format!(
                "new route '{name}': click the map to add via-points"
            ))
        }
        C::Import(path) => {
            studio.routes.pending = None;
            import_file(studio, &path, now_s)
        }
        C::ImportPart(choice) => {
            let p = studio
                .routes
                .pending
                .clone()
                .ok_or("no file is waiting for a choice (import a file first)")?;
            let msg = import_part(studio, &p, choice, now_s)?;
            studio.routes.pending = None;
            Ok(msg)
        }
        C::CancelImport => {
            studio.routes.pending = None;
            Ok(String::new())
        }
        C::Power(w) => {
            if !w.is_finite() || w < POWER_RANGE_W.0 || w > POWER_RANGE_W.1 {
                return Err(format!(
                    "the power is {:.0} – {:.0} W",
                    POWER_RANGE_W.0, POWER_RANGE_W.1
                ));
            }
            studio.routes.settings.power_w = w.round();
            // At once, so `studio route info` right after has the new time.
            studio.routes.update_estimate();
            save_settings(studio)?;
            Ok(format!("expected time at {:.0} W", w.round()))
        }
        C::PackageRoute(i) => {
            let net = studio.routes.net()?;
            let gr = studio
                .package
                .as_ref()
                .and_then(|p| p.pkg.routes.get(i))
                .cloned()
                .ok_or_else(|| format!("the package has no route {}", n(i)))?;
            let geometry = geometry_of_graph_route(&net, &gr)?;
            let mut doc = RouteDoc::new(studio.routes.package.clone(), gr.name.clone());
            doc.geometry = geometry;
            open_route(studio, doc, None, now_s)?;
            studio.routes.file = None;
            Ok(format!("opened the package's route '{}'", gr.name))
        }
        C::Click(at, tol) => {
            let near = studio.route().and_then(|d| {
                d.content()
                    .geometry
                    .vias
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i, (v.at[0] - at[0]).hypot(v.at[1] - at[1])))
                    .filter(|(_, d)| *d <= tol)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            });
            match near {
                Some(i) => command(studio, C::SelectVia(Some(i)), now_s),
                None => command(studio, C::AddVia(at), now_s),
            }
        }
        C::SelectVia(i) => {
            let d = studio.route_mut()?;
            match i {
                Some(i) if i < d.content().geometry.vias.len() => {
                    d.selection.select_only(i);
                    Ok(format!("via-point {} selected", n(i)))
                }
                Some(i) => Err(format!("no via-point {}", n(i))),
                None => {
                    d.selection.clear();
                    Ok(String::new())
                }
            }
        }
        C::AddVia(at) => {
            let op = with_editor(studio, |ed, d| ed.add_via(d, at))?;
            execute(studio, op, now_s, true)?;
            let d = studio.route_mut()?;
            let i = d.content().geometry.vias.len() - 1;
            d.selection.select_only(i);
            Ok(format!("via-point {} added", n(i)))
        }
        C::InsertVia(i, at) => {
            let op = with_editor(studio, |ed, d| ed.insert_via(d, i, at))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("via-point {} inserted", n(i)))
        }
        C::MoveVia(i, at) => {
            let op = with_editor(studio, |ed, d| ed.move_via(d, i, at))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("via-point {} moved", n(i)))
        }
        C::RemoveVia(i) => {
            let op = with_editor(studio, |ed, d| ed.remove_via(d, i))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("via-point {} removed", n(i)))
        }
        C::Plan(points) => {
            let op = with_editor(studio, |ed, _| ed.plan_through(&points))?;
            execute(studio, op, now_s, true)?;
            let len = studio.route().and_then(|d| {
                let net = studio.routes.net.as_ref()?;
                d.content().plan(net).ok().map(|p| p.length_m())
            });
            Ok(format!(
                "planned through {} via-points ({:.2} km)",
                points.len(),
                len.unwrap_or(0.0) / 1000.0
            ))
        }
        C::DragStart { grab, to, tol } => {
            let near = studio.route().and_then(|d| {
                d.content()
                    .geometry
                    .vias
                    .iter()
                    .position(|v| (v.at[0] - grab[0]).hypot(v.at[1] - grab[1]) <= tol)
            });
            match near {
                Some(i) => {
                    studio.routes.dragging = Some(i);
                    let op = with_editor(studio, |ed, d| ed.move_via(d, i, to))?;
                    execute(studio, op, now_s, false)?;
                    Ok(format!("dragging via-point {}", n(i)))
                }
                None => {
                    let (op, i) = with_editor(studio, |ed, d| ed.drag(d, grab, to))?;
                    execute(studio, op, now_s, false)?;
                    studio.routes.dragging = Some(i);
                    Ok("dragging the route".into())
                }
            }
        }
        C::DragTo(to) => {
            let i = studio.routes.dragging.ok_or("nothing is being dragged")?;
            let op = with_editor(studio, |ed, d| ed.move_via(d, i, to))?;
            execute(studio, op, now_s, false)?;
            Ok(String::new())
        }
        C::DragEnd => {
            studio.routes.dragging = None;
            if let Ok(d) = studio.route_mut() {
                d.seal();
            }
            Ok(String::new())
        }
        C::Fit => Ok(String::new()),
        C::Seal => {
            if let Ok(d) = studio.route_mut() {
                d.seal();
            }
            Ok(String::new())
        }
        C::Reverse => {
            let op = with_editor(studio, |ed, d| ed.reverse(d))?;
            execute(studio, op, now_s, true)?;
            Ok("route reversed".into())
        }
        C::Loop(on) => {
            let op = with_editor(studio, |ed, d| ed.set_loop(d, on))?;
            execute(studio, op, now_s, true)?;
            Ok(if on {
                "the route is a loop"
            } else {
                "the loop is open"
            }
            .into())
        }
        C::OutAndBack => {
            let op = with_editor(studio, |ed, d| ed.out_and_back(d))?;
            execute(studio, op, now_s, true)?;
            Ok("out and back: a U-turn at the end".into())
        }
        C::CutStart(i) => {
            let op = with_editor(studio, |ed, d| ed.cut_start(d, i))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("the route starts at via-point {} now", n(i)))
        }
        C::CutEnd(i) => {
            let op = with_editor(studio, |ed, d| ed.cut_end(d, i))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("the route ends at via-point {} now", n(i)))
        }
        C::Split(i) => {
            let (op, second) = with_editor(studio, |ed, d| ed.split(d, i))?;
            let name = studio
                .route()
                .map(|d| d.content().name.clone())
                .unwrap_or_default();
            execute(studio, op, now_s, true)?;
            // The second part is saved as its own route file.
            let net = studio.routes.net()?;
            let mut other = RouteDoc::new(studio.routes.package.clone(), format!("{name} (2)"));
            other.geometry = second;
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let path = default_route_path(&pkg.file, &other.name);
            write_file(
                &path,
                &formats::export(&other, &net, TrackFormat::GraphRoute)?,
            )?;
            Ok(format!(
                "split at via-point {}: the second part is saved as {}",
                n(i),
                shown_path(&path)
            ))
        }
        C::Profile(p) => {
            let profile = Profile::parse(&p)
                .ok_or_else(|| format!("profiles: {}", Profile::ALL.map(Profile::id).join(", ")))?;
            let op = with_editor(studio, |ed, d| ed.replan(d, profile.id()))?;
            execute(studio, op, now_s, true)?;
            Ok(format!("re-planned for {}", profile.name()))
        }
        C::Rename(name) => {
            execute(studio, RouteOp::SetName(name), now_s, false)?;
            Ok(String::new())
        }
        C::Describe(text) => {
            execute(studio, RouteOp::SetDescription(text), now_s, false)?;
            Ok(String::new())
        }
        C::Planner(k) => {
            studio.routes.settings.planner = k;
            save_settings(studio)?;
            Ok(format!("planner: {k:?}"))
        }
        C::BRouterUrl(url) => {
            let url = url.trim().to_string();
            if !url.is_empty() && !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("the BRouter server is an http:// or https:// URL".into());
            }
            if url != studio.routes.settings.brouter_url {
                // A new server: ask again before sending anything.
                studio.routes.brouter_consent = false;
            }
            studio.routes.settings.brouter_url = url;
            save_settings(studio)?;
            Ok("BRouter server set".into())
        }
        C::BRouterProfile(p) => {
            studio.routes.settings.brouter_profile = p.trim().to_string();
            save_settings(studio)?;
            Ok(format!("BRouter profile: {}", p.trim()))
        }
        C::BRouterConsent(on) => {
            studio.routes.brouter_consent = on;
            Ok(String::new())
        }
        C::PlanExternal => plan_external(studio),
        C::Export(path, format) => {
            let net = studio.routes.net()?;
            let doc = studio.route().ok_or("no route is open")?.content().clone();
            let p = crate::edit::expand_home(path.trim());
            let format = format
                .or_else(|| TrackFormat::of_name(&p.to_string_lossy()))
                .ok_or("give the file an extension: .gpx, .tcx, .geojson, .fit or .ocwroute")?;
            let bytes = formats::export(&doc, &net, format)?;
            write_file(&p, &bytes)?;
            Ok(format!("exported {} as {}", shown_path(&p), format.name()))
        }
        C::Save(path) => Ok(save(studio, path)?.message()),
        C::TestRide => {
            let route = save(studio, None)?.path;
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let exe = std::env::current_exe().ok();
            let (prog, args) = test_ride_command(exe.as_deref(), &pkg.file, &route);
            if let Some(mut c) = studio.routes.test_ride.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            let child = std::process::Command::new(&prog)
                .args(&args)
                .spawn()
                .map_err(|e| format!("{}: {e}", prog.display()))?;
            studio.routes.test_ride = Some(child);
            Ok(format!(
                "test ride: ocw-desktop with {}",
                shown_path(&route)
            ))
        }
    }
}

fn save_settings(studio: &Studio) -> Result<(), String> {
    studio
        .routes
        .settings
        .save(studio.routes.settings_path.as_deref())
}

/// The colour of a grade (the profile and the route line).
pub fn grade_rgb(g: f64) -> [u8; 3] {
    let g = g.abs();
    if g < 0.03 {
        [90, 200, 120]
    } else if g < 0.06 {
        [230, 210, 70]
    } else if g < 0.09 {
        [240, 140, 50]
    } else {
        [220, 60, 50]
    }
}

/// What the views draw of `doc` on `net`.
pub fn route_view(
    doc: &Document<RouteDoc>,
    net: &RoadNetwork,
    surface: &dyn Fn(usize) -> ocw_world::RoadSurface,
) -> RouteView {
    let c = doc.content();
    let proj = c.projection();
    let mut v = RouteView {
        key: Some((doc.id, doc.state())),
        vias: c.geometry.vias.iter().map(|v| v.at).collect(),
        selected: doc.selection.single().copied(),
        ..Default::default()
    };
    if let Some(t) = &c.track {
        v.track = t
            .points
            .iter()
            .map(|p| proj.project(p.lat, p.lon))
            .collect();
        v.track_ele = t.elevation();
        for p in &c.problems {
            let last = p.last.min(v.track.len().saturating_sub(1));
            let first = p.first.min(last);
            let mut pts = v.track[first..=last].to_vec();
            if pts.len() == 1 {
                // A single point: a short mark around it.
                pts.push([pts[0][0] + 0.1, pts[0][1]]);
            }
            v.problems.push((p.kind, pts));
        }
    }
    if c.geometry.vias.len() >= 2 {
        match c.plan(net) {
            Ok(plan) => {
                v.line = steps_points(net, &plan.steps, 8.0);
                v.grades = v
                    .line
                    .windows(2)
                    .map(|w| {
                        let d = (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]).max(0.01);
                        (w[1][1] - w[0][1]) / d
                    })
                    .collect();
                v.profile = Some(route_profile(net, &plan, surface));
            }
            Err(e) => v.error = Some(e),
        }
    }
    v
}

/// The bounding box (x, z) of the route, its via-points and its track.
pub fn route_extent(v: &RouteView) -> Option<([f64; 2], [f64; 2])> {
    let pts = v
        .line
        .iter()
        .map(|p| [p[0], p[2]])
        .chain(v.vias.iter().copied())
        .chain(v.track.iter().copied());
    let mut ext: Option<([f64; 2], [f64; 2])> = None;
    for p in pts {
        let (lo, hi) = ext.get_or_insert((p, p));
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    ext
}

/// Makes the route's view now (else the next frame does).
pub fn refresh_view(studio: &mut Studio) {
    let (Some(doc), Some(net)) = (studio.route(), studio.routes.net.clone()) else {
        return;
    };
    let routes = &studio.routes;
    let surface = |e: usize| routes.surface(&net, e);
    let view = route_view(doc, &net, &surface);
    studio.routes.view = view;
    studio.routes.update_estimate();
}

/// Keeps [`RouteTools::view`] up to date; takes BRouter's answers; reaps a
/// finished test ride.
pub fn update_routes(mut studio: ResMut<Studio>, time: Res<Time<Real>>) {
    let now_s = time.elapsed_secs_f64();
    let studio = &mut *studio;
    let polled = studio.routes.job.as_ref().map(|j| match j.rx.lock() {
        Ok(rx) => rx.try_recv(),
        Err(_) => Err(TryRecvError::Disconnected),
    });
    match polled {
        None | Some(Err(TryRecvError::Empty)) => {}
        Some(Err(TryRecvError::Disconnected)) => {
            studio.routes.job = None;
            studio.status = "error: the BRouter request ended without an answer".into();
        }
        Some(Ok(answer)) => {
            let job = studio.routes.job.take().expect("checked");
            let msg = match answer
                .map_err(|e| e.to_string())
                .and_then(|t| take_planned(studio, &t, &job.vias, now_s))
            {
                Ok(m) => m,
                Err(e) => format!("error: {e}"),
            };
            info!("{msg}");
            studio.status = msg;
        }
    }
    if let Some(c) = studio.routes.test_ride.as_mut()
        && let Ok(Some(status)) = c.try_wait()
    {
        studio.routes.test_ride = None;
        studio.status = format!("the test ride ended ({status})");
    }
    let Some(doc) = studio.route() else {
        if studio.routes.view != RouteView::default() {
            studio.routes.view = RouteView::default();
        }
        studio.routes.update_estimate();
        return;
    };
    let selected = doc.selection.single().copied();
    if studio.routes.view.key == Some((doc.id, doc.state()))
        && selected == studio.routes.view.selected
    {
        studio.routes.update_estimate();
        return;
    }
    let Some(net) = studio.routes.net.clone() else {
        return;
    };
    let routes = &studio.routes;
    let surface = |e: usize| routes.surface(&net, e);
    let view = route_view(doc, &net, &surface);
    debug!(
        "route view: {} points, {} via-points, error {:?}",
        view.line.len(),
        view.vias.len(),
        view.error
    );
    studio.routes.view = view;
    studio.routes.update_estimate();
}

/// The route words of `studio route …` (pure: the words to a command).
pub fn console_route(words: &[String]) -> Result<RouteCommand, String> {
    use RouteCommand as C;
    let w = |i: usize| words.get(i).map(String::as_str);
    let num = |i: usize, what: &str| -> Result<f64, String> {
        w(i).ok_or_else(|| format!("{what} is missing"))?
            .parse::<f64>()
            .map_err(|_| format!("{what} must be a number"))
    };
    let index = |i: usize| -> Result<usize, String> {
        let v = w(i)
            .ok_or("the via-point's number is missing")?
            .parse::<usize>()
            .map_err(|_| "a via-point's number (1, 2, …)".to_string())?;
        v.checked_sub(1)
            .ok_or_else(|| "via-points count from 1".into())
    };
    let rest = |i: usize| words.get(i..).map(|r| r.join(" ")).unwrap_or_default();
    let usage = "studio route new [NAME] | import PATH | import part N|all | import cancel | power W | package N | fit | plan X Z X Z … | via add X Z | via insert N X Z | via move N X Z | via remove N | drag GX GZ X Z | reverse | loop on|off | outback | cut start|end N | split N | profile road|gravel|mtb|quiet | name TEXT | planner builtin|brouter | brouter url URL | brouter profile P | brouter ok | brouter plan | export PATH | save [PATH] | testride";
    Ok(match (w(0), w(1)) {
        (Some("new"), _) => C::New((words.len() > 1).then(|| rest(1))),
        (Some("import"), Some("part")) => match w(2) {
            Some("all") => C::ImportPart(PartChoice::All),
            _ => C::ImportPart(PartChoice::One(index(2)?)),
        },
        (Some("import"), Some("cancel")) => C::CancelImport,
        (Some("import"), Some(_)) => C::Import(rest(1)),
        (Some("power"), Some(_)) => C::Power(num(1, "the power (W)")?),
        (Some("package"), _) => C::PackageRoute(index(1).unwrap_or(0)),
        (Some("plan"), _) => {
            let nums = words[1..]
                .iter()
                .map(|s| s.parse::<f64>().map_err(|_| format!("{s}: not a number")))
                .collect::<Result<Vec<_>, _>>()?;
            if nums.len() < 4 || nums.len() % 2 != 0 {
                return Err("studio route plan X Z X Z … (two points or more)".into());
            }
            C::Plan(nums.chunks(2).map(|c| [c[0], c[1]]).collect())
        }
        (Some("via"), Some("add")) => C::AddVia([num(2, "X")?, num(3, "Z")?]),
        (Some("via"), Some("insert")) => C::InsertVia(index(2)?, [num(3, "X")?, num(4, "Z")?]),
        (Some("via"), Some("move")) => C::MoveVia(index(2)?, [num(3, "X")?, num(4, "Z")?]),
        (Some("via"), Some("remove")) => C::RemoveVia(index(2)?),
        (Some("via"), Some("select")) => C::SelectVia(Some(index(2)?)),
        (Some("drag"), _) => C::DragStart {
            grab: [num(1, "GX")?, num(2, "GZ")?],
            to: [num(3, "X")?, num(4, "Z")?],
            tol: 0.0,
        },
        (Some("reverse"), _) => C::Reverse,
        (Some("loop"), Some("on")) => C::Loop(true),
        (Some("loop"), Some("off")) => C::Loop(false),
        (Some("outback"), _) => C::OutAndBack,
        (Some("cut"), Some("start")) => C::CutStart(index(2)?),
        (Some("cut"), Some("end")) => C::CutEnd(index(2)?),
        (Some("split"), _) => C::Split(index(1)?),
        (Some("profile"), Some(p)) => C::Profile(p.into()),
        (Some("name"), Some(_)) => C::Rename(rest(1)),
        (Some("planner"), Some("builtin")) => C::Planner(PlannerKind::Builtin),
        (Some("planner"), Some("brouter")) => C::Planner(PlannerKind::Brouter),
        (Some("brouter"), Some("url")) => C::BRouterUrl(rest(2)),
        (Some("brouter"), Some("profile")) => C::BRouterProfile(rest(2)),
        (Some("brouter"), Some("ok")) => C::BRouterConsent(true),
        (Some("brouter"), Some("plan")) => C::PlanExternal,
        (Some("export"), Some(_)) => C::Export(rest(1), None),
        (Some("save"), _) => C::Save((words.len() > 1).then(|| rest(1))),
        (Some("testride"), _) => C::TestRide,
        (Some("fit"), _) => C::Fit,
        _ => return Err(usage.into()),
    })
}

/// "expected 1 h 05 min at 200 W (24.3 km/h), Ada Rider, 68 kg, Road".
pub fn estimate_line(e: &RideEstimate, rider: &LocalRider) -> String {
    format!(
        "expected {} at {:.0} W ({:.1} km/h), {}",
        ocw_sim::estimate::duration_text(e.time_s),
        e.power_w,
        e.average_mps * 3.6,
        rider.summary()
    )
}

/// What the map fits: the parts of a waiting import, else the route.
pub fn fit_extent(routes: &RouteTools) -> Option<([f64; 2], [f64; 2])> {
    if let Some(p) = &routes.pending {
        let v = RouteView {
            track: p
                .parts
                .iter()
                .flat_map(|t| t.line.iter().copied())
                .collect(),
            ..Default::default()
        };
        return route_extent(&v);
    }
    route_extent(&routes.view)
}

/// "47.1412° N 9.5213° E".
pub fn lat_lon_text((lat, lon): (f64, f64)) -> String {
    format!(
        "{:.4}° {} {:.4}° {}",
        lat.abs(),
        if lat >= 0.0 { "N" } else { "S" },
        lon.abs(),
        if lon >= 0.0 { "E" } else { "W" }
    )
}

/// The package place nearest to `p` (x, z) within 5 km: "near Vaduz".
pub fn near_place(studio: &Studio, p: [f64; 2]) -> Option<String> {
    let places = studio.package.as_ref()?.pkg.places.as_ref()?;
    places
        .places
        .iter()
        .map(|pl| {
            let d = (f64::from(pl.pos[0]) - p[0]).hypot(f64::from(pl.pos[1]) - p[1]);
            (d, pl)
        })
        .filter(|(d, _)| *d < 5000.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(d, pl)| {
            if d < 300.0 {
                format!("in {}", pl.name)
            } else {
                format!("near {}", pl.name)
            }
        })
}

/// `studio route info`: the route in a few lines.
pub fn route_lines(studio: &Studio) -> Vec<String> {
    let Some(d) = studio.route() else {
        return vec!["no route is open".into()];
    };
    let c = d.content();
    let mut out = vec![format!(
        "'{}': {} via-points{}, profile {}",
        c.name,
        c.geometry.vias.len(),
        if c.geometry.closed { ", a loop" } else { "" },
        c.profile
    )];
    if let Some(p) = &studio.routes.view.profile {
        out.push(format!(
            "{:.2} km, +{:.0} m / -{:.0} m, steepest {:.1} %",
            p.length_m / 1000.0,
            p.gain_m,
            p.loss_m,
            p.max_grade * 100.0
        ));
    }
    if let Some(e) = &studio.routes.estimate {
        out.push(estimate_line(e, &studio.routes.rider));
    }
    for f in d.findings() {
        let tag = match f.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        out.push(format!("{tag}: {}", f.message));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_edit::MemoryStore;
    use ocw_world::plan::test_grid;

    /// A studio with the test grid as its package's road graph (no
    /// package file: saving needs one).
    pub fn studio() -> Studio {
        let mut s = Studio::new(Some(Box::new(MemoryStore::default())), None);
        s.routes = RouteTools::detached();
        let net = Arc::new(test_grid());
        s.routes.package = RoutePackage {
            id: "local/grid".into(),
            name: "Grid".into(),
            fingerprint: net.fingerprint(),
            origin: (47.14, 9.52),
        };
        s.routes.net = Some(net);
        s
    }

    fn run(s: &mut Studio, c: RouteCommand, t: f64) -> String {
        command(s, c, t).unwrap_or_else(|e| panic!("{e}"))
    }

    fn length(s: &Studio) -> f64 {
        s.route()
            .unwrap()
            .content()
            .plan(s.routes.net.as_ref().unwrap())
            .unwrap()
            .length_m()
    }

    #[test]
    fn clicks_add_and_select_via_points_and_drags_reshape_the_route() {
        let mut s = studio();
        run(&mut s, RouteCommand::New(Some("Ada's grid".into())), 0.0);
        run(&mut s, RouteCommand::Click([0.0, 200.0], 5.0), 1.0);
        run(&mut s, RouteCommand::Click([400.0, 200.0], 5.0), 2.0);
        assert!((length(&s) - 400.0).abs() < 1e-6);
        // A click on a via-point selects it.
        assert!(run(&mut s, RouteCommand::Click([1.0, 201.0], 5.0), 3.0).contains("selected"));
        assert_eq!(s.route().unwrap().selection.single(), Some(&0));
        // Drag the route's middle down to row 2: a via-point is inserted,
        // the drag's moves are one undo step.
        run(
            &mut s,
            RouteCommand::DragStart {
                grab: [200.0, 202.0],
                to: [200.0, 300.0],
                tol: 5.0,
            },
            4.0,
        );
        run(&mut s, RouteCommand::DragTo([200.0, 400.0]), 4.3);
        run(&mut s, RouteCommand::DragEnd, 4.5);
        assert_eq!(s.route().unwrap().content().geometry.vias.len(), 3);
        assert!((length(&s) - 800.0).abs() < 1e-6, "{}", length(&s));
        assert_eq!(s.route().unwrap().undo_label(), Some("Drag route"));
        run(&mut s, RouteCommand::Reverse, 6.0);
        run(&mut s, RouteCommand::Loop(true), 8.0);
        assert!(
            s.route()
                .unwrap()
                .findings()
                .iter()
                .all(|f| f.severity != Severity::Error)
        );
        // Undo through the session (the Edit menu) acts on the route.
        s.session.undo(9.0).unwrap();
        s.session.undo(9.0).unwrap();
        s.session.undo(9.0).unwrap();
        assert_eq!(s.route().unwrap().content().geometry.vias.len(), 2);
    }

    #[test]
    fn the_view_follows_the_route_with_grades_and_a_profile() {
        let mut s = studio();
        run(&mut s, RouteCommand::New(None), 0.0);
        run(
            &mut s,
            RouteCommand::Plan(vec![[0.0, 0.0], [600.0, 600.0]]),
            1.0,
        );
        let net = s.routes.net.clone().unwrap();
        let v = route_view(s.route().unwrap(), &net, &|_| {
            ocw_world::RoadSurface::Asphalt
        });
        assert_eq!(v.vias.len(), 2);
        assert!(v.line.len() > 100);
        assert_eq!(v.grades.len(), v.line.len() - 1);
        let p = v.profile.unwrap();
        assert!((p.length_m - 1200.0).abs() < 1e-6);
        assert_eq!(grade_rgb(0.0), [90, 200, 120]);
        assert_eq!(grade_rgb(-0.12), [220, 60, 50]);
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let base = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!("ocw-studio-routes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_imported_gpx_shows_its_problems_and_exports_round_trip() {
        let mut s = studio();
        let dir = tmp_dir("import");
        let gpx = dir.join("ride.gpx");
        std::fs::write(
            &gpx,
            include_str!("../../../crates/ocw-edit/tests/fixtures/grid-problems.gpx"),
        )
        .unwrap();
        let msg = run(&mut s, RouteCommand::Import(gpx.display().to_string()), 0.0);
        assert!(msg.contains("problem sections"), "{msg}");
        let net = s.routes.net.clone().unwrap();
        let v = route_view(s.route().unwrap(), &net, &|_| {
            ocw_world::RoadSurface::Asphalt
        });
        assert!(!v.problems.is_empty());
        assert!(!v.track.is_empty() && !v.track_ele.is_empty());
        // Exports of a planned route import onto the same roads again.
        run(&mut s, RouteCommand::New(None), 1.0);
        run(
            &mut s,
            RouteCommand::Plan(vec![
                [0.0, 200.0],
                [400.0, 200.0],
                [400.0, 600.0],
                [100.0, 600.0],
            ]),
            1.0,
        );
        let before = length(&s);
        for ext in ["gpx", "tcx", "geojson", "fit", "ocwroute"] {
            let out = dir.join(format!("out.{ext}"));
            run(
                &mut s,
                RouteCommand::Export(out.display().to_string(), None),
                1.0,
            );
            run(&mut s, RouteCommand::Import(out.display().to_string()), 2.0);
            assert!(
                (length(&s) - before).abs() < 2.0,
                "{ext}: {} vs {before}",
                length(&s)
            );
        }
        assert!(
            command(
                &mut s,
                RouteCommand::Export(dir.join("x").display().to_string(), None),
                3.0
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn brouter_needs_a_server_and_consent_and_its_answer_is_matched() {
        let mut s = studio();
        run(&mut s, RouteCommand::New(None), 0.0);
        run(
            &mut s,
            RouteCommand::Plan(vec![[0.0, 0.0], [200.0, 400.0], [600.0, 600.0]]),
            1.0,
        );
        let e = command(&mut s, RouteCommand::PlanExternal, 2.0).unwrap_err();
        assert!(e.contains("no default"), "{e}");
        run(
            &mut s,
            RouteCommand::BRouterUrl("http://127.0.0.1:9".into()),
            2.0,
        );
        let e = command(&mut s, RouteCommand::PlanExternal, 2.0).unwrap_err();
        assert!(e.contains("agree first"), "{e}");
        assert!(
            command(
                &mut s,
                RouteCommand::BRouterUrl("brouter.example".into()),
                2.0
            )
            .is_err()
        );
        // The answer (as BRouter sends it) is matched and split.
        let planned = ocw_world::plan::brouter::parse_geojson(include_str!(
            "../../../crates/ocw-world/tests/fixtures/brouter-grid.geojson"
        ))
        .unwrap();
        let vias: Vec<[f64; 2]> = s
            .route()
            .unwrap()
            .content()
            .geometry
            .vias
            .iter()
            .map(|v| v.at)
            .collect();
        let msg = take_planned(&mut s, &planned, &vias, 3.0).unwrap();
        assert!(msg.contains("BRouter"), "{msg}");
        let d = s.route().unwrap();
        assert_eq!(d.content().geometry.vias.len(), 3);
        assert!(d.content().geometry.credits[0].contains("BRouter"));
        assert_eq!(d.content().track.as_ref().unwrap().format, "BRouter");
        assert!(
            d.findings().iter().all(|f| f.severity != Severity::Error),
            "{:?}",
            d.findings()
        );
        assert!((length(&s) - 1200.0).abs() < 5.0);
    }

    /// The native transport against a local fake server answering what
    /// BRouter answers (no network beyond the loopback).
    #[test]
    fn the_native_client_asks_a_local_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let body =
                include_str!("../../../crates/ocw-world/tests/fixtures/brouter-grid.geojson");
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.geo+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&req).to_string()
        });
        let client = BRouterClient {
            server: BRouterServer {
                url: format!("http://127.0.0.1:{port}"),
                profile: "trekking".into(),
            },
            http: UreqGet::default(),
        };
        let t = client.plan(&[(47.14, 9.52), (47.135, 9.528)], "").unwrap();
        assert!(t.points.len() > 20);
        let req = server.join().unwrap();
        assert!(
            req.starts_with("GET /brouter?lonlats=9.520000,47.140000%7C"),
            "{req}"
        );
        assert!(
            req.to_ascii_lowercase()
                .contains("user-agent: opencyclingworld-studio/")
        );
    }

    /// A studio on the grid with an open route "Ada's grid" and the
    /// package file `file`.
    fn studio_with_route(file: PathBuf, fallback: Option<PathBuf>) -> Studio {
        let mut s = studio();
        let mut pkg = crate::patch::tests::package();
        pkg.file = file;
        s.package = Some(pkg);
        s.routes.fallback_dir = fallback;
        run(&mut s, RouteCommand::New(Some("Ada's grid".into())), 0.0);
        run(&mut s, RouteCommand::Click([0.0, 200.0], 5.0), 1.0);
        run(&mut s, RouteCommand::Click([400.0, 200.0], 5.0), 2.0);
        s
    }

    /// Spec 16H Q19 (a): *Save route* writes next to the package, else
    /// (the package folder is not writable) into `<data dir>/ocw/routes`,
    /// and says where.
    #[test]
    fn save_route_falls_back_to_the_data_dir_and_says_where() {
        let dir = tmp_dir("save-fallback");
        let data = dir.join("data").join("routes");
        // A writable package folder: `routes/` next to the package.
        let open = dir.join("open");
        std::fs::create_dir_all(&open).unwrap();
        let mut s = studio_with_route(open.join("grid.ocwpkg"), Some(data.clone()));
        let m = run(&mut s, RouteCommand::Save(None), 3.0);
        let next_to = open.join("routes").join("ada-s-grid.ocwroute");
        assert!(next_to.is_file(), "{m}");
        assert!(
            m.starts_with("route saved as ") && !m.contains("not writable"),
            "{m}"
        );
        assert!(!data.exists(), "nothing in the data dir");
        // A package folder whose `routes` cannot be made (a file is in the
        // way, as a read-only folder would be): the data directory.
        let locked = dir.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("routes"), "in the way").unwrap();
        let mut s = studio_with_route(locked.join("grid.ocwpkg"), Some(data.clone()));
        let m = run(&mut s, RouteCommand::Save(None), 3.0);
        let fell = data.join("ada-s-grid.ocwroute");
        assert!(fell.is_file(), "{m}");
        assert!(m.contains("ada-s-grid.ocwroute"), "says where: {m}");
        assert!(
            m.contains("the package folder is not writable"),
            "and why: {m}"
        );
        assert_eq!(s.routes.file.as_deref(), Some(fell.as_path()));
        assert!(!s.route().unwrap().is_dirty(), "saved");
        let (route, made_for) =
            ocw_world::GraphRoute::parse(&std::fs::read_to_string(&fell).unwrap()).unwrap();
        assert_eq!(route.name, "Ada's grid");
        assert_eq!(made_for.map(|(n, _)| n), Some("Grid".to_string()));
        // Saving again goes to the same file.
        let again = run(&mut s, RouteCommand::Save(None), 4.0);
        assert!(again.contains("ada-s-grid.ocwroute") && !again.contains("not writable"));
        // Another package's route of the same name is not overwritten.
        assert_eq!(fallback_route_path(&data, "Grid", "Ada's grid"), fell);
        assert_eq!(
            fallback_route_path(&data, "Other Land", "Ada's grid"),
            data.join("other-land-ada-s-grid.ocwroute")
        );
        // No data directory either: the error says so.
        let mut s = studio_with_route(locked.join("grid.ocwpkg"), None);
        let e = command(&mut s, RouteCommand::Save(None), 3.0).unwrap_err();
        assert!(e.contains("no data directory"), "{e}");
        // A file given by name is not redirected.
        let mut s = studio_with_route(open.join("grid.ocwpkg"), Some(data.clone()));
        let bad = locked.join("routes").join("x.ocwroute");
        assert!(
            command(
                &mut s,
                RouteCommand::Save(Some(bad.display().to_string())),
                3.0
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_test_ride_starts_the_game_with_the_package_and_the_route() {
        let (exe, args) = test_ride_command(
            None,
            Path::new("/data/grid.ocwpkg"),
            Path::new("/data/routes/ada.ocwroute"),
        );
        if std::env::var_os("OCW_DESKTOP").is_none() {
            assert!(exe.to_string_lossy().starts_with("ocw-desktop"));
        }
        assert_eq!(
            args,
            [
                "--world",
                "/data/grid.ocwpkg",
                "--route",
                "/data/routes/ada.ocwroute"
            ]
        );
        assert_eq!(
            default_route_path(Path::new("/data/grid.ocwpkg"), "Ada's Grid Loop!"),
            PathBuf::from("/data/routes/ada-s-grid-loop.ocwroute")
        );
    }

    #[test]
    fn planner_settings_have_no_server_by_default_and_round_trip() {
        let d = PlannerSettings::default();
        assert!(d.brouter_url.is_empty());
        assert_eq!(d.planner, PlannerKind::Builtin);
        let dir = tmp_dir("settings");
        let p = dir.join("planner.json");
        let s = PlannerSettings {
            planner: PlannerKind::Brouter,
            brouter_url: "http://localhost:17777".into(),
            brouter_profile: "fastbike".into(),
            power_w: 250.0,
        };
        s.save(Some(&p)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"route.planner.brouter_url\""));
        assert_eq!(PlannerSettings::load(Some(&p)), s);
        assert_eq!(PlannerSettings::load(Some(&dir.join("none.json"))), d);
        // An older file without the power gets the default.
        std::fs::write(&p, r#"{"route.planner": "builtin"}"#).unwrap();
        assert_eq!(PlannerSettings::load(Some(&p)).power_w, DEFAULT_POWER_W);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_package_surfaces_steer_the_planner() {
        let mut s = studio();
        run(&mut s, RouteCommand::New(None), 0.0);
        let via = vec![[0.0, 200.0], [600.0, 200.0]];
        run(&mut s, RouteCommand::Plan(via.clone()), 1.0);
        assert!((length(&s) - 600.0).abs() < 1e-6);
        // The package's strips say row 1's middle (vertex 5 → 6) is dirt:
        // the road bike goes round it.
        let net = s.routes.net.clone().unwrap();
        let dirt = net
            .graph()
            .edges
            .iter()
            .position(|e| (e.from.0, e.to.0) == (5, 6))
            .unwrap();
        s.routes.surfaces = Arc::new(
            (0..net.graph().edges.len())
                .map(|e| {
                    if e == dirt {
                        RoadSurface::Dirt
                    } else {
                        RoadSurface::of_class(net.edge(e).class)
                    }
                })
                .collect(),
        );
        s.routes.costs = None;
        run(&mut s, RouteCommand::New(None), 2.0);
        run(&mut s, RouteCommand::Plan(via), 3.0);
        assert!((length(&s) - 1000.0).abs() < 1e-6, "{}", length(&s));
        // The figures show the package's surfaces too.
        refresh_view(&mut s);
        let p = s.routes.view.profile.as_ref().unwrap();
        assert!(p.surfaces.iter().all(|(sf, _)| *sf != RoadSurface::Dirt));
        assert_eq!(s.routes.surface(&net, dirt), RoadSurface::Dirt);
    }

    #[test]
    fn the_expected_time_follows_the_route_and_the_power() {
        let mut s = studio();
        run(&mut s, RouteCommand::New(None), 0.0);
        run(
            &mut s,
            RouteCommand::Plan(vec![[0.0, 0.0], [600.0, 600.0]]),
            1.0,
        );
        refresh_view(&mut s);
        // 1.2 km on the flat at 200 W (75 kg, road bike): about 33 km/h.
        let e = s.routes.estimate.unwrap();
        assert_eq!(e.power_w, DEFAULT_POWER_W);
        assert!((e.length_m - 1200.0).abs() < 1e-6);
        let kmh = e.average_mps * 3.6;
        assert!((29.0..34.0).contains(&kmh), "{kmh}");
        let steady = ocw_sim::physics::steady_speed(
            &s.routes.rider.physics,
            &ocw_sim::Environment::with_grade(0.0),
            200.0,
        );
        assert!(e.time_s > 1200.0 / steady && e.time_s < 1200.0 / steady + 15.0);
        assert!(route_lines(&s).iter().any(|l| l.starts_with("expected")));
        // More power: faster; the setting is kept.
        // (Regression: `route info` right after the power said the old time.)
        run(&mut s, RouteCommand::Power(400.0), 2.0);
        assert!(route_lines(&s).iter().any(|l| l.contains("at 400 W")));
        let fast = s.routes.estimate.unwrap();
        assert!(fast.time_s < e.time_s * 0.85);
        assert_eq!(s.routes.settings.power_w, 400.0);
        assert!(command(&mut s, RouteCommand::Power(5.0), 3.0).is_err());
        assert!(command(&mut s, RouteCommand::Power(f64::NAN), 3.0).is_err());
        // A heavier rider climbs slower: edge 0 climbs 30 m.
        let mut g = s.routes.net.as_ref().unwrap().graph().clone();
        g.edges[0].profile = (0..=20).map(|k| f64::from(k) * 1.5).collect();
        let net = RoadNetwork::new(g);
        let mut d = RouteDoc::new(s.routes.package.clone(), "climb");
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        let ed = Editor {
            net: &net,
            costs: &costs,
        };
        let RouteOp::Set(set) = ed.plan_through(&[[0.0, 0.0], [200.0, 0.0]]).unwrap() else {
            panic!()
        };
        d.geometry = set.geometry;
        let profile = route_profile(&net, &d.plan(&net).unwrap(), &|_| RoadSurface::Asphalt);
        let light = estimate(&profile, &RiderProfile::for_rider_weight(60.0), 250.0).unwrap();
        let heavy = estimate(&profile, &RiderProfile::for_rider_weight(95.0), 250.0).unwrap();
        assert!(
            heavy.time_s > light.time_s * 1.1,
            "{} {}",
            heavy.time_s,
            light.time_s
        );
        // No route, no estimate.
        run(&mut s, RouteCommand::New(None), 4.0);
        refresh_view(&mut s);
        s.routes.view = RouteView::default();
        s.routes.update_estimate();
        assert!(s.routes.estimate.is_none());
        assert!(estimate_line(&fast, &s.routes.rider).contains("at 400 W"));
    }

    #[test]
    fn the_rider_comes_from_the_games_config() {
        let dir = tmp_dir("rider");
        assert_eq!(LocalRider::load(Some(&dir)), LocalRider::default());
        assert_eq!(LocalRider::default().weight_kg, 75.0);
        // The old riders file …
        std::fs::write(
            dir.join("riders.toml"),
            "[[rider]]\nname = \"Bea Climber\"\nweight_kg = 58.0\n",
        )
        .unwrap();
        let r = LocalRider::load(Some(&dir));
        assert_eq!((r.name.as_str(), r.weight_kg), ("Bea Climber", 58.0));
        assert_eq!(r.physics.rider_mass_kg, 58.0);
        // … and ocw.toml's section first, its active rider.
        std::fs::write(
            dir.join("ocw.toml"),
            "[settings]\n\n[riders]\nactive = \"Cy Sprinter\"\n\n[[riders.rider]]\nname = \"Ada Rider\"\nweight_kg = 64.0\n\n[[riders.rider]]\nname = \"Cy Sprinter\"\nweight_kg = 88.0\nbike = \"gravel\"\n",
        )
        .unwrap();
        let r = LocalRider::load(Some(&dir));
        assert_eq!((r.name.as_str(), r.weight_kg), ("Cy Sprinter", 88.0));
        assert_eq!(r.physics.bike, ocw_core::ProfileId::Gravel);
        assert!(r.from.as_deref().unwrap().ends_with("ocw.toml"));
        assert!(
            r.summary().starts_with("Cy Sprinter, 88 kg"),
            "{}",
            r.summary()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A GPX file of two segments on the grid: row 1 east (600 m), then
    /// 200 m down column 0 from z = 400.
    fn two_segments() -> String {
        let proj = ocw_world::geo::LocalProjection::new(47.14, 9.52);
        let seg = |pts: Vec<[f64; 2]>| -> String {
            let mut s = String::from("<trkseg>");
            for p in pts {
                let (lat, lon) = proj.unproject(p);
                s.push_str(&format!(r#"<trkpt lat="{lat:.7}" lon="{lon:.7}"/>"#));
            }
            s + "</trkseg>"
        };
        format!(
            "<gpx><trk><name>Ada's two rides</name>{}{}</trk></gpx>",
            seg((0..=30).map(|k| [f64::from(k) * 20.0, 200.0]).collect()),
            seg((0..=20)
                .map(|k| [0.0, 400.0 + f64::from(k) * 10.0])
                .collect())
        )
    }

    #[test]
    fn a_file_of_several_parts_waits_for_the_choice_of_one() {
        let mut s = studio();
        let dir = tmp_dir("parts");
        let gpx = dir.join("two.gpx");
        std::fs::write(&gpx, two_segments()).unwrap();
        let path = gpx.display().to_string();
        let msg = run(&mut s, RouteCommand::Import(path.clone()), 0.0);
        assert!(msg.contains("has 2 parts"), "{msg}");
        // Nothing imported yet; the parts are listed.
        assert!(s.route().is_none());
        let p = s.routes.pending.as_ref().unwrap();
        assert_eq!(p.parts.len(), 2);
        assert!((p.parts[0].length_m - 600.0).abs() < 3.0);
        assert!(lat_lon_text(p.parts[0].start).starts_with("47.1"));
        let ext = fit_extent(&s.routes).unwrap();
        assert!(ext.0[0] < 1.0 && ext.1[0] > 599.0 && ext.1[1] > 599.0);
        // The second part alone.
        let msg = run(&mut s, RouteCommand::ImportPart(PartChoice::One(1)), 1.0);
        assert!(msg.contains("part 2 of 2"), "{msg}");
        assert!(s.routes.pending.is_none());
        assert!((length(&s) - 200.0).abs() < 2.0, "{}", length(&s));
        assert_eq!(s.route().unwrap().content().name, "Ada's two rides (2/2)");
        // Again, all joined: both parts and the jump between them.
        run(&mut s, RouteCommand::Import(path.clone()), 2.0);
        let msg = run(&mut s, RouteCommand::ImportPart(PartChoice::All), 3.0);
        assert!(msg.contains("2 parts joined"), "{msg}");
        assert!(
            s.route()
                .unwrap()
                .content()
                .problems
                .iter()
                .any(|p| p.kind == ProblemKind::Gap)
        );
        // A part that is not there keeps the file waiting; cancel forgets it.
        run(&mut s, RouteCommand::Import(path), 4.0);
        assert!(command(&mut s, RouteCommand::ImportPart(PartChoice::One(5)), 5.0).is_err());
        assert!(s.routes.pending.is_some());
        run(&mut s, RouteCommand::CancelImport, 6.0);
        assert!(s.routes.pending.is_none());
        assert!(command(&mut s, RouteCommand::ImportPart(PartChoice::All), 7.0).is_err());
        // A file of one part is imported at once, as before.
        let one = dir.join("one.gpx");
        let text = two_segments().replacen("</trkseg><trkseg>", "", 1);
        std::fs::write(&one, text).unwrap();
        let msg = run(&mut s, RouteCommand::Import(one.display().to_string()), 8.0);
        assert!(msg.starts_with("imported"), "{msg}");
        assert!(s.routes.pending.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn console_words_become_route_commands() {
        let w = |s: &str| -> Vec<String> { s.split_whitespace().map(String::from).collect() };
        assert_eq!(
            console_route(&w("plan 0 0 600 600")).unwrap(),
            RouteCommand::Plan(vec![[0.0, 0.0], [600.0, 600.0]])
        );
        assert_eq!(
            console_route(&w("via move 2 10 -20")).unwrap(),
            RouteCommand::MoveVia(1, [10.0, -20.0])
        );
        assert_eq!(
            console_route(&w("package")).unwrap(),
            RouteCommand::PackageRoute(0)
        );
        assert_eq!(
            console_route(&w("package 2")).unwrap(),
            RouteCommand::PackageRoute(1)
        );
        assert_eq!(
            console_route(&w("new Ada loop")).unwrap(),
            RouteCommand::New(Some("Ada loop".into()))
        );
        assert_eq!(
            console_route(&w("loop on")).unwrap(),
            RouteCommand::Loop(true)
        );
        assert_eq!(console_route(&w("fit")).unwrap(), RouteCommand::Fit);
        assert_eq!(
            console_route(&w("import part 2")).unwrap(),
            RouteCommand::ImportPart(PartChoice::One(1))
        );
        assert_eq!(
            console_route(&w("import part all")).unwrap(),
            RouteCommand::ImportPart(PartChoice::All)
        );
        assert_eq!(
            console_route(&w("import cancel")).unwrap(),
            RouteCommand::CancelImport
        );
        assert_eq!(
            console_route(&w("import ~/two rides.gpx")).unwrap(),
            RouteCommand::Import("~/two rides.gpx".into())
        );
        assert_eq!(
            console_route(&w("power 250")).unwrap(),
            RouteCommand::Power(250.0)
        );
        assert!(console_route(&w("power lots")).is_err());
        let v = RouteView {
            line: vec![[0.0, 5.0, 10.0], [30.0, 5.0, -20.0]],
            ..Default::default()
        };
        assert_eq!(route_extent(&v), Some(([0.0, -20.0], [30.0, 10.0])));
        assert_eq!(route_extent(&RouteView::default()), None);
        assert!(console_route(&w("via remove 0")).is_err());
        assert!(console_route(&w("plan 1 2 3")).is_err());
        assert!(
            console_route(&w("fly"))
                .unwrap_err()
                .contains("studio route new")
        );
    }
}
