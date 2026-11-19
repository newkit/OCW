//! The world patch in the studio (spec §14 Phase 16, 16C, E5 step 2): its
//! file (`<package>.ocwpatch` next to the package by default; Save, Save
//! as, Open patch), the open package as the patch sees it (its road graph
//! and the indices the views need), and the **preview**: the patch applied
//! to the package's road graph with `ocw_world::patch::preview` – what
//! each operation did or why it found no target, and the road profiles
//! the height points change – recomputed shortly after every change.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ocw_edit::world::RoadTarget;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Edge, RoadGraph};
use ocw_world::package::WorldPackage;
use ocw_world::patch::{LoadedPatch, OpRef, Outcome, Report, WorldPatch};
use ocw_world::road_surface::RoadSurface;

/// The open package, as the patch tools need it.
pub struct StudioPackage {
    pub pkg: Arc<WorldPackage>,
    /// The package file and its manifest.
    pub file: PathBuf,
    pub manifest: Option<PathBuf>,
    /// Edge index by stable edge id, edge indices by OSM way.
    pub edges: HashMap<u64, usize>,
    pub ways: HashMap<i64, Vec<usize>>,
}

impl StudioPackage {
    pub fn new(pkg: Arc<WorldPackage>, file: PathBuf, manifest: Option<PathBuf>) -> Self {
        let mut edges = HashMap::new();
        let mut ways: HashMap<i64, Vec<usize>> = HashMap::new();
        if let Some(g) = &pkg.graph {
            for (i, e) in g.edges.iter().enumerate() {
                edges.insert(e.id.0, i);
                ways.entry(e.osm_way).or_default().push(i);
            }
        }
        Self {
            pkg,
            file,
            manifest,
            edges,
            ways,
        }
    }

    pub fn graph(&self) -> Option<&RoadGraph> {
        self.pkg.graph.as_ref()
    }

    /// The edge with this stable id.
    pub fn edge(&self, id: u64) -> Option<&Edge> {
        let i = *self.edges.get(&id)?;
        self.graph()?.edges.get(i)
    }

    /// The edges a road target covers (indices).
    pub fn target_edges(&self, t: RoadTarget) -> Vec<usize> {
        match t {
            RoadTarget::Edge(e) => self.edges.get(&e).copied().into_iter().collect(),
            RoadTarget::Way(w) => self.ways.get(&w).cloned().unwrap_or_default(),
        }
    }

    pub fn projection(&self) -> LocalProjection {
        LocalProjection::new(self.pkg.origin.0, self.pkg.origin.1)
    }

    /// The default patch file: `<package>.ocwpatch` next to the package.
    pub fn default_patch_path(&self) -> PathBuf {
        default_patch_path(&self.file)
    }

