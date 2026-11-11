//! GPX 1.0 / 1.1 reading (spec §2.7, step 1): the points of all tracks
//! (`trk/trkseg/trkpt`, segments joined) and routes (`rte/rtept`), with
//! latitude, longitude and the optional elevation.
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
    /// Number of track segments and routes that were joined.
    pub parts: usize,
}

impl Gpx {
    /// Parses a GPX document. Errors name the problem (not GPX, a point
    /// without coordinates, coordinates out of range, no points).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut gpx = Gpx::default();
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
                "trkseg" | "rte" => gpx.parts += 1,
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
        assert_eq!(g.parts, 2);
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
        assert_eq!(g.parts, 1);
    }

    #[test]
    fn point_names_do_not_name_the_track() {
        let text = r#"<gpx><trk><trkseg><trkpt lat="1" lon="2"><name>wp</name></trkpt></trkseg><name>T</name></trk></gpx>"#;
        assert_eq!(Gpx::parse(text).unwrap().name.as_deref(), Some("T"));
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
