//! Peaks, mountain passes and viewpoints of the world (spec 4.7 *Places
//! section*, 2.11 Q4, 3.2 *Navigation map* Q8): OSM `natural=peak` with a
//! `name`, mountain passes (`mountain_pass=yes`, or `natural=saddle` with a
//! name) and `tourism=viewpoint` (named or not, with its `direction`). The
//! photographer's view score counts them as landmarks, the navigation map
//! and the studio's 2D map name the peaks and passes beside the
//! settlements.
//!
//! **Section `LMRK`** (optional, additive: older readers skip the tag, so a
//! package with it loads everywhere; packages without it have none). It
//! travels with the places ([`crate::places::Places::landmarks`]), is
//! small and read by skeletons and streamed bases too. Stored peaks first,
//! then passes, then viewpoints; higher first, then by name and id
//! ([`normalise`]).
//!
//! ```text
//! layout version: u8 (1)
//! count: u32, then per landmark:
//!   record length: u16 (the bytes after it; readers skip unknown bytes
//!     at a record's end, so fields can be added)
//!   kind u8 (0 peak, 1 pass, 2 viewpoint; only add, unknown codes are
//!     skipped by readers)
//!   flags u8 (bit 0: elevation known; bit 1: the elevation is the
//!     terrain's (the element has no `ele`); bit 2: direction known;
//!     bits 3–4: the OSM element, 0 node, 1 way)
//!   x, z: f32 f32 (package metres)
//!   elevation: f32 (m above sea level; 0 when not known)
//!   direction: u16 u16 (the centre in degrees clockwise from north
//!     0 … 359, the width in degrees 1 … 360; 0 0 when not known)
//!   OSM id: i64
//!   name: u16 length + UTF-8 (empty: no name)
//!   other names: u8 count, per name: language u8 length + ASCII,
//!     name u16 length + UTF-8 (as in `PLCE`)
//! ```

use crate::package::Bytes;
use crate::places::OsmElement;

/// Section tag.
pub const LANDMARKS_TAG: &[u8; 4] = b"LMRK";
const LANDMARKS_VERSION: u8 = 1;

/// What a landmark is. Codes are stored in packages: only add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LandmarkKind {
    /// `natural=peak` with a name.
    Peak,
    /// `mountain_pass=yes`, or `natural=saddle` with a name.
    Pass,
    /// `tourism=viewpoint`.
    Viewpoint,
}

impl LandmarkKind {
    pub const ALL: [LandmarkKind; 3] = [Self::Peak, Self::Pass, Self::Viewpoint];

    pub fn code(self) -> u8 {
        match self {
            Self::Peak => 0,
            Self::Pass => 1,
            Self::Viewpoint => 2,
        }
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == c)
    }

    /// The word for reports ("peak", "pass", "viewpoint").
    pub fn word(self) -> &'static str {
        match self {
            Self::Peak => "peak",
            Self::Pass => "pass",
            Self::Viewpoint => "viewpoint",
        }
    }
}

/// Where a viewpoint looks: the centre (degrees clockwise from north) and
/// the width of the view (degrees, 360 = all round).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewDirection {
    pub centre_deg: u16,
    pub width_deg: u16,
}

impl ViewDirection {
    /// Whether a bearing (degrees clockwise from north) lies in the view,
    /// with `margin_deg` more on each side.
    pub fn contains(&self, bearing_deg: f64, margin_deg: f64) -> bool {
        if self.width_deg >= 360 {
            return true;
        }
        let off = (bearing_deg - f64::from(self.centre_deg) + 540.0).rem_euclid(360.0) - 180.0;
        off.abs() <= f64::from(self.width_deg) / 2.0 + margin_deg
    }
}

/// The compass points OSM `direction` uses, in degrees.
fn compass_deg(v: &str) -> Option<f64> {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let v = v.trim().to_ascii_uppercase();
    match v.as_str() {
        "NORTH" => return Some(0.0),
        "EAST" => return Some(90.0),
        "SOUTH" => return Some(180.0),
        "WEST" => return Some(270.0),
        _ => {}
    }
    POINTS.iter().position(|p| *p == v).map(|i| i as f64 * 22.5)
}

fn angle(v: &str) -> Option<f64> {
    let v = v.trim();
    v.parse::<f64>()
        .ok()
        .filter(|a| a.is_finite())
        .or_else(|| compass_deg(v))
}

