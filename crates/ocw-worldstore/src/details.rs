//! What a loaded world is, for people (the World menu's *About this
//! world*, spec §3.2.1; OCW Studio's World workspace, spec §14 Phase 16):
//! id and version, region, trust, area, elevation, time zone, credits and
//! the joined neighbours. Shared by the desktop app and the studio.

use ocw_world::manifest::{Attribution, TrustStatus};

use crate::LoadedWorld;

/// What the World pages know about a loaded world (built when the
/// package is loaded).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldDetails {
    /// Manifest id and version (`local/<name>`, 0 without a manifest).
    pub id: String,
    pub version: u64,
    /// The region's name for people.
    pub region: String,
    pub created_unix_s: Option<i64>,
    /// "verified", "unofficial" (a trusted user key, spec §4.7), "unsigned"
    /// or "untrusted".
    pub trust: &'static str,
    /// The name an unofficial package's key claims (not verified).
    pub signer: Option<String>,
    /// The covered area (km²), the parts included.
    pub area_km2: Option<f64>,
    /// Lowest and highest terrain (m), the parts included.
    pub elevation_m: Option<(f32, f32)>,
    /// The package's own time zone (section `TZID`).
    pub time_zone: Option<String>,
    /// The credits of the package and its parts (spec §13), each once.
    pub attributions: Vec<Attribution>,
    /// The joined neighbouring packages (names).
    pub parts: Vec<String>,
    pub size_bytes: u64,
}

impl WorldDetails {
    /// The details of a loaded package and its joined neighbours.
    pub fn of(home: &LoadedWorld, parts: &[LoadedWorld]) -> Self {
        let mut attributions: Vec<Attribution> = Vec::new();
        let mut area = 0.0;
        let mut elevation: Option<(f32, f32)> = None;
        for w in std::iter::once(home).chain(parts) {
            let credits = match &w.manifest {
                Some(m) if !m.attribution.is_empty() => m.attribution.clone(),
                _ => vec![Attribution::openstreetmap()],
            };
            for a in credits {
                if !attributions.contains(&a) {
                    attributions.push(a);
                }
            }
            let p = &w.package;
            area += match &p.region {
                Some(r) => r.area_km2(),
                None => p.tiles.len() as f64 * f64::from(p.tile_size_m).powi(2) / 1e6,
            };
            if let Some((lo, hi)) = p.height_range() {
                elevation = Some(elevation.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        }
        let trust = match &home.trust {
            TrustStatus::Trusted(s) if s.root_user => "unofficial",
            TrustStatus::Trusted(_) => "verified",
            TrustStatus::Untrusted(_) if home.manifest.is_none() => "unsigned",
            TrustStatus::Untrusted(why) if why.contains("not signed") => "unsigned",
            TrustStatus::Untrusted(_) => "untrusted",
        };
        Self {
            id: home.identity.id.clone(),
            version: home.identity.version,
            region: home
                .manifest
                .as_ref()
                .map_or_else(|| home.package.name.clone(), |m| m.name.clone()),
            created_unix_s: home.manifest.as_ref().map(|m| m.created_unix_s),
            trust,
            signer: match &home.trust {
                TrustStatus::Trusted(s) if s.root_user => Some(s.root_name.clone()),
                _ => None,
            },
            area_km2: (area > 0.0).then_some(area),
            elevation_m: elevation,
            time_zone: home.package.time_zone.clone(),
            attributions,
            parts: parts
                .iter()
                .map(|w| {
                    w.manifest
                        .as_ref()
                        .map_or_else(|| w.package.name.clone(), |m| m.name.clone())
                })
                .collect(),
            size_bytes: std::fs::metadata(&home.path).map_or(0, |m| m.len()),
        }
    }
}
