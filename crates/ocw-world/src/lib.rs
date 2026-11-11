//! World data: routes, the road graph and world packages (spec §4).
//!
//! Coordinates are local metres, right-handed: `x` east, `y` up, `z` south
//! (matches Bevy's convention, so the renderer can use them directly).
//! Riders move along a [`Track`]: a fixed [`Route`] or a free-ride
//! [`RidePath`] through the [`RoadNetwork`] (spec §5.2).
//! This crate must build for `wasm32-unknown-unknown` (spec §3.5).

pub mod border;
pub mod buildings;
pub mod geo;
pub mod gpx;
pub mod graph;
pub mod graph_route;
pub mod landuse;
pub mod manifest;
pub mod matching;
pub mod network;
pub mod package;
pub mod path;
mod route;
pub mod route_file;
pub mod surface;
mod terrain;
pub mod track;

pub use graph_route::{GraphRoute, RouteBreak, RoutePlan, RouteStep};
pub use manifest::{Manifest, PackageIdentity, SignedManifest, TrustStore};
pub use network::{Exit, Junction, RoadNetwork, TurnClass};
pub use package::{RoadStrip, Tile, TileKey, WorldPackage};
pub use path::{RidePath, RouteProgress, RouteState, Segment, TurnReason};
pub use route::{Route, RouteError, RoutePoint};
pub use route_file::{ElevationGrid, RouteFile};
pub use surface::Surface;
pub use terrain::{RoadProximity, Terrain, TerrainParams};
pub use track::Track;
