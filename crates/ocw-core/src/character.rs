//! The rider's character (spec §2.6, §14 Phase 4, 4C; milestone W4;
//! refined by 4N): body (height, base body, shape), appearance age, skin
//! tone, face, hair, facial hair and glasses. Free choices – "who someone
//! is never costs anything" (Phase 4 Q4) – and never physics: the kit
//! (helmet, shoes, gloves, clothing) is made of catalogue items with
//! prices and small effects, the character is only looks. Part of
//! [`crate::RiderLook`]; on the wire the character travels once per
//! change (`RiderBuild.look`), not in every snapshot.
//!
//! **The body (4N Q6, Q6b, Q7):** two independent dimensions – the
//! **base body** A / B / C (shoulders, hips, chest; a shape choice, not a
//! sex or gender field) and the **shape**, seven named steps from
//! *anorexia* to *obese*. **Every shape can be chosen whatever weight
//! and height say (Q23, replacing Q13's band):** the shape the profile's
//! weight and height give ([`weight_shape`]) is only a proposal – the
//! preselected shape of a new rider and the *Match my weight* marker,
//! never a limit. **The appearance age (Q12)** changes the face (lines, a softer
//! jaw), the body (posture, a fuller middle), the skin (less saturated)
//! and the hair (greying unless *Keep my hair colour*).

use crate::bike::Rgb;

/// Default body height (cm): the generated rider is 1.78 m tall.
pub const DEFAULT_HEIGHT_CM: u16 = 178;
/// The body heights the rider model is drawn at (cm; a profile's height
/// outside is clamped for drawing only).
pub const HEIGHT_RANGE_CM: (u16, u16) = (150, 205);

/// The body build of W4 (three steps): still sent on the wire
/// (`RiderLook.body_build`) for older apps, as the nearest of the seven
/// [`BodyShape`]s; read from older files and apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BodyBuild {
    Slim,
    #[default]
    Medium,
    Strong,
}

impl BodyBuild {
    pub const ALL: [BodyBuild; 3] = [Self::Slim, Self::Medium, Self::Strong];

    pub fn id(self) -> &'static str {
        match self {
            Self::Slim => "slim",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.id().eq_ignore_ascii_case(s.trim()))
    }

    pub fn wire(self) -> u32 {
        self as u32
    }

    /// Unknown numbers are the medium build.
    pub fn from_wire(n: u32) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }
}

/// The base body (4N N2, Q6b): the same rig, node names and anchors;
/// they differ in the shoulders, the hips (thighs) and the chest. Shown
/// with neutral captions; nothing else depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum BaseBody {
    /// Broader shoulders, narrower hips, a flat chest (today's model is
    /// closest to it: existing characters keep it).
    #[default]
    A,
    /// Narrower shoulders, wider hips, a chest.
    B,
    /// In between.
    C,
}

impl BaseBody {
    pub const ALL: [BaseBody; 3] = [Self::A, Self::B, Self::C];

    pub fn id(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
        }
    }

    /// "Body A".
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "Body A",
            Self::B => "Body B",
            Self::C => "Body C",
        }
    }

    /// What differs, for the editor's help line.
    pub fn about(self) -> &'static str {
        match self {
            Self::A => "broader shoulders, narrower hips, flat chest",
            Self::B => "narrower shoulders, wider hips, a chest",
            Self::C => "in between",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s
            .strip_prefix("body ")
            .or_else(|| s.strip_prefix("Body "))
            .unwrap_or(s);
        Self::ALL
            .into_iter()
            .find(|b| b.id().eq_ignore_ascii_case(s))
    }

    pub fn wire(self) -> u32 {
        self as u32
    }

    /// Unknown numbers are body A.
    pub fn from_wire(n: u32) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }
}

/// The body shape (4N Q6, Q7): seven named steps, each with its picture
/// in the editor; any of them for any rider (Q23), [`weight_shape`]
/// proposes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum BodyShape {
    Anorexia,
    Skinny,
    Slim,
    #[default]
    Normal,
    Ample,
    Fat,
    Obese,
}

