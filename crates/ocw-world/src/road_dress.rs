//! What a road wears besides its surface (spec §4.4, §4.6 step 6, §14
//! Phase 8 T5): lanes and markings, cycle lanes, kerbs and sidewalks, and
//! the crossings, stop and give-way lines painted across it – from OSM
//! (`lanes`, `lanes:forward` / `backward`, `overtaking`, `lane_markings`,
//! `cycleway*`, `sidewalk*`, `kerb`, crossing nodes) with defaults by road
//! class.
//!
//! The renderer draws the markings in the road shader (`road.wesl`) from a
//! small code per strip ([`MarkCode`]); [`marking_lines`] and
//! [`paint_at`] are the shader's Rust twins (the layout per class, lanes
//! and regional [`MarkingStyle`]), [`RoadDress::kerb_at`] the sidewalk and
//! kerb profile along a strip.
//!
//! In a world package the dress is the section `RDRS`: per tile, per road
//! strip (in the tile's strip order) its dress. Only tiles where a strip
//! differs from its class default ([`RoadDress::of_class`]) are written,
//! so a package without OSM dress has no section, and older readers skip
//! it. The section starts with a layout version; a reader skips a newer
//! version it does not know.

use crate::graph::{Oneway, RoadClass};
use crate::package::{Bytes, TileKey};
use crate::road_surface::RoadSurface;

/// Section tag of the road dress.
pub const ROAD_DRESS_TAG: &[u8; 4] = b"RDRS";
/// Layout version inside the section.
const DRESS_VERSION: u8 = 1;

/// Road material layer of the sidewalk pavers (after the five
/// [`RoadSurface`] layers; `ocw_assets::materials::ROAD_IDS`).
pub const SIDEWALK_LAYER: u8 = 5;
/// Road material layer of the kerb stones.
pub const KERB_LAYER: u8 = 6;
/// Height of a raised kerb (m).
pub const DEFAULT_KERB_M: f32 = 0.12;
/// Height of a lowered kerb (m).
pub const LOWERED_KERB_M: f32 = 0.04;
/// Width of a sidewalk without `sidewalk:width` (m).
pub const DEFAULT_SIDEWALK_M: f32 = 2.0;
/// Sidewalk widths accepted from OSM (m).
pub const SIDEWALK_RANGE_M: (f32, f32) = (0.8, 6.0);
/// Sidewalks fall towards the road by this gradient (the renderer's
/// `road_mesh` draws them so; [`crate::package::PavementIndex::height`]).
pub const SIDEWALK_FALL: f32 = 0.02;
/// Kerbs and sidewalks taper to nothing over this length into a junction
/// zone (a lowered, rounded corner; m).
pub const KERB_RAMP_M: f32 = 2.0;
/// A junction zone reaches this far beyond the half width of the widest
/// other road at the junction (the corner; m).
pub const CORNER_M: f32 = 2.0;
/// Largest junction zone stored (m; the section keeps decimetres in a byte).
pub const MAX_JUNCTION_M: f32 = 25.5;

/// The line between the two directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CentreLine {
    #[default]
    None = 0,
    Dashed = 1,
    Solid = 2,
    /// Two solid lines (wide roads with two or more lanes each way).
    Double = 3,
    /// No overtaking for the forward direction (`overtaking:forward=no`):
    /// solid on the forward (right) side, dashed on the other.
    SolidForward = 4,
    /// No overtaking for the backward direction.
    SolidBackward = 5,
}

impl CentreLine {
    pub const ALL: [CentreLine; 6] = [
        CentreLine::None,
        CentreLine::Dashed,
        CentreLine::Solid,
        CentreLine::Double,
        CentreLine::SolidForward,
        CentreLine::SolidBackward,
    ];

    fn from_code(c: u8) -> Option<Self> {
        Self::ALL.get(usize::from(c)).copied()
    }
}

/// A cycle lane painted on the carriageway (`cycleway*=lane`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CycleLane {
    #[default]
    None = 0,
    /// A lane of unknown kind: the regional style decides.
    Lane = 1,
    /// Crossable, dashed (`cycleway:lane=advisory`, German "Schutzstreifen").
    Advisory = 2,
    /// Solid (`cycleway:lane=exclusive`, "Radfahrstreifen").
    Exclusive = 3,
}

impl CycleLane {
    pub const ALL: [CycleLane; 4] = [
        CycleLane::None,
        CycleLane::Lane,
        CycleLane::Advisory,
        CycleLane::Exclusive,
    ];
}

/// What is painted across the road at a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CrossingKind {
    /// Zebra crossing: bars along the road across its whole width.
    Zebra = 0,
    /// Crossing with traffic signals: two dashed lines across the road
    /// and a stop line before them in each direction.
    Signals = 1,
    /// Stop line (`highway=stop`).
    Stop = 2,
    /// Give-way line (`highway=give_way`): dashed.
    GiveWay = 3,
}

impl CrossingKind {
    pub const ALL: [CrossingKind; 4] = [
        CrossingKind::Zebra,
        CrossingKind::Signals,
        CrossingKind::Stop,
        CrossingKind::GiveWay,
    ];

    /// The kind of a way node from its OSM tags (`None`: nothing painted):
    /// marked crossings are zebras unless they have signals (spec §14
    /// Phase 8 T5).
    pub fn from_osm<'a>(tag: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        match tag("highway") {
            Some("stop") => return Some(CrossingKind::Stop),
            Some("give_way") => return Some(CrossingKind::GiveWay),
            _ => {}
        }
        let crossing = tag("crossing");
        let markings = tag("crossing:markings");
        if matches!(crossing, Some("unmarked" | "no" | "informal"))
            || matches!(markings, Some("no"))
        {
            return None;
        }
        // Zebra markings win over signals (Swiss crossings with lights
        // keep their yellow bars).
        if matches!(markings, Some("zebra" | "zebra:double" | "zebra:bicolour"))
            || tag("crossing_ref") == Some("zebra")
            || crossing == Some("zebra")
        {
            return Some(CrossingKind::Zebra);
        }
        if crossing == Some("traffic_signals") || tag("crossing:signals") == Some("yes") {
            return Some(CrossingKind::Signals);
        }
        if matches!(crossing, Some("marked" | "uncontrolled")) || markings == Some("yes") {
            return Some(CrossingKind::Zebra);
        }
        if matches!(
            markings,
            Some("lines" | "dashes" | "ladder" | "ladder:skewed")
        ) {
            return Some(CrossingKind::Signals);
        }
        None
    }
}

/// Which half of the road a transverse line covers (looking along the
/// strip; right-hand traffic: the forward lanes are on the right, +u).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Half {
    #[default]
    Both = 0,
    Forward = 1,
    Backward = 2,
}

impl Half {
    pub const ALL: [Half; 3] = [Half::Both, Half::Forward, Half::Backward];
}

/// A transverse marking on a strip, centred at `along_m` (strip-local
/// metres from its first point).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub along_m: f32,
    pub kind: CrossingKind,
    pub half: Half,
}

