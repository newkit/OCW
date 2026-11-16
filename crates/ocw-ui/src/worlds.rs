//! A loaded world package for the renderer (native apps: the desktop app
//! and OCW Studio; the browser streams its package instead, B2).

use std::sync::Arc;

use ocw_render::package::{PackageLoader, PackageWorld};
use ocw_worldstore::{LoadOptions, Neighbourhood};

/// The render resource: the world set of the home package and its
/// neighbours, with a loader that reads (and verifies) a neighbour's tiles
/// again when the camera comes near it (none for a single package, or
/// without load options).
pub fn package_world(hood: Neighbourhood, opts: Option<LoadOptions>) -> PackageWorld {
    let set = Arc::new(hood.set.clone());
    let loader: Option<PackageLoader> = match opts {
        Some(opts) if set.len() > 1 => Some(Arc::new(move |i| hood.reload(i, &opts))),
        _ => None,
    };
    PackageWorld { set, loader }
}
