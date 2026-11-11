//! World data: routes and road geometry.
//!
//! Phase 1 only provides a procedurally generated test route and terrain
//! conformed to it. The OSM road
//! graph, world packages and tiles follow in Phase 3 (spec §4).
//!
//! Coordinates are local metres, right-handed: `x` east, `y` up, `z` south
//! (matches Bevy's convention, so the renderer can use them directly).
//! This crate must build for `wasm32-unknown-unknown` (spec §3.5).

mod route;
mod terrain;

pub use route::{Route, RouteError, RoutePoint};
pub use terrain::{RoadProximity, Terrain, TerrainParams};