/// The dress of one road strip. Left and right are seen along the strip
/// (its points' order, the OSM way's direction).
#[derive(Debug, Clone, PartialEq)]
pub struct RoadDress {
    /// Arc length (m, along the drawn points) of the strip's first point
    /// on its road edge: dashes, wear and textures run on across the tiles.
    pub start_m: f32,
    /// Length of the road edge (m; 0: unknown, no junction zones).
    pub edge_m: f32,
    /// Lanes [forward, backward]; [0, 0]: unknown (unmarked).
    pub lanes: [u8; 2],
    pub centre: CentreLine,
    /// Solid lines along the carriageway edges (not beside a kerb).
    pub edge_lines: bool,
    /// Dashed lines between lanes of the same direction.
    pub lane_lines: bool,
    /// Cycle lanes [left, right].
    pub cycle: [CycleLane; 2],
    /// Sidewalk widths [left, right] (m; 0: none).
    pub sidewalk_m: [f32; 2],
    /// Kerb height (m) where there is a sidewalk.
    pub kerb_m: f32,
    /// The sidewalks are asphalt (`sidewalk:*:surface=asphalt`), not pavers.
    pub sidewalk_asphalt: bool,
    /// Junction zones at the edge's [start, end] (m along the edge; 0:
    /// none): no kerbs or sidewalks there, the crossing roads come in.
    pub junction_m: [f32; 2],
    pub crossings: Vec<Crossing>,
}

impl Default for RoadDress {
    fn default() -> Self {
        Self::of_class(RoadClass::Residential, 5.0)
    }
}

/// The main roads that get lane markings without tags.
fn main_road(class: RoadClass) -> bool {
    matches!(
        class,
        RoadClass::Trunk
            | RoadClass::Primary
            | RoadClass::Secondary
            | RoadClass::Tertiary
            | RoadClass::Unclassified
    )
}

/// Roads for motor traffic (kerbs and sidewalks only there).
fn carriageway(class: RoadClass) -> bool {
    main_road(class) || matches!(class, RoadClass::Residential | RoadClass::Service)
}

/// Main roads at least this wide get a centre line without `lanes` (m).
pub const CENTRE_LINE_MIN_WIDTH_M: f32 = 5.5;

/// What a sidewalk tag says about one side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidewalkTag {
    /// No tag: the world builder looks for a separately mapped sidewalk.
    Unknown,
    No,
    /// A sidewalk of this width (m).
    Yes(f32),
    /// Mapped as a separate way (`footway=sidewalk`) beside the road.
    Separate,
}

/// A length in metres from OSM ("2", "2.5 m", "2,5").
pub fn parse_metres(v: &str) -> Option<f32> {
    let v = v.trim().trim_end_matches('m').trim().replace(',', ".");
    v.parse::<f32>().ok().filter(|x| x.is_finite())
}

/// The sidewalk tags of a way, [left, right]: `sidewalk`, `sidewalk:both`,
/// `sidewalk:left` / `:right` (most specific first) and their `:width`.
pub fn sidewalk_tags<'a>(tag: impl Fn(&str) -> Option<&'a str>) -> [SidewalkTag; 2] {
    let width = |side: &str| {
        tag(&format!("sidewalk:{side}:width"))
            .or_else(|| tag("sidewalk:both:width"))
            .or_else(|| tag("sidewalk:width"))
            .and_then(parse_metres)
            .map_or(DEFAULT_SIDEWALK_M, |w| {
                w.clamp(SIDEWALK_RANGE_M.0, SIDEWALK_RANGE_M.1)
            })
    };
    let value = |v: &str, side: &str| match v {
        "yes" | "both" | "left" | "right" => SidewalkTag::Yes(width(side)),
        "no" | "none" => SidewalkTag::No,
        "separate" => SidewalkTag::Separate,
        _ => SidewalkTag::Unknown,
    };
    let mut out = [SidewalkTag::Unknown; 2];
    for (i, side) in ["left", "right"].into_iter().enumerate() {
        if let Some(v) = tag(&format!("sidewalk:{side}")) {
            out[i] = value(v, side);
        } else if let Some(v) = tag("sidewalk:both") {
            out[i] = value(v, side);
        } else if let Some(v) = tag("sidewalk") {
            out[i] = match v {
                "both" | "yes" | "separate" | "no" | "none" => value(v, side),
                "left" | "right" if v == side => SidewalkTag::Yes(width(side)),
                "left" | "right" => SidewalkTag::No,
                _ => SidewalkTag::Unknown,
            };
        }
    }
    out
}

/// Kerb height from `kerb`, `sidewalk:kerb`, `sidewalk:both:kerb` (m).
pub fn kerb_height<'a>(tag: impl Fn(&str) -> Option<&'a str>) -> f32 {
    if let Some(h) = tag("kerb:height").and_then(parse_metres) {
        return h.clamp(0.0, 0.25);
    }
    let v = tag("sidewalk:both:kerb")
        .or_else(|| tag("sidewalk:kerb"))
        .or_else(|| tag("kerb"));
    match v {
        Some("lowered" | "rolled") => LOWERED_KERB_M,
        Some("flush" | "no") => 0.0,
        _ => DEFAULT_KERB_M,
    }
}

/// Cycle lanes [left, right] of a way.
pub fn cycle_lanes<'a>(oneway: Oneway, tag: impl Fn(&str) -> Option<&'a str>) -> [CycleLane; 2] {
    let kind = |side: Option<&str>| {
        let sub = side
            .and_then(|s| tag(&format!("cycleway:{s}:lane")))
            .or_else(|| tag("cycleway:both:lane"))
            .or_else(|| tag("cycleway:lane"));
        match sub {
            Some("advisory" | "pictogram") => CycleLane::Advisory,
            Some("exclusive" | "mandatory") => CycleLane::Exclusive,
            _ => CycleLane::Lane,
        }
    };
    let mut out = [CycleLane::None; 2];
    for (i, side) in ["left", "right"].into_iter().enumerate() {
        let v = tag(&format!("cycleway:{side}")).or_else(|| tag("cycleway:both"));
        if let Some(v) = v {
            if v == "lane" {
                out[i] = kind(Some(side));
            }
            continue;
        }
        let plain = tag("cycleway");
        // On a one-way road a plain cycle lane runs on the right of the
        // traffic, an opposite lane against it on the left.
        let here = matches!(
            (plain, oneway, i),
            (Some("lane"), Oneway::No, _)
                | (Some("lane"), Oneway::Forward, 1)
                | (Some("lane"), Oneway::Backward, 0)
                | (Some("opposite_lane"), Oneway::Forward, 0)
                | (Some("opposite_lane"), Oneway::Backward, 1)
        );
        if here {
            out[i] = kind(None);
        }
    }
    out
}

impl RoadDress {
    /// The dress of a road of `class` without OSM tags: main roads at
    /// least 5.5 m wide get a dashed centre line, trunk, primary and
    /// secondary roads from 6 m edge lines; nothing else is marked, and
    /// there are no sidewalks.
    pub fn of_class(class: RoadClass, width_m: f32) -> Self {
        let two = main_road(class) && width_m >= CENTRE_LINE_MIN_WIDTH_M;
        Self {
            start_m: 0.0,
            edge_m: 0.0,
            lanes: if two { [1, 1] } else { [0, 0] },
            centre: if two {
                CentreLine::Dashed
            } else {
                CentreLine::None
            },
            edge_lines: matches!(
                class,
                RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary
            ) && width_m >= 6.0,
            lane_lines: false,
            cycle: [CycleLane::None; 2],
            sidewalk_m: [0.0; 2],
            kerb_m: DEFAULT_KERB_M,
            sidewalk_asphalt: false,
            junction_m: [0.0; 2],
            crossings: Vec::new(),
        }
    }