/// An OSM `direction` value as a view: degrees ("135"), compass points
/// ("SE", "NNW"), ranges ("90-180", "N-E", "W-NE", clockwise from the
/// first to the second), "all" / "360" (all round). A single direction
/// is a view 90° wide. `None` for anything else.
pub fn parse_direction(v: &str) -> Option<ViewDirection> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if matches!(v.to_ascii_lowercase().as_str(), "all" | "360" | "0-360") {
        return Some(ViewDirection {
            centre_deg: 0,
            width_deg: 360,
        });
    }
    // A range: "a-b" (a negative first number is not a range).
    let first = v.chars().next().map_or(1, char::len_utf8);
    if let Some((a, b)) = v[first..]
        .split_once('-')
        .map(|(a, b)| (&v[..a.len() + first], b))
        && let (Some(a), Some(b)) = (angle(a), angle(b))
    {
        let width = (b - a).rem_euclid(360.0);
        let width = if width == 0.0 { 360.0 } else { width };
        let centre = (a + width / 2.0).rem_euclid(360.0);
        return Some(ViewDirection {
            centre_deg: (centre.round() as u16) % 360,
            width_deg: (width.round() as u16).clamp(1, 360),
        });
    }
    let a = angle(v)?;
    Some(ViewDirection {
        centre_deg: (a.rem_euclid(360.0).round() as u16) % 360,
        width_deg: 90,
    })
}

/// An OSM `ele` value ("3715", "3715 m", "3715,5", "1234.5m") in metres;
/// feet ("12188 ft") are converted. `None` for anything else.
pub fn parse_ele(v: &str) -> Option<f32> {
    let v = v.trim();
    let num: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))
        .collect();
    let x: f64 = num.replace(',', ".").parse().ok()?;
    let rest = v[num.len()..].trim().to_ascii_lowercase();
    let m = match rest.as_str() {
        "" | "m" | "masl" | "m a.s.l." | "müm" => x,
        "ft" | "feet" | "'" => x * 0.3048,
        _ => return None,
    };
    (m.is_finite() && (-500.0..9000.0).contains(&m)).then_some(m as f32)
}

/// A peak, pass or viewpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub kind: LandmarkKind,
    /// Package metres (x east, z south).
    pub pos: [f32; 2],
    /// Metres above sea level: OSM `ele`, else the terrain's height there
    /// ([`Landmark::ele_from_terrain`]), `None` when neither is known.
    pub ele: Option<f32>,
    /// The elevation is the package terrain's, not a tagged `ele`.
    pub ele_from_terrain: bool,
    /// A viewpoint's `direction`.
    pub direction: Option<ViewDirection>,
    /// OSM `name` (empty: none – viewpoints may have none).
    pub name: String,
    /// `name:LANG` → name, the [`crate::places::NAME_LANGUAGES`] that
    /// differ from `name`.
    pub names: Vec<(String, String)>,
    pub element: OsmElement,
    pub osm_id: i64,
}

impl Landmark {
    /// The name in `lang` if the package has it, else `name`.
    pub fn label(&self, lang: Option<&str>) -> &str {
        lang.and_then(|l| self.names.iter().find(|(k, _)| k == l))
            .map_or(self.name.as_str(), |(_, v)| v.as_str())
    }

    /// The elevation as the maps write it ("3715 m"), rounded to the metre.
    pub fn ele_text(&self) -> Option<String> {
        self.ele.map(|e| format!("{} m", e.round() as i32))
    }
}

/// Sorts peaks first, then passes, then viewpoints; higher first, then
/// name, element and id – deterministic for any input order.
pub fn normalise(v: &mut [Landmark]) {
    v.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(
                b.ele
                    .unwrap_or(f32::NEG_INFINITY)
                    .total_cmp(&a.ele.unwrap_or(f32::NEG_INFINITY)),
            )
            .then_with(|| a.name.cmp(&b.name))
            .then(a.element.cmp(&b.element))
            .then(a.osm_id.cmp(&b.osm_id))
    });
}

/// How many of each kind ([`LandmarkKind::ALL`] order).
pub fn counts(v: &[Landmark]) -> [usize; 3] {
    let mut c = [0; 3];
    for l in v {
        c[l.kind.code() as usize] += 1;
    }
    c
}