impl BodyShape {
    pub const ALL: [BodyShape; 7] = [
        Self::Anorexia,
        Self::Skinny,
        Self::Slim,
        Self::Normal,
        Self::Ample,
        Self::Fat,
        Self::Obese,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Anorexia => "anorexia",
            Self::Skinny => "skinny",
            Self::Slim => "slim",
            Self::Normal => "normal",
            Self::Ample => "ample",
            Self::Fat => "fat",
            Self::Obese => "obese",
        }
    }

    /// The label (Q7: the seven words).
    pub fn name(self) -> &'static str {
        match self {
            Self::Anorexia => "Anorexia",
            Self::Skinny => "Skinny",
            Self::Slim => "Slim",
            Self::Normal => "Normal",
            Self::Ample => "Ample",
            Self::Fat => "Fat",
            Self::Obese => "Obese",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The shape at `i` (clamped to the seven).
    pub fn at(i: isize) -> Self {
        Self::ALL[i.clamp(0, Self::ALL.len() as isize - 1) as usize]
    }

    /// The id, or a W4 build's id (slim / medium / strong).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|b| b.id().eq_ignore_ascii_case(s))
            .or_else(|| BodyBuild::parse(s).map(Self::from_build))
    }

    pub fn wire(self) -> u32 {
        self as u32
    }

    pub fn from_wire(n: u32) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }

    /// The W4 build's shape (migration: slim / medium / strong → slim /
    /// normal / ample).
    pub fn from_build(b: BodyBuild) -> Self {
        match b {
            BodyBuild::Slim => Self::Slim,
            BodyBuild::Medium => Self::Normal,
            BodyBuild::Strong => Self::Ample,
        }
    }

    /// The nearest W4 build (sent for older apps).
    pub fn nearest_build(self) -> BodyBuild {
        match self {
            Self::Anorexia | Self::Skinny | Self::Slim => BodyBuild::Slim,
            Self::Normal => BodyBuild::Medium,
            Self::Ample | Self::Fat | Self::Obese => BodyBuild::Strong,
        }
    }

    /// The girth of the torso and pelvis, and of the limbs, against the
    /// normal shape (x and z of the rigid parts' meshes; the bellies of
    /// the heavier shapes grow more than the limbs).
    pub fn girth(self) -> (f32, f32) {
        match self {
            Self::Anorexia => (0.76, 0.72),
            Self::Skinny => (0.85, 0.83),
            Self::Slim => (0.92, 0.91),
            Self::Normal => (1.0, 1.0),
            Self::Ample => (1.11, 1.08),
            Self::Fat => (1.24, 1.16),
            Self::Obese => (1.38, 1.25),
        }
    }

    /// The shape of a body-mass index (kg / m²): the *Match my weight*
    /// marker (4N N4, Q13). Steps at 16, 18.5, 21, 25, 28 and 32.
    pub fn from_bmi(bmi: f64) -> Self {
        const STEPS: [f64; 6] = [16.0, 18.5, 21.0, 25.0, 28.0, 32.0];
        Self::ALL[STEPS.iter().filter(|s| bmi >= **s).count()]
    }
}

/// The body-mass index of `weight_kg` at `height_cm`.
pub fn bmi(weight_kg: f64, height_cm: f64) -> f64 {
    let m = height_cm.max(1.0) / 100.0;
    weight_kg / (m * m)
}

/// The *Match my weight* shape (4N Q13, Q23): what the profile's weight
/// and height give (no height: the default 1.78 m) – a proposal (a new
/// rider's preselected shape, the editor's marker), never a limit.
pub fn weight_shape(weight_kg: f64, height_cm: Option<f64>) -> BodyShape {
    BodyShape::from_bmi(bmi(
        weight_kg,
        height_cm.unwrap_or(f64::from(DEFAULT_HEIGHT_CM)),
    ))
}

/// The appearance age (4N N3): six steps, never a birth date or a number
/// shown to others, never physics or animation tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum Age {
    Twenties,
    #[default]
    Thirties,
    Forties,
    Fifties,
    Sixties,
    Seventies,
}

impl Age {
    pub const ALL: [Age; 6] = [
        Self::Twenties,
        Self::Thirties,
        Self::Forties,
        Self::Fifties,
        Self::Sixties,
        Self::Seventies,
    ];

    /// "20s" … "70+".
    pub fn id(self) -> &'static str {
        match self {
            Self::Twenties => "20s",
            Self::Thirties => "30s",
            Self::Forties => "40s",
            Self::Fifties => "50s",
            Self::Sixties => "60s",
            Self::Seventies => "70+",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = if s == "70s" { "70+" } else { s };
        Self::ALL
            .into_iter()
            .find(|a| a.id().eq_ignore_ascii_case(s))
    }

    pub fn wire(self) -> u32 {
        self as u32
    }

    /// Unknown numbers are the 30s.
    pub fn from_wire(n: u32) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }

    /// The hair's grey share at this age (0 none … 1 all grey), before
    /// the rider's own spread ([`Character::grey_share`]).
    pub fn grey(self) -> f32 {
        match self {
            Self::Twenties | Self::Thirties => 0.0,
            Self::Forties => 0.15,
            Self::Fifties => 0.35,
            Self::Sixties => 0.6,
            Self::Seventies => 0.82,
        }
    }

    /// How much the skin's saturation drops (0 … 1).
    pub fn skin_fade(self) -> f32 {
        match self {
            Self::Sixties => 0.05,
            Self::Seventies => 0.12,
            _ => 0.0,
        }
    }

    /// The face's lines at the eyes and mouth: 0 none, 1 light (40s,
    /// 50s), 2 deeper (60s, 70+).
    pub fn face_lines(self) -> u8 {
        match self {
            Self::Twenties | Self::Thirties => 0,
            Self::Forties | Self::Fifties => 1,
            Self::Sixties | Self::Seventies => 2,
        }
    }

    /// A softer jaw (from the 60s).
    pub fn soft_jaw(self) -> bool {
        self >= Self::Sixties
    }

    /// The body's middle, deeper with the years (z of the torso and
    /// pelvis), and the limbs' girth (a little less muscle at 70+).
    pub fn body(self) -> (f32, f32) {
        match self {
            Self::Twenties | Self::Thirties | Self::Forties => (1.0, 1.0),
            Self::Fifties => (1.03, 1.0),
            Self::Sixties => (1.05, 0.99),
            Self::Seventies => (1.06, 0.97),
        }
    }

    /// The idle stance's rounder shoulders (rad, forward; the workshop's
    /// standing pose only – on the bike the pose comes from the IK).
    pub fn idle_hunch(self) -> f32 {
        match self {
            Self::Sixties => 0.025,
            Self::Seventies => 0.035,
            _ => 0.0,
        }
    }
}

