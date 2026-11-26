//! World patches (spec §14 Phase 16, 16C; decided Q2): edits of a world
//! as a patch layer **on the sources**, a TOML file `<package>.ocwpatch`
//! that `world-builder package --patch FILE` applies after reading OSM and
//! before meshing. A patch is reproducible, reviewable as a diff and stays
//! valid for newer extracts: every operation is keyed to an OSM id, a
//! stable edge id (§4.4) or a geo position, and an operation whose target
//! is gone is **reported** ([`Report`]), never silently dropped.
//!
//! ```toml
//! ocwpatch = 1                      # format version
//! package = "europe/liechtenstein"  # the package id it is made for
//! title = "Malbun road fixes"
//!
//! [[source]]                        # imported data (spec §13), if any
//! what = "surface checked on site"
//! licence = "CC0-1.0"
//!
//! [[op]]
//! op = "road"                       # road attributes
//! way = 4211                        # OSM way …
//! surface = "gravel"
//!
//! [[op]]
//! op = "remove_road"
//! edge = "1234567890123456789"      # … or stable edge id (a string)
//!
//! [[op]]
//! op = "height_point"               # a road profile control point
//! id = "spike"
//! at = [47.0912, 9.5543]            # lat, lon
//! height_m = 1201.5
//! ```
//!
//! This module is pure and wasm-safe (the studio's live preview, 16C):
//! the format, its checks, the patch stack ([`Plan`]) and the parts that
//! act on a [`RoadGraph`] (edge attributes and removals, height points).
//! The OSM tag edits of way targets live in `ocw_worldgen::patch`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::RoadSurface;
use crate::geo::LocalProjection;
use crate::graph::{EdgeId, Oneway, PROFILE_STEP_M, RoadClass, RoadGraph, VertexId};

/// The format version this code reads and writes (`ocwpatch = 1`).
pub const FORMAT_VERSION: u32 = 1;
/// File extension of world patches.
pub const EXTENSION: &str = "ocwpatch";
/// Radius of a height point's influence along the road when not given.
pub const DEFAULT_RADIUS_M: f64 = 100.0;
/// How far a height point's position may be from a road centreline.
pub const SNAP_M: f64 = 15.0;

/// A world patch file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldPatch {
    /// Format version ([`FORMAT_VERSION`]).
    pub ocwpatch: u32,
    /// Id of the package the patch is made for (its manifest id, e.g.
    /// `europe/liechtenstein`).
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Imported data used by the operations, with its licence (spec §13:
    /// the user's own or openly licensed).
    #[serde(default, rename = "source", skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// The operations, applied in order (a later one wins).
    #[serde(default, rename = "op", skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<Operation>,
    /// OCW Studio's saved camera views (16H Q13 (b)): named bookmarks that
    /// travel with the patch. Optional and written only when there are
    /// some, so a patch without views reads in every version; builds
    /// ignore them.
    #[serde(default, rename = "view", skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<PatchView>,
}

/// A saved camera view in a patch (`[[view]]`, 16H Q13 (b)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchView {
    pub name: String,
    /// The eye in the package's metres (x, height, z).
    pub pos: [f64; 3],
    /// Yaw (rad, counter-clockwise from north) and pitch (rad, up).
    pub yaw: f64,
    pub pitch: f64,
}

/// Imported data a patch uses (spec §13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// What it is, e.g. "own GPX track of the park path, 2026-11".
    pub what: String,
    /// Licence (SPDX id or name), e.g. `CC0-1.0`, `ODbL-1.0`, "own work".
    pub licence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// One operation. The `op` key names the kind; later kinds (split / join,
/// new connections, bridges and tunnels, terrain stamps, areas, objects,
/// buildings, ride metadata – 16C) are added as new variants without
/// changing the existing ones, so old patches keep their meaning. A patch
/// with a kind this code does not know is refused with its line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Change a road's attributes.
    Road(RoadEdit),
    /// Take a road out of the ridable graph.
    RemoveRoad(RemoveRoad),
    /// Add a road profile control point.
    HeightPoint(HeightPoint),
    /// Move or change a control point defined earlier (in this patch or
    /// in an earlier patch of the stack).
    MoveHeightPoint(MoveHeightPoint),
    /// Remove a control point defined earlier.
    RemoveHeightPoint(RemoveHeightPoint),
}

/// Road class names in patches: the OSM `highway` values of the classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchClass {
    Trunk,
    Primary,
    Secondary,
    Tertiary,
    Unclassified,
    Residential,
    Service,
    Track,
    Cycleway,
    Path,
}

impl PatchClass {
    pub fn road_class(self) -> RoadClass {
        match self {
            Self::Trunk => RoadClass::Trunk,
            Self::Primary => RoadClass::Primary,
            Self::Secondary => RoadClass::Secondary,
            Self::Tertiary => RoadClass::Tertiary,
            Self::Unclassified => RoadClass::Unclassified,
            Self::Residential => RoadClass::Residential,
            Self::Service => RoadClass::Service,
            Self::Track => RoadClass::Track,
            Self::Cycleway => RoadClass::Cycleway,
            Self::Path => RoadClass::Path,
        }
    }

