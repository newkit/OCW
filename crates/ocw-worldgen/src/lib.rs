//! OSM and elevation import for world packages (spec §4). Used by the
//! `world-builder` tool on the build machines, never by the client.
//!
//! - [`osm`]: OSM data in memory, PBF reading
//! - [`areas`]: OSM areas (closed ways, multipolygons) and waterways
//! - [`landuse`]: land-use classification, splat-map rasterisation, water surfaces
//! - [`buildings`]: building footprints, heights, roofs and placement
//! - [`osm`]: OSM data in memory, PBF reading, OSM XML for fixtures
//! - [`diff`]: which stable edge IDs survive a rebuild (route re-matching)
//! - [`filter`]: rideable ways and road classes
//! - [`projection`]: local metric projection
//! - [`build`]: ways → road graph with stable edge IDs, cleanup, dead ends
//! - [`dem`]: SRTM elevation tiles
//! - [`chains`]: through-roads across the OSM way splits
//! - [`profile`]: road elevation profiles (sampling, smoothing, limits)
//! - [`package`]: road-conformed terrain and 1 km tiles → world package
//! - [`route`]: routes through the graph with their elevation
//! - [`svg`]: map of a road graph for visual checks

pub mod areas;
pub mod build;
pub mod buildings;
pub mod chains;
pub mod dem;
pub mod diff;
pub mod filter;
pub mod landuse;
pub mod osm;
pub mod package;
pub mod profile;
pub mod projection;
pub mod route;
pub mod svg;