/// The skin-tone palette (spec §14 Phase 4, 4N; asked 2026-11-17: "add
/// skin tone selector to character editor"): 16 steps from very pale to
/// very dark (sRGB), evenly spread in lightness (CIE L* from about 92 to
/// 12, steps of 3.5 – 8.5) and alternating warm (red-pink) and neutral to
/// golden / olive undertones in the light, the medium and the dark range.
pub const SKIN_TONES: [Rgb; 16] = [
    Rgb(0xF7E3D6),
    Rgb(0xF1D5C1),
    Rgb(0xE9CBAD),
    Rgb(0xE1BA9C),
    Rgb(0xD5AE8A),
    Rgb(0xC89D7C),
    Rgb(0xBB8F68),
    Rgb(0xAC7D5C),
    Rgb(0x9C6E4E),
    Rgb(0x8C6646),
    Rgb(0x77513A),
    Rgb(0x66472F),
    Rgb(0x543828),
    Rgb(0x45301F),
    Rgb(0x37231A),
    Rgb(0x2A1C13),
];

/// The 12-step palette of W4 / 4C (all one warm undertone, L* 90 – 17):
/// what older apps and files mean by a skin index ([`skin_from_legacy`],
/// [`skin_to_legacy`]).
pub const LEGACY_SKIN_TONES: [Rgb; 12] = [
    Rgb(0xF6DDCB),
    Rgb(0xF0CDB4),
    Rgb(0xE6BC9E),
    Rgb(0xD9A98C),
    Rgb(0xC99477),
    Rgb(0xB98163),
    Rgb(0xA66F52),
    Rgb(0x915D43),
    Rgb(0x7A4C36),
    Rgb(0x643D2B),
    Rgb(0x4E3022),
    Rgb(0x3A2419),
];

/// CIE L*a*b* (D65) of an sRGB colour.
pub fn lab(c: Rgb) -> [f32; 3] {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = c.channels().map(lin);
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    [
        116.0 * f(y) - 16.0,
        500.0 * (f(x) - f(y)),
        200.0 * (f(y) - f(z)),
    ]
}

/// The index in `palette` nearest to `c` (in L*a*b*).
fn nearest(palette: &[Rgb], c: Rgb) -> u8 {
    let [l, a, b] = lab(c);
    let d = |p: &Rgb| {
        let [l2, a2, b2] = lab(*p);
        (l - l2).powi(2) + (a - a2).powi(2) + (b - b2).powi(2)
    };
    palette
        .iter()
        .enumerate()
        .min_by(|x, y| d(x.1).total_cmp(&d(y.1)))
        .map_or(0, |(i, _)| i as u8)
}

/// An index of the 12-step palette (older apps, `riders.toml` before
/// `skin_tone`) as the nearest tone of [`SKIN_TONES`].
pub fn skin_from_legacy(i: u8) -> u8 {
    let i = usize::from(i).min(LEGACY_SKIN_TONES.len() - 1);
    nearest(&SKIN_TONES, LEGACY_SKIN_TONES[i])
}

/// A tone of [`SKIN_TONES`] as the nearest of the 12-step palette (written
/// for older apps).
pub fn skin_to_legacy(i: u8) -> u8 {
    let i = usize::from(i).min(SKIN_TONES.len() - 1);
    nearest(&LEGACY_SKIN_TONES, SKIN_TONES[i])
}

/// Face shapes (the face's own presets; facial hair is its own choice
/// since 4N N6).
pub const FACES: [&str; 1] = ["Classic"];

/// The W4 faces, which held the facial hair (migrated, [`legacy_face`]).
pub const LEGACY_FACES: [&str; 4] = ["Classic", "Beard", "Moustache", "Stubble"];

/// Hair styles (generated meshes; "Bald": none). Today's six keep their
/// indices (wire and files), the 4N catalogue follows (N5); every style
/// is drawn under every helmet (tails out through the helmet's back).
pub const HAIR_STYLES: [&str; 21] = [
    "Short",
    "Buzz cut",
    "Long",
    "Ponytail",
    "Bun",
    "Bald",
    "Shaved",
    "Crew cut",
    "Short textured",
    "Side part",
    "Short curly",
    "Afro",
    "Pixie",
    "Mid-length",
    "Long curly",
    "Low ponytail",
    "Single braid",
    "Two braids",
    "Low bun",
    "Locs tied back",
    "Cornrows",
];

/// The hair style ids for files and the console (lower case, dashes).
pub fn hair_id(i: u8) -> String {
    HAIR_STYLES
        .get(i as usize)
        .unwrap_or(&HAIR_STYLES[0])
        .to_lowercase()
        .replace(' ', "-")
}

