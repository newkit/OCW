//! GPX 1.0 / 1.1 reading (spec §2.7, step 1): the points of all tracks
//! (`trk/trkseg/trkpt`, segments joined) and routes (`rte/rtept`), with
//! latitude, longitude and the optional elevation; where each segment
//! starts is kept ([`Gpx::segments`]), so one of them can be chosen
//! ([`Gpx::part`], spec 14 Phase 16, 16D).
//!
//! A small hand-written scanner instead of an XML library: GPX is simple,
//! and `ocw-world` stays dependency-free and builds for wasm (spec §3.5),
//! so the web companion can import GPX with the same code (§2.10). It
//! understands elements, attributes in either quote style, comments,
//! CDATA, processing instructions and the five predefined entities; it
//! ignores namespaces' prefixes (`<gpx:trkpt>` works too).

/// One GPX point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpxPoint {
    pub lat: f64,
    pub lon: f64,
    /// Elevation in metres (only shown for comparison, spec §2.7 step 4).
    pub ele: Option<f64>,
}

/// The points of a GPX file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gpx {
    /// `<name>` of the first track or route (or of the metadata).
    pub name: Option<String>,
    /// Track points of all tracks and segments in file order, then route
    /// points (a file usually has one or the other).
    pub points: Vec<GpxPoint>,
    /// The track segments and routes that were joined, in file order
    /// (empty: the points are one part).
    pub segments: Vec<GpxSegment>,
}

/// Where a track segment (`trkseg`) or route (`rte`) starts in
/// [`Gpx::points`] (TCX: a `Track`, GeoJSON: a line).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpxSegment {
    /// Index of its first point.
    pub first: usize,
    /// `<name>` of its track or route.
    pub name: Option<String>,
}