    /// The OSM `highway` value.
    pub fn osm(self) -> &'static str {
        match self {
            Self::Trunk => "trunk",
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Tertiary => "tertiary",
            Self::Unclassified => "unclassified",
            Self::Residential => "residential",
            Self::Service => "service",
            Self::Track => "track",
            Self::Cycleway => "cycleway",
            Self::Path => "path",
        }
    }
}

/// One-way for bicycles, along the OSM way's (and its edges') direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOneway {
    No,
    Forward,
    Backward,
}

impl PatchOneway {
    pub fn oneway(self) -> Oneway {
        match self {
            Self::No => Oneway::No,
            Self::Forward => Oneway::Forward,
            Self::Backward => Oneway::Backward,
        }
    }
}

/// Bike access of a road.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BikeAccess {
    /// Closed for bikes (16H Q6): the road stays drawn and rideable, with
    /// a warning sign at each of its ends (before v6.22 it was left out of
    /// the graph like OSM's `bicycle=no`; such patches now keep the road).
    Closed,
}

/// `op = "road"`: attributes of a road (way or edge), each optional.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoadEdit {
    /// OSM way id (all edges of the way) …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub way: Option<i64>,
    /// … or one stable edge id (§4.4), written as a string.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "edge_text")]
    pub edge: Option<EdgeId>,
    /// An OSM `surface` value (asphalt, paving_stones, gravel, dirt, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// Carriageway width in metres (1 – 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<PatchClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oneway: Option<PatchOneway>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bike: Option<BikeAccess>,
    /// The road's name; an empty string removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Why (shown in reviews and reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `op = "remove_road"`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveRoad {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub way: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "edge_text")]
    pub edge: Option<EdgeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `op = "height_point"`: the road passes through `height_m` (or
/// `raise_m` above its built profile) at a position on the road; the
/// correction fades out along the road graph (a raised cosine over
/// `radius_m`, across junctions), and the terrain is conformed to the
/// changed road afterwards (spec §4.3 step 4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeightPoint {
    /// Name of the point, unique in the patch stack (moves and removals
    /// refer to it).
    pub id: String,
    /// Position as [lat, lon] in degrees …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    /// … or an OSM node on the road.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<i64>,
    /// Only snap to this edge (else the nearest road within [`SNAP_M`]).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "edge_text")]
    pub edge: Option<EdgeId>,
    /// Road height in metres (the DEM's datum) …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f64>,
    /// … or a change of the built profile in metres (negative lowers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raise_m: Option<f64>,
    /// Influence along the road (default [`DEFAULT_RADIUS_M`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `op = "move_height_point"`: new position and / or height of a point.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveHeightPoint {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "edge_text")]
    pub edge: Option<EdgeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raise_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `op = "remove_height_point"`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveHeightPoint {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Stable edge ids are 64-bit (beyond TOML's integers): written as decimal
/// strings, as in `ocw-graph-route` files; small ids may be integers.
mod edge_text {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::graph::EdgeId;

    pub fn serialize<S: Serializer>(id: &Option<EdgeId>, s: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => s.serialize_str(&id.0.to_string()),
            None => s.serialize_none(),
        }
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Number(i64),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<EdgeId>, D::Error> {
        use serde::de::Error;
        match Option::<Raw>::deserialize(d)? {
            None => Ok(None),
            Some(Raw::Text(t)) => t
                .trim()
                .parse::<u64>()
                .map(|v| Some(EdgeId(v)))
                .map_err(|_| D::Error::custom(format!("edge id `{t}` is not a number"))),
            Some(Raw::Number(n)) => u64::try_from(n)
                .map(|v| Some(EdgeId(v)))
                .map_err(|_| D::Error::custom(format!("edge id {n} is negative"))),
        }
    }
}

/// A patch that could not be read, with its line where known.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchError {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for PatchError {}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].matches('\n').count() + 1
}

/// The patch as read, with the operations' spans.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPatch {
    ocwpatch: u32,
    package: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source: Vec<Source>,
    #[serde(default)]
    op: Vec<toml::Spanned<Operation>>,
    #[serde(default)]
    view: Vec<PatchView>,
}

#[derive(Deserialize)]
struct Head {
    ocwpatch: Option<toml::Value>,
}

impl WorldPatch {
    /// An empty patch for `package`.
    pub fn new(package: impl Into<String>) -> Self {
        Self {
            ocwpatch: FORMAT_VERSION,
            package: package.into(),
            title: None,
            author: None,
            description: None,
            sources: Vec::new(),
            ops: Vec::new(),
            views: Vec::new(),
        }
    }

    /// Reads and checks a patch.
    pub fn parse(text: &str) -> Result<Self, PatchError> {
        Self::parse_located(text).map(|(p, _)| p)
    }

    /// Reads and checks a patch; also returns the line of each operation.
    pub fn parse_located(text: &str) -> Result<(Self, Vec<usize>), PatchError> {
        let toml_err = |e: toml::de::Error| PatchError {
            line: e.span().map(|s| line_of(text, s.start)),
            message: e.message().to_string(),
        };
        // The version first, so a newer file says so instead of failing
        // on its first new key.
        let head: Head = toml::from_str(text).map_err(toml_err)?;
        match head.ocwpatch {
            None => {
                return Err(PatchError {
                    line: None,
                    message: format!(
                        "not a world patch: the `ocwpatch = {FORMAT_VERSION}` line is missing"
                    ),
                });
            }
            Some(toml::Value::Integer(v)) if v > i64::from(FORMAT_VERSION) => {
                return Err(PatchError {
                    line: None,
                    message: format!(
                        "patch format {v} is newer than this program reads ({FORMAT_VERSION}); update OCW"
                    ),
                });
            }
            Some(toml::Value::Integer(v)) if v >= 1 => {}
            Some(v) => {
                return Err(PatchError {
                    line: None,
                    message: format!("`ocwpatch` must be a format version like 1, not {v}"),
                });
            }
        }
        let raw: RawPatch = toml::from_str(text).map_err(toml_err)?;
        let lines: Vec<usize> = raw
            .op
            .iter()
            .map(|s| line_of(text, s.span().start))
            .collect();
        let patch = WorldPatch {
            ocwpatch: raw.ocwpatch,
            package: raw.package,
            title: raw.title,
            author: raw.author,
            description: raw.description,
            sources: raw.source,
            ops: raw.op.into_iter().map(toml::Spanned::into_inner).collect(),
            views: raw.view,
        };
        patch.check(&lines)?;
        Ok((patch, lines))
    }

    /// The TOML text (reads back to the same patch).
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a world patch serialises")
    }

    /// The format rules beyond the types: one target per operation, values
    /// in range, known surfaces, unique height point ids.
    fn check(&self, lines: &[usize]) -> Result<(), PatchError> {
        if let Some(m) = self.header_problem() {
            return Err(PatchError {
                line: None,
                message: m,
            });
        }
        if let Some((i, m)) = self.op_problems().into_iter().next() {
            return Err(PatchError {
                line: lines.get(i).copied(),
                message: format!("[[op]] {} ({}): {m}", i + 1, self.ops[i].kind()),
            });
        }
        Ok(())
    }

    /// What is wrong with the header and the sources (the first problem),
    /// as [`WorldPatch::parse`] would refuse it.
    pub fn header_problem(&self) -> Option<String> {
        if self.package.trim().is_empty() {
            return Some("`package` (the package id) is empty".into());
        }
        if self
            .sources
            .iter()
            .any(|s| s.what.trim().is_empty() || s.licence.trim().is_empty())
        {
            return Some("every [[source]] needs `what` and `licence` (spec §13)".into());
        }
        if let Some(v) = self.views.iter().find(|v| {
            v.name.trim().is_empty()
                || !(v.pos.iter().all(|c| c.is_finite())
                    && v.yaw.is_finite()
                    && v.pitch.is_finite())
        }) {
            return Some(format!(
                "every [[view]] needs a name and finite numbers ({:?})",
                v.name
            ));
        }
        None
    }

    /// Every operation that breaks the format rules (index, what is
    /// wrong): the same checks [`WorldPatch::parse`] makes, but all of
    /// them – an editor shows each (the studio's validator, 16C).
    pub fn op_problems(&self) -> Vec<(usize, String)> {
        let mut ids = BTreeSet::new();
        self.ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| check_op(op, &mut ids).err().map(|m| (i, m)))
            .collect()
    }
}