    /// The surface the package draws for an edge (from its tile's road
    /// strips: the strip of the same class nearest to the edge's middle),
    /// else the class's usual one.
    pub fn edge_surface(&self, edge: &Edge) -> (RoadSurface, bool) {
        let mid = crate::pick::point_at(&edge.points, edge.length_m() * 0.5);
        let Some(mid) = mid else {
            return (RoadSurface::of_class(edge.class), false);
        };
        let key = ocw_world::package::TileKey::containing(mid, f64::from(self.pkg.tile_size_m));
        let found = self.pkg.tiles.get(&key).and_then(|t| {
            t.roads
                .iter()
                .filter(|s| s.class == edge.class)
                .map(|s| {
                    let pts: Vec<[f64; 2]> = s
                        .points
                        .iter()
                        .map(|p| [f64::from(p[0]), f64::from(p[2])])
                        .collect();
                    (crate::pick::nearest_on_polyline(&pts, mid).0, s.surface)
                })
                .filter(|(d, _)| *d < 3.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
        });
        match found {
            Some((_, s)) => (s, true),
            None => (RoadSurface::of_class(edge.class), false),
        }
    }

    /// [`StudioPackage::edge_surface`] of every edge of the graph (by
    /// index; empty without a graph), with each tile's strips read once –
    /// the route planner's costs and the route's figures (16D).
    pub fn edge_surfaces(&self) -> Vec<RoadSurface> {
        type Strips = Vec<(ocw_world::graph::RoadClass, Vec<[f64; 2]>, RoadSurface)>;
        let Some(g) = self.graph() else {
            return Vec::new();
        };
        let size = f64::from(self.pkg.tile_size_m);
        let mut tiles: HashMap<ocw_world::package::TileKey, Strips> = HashMap::new();
        g.edges
            .iter()
            .map(|edge| {
                let Some(mid) = crate::pick::point_at(&edge.points, edge.length_m() * 0.5) else {
                    return RoadSurface::of_class(edge.class);
                };
                let key = ocw_world::package::TileKey::containing(mid, size);
                let strips = tiles.entry(key).or_insert_with(|| {
                    self.pkg.tiles.get(&key).map_or_else(Vec::new, |t| {
                        t.roads
                            .iter()
                            .map(|s| {
                                let pts = s
                                    .points
                                    .iter()
                                    .map(|p| [f64::from(p[0]), f64::from(p[2])])
                                    .collect();
                                (s.class, pts, s.surface)
                            })
                            .collect()
                    })
                });
                strips
                    .iter()
                    .filter(|(c, _, _)| *c == edge.class)
                    .map(|(_, pts, s)| (crate::pick::nearest_on_polyline(pts, mid).0, *s))
                    .filter(|(d, _)| *d < 3.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map_or(RoadSurface::of_class(edge.class), |(_, s)| s)
            })
            .collect()
    }
}

/// `<package>.ocwpatch` next to `package`.
pub fn default_patch_path(package: &Path) -> PathBuf {
    package.with_extension(ocw_world::patch::EXTENSION)
}

/// Reads a patch file for the package `package_id`; a patch for another
/// package is refused (it would act on the wrong roads).
pub fn read_patch(path: &Path, package_id: &str) -> Result<WorldPatch, String> {
    let shown = crate::edit::shown_path(path);
    let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
    let patch = WorldPatch::parse(&text).map_err(|e| format!("{shown}: {e}"))?;
    if patch.package != package_id {
        return Err(format!(
            "{shown} is a patch for package `{}`, but the open package is `{package_id}`: not loaded",
            patch.package
        ));
    }
    Ok(patch)
}

/// Writes a patch file (to a temporary file next to it, then renamed, so
/// a failed write never leaves half a patch).
pub fn write_patch(path: &Path, patch: &WorldPatch) -> Result<(), String> {
    let shown = crate::edit::shown_path(path);
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{shown}: {e}"))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let text = format!(
        "# World patch made with OCW Studio (spec §14 Phase 16, 16C).\n{}",
        patch.to_toml()
    );
    std::fs::write(&tmp, text).map_err(|e| format!("{shown}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{shown}: {e}"))
}

/// What the preview found for one operation.
#[derive(Debug, Clone, PartialEq)]
pub enum OpStatus {
    /// Applied (what it did).
    Ok(String),
    /// Its target is gone: `world-builder` would not apply it.
    Missing(String),
    /// Not checked (an OSM node target needs the OSM sources).
    Unchecked(String),
    /// Not computed (yet, or the patch has errors).
    Pending,
}

impl OpStatus {
    /// The short tag of the operations list.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Missing(_) => "MISSING",
            Self::Unchecked(_) => "?",
            Self::Pending => "…",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::Ok(s) | Self::Missing(s) | Self::Unchecked(s) => s,
            Self::Pending => "",
        }
    }
}

/// The patch applied to the package's graph.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    pub report: Option<Report>,
    /// The new road profiles of the edges whose height the patch changes
    /// (by stable edge id).
    pub profiles: BTreeMap<u64, Vec<f64>>,
    /// Why there is no report.
    pub note: Option<String>,
}

impl Preview {
    /// Computes the preview of `patch` on `pkg` (label: the patch file's
    /// name in the report). A patch with errors is not applied.
    pub fn of(pkg: &StudioPackage, patch: &WorldPatch, label: &str) -> Self {
        let Some(graph) = pkg.graph() else {
            return Self {
                note: Some("the package has no road graph".into()),
                ..Default::default()
            };
        };
        if patch.header_problem().is_some() || !patch.op_problems().is_empty() {
            return Self {
                note: Some("the patch has errors (see the findings)".into()),
                ..Default::default()
            };
        }
        let stack = [LoadedPatch {
            label: label.into(),
            patch: patch.clone(),
            lines: Vec::new(),
        }];
        let (report, patched) = ocw_world::patch::preview(graph, pkg.pkg.origin, &stack);
        let mut profiles = BTreeMap::new();
        for e in &patched.edges {
            let Some(&i) = pkg.edges.get(&e.id.0) else {
                continue;
            };
            let old = &graph.edges[i].profile;
            if old.len() == e.profile.len()
                && old
                    .iter()
                    .zip(&e.profile)
                    .any(|(a, b)| (a - b).abs() > 0.01)
            {
                profiles.insert(e.id.0, e.profile.clone());
            }
        }
        Self {
            report: Some(report),
            profiles,
            note: None,
        }
    }

