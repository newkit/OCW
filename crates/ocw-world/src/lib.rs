//! World data: routes and road geometry.
//!
//! Phase 1 only provides a procedurally generated test route and terrain
//! conformed to it. The OSM road
//! graph, world packages and tiles follow in Phase 3 (spec §4).
//!
//! Coordinates are local metres, right-handed: `x` east, `y` up, `z` south
//! (matches Bevy's convention, so the renderer can use them directly).
//! This crate must build for `wasm32-unknown-unknown` (spec §3.5).

pub mod geo;
pub mod graph;
pub mod package;
mod route;
pub mod route_file;
pub mod surface;
mod terrain;

pub use package::{RoadStrip, Tile, TileKey, WorldPackage};
pub use route::{Route, RouteError, RoutePoint};
pub use route_file::{ElevationGrid, RouteFile};
pub use surface::Surface;
pub use terrain::{RoadProximity, Terrain, TerrainParams};