fn one_target(way: Option<i64>, edge: Option<EdgeId>) -> Result<(), String> {
    match (way, edge) {
        (Some(_), None) | (None, Some(_)) => Ok(()),
        _ => Err("needs exactly one target: `way` (OSM way id) or `edge` (stable edge id)".into()),
    }
}

fn check_at(at: Option<[f64; 2]>) -> Result<(), String> {
    if let Some([lat, lon]) = at
        && !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
    {
        return Err(format!(
            "`at` must be [lat, lon] in degrees, not [{lat}, {lon}]"
        ));
    }
    Ok(())
}

fn check_height(h: Option<f64>, raise: Option<f64>, radius: Option<f64>) -> Result<(), String> {
    if h.is_some() && raise.is_some() {
        return Err("give `height_m` or `raise_m`, not both".into());
    }
    if let Some(h) = h
        && !(-500.0..=9000.0).contains(&h)
    {
        return Err(format!("`height_m` {h} is out of range"));
    }
    if let Some(r) = raise
        && !(-500.0..=500.0).contains(&r)
    {
        return Err(format!("`raise_m` {r} is out of range (±500 m)"));
    }
    if let Some(r) = radius
        && !(5.0..=5000.0).contains(&r)
    {
        return Err(format!("`radius_m` {r} must be 5 – 5000 m"));
    }
    Ok(())
}

