//! Place names of the world (spec §4.7 *Places section*, §14 Phase 16 16A
//! *Map labels*): the cities, towns, villages, hamlets, suburbs,
//! neighbourhoods, localities and isolated dwellings OSM maps
//! (`place=*` with a `name`), and the label layout the 2D maps draw them
//! with.
//!
//! **Section `PLCE`** (optional, additive; older readers skip it, packages
//! without it have no places). Written by the world builder from OSM
//! nodes and, where a place is mapped only as an area (a closed way or a
//! multipolygon), from the area's centroid. Places are stored most
//! important first ([`Places::normalise`]): by [`Place::rank`], then
//! population, then name.
//!
//! ```text
//! layout version: u8 (1)
//! place count: u32, then per place:
//!   record length: u16 (the bytes after it; readers skip what they do
//!     not know at a record's end, so fields can be added)
//!   kind u8 (PlaceKind code; only add), rank u8 (0 = most important),
//!   flags u8 (bit 0: mapped as an area; bits 1–2: the OSM element,
//!     0 node, 1 way, 2 relation), capital u8 (admin level it is the
//!     capital of, 0 = none)
//!   x, z: f32 f32 (package metres)
//!   population: u32 (u32::MAX = not tagged)
//!   OSM id: i64
//!   name: u16 length + UTF-8
//!   other names: u8 count, per name: language u8 length + ASCII,
//!     name u16 length + UTF-8 (the `name:LANG` of [`NAME_LANGUAGES`] that
//!     differ from `name`)
//! ```
//!
//! [`layout_labels`] is the decluttering pass of the maps (pure, used
//! every frame): a greedy placement in priority order where a label is
//! shown only if its box overlaps no label placed before it, with a small
//! hysteresis so labels do not flicker while the map pans.

use std::collections::BTreeSet;

use crate::package::Bytes;

/// Section tag.
pub const PLACES_TAG: &[u8; 4] = b"PLCE";
const PLACES_VERSION: u8 = 1;
/// Population value of a place without a `population` tag.
const NO_POPULATION: u32 = u32::MAX;
/// The `name:LANG` keys the builder keeps (when they differ from `name`).
pub const NAME_LANGUAGES: [&str; 6] = ["en", "de", "es", "fr", "it", "rm"];
/// The lowest (least important) rank.
pub const LOWEST_RANK: u8 = 5;

/// What OSM's `place=*` says a place is. Codes are stored in packages:
/// only add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlaceKind {
    City,
    Town,
    Village,
    Hamlet,
    Suburb,
    Neighbourhood,
    Locality,
    IsolatedDwelling,
    /// A code from a newer builder.
    Unknown,
}

impl PlaceKind {
    pub const ALL: [PlaceKind; 8] = [
        Self::City,
        Self::Town,
        Self::Village,
        Self::Hamlet,
        Self::Suburb,
        Self::Neighbourhood,
        Self::Locality,
        Self::IsolatedDwelling,
    ];

    /// The kind of an OSM `place` value; `None` for values that are no
    /// settlement label (island, square, farm, county, …).
    pub fn from_osm(v: &str) -> Option<Self> {
        Some(match v {
            "city" => Self::City,
            "town" => Self::Town,
            "village" => Self::Village,
            "hamlet" => Self::Hamlet,
            "suburb" | "quarter" => Self::Suburb,
            "neighbourhood" | "neighborhood" => Self::Neighbourhood,
            "locality" => Self::Locality,
            "isolated_dwelling" => Self::IsolatedDwelling,
            _ => return None,
        })
    }

    pub fn osm(self) -> &'static str {
        match self {
            Self::City => "city",
            Self::Town => "town",
            Self::Village => "village",
            Self::Hamlet => "hamlet",
            Self::Suburb => "suburb",
            Self::Neighbourhood => "neighbourhood",
            Self::Locality => "locality",
            Self::IsolatedDwelling => "isolated_dwelling",
            Self::Unknown => "unknown",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::City => 0,
            Self::Town => 1,
            Self::Village => 2,
            Self::Hamlet => 3,
            Self::Suburb => 4,
            Self::Neighbourhood => 5,
            Self::Locality => 6,
            Self::IsolatedDwelling => 7,
            Self::Unknown => 255,
        }
    }

    pub fn from_code(c: u8) -> Self {
        Self::ALL
            .into_iter()
            .find(|k| k.code() == c)
            .unwrap_or(Self::Unknown)
    }

    /// The rank of the kind alone: city 0, town 1, village 2, hamlet and
    /// suburb 3, neighbourhood 4, locality and isolated dwelling 5.
    pub fn base_rank(self) -> u8 {
        match self {
            Self::City => 0,
            Self::Town => 1,
            Self::Village => 2,
            Self::Hamlet | Self::Suburb => 3,
            Self::Neighbourhood => 4,
            Self::Locality | Self::IsolatedDwelling | Self::Unknown => LOWEST_RANK,
        }
    }
}