    /// From the OSM tags of a way (`tag` returns a tag's value). Sidewalks
    /// tagged `separate` get the default width; untagged sides none (the
    /// world builder adds sidewalks mapped as separate ways).
    pub fn from_osm<'a>(
        class: RoadClass,
        width_m: f32,
        oneway: Oneway,
        tag: impl Fn(&str) -> Option<&'a str> + Copy,
    ) -> Self {
        let mut d = Self::of_class(class, width_m);
        let marked = match tag("lane_markings") {
            Some("no") => false,
            Some("yes") => true,
            _ => main_road(class),
        };
        let num = |k: &str| {
            tag(k)
                .and_then(|v| v.split(';').next()?.trim().parse::<u8>().ok())
                .map(|n| n.min(7))
        };
        let lanes = num("lanes");
        let (lf, lb) = (num("lanes:forward"), num("lanes:backward"));
        let explicit = lanes.is_some() || lf.is_some() || lb.is_some();
        d.lanes = match oneway {
            Oneway::Forward => [lanes.or(lf).unwrap_or(1).max(1), 0],
            Oneway::Backward => [0, lanes.or(lb).unwrap_or(1).max(1)],
            Oneway::No => match (lanes, lf, lb) {
                (_, Some(f), Some(b)) => [f, b],
                (Some(n), Some(f), None) => [f, n.saturating_sub(f)],
                (Some(n), None, Some(b)) => [n.saturating_sub(b), b],
                (Some(n), None, None) if n >= 2 => [n.div_ceil(2), n / 2],
                (Some(_), None, None) => [0, 0],
                (None, Some(f), None) => [f, 1],
                (None, None, Some(b)) => [1, b],
                (None, None, None) => d.lanes,
            },
        };
        let [f, b] = d.lanes;
        let wide_enough = if explicit {
            width_m >= 4.5
        } else {
            width_m >= CENTRE_LINE_MIN_WIDTH_M
        };
        let no = |k: &str| tag(k) == Some("no");
        d.centre = if !marked || f == 0 || b == 0 || !wide_enough {
            CentreLine::None
        } else if no("overtaking") || (no("overtaking:forward") && no("overtaking:backward")) {
            CentreLine::Solid
        } else if f >= 2 && b >= 2 {
            CentreLine::Double
        } else if no("overtaking:forward") {
            CentreLine::SolidForward
        } else if no("overtaking:backward") {
            CentreLine::SolidBackward
        } else {
            CentreLine::Dashed
        };
        d.lane_lines = marked && (f >= 2 || b >= 2);
        d.edge_lines = marked && d.edge_lines;
        if carriageway(class) {
            d.cycle = cycle_lanes(oneway, tag);
            let tags = sidewalk_tags(tag);
            for (w, t) in d.sidewalk_m.iter_mut().zip(tags) {
                *w = match t {
                    SidewalkTag::Yes(w) => w,
                    SidewalkTag::Separate => DEFAULT_SIDEWALK_M,
                    SidewalkTag::No | SidewalkTag::Unknown => 0.0,
                };
            }
            d.kerb_m = kerb_height(tag);
            d.sidewalk_asphalt = matches!(
                tag("sidewalk:both:surface")
                    .or_else(|| tag("sidewalk:surface"))
                    .or_else(|| tag("sidewalk:right:surface"))
                    .or_else(|| tag("sidewalk:left:surface")),
                Some("asphalt" | "concrete")
            );
        }
        // As the package stores them: decimetres and centimetres.
        d.sidewalk_m = d.sidewalk_m.map(|w| (w * 10.0).round() / 10.0);
        d.kerb_m = (d.kerb_m * 100.0).round() / 100.0;
        d
    }

    /// A street in a town (a sidewalk on either side): short dashes, no
    /// edge lines beside the kerbs.
    pub fn town(&self) -> bool {
        self.sidewalk_m.iter().any(|w| *w > 0.0)
    }

    /// Sidewalk width and kerb height on `side` (0 left, 1 right) at
    /// `along` (strip-local m): full between the junction zones, tapering
    /// to nothing over [`KERB_RAMP_M`] into them (a lowered corner).
    pub fn kerb_at(&self, side: usize, along: f32) -> (f32, f32) {
        let w = self.sidewalk_m[side];
        if w <= 0.0 {
            return (0.0, 0.0);
        }
        let s = self.start_m + along;
        let mut t = 1.0f32;
        if self.junction_m[0] > 0.0 {
            t = t.min((s - self.junction_m[0]) / KERB_RAMP_M);
        }
        if self.junction_m[1] > 0.0 && self.edge_m > 0.0 {
            t = t.min((self.edge_m - self.junction_m[1] - s) / KERB_RAMP_M);
        }
        let t = t.clamp(0.0, 1.0);
        (w * t, self.kerb_m * t)
    }

    /// Whether `along` (strip-local m) lies in a junction zone.
    pub fn in_junction(&self, along: f32) -> bool {
        let s = self.start_m + along;
        (self.junction_m[0] > 0.0 && s < self.junction_m[0])
            || (self.junction_m[1] > 0.0
                && self.edge_m > 0.0
                && s > self.edge_m - self.junction_m[1])
    }
}

// --- Markings: the shader's input and its Rust twin ---------------------------

/// The marking code of a strip as the road shader reads it (two small
/// integers in the second texture coordinate): what is painted, after the
/// surface rules (no paint on gravel, dirt or cobbles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MarkCode {
    pub lanes: [u8; 2],
    pub centre: CentreLine,
    pub edge_lines: bool,
    pub lane_lines: bool,
    pub town: bool,
    pub cycle: [CycleLane; 2],
    /// A track (`highway=track`): grass in the middle, wheel ruts.
    pub track: bool,
}

impl MarkCode {
    /// The code of a strip: its dress where the surface takes paint
    /// (asphalt), nothing painted elsewhere.
    pub fn of(dress: &RoadDress, class: RoadClass, surface: RoadSurface) -> Self {
        let paint = matches!(surface, RoadSurface::Asphalt | RoadSurface::WornAsphalt);
        Self {
            lanes: dress.lanes,
            centre: if paint {
                dress.centre
            } else {
                CentreLine::None
            },
            edge_lines: paint && dress.edge_lines,
            lane_lines: paint && dress.lane_lines,
            town: dress.town(),
            cycle: if paint {
                dress.cycle
            } else {
                [CycleLane::None; 2]
            },
            track: class == RoadClass::Track,
        }
    }

    /// The first integer (`uv_b.y`, < 4096): lanes, centre, edges, lane
    /// lines, town.
    pub fn lines_code(&self) -> u32 {
        u32::from(self.lanes[0].min(7))
            | u32::from(self.lanes[1].min(7)) << 3
            | (self.centre as u32) << 6
            | u32::from(self.edge_lines) << 9
            | u32::from(self.lane_lines) << 10
            | u32::from(self.town) << 11
    }

    /// The extra bits beside the layer and the kind (`uv_b.x / 64`, < 32):
    /// cycle lanes and the track look.
    pub fn extra_code(&self) -> u32 {
        self.cycle[0] as u32 | (self.cycle[1] as u32) << 2 | u32::from(self.track) << 4
    }