/// The hair style of an id or a name (or an index).
pub fn hair_parse(s: &str) -> Option<u8> {
    let k = s.trim().to_lowercase().replace([' ', '_'], "-");
    (0..HAIR_STYLES.len() as u8)
        .find(|i| hair_id(*i) == k)
        .or_else(|| {
            s.trim()
                .parse::<u8>()
                .ok()
                .filter(|i| (*i as usize) < HAIR_STYLES.len())
        })
}

/// The moustaches (4N N6).
pub const MOUSTACHES: [&str; 6] = [
    "None",
    "Classic",
    "Chevron",
    "Handlebar",
    "Horseshoe",
    "Pencil",
];

/// The beards (4N N6).
pub const BEARDS: [&str; 8] = [
    "None",
    "Stubble",
    "Short stubble beard",
    "Short boxed",
    "Full",
    "Goatee",
    "Chin strap",
    "Long",
];

/// An id of a list's entry ("Short boxed" → "short-boxed").
pub fn list_id(list: &[&str], i: u8) -> String {
    list.get(i as usize)
        .unwrap_or(&list[0])
        .to_lowercase()
        .replace(' ', "-")
}

/// The entry of `list` named by an id, a name or an index.
pub fn list_parse(list: &[&str], s: &str) -> Option<u8> {
    let k = s.trim().to_lowercase().replace([' ', '_'], "-");
    (0..list.len() as u8)
        .find(|i| list_id(list, *i) == k)
        .or_else(|| {
            s.trim()
                .parse::<u8>()
                .ok()
                .filter(|i| (*i as usize) < list.len())
        })
}

/// The natural hair palette (4N N5): black, dark brown, brown, light
/// brown, auburn, red, copper, strawberry blond, blond, platinum, grey,
/// white. Any `#RRGGBB` is a custom colour (dyed hair).
pub const HAIR_PALETTE: [(&str, Rgb); 12] = [
    ("Black", Rgb(0x15110F)),
    ("Dark brown", Rgb(0x3B2A20)),
    ("Brown", Rgb(0x6B4426)),
    ("Light brown", Rgb(0xA0703F)),
    ("Auburn", Rgb(0x7E3A24)),
    ("Red", Rgb(0xB3482A)),
    ("Copper", Rgb(0xC0682E)),
    ("Strawberry blond", Rgb(0xD39A6A)),
    ("Blond", Rgb(0xD8B27A)),
    ("Platinum", Rgb(0xE6DCC4)),
    ("Grey", Rgb(0x9C9A96)),
    ("White", Rgb(0xE8E4DC)),
];

/// The palette's colours (the rows; any `#RRGGBB` from the console).
pub const HAIR_COLOURS: [Rgb; 12] = {
    let mut out = [Rgb(0); 12];
    let mut i = 0;
    while i < 12 {
        out[i] = HAIR_PALETTE[i].1;
        i += 1;
    }
    out
};

/// The palette name of `c`, or `None` for a custom colour.
pub fn hair_colour_name(c: Rgb) -> Option<&'static str> {
    HAIR_PALETTE.iter().find(|(_, p)| *p == c).map(|(n, _)| *n)
}

/// What grey hair turns to (the grey share blends towards it).
pub const GREY: Rgb = Rgb(0xC4C2BE);

/// Glasses (free, spec 4C): none, the sport wrap, round frames.
pub const GLASSES: [&str; 3] = ["None", "Sport wrap", "Round"];

/// The W4 face `face` as (moustache, beard) (4N N11 migration: 1 beard
/// → full beard, 2 moustache → classic, 3 stubble → stubble).
pub fn legacy_face(face: u8) -> (u8, u8) {
    match face {
        1 => (0, 4),
        2 => (1, 0),
        3 => (0, 1),
        _ => (0, 0),
    }
}

/// The character part of a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Character {
    /// Body height (cm, from the rider profile).
    pub height_cm: u16,
    pub base: BaseBody,
    pub shape: BodyShape,
    pub age: Age,
    /// Index into [`SKIN_TONES`].
    pub skin: u8,
    /// Index into [`FACES`] (the face's shape).
    pub face: u8,
    /// Index into [`HAIR_STYLES`].
    pub hair: u8,
    pub hair_colour: Rgb,
    /// Greying with the age (*Keep my hair colour* turns it off).
    pub greying: bool,
    /// Index into [`MOUSTACHES`].
    pub moustache: u8,
    /// Index into [`BEARDS`].
    pub beard: u8,
    /// The moustache's and beard's own colour (`None`: the hair's,
    /// greying included).
    pub facial_hair_colour: Option<Rgb>,
    /// Index into [`GLASSES`].
    pub glasses: u8,
}

impl Default for Character {
    /// The generated rider as shipped: 1.78 m, body A, the normal shape,
    /// in the 30s, the fifth skin tone (the 12-step palette's fourth),
    /// short dark-brown hair, no facial hair, sport glasses.
    fn default() -> Self {
        Self {
            height_cm: DEFAULT_HEIGHT_CM,
            base: BaseBody::A,
            shape: BodyShape::Normal,
            age: Age::Thirties,
            skin: 4,
            face: 0,
            hair: 0,
            hair_colour: HAIR_PALETTE[1].1,
            greying: true,
            moustache: 0,
            beard: 0,
            facial_hair_colour: None,
            glasses: 1,
        }
    }
}