/// Which OSM element a place comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OsmElement {
    Node,
    Way,
    Relation,
}

impl OsmElement {
    fn code(self) -> u8 {
        match self {
            Self::Node => 0,
            Self::Way => 1,
            Self::Relation => 2,
        }
    }

    fn from_code(c: u8) -> Self {
        match c {
            1 => Self::Way,
            2 => Self::Relation,
            _ => Self::Node,
        }
    }
}

/// One named place.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub kind: PlaceKind,
    /// Importance for labels, 0 (most important) … [`LOWEST_RANK`]:
    /// [`place_rank`].
    pub rank: u8,
    /// Package metres (x east, z south).
    pub pos: [f32; 2],
    /// OSM `name`.
    pub name: String,
    /// `name:LANG` → name, for the languages of [`NAME_LANGUAGES`] that
    /// differ from `name`.
    pub names: Vec<(String, String)>,
    /// OSM `population`, when tagged.
    pub population: Option<u32>,
    /// The admin level this place is the capital of (OSM `capital`:
    /// `yes` = 2 unless `admin_level` says more), `None` if none.
    pub capital: Option<u8>,
    /// Mapped as an area (its position is the area's centroid).
    pub area: bool,
    pub element: OsmElement,
    pub osm_id: i64,
}

impl Place {
    /// The name in `lang` if the package has it, else `name`.
    pub fn label(&self, lang: Option<&str>) -> &str {
        lang.and_then(|l| self.names.iter().find(|(k, _)| k == l))
            .map_or(self.name.as_str(), |(_, v)| v.as_str())
    }
}

/// The rank of a place: its kind's [`PlaceKind::base_rank`], one higher
/// (more important) for the capital of a country or state (admin level 4
/// or lower), and a town or village with 100,000 or more inhabitants
/// ranks with the cities.
pub fn place_rank(kind: PlaceKind, capital: Option<u8>, population: Option<u32>) -> u8 {
    let mut r = kind.base_rank();
    if capital.is_some_and(|l| l <= 4) {
        r = r.saturating_sub(1);
    }
    if population.is_some_and(|p| p >= 100_000) && r <= 2 {
        r = 0;
    }
    r
}

/// The OSM `capital` value (and `admin_level`) as the admin level it is
/// the capital of.
pub fn capital_level(capital: Option<&str>, admin_level: Option<&str>) -> Option<u8> {
    match capital?.trim() {
        "yes" => Some(admin_level.and_then(|a| a.trim().parse().ok()).unwrap_or(2)),
        "no" => None,
        v => v.parse().ok().filter(|l| (1..=12).contains(l)),
    }
}

/// An OSM `population` value ("12,345", "12 345", "12345") as a number.
pub fn parse_population(v: &str) -> Option<u32> {
    let digits: String = v
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '\''))
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The places of a package (section `PLCE`) and its peaks, passes and
/// viewpoints (section `LMRK`, [`crate::landmarks`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Places {
    /// Most important first ([`Places::normalise`]).
    pub places: Vec<Place>,
    /// Peaks, passes and viewpoints (section `LMRK`; empty in packages
    /// without it), [`crate::landmarks::normalise`]d.
    pub landmarks: Vec<crate::landmarks::Landmark>,
}

impl Places {
    pub fn new(mut places: Vec<Place>) -> Self {
        Self::normalise(&mut places);
        Self {
            places,
            landmarks: Vec::new(),
        }
    }

    /// These places with `landmarks` (sorted).
    pub fn with_landmarks(mut self, mut landmarks: Vec<crate::landmarks::Landmark>) -> Self {
        crate::landmarks::normalise(&mut landmarks);
        self.landmarks = landmarks;
        self
    }

