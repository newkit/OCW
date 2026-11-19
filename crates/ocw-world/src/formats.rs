//! Track files beside GPX (spec §14 Phase 16, 16D, 16F E2): reading TCX
//! (courses and activities) and GeoJSON line strings – the formats planners
//! and head units export – into the same [`Gpx`] points the matcher takes,
//! and writing a route as GPX 1.1 (with the OSM credit and the via-points
//! as waypoints), TCX course and GeoJSON. FIT is in `ocw-fit` (the studio
//! joins them in `ocw_edit::route::formats`).
//!
//! Hand-written like [`crate::gpx`]: pure, no XML library, wasm-safe.

use crate::gpx::{Gpx, GpxPoint, GpxSegment, haversine_m};

/// The track file formats of the Routes workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackFormat {
    Gpx,
    Tcx,
    GeoJson,
    Fit,
    /// `ocw-graph-route 1` (stable edge ids on the package's graph).
    GraphRoute,
}

impl TrackFormat {
    pub const ALL: [TrackFormat; 5] = [
        TrackFormat::Gpx,
        TrackFormat::Tcx,
        TrackFormat::GeoJson,
        TrackFormat::Fit,
        TrackFormat::GraphRoute,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Gpx => "GPX",
            Self::Tcx => "TCX",
            Self::GeoJson => "GeoJSON",
            Self::Fit => "FIT",
            Self::GraphRoute => "graph route",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Gpx => "gpx",
            Self::Tcx => "tcx",
            Self::GeoJson => "geojson",
            Self::Fit => "fit",
            Self::GraphRoute => "ocwroute",
        }
    }

    /// The format of a file name (by its extension).
    pub fn of_name(name: &str) -> Option<Self> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "gpx" => Self::Gpx,
            "tcx" => Self::Tcx,
            "geojson" | "json" => Self::GeoJson,
            "fit" => Self::Fit,
            "ocwroute" | "route" => Self::GraphRoute,
            _ => return None,
        })
    }

    /// The format of a file's content (when the name does not tell).
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= 12 && &bytes[8..12] == b".FIT" {
            return Some(Self::Fit);
        }
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]);
        let t = head.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with("ocw-graph-route") {
            Some(Self::GraphRoute)
        } else if t.starts_with('{') {
            Some(Self::GeoJson)
        } else if head.contains("<TrainingCenterDatabase")
            || head.contains(":TrainingCenterDatabase")
        {
            Some(Self::Tcx)
        } else if head.contains("<gpx") || head.contains(":gpx") {
            Some(Self::Gpx)
        } else {
            None
        }
    }
}

/// One XML event of the small scanner.
#[derive(Debug, Clone, PartialEq)]
enum Ev {
    Open(String),
    Close(String),
    Text(String),
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Elements and text of an XML document (comments, PIs, doctype skipped;
/// CDATA as text; attributes ignored).
fn events(text: &str) -> Result<Vec<Ev>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        let t = &rest[..i];
        if !t.trim().is_empty() {
            out.push(Ev::Text(unescape(t.trim())));
        }
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("<!--") {
            let e = r.find("-->").ok_or("an unclosed comment")?;
            rest = &r[e + 3..];
            continue;
        }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>").ok_or("an unclosed CDATA section")?;
            out.push(Ev::Text(r[..e].trim().to_string()));
            rest = &r[e + 3..];
            continue;
        }
        let e = rest.find('>').ok_or("an unclosed tag")?;
        let tag = &rest[1..e];
        rest = &rest[e + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            out.push(Ev::Close(local(name.trim())));
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_end_matches('/')
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        out.push(Ev::Open(local(name)));
        if self_closing {
            out.push(Ev::Close(local(name)));
        }
    }
    Ok(out)
}

