//! World data: routes, the road graph and world packages (spec §4).
//!
//! Coordinates are local metres, right-handed: `x` east, `y` up, `z` south
//! (matches Bevy's convention, so the renderer can use them directly).
//! Riders move along a [`Track`]: a fixed [`Route`] or a free-ride
//! [`RidePath`] through the [`RoadNetwork`] (spec §5.2).
//! This crate must build for `wasm32-unknown-unknown` (spec §3.5).

pub mod aero;
pub mod bike_signs;
pub mod border;
pub mod bridges;
pub mod buildings;
pub mod flora;
pub mod forest;
pub mod formats;
pub mod geo;
pub mod gpx;
pub mod gradient;
pub mod graph;
pub mod graph_route;
pub mod ground;
pub mod inspect;
pub mod junction;
pub mod lamps;
pub mod landmarks;
pub mod landuse;
pub mod leaves;
pub mod look;
pub mod manifest;
pub mod matching;
pub mod network;
pub mod obstacles;
pub mod ortho;
pub mod package;
pub mod patch;
pub mod path;
pub mod photo;
pub mod photo_drone;
pub mod photo_share;
pub mod places;
pub mod plan;
pub mod plantings;
pub mod recipe;
pub mod region;
pub mod road_condition;
pub mod road_dress;
pub mod road_feel;
pub mod road_surface;
pub mod road_wear;
pub mod roof_skeleton;
mod route;
pub mod route_file;
pub mod route_fit;
pub mod summary;
pub mod surface;
mod terrain;
pub mod track;
pub mod traffic;
pub mod trees;
pub mod tunnels;
pub mod vegetation;
pub mod water_lines;
pub mod woods;
pub mod world_set;

pub use gradient::{AVERAGE_AHEAD_M, ElevationProfile, GradeBand, ProfileSpec, average_grade};
pub use graph_route::{GraphRoute, RouteBreak, RoutePlan, RouteStep};
pub use lamps::RoadLit;
pub use manifest::{Manifest, PackageIdentity, SignedManifest, TrustStore};
pub use network::{Exit, Junction, RoadNetwork, TurnClass};
pub use package::{RoadStrip, Tile, TileKey, WorldPackage};
pub use path::{RidePath, RouteProgress, RouteState, Segment, TurnReason};
pub use road_condition::RoadCondition;
pub use road_dress::RoadDress;
pub use road_surface::RoadSurface;
pub use route::{Route, RouteError, RoutePoint};
pub use route_file::{ElevationGrid, RouteFile};
pub use surface::Surface;
pub use terrain::{RoadProximity, Terrain, TerrainParams};
pub use track::Track;
pub use world_set::{Frame, WorldSet};