fn check_op(op: &Operation, ids: &mut BTreeSet<String>) -> Result<(), String> {
    match op {
        Operation::Road(r) => {
            one_target(r.way, r.edge)?;
            if r.surface.is_none()
                && r.width_m.is_none()
                && r.class.is_none()
                && r.oneway.is_none()
                && r.bike.is_none()
                && r.name.is_none()
            {
                return Err(
                    "changes nothing: give surface, width_m, class, oneway, bike or name".into(),
                );
            }
            if let Some(s) = &r.surface
                && RoadSurface::from_surface_tag(s).is_none()
            {
                return Err(format!("unknown surface `{s}` (an OSM surface value)"));
            }
            if let Some(w) = r.width_m
                && !(1.0..=30.0).contains(&w)
            {
                return Err(format!("`width_m` {w} must be 1 – 30 m"));
            }
            Ok(())
        }
        Operation::RemoveRoad(r) => one_target(r.way, r.edge),
        Operation::HeightPoint(h) => {
            if h.id.trim().is_empty() {
                return Err("`id` is empty".into());
            }
            if !ids.insert(h.id.clone()) {
                return Err(format!("height point `{}` is defined twice", h.id));
            }
            if h.at.is_some() == h.node.is_some() {
                return Err("needs exactly one position: `at` = [lat, lon] or `node`".into());
            }
            check_at(h.at)?;
            if h.height_m.is_none() && h.raise_m.is_none() {
                return Err("needs `height_m` or `raise_m`".into());
            }
            check_height(h.height_m, h.raise_m, h.radius_m)
        }
        Operation::MoveHeightPoint(m) => {
            if m.at.is_some() && m.node.is_some() {
                return Err("give `at` or `node`, not both".into());
            }
            check_at(m.at)?;
            if m.at.is_none()
                && m.node.is_none()
                && m.edge.is_none()
                && m.height_m.is_none()
                && m.raise_m.is_none()
                && m.radius_m.is_none()
            {
                return Err(
                    "changes nothing: give at, node, edge, height_m, raise_m or radius_m".into(),
                );
            }
            check_height(m.height_m, m.raise_m, m.radius_m)
        }
        Operation::RemoveHeightPoint(r) => {
            if r.id.trim().is_empty() {
                return Err("`id` is empty".into());
            }
            Ok(())
        }
    }
}

impl Operation {
    /// The `op` name.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Road(_) => "road",
            Self::RemoveRoad(_) => "remove_road",
            Self::HeightPoint(_) => "height_point",
            Self::MoveHeightPoint(_) => "move_height_point",
            Self::RemoveHeightPoint(_) => "remove_height_point",
        }
    }

    /// A short description for reports: kind and target.
    pub fn describe(&self) -> String {
        let target = |way: Option<i64>, edge: Option<EdgeId>| match (way, edge) {
            (Some(w), _) => format!("way {w}"),
            (_, Some(e)) => format!("edge {}", e.0),
            _ => "?".into(),
        };
        match self {
            Self::Road(r) => format!("road {}", target(r.way, r.edge)),
            Self::RemoveRoad(r) => format!("remove_road {}", target(r.way, r.edge)),
            Self::HeightPoint(h) => format!("height_point `{}`", h.id),
            Self::MoveHeightPoint(m) => format!("move_height_point `{}`", m.id),
            Self::RemoveHeightPoint(r) => format!("remove_height_point `{}`", r.id),
        }
    }
}

// ---------------------------------------------------------------------------
// The patch stack and its report

/// An operation's place: patch number in the stack and operation number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpRef {
    pub patch: usize,
    pub op: usize,
}

/// A patch of a stack, with its file name and the lines of its operations.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPatch {
    /// File name (or another label) for reports.
    pub label: String,
    pub patch: WorldPatch,
    pub lines: Vec<usize>,
}

impl LoadedPatch {
    pub fn parse(label: impl Into<String>, text: &str) -> Result<Self, PatchError> {
        let (patch, lines) = WorldPatch::parse_located(text)?;
        Ok(Self {
            label: label.into(),
            patch,
            lines,
        })
    }
}

/// What became of an operation.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Applied (with what it did).
    Applied(String),
    /// Its target is gone (or never was): **not applied**.
    Missing(String),
    /// Could not be checked here (e.g. an OSM node target without the
    /// OSM sources).
    Unchecked(String),
}

/// One line of a [`Report`].
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `file:line`.
    pub place: String,
    /// Kind and target.
    pub what: String,
    pub outcome: Outcome,
}

/// What each operation of a stack did. A stage marks the operations it
/// handles; `Applied` wins over `Missing` (an edge found in the context
/// graph but not in the region's counts as applied).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    entries: BTreeMap<OpRef, Entry>,
}

impl Report {
    /// Every operation of the stack, not yet handled.
    pub fn new(stack: &[LoadedPatch]) -> Self {
        let mut entries = BTreeMap::new();
        for (p, lp) in stack.iter().enumerate() {
            for (o, op) in lp.patch.ops.iter().enumerate() {
                let place = match lp.lines.get(o) {
                    Some(l) => format!("{}:{l}", lp.label),
                    None => lp.label.clone(),
                };
                entries.insert(
                    OpRef { patch: p, op: o },
                    Entry {
                        place,
                        what: op.describe(),
                        outcome: Outcome::Unchecked("not handled by this build".into()),
                    },
                );
            }
        }
        Self { entries }
    }

    fn set(&mut self, at: OpRef, outcome: Outcome) {
        let Some(e) = self.entries.get_mut(&at) else {
            return;
        };
        let keep = match (&e.outcome, &outcome) {
            (Outcome::Applied(_), Outcome::Applied(_)) => false,
            (Outcome::Applied(_), _) => true,
            (Outcome::Missing(_), Outcome::Unchecked(_)) => true,
            _ => false,
        };
        if !keep {
            e.outcome = outcome;
        }
    }

    pub fn applied(&mut self, at: OpRef, what: impl Into<String>) {
        self.set(at, Outcome::Applied(what.into()));
    }