    pub fn decode(lines: u32, extra: u32) -> Self {
        let cyc = |c: u32| CycleLane::ALL[(c & 3) as usize];
        Self {
            lanes: [(lines & 7) as u8, (lines >> 3 & 7) as u8],
            centre: CentreLine::from_code((lines >> 6 & 7) as u8).unwrap_or_default(),
            edge_lines: lines >> 9 & 1 == 1,
            lane_lines: lines >> 10 & 1 == 1,
            town: lines >> 11 & 1 == 1,
            cycle: [cyc(extra), cyc(extra >> 2)],
            track: extra >> 4 & 1 == 1,
        }
    }
}

/// A painted line: width, dash and gap (gap 0: solid) in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineSpec {
    pub width: f32,
    pub dash: f32,
    pub gap: f32,
}

impl LineSpec {
    pub const fn solid(width: f32) -> Self {
        Self {
            width,
            dash: 1.0,
            gap: 0.0,
        }
    }

    pub const fn dashed(width: f32, dash: f32, gap: f32) -> Self {
        Self { width, dash, gap }
    }
}

/// Which colour of the style a line has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Line,
    Cycle,
    Crossing,
}

/// A regional marking style (spec §14 Phase 8 T5; `assets/road-styles.toml`
/// in the renderer): line widths, dash lengths and colours. Colours are
/// linear RGB.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkingStyle {
    pub id: String,
    pub time_zones: Vec<String>,
    /// Centre line outside towns and in towns.
    pub centre: LineSpec,
    pub centre_town: LineSpec,
    /// Distance between the two lines of a double or mixed centre line
    /// (centre to centre).
    pub double_gap_m: f32,
    /// Lines between lanes of one direction.
    pub lane: LineSpec,
    pub lane_town: LineSpec,
    /// Edge line width and how far its outer side stays inside the
    /// carriageway edge.
    pub edge_width_m: f32,
    pub edge_inset_m: f32,
    /// Width of a cycle lane and its line (advisory dashed, exclusive
    /// solid); `cycle_default` is what an untyped lane is.
    pub cycle_lane_m: f32,
    pub cycle_advisory: LineSpec,
    pub cycle_exclusive_m: f32,
    pub cycle_default: CycleLane,
    /// Zebra bars (width = bar, dash = gap between bars), the crossing's
    /// length along the road.
    pub zebra_bar_m: f32,
    pub zebra_gap_m: f32,
    pub zebra_length_m: f32,
    /// Stop line width; give-way line; the dashed lines of a signalled
    /// crossing and their distance; the stop lines' distance before them.
    pub stop_m: f32,
    pub give_way: LineSpec,
    pub signals: LineSpec,
    pub signals_spacing_m: f32,
    pub stop_before_m: f32,
    pub line_colour: [f32; 3],
    pub cycle_colour: [f32; 3],
    pub crossing_colour: [f32; 3],
}

impl Default for MarkingStyle {
    /// German style (RMS): narrow 12 cm lines, dashes 6 m / gaps 12 m
    /// outside towns and 3 m / 6 m inside, white.
    fn default() -> Self {
        let white = [0.75, 0.75, 0.72];
        Self {
            id: "de".into(),
            time_zones: Vec::new(),
            centre: LineSpec::dashed(0.12, 6.0, 12.0),
            centre_town: LineSpec::dashed(0.12, 3.0, 6.0),
            double_gap_m: 0.3,
            lane: LineSpec::dashed(0.12, 6.0, 12.0),
            lane_town: LineSpec::dashed(0.12, 3.0, 6.0),
            edge_width_m: 0.15,
            edge_inset_m: 0.25,
            cycle_lane_m: 1.5,
            cycle_advisory: LineSpec::dashed(0.12, 1.0, 1.0),
            cycle_exclusive_m: 0.25,
            cycle_default: CycleLane::Exclusive,
            zebra_bar_m: 0.5,
            zebra_gap_m: 0.5,
            zebra_length_m: 4.0,
            stop_m: 0.5,
            give_way: LineSpec::dashed(0.5, 0.5, 0.25),
            signals: LineSpec::dashed(0.25, 0.5, 0.2),
            signals_spacing_m: 4.0,
            stop_before_m: 1.0,
            line_colour: white,
            cycle_colour: white,
            crossing_colour: white,
        }
    }
}

impl MarkingStyle {
    /// The overlay length along the road of a transverse marking (m).
    pub fn crossing_length_m(&self, kind: CrossingKind) -> f32 {
        match kind {
            CrossingKind::Zebra => self.zebra_length_m,
            CrossingKind::Signals => {
                self.signals_spacing_m
                    + self.signals.width
                    + 2.0 * (self.stop_before_m + self.stop_m)
            }
            CrossingKind::Stop => self.stop_m,
            CrossingKind::GiveWay => self.give_way.width,
        }
    }
}

/// A longitudinal line of the layout: centre across the road (m, + right),
/// its spec and colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub at: f32,
    pub spec: LineSpec,
    pub paint: Paint,
}

/// The longitudinal lines of a strip `half` metres half wide (the layout
/// the shader draws; spec §14 Phase 8 T5).
pub fn marking_lines(code: &MarkCode, half: f32, style: &MarkingStyle) -> Vec<Line> {
    let mut out = Vec::new();
    let line = |at, spec| Line {
        at,
        spec,
        paint: Paint::Line,
    };
    let cycle_kind = |c: CycleLane| match c {
        CycleLane::Lane => style.cycle_default,
        c => c,
    };
    // Cycle lanes along the edges, then the lanes for motor traffic.
    let mut lo = -half;
    let mut hi = half;
    for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let spec = match cycle_kind(code.cycle[i]) {
            CycleLane::None | CycleLane::Lane => None,
            CycleLane::Advisory => Some(style.cycle_advisory),
            CycleLane::Exclusive => Some(LineSpec::solid(style.cycle_exclusive_m)),
        };
        if let Some(spec) = spec.filter(|_| half > style.cycle_lane_m + 1.0) {
            let at = side * (half - style.cycle_lane_m);
            out.push(Line {
                at,
                spec,
                paint: Paint::Cycle,
            });
            if side < 0.0 {
                lo = at;
            } else {
                hi = at;
            }
        } else if code.edge_lines && !code.town {
            let at = side * (half - style.edge_inset_m - style.edge_width_m / 2.0);
            out.push(line(at, LineSpec::solid(style.edge_width_m)));
        }
    }
    let [f, b] = code.lanes;
    let (centre, lane) = if code.town {
        (style.centre_town, style.lane_town)
    } else {
        (style.centre, style.lane)
    };
    // The centre between the directions, in proportion to their lanes.
    let c = if f > 0 && b > 0 {
        lo + (hi - lo) * f32::from(b) / f32::from(f + b)
    } else {
        lo
    };
    let g = style.double_gap_m / 2.0;
    let solid = LineSpec::solid(centre.width);
    match code.centre {
        CentreLine::None => {}
        CentreLine::Dashed => out.push(line(c, centre)),
        CentreLine::Solid => out.push(line(c, solid)),
        CentreLine::Double => {
            out.push(line(c - g, solid));
            out.push(line(c + g, solid));
        }
        CentreLine::SolidForward => {
            out.push(line(c - g, centre));
            out.push(line(c + g, solid));
        }
        CentreLine::SolidBackward => {
            out.push(line(c - g, solid));
            out.push(line(c + g, centre));
        }
    }
    if code.lane_lines {
        // Backward lanes from lo to c, forward lanes from c to hi (all of
        // lo … hi on a one-way road).
        let groups = if b == 0 {
            [(lo, hi, f), (0.0, 0.0, 0)]
        } else if f == 0 {
            [(lo, hi, b), (0.0, 0.0, 0)]
        } else {
            [(lo, c, b), (c, hi, f)]
        };
        for (a, z, n) in groups {
            for k in 1..n {
                out.push(line(a + (z - a) * f32::from(k) / f32::from(n), lane));
            }
        }
    }
    out
}