/// `c` moved `share` (0 … 1) towards `to` in sRGB.
fn blend(c: Rgb, to: Rgb, share: f32) -> Rgb {
    let a = c.channels();
    let b = to.channels();
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * share).round() as u32;
    Rgb((mix(a[0], b[0]) << 16) | (mix(a[1], b[1]) << 8) | mix(a[2], b[2]))
}

impl Character {
    /// Values from the network or a file, made valid: indices past the
    /// lists become the default's, the height is clamped to the drawn
    /// range (0: the default height).
    pub fn validated(self) -> Self {
        let d = Self::default();
        let pick = |v: u8, n: usize, d: u8| if (v as usize) < n { v } else { d };
        Self {
            height_cm: if self.height_cm == 0 {
                DEFAULT_HEIGHT_CM
            } else {
                self.height_cm.clamp(HEIGHT_RANGE_CM.0, HEIGHT_RANGE_CM.1)
            },
            base: self.base,
            shape: self.shape,
            age: self.age,
            skin: pick(self.skin, SKIN_TONES.len(), d.skin),
            face: pick(self.face, FACES.len(), d.face),
            hair: pick(self.hair, HAIR_STYLES.len(), d.hair),
            hair_colour: Rgb::from_wire(self.hair_colour.0),
            greying: self.greying,
            moustache: pick(self.moustache, MOUSTACHES.len(), d.moustache),
            beard: pick(self.beard, BEARDS.len(), d.beard),
            facial_hair_colour: self.facial_hair_colour.map(|c| Rgb::from_wire(c.0)),
            glasses: pick(self.glasses, GLASSES.len(), d.glasses),
        }
    }

    /// The scale of the rider model's lengths (1 = 1.78 m).
    pub fn scale(&self) -> f32 {
        f32::from(self.validated().height_cm) / f32::from(DEFAULT_HEIGHT_CM)
    }

    /// The skin colour as drawn: the tone, a little less saturated in the
    /// 60s and 70+ (4N Q12).
    pub fn skin_colour(&self) -> Rgb {
        let c = self.validated();
        let tone = SKIN_TONES[c.skin as usize];
        let fade = c.age.skin_fade();
        if fade == 0.0 {
            return tone;
        }
        let [r, g, b] = tone.channels();
        let l = (u32::from(r) * 2126 + u32::from(g) * 7152 + u32::from(b) * 722) / 10_000;
        let l = l.min(255);
        blend(tone, Rgb((l << 16) | (l << 8) | l), fade)
    }

    /// The lips' colour: the skin as drawn (age included), redder and a
    /// little darker.
    pub fn lip_colour(&self) -> Rgb {
        blend(self.skin_colour(), Rgb(0x9A3F3A), 0.32)
    }

    /// The colour of the face's lines (4N Q12): the skin, darker.
    pub fn skin_line_colour(&self) -> Rgb {
        blend(self.skin_colour(), Rgb(0x2A1A12), 0.28)
    }

    /// A share in 0 … 1 that differs between riders with the same age
    /// (from the look itself, so every app draws the same).
    fn spread(&self) -> f32 {
        let c = self.validated();
        let mut h: u32 = 0x811C_9DC5;
        for b in c.hair_colour.0.to_le_bytes().into_iter().chain([
            c.hair,
            c.skin,
            c.base as u8,
            c.shape as u8,
            c.beard,
        ]) {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
        (h % 1000) as f32 / 999.0
    }

    /// The hair's grey share (0 … 1): the age's, spread ±25 % per rider
    /// (4N N3); 0 with *Keep my hair colour*.
    pub fn grey_share(&self) -> f32 {
        let c = self.validated();
        if !c.greying {
            return 0.0;
        }
        (c.age.grey() * (0.75 + 0.5 * c.spread())).min(1.0)
    }

    /// The hair colour as drawn: the chosen colour greyed by the age.
    pub fn hair_shown(&self) -> Rgb {
        let c = self.validated();
        blend(c.hair_colour, GREY, c.grey_share())
    }

    /// The moustache's and beard's colour as drawn: their own colour, or
    /// the hair's as drawn (greying included).
    pub fn facial_hair_shown(&self) -> Rgb {
        self.validated()
            .facial_hair_colour
            .unwrap_or_else(|| self.hair_shown())
    }

    /// The face of a W4 app (wire field `face` for older apps): the
    /// facial hair as one of its four faces.
    pub fn legacy_face(&self) -> u8 {
        let c = self.validated();
        match (c.moustache, c.beard) {
            (_, 3 | 4 | 7) => 1,
            (_, 1 | 2) => 3,
            (m, _) if m > 0 => 2,
            _ => 0,
        }
    }

    /// The height of a rider profile (cm, `None`: the default).
    pub fn with_height(self, height_cm: Option<f64>) -> Self {
        Self {
            height_cm: height_cm.map_or(DEFAULT_HEIGHT_CM, |h| {
                h.round().clamp(1.0, f64::from(u16::MAX)) as u16
            }),
            ..self
        }
        .validated()
    }

    /// "1.78 m, body A, normal, 30s, skin 5 of 16, short hair #3B2A20,
    /// no facial hair, glasses: sport wrap".
    pub fn summary(&self) -> String {
        let c = self.validated();
        let facial = match (c.moustache, c.beard) {
            (0, 0) => "no facial hair".to_string(),
            (m, 0) => format!("{} moustache", MOUSTACHES[m as usize].to_lowercase()),
            (0, b) => format!("{} beard", BEARDS[b as usize].to_lowercase()),
            (m, b) => format!(
                "{} moustache, {} beard",
                MOUSTACHES[m as usize].to_lowercase(),
                BEARDS[b as usize].to_lowercase()
            ),
        };
        format!(
            "{:.2} m, {}, {}, {}, skin {} of {}, {} hair {}, {facial}, glasses: {}",
            f32::from(c.height_cm) / 100.0,
            c.base.name().to_lowercase(),
            c.shape.id(),
            c.age.id(),
            c.skin + 1,
            SKIN_TONES.len(),
            HAIR_STYLES[c.hair as usize].to_lowercase(),
            c.hair_colour,
            GLASSES[c.glasses as usize].to_lowercase()
        )
    }
}

/// Whether a rider may change the character and kit: anonymous riders of
/// the browser app get the plain default rider and no customization
/// (Phase 10 Q5) – "log in to customize".
pub fn may_customize(anonymous: bool) -> bool {
    !anonymous
}

/// What the locked mirror says to an anonymous rider.
pub const LOCKED_MIRROR: &str =
    "The mirror is locked: log in to customize your rider (accounts come with Phase 5).";

#[cfg(test)]
mod tests {
    use super::*;