/// "12 peaks (11 with ele), 3 passes, 40 viewpoints (12 named)" for build
/// reports.
pub fn summary(v: &[Landmark]) -> String {
    let c = counts(v);
    let tagged = |k: LandmarkKind| {
        v.iter()
            .filter(|l| l.kind == k && l.ele.is_some() && !l.ele_from_terrain)
            .count()
    };
    let named = v
        .iter()
        .filter(|l| l.kind == LandmarkKind::Viewpoint && !l.name.is_empty())
        .count();
    format!(
        "{} peaks ({} with ele), {} passes ({} with ele), {} viewpoints ({named} named, {} with direction)",
        c[0],
        tagged(LandmarkKind::Peak),
        c[1],
        tagged(LandmarkKind::Pass),
        c[2],
        v.iter().filter(|l| l.direction.is_some()).count()
    )
}

fn put_str16(b: &mut Vec<u8>, s: &str) {
    let s = truncate(s, 1024);
    b.extend((s.len() as u16).to_le_bytes());
    b.extend(s.as_bytes());
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Writes the `LMRK` payload.
pub fn write_landmarks(v: &[Landmark], b: &mut Vec<u8>) {
    b.push(LANDMARKS_VERSION);
    b.extend((v.len() as u32).to_le_bytes());
    for l in v {
        let element = match l.element {
            OsmElement::Node => 0u8,
            _ => 1,
        };
        let flags = u8::from(l.ele.is_some())
            | (u8::from(l.ele.is_some() && l.ele_from_terrain) << 1)
            | (u8::from(l.direction.is_some()) << 2)
            | (element << 3);
        let mut r = vec![l.kind.code(), flags];
        r.extend(l.pos[0].to_le_bytes());
        r.extend(l.pos[1].to_le_bytes());
        r.extend(l.ele.unwrap_or(0.0).to_le_bytes());
        let d = l.direction.unwrap_or(ViewDirection {
            centre_deg: 0,
            width_deg: 0,
        });
        r.extend(d.centre_deg.to_le_bytes());
        r.extend(d.width_deg.to_le_bytes());
        r.extend(l.osm_id.to_le_bytes());
        put_str16(&mut r, &l.name);
        let names: Vec<_> = l.names.iter().take(16).collect();
        r.push(names.len() as u8);
        for (lang, name) in names {
            let lang = truncate(lang, 16);
            r.push(lang.len() as u8);
            r.extend(lang.as_bytes());
            put_str16(&mut r, name);
        }
        b.extend((r.len() as u16).to_le_bytes());
        b.extend(r);
    }
}

fn str16(r: &mut Bytes) -> Result<String, String> {
    let n = u16::from_le_bytes(r.arr()?) as usize;
    String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "landmark name is not UTF-8".to_string())
}