    pub fn missing(&mut self, at: OpRef, why: impl Into<String>) {
        self.set(at, Outcome::Missing(why.into()));
    }

    pub fn unchecked(&mut self, at: OpRef, why: impl Into<String>) {
        self.set(at, Outcome::Unchecked(why.into()));
    }

    pub fn get(&self, at: OpRef) -> Option<&Entry> {
        self.entries.get(&at)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&OpRef, &Entry)> {
        self.entries.iter()
    }

    /// The operations not applied because their target is gone.
    pub fn missing_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| matches!(e.outcome, Outcome::Missing(_)))
            .count()
    }

    pub fn unchecked_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| matches!(e.outcome, Outcome::Unchecked(_)))
            .count()
    }

    /// One line per operation and a summary, for the console.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .values()
            .map(|e| match &e.outcome {
                Outcome::Applied(s) => format!("  ok       {} {}: {s}", e.place, e.what),
                Outcome::Missing(s) => format!("  MISSING  {} {}: {s}", e.place, e.what),
                Outcome::Unchecked(s) => format!("  ?        {} {}: {s}", e.place, e.what),
            })
            .collect();
        out.push(format!(
            "patch: {} operations, {} applied, {} missing (target gone, not applied), {} unchecked",
            self.entries.len(),
            self.entries.len() - self.missing_count() - self.unchecked_count(),
            self.missing_count(),
            self.unchecked_count()
        ));
        out
    }
}

/// A road target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Way(i64),
    Edge(EdgeId),
}

impl Target {
    fn of(way: Option<i64>, edge: Option<EdgeId>) -> Self {
        match (way, edge) {
            (Some(w), _) => Self::Way(w),
            (_, Some(e)) => Self::Edge(e),
            _ => unreachable!("checked on parsing"),
        }
    }
}

/// Where a height point is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    /// (lat, lon) in degrees.
    At(f64, f64),
    /// An OSM node; [`Plan::resolve_nodes`] turns it into `At`.
    Node(i64),
}

/// The height a point asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Height {
    Absolute(f64),
    Raise(f64),
}

/// A height point after the stack's moves and removals.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPoint {
    pub id: String,
    pub position: Position,
    pub edge: Option<EdgeId>,
    pub height: Height,
    pub radius_m: f64,
    /// The operation that defined it and those that moved it.
    pub ops: Vec<OpRef>,
}

/// A road edit of the stack, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRoad {
    pub at: OpRef,
    pub target: Target,
    pub edit: RoadEdit,
}

/// The patch stack resolved into what to apply: road edits and removals
/// in order, and the height points after all moves and removals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub roads: Vec<PlannedRoad>,
    pub removals: Vec<(OpRef, Target)>,
    pub points: Vec<ResolvedPoint>,
}

impl Plan {
    /// Resolves the stack (patches in order; a later one may move or
    /// remove an earlier one's height points). Moves and removals of
    /// unknown points, and a point defined twice, go into the report.
    pub fn new(stack: &[LoadedPatch], report: &mut Report) -> Self {
        let mut plan = Plan::default();
        let mut points: BTreeMap<String, usize> = BTreeMap::new();
        let mut slots: Vec<Option<ResolvedPoint>> = Vec::new();
        for (p, lp) in stack.iter().enumerate() {
            for (o, op) in lp.patch.ops.iter().enumerate() {
                let at = OpRef { patch: p, op: o };
                match op {
                    Operation::Road(r) => plan.roads.push(PlannedRoad {
                        at,
                        target: Target::of(r.way, r.edge),
                        edit: r.clone(),
                    }),
                    Operation::RemoveRoad(r) => {
                        plan.removals.push((at, Target::of(r.way, r.edge)));
                    }
                    Operation::HeightPoint(h) => {
                        if points.contains_key(&h.id) {
                            report.missing(
                                at,
                                format!(
                                    "height point `{}` is already defined earlier in the stack",
                                    h.id
                                ),
                            );
                            continue;
                        }
                        points.insert(h.id.clone(), slots.len());
                        slots.push(Some(ResolvedPoint {
                            id: h.id.clone(),
                            position: match (h.at, h.node) {
                                (Some([lat, lon]), _) => Position::At(lat, lon),
                                (_, Some(n)) => Position::Node(n),
                                _ => unreachable!("checked on parsing"),
                            },
                            edge: h.edge,
                            height: match (h.height_m, h.raise_m) {
                                (Some(v), _) => Height::Absolute(v),
                                (_, Some(r)) => Height::Raise(r),
                                _ => unreachable!("checked on parsing"),
                            },
                            radius_m: h.radius_m.unwrap_or(DEFAULT_RADIUS_M),
                            ops: vec![at],
                        }));
                    }
                    Operation::MoveHeightPoint(m) => {
                        let Some(pt) = points.get(&m.id).and_then(|&i| slots[i].as_mut()) else {
                            report.missing(at, format!("no height point `{}` before this", m.id));
                            continue;
                        };
                        if let Some([lat, lon]) = m.at {
                            pt.position = Position::At(lat, lon);
                        }
                        if let Some(n) = m.node {
                            pt.position = Position::Node(n);
                        }
                        if m.edge.is_some() {
                            pt.edge = m.edge;
                        }
                        if let Some(v) = m.height_m {
                            pt.height = Height::Absolute(v);
                        }
                        if let Some(r) = m.raise_m {
                            pt.height = Height::Raise(r);
                        }
                        if let Some(r) = m.radius_m {
                            pt.radius_m = r;
                        }
                        pt.ops.push(at);
                    }
                    Operation::RemoveHeightPoint(r) => {
                        match points.remove(&r.id).and_then(|i| slots[i].take()) {
                            Some(pt) => {
                                for o in pt.ops {
                                    report.applied(o, format!("removed by {}", lp.label));
                                }
                                report.applied(at, "removed");
                            }
                            None => report
                                .missing(at, format!("no height point `{}` before this", r.id)),
                        }
                    }
                }
            }
        }
        plan.points = slots.into_iter().flatten().collect();
        plan
    }