    /// The skin tones (spec §14 Phase 4, 4N, asked 2026-11-17) cover the
    /// range of human skin evenly: from very pale (L* > 90) to very dark
    /// (L* < 13), lighter to darker in every step, no step much larger
    /// than another, and warm (red-pink) as well as neutral / golden /
    /// olive undertones in the light, the medium and the dark third.
    #[test]
    fn the_skin_tones_cover_the_range_evenly_with_both_undertones() {
        let l: Vec<[f32; 3]> = SKIN_TONES.iter().map(|c| lab(*c)).collect();
        assert!(l[0][0] > 90.0 && l[15][0] < 13.0, "{:?} {:?}", l[0], l[15]);
        let steps: Vec<f32> = l.windows(2).map(|w| w[0][0] - w[1][0]).collect();
        for s in &steps {
            assert!((3.0..9.0).contains(s), "{steps:?}");
        }
        let (lo, hi) = steps
            .iter()
            .fold((f32::MAX, 0.0f32), |(a, b), s| (a.min(*s), b.max(*s)));
        assert!(hi <= 2.5 * lo, "even: {steps:?}");
        // Undertone: the yellow–red ratio b*/a* (≥ 2: neutral to golden /
        // olive; < 1.9: warm).
        for third in [0..5, 5..11, 11..16] {
            let r: Vec<f32> = l[third.clone()].iter().map(|c| c[2] / c[1]).collect();
            assert!(r.iter().any(|x| *x >= 2.0), "{third:?}: {r:?}");
            assert!(r.iter().any(|x| *x < 1.9), "{third:?}: {r:?}");
        }
        // All skin, never grey or saturated: a* and b* positive and modest.
        for c in &l {
            assert!(c[1] > 3.0 && c[1] < 20.0 && c[2] > 7.0 && c[2] < 30.0, "{c:?}");
        }
    }

    /// The 12-step palette's indices (older apps and files) map to the
    /// nearest new tone, in order, the default to the default; and back.
    #[test]
    fn old_skin_indices_map_to_the_nearest_new_tone() {
        let new: Vec<u8> = (0..12).map(skin_from_legacy).collect();
        assert!(new.windows(2).all(|w| w[0] <= w[1]), "in order: {new:?}");
        assert_eq!(new[0], 1);
        assert_eq!(new[11], 14);
        assert_eq!(skin_from_legacy(3), Character::default().skin);
        for (old, n) in new.iter().enumerate() {
            let [l1, ..] = lab(LEGACY_SKIN_TONES[old]);
            let [l2, ..] = lab(SKIN_TONES[*n as usize]);
            assert!((l1 - l2).abs() < 6.0, "{old} → {n}");
            let back = i16::from(skin_to_legacy(*n));
            assert!((back - old as i16).abs() <= 1, "round trip {old}: {back}");
        }
        assert_eq!(skin_from_legacy(200), 14, "out of range: the darkest");
        assert_eq!(skin_to_legacy(0), 0);
        assert_eq!(skin_to_legacy(15), 11);
    }