impl Gpx {
    /// Parses a GPX document. Errors name the problem (not GPX, a point
    /// without coordinates, coordinates out of range, no points).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut gpx = Gpx::default();
        // The current track's (or route's) first segment and its name.
        let mut track_first = 0;
        let mut track_name: Option<String> = None;
        // Whose <name> is being read ("trk", "rte", "metadata", "gpx").
        let mut name_of = String::new();
        let mut seen_root = false;
        // Point being read: (lat, lon, ele).
        let mut point: Option<GpxPoint> = None;
        // Inside <ele> (of a point) / <name> (of a track, route or metadata).
        let mut text_target: Option<&'static str> = None;
        let mut depth_names: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut rest = text;
        while !rest.is_empty() {
            let Some(lt) = rest.find('<') else {
                break;
            };
            if text_target.is_some() {
                buf.push_str(&unescape(&rest[..lt]));
            }
            rest = &rest[lt..];
            if let Some(r) = rest.strip_prefix("<!--") {
                let end = r.find("-->").ok_or("unterminated comment")?;
                rest = &r[end + 3..];
                continue;
            }
            if let Some(r) = rest.strip_prefix("<![CDATA[") {
                let end = r.find("]]>").ok_or("unterminated CDATA")?;
                if text_target.is_some() {
                    buf.push_str(&r[..end]);
                }
                rest = &r[end + 3..];
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>').ok_or("unterminated declaration")?;
                rest = &rest[end + 1..];
                continue;
            }
            let end = tag_end(rest).ok_or("unterminated tag")?;
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                let name = local(name.trim());
                depth_names.pop();
                match name {
                    "trkpt" | "rtept" => {
                        if let Some(p) = point.take() {
                            gpx.points.push(p);
                        }
                    }
                    "ele" if text_target == Some("ele") => {
                        if let Some(p) = &mut point {
                            p.ele = buf.trim().parse().ok();
                        }
                        text_target = None;
                    }
                    "name" if text_target == Some("name") => {
                        let n = buf.trim();
                        if gpx.name.is_none() && !n.is_empty() {
                            gpx.name = Some(n.to_string());
                        }
                        if matches!(name_of.as_str(), "trk" | "rte") && !n.is_empty() {
                            track_name = Some(n.to_string());
                            for s in &mut gpx.segments[track_first..] {
                                s.name = track_name.clone();
                            }
                        }
                        text_target = None;
                    }
                    _ => {}
                }
                continue;
            }
            let self_closing = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let (name, attrs) = match tag.find(|c: char| c.is_whitespace()) {
                Some(i) => (&tag[..i], &tag[i..]),
                None => (tag, ""),
            };
            let name = local(name);
            if !seen_root {
                if name != "gpx" {
                    return Err(format!("not a GPX file (root element <{name}>)"));
                }
                seen_root = true;
            }
            match name {
                "trk" | "rte" => {
                    track_first = gpx.segments.len();
                    track_name = None;
                    if name == "rte" {
                        gpx.segments.push(GpxSegment {
                            first: gpx.points.len(),
                            name: None,
                        });
                    }
                }
                "trkseg" => gpx.segments.push(GpxSegment {
                    first: gpx.points.len(),
                    name: track_name.clone(),
                }),
                "trkpt" | "rtept" => {
                    let get = |key: &str| -> Result<f64, String> {
                        let v = attribute(attrs, key)
                            .ok_or_else(|| format!("<{name}> without {key}"))?;
                        v.trim()
                            .parse::<f64>()
                            .map_err(|e| format!("<{name}> {key}=\"{v}\": {e}"))
                    };
                    let (lat, lon) = (get("lat")?, get("lon")?);
                    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                        return Err(format!("<{name}> lat {lat} lon {lon} out of range"));
                    }
                    let p = GpxPoint {
                        lat,
                        lon,
                        ele: None,
                    };
                    if self_closing {
                        gpx.points.push(p);
                    } else {
                        point = Some(p);
                    }
                }
                "ele" if point.is_some() && !self_closing => {
                    text_target = Some("ele");
                    buf.clear();
                }
                "name"
                    if !self_closing
                        && point.is_none()
                        && matches!(
                            depth_names.last().map(String::as_str),
                            Some("trk" | "rte" | "metadata" | "gpx")
                        ) =>
                {
                    text_target = Some("name");
                    name_of = depth_names.last().cloned().unwrap_or_default();
                    buf.clear();
                }
                _ => {}
            }
            if !self_closing {
                depth_names.push(name.to_string());
            }
        }
        if !seen_root {
            return Err("not a GPX file (no <gpx> element)".into());
        }
        if gpx.points.is_empty() {
            return Err("the GPX file has no track or route points".into());
        }
        Ok(gpx)
    }

    /// Number of joined parts (segments and routes with points).
    pub fn parts(&self) -> usize {
        self.part_ranges().len()
    }

    /// The parts with points: their point ranges and names, in file order
    /// (points before the first segment are a part of their own).
    pub fn part_ranges(&self) -> Vec<(std::ops::Range<usize>, Option<String>)> {
        let mut starts: Vec<(usize, Option<String>)> = Vec::new();
        if self.segments.first().is_none_or(|s| s.first > 0) {
            starts.push((0, None));
        }
        starts.extend(self.segments.iter().map(|s| (s.first, s.name.clone())));
        let mut out = Vec::new();
        for (k, (first, name)) in starts.iter().enumerate() {
            let end = starts
                .get(k + 1)
                .map_or(self.points.len(), |n| n.0)
                .min(self.points.len());
            if end > *first {
                out.push((*first..end, name.clone()));
            }
        }
        out
    }

    /// Part `i` of [`Gpx::part_ranges`] alone (its own name, else the
    /// file's), or `None`.
    pub fn part(&self, i: usize) -> Option<Gpx> {
        let (range, name) = self.part_ranges().into_iter().nth(i)?;
        Some(Gpx {
            name: name.or_else(|| self.name.clone()),
            points: self.points[range].to_vec(),
            segments: Vec::new(),
        })
    }

    /// Total horizontal length (great-circle approximation), in metres.
    pub fn length_m(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| haversine_m(w[0].lat, w[0].lon, w[1].lat, w[1].lon))
            .sum()
    }

    /// Bounding box (south, west, north, east) in degrees (spec §2.7 step 2:
    /// the region check).
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let mut b = (f64::INFINITY, f64::INFINITY, -f64::INFINITY, -f64::INFINITY);
        for p in &self.points {
            b = (
                b.0.min(p.lat),
                b.1.min(p.lon),
                b.2.max(p.lat),
                b.3.max(p.lon),
            );
        }
        b
    }

    /// A GPX 1.1 document with one track of `points` (for fixtures and the
    /// web companion's export).
    pub fn to_text(name: &str, points: &[GpxPoint]) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"Open Cycling World\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
        );
        s.push_str(&format!(
            "  <trk>\n    <name>{}</name>\n    <trkseg>\n",
            escape(name)
        ));
        for p in points {
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
}

/// Great-circle distance in metres.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * crate::geo::EARTH_RADIUS_M * a.sqrt().asin()
}