    /// Turns OSM node positions into coordinates (`node` gives a node's
    /// lat / lon); points on unknown nodes are reported and left out.
    pub fn resolve_nodes(&mut self, node: impl Fn(i64) -> Option<(f64, f64)>, report: &mut Report) {
        self.points.retain_mut(|pt| {
            let Position::Node(n) = pt.position else {
                return true;
            };
            match node(n) {
                Some((lat, lon)) => {
                    pt.position = Position::At(lat, lon);
                    true
                }
                None => {
                    for &o in &pt.ops {
                        report.missing(o, format!("OSM node {n} is not in the data"));
                    }
                    false
                }
            }
        });
    }

    /// The surfaces asked for edge targets, by edge id (for the package's
    /// road surface section; way targets change the OSM tags instead).
    pub fn edge_surfaces(&self) -> BTreeMap<EdgeId, RoadSurface> {
        let mut out = BTreeMap::new();
        for r in &self.roads {
            if let (Target::Edge(id), Some(s)) = (r.target, &r.edit.surface)
                && let Some(s) = RoadSurface::from_surface_tag(s)
            {
                out.insert(id, s);
            }
        }
        out
    }
}

/// Which road targets [`apply_road_edits`] handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Targets {
    /// Only edge targets: the builder applies way targets to the OSM tags
    /// before the graph is built.
    EdgesOnly,
    /// Edge and way targets (way = every edge with that `osm_way`): a
    /// built package's graph (`world-builder patch check`, previews).
    All,
}

/// Road attributes and removals on a graph, in plan order. Removed edges
/// (and roads closed for bikes) leave the graph. Returns the road
/// surfaces asked for, by edge id.
pub fn apply_road_edits(
    plan: &Plan,
    graph: &mut RoadGraph,
    targets: Targets,
    report: &mut Report,
) -> BTreeMap<EdgeId, RoadSurface> {
    let mut surfaces = BTreeMap::new();
    let matches = |graph: &RoadGraph, t: Target| -> Vec<usize> {
        (0..graph.edges.len())
            .filter(|&i| match t {
                Target::Way(w) => graph.edges[i].osm_way == w,
                Target::Edge(id) => graph.edges[i].id == id,
            })
            .collect()
    };
    let handled = |t: Target| targets == Targets::All || matches!(t, Target::Edge(_));
    let gone = |t: Target| match t {
        Target::Way(w) => format!("way {w} is not in the road graph"),
        Target::Edge(e) => format!("edge {} is not in the road graph", e.0),
    };
    let mut remove = BTreeSet::new();
    for r in &plan.roads {
        if !handled(r.target) {
            continue;
        }
        let hit = matches(graph, r.target);
        if hit.is_empty() {
            report.missing(r.at, gone(r.target));
            continue;
        }
        let ed = &r.edit;
        for &i in &hit {
            let e = &mut graph.edges[i];
            if let Some(c) = ed.class {
                let old_default = e.class.default_width_m();
                e.class = c.road_class();
                if (e.width_m - old_default).abs() < 1e-9 {
                    e.width_m = e.class.default_width_m();
                }
            }
            if let Some(w) = ed.width_m {
                e.width_m = w;
            }
            if let Some(o) = ed.oneway {
                e.oneway = o.oneway();
            }
            if let Some(n) = &ed.name {
                e.name = (!n.is_empty()).then(|| n.clone());
            }
            if let Some(s) = ed
                .surface
                .as_deref()
                .and_then(RoadSurface::from_surface_tag)
            {
                surfaces.insert(e.id, s);
            }
            // Closed for bikes stays drawn and rideable, with warning
            // signs at its ends (16H Q6; [`closed_edges`], `bike_signs`).
        }
        let what = if ed.bike == Some(BikeAccess::Closed) {
            format!(
                "{} edge(s); closed for bikes: rideable, warning signs at its ends",
                hit.len()
            )
        } else {
            format!("{} edge(s)", hit.len())
        };
        report.applied(r.at, what);
    }
    for &(at, t) in &plan.removals {
        if !handled(t) {
            continue;
        }
        let hit = matches(graph, t);
        if hit.is_empty() {
            report.missing(at, gone(t));
            continue;
        }
        report.applied(at, format!("{} edge(s) removed", hit.len()));
        remove.extend(hit);
    }
    if !remove.is_empty() {
        let keep: BTreeSet<usize> = (0..graph.edges.len())
            .filter(|i| !remove.contains(i))
            .collect();
        graph.retain_edges(&keep);
    }
    surfaces
}