/// Reads a TCX file: the track points of all courses' and activities'
/// tracks (points without a position skipped), the first course's name or
/// activity's id.
pub fn read_tcx(text: &str) -> Result<Gpx, String> {
    let ev = events(text)?;
    if !ev
        .iter()
        .any(|e| matches!(e, Ev::Open(n) if n == "TrainingCenterDatabase"))
    {
        return Err("not a TCX file (no <TrainingCenterDatabase>)".into());
    }
    let mut gpx = Gpx::default();
    let mut path: Vec<String> = Vec::new();
    let (mut lat, mut lon, mut ele): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
    for e in ev {
        match e {
            Ev::Open(n) => {
                if n == "Trackpoint" {
                    (lat, lon, ele) = (None, None, None);
                } else if n == "Track" {
                    gpx.segments.push(GpxSegment {
                        first: gpx.points.len(),
                        name: None,
                    });
                }
                path.push(n);
            }
            Ev::Close(n) => {
                if n == "Trackpoint"
                    && let (Some(a), Some(o)) = (lat, lon)
                {
                    if !(-90.0..=90.0).contains(&a) || !(-180.0..=180.0).contains(&o) {
                        return Err(format!("a track point out of range: {a}, {o}"));
                    }
                    gpx.points.push(GpxPoint {
                        lat: a,
                        lon: o,
                        ele,
                    });
                }
                if let Some(k) = path.iter().rposition(|p| *p == n) {
                    path.truncate(k);
                }
            }
            Ev::Text(t) => {
                let last = path.last().map(String::as_str);
                let parent = path.len().checked_sub(2).map(|k| path[k].as_str());
                match (parent, last) {
                    (_, Some("LatitudeDegrees")) => lat = t.parse().ok(),
                    (_, Some("LongitudeDegrees")) => lon = t.parse().ok(),
                    (Some("Trackpoint"), Some("AltitudeMeters")) => ele = t.parse().ok(),
                    (Some("Course"), Some("Name")) | (Some("Activity"), Some("Id"))
                        if gpx.name.is_none() =>
                    {
                        gpx.name = Some(t);
                    }
                    _ => {}
                }
            }
        }
    }
    if gpx.points.is_empty() {
        return Err("the TCX file has no track points with a position".into());
    }
    Ok(gpx)
}

