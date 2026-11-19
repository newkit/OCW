//! Route files in and out of the Routes workspace (spec §14 Phase 16, 16D,
//! 16F E2): GPX, TCX, GeoJSON and FIT tracks are read into a [`Track`]
//! (then matched onto the graph); a route is written as GPX 1.1 (elevation
//! from our road profile, the via-points as waypoints), TCX course, FIT
//! course, GeoJSON or `ocw-graph-route` – all with "© OpenStreetMap
//! contributors" (and the planner's credit) where the format has room.

use ocw_world::formats::{
    ExportTrack, OSM_CREDIT, TrackFormat, read_geojson, read_tcx, write_geojson, write_gpx,
    write_tcx,
};
use ocw_world::gpx::{Gpx, GpxPoint};
use ocw_world::network::RoadNetwork;
use ocw_world::plan::steps_points;

use super::{RouteDoc, Track};

/// The speed course times assume (m/s; 25 km/h).
pub const COURSE_SPEED_MPS: f64 = 25.0 / 3.6;

/// Reads a track file (`name` gives the format by its extension, else
/// the content does). Graph routes are not tracks: see
/// [`ocw_world::GraphRoute::parse`].
pub fn read_track(name: &str, bytes: &[u8]) -> Result<(Track, TrackFormat), String> {
    let (gpx, format) = read_parts(name, bytes)?;
    Ok((Track::of_gpx(&gpx, format.name()), format))
}

/// Reads a track file with its parts (GPX track segments and routes, TCX
/// tracks, GeoJSON lines; [`Gpx::part_ranges`]), for choosing one of
/// them ([`super::track_parts`]).
pub fn read_parts(name: &str, bytes: &[u8]) -> Result<(Gpx, TrackFormat), String> {
    let format = TrackFormat::of_name(name)
        .or_else(|| TrackFormat::sniff(bytes))
        .ok_or_else(|| format!("{name}: not a GPX, TCX, GeoJSON or FIT file"))?;
    let text = || {
        std::str::from_utf8(bytes)
            .map(|t| t.trim_start_matches('\u{feff}'))
            .map_err(|_| format!("{name}: not a text file"))
    };
    let gpx = match format {
        TrackFormat::Gpx => Gpx::parse(text()?)?,
        TrackFormat::Tcx => read_tcx(text()?)?,
        TrackFormat::GeoJson => read_geojson(text()?)?,
        TrackFormat::Fit => {
            let a = ocw_fit::decode(bytes).map_err(|e| format!("{name}: {e}"))?;
            let points: Vec<GpxPoint> = a
                .records
                .iter()
                .filter_map(|r| {
                    let (lat, lon) = r.position?;
                    Some(GpxPoint {
                        lat,
                        lon,
                        ele: r.altitude_m,
                    })
                })
                .collect();
            if points.len() < 2 {
                return Err(format!("{name}: the FIT file has no positions"));
            }
            Gpx {
                name: None,
                points,
                segments: Vec::new(),
            }
        }
        TrackFormat::GraphRoute => {
            return Err(format!(
                "{name} is a graph route: it is opened, not matched"
            ));
        }
    };
    Ok((gpx, format))
}

/// The route's points with heights from the road profile (lat, lon, ele),
/// every `step_m` metres.
pub fn route_points(
    doc: &RouteDoc,
    net: &RoadNetwork,
    step_m: f64,
) -> Result<Vec<GpxPoint>, String> {
    let plan = doc.plan(net)?;
    let proj = doc.projection();
    Ok(steps_points(net, &plan.steps, step_m)
        .into_iter()
        .map(|p| {
            let (lat, lon) = proj.unproject([p[0], p[2]]);
            GpxPoint {
                lat,
                lon,
                ele: Some(p[1]),
            }
        })
        .collect())
}

/// The credit line of an export: OSM and the planners.
pub fn credit(doc: &RouteDoc) -> String {
    let mut c = OSM_CREDIT.to_string();
    for p in &doc.geometry.credits {
        let p = p.replace(", © OpenStreetMap contributors", "");
        c.push_str("; ");
        c.push_str(&p);
    }
    c
}