/// Reads a `LMRK` payload written by [`write_landmarks`]; an unknown
/// layout is "not supported" (the reader skips the section), records of
/// unknown kinds are left out.
pub fn read_landmarks(payload: &[u8]) -> Result<Vec<Landmark>, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != LANDMARKS_VERSION {
        return Err(format!("landmarks layout {version} not supported"));
    }
    let count = r.u32()? as usize;
    let mut out = Vec::with_capacity(count.min(payload.len() / 30 + 1));
    for _ in 0..count {
        let len = u16::from_le_bytes(r.arr()?) as usize;
        let mut q = Bytes::new(r.take(len)?);
        let Some(kind) = LandmarkKind::from_code(q.u8()?) else {
            continue;
        };
        let flags = q.u8()?;
        let pos = [q.f32()?, q.f32()?];
        let ele = q.f32()?;
        let centre: u16 = u16::from_le_bytes(q.arr()?);
        let width: u16 = u16::from_le_bytes(q.arr()?);
        let osm_id = q.i64()?;
        let name = str16(&mut q)?;
        let n = q.u8()?;
        let mut names = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let l = q.u8()? as usize;
            let lang = String::from_utf8(q.take(l)?.to_vec())
                .map_err(|_| "landmark name language is not UTF-8".to_string())?;
            names.push((lang, str16(&mut q)?));
        }
        // Fields of newer builders after these are skipped.
        out.push(Landmark {
            kind,
            pos,
            ele: (flags & 1 != 0).then_some(ele),
            ele_from_terrain: flags & 2 != 0,
            direction: (flags & 4 != 0).then_some(ViewDirection {
                centre_deg: centre % 360,
                width_deg: width.clamp(1, 360),
            }),
            name,
            names,
            element: if (flags >> 3) & 3 == 1 {
                OsmElement::Way
            } else {
                OsmElement::Node
            },
            osm_id,
        });
    }
    r.finish("landmarks section")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(kind: LandmarkKind, name: &str, ele: Option<f32>) -> Landmark {
        Landmark {
            kind,
            pos: [10.0, -20.0],
            ele,
            ele_from_terrain: false,
            direction: None,
            name: name.into(),
            names: Vec::new(),
            element: OsmElement::Node,
            osm_id: 1,
        }
    }

    #[test]
    fn landmarks_round_trip_and_sort_peaks_first_higher_first() {
        let mut a = mark(LandmarkKind::Viewpoint, "", None);
        a.direction = parse_direction("SE");
        a.element = OsmElement::Way;
        a.osm_id = -5;
        let mut b = mark(LandmarkKind::Peak, "Pico Ada", Some(2100.0));
        b.names = vec![("en".into(), "Ada Peak".into())];
        let c = mark(LandmarkKind::Peak, "Monte Bea", Some(3100.5));
        let mut d = mark(LandmarkKind::Pass, "Cora Pass", Some(1800.0));
        d.ele_from_terrain = true;
        let mut v = vec![a, b, c, d];
        normalise(&mut v);
        let names: Vec<&str> = v.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Monte Bea", "Pico Ada", "Cora Pass", ""]);
        let mut bytes = Vec::new();
        write_landmarks(&v, &mut bytes);
        let back = read_landmarks(&bytes).unwrap();
        assert_eq!(back, v);
        assert_eq!(back[1].label(Some("en")), "Ada Peak");
        assert_eq!(back[0].ele_text().as_deref(), Some("3101 m"));
        assert!(back[2].ele_from_terrain);
        assert_eq!(back[3].direction.unwrap().centre_deg, 135);
        assert!(summary(&back).contains("2 peaks (2 with ele), 1 passes (0 with ele)"));
    }

    #[test]
    fn newer_layouts_kinds_and_fields_are_skipped() {
        let v = vec![
            mark(LandmarkKind::Peak, "Pico Ada", Some(900.0)),
            mark(LandmarkKind::Pass, "Bea Pass", None),
        ];
        let mut bytes = Vec::new();
        write_landmarks(&v, &mut bytes);
        let mut newer = bytes.clone();
        newer[0] = 2;
        assert!(
            read_landmarks(&newer)
                .unwrap_err()
                .contains("not supported")
        );
        // An unknown kind: that record is left out.
        let mut unknown = bytes.clone();
        unknown[7] = 40;
        let back = read_landmarks(&unknown).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Bea Pass");
        // Two more bytes at the first record's end (a newer field).
        let len = u16::from_le_bytes([bytes[5], bytes[6]]);
        let mut longer = bytes[..5].to_vec();
        longer.extend((len + 2).to_le_bytes());
        longer.extend(&bytes[7..7 + len as usize]);
        longer.extend([1, 2]);
        longer.extend(&bytes[7 + len as usize..]);
        assert_eq!(read_landmarks(&longer).unwrap(), v);
    }

    #[test]
    fn directions_and_elevations_parse_as_osm_writes_them() {
        let d = |v: &str| parse_direction(v).map(|d| (d.centre_deg, d.width_deg));
        assert_eq!(d("90"), Some((90, 90)));
        assert_eq!(d("NNW"), Some((338, 90)));
        assert_eq!(d("90-180"), Some((135, 90)));
        assert_eq!(d("W-NE"), Some((338, 135)));
        assert_eq!(d("Ö"), None);
        assert_eq!(d("315-45"), Some((0, 90)));
        assert_eq!(d("all"), Some((0, 360)));
        assert_eq!(d("over the valley"), None);
        let all = parse_direction("360").unwrap();
        assert!(all.contains(200.0, 0.0));
        let se = parse_direction("SE").unwrap();
        assert!(se.contains(100.0, 0.0) && !se.contains(80.0, 0.0) && se.contains(80.0, 10.0));
        let north = parse_direction("N").unwrap();
        assert!(north.contains(350.0, 0.0) && north.contains(40.0, 0.0));
        assert_eq!(parse_ele("3715"), Some(3715.0));
        assert_eq!(parse_ele("3715 m"), Some(3715.0));
        assert_eq!(parse_ele("1234,5"), Some(1234.5));
        assert_eq!(parse_ele("1000 ft"), Some(304.8));
        assert_eq!(parse_ele("about 900"), None);
        assert_eq!(parse_ele("12000"), None);
    }
}