/// Whether a dashed pattern starting at 0 is painted at `v` (m).
pub fn dash_on(spec: &LineSpec, v: f32) -> bool {
    if spec.gap <= 0.0 {
        return true;
    }
    let p = spec.dash + spec.gap;
    v.rem_euclid(p) < spec.dash
}

/// The paint at (u across, v along the edge) of a strip (point sample; the
/// shader filters the same layout over each pixel).
pub fn paint_at(code: &MarkCode, half: f32, u: f32, v: f32, style: &MarkingStyle) -> Option<Paint> {
    marking_lines(code, half, style)
        .into_iter()
        .find(|l| (u - l.at).abs() <= l.spec.width / 2.0 && dash_on(&l.spec, v))
        .map(|l| l.paint)
}

/// The paint of a transverse marking at (u across, t along its overlay,
/// 0 … [`MarkingStyle::crossing_length_m`]) on a road `half` metres half
/// wide.
pub fn crossing_paint_at(
    kind: CrossingKind,
    half_of: Half,
    half: f32,
    u: f32,
    t: f32,
    style: &MarkingStyle,
) -> Option<Paint> {
    let in_half = |h: Half| match h {
        Half::Both => u.abs() <= half,
        Half::Forward => (0.0..=half).contains(&u),
        Half::Backward => (-half..=0.0).contains(&u),
    };
    let band = |t0: f32, w: f32| (t0..=t0 + w).contains(&t);
    match kind {
        CrossingKind::Zebra => {
            // Bars along the road, centred on the road's axis.
            let spec = LineSpec::dashed(style.zebra_bar_m, style.zebra_bar_m, style.zebra_gap_m);
            let p = spec.dash + spec.gap;
            let inside = u.abs() <= half - 0.2 && (0.0..=style.zebra_length_m).contains(&t);
            (inside && dash_on(&spec, u + style.zebra_bar_m / 2.0 + p * 64.0))
                .then_some(Paint::Crossing)
        }
        CrossingKind::Signals => {
            let s = style.stop_before_m + style.stop_m;
            let lines = band(s, style.signals.width)
                || band(s + style.signals_spacing_m, style.signals.width);
            if lines && u.abs() <= half && dash_on(&style.signals, u + half) {
                return Some(Paint::Line);
            }
            // Stop lines: forward traffic first meets t = 0, backward
            // traffic the far end.
            let fwd = band(0.0, style.stop_m) && in_half(Half::Forward);
            let end = s + style.signals_spacing_m + style.signals.width + style.stop_before_m;
            let bwd = band(end, style.stop_m) && in_half(Half::Backward);
            (fwd || bwd).then_some(Paint::Line)
        }
        CrossingKind::Stop => (band(0.0, style.stop_m) && in_half(half_of)).then_some(Paint::Line),
        CrossingKind::GiveWay => (band(0.0, style.give_way.width)
            && in_half(half_of)
            && dash_on(&style.give_way, u + half))
        .then_some(Paint::Line),
    }
}

// --- Wear (the shader's twin) -------------------------------------------------

/// Cell length of the asphalt patches along the road (m).
pub const PATCH_CELL_M: f32 = 8.0;