/// Reads GeoJSON: the line strings of a `LineString`, `MultiLineString`,
/// `Feature` or `FeatureCollection` (all lines in order, joined), the
/// first feature's `name`.
pub fn read_geojson(text: &str) -> Result<Gpx, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not GeoJSON: {e}"))?;
    let mut gpx = Gpx::default();
    fn line(gpx: &mut Gpx, coords: &serde_json::Value) -> Result<(), String> {
        let list = coords.as_array().ok_or("coordinates are not a list")?;
        gpx.segments.push(GpxSegment {
            first: gpx.points.len(),
            name: None,
        });
        for c in list {
            let c = c.as_array().ok_or("a position is not a list")?;
            let n = |i: usize| c.get(i).and_then(serde_json::Value::as_f64);
            let (Some(lon), Some(lat)) = (n(0), n(1)) else {
                return Err("a position without longitude and latitude".into());
            };
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                return Err(format!("a position out of range: {lon}, {lat}"));
            }
            gpx.points.push(GpxPoint {
                lat,
                lon,
                ele: n(2),
            });
        }
        Ok(())
    }
    fn geometry(gpx: &mut Gpx, g: &serde_json::Value) -> Result<(), String> {
        match g.get("type").and_then(|t| t.as_str()) {
            Some("LineString") => line(gpx, g.get("coordinates").ok_or("no coordinates")?),
            Some("MultiLineString") => {
                for l in g
                    .get("coordinates")
                    .and_then(|c| c.as_array())
                    .ok_or("no coordinates")?
                {
                    line(gpx, l)?;
                }
                Ok(())
            }
            Some("GeometryCollection") => {
                for g in g
                    .get("geometries")
                    .and_then(|c| c.as_array())
                    .ok_or("no geometries")?
                {
                    geometry(gpx, g)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    fn feature(gpx: &mut Gpx, f: &serde_json::Value) -> Result<(), String> {
        let name = f
            .get("properties")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from);
        if gpx.name.is_none() {
            gpx.name = name.clone();
        }
        let before = gpx.segments.len();
        match f.get("geometry") {
            Some(g) if !g.is_null() => geometry(gpx, g)?,
            _ => {}
        }
        // The feature's lines carry its name.
        for s in &mut gpx.segments[before..] {
            s.name = name.clone();
        }
        Ok(())
    }
    match v.get("type").and_then(|t| t.as_str()) {
        Some("FeatureCollection") => {
            for f in v
                .get("features")
                .and_then(|f| f.as_array())
                .ok_or("no features")?
            {
                feature(&mut gpx, f)?;
            }
        }
        Some("Feature") => feature(&mut gpx, &v)?,
        Some(_) => geometry(&mut gpx, &v)?,
        None => return Err("not GeoJSON (no type)".into()),
    }
    if gpx.points.len() < 2 {
        return Err("the GeoJSON has no line with two or more positions".into());
    }
    Ok(gpx)
}

/// A route to write.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportTrack<'a> {
    pub name: &'a str,
    pub description: &'a str,
    /// The route's points with heights from our road profile.
    pub points: &'a [GpxPoint],
    /// Named waypoints (the via-points): name, lat, lon.
    pub waypoints: &'a [(String, f64, f64)],
    /// The credit: "© OpenStreetMap contributors" and the planner's.
    pub credit: &'a str,
}

/// "© OpenStreetMap contributors": exported tracks follow OSM roads (16D).
pub const OSM_CREDIT: &str = "© OpenStreetMap contributors";

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// GPX 1.1: metadata (name, description, the OSM copyright), the
/// via-points as waypoints, one track with elevation.
pub fn write_gpx(t: &ExportTrack) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"OCW Studio (Open Cycling World)\" xmlns=\"http://www.topografix.com/GPX/1/1\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://www.topografix.com/GPX/1/1 http://www.topografix.com/GPX/1/1/gpx.xsd\">\n  <metadata>\n",
    );
    s.push_str(&format!("    <name>{}</name>\n", xml(t.name)));
    if !t.description.is_empty() {
        s.push_str(&format!("    <desc>{}</desc>\n", xml(t.description)));
    }
    s.push_str(&format!(
        "    <copyright author=\"{}\"><license>https://opendatacommons.org/licenses/odbl/</license></copyright>\n  </metadata>\n",
        xml(t.credit)
    ));
    for (name, lat, lon) in t.waypoints {
        s.push_str(&format!(
            "  <wpt lat=\"{lat:.7}\" lon=\"{lon:.7}\"><name>{}</name><type>via</type></wpt>\n",
            xml(name)
        ));
    }
    s.push_str(&format!(
        "  <trk>\n    <name>{}</name>\n    <trkseg>\n",
        xml(t.name)
    ));
    for p in t.points {
        match p.ele {
            Some(e) => s.push_str(&format!(
                "      <trkpt lat=\"{:.7}\" lon=\"{:.7}\"><ele>{e:.1}</ele></trkpt>\n",
                p.lat, p.lon
            )),
            None => s.push_str(&format!(
                "      <trkpt lat=\"{:.7}\" lon=\"{:.7}\"/>\n",
                p.lat, p.lon
            )),
        }
    }
    s.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    s
}

/// The cumulative distance at each point (m).
pub fn distances(points: &[GpxPoint]) -> Vec<f64> {
    let mut d = 0.0;
    let mut out = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            let q = points[i - 1];
            d += haversine_m(q.lat, q.lon, p.lat, p.lon);
        }
        out.push(d);
    }
    out
}