    /// Sorts most important first: rank, population (larger first), name,
    /// then the OSM element – a deterministic order for any input order.
    pub fn normalise(places: &mut [Place]) {
        places.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(b.population.unwrap_or(0).cmp(&a.population.unwrap_or(0)))
                .then_with(|| a.name.cmp(&b.name))
                .then(a.element.cmp(&b.element))
                .then(a.osm_id.cmp(&b.osm_id))
        });
    }

    /// How many places of each kind ([`PlaceKind::ALL`] order, then
    /// unknown).
    pub fn counts(&self) -> [usize; 9] {
        let mut c = [0; 9];
        for p in &self.places {
            let i = PlaceKind::ALL
                .iter()
                .position(|k| *k == p.kind)
                .unwrap_or(8);
            c[i] += 1;
        }
        c
    }

    /// "3 cities, 12 towns, …" for build reports.
    pub fn summary(&self) -> String {
        let c = self.counts();
        let mut parts = vec![format!("{} places", self.places.len())];
        for (k, n) in PlaceKind::ALL.iter().zip(c) {
            if n > 0 {
                parts.push(format!("{} {n}", k.osm()));
            }
        }
        parts.push(format!(
            "{} as areas, {} with population",
            self.places.iter().filter(|p| p.area).count(),
            self.places
                .iter()
                .filter(|p| p.population.is_some())
                .count()
        ));
        parts.join(", ")
    }
}

fn put_str16(b: &mut Vec<u8>, s: &str) {
    let s = truncate(s, u16::MAX as usize);
    b.extend((s.len() as u16).to_le_bytes());
    b.extend(s.as_bytes());
}

/// `s` cut to at most `max` bytes on a character boundary.
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

/// Writes the `PLCE` payload.
pub fn write_places(p: &Places, b: &mut Vec<u8>) {
    b.push(PLACES_VERSION);
    b.extend((p.places.len() as u32).to_le_bytes());
    for place in &p.places {
        let mut r = vec![
            place.kind.code(),
            place.rank,
            u8::from(place.area) | (place.element.code() << 1),
            place.capital.unwrap_or(0),
        ];
        r.extend(place.pos[0].to_le_bytes());
        r.extend(place.pos[1].to_le_bytes());
        r.extend(place.population.unwrap_or(NO_POPULATION).to_le_bytes());
        r.extend(place.osm_id.to_le_bytes());
        put_str16(&mut r, &place.name);
        let names: Vec<_> = place.names.iter().take(255).collect();
        r.push(names.len() as u8);
        for (lang, name) in names {
            let lang = truncate(lang, 255);
            r.push(lang.len() as u8);
            r.extend(lang.as_bytes());
            put_str16(&mut r, name);
        }
        // A record is far below 64 KB unless the names are huge; cut the
        // other names then rather than write a broken record.
        if r.len() > u16::MAX as usize {
            r.truncate(28);
            put_str16(&mut r, truncate(&place.name, 1024));
            r.push(0);
        }
        b.extend((r.len() as u16).to_le_bytes());
        b.extend(r);
    }
}

fn str16(r: &mut Bytes) -> Result<String, String> {
    let n = u16::from_le_bytes(r.arr()?) as usize;
    String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "place name is not UTF-8".to_string())
}

/// Reads a `PLCE` payload written by [`write_places`]; an unknown layout
/// is "not supported" (the reader skips the section).
pub fn read_places(payload: &[u8]) -> Result<Places, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != PLACES_VERSION {
        return Err(format!("places layout {version} not supported"));
    }
    let count = r.u32()? as usize;
    let mut places = Vec::with_capacity(count.min(payload.len() / 30 + 1));
    for _ in 0..count {
        let len = u16::from_le_bytes(r.arr()?) as usize;
        let mut q = Bytes::new(r.take(len)?);
        let kind = PlaceKind::from_code(q.u8()?);
        let rank = q.u8()?.min(LOWEST_RANK);
        let flags = q.u8()?;
        let capital = Some(q.u8()?).filter(|c| *c != 0);
        let pos = [q.f32()?, q.f32()?];
        let population = Some(q.u32()?).filter(|p| *p != NO_POPULATION);
        let osm_id = q.i64()?;
        let name = str16(&mut q)?;
        let n = q.u8()?;
        let mut names = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let l = q.u8()? as usize;
            let lang = String::from_utf8(q.take(l)?.to_vec())
                .map_err(|_| "place name language is not UTF-8".to_string())?;
            names.push((lang, str16(&mut q)?));
        }
        // Fields of newer builders after these are skipped.
        places.push(Place {
            kind,
            rank,
            pos,
            name,
            names,
            population,
            capital,
            area: flags & 1 != 0,
            element: OsmElement::from_code((flags >> 1) & 3),
            osm_id,
        });
    }
    r.finish("places section")?;
    Ok(Places {
        places,
        landmarks: Vec::new(),
    })
}