/// End of the tag starting at `s[0] == '<'`: the `>` outside quotes.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// Element name without a namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Value of attribute `key` (no namespace prefix) in an attribute list.
fn attribute(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = local(rest[..eq].trim());
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let close = after[1..].find(q)? + 1;
        let value = &after[1..close];
        if name == key {
            return Some(unescape(value));
        }
        rest = &after[close + 1..];
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = include_str!("../tests/fixtures/two-segments.gpx");

    #[test]
    fn tracks_with_segments_are_joined() {
        let g = Gpx::parse(TRACK).unwrap();
        assert_eq!(g.name.as_deref(), Some("Morning ride & coffee"));
        assert_eq!(g.parts(), 2);
        // Each segment on its own, named after its track.
        let r = g.part_ranges();
        assert_eq!((r[0].0.clone(), r[1].0.clone()), (0..3, 3..5));
        let second = g.part(1).unwrap();
        assert_eq!(second.points, g.points[3..]);
        assert_eq!(second.name.as_deref(), Some("Morning ride & coffee"));
        assert_eq!(second.parts(), 1);
        assert!(g.part(2).is_none());
        assert_eq!(g.points.len(), 5);
        assert_eq!(
            g.points[0],
            GpxPoint {
                lat: 47.14,
                lon: 9.52,
                ele: Some(455.5)
            }
        );
        // Single-quoted, self-closing, no elevation; a comment in between.
        assert_eq!(g.points[3].ele, None);
        assert_eq!(g.points[4].lon, 9.5232);
        let (s, w, n, e) = g.bounds();
        assert!(s < n && w < e);
        assert!(g.length_m() > 100.0);
    }

    #[test]
    fn routes_and_prefixed_names() {
        let text = r#"<?xml version="1.0"?>
<gpx:gpx xmlns:gpx="http://www.topografix.com/GPX/1/1" version="1.1">
  <gpx:rte><gpx:name><![CDATA[Planned <loop>]]></gpx:name>
    <gpx:rtept lat="47.1" lon="9.5"><gpx:ele>500</gpx:ele></gpx:rtept>
    <gpx:rtept lat="47.2" lon="9.6"></gpx:rtept>
  </gpx:rte>
</gpx:gpx>"#;
        let g = Gpx::parse(text).unwrap();
        assert_eq!(g.name.as_deref(), Some("Planned <loop>"));
        assert_eq!(g.points.len(), 2);
        assert_eq!(g.points[0].ele, Some(500.0));
        assert_eq!(g.parts(), 1);
    }

    #[test]
    fn point_names_do_not_name_the_track() {
        let text = r#"<gpx><trk><trkseg><trkpt lat="1" lon="2"><name>wp</name></trkpt></trkseg><name>T</name></trk></gpx>"#;
        assert_eq!(Gpx::parse(text).unwrap().name.as_deref(), Some("T"));
    }

    #[test]
    fn every_track_and_route_is_a_part_with_its_own_name() {
        let text = r#"<gpx>
  <trk><name>Out</name><trkseg><trkpt lat="1" lon="2"/><trkpt lat="1.1" lon="2"/></trkseg></trk>
  <trk><trkseg><trkpt lat="2" lon="2"/><trkpt lat="2.1" lon="2"/></trkseg><trkseg></trkseg><name>Back</name></trk>
  <rte><rtept lat="3" lon="2"/><rtept lat="3.1" lon="2"/><rtept lat="3.2" lon="2"/></rte>
</gpx>"#;
        let g = Gpx::parse(text).unwrap();
        assert_eq!(g.name.as_deref(), Some("Out"));
        assert_eq!(g.points.len(), 7);
        // The empty segment is no part.
        let r = g.part_ranges();
        assert_eq!(
            r,
            vec![
                (0..2, Some("Out".to_string())),
                (2..4, Some("Back".to_string())),
                (4..7, None)
            ]
        );
        // A part without a name gets the file's.
        assert_eq!(g.part(2).unwrap().name.as_deref(), Some("Out"));
        assert_eq!(g.part(1).unwrap().points[0].lat, 2.0);
        // Points without segments are one part.
        let one = Gpx {
            name: None,
            points: g.points.clone(),
            segments: Vec::new(),
        };
        assert_eq!(one.part_ranges(), vec![(0..7, None)]);
    }

    #[test]
    fn broken_files_are_reported() {
        assert!(Gpx::parse("<kml></kml>").unwrap_err().contains("not a GPX"));
        assert!(Gpx::parse("hello").unwrap_err().contains("not a GPX"));
        assert!(Gpx::parse("<gpx></gpx>").unwrap_err().contains("no track"));
        assert!(
            Gpx::parse(r#"<gpx><trk><trkseg><trkpt lat="1"/></trkseg></trk></gpx>"#)
                .unwrap_err()
                .contains("without lon")
        );
        assert!(
            Gpx::parse(r#"<gpx><trkpt lat="91" lon="0"/></gpx>"#)
                .unwrap_err()
                .contains("out of range")
        );
        assert!(
            Gpx::parse(r#"<gpx><trkpt lat="x" lon="0"/></gpx>"#)
                .unwrap_err()
                .contains("lat=\"x\"")
        );
        assert!(Gpx::parse("<gpx><!-- open").is_err());
        assert!(Gpx::parse("<gpx><trkpt lat=\"1").is_err());
    }

    #[test]
    fn written_files_read_back() {
        let pts = vec![
            GpxPoint {
                lat: 47.1,
                lon: 9.5,
                ele: Some(450.25),
            },
            GpxPoint {
                lat: 47.1001,
                lon: 9.5002,
                ele: None,
            },
        ];
        let g = Gpx::parse(&Gpx::to_text("A & B", &pts)).unwrap();
        assert_eq!(g.name.as_deref(), Some("A & B"));
        assert_eq!(g.points.len(), 2);
        assert!((g.points[0].ele.unwrap() - 450.2).abs() < 0.11);
        assert!((haversine_m(47.0, 9.0, 47.001, 9.0) - 111.2).abs() < 0.2);
    }
}