/// `YYYY-MM-DDThh:mm:ssZ` of Unix seconds (proleptic Gregorian, UTC).
pub fn iso_time(unix_s: i64) -> String {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// The start time written into courses (the format needs one; head units
/// ignore it for a course).
pub const COURSE_EPOCH_UNIX_S: i64 = 1_767_225_600; // 2026-01-01

/// A TCX course: one lap, the track with distance and altitude, times at
/// a steady `speed_mps` from [`COURSE_EPOCH_UNIX_S`]. The course name is
/// cut to TCX's 15 characters.
pub fn write_tcx(t: &ExportTrack, speed_mps: f64) -> String {
    let d = distances(t.points);
    let total = d.last().copied().unwrap_or(0.0);
    let speed = speed_mps.max(0.5);
    let name: String = t.name.chars().take(15).collect();
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TrainingCenterDatabase xmlns=\"http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2\">\n  <Courses>\n    <Course>\n",
    );
    s.push_str(&format!("      <Name>{}</Name>\n", xml(&name)));
    let (first, last) = (t.points.first(), t.points.last());
    s.push_str(&format!(
        "      <Lap>\n        <TotalTimeSeconds>{:.1}</TotalTimeSeconds>\n        <DistanceMeters>{total:.1}</DistanceMeters>\n",
        total / speed
    ));
    for (tag, p) in [("BeginPosition", first), ("EndPosition", last)] {
        if let Some(p) = p {
            s.push_str(&format!(
                "        <{tag}><LatitudeDegrees>{:.7}</LatitudeDegrees><LongitudeDegrees>{:.7}</LongitudeDegrees></{tag}>\n",
                p.lat, p.lon
            ));
        }
    }
    s.push_str("        <Intensity>Active</Intensity>\n      </Lap>\n      <Track>\n");
    for (p, dist) in t.points.iter().zip(&d) {
        s.push_str(&format!(
            "        <Trackpoint>\n          <Time>{}</Time>\n          <Position><LatitudeDegrees>{:.7}</LatitudeDegrees><LongitudeDegrees>{:.7}</LongitudeDegrees></Position>\n",
            iso_time(COURSE_EPOCH_UNIX_S + (dist / speed).round() as i64),
            p.lat,
            p.lon
        ));
        if let Some(e) = p.ele {
            s.push_str(&format!(
                "          <AltitudeMeters>{e:.1}</AltitudeMeters>\n"
            ));
        }
        s.push_str(&format!(
            "          <DistanceMeters>{dist:.1}</DistanceMeters>\n        </Trackpoint>\n"
        ));
    }
    s.push_str(&format!(
        "      </Track>\n      <Notes>{}</Notes>\n    </Course>\n  </Courses>\n</TrainingCenterDatabase>\n",
        xml(t.credit)
    ));
    s
}

/// A GeoJSON feature collection: the route as a `LineString` of
/// `[lon, lat, ele]` (name, description, credit in its properties) and
/// the via-points as `Point`s.
pub fn write_geojson(t: &ExportTrack) -> String {
    let coords: Vec<serde_json::Value> = t
        .points
        .iter()
        .map(|p| {
            let mut c = vec![round7(p.lon), round7(p.lat)];
            if let Some(e) = p.ele {
                c.push(((e * 10.0).round() / 10.0).into());
            }
            serde_json::Value::Array(c)
        })
        .collect();
    let mut features = vec![serde_json::json!({
        "type": "Feature",
        "properties": {
            "name": t.name,
            "description": t.description,
            "attribution": t.credit,
            "creator": "OCW Studio (Open Cycling World)",
        },
        "geometry": { "type": "LineString", "coordinates": coords },
    })];
    for (name, lat, lon) in t.waypoints {
        features.push(serde_json::json!({
            "type": "Feature",
            "properties": { "name": name, "type": "via" },
            "geometry": { "type": "Point", "coordinates": [round7(*lon), round7(*lat)] },
        }));
    }
    let v = serde_json::json!({ "type": "FeatureCollection", "features": features });
    let mut s = serde_json::to_string_pretty(&v).expect("plain JSON");
    s.push('\n');
    s
}