/// The edges of `graph` that the plan closes for bikes (16H Q6): the
/// targets of its `road` operations with `bike = "closed"` – a way's
/// edges or the edge. They stay in the graph, drawn and rideable; the package
/// gets a warning sign at each end of them (`crate::bike_signs`).
pub fn closed_edges(plan: &Plan, graph: &RoadGraph) -> Vec<usize> {
    let mut closed: BTreeSet<Target> = BTreeSet::new();
    for r in &plan.roads {
        if r.edit.bike == Some(BikeAccess::Closed) {
            closed.insert(r.target);
        }
    }
    (0..graph.edges.len())
        .filter(|&i| {
            let e = &graph.edges[i];
            closed.contains(&Target::Way(e.osm_way)) || closed.contains(&Target::Edge(e.id))
        })
        .collect()
}

/// What a patch stack does to a built package's road graph, without the
/// OSM sources: way and edge targets against the graph ([`Targets::All`]),
/// then the height points (`origin`: the package's projection origin).
/// The report says what each operation did or why it found no target;
/// the graph is the patched one (`world-builder patch check`, the
/// studio's preview, 16C). Node positions stay unchecked.
pub fn preview(
    graph: &RoadGraph,
    origin: (f64, f64),
    stack: &[LoadedPatch],
) -> (Report, RoadGraph) {
    let mut report = Report::new(stack);
    let plan = Plan::new(stack, &mut report);
    let mut g = graph.clone();
    apply_road_edits(&plan, &mut g, Targets::All, &mut report);
    let proj = LocalProjection::new(origin.0, origin.1);
    apply_height_points(&plan, &mut g, &proj, &mut report);
    (report, g)
}

/// Arc position of profile sample `i` on an edge of length `len`.
fn sample_arc(i: usize, len: f64) -> f64 {
    (i as f64 * PROFILE_STEP_M).min(len)
}

/// The nearest point of edge `e` to `p`: (distance, arc position).
fn nearest_on_edge(points: &[[f64; 2]], p: [f64; 2]) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    let mut arc = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let l = l2.sqrt();
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + d[0] * t, a[1] + d[1] * t];
        let dist = (q[0] - p[0]).hypot(q[1] - p[1]);
        if dist < best.0 {
            best = (dist, arc + l * t);
        }
        arc += l;
    }
    best
}