    /// The status of operation `i` of the patch.
    pub fn status(&self, i: usize) -> OpStatus {
        let Some(e) = self
            .report
            .as_ref()
            .and_then(|r| r.get(OpRef { patch: 0, op: i }))
        else {
            return OpStatus::Pending;
        };
        match &e.outcome {
            Outcome::Applied(s) => OpStatus::Ok(s.clone()),
            Outcome::Missing(s) => OpStatus::Missing(s.clone()),
            Outcome::Unchecked(s) => OpStatus::Unchecked(s.clone()),
        }
    }
}

/// The preview of the open patch, recomputed a little after it changed
/// (not on every keystroke).
#[derive(bevy::prelude::Resource, Default)]
pub struct PatchPreview {
    /// The document state the preview is of (0: none yet).
    pub state: Option<u64>,
    /// When the document's state last differed from `state` (s).
    pub changed_at: Option<f64>,
    pub preview: Preview,
}

/// Seconds after the last change the preview is recomputed.
pub const PREVIEW_DELAY_S: f64 = 0.4;

impl PatchPreview {
    /// Whether the preview of document state `state` is due at `now`.
    pub fn due(&mut self, state: u64, now: f64) -> bool {
        if self.state == Some(state) {
            self.changed_at = None;
            return false;
        }
        let since = *self.changed_at.get_or_insert(now);
        // The first preview of a document comes at once.
        self.state.is_none() || now - since >= PREVIEW_DELAY_S
    }
}

/// Recomputes the preview when the patch changed.
pub fn update_preview(
    studio: bevy::prelude::Res<crate::edit::Studio>,
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>,
    mut preview: bevy::prelude::ResMut<PatchPreview>,
) {
    let (Some(doc), Some(pkg)) = (studio.world(), studio.package.as_ref()) else {
        return;
    };
    // The document id is in the key: another document starts over.
    let state = doc.state() ^ (doc.id.0 << 48);
    if !preview.due(state, time.elapsed_secs_f64()) {
        return;
    }
    let label = studio
        .patch_file
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or_else(|| "patch".into(), |n| n.to_string_lossy().into_owned());
    preview.preview = Preview::of(pkg, &doc.content().patch, &label);
    preview.state = Some(state);
    preview.changed_at = None;
}

#[cfg(test)]
pub mod tests {
    use ocw_world::graph::EdgeId;
    use ocw_world::patch::{HeightPoint, Operation, RemoveRoad, RoadEdit};

    use super::*;

    /// The map test package with the picking test graph.
    pub fn package() -> StudioPackage {
        let mut p = crate::map::tests::package();
        p.graph = Some(crate::pick::tests::graph());
        StudioPackage::new(Arc::new(p), PathBuf::from("/data/testland.ocwpkg"), None)
    }

