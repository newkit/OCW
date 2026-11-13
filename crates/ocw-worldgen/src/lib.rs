//! OSM and elevation import for world packages (spec §4). Used by the
//! `world-builder` tool on the build machines, never by the client.
//!
//! - [`osm`]: OSM data in memory, PBF reading
//! - [`areas`]: OSM areas (closed ways, multipolygons) and waterways
//! - [`landuse`]: land-use classification, splat-map rasterisation, water surfaces
//! - [`buildings`]: building footprints, heights, roofs and placement
//! - [`imagery`]: orthophotos and seasonal Sentinel-2 composites at local metres (spec §14 Phase 8 T3)
//! - [`calibrate`]: region calibration – palettes, roof colours, seasonal tints (Phase 8 T3)
//! - [`osm`]: OSM data in memory, PBF reading, OSM XML for fixtures
//! - [`diff`]: which stable edge IDs survive a rebuild (route re-matching)
//! - [`filter`]: rideable ways and road classes
//! - [`projection`]: local metric projection
//! - [`build`]: ways → road graph with stable edge IDs, cleanup, dead ends
//! - [`dem`]: elevation sources (SRTM tiles, DTM grid layers, feathering)
//! - [`dtm`]: high-resolution DTM grids (GeoTIFF, XYZ, resampling)
//! - [`crs`]: UTM and Swiss LV95 projections of the DTM grids
//! - [`chains`]: through-roads across the OSM way splits
//! - [`profile`]: road elevation profiles (sampling, smoothing, limits)
//! - [`package`]: road-conformed terrain and 1 km tiles → world package
//! - [`par`]: parallel map with dynamic scheduling, results in order
//! - [`route`]: routes through the graph with their elevation
//! - [`svg`]: map of a road graph for visual checks
//! - [`bridges`]: bridge detection, planned decks and structures (spec §4.8.6)
//! - [`clean`]: world cleaning passes – polylines, near misses, junction mouths, vertical curves, terrain settling (spec §4.8)
//! - [`lint`]: world lint – what looks physically wrong in a package (spec §4.8)
//! - [`spatial`]: bucket index over road segments
//! - [`stages`]: stage profile of a world build (time, CPU, peak memory)
//! - [`water`]: river and lake levels (spec §4.8)

pub mod areas;
pub mod bridges;
pub mod build;
pub mod buildings;
pub mod calibrate;
pub mod chains;
pub mod clean;
pub mod crs;
pub mod dem;
pub mod diff;
pub mod dress;
pub mod dtm;
pub mod filter;
pub mod imagery;
pub mod landuse;
pub mod lint;
pub mod osm;
pub mod package;
pub mod par;
pub mod profile;
pub mod projection;
pub mod route;
pub mod spatial;
pub mod stages;
pub mod svg;
pub mod water;