    #[test]
    fn palettes_and_defaults() {
        assert_eq!(SKIN_TONES.len(), 16, "a 16-step palette (spec 4N)");
        // From light to dark.
        let lum = |c: Rgb| {
            let [r, g, b] = c.channels();
            0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)
        };
        for w in SKIN_TONES.windows(2) {
            assert!(lum(w[0]) > lum(w[1]), "{} {}", w[0], w[1]);
        }
        let d = Character::default();
        assert_eq!(d.validated(), d);
        assert_eq!(d.scale(), 1.0);
        assert_eq!(d.skin_colour(), SKIN_TONES[4], "the shipped skin colour");
        assert!((lab(d.skin_colour())[0] - lab(Rgb(0xD9A98C))[0]).abs() < 3.0, "as before 4.51");
        assert_eq!(d.hair_colour, Rgb(0x3B2A20), "the shipped hair colour");
        assert_eq!(d.hair_shown(), d.hair_colour, "no grey in the 30s");
        for b in BodyBuild::ALL {
            assert_eq!(BodyBuild::parse(b.id()), Some(b));
            assert_eq!(BodyBuild::from_wire(b.wire()), b);
            assert_eq!(BodyShape::from_build(b).nearest_build(), b);
        }
        assert_eq!(BodyBuild::from_wire(9), BodyBuild::Medium);
        assert_eq!(HAIR_PALETTE.len(), 12, "the natural palette of N5");
        assert_eq!(HAIR_COLOURS[11], Rgb(0xE8E4DC));
        assert_eq!(hair_colour_name(Rgb(0xD8B27A)), Some("Blond"));
        assert_eq!(hair_colour_name(Rgb(0x00FF88)), None);
    }