/// Graph distances from a point on edge `edge` at `arc` to the vertices,
/// up to `limit` (Dijkstra; unreached vertices are infinite).
fn vertex_distances(
    graph: &RoadGraph,
    adj: &[Vec<usize>],
    edge: usize,
    arc: f64,
    limit: f64,
) -> Vec<f64> {
    #[derive(PartialEq)]
    struct Item(f64, usize);
    impl Eq for Item {}
    impl PartialOrd for Item {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Item {
        fn cmp(&self, o: &Self) -> Ordering {
            o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
        }
    }
    let mut dist = vec![f64::INFINITY; graph.vertices.len()];
    let e = &graph.edges[edge];
    let len = e.length_m();
    let mut heap = BinaryHeap::new();
    for (v, d) in [(e.from.0 as usize, arc), (e.to.0 as usize, len - arc)] {
        if d < dist[v] {
            dist[v] = d;
            heap.push(Item(d, v));
        }
    }
    while let Some(Item(d, v)) = heap.pop() {
        if d > dist[v] || d > limit {
            continue;
        }
        for &k in &adj[v] {
            let ek = &graph.edges[k];
            let o = ek.other(VertexId(v as u32)).0 as usize;
            let nd = d + ek.length_m();
            if nd < dist[o] {
                dist[o] = nd;
                heap.push(Item(nd, o));
            }
        }
    }
    dist
}

/// A value given at the profile samples of an edge (`at(arc)`), read at
/// `arc` the way [`crate::graph::Edge::height_at`] reads the profile.
fn interpolated(len: f64, samples: usize, arc: f64, at: impl Fn(f64) -> f64) -> f64 {
    if samples == 0 {
        return 0.0;
    }
    let d = arc.clamp(0.0, len);
    let i = ((d / PROFILE_STEP_M).floor() as usize).min(samples - 1);
    if i + 1 >= samples {
        return at(sample_arc(samples - 1, len));
    }
    let seg = (len - i as f64 * PROFILE_STEP_M).clamp(1e-9, PROFILE_STEP_M);
    let t = ((d - i as f64 * PROFILE_STEP_M) / seg).clamp(0.0, 1.0);
    at(sample_arc(i, len)) * (1.0 - t) + at(sample_arc(i + 1, len)) * t
}

/// Weight of a correction at graph distance `d` (raised cosine).
fn falloff(d: f64, radius: f64) -> f64 {
    if d >= radius {
        0.0
    } else {
        0.5 * (1.0 + (std::f64::consts::PI * d / radius).cos())
    }
}

/// A height point placed on the graph.
struct Placed {
    point: usize,
    edge: usize,
    arc: f64,
    radius: f64,
    target: f64,
    dist: Vec<f64>,
}

impl Placed {
    fn distance(&self, graph: &RoadGraph, k: usize, t: f64, len: f64) -> f64 {
        let e = &graph.edges[k];
        self.distance_between(e.from, e.to, k, t, len)
    }

    /// Graph distance to arc `t` of edge `k` (ends `from`, `to`, length
    /// `len`).
    fn distance_between(&self, from: VertexId, to: VertexId, k: usize, t: f64, len: f64) -> f64 {
        let mut d = (self.dist[from.0 as usize] + t).min(self.dist[to.0 as usize] + (len - t));
        if k == self.edge {
            d = d.min((t - self.arc).abs());
        }
        d
    }
}

/// Moves the road profiles through the plan's height points (after the
/// profiles and their cleaning, before the terrain is conformed to the
/// roads). Each point snaps to the nearest road within [`SNAP_M`] (or to
/// its `edge`); the corrections are solved together so that the road
/// passes through every point, each fading out along the graph over its
/// radius, so junctions stay continuous. Points that find no road are
/// reported missing; points without coordinates yet ([`Position::Node`])
/// are reported unchecked.
pub fn apply_height_points(
    plan: &Plan,
    graph: &mut RoadGraph,
    proj: &LocalProjection,
    report: &mut Report,
) {
    if plan.points.is_empty() {
        return;
    }
    let adj = graph.adjacency();
    let mut placed: Vec<Placed> = Vec::new();
    for (i, pt) in plan.points.iter().enumerate() {
        let (lat, lon) = match pt.position {
            Position::At(lat, lon) => (lat, lon),
            Position::Node(n) => {
                for &o in &pt.ops {
                    report.unchecked(o, format!("OSM node {n}: needs the OSM sources"));
                }
                continue;
            }
        };
        let p = proj.project(lat, lon);
        let mut best: Option<(f64, usize, f64)> = None;
        for (k, e) in graph.edges.iter().enumerate() {
            if e.profile.is_empty() || pt.edge.is_some_and(|id| id != e.id) {
                continue;
            }
            let (d, arc) = nearest_on_edge(&e.points, p);
            if d <= SNAP_M && best.is_none_or(|b| d < b.0) {
                best = Some((d, k, arc));
            }
        }
        let Some((_, k, arc)) = best else {
            let why = match pt.edge {
                Some(id) => format!("edge {} is not within {SNAP_M} m of [{lat}, {lon}]", id.0),
                None => format!("no road within {SNAP_M} m of [{lat}, {lon}]"),
            };
            for &o in &pt.ops {
                report.missing(o, why.clone());
            }
            continue;
        };
        let base = graph.edges[k].height_at(arc).unwrap_or(0.0);
        let target = match pt.height {
            Height::Absolute(h) => h,
            Height::Raise(r) => base + r,
        };
        placed.push(Placed {
            point: i,
            edge: k,
            arc,
            radius: pt.radius_m,
            target,
            dist: vertex_distances(graph, &adj, k, arc, pt.radius_m),
        });
    }
    if placed.is_empty() {
        return;
    }
    // Weights between the points: w[i][j] = influence of point j at point i.
    let n = placed.len();
    let lens: Vec<f64> = graph.edges.iter().map(|e| e.length_m()).collect();
    let base: Vec<f64> = placed
        .iter()
        .map(|q| graph.edges[q.edge].height_at(q.arc).unwrap_or(0.0))
        .collect();
    // The effect of a correction on the road height at point i, as
    // `Edge::height_at` reads it (linear between the 5 m samples).
    let w: Vec<Vec<f64>> = placed
        .iter()
        .map(|qi| {
            let e = &graph.edges[qi.edge];
            let len = lens[qi.edge];
            placed
                .iter()
                .map(|qj| {
                    interpolated(len, e.profile.len(), qi.arc, |t| {
                        falloff(qj.distance(graph, qi.edge, t, len), qj.radius)
                    })
                })
                .collect()
        })
        .collect();
    // Gauss–Seidel on the correction amplitudes (the diagonal is about 1).
    let mut c = vec![0.0; n];
    for _ in 0..100 {
        let mut worst: f64 = 0.0;
        for i in 0..n {
            let now: f64 = base[i] + (0..n).map(|j| c[j] * w[i][j]).sum::<f64>();
            let r = placed[i].target - now;
            c[i] += r / w[i][i].max(0.25);
            worst = worst.max(r.abs());
        }
        if worst < 1e-5 {
            break;
        }
    }
    let mut touched = vec![0usize; n];
    for (k, e) in graph.edges.iter_mut().enumerate() {
        let len = lens[k];
        for s in 0..e.profile.len() {
            let t = sample_arc(s, len);
            let mut dh = 0.0;
            for (j, q) in placed.iter().enumerate() {
                let f = falloff(q.distance_between(e.from, e.to, k, t, len), q.radius);
                if f > 0.0 {
                    dh += c[j] * f;
                    touched[j] += 1;
                }
            }
            e.profile[s] += dh;
        }
    }
    for (j, q) in placed.iter().enumerate() {
        let pt = &plan.points[q.point];
        let e = &graph.edges[q.edge];
        let now = e.height_at(q.arc).unwrap_or(0.0);
        let what = format!(
            "edge {} at {:.1} m: road {:.2} → {:.2} m (asked {:.2} m), radius {:.0} m, {} profile samples moved",
            e.id.0, q.arc, base[j], now, q.target, q.radius, touched[j]
        );
        for &o in &pt.ops {
            report.applied(o, what.clone());
        }
    }
}

#[cfg(test)]
mod tests;