// ---------------------------------------------------------------------
// Labels on the 2D maps (spec §14 Phase 16 16A *Map labels*).

/// How a place label of a rank is drawn at a zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelStyle {
    /// Font size (screen points).
    pub size: f32,
    /// Bold (cities and towns).
    pub strong: bool,
    /// Grey level of the text, 0 … 255 (smaller places darker).
    pub grey: u8,
}

/// The coarsest zoom (metres per screen point) a rank's labels are drawn
/// at: cities always, towns from 150, villages from 40, hamlets and
/// suburbs from 15, neighbourhoods from 8, localities and isolated
/// dwellings from 4.
pub fn rank_max_m_per_px(rank: u8) -> f64 {
    match rank {
        0 => f64::INFINITY,
        1 => 150.0,
        2 => 40.0,
        3 => 15.0,
        4 => 8.0,
        _ => 4.0,
    }
}

/// The style of a rank's labels at `m_per_px`, `None` where the rank is
/// not drawn. The size grows a little as the map zooms in past the rank's
/// threshold (up to +20 % at a quarter of it).
pub fn label_style(rank: u8, m_per_px: f64) -> Option<LabelStyle> {
    let max = rank_max_m_per_px(rank);
    if m_per_px.is_nan() || m_per_px > max {
        return None;
    }
    let (base, strong, grey) = match rank {
        0 => (18.0, true, 255),
        1 => (15.0, true, 250),
        2 => (13.0, false, 240),
        3 => (11.5, false, 225),
        4 => (10.5, false, 215),
        _ => (10.0, false, 205),
    };
    // Cities: grow from 150 m per point in (their own threshold is none).
    let reference = if max.is_finite() { max } else { 150.0 };
    let closer = (reference / m_per_px.max(1e-6)).log2().clamp(0.0, 2.0);
    Some(LabelStyle {
        size: base * (1.0 + 0.1 * closer as f32),
        strong,
        grey,
    })
}

/// A label that could be drawn: its place, its priority and its box on
/// the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelCandidate {
    /// The caller's id of the place (e.g. its index in [`Places`]):
    /// stable across frames, used for the hysteresis.
    pub id: u32,
    pub rank: u8,
    /// 0 when not tagged.
    pub population: u32,
    /// The place on the screen (points).
    pub anchor: [f32; 2],
    /// The label's box on the screen: min, max (points).
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// Space kept free around every label (points).
pub const LABEL_PAD: f32 = 2.0;
/// Extra space a label that was not shown in the last layout needs to
/// appear (points): the hysteresis that keeps labels from flickering when
/// two of them nearly touch while the map pans or zooms.
pub const LABEL_APPEAR_PAD: f32 = 4.0;

/// Population in coarse classes (factors of two), so that close numbers do
/// not reorder labels.
fn population_class(p: u32) -> u32 {
    32 - p.leading_zeros()
}