/// A small integer hash (the shader's `hash2`).
pub fn hash2(a: i32, b: i32) -> u32 {
    let mut h = (a as u32)
        .wrapping_mul(0x8da6_b343)
        .wrapping_add((b as u32).wrapping_mul(0xd816_3841))
        .wrapping_add(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

/// The hash as a number in 0 … 1.
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Whether (u, v) lies in a repaired patch of asphalt, for a patch chance
/// per cell and half (deterministic per position; the shader's twin).
pub fn patch_at(half: f32, u: f32, v: f32, chance: f32) -> bool {
    let k = (v / PATCH_CELL_M).floor() as i32;
    let side = i32::from(u >= 0.0);
    let h = hash2(k, side);
    if unit(h) >= chance {
        return false;
    }
    let start = k as f32 * PATCH_CELL_M + unit(hash2(k, side + 7)) * 3.0;
    let len = 1.5 + unit(hash2(k, side + 13)) * 4.0;
    let w = half * (0.45 + 0.5 * unit(hash2(k, side + 29)));
    (start..start + len).contains(&v) && u.abs() <= w
}

// --- Package section ----------------------------------------------------------

/// Decimetres in a byte (0 … 25.5 m).
fn dm(m: f32) -> u8 {
    (m * 10.0).round().clamp(0.0, 255.0) as u8
}

/// Writes the section payload: tiles (in `tiles` order) with a strip whose
/// dress differs from its class default.
pub fn write_road_dress<'a>(
    tiles: impl Iterator<Item = (TileKey, &'a [crate::RoadStrip])>,
    b: &mut Vec<u8>,
) -> bool {
    let mut payload = Vec::new();
    let mut count = 0u32;
    for (key, strips) in tiles {
        if strips
            .iter()
            .all(|s| s.dress == RoadDress::of_class(s.class, s.width_m))
        {
            continue;
        }
        count += 1;
        payload.extend(key.x.to_le_bytes());
        payload.extend(key.z.to_le_bytes());
        payload.extend((strips.len() as u32).to_le_bytes());
        for s in strips {
            let d = &s.dress;
            payload.extend(d.start_m.to_le_bytes());
            payload.extend(d.edge_m.to_le_bytes());
            payload.push(d.lanes[0].min(15) | d.lanes[1].min(15) << 4);
            payload.push(
                d.centre as u8
                    | u8::from(d.edge_lines) << 3
                    | u8::from(d.lane_lines) << 4
                    | u8::from(d.sidewalk_asphalt) << 5,
            );
            payload.push(d.cycle[0] as u8 | (d.cycle[1] as u8) << 2);
            payload.push((d.kerb_m * 100.0).round().clamp(0.0, 255.0) as u8);
            payload.push(dm(d.sidewalk_m[0]));
            payload.push(dm(d.sidewalk_m[1]));
            payload.push(dm(d.junction_m[0]));
            payload.push(dm(d.junction_m[1]));
            payload.push(d.crossings.len().min(255) as u8);
            for c in d.crossings.iter().take(255) {
                payload.extend(c.along_m.to_le_bytes());
                payload.push(c.kind as u8 | (c.half as u8) << 4);
            }
        }
    }
    if count == 0 {
        return false;
    }
    b.push(DRESS_VERSION);
    b.extend(count.to_le_bytes());
    b.extend(payload);
    true
}

/// Reads the section payload: per tile, the dress of each strip. A newer
/// layout version is "not supported" (the package reader skips it).
pub fn read_road_dress(bytes: &[u8]) -> Result<Vec<(TileKey, Vec<RoadDress>)>, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u8()?;
    if version != DRESS_VERSION {
        return Err(format!("road dress version {version} not supported"));
    }
    let n = r.u32()?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let strips = r.u32()? as usize;
        let mut v = Vec::with_capacity(strips.min(1 << 16));
        for _ in 0..strips {
            let start_m = r.f32()?;
            let edge_m = r.f32()?;
            let lanes = r.u8()?;
            let flags = r.u8()?;
            let cycle = r.u8()?;
            let kerb = r.u8()?;
            let [sl, sr, jf, jt, nc] = r.arr::<5>()?;
            let mut crossings = Vec::with_capacity(usize::from(nc));
            for _ in 0..nc {
                let along_m = r.f32()?;
                let k = r.u8()?;
                crossings.push(Crossing {
                    along_m,
                    kind: *CrossingKind::ALL
                        .get(usize::from(k & 15))
                        .ok_or_else(|| format!("unknown crossing kind {}", k & 15))?,
                    half: *Half::ALL
                        .get(usize::from(k >> 4))
                        .ok_or_else(|| format!("unknown crossing half {}", k >> 4))?,
                });
            }
            v.push(RoadDress {
                start_m,
                edge_m,
                lanes: [lanes & 15, lanes >> 4],
                centre: CentreLine::from_code(flags & 7)
                    .ok_or_else(|| format!("unknown centre line {}", flags & 7))?,
                edge_lines: flags >> 3 & 1 == 1,
                lane_lines: flags >> 4 & 1 == 1,
                sidewalk_asphalt: flags >> 5 & 1 == 1,
                cycle: [
                    CycleLane::ALL[usize::from(cycle & 3)],
                    CycleLane::ALL[usize::from(cycle >> 2 & 3)],
                ],
                sidewalk_m: [f32::from(sl) / 10.0, f32::from(sr) / 10.0],
                kerb_m: f32::from(kerb) / 100.0,
                junction_m: [f32::from(jf) / 10.0, f32::from(jt) / 10.0],
                crossings,
            });
        }
        out.push((key, v));
    }
    r.finish("road dress section")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> + Copy {
        move |k| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| *v)
    }

    fn osm(class: RoadClass, w: f32, pairs: &[(&str, &str)]) -> RoadDress {
        RoadDress::from_osm(class, w, Oneway::No, tags(pairs))
    }

    #[test]
    fn class_defaults_mark_main_roads_only() {
        let p = RoadDress::of_class(RoadClass::Primary, 7.5);
        assert_eq!(
            (p.lanes, p.centre, p.edge_lines),
            ([1, 1], CentreLine::Dashed, true)
        );
        let t = RoadDress::of_class(RoadClass::Tertiary, 6.0);
        assert_eq!((t.centre, t.edge_lines), (CentreLine::Dashed, false));
        // Narrow country lanes and town streets are unmarked.
        for (c, w) in [
            (RoadClass::Unclassified, 5.0),
            (RoadClass::Residential, 5.0),
            (RoadClass::Service, 3.5),
            (RoadClass::Track, 3.0),
            (RoadClass::Cycleway, 2.5),
            (RoadClass::Path, 1.8),
        ] {
            let d = RoadDress::of_class(c, w);
            assert_eq!(d.centre, CentreLine::None, "{c:?}");
            assert!(!d.edge_lines && !d.town(), "{c:?}");
        }
        // Untagged ways get the class default.
        assert_eq!(
            osm(RoadClass::Secondary, 6.5, &[]),
            RoadDress::of_class(RoadClass::Secondary, 6.5)
        );
    }

    #[test]
    fn lanes_and_overtaking_shape_the_centre_line() {
        let four = osm(RoadClass::Primary, 14.0, &[("lanes", "4")]);
        assert_eq!(
            (four.lanes, four.centre, four.lane_lines),
            ([2, 2], CentreLine::Double, true)
        );
        let three = osm(
            RoadClass::Primary,
            10.0,
            &[("lanes", "3"), ("lanes:forward", "2")],
        );
        assert_eq!((three.lanes, three.centre), ([2, 1], CentreLine::Dashed));
        assert_eq!(
            osm(RoadClass::Secondary, 7.0, &[("overtaking", "no")]).centre,
            CentreLine::Solid
        );
        assert_eq!(
            osm(RoadClass::Secondary, 7.0, &[("overtaking:forward", "no")]).centre,
            CentreLine::SolidForward
        );
        assert_eq!(
            osm(RoadClass::Secondary, 7.0, &[("overtaking:backward", "no")]).centre,
            CentreLine::SolidBackward
        );
        // lane_markings=no wins; a residential street with markings tagged.
        let none = osm(RoadClass::Primary, 7.0, &[("lane_markings", "no")]);
        assert_eq!((none.centre, none.edge_lines), (CentreLine::None, false));
        assert_eq!(
            osm(
                RoadClass::Residential,
                6.0,
                &[("lane_markings", "yes"), ("lanes", "2")]
            )
            .centre,
            CentreLine::Dashed
        );
        // One-way: all lanes forward (or backward), no centre line.
        let one = RoadDress::from_osm(
            RoadClass::Primary,
            7.0,
            Oneway::Forward,
            tags(&[("lanes", "2")]),
        );
        assert_eq!(
            (one.lanes, one.centre, one.lane_lines),
            ([2, 0], CentreLine::None, true)
        );
        let back = RoadDress::from_osm(RoadClass::Primary, 7.0, Oneway::Backward, tags(&[]));
        assert_eq!(back.lanes, [0, 1]);
        // A single lane for both directions: no centre line.
        assert_eq!(
            osm(RoadClass::Tertiary, 4.0, &[("lanes", "1")]).centre,
            CentreLine::None
        );
    }

    #[test]
    fn sidewalk_kerb_and_cycle_lane_tags() {
        assert_eq!(
            sidewalk_tags(tags(&[("sidewalk", "both")])),
            [SidewalkTag::Yes(DEFAULT_SIDEWALK_M); 2]
        );
        assert_eq!(
            sidewalk_tags(tags(&[("sidewalk", "right"), ("sidewalk:width", "3")])),
            [SidewalkTag::No, SidewalkTag::Yes(3.0)]
        );
        assert_eq!(
            sidewalk_tags(tags(&[
                ("sidewalk:left", "separate"),
                ("sidewalk:right", "no")
            ])),
            [SidewalkTag::Separate, SidewalkTag::No]
        );
        assert_eq!(
            sidewalk_tags(tags(&[
                ("sidewalk:both", "yes"),
                ("sidewalk:left:width", "1,5 m")
            ])),
            [SidewalkTag::Yes(1.5), SidewalkTag::Yes(DEFAULT_SIDEWALK_M)]
        );
        assert_eq!(sidewalk_tags(tags(&[])), [SidewalkTag::Unknown; 2]);
        let d = osm(RoadClass::Residential, 6.0, &[("sidewalk", "left")]);
        assert_eq!(d.sidewalk_m, [DEFAULT_SIDEWALK_M, 0.0]);
        assert!(d.town() && !d.sidewalk_asphalt);
        let a = osm(
            RoadClass::Residential,
            6.0,
            &[("sidewalk", "both"), ("sidewalk:both:surface", "asphalt")],
        );
        assert!(a.sidewalk_asphalt);
        assert_eq!(kerb_height(tags(&[])), DEFAULT_KERB_M);
        assert_eq!(kerb_height(tags(&[("kerb", "lowered")])), LOWERED_KERB_M);
        assert_eq!(kerb_height(tags(&[("sidewalk:both:kerb", "flush")])), 0.0);
        // Paths get no sidewalks even when tagged.
        assert_eq!(
            osm(RoadClass::Path, 2.0, &[("sidewalk", "both")]).sidewalk_m,
            [0.0; 2]
        );
        // Cycle lanes: both sides, one side, kinds, one-way roads.
        let c = |o, p: &[(&str, &str)]| cycle_lanes(o, tags(p));
        assert_eq!(c(Oneway::No, &[("cycleway", "lane")]), [CycleLane::Lane; 2]);
        assert_eq!(
            c(
                Oneway::No,
                &[
                    ("cycleway:right", "lane"),
                    ("cycleway:right:lane", "advisory")
                ]
            ),
            [CycleLane::None, CycleLane::Advisory]
        );
        assert_eq!(
            c(
                Oneway::No,
                &[
                    ("cycleway:both", "lane"),
                    ("cycleway:both:lane", "exclusive")
                ]
            ),
            [CycleLane::Exclusive; 2]
        );
        assert_eq!(
            c(Oneway::Forward, &[("cycleway", "lane")]),
            [CycleLane::None, CycleLane::Lane]
        );
        assert_eq!(
            c(Oneway::Forward, &[("cycleway", "opposite_lane")]),
            [CycleLane::Lane, CycleLane::None]
        );
        assert_eq!(
            c(Oneway::No, &[("cycleway", "track")]),
            [CycleLane::None; 2]
        );
    }

    #[test]
    fn crossing_nodes() {
        let k = |p: &[(&str, &str)]| CrossingKind::from_osm(tags(p));
        assert_eq!(
            k(&[("highway", "crossing"), ("crossing", "zebra")]),
            Some(CrossingKind::Zebra)
        );
        assert_eq!(
            k(&[("highway", "crossing"), ("crossing", "marked")]),
            Some(CrossingKind::Zebra)
        );
        assert_eq!(
            k(&[
                ("highway", "crossing"),
                ("crossing", "uncontrolled"),
                ("crossing_ref", "zebra")
            ]),
            Some(CrossingKind::Zebra)
        );
        assert_eq!(
            k(&[("highway", "crossing"), ("crossing:markings", "zebra")]),
            Some(CrossingKind::Zebra)
        );
        assert_eq!(
            k(&[("highway", "crossing"), ("crossing", "traffic_signals")]),
            Some(CrossingKind::Signals)
        );
        assert_eq!(
            k(&[("highway", "crossing"), ("crossing", "unmarked")]),
            None
        );
        assert_eq!(
            k(&[("crossing", "uncontrolled"), ("crossing:markings", "no")]),
            None
        );
        assert_eq!(k(&[("highway", "crossing")]), None);
        assert_eq!(
            k(&[
                ("crossing", "traffic_signals"),
                ("crossing:markings", "zebra")
            ]),
            Some(CrossingKind::Zebra)
        );
        assert_eq!(k(&[("highway", "stop")]), Some(CrossingKind::Stop));
        assert_eq!(k(&[("highway", "give_way")]), Some(CrossingKind::GiveWay));
        assert_eq!(k(&[("highway", "traffic_signals")]), None);
    }

    fn code(lanes: [u8; 2], centre: CentreLine) -> MarkCode {
        MarkCode {
            lanes,
            centre,
            ..Default::default()
        }
    }

    #[test]
    fn mark_codes_round_trip_and_follow_the_surface() {
        let d = RoadDress {
            lanes: [2, 1],
            centre: CentreLine::SolidBackward,
            edge_lines: true,
            lane_lines: true,
            cycle: [CycleLane::Advisory, CycleLane::Exclusive],
            sidewalk_m: [2.0, 0.0],
            ..RoadDress::of_class(RoadClass::Primary, 9.0)
        };
        let c = MarkCode::of(&d, RoadClass::Primary, RoadSurface::Asphalt);
        assert!(c.lines_code() < 4096 && c.extra_code() < 32);
        assert_eq!(MarkCode::decode(c.lines_code(), c.extra_code()), c);
        // No paint on cobbles or gravel; the track look only on tracks.
        let cobbles = MarkCode::of(&d, RoadClass::Primary, RoadSurface::Paving);
        assert_eq!(cobbles.centre, CentreLine::None);
        assert!(!cobbles.edge_lines && !cobbles.lane_lines);
        assert_eq!(cobbles.cycle, [CycleLane::None; 2]);
        let track = MarkCode::of(
            &RoadDress::of_class(RoadClass::Track, 3.0),
            RoadClass::Track,
            RoadSurface::Gravel,
        );
        assert!(track.track && MarkCode::decode(track.lines_code(), track.extra_code()).track);
    }

    #[test]
    fn marking_layout_per_class_and_lanes() {
        let s = MarkingStyle::default();
        // Two lanes: a dashed centre line in the middle.
        let two = marking_lines(&code([1, 1], CentreLine::Dashed), 3.0, &s);
        assert_eq!(two.len(), 1);
        assert_eq!(two[0].at, 0.0);
        assert_eq!(two[0].spec, s.centre);
        // With edge lines (country road): inside both edges.
        let mut c = code([1, 1], CentreLine::Dashed);
        c.edge_lines = true;
        let l = marking_lines(&c, 3.5, &s);
        assert_eq!(l.len(), 3);
        let edge = 3.5 - s.edge_inset_m - s.edge_width_m / 2.0;
        assert!(
            l.iter()
                .any(|x| (x.at - edge).abs() < 1e-6 && x.spec.gap == 0.0)
        );
        assert!(l.iter().any(|x| (x.at + edge).abs() < 1e-6));
        // In town the edge lines give way to the kerbs, dashes are short.
        c.town = true;
        let t = marking_lines(&c, 3.5, &s);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].spec, s.centre_town);
        // Four lanes: a double centre line and one lane line each way.
        let mut four = code([2, 2], CentreLine::Double);
        four.lane_lines = true;
        let f = marking_lines(&four, 7.0, &s);
        let mut at: Vec<f32> = f.iter().map(|x| x.at).collect();
        at.sort_by(f32::total_cmp);
        assert_eq!(at, vec![-3.5, -0.15, 0.15, 3.5]);
        // Three lanes, two of them forward: the centre line sits left of
        // the middle, one third across.
        let mut three = code([2, 1], CentreLine::Dashed);
        three.lane_lines = true;
        let m = marking_lines(&three, 4.5, &s);
        assert!(
            m.iter()
                .any(|x| (x.at - (-1.5)).abs() < 1e-5 && x.spec == s.centre)
        );
        assert!(
            m.iter()
                .any(|x| (x.at - 1.5).abs() < 1e-5 && x.spec == s.lane)
        );
        // A mixed line: solid on the forward side.
        let mixed = marking_lines(&code([1, 1], CentreLine::SolidForward), 3.0, &s);
        let solid = mixed.iter().find(|x| x.spec.gap == 0.0).unwrap();
        assert!(solid.at > 0.0);
        // Nothing painted on an unmarked street.
        assert!(marking_lines(&MarkCode::default(), 2.5, &s).is_empty());
    }

    #[test]
    fn cycle_lanes_replace_the_edge_lines_and_narrow_the_lanes() {
        let s = MarkingStyle::default();
        let mut c = code([1, 1], CentreLine::Dashed);
        c.edge_lines = true;
        c.cycle = [CycleLane::Lane, CycleLane::Advisory];
        let l = marking_lines(&c, 4.5, &s);
        let cyc: Vec<&Line> = l.iter().filter(|x| x.paint == Paint::Cycle).collect();
        assert_eq!(cyc.len(), 2);
        // The untyped lane takes the style's default (exclusive: solid).
        let left = cyc.iter().find(|x| x.at < 0.0).unwrap();
        assert_eq!(left.spec.gap, 0.0);
        assert!((left.at + 3.0).abs() < 1e-6);
        let right = cyc.iter().find(|x| x.at > 0.0).unwrap();
        assert_eq!(right.spec, s.cycle_advisory);
        // No edge lines beside cycle lanes; the centre between them.
        assert_eq!(l.len(), 3);
        assert!(l.iter().any(|x| x.at == 0.0 && x.paint == Paint::Line));
        // Point samples: paint on the centre dash, asphalt beside it.
        assert_eq!(paint_at(&c, 4.5, 0.0, 1.0, &s), Some(Paint::Line));
        assert_eq!(paint_at(&c, 4.5, 0.0, 10.0, &s), None, "in the gap");
        assert_eq!(paint_at(&c, 4.5, 3.0, 0.5, &s), Some(Paint::Cycle));
        assert_eq!(paint_at(&c, 4.5, 3.0, 1.5, &s), None, "advisory gap");
        assert_eq!(paint_at(&c, 4.5, 1.0, 1.0, &s), None);
    }

    #[test]
    fn crossings_are_painted_across_the_road() {
        let s = MarkingStyle::default();
        let z = |u, t| crossing_paint_at(CrossingKind::Zebra, Half::Both, 3.5, u, t, &s);
        // Bars centred on the axis, 0.5 m wide with 0.5 m gaps.
        assert_eq!(z(0.0, 2.0), Some(Paint::Crossing));
        assert_eq!(z(0.5, 2.0), None);
        assert_eq!(z(1.0, 2.0), Some(Paint::Crossing));
        assert_eq!(z(-1.0, 2.0), Some(Paint::Crossing));
        assert_eq!(z(0.0, 4.5), None, "beyond its length");
        assert_eq!(z(3.45, 2.0), None, "kept off the edge");
        // A stop line covers its half only.
        let stop = |u| crossing_paint_at(CrossingKind::Stop, Half::Forward, 3.0, u, 0.2, &s);
        assert_eq!(stop(1.0), Some(Paint::Line));
        assert_eq!(stop(-1.0), None);
        // Signals: stop lines for both directions on their halves.
        let len = s.crossing_length_m(CrossingKind::Signals);
        let sig = |u, t| crossing_paint_at(CrossingKind::Signals, Half::Both, 3.0, u, t, &s);
        assert_eq!(sig(1.0, 0.2), Some(Paint::Line));
        assert_eq!(sig(-1.0, 0.2), None);
        assert_eq!(sig(-1.0, len - 0.2), Some(Paint::Line));
        assert_eq!(sig(1.0, len - 0.2), None);
    }

    #[test]
    fn kerbs_taper_into_junction_zones() {
        let d = RoadDress {
            start_m: 10.0,
            edge_m: 100.0,
            sidewalk_m: [2.0, 0.0],
            junction_m: [12.0, 8.0],
            ..RoadDress::of_class(RoadClass::Residential, 6.0)
        };
        // Edge metre 12 is strip metre 2: nothing there, full 2 m later.
        assert_eq!(d.kerb_at(0, 1.0), (0.0, 0.0));
        assert_eq!(d.kerb_at(0, 2.0), (0.0, 0.0));
        let (w, k) = d.kerb_at(0, 3.0);
        assert!((w - 1.0).abs() < 1e-5 && (k - DEFAULT_KERB_M / 2.0).abs() < 1e-5);
        assert_eq!(d.kerb_at(0, 40.0), (2.0, DEFAULT_KERB_M));
        // The end zone starts at edge metre 92 = strip metre 82.
        assert_eq!(d.kerb_at(0, 82.0).0, 0.0);
        assert!(d.kerb_at(0, 79.0).0 > 1.9);
        assert_eq!(d.kerb_at(1, 40.0), (0.0, 0.0), "no sidewalk on the right");
        assert!(d.in_junction(1.0) && !d.in_junction(40.0) && d.in_junction(85.0));
        // Without junction zones: full everywhere.
        let free = RoadDress {
            junction_m: [0.0; 2],
            ..d.clone()
        };
        assert_eq!(free.kerb_at(0, 0.0), (2.0, DEFAULT_KERB_M));
    }

    #[test]
    fn patches_are_deterministic_and_rare() {
        let mut hits = 0;
        let mut n = 0;
        for i in 0..4000 {
            let v = i as f32 * 0.5;
            for u in [-2.0f32, -0.5, 0.5, 2.0] {
                n += 1;
                let p = patch_at(3.0, u, v, 0.15);
                assert_eq!(p, patch_at(3.0, u, v, 0.15));
                hits += usize::from(p);
            }
        }
        let share = hits as f32 / n as f32;
        assert!((0.01..0.12).contains(&share), "{share}");
        assert!((0..2000).all(|i| !patch_at(3.0, 0.5, i as f32, 0.0)));
    }

    #[test]
    fn section_round_trip_and_defaults_are_not_written() {
        use crate::RoadStrip;
        let strip = |class, dress| RoadStrip {
            class,
            width_m: 6.0,
            points: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            surface: RoadSurface::Asphalt,
            dress,
            lit: crate::RoadLit::Unknown,
        };
        let plain = vec![strip(
            RoadClass::Tertiary,
            RoadDress::of_class(RoadClass::Tertiary, 6.0),
        )];
        let mut b = Vec::new();
        assert!(!write_road_dress(
            [(TileKey { x: 0, z: 0 }, plain.as_slice())].into_iter(),
            &mut b
        ));
        assert!(b.is_empty());
        let dressed = RoadDress {
            start_m: 123.5,
            edge_m: 812.25,
            lanes: [2, 1],
            centre: CentreLine::SolidForward,
            edge_lines: true,
            lane_lines: true,
            cycle: [CycleLane::Advisory, CycleLane::Lane],
            sidewalk_m: [2.5, 1.2],
            kerb_m: 0.04,
            sidewalk_asphalt: true,
            junction_m: [7.5, 12.3],
            crossings: vec![
                Crossing {
                    along_m: 4.25,
                    kind: CrossingKind::Zebra,
                    half: Half::Both,
                },
                Crossing {
                    along_m: 9.0,
                    kind: CrossingKind::GiveWay,
                    half: Half::Backward,
                },
            ],
        };
        let tile = vec![
            strip(
                RoadClass::Tertiary,
                RoadDress::of_class(RoadClass::Tertiary, 6.0),
            ),
            strip(RoadClass::Residential, dressed.clone()),
        ];
        let key = TileKey { x: -3, z: 7 };
        assert!(write_road_dress(
            [(key, tile.as_slice())].into_iter(),
            &mut b
        ));
        let back = read_road_dress(&b).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, key);
        assert_eq!(back[0].1[0], tile[0].dress);
        assert_eq!(back[0].1[1], dressed);
        // A newer layout is refused as "not supported" (the reader skips it).
        let mut newer = b.clone();
        newer[0] = 99;
        assert!(
            read_road_dress(&newer)
                .unwrap_err()
                .contains("not supported")
        );
        assert!(read_road_dress(&b[..b.len() - 1]).is_err());
    }
}