    #[test]
    fn seven_shapes_and_three_base_bodies() {
        // Q6 / Q7: the seven words, from very thin to very heavy.
        let names: Vec<&str> = BodyShape::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            [
                "Anorexia", "Skinny", "Slim", "Normal", "Ample", "Fat", "Obese"
            ]
        );
        for w in BodyShape::ALL.windows(2) {
            let (a, b) = (w[0].girth(), w[1].girth());
            assert!(a.0 < b.0 && a.1 < b.1, "{w:?}");
        }
        assert_eq!(BodyShape::Normal.girth(), (1.0, 1.0));
        for s in BodyShape::ALL {
            assert_eq!(BodyShape::parse(s.id()), Some(s));
            assert_eq!(BodyShape::from_wire(s.wire()), Some(s));
        }
        assert_eq!(BodyShape::parse("strong"), Some(BodyShape::Ample));
        assert_eq!(BodyShape::from_wire(7), None);
        // Q6b: base bodies, a second dimension.
        for b in BaseBody::ALL {
            assert_eq!(BaseBody::parse(b.id()), Some(b));
            assert_eq!(BaseBody::parse(b.name()), Some(b));
            assert_eq!(BaseBody::from_wire(b.wire()), b);
        }
        assert_eq!(BaseBody::from_wire(5), BaseBody::A);
    }

    /// The *Match my weight* proposal (4N Q13, Q23): the shape of the
    /// body-mass index of weight and height – a proposal, not a limit.
    #[test]
    fn weight_and_height_propose_a_shape() {
        // BMI steps: 16, 18.5, 21, 25, 28, 32.
        assert_eq!(BodyShape::from_bmi(14.0), BodyShape::Anorexia);
        assert_eq!(BodyShape::from_bmi(17.0), BodyShape::Skinny);
        assert_eq!(BodyShape::from_bmi(20.0), BodyShape::Slim);
        assert_eq!(BodyShape::from_bmi(22.0), BodyShape::Normal);
        assert_eq!(BodyShape::from_bmi(27.0), BodyShape::Ample);
        assert_eq!(BodyShape::from_bmi(30.0), BodyShape::Fat);
        assert_eq!(BodyShape::from_bmi(40.0), BodyShape::Obese);
        assert!((bmi(75.0, 178.0) - 23.67).abs() < 0.01);
        // The height counts (Q13: "take users height into consideration").
        assert_eq!(weight_shape(75.0, Some(178.0)), BodyShape::Normal);
        assert_eq!(weight_shape(75.0, Some(160.0)), BodyShape::Fat);
        assert_eq!(weight_shape(75.0, Some(200.0)), BodyShape::Slim);
        assert_eq!(weight_shape(75.0, None), BodyShape::Normal);
        assert_eq!(weight_shape(45.0, Some(178.0)), BodyShape::Anorexia);
        assert_eq!(weight_shape(120.0, Some(178.0)), BodyShape::Obese);
    }

    /// The appearance age (4N Q12): face, body, skin and hair; greying per
    /// age and per rider, off with *Keep my hair colour*.
    #[test]
    fn the_age_changes_face_body_skin_and_hair() {
        for a in Age::ALL {
            assert_eq!(Age::parse(a.id()), Some(a));
            assert_eq!(Age::from_wire(a.wire()), a);
        }
        assert_eq!(Age::parse("70s"), Some(Age::Seventies));
        let at = |age: Age| Character {
            age,
            hair_colour: HAIR_PALETTE[0].1,
            ..Character::default()
        };
        // Hair: no grey before the 40s, more with every step.
        assert_eq!(at(Age::Twenties).grey_share(), 0.0);
        assert_eq!(at(Age::Thirties).hair_shown(), HAIR_PALETTE[0].1);
        let shares: Vec<f32> = Age::ALL.iter().map(|a| at(*a).grey_share()).collect();
        for w in shares.windows(2) {
            assert!(w[0] <= w[1], "{shares:?}");
        }
        assert!(shares[5] > 0.5, "{shares:?}");
        let old = at(Age::Seventies);
        assert_ne!(old.hair_shown(), old.hair_colour);
        let kept = Character {
            greying: false,
            ..old
        };
        assert_eq!(kept.hair_shown(), kept.hair_colour, "keep my hair colour");
        // The spread differs between riders, the same for the same look.
        let other = Character {
            hair: 3,
            ..at(Age::Fifties)
        };
        assert_eq!(at(Age::Fifties).grey_share(), at(Age::Fifties).grey_share());
        assert_ne!(at(Age::Fifties).grey_share(), other.grey_share());
        // Facial hair follows the hair (greyed) or its own colour.
        let bearded = Character { beard: 4, ..old };
        assert_eq!(bearded.facial_hair_shown(), bearded.hair_shown());
        let own = Character {
            facial_hair_colour: Some(Rgb(0x6B4426)),
            ..bearded
        };
        assert_eq!(own.facial_hair_shown(), Rgb(0x6B4426));
        // Skin: less saturated at 70+, unchanged before the 60s.
        assert_eq!(
            at(Age::Fifties).skin_colour(),
            at(Age::Twenties).skin_colour()
        );
        let sat = |c: Rgb| {
            let [r, g, b] = c.channels();
            i32::from(r.max(g).max(b)) - i32::from(r.min(g).min(b))
        };
        assert!(sat(old.skin_colour()) < sat(at(Age::Twenties).skin_colour()));
        // Face and body.
        assert_eq!(Age::Thirties.face_lines(), 0);
        assert_eq!(Age::Forties.face_lines(), 1);
        assert_eq!(Age::Seventies.face_lines(), 2);
        assert!(Age::Sixties.soft_jaw() && !Age::Fifties.soft_jaw());
        assert!(Age::Seventies.body().0 > 1.0 && Age::Thirties.body() == (1.0, 1.0));
        assert!(Age::Seventies.idle_hunch() > 0.0 && Age::Fifties.idle_hunch() == 0.0);
    }

    #[test]
    fn facial_hair_and_hair_catalogue() {
        assert_eq!(HAIR_STYLES.len(), 21);
        // Today's six keep their indices.
        assert_eq!(
            &HAIR_STYLES[..6],
            &["Short", "Buzz cut", "Long", "Ponytail", "Bun", "Bald"]
        );
        for i in 0..HAIR_STYLES.len() as u8 {
            assert_eq!(hair_parse(&hair_id(i)), Some(i));
        }
        assert_eq!(hair_parse("Single braid"), Some(16));
        assert_eq!(hair_parse("3"), Some(3));
        assert_eq!(hair_parse("mohawk"), None);
        assert_eq!(list_parse(&BEARDS, "short-boxed"), Some(3));
        assert_eq!(list_parse(&MOUSTACHES, "Handlebar"), Some(3));
        assert_eq!(list_id(&BEARDS, 6), "chin-strap");
        // The W4 faces split into face 0 and facial hair (N11).
        assert_eq!(legacy_face(0), (0, 0));
        assert_eq!(legacy_face(1), (0, 4));
        assert_eq!(legacy_face(2), (1, 0));
        assert_eq!(legacy_face(3), (0, 1));
        for f in 0..4 {
            let (moustache, beard) = legacy_face(f);
            let c = Character {
                moustache,
                beard,
                ..Character::default()
            };
            assert_eq!(c.legacy_face(), f, "round trip of face {f}");
        }
    }

    #[test]
    fn bad_values_are_made_valid() {
        let c = Character {
            height_cm: 400,
            skin: 99,
            face: 40,
            hair: 70,
            hair_colour: Rgb(0xFF00_1234),
            glasses: 9,
            moustache: 30,
            beard: 30,
            facial_hair_colour: Some(Rgb(0xFF00_0001)),
            ..Default::default()
        }
        .validated();
        let d = Character::default();
        assert_eq!(c.height_cm, HEIGHT_RANGE_CM.1);
        assert_eq!(
            (c.skin, c.face, c.hair, c.glasses, c.moustache, c.beard),
            (d.skin, d.face, d.hair, d.glasses, 0, 0)
        );
        assert_eq!(c.hair_colour, Rgb(0x001234));
        assert_eq!(c.facial_hair_colour, Some(Rgb(1)));
        assert_eq!(
            Character { height_cm: 0, ..d }.validated().height_cm,
            DEFAULT_HEIGHT_CM
        );
        assert_eq!(d.with_height(Some(191.4)).height_cm, 191);
        assert_eq!(d.with_height(None).height_cm, DEFAULT_HEIGHT_CM);
        assert_eq!(d.with_height(Some(120.0)).height_cm, HEIGHT_RANGE_CM.0);
        assert!(
            d.summary().starts_with("1.78 m, body a, normal, 30s"),
            "{}",
            d.summary()
        );
    }

    #[test]
    fn anonymous_riders_cannot_customize() {
        assert!(!may_customize(true));
        assert!(may_customize(false));
        assert!(LOCKED_MIRROR.contains("log in to customize"));
    }
}
