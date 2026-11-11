//! OSM and elevation import for world packages (spec §4). Used by the
//! `world-builder` tool on the build machines, never by the client.
//!
//! - [`osm`]: OSM data in memory, PBF reading
//! - [`filter`]: rideable ways and road classes
//! - [`projection`]: local metric projection
//! - [`build`]: ways → road graph with stable edge IDs, cleanup, dead ends
//! - [`svg`]: map of a road graph for visual checks

pub mod build;
pub mod filter;
pub mod osm;
pub mod projection;
pub mod svg;