fn round7(v: f64) -> serde_json::Value {
    ((v * 1e7).round() / 1e7).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Vec<GpxPoint> {
        (0..30)
            .map(|i| GpxPoint {
                lat: 47.14 + f64::from(i) * 1e-4,
                lon: 9.52 + f64::from(i) * 5e-5,
                ele: Some(450.0 + f64::from(i) * 0.5),
            })
            .collect()
    }

    fn export<'a>(points: &'a [GpxPoint], wp: &'a [(String, f64, f64)]) -> ExportTrack<'a> {
        ExportTrack {
            name: "Ada's <loop> & more",
            description: "made-up test route",
            points,
            waypoints: wp,
            credit: OSM_CREDIT,
        }
    }

    fn same(a: &[GpxPoint], b: &[GpxPoint], ele_tol: f64) {
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(b) {
            assert!((p.lat - q.lat).abs() < 1e-6 && (p.lon - q.lon).abs() < 1e-6);
            assert!((p.ele.unwrap() - q.ele.unwrap()).abs() <= ele_tol);
        }
    }

    #[test]
    fn gpx_written_reads_back_with_its_credit_and_waypoints() {
        let pts = track();
        let wp = vec![("Via 1".to_string(), 47.14, 9.52)];
        let text = write_gpx(&export(&pts, &wp));
        assert!(text.contains("<copyright author=\"© OpenStreetMap contributors\">"));
        assert!(text.contains("<wpt lat=\"47.1400000\" lon=\"9.5200000\"><name>Via 1</name>"));
        let back = Gpx::parse(&text).unwrap();
        assert_eq!(back.name.as_deref(), Some("Ada's <loop> & more"));
        // Waypoints are not track points.
        same(&pts, &back.points, 0.05);
        assert_eq!(TrackFormat::sniff(text.as_bytes()), Some(TrackFormat::Gpx));
    }

    #[test]
    fn tcx_written_reads_back() {
        let pts = track();
        let text = write_tcx(&export(&pts, &[]), 7.0);
        assert!(
            text.contains("<Name>Ada's &lt;loop&gt; &amp; </Name>"),
            "{text}"
        );
        let back = read_tcx(&text).unwrap();
        same(&pts, &back.points, 0.05);
        assert_eq!(back.name.as_deref(), Some("Ada's <loop> &"));
        assert_eq!(TrackFormat::sniff(text.as_bytes()), Some(TrackFormat::Tcx));
        assert!(text.contains("<Time>2026-01-01T00:00:00Z</Time>"));
    }

    #[test]
    fn geojson_written_reads_back_without_the_via_points() {
        let pts = track();
        let wp = vec![("Via 1".to_string(), 47.14, 9.52)];
        let text = write_geojson(&export(&pts, &wp));
        let back = read_geojson(&text).unwrap();
        same(&pts, &back.points, 0.05);
        assert_eq!(back.parts(), 1);
        assert_eq!(
            TrackFormat::sniff(text.as_bytes()),
            Some(TrackFormat::GeoJson)
        );
        assert!(text.contains("OpenStreetMap"));
    }

    #[test]
    fn tcx_activities_with_prefixes_and_points_without_position() {
        let text = r#"<?xml version="1.0"?>
<ns:TrainingCenterDatabase xmlns:ns="x"><ns:Activities><ns:Activity Sport="Biking">
<ns:Id>2026-11-16T08:00:00Z</ns:Id><ns:Lap><ns:Track>
<ns:Trackpoint><ns:Time>t</ns:Time><ns:HeartRateBpm><ns:Value>120</ns:Value></ns:HeartRateBpm></ns:Trackpoint>
<ns:Trackpoint><ns:Position><ns:LatitudeDegrees>47.1</ns:LatitudeDegrees><ns:LongitudeDegrees>9.5</ns:LongitudeDegrees></ns:Position><ns:AltitudeMeters>455</ns:AltitudeMeters></ns:Trackpoint>
<!-- <ns:Trackpoint><ns:Position><ns:LatitudeDegrees>0</ns:LatitudeDegrees></ns:Position></ns:Trackpoint> -->
<ns:Trackpoint><ns:Position><ns:LatitudeDegrees>47.2</ns:LatitudeDegrees><ns:LongitudeDegrees>9.6</ns:LongitudeDegrees></ns:Position></ns:Trackpoint>
</ns:Track></ns:Lap></ns:Activity></ns:Activities></ns:TrainingCenterDatabase>"#;
        let g = read_tcx(text).unwrap();
        assert_eq!(g.points.len(), 2);
        assert_eq!(g.points[0].ele, Some(455.0));
        assert_eq!(g.points[1].ele, None);
        assert_eq!(g.name.as_deref(), Some("2026-11-16T08:00:00Z"));
        assert!(read_tcx("<gpx/>").is_err());
        assert!(read_tcx("<TrainingCenterDatabase></TrainingCenterDatabase>").is_err());
    }

    #[test]
    fn geojson_variants() {
        let multi = r#"{"type":"MultiLineString","coordinates":[[[9.5,47.1],[9.51,47.1]],[[9.52,47.1,460],[9.53,47.1]]]}"#;
        let g = read_geojson(multi).unwrap();
        assert_eq!((g.points.len(), g.parts()), (4, 2));
        assert_eq!(g.points[2].ele, Some(460.0));
        let fc = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Cy"},"geometry":{"type":"Point","coordinates":[9.5,47.1]}},{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[9.5,47.1],[9.51,47.1]]}}]}"#;
        let g = read_geojson(fc).unwrap();
        assert_eq!(g.points.len(), 2);
        assert_eq!(g.name.as_deref(), Some("Cy"));
        // Each feature's lines are parts with its name.
        let two = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Up"},"geometry":{"type":"LineString","coordinates":[[9.5,47.1],[9.51,47.1]]}},{"type":"Feature","properties":{"name":"Down"},"geometry":{"type":"LineString","coordinates":[[9.52,47.1],[9.53,47.1],[9.54,47.1]]}}]}"#;
        let g = read_geojson(two).unwrap();
        assert_eq!(
            g.part_ranges(),
            vec![(0..2, Some("Up".into())), (2..5, Some("Down".into()))]
        );
        for bad in [
            "[]",
            "{}",
            r#"{"type":"Point","coordinates":[1,2]}"#,
            r#"{"type":"LineString","coordinates":[[1,99],[1,2]]}"#,
            "not json",
        ] {
            assert!(read_geojson(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn formats_by_name_and_content() {
        assert_eq!(TrackFormat::of_name("a/b/Ride.GPX"), Some(TrackFormat::Gpx));
        assert_eq!(TrackFormat::of_name("x.fit"), Some(TrackFormat::Fit));
        assert_eq!(
            TrackFormat::of_name("x.ocwroute"),
            Some(TrackFormat::GraphRoute)
        );
        assert_eq!(TrackFormat::of_name("x.kml"), None);
        assert_eq!(TrackFormat::of_name("noext"), None);
        let mut fit = vec![14u8, 0x20, 0, 0, 0, 0, 0, 0];
        fit.extend_from_slice(b".FIT");
        assert_eq!(TrackFormat::sniff(&fit), Some(TrackFormat::Fit));
        assert_eq!(
            TrackFormat::sniff(b"ocw-graph-route 1\nname x\n"),
            Some(TrackFormat::GraphRoute)
        );
        assert_eq!(TrackFormat::sniff(b"hello"), None);
    }

    #[test]
    fn iso_times() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(COURSE_EPOCH_UNIX_S), "2026-01-01T00:00:00Z");
        assert_eq!(iso_time(1_772_323_199), "2026-02-28T23:59:59Z");
        assert_eq!(iso_time(951_782_400), "2000-02-29T00:00:00Z");
    }
}
