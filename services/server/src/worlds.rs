//! The worlds a headless server can switch to (spec §3.2.1, World →
//! Change world in a shared session): its catalog is what
//! [`ocw_worldstore::discover`] finds – the cache and the packages next to
//! `--world` – and a [`crate::WorldSwitch`] names one of them by id,
//! `id@version`, name or file name. A client never makes the server load
//! a path of its choosing.

use std::sync::Arc;

use ocw_protocol::v1;
use ocw_world::RoadNetwork;
use ocw_worldstore::discover::WorldEntry;
use ocw_worldstore::{LoadOptions, load_world};

use crate::Course;

/// The catalog entry a request means: an exact id, `id@version` or spec
/// first, then the name or file name (ignoring case). Broken or blocked
/// packages are refused with their reason; parts of a world count too.
pub fn resolve<'a>(entries: &'a [WorldEntry], wanted: &str) -> Result<&'a WorldEntry, String> {
    let wanted = wanted.trim();
    let all: Vec<&WorldEntry> = entries.iter().flat_map(WorldEntry::members).collect();
    let stem = |e: &WorldEntry| {
        e.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_lowercase())
    };
    let exact = all.iter().find(|e| {
        e.id == wanted || e.spec == wanted || format!("{}@{}", e.id, e.version) == wanted
    });
    let w = wanted.to_lowercase();
    let loose = || {
        all.iter().find(|e| {
            e.name.to_lowercase() == w
                || stem(e).as_deref() == Some(w.as_str())
                || stem(e).as_deref() == Some(w.trim_end_matches(".ocwpkg"))
        })
    };
    let Some(e) = exact.or_else(loose) else {
        let names: Vec<String> = all
            .iter()
            .filter(|e| e.is_rideable())
            .map(|e| format!("{} ({})", e.id, e.name))
            .collect();
        return Err(format!(
            "no world {wanted:?} (known: {})",
            if names.is_empty() {
                "none".into()
            } else {
                names.join(", ")
            }
        ));
    };
    match &e.blocked {
        Some(why) => Err(format!("{}: {why}", e.name)),
        None => Ok(e),
    }
}

/// Loads `spec` (verified, with its installed neighbours) and builds the
/// course: the route `route` of the package, else a free ride from a start
/// chosen by `seed`; with the [`v1::WorldChange`] that tells the clients.
pub fn load_course(
    spec: &str,
    route: Option<&str>,
    opts: &LoadOptions,
    neighbours: bool,
    seed: u64,
) -> Result<(Course, v1::WorldChange), String> {
    let world = load_world(spec, opts).map_err(|e| e.to_string())?;
    tracing::info!(world = %world.describe(), "world package");
    let hood = ocw_worldstore::neighbourhood(&world, spec, opts, neighbours);
    for line in &hood.log {
        tracing::info!("{line}");
    }
    let graph = hood
        .set
        .graph()
        .cloned()
        .ok_or("no road graph (rebuild it with world-builder package)")?;
    let net = Arc::new(RoadNetwork::new(graph).with_package_name(hood.set.name()));
    let change = v1::WorldChange {
        package_id: world.identity.id.clone(),
        version: world.identity.version,
        name: world
            .manifest
            .as_ref()
            .map_or_else(|| world.package.name.clone(), |m| m.name.clone()),
        world: Some(ocw_protocol::world_id(&net)),
        route: route.unwrap_or_default().to_owned(),
        content_sha256: world.identity.content_hash.0.to_vec(),
        by: String::new(),
    };
    let pkg = &world.package;
    if let Some(name) = route {
        let stored = pkg.route(name).ok_or_else(|| {
            let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
            format!("no route {name:?} (the package has: {})", names.join(", "))
        })?;
        let plan = stored
            .resolve(&net)
            .map_err(|e| format!("route {name:?} does not fit the road graph: {e}"))?;
        return Ok((
            Course::RouteRide {
                net,
                plan: Arc::new(plan),
            },
            change,
        ));
    }
    let (edge, offset_m, forward) = net.random_start(seed).ok_or("empty road graph")?;
    Ok((
        Course::FreeRide {
            net,
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
        },
        change,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_worldstore::discover::{Source, Status};

    fn entry(id: &str, name: &str, file: &str, blocked: Option<&str>) -> WorldEntry {
        WorldEntry {
            spec: format!("/srv/{file}"),
            id: id.into(),
            version: 3,
            name: name.into(),
            path: Some(format!("/srv/{file}").into()),
            source: Source::Folder,
            size_bytes: 1,
            created_unix_s: None,
            status: if blocked.is_some() {
                Status::Broken(blocked.unwrap().into())
            } else {
                Status::Unsigned
            },
            blocked: blocked.map(Into::into),
            summary: None,
            attributions: Vec::new(),
            parts: Vec::new(),
            also: Vec::new(),
        }
    }

    #[test]
    fn requests_name_a_world_of_the_catalog() {
        let mut li = entry(
            "europe/liechtenstein",
            "Liechtenstein",
            "liechtenstein.ocwpkg",
            None,
        );
        li.parts.push(entry(
            "europe/graubuenden",
            "Graubuenden",
            "gr.ocwpkg",
            None,
        ));
        let entries = vec![
            li,
            entry(
                "europe/blackforest",
                "Black Forest",
                "blackforest.ocwpkg",
                None,
            ),
            entry(
                "local/zip",
                "zip",
                "zip.ocwpkg",
                Some("not an OCW world package"),
            ),
        ];
        let id = |w: &str| resolve(&entries, w).map(|e| e.id.clone());
        assert_eq!(id("europe/blackforest").unwrap(), "europe/blackforest");
        assert_eq!(id("europe/blackforest@3").unwrap(), "europe/blackforest");
        assert_eq!(id("black forest").unwrap(), "europe/blackforest");
        assert_eq!(id("blackforest.ocwpkg").unwrap(), "europe/blackforest");
        assert_eq!(id(" Liechtenstein ").unwrap(), "europe/liechtenstein");
        assert_eq!(id("graubuenden").unwrap(), "europe/graubuenden", "a part");
        let err = id("zip").unwrap_err();
        assert!(err.contains("not an OCW world package"), "{err}");
        // Paths the client names are not loaded unless listed.
        let err = id("/etc/passwd").unwrap_err();
        assert!(
            err.contains("no world \"/etc/passwd\"") && err.contains("europe/blackforest"),
            "{err}"
        );
    }
}