    #[test]
    fn the_package_indexes_edges_and_ways() {
        let p = package();
        assert_eq!(p.edge(300).unwrap().osm_way, 7);
        assert_eq!(p.target_edges(RoadTarget::Way(7)), vec![0, 2]);
        assert_eq!(p.target_edges(RoadTarget::Edge(200)), vec![1]);
        assert!(p.target_edges(RoadTarget::Edge(9)).is_empty());
        assert_eq!(
            p.default_patch_path(),
            PathBuf::from("/data/testland.ocwpatch")
        );
        // The tile strips of the test package are secondary roads.
        let (s, measured) = p.edge_surface(p.edge(100).unwrap());
        assert_eq!((s, measured), (RoadSurface::Asphalt, false));
        let mut e = p.edge(100).unwrap().clone();
        e.class = ocw_world::graph::RoadClass::Secondary;
        e.points = vec![[0.0, 50.0], [90.0, 50.0]];
        assert_eq!(p.edge_surface(&e), (RoadSurface::Asphalt, true));
        // All edges at once: the same answers.
        let all = p.edge_surfaces();
        let g = p.graph().unwrap();
        assert_eq!(all.len(), g.edges.len());
        for (i, e) in g.edges.iter().enumerate() {
            assert_eq!(all[i], p.edge_surface(e).0);
        }
    }

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ocw-studio-patch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn sample_patch() -> WorldPatch {
        let mut p = WorldPatch::new("local/testland");
        p.title = Some("Testland fixes".into());
        p.author = Some("Ada Rider".into());
        p.ops = vec![
            Operation::Road(RoadEdit {
                edge: Some(EdgeId(u64::MAX - 1)),
                surface: Some("gravel".into()),
                name: Some("Alter Weg".into()),
                width_m: Some(3.25),
                ..Default::default()
            }),
            Operation::RemoveRoad(RemoveRoad {
                way: Some(8),
                note: Some("closed on site".into()),
                ..Default::default()
            }),
            Operation::HeightPoint(HeightPoint {
                id: "point-1".into(),
                at: Some([47.000_123_456_789, 9.000_987_654_321]),
                edge: Some(EdgeId(100)),
                height_m: Some(401.7),
                radius_m: Some(60.0),
                ..Default::default()
            }),
        ];
        p
    }

    #[test]
    fn a_saved_patch_opens_identical_and_another_package_is_refused() {
        let d = dir();
        let path = d.join("sub").join("testland.ocwpatch");
        let p = sample_patch();
        write_patch(&path, &p).unwrap();
        assert_eq!(read_patch(&path, "local/testland").unwrap(), p);
        assert!(!path.with_extension("ocwpatch.tmp").exists());
        let e = read_patch(&path, "europe/elsewhere").unwrap_err();
        assert!(
            e.contains("for package `local/testland`") && e.contains("not loaded"),
            "{e}"
        );
        std::fs::write(
            &path,
            "ocwpatch = 1\npackage = \"x\"\n[[op]]\nop = \"lava\"\n",
        )
        .unwrap();
        assert!(read_patch(&path, "x").unwrap_err().contains("line"));
        assert!(read_patch(&d.join("none.ocwpatch"), "x").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_preview_reports_each_operation_and_the_changed_profiles() {
        let pkg = package();
        // The test package's origin is (47.0, 9.0); 30 m along edge 100.
        let (lat, lon) = pkg.projection().unproject([30.0, 0.0]);
        let mut p = WorldPatch::new("local/testland");
        p.ops = vec![
            Operation::Road(RoadEdit {
                edge: Some(EdgeId(100)),
                surface: Some("sett".into()),
                ..Default::default()
            }),
            Operation::RemoveRoad(RemoveRoad {
                edge: Some(EdgeId(555)),
                ..Default::default()
            }),
            Operation::HeightPoint(HeightPoint {
                id: "p".into(),
                at: Some([lat, lon]),
                raise_m: Some(3.0),
                radius_m: Some(20.0),
                ..Default::default()
            }),
        ];
        let v = Preview::of(&pkg, &p, "testland.ocwpatch");
        assert_eq!(v.status(0).tag(), "ok");
        assert_eq!(v.status(1).tag(), "MISSING");
        assert!(v.status(1).detail().contains("edge 555"));
        assert_eq!(v.status(2).tag(), "ok", "{:?}", v.status(2));
        assert_eq!(v.status(9), OpStatus::Pending);
        let prof = &v.profiles[&100];
        assert!((prof[6] - 403.0).abs() < 1e-6, "{prof:?}");
        assert_eq!(v.profiles.len(), 1, "only edge 100 is near the point");
        // A broken patch is not applied.
        p.ops.push(Operation::RemoveRoad(RemoveRoad::default()));
        let v = Preview::of(&pkg, &p, "x");
        assert!(v.report.is_none() && v.note.is_some());
        assert_eq!(v.status(0), OpStatus::Pending);
    }

    #[test]
    fn the_preview_waits_for_the_changes_to_settle() {
        let mut p = PatchPreview::default();
        assert!(p.due(1, 0.0), "the first one at once");
        p.state = Some(1);
        assert!(!p.due(1, 0.1));
        assert!(!p.due(2, 1.0), "changed: wait");
        assert!(!p.due(3, 1.2));
        assert!(p.due(3, 1.5));
    }
}