/// Writes the route as `format`.
pub fn export(doc: &RouteDoc, net: &RoadNetwork, format: TrackFormat) -> Result<Vec<u8>, String> {
    if format == TrackFormat::GraphRoute {
        let gr = doc.graph_route(net)?;
        return Ok(gr
            .to_text(Some((&doc.package.name, net.fingerprint())))
            .into_bytes());
    }
    let points = route_points(doc, net, 10.0)?;
    let proj = doc.projection();
    let waypoints: Vec<(String, f64, f64)> = doc
        .geometry
        .vias
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (lat, lon) = proj.unproject(v.at);
            (format!("Via {}", i + 1), lat, lon)
        })
        .collect();
    let credit = credit(doc);
    let t = ExportTrack {
        name: &doc.name,
        description: &doc.description,
        points: &points,
        waypoints: &waypoints,
        credit: &credit,
    };
    Ok(match format {
        TrackFormat::Gpx => write_gpx(&t).into_bytes(),
        TrackFormat::Tcx => write_tcx(&t, COURSE_SPEED_MPS).into_bytes(),
        TrackFormat::GeoJson => write_geojson(&t).into_bytes(),
        TrackFormat::Fit => {
            let d = ocw_world::formats::distances(&points);
            ocw_fit::encode_course(&ocw_fit::Course {
                name: doc.name.clone(),
                points: points
                    .iter()
                    .zip(d)
                    .map(|(p, distance_m)| ocw_fit::CoursePoint {
                        lat: p.lat,
                        lon: p.lon,
                        altitude_m: p.ele,
                        distance_m,
                    })
                    .collect(),
                start_unix_s: ocw_world::formats::COURSE_EPOCH_UNIX_S,
                speed_mps: COURSE_SPEED_MPS,
            })
        }
        TrackFormat::GraphRoute => unreachable!("handled above"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{Editor, RouteOp, RoutePackage, import_track};
    use ocw_world::matching::MatchParams;
    use ocw_world::plan::{EdgeCosts, Profile, test_grid};

    fn route(net: &RoadNetwork) -> RouteDoc {
        let costs = EdgeCosts::new(net, Profile::RoadBike, None);
        let ed = Editor { net, costs: &costs };
        let mut d = RouteDoc::new(
            RoutePackage {
                id: "local/grid".into(),
                name: "Grid".into(),
                fingerprint: net.fingerprint(),
                origin: (47.14, 9.52),
            },
            "Ada's export",
        );
        let RouteOp::Set(s) = ed
            .plan_through(&[[0.0, 200.0], [400.0, 200.0], [400.0, 600.0], [100.0, 600.0]])
            .unwrap()
        else {
            panic!()
        };
        d.geometry = s.geometry;
        d
    }

    fn edge_ids(doc: &RouteDoc, net: &RoadNetwork) -> Vec<u64> {
        let plan = doc.plan(net).unwrap();
        plan.steps
            .iter()
            .filter_map(|s| match *s {
                ocw_world::Segment::Edge { edge, .. } => Some(net.edge(edge).id.0),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_export_imports_again_onto_the_same_edges() {
        let net = test_grid();
        let doc = route(&net);
        let ids = edge_ids(&doc, &net);
        for format in [
            TrackFormat::Gpx,
            TrackFormat::Tcx,
            TrackFormat::GeoJson,
            TrackFormat::Fit,
        ] {
            let bytes = export(&doc, &net, format).unwrap();
            let name = format!("route.{}", format.extension());
            let (track, f) = read_track(&name, &bytes).unwrap();
            assert_eq!(f, format);
            // Also by content alone.
            assert_eq!(read_track("download", &bytes).unwrap().1, format);
            let r = import_track(&net, &doc.package, &track, &MatchParams::default()).unwrap();
            let back = RouteDoc {
                geometry: r.geometry,
                ..doc.clone()
            };
            assert_eq!(edge_ids(&back, &net), ids, "{format:?}");
            assert!(r.problems.is_empty(), "{format:?}: {:?}", r.problems);
            assert!(
                (back.plan(&net).unwrap().length_m() - doc.plan(&net).unwrap().length_m()).abs()
                    < 2.0
            );
        }
    }

    #[test]
    fn the_graph_route_export_is_the_same_route() {
        let net = test_grid();
        let doc = route(&net);
        let text = String::from_utf8(export(&doc, &net, TrackFormat::GraphRoute).unwrap()).unwrap();
        assert!(text.starts_with("ocw-graph-route 1\nname Ada's export\npackage Grid "));
        let (gr, pkg) = ocw_world::GraphRoute::parse(&text).unwrap();
        assert_eq!(pkg.unwrap().1, net.fingerprint());
        let g = crate::route::geometry_of_graph_route(&net, &gr).unwrap();
        let back = RouteDoc {
            geometry: g,
            ..doc.clone()
        };
        assert_eq!(edge_ids(&back, &net), edge_ids(&doc, &net));
        assert!(read_track("x.ocwroute", text.as_bytes()).is_err());
    }

    #[test]
    fn exports_carry_the_credit_and_the_road_heights() {
        let net = test_grid();
        let mut doc = route(&net);
        doc.geometry.credits = vec!["Routing: BRouter, © OpenStreetMap contributors".into()];
        let gpx = String::from_utf8(export(&doc, &net, TrackFormat::Gpx).unwrap()).unwrap();
        assert!(gpx.contains("© OpenStreetMap contributors; Routing: BRouter"));
        assert!(gpx.contains("<ele>0.0</ele>"));
        assert!(gpx.contains("<name>Via 4</name>"));
        let geo = String::from_utf8(export(&doc, &net, TrackFormat::GeoJson).unwrap()).unwrap();
        assert!(geo.contains("Routing: BRouter"));
    }

    #[test]
    fn unreadable_files_say_why() {
        assert!(
            read_track("x.kml", b"<kml/>")
                .unwrap_err()
                .contains("not a GPX")
        );
        assert!(read_track("x.fit", b"nope").is_err());
        assert!(read_track("x.gpx", &[0xff, 0xfe, 0]).is_err());
    }
}