/// The decluttering pass: the ids of the candidates to draw, in priority
/// order. Candidates are taken by rank, then population (in factors of
/// two), then whether they were shown last time (`previous`), then their
/// distance to `centre` (the view's centre on the screen), then id; one is
/// placed only if its box (grown by [`LABEL_PAD`], and by
/// [`LABEL_APPEAR_PAD`] more if it was not in `previous`) overlaps no box
/// placed before it, and only if its anchor lies in the viewport
/// (`view_min`, `view_max`). So the most important places stay when it is
/// crowded and smaller ones come in as the map zooms in; the same input
/// gives the same output.
pub fn layout_labels(
    candidates: &[LabelCandidate],
    centre: [f32; 2],
    view_min: [f32; 2],
    view_max: [f32; 2],
    previous: &BTreeSet<u32>,
) -> Vec<u32> {
    let inside = |p: [f32; 2]| {
        p[0] >= view_min[0] && p[0] <= view_max[0] && p[1] >= view_min[1] && p[1] <= view_max[1]
    };
    let mut order: Vec<(&LabelCandidate, bool, f32)> = candidates
        .iter()
        .filter(|c| inside(c.anchor))
        .map(|c| {
            let d = (c.anchor[0] - centre[0]).hypot(c.anchor[1] - centre[1]);
            (c, previous.contains(&c.id), d)
        })
        .collect();
    order.sort_by(|a, b| {
        a.0.rank
            .cmp(&b.0.rank)
            .then(population_class(b.0.population).cmp(&population_class(a.0.population)))
            .then(b.1.cmp(&a.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.0.id.cmp(&b.0.id))
    });
    let mut placed: Vec<([f32; 2], [f32; 2])> = Vec::new();
    let mut shown = Vec::new();
    for (c, was, _) in order {
        let pad = LABEL_PAD + if was { 0.0 } else { LABEL_APPEAR_PAD };
        let (lo, hi) = (
            [c.min[0] - pad, c.min[1] - pad],
            [c.max[0] + pad, c.max[1] + pad],
        );
        let free = placed
            .iter()
            .all(|(a, b)| lo[0] >= b[0] || hi[0] <= a[0] || lo[1] >= b[1] || hi[1] <= a[1]);
        if free {
            // Placed boxes keep only the common pad: the appear pad is a
            // condition for the newcomer, not space it takes.
            placed.push((
                [c.min[0] - LABEL_PAD, c.min[1] - LABEL_PAD],
                [c.max[0] + LABEL_PAD, c.max[1] + LABEL_PAD],
            ));
            shown.push(c.id);
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, kind: PlaceKind, pos: [f32; 2], population: Option<u32>) -> Place {
        Place {
            kind,
            rank: place_rank(kind, None, population),
            pos,
            name: name.into(),
            names: Vec::new(),
            population,
            capital: None,
            area: false,
            element: OsmElement::Node,
            osm_id: 1,
        }
    }

    #[test]
    fn places_round_trip_through_the_section() {
        let mut a = place("Ada Town", PlaceKind::Town, [12.5, -40.25], Some(5_200));
        a.capital = Some(2);
        a.rank = place_rank(a.kind, a.capital, a.population);
        a.names = vec![
            ("en".into(), "Ada City".into()),
            ("de".into(), "Adastadt".into()),
        ];
        let mut b = place("Bea Weiler", PlaceKind::Hamlet, [-300.0, 900.0], None);
        b.area = true;
        b.element = OsmElement::Relation;
        b.osm_id = -7;
        let p = Places::new(vec![b, a]);
        let mut bytes = Vec::new();
        write_places(&p, &mut bytes);
        let back = read_places(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.places[0].name, "Ada Town");
        assert_eq!(
            back.places[0].rank, 0,
            "a capital town ranks with the cities"
        );
        assert_eq!(back.places[0].label(Some("en")), "Ada City");
        assert_eq!(back.places[0].label(Some("fr")), "Ada Town");
        assert_eq!(back.places[1].element, OsmElement::Relation);
    }

    #[test]
    fn a_newer_layout_is_not_supported_and_newer_fields_are_skipped() {
        let p = Places::new(vec![place("Cora", PlaceKind::Village, [1.0, 2.0], None)]);
        let mut bytes = Vec::new();
        write_places(&p, &mut bytes);
        let mut newer = bytes.clone();
        newer[0] = 9;
        assert!(read_places(&newer).unwrap_err().contains("not supported"));
        // A record with two more bytes at its end (a newer field).
        let mut longer = bytes[..5].to_vec();
        let len = u16::from_le_bytes([bytes[5], bytes[6]]) + 2;
        longer.extend(len.to_le_bytes());
        longer.extend(&bytes[7..]);
        longer.extend([0xAB, 0xCD]);
        assert_eq!(read_places(&longer).unwrap(), p);
        // Unknown kinds read as unknown.
        let mut unknown = bytes.clone();
        unknown[7] = 77;
        assert_eq!(
            read_places(&unknown).unwrap().places[0].kind,
            PlaceKind::Unknown
        );
    }

    #[test]
    fn ranks_follow_kind_capital_and_population() {
        assert_eq!(place_rank(PlaceKind::City, None, None), 0);
        assert_eq!(place_rank(PlaceKind::Town, None, None), 1);
        assert_eq!(place_rank(PlaceKind::Town, Some(2), None), 0);
        assert_eq!(place_rank(PlaceKind::Town, Some(6), None), 1);
        assert_eq!(place_rank(PlaceKind::Village, None, Some(120_000)), 0);
        assert_eq!(place_rank(PlaceKind::Hamlet, None, None), 3);
        assert_eq!(place_rank(PlaceKind::Suburb, None, None), 3);
        assert_eq!(place_rank(PlaceKind::Locality, None, None), 5);
        assert_eq!(capital_level(Some("yes"), None), Some(2));
        assert_eq!(capital_level(Some("yes"), Some("4")), Some(4));
        assert_eq!(capital_level(Some("6"), None), Some(6));
        assert_eq!(capital_level(Some("no"), None), None);
        assert_eq!(parse_population("12,345"), Some(12_345));
        assert_eq!(parse_population("5 400 (2020)"), Some(5_400));
        assert_eq!(parse_population("about"), None);
    }

    #[test]
    fn normalise_orders_by_importance_whatever_the_input_order() {
        let ps = vec![
            place("Dora", PlaceKind::Village, [0.0; 2], Some(800)),
            place("Elia", PlaceKind::City, [0.0; 2], Some(90_000)),
            place("Fina", PlaceKind::Village, [0.0; 2], Some(3_000)),
            place("Gita", PlaceKind::Town, [0.0; 2], None),
        ];
        let mut rev = ps.clone();
        rev.reverse();
        let a = Places::new(ps);
        let b = Places::new(rev);
        assert_eq!(a, b);
        let names: Vec<&str> = a.places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Elia", "Gita", "Fina", "Dora"]);
        assert!(a.summary().contains("4 places"));
    }

    /// Candidates for `places` seen at `m_per_px` around (0, 0) on a
    /// 800 × 600 map, with a made-up text width of 7 points per letter.
    fn candidates(places: &[Place], m_per_px: f64, centre: [f64; 2]) -> Vec<LabelCandidate> {
        places
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let s = label_style(p.rank, m_per_px)?;
                let anchor = [
                    400.0 + ((f64::from(p.pos[0]) - centre[0]) / m_per_px) as f32,
                    300.0 + ((f64::from(p.pos[1]) - centre[1]) / m_per_px) as f32,
                ];
                let w = p.name.len() as f32 * s.size * 0.55;
                Some(LabelCandidate {
                    id: i as u32,
                    rank: p.rank,
                    population: p.population.unwrap_or(0),
                    anchor,
                    min: [anchor[0] - w / 2.0, anchor[1] - s.size - 3.0],
                    max: [anchor[0] + w / 2.0, anchor[1] + 3.0],
                })
            })
            .collect()
    }

    fn layout(places: &[Place], m_per_px: f64, centre: [f64; 2], prev: &BTreeSet<u32>) -> Vec<u32> {
        layout_labels(
            &candidates(places, m_per_px, centre),
            [400.0, 300.0],
            [0.0, 0.0],
            [800.0, 600.0],
            prev,
        )
    }

    /// Two cities 6 km apart with villages crowded around each (every
    /// 400 m on a ring).
    fn crowded() -> Vec<Place> {
        let mut v = vec![
            place("Ada City", PlaceKind::City, [-3000.0, 0.0], Some(80_000)),
            place("Bea City", PlaceKind::City, [3000.0, 0.0], Some(60_000)),
            place("Cora Town", PlaceKind::Town, [0.0, 2500.0], Some(9_000)),
        ];
        for (c, cx) in [("A", -3000.0f32), ("B", 3000.0)] {
            for i in 0..16 {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                for r in [400.0f32, 900.0] {
                    v.push(place(
                        &format!("{c}-Dorf {i} {r}"),
                        PlaceKind::Village,
                        [cx + r * a.cos(), r * a.sin()],
                        None,
                    ));
                }
            }
        }
        v.push(place("Hof Ida", PlaceKind::Hamlet, [-3000.0, 150.0], None));
        Places::new(v).places
    }

    #[test]
    fn a_crowded_map_keeps_the_cities_and_drops_smaller_places() {
        let ps = crowded();
        let shown = layout(&ps, 30.0, [0.0, 0.0], &BTreeSet::new());
        let names: Vec<&str> = shown
            .iter()
            .map(|i| ps[*i as usize].name.as_str())
            .collect();
        assert!(
            names.contains(&"Ada City") && names.contains(&"Bea City"),
            "{names:?}"
        );
        assert!(names.contains(&"Cora Town"), "{names:?}");
        let villages = ps.iter().filter(|p| p.kind == PlaceKind::Village).count();
        let shown_villages = shown
            .iter()
            .filter(|i| ps[**i as usize].kind == PlaceKind::Village)
            .count();
        assert!(
            shown_villages < villages / 2,
            "{shown_villages} of {villages}"
        );
        // Hamlets are not drawn this far out at all.
        assert!(!names.contains(&"Hof Ida"));
        // No two shown boxes overlap.
        let c = candidates(&ps, 30.0, [0.0, 0.0]);
        let boxes: Vec<_> = shown
            .iter()
            .map(|i| c.iter().find(|c| c.id == *i).unwrap())
            .collect();
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let apart = a.min[0] >= b.max[0]
                    || a.max[0] <= b.min[0]
                    || a.min[1] >= b.max[1]
                    || a.max[1] <= b.min[1];
                assert!(
                    apart,
                    "{} / {}",
                    ps[a.id as usize].name, ps[b.id as usize].name
                );
            }
        }
    }

    #[test]
    fn zooming_in_reveals_villages() {
        let ps = crowded();
        let far = layout(&ps, 30.0, [-3000.0, 0.0], &BTreeSet::new());
        let near = layout(&ps, 3.0, [-3000.0, 0.0], &BTreeSet::new());
        let villages = |s: &[u32]| {
            s.iter()
                .filter(|i| ps[**i as usize].kind == PlaceKind::Village)
                .count()
        };
        assert!(
            villages(&near) > villages(&far),
            "{} vs {}",
            villages(&near),
            villages(&far)
        );
        let names: Vec<&str> = near.iter().map(|i| ps[*i as usize].name.as_str()).collect();
        assert!(names.contains(&"Ada City"), "{names:?}");
        // Out of view at 3 m per point: the other city.
        assert!(!names.contains(&"Bea City"));
    }

    #[test]
    fn the_layout_is_deterministic_and_does_not_flicker_while_panning() {
        let ps = crowded();
        let a = layout(&ps, 20.0, [0.0, 0.0], &BTreeSet::new());
        let b = layout(&ps, 20.0, [0.0, 0.0], &BTreeSet::new());
        assert_eq!(a, b);
        // Pan in small steps: a label stays shown while its place stays in
        // view – labels only go by leaving the view or by a more important
        // label (whose rank is lower) taking the place.
        let mut prev: BTreeSet<u32> = a.iter().copied().collect();
        for step in 1..=40 {
            let centre = [step as f64 * 15.0, step as f64 * 4.0];
            let now = layout(&ps, 20.0, centre, &prev);
            let now_set: BTreeSet<u32> = now.iter().copied().collect();
            let in_view = |i: u32| {
                let p = ps[i as usize].pos;
                let (x, y) = (
                    400.0 + (f64::from(p[0]) - centre[0]) / 20.0,
                    300.0 + (f64::from(p[1]) - centre[1]) / 20.0,
                );
                (0.0..=800.0).contains(&x) && (0.0..=600.0).contains(&y)
            };
            for gone in prev.difference(&now_set) {
                assert!(
                    !in_view(*gone),
                    "step {step}: {} dropped",
                    ps[*gone as usize].name
                );
            }
            prev = now_set;
        }
    }

    #[test]
    fn styles_shrink_with_rank_and_grow_with_zoom() {
        let city = label_style(0, 200.0).unwrap();
        let town = label_style(1, 100.0).unwrap();
        let village = label_style(2, 30.0).unwrap();
        assert!(city.size > town.size && town.size > village.size);
        assert!(city.strong && town.strong && !village.strong);
        assert!(label_style(2, 60.0).is_none());
        assert!(label_style(5, 5.0).is_none());
        assert!(label_style(5, 3.0).is_some());
        assert!(label_style(2, 5.0).unwrap().size > village.size);
    }
}
