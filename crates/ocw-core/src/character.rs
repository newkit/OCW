//! The rider's character (spec §2.6, §14 Phase 4, 4C; milestone W4): body
//! (height and build), skin tone, face, hair and glasses. Free choices –
//! "who someone is never costs anything" (Phase 4 Q4) – and never physics:
//! the kit (helmet, shoes, gloves, clothing) is made of catalogue items
//! with prices and small effects, the character is only looks. Part of
//! [`crate::RiderLook`]; on the wire the character travels once per
//! change (`RiderBuild.look`), not in every snapshot.

use crate::bike::Rgb;

/// Default body height (cm): the generated rider is 1.78 m tall.
pub const DEFAULT_HEIGHT_CM: u16 = 178;
/// The body heights the rider model is drawn at (cm; a profile's height
/// outside is clamped for drawing only).
pub const HEIGHT_RANGE_CM: (u16, u16) = (150, 205);

/// The body build: girth of the rigid body parts.
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

    pub fn name(self) -> &'static str {
        match self {
            Self::Slim => "Slim",
            Self::Medium => "Medium",
            Self::Strong => "Strong",
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

    /// Girth of the torso and limbs against the medium build.
    pub fn girth(self) -> f32 {
        match self {
            Self::Slim => 0.9,
            Self::Medium => 1.0,
            Self::Strong => 1.12,
        }
    }
}

/// The skin-tone palette: 12 steps from very light to very dark (sRGB).
pub const SKIN_TONES: [Rgb; 12] = [
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

/// Face presets (generated shapes on the head).
pub const FACES: [&str; 4] = ["Classic", "Beard", "Moustache", "Stubble"];

/// Hair styles (generated meshes; "Bald": none).
pub const HAIR_STYLES: [&str; 6] = ["Short", "Buzz cut", "Long", "Ponytail", "Bun", "Bald"];

/// Hair colours offered on the rows (any `#RRGGBB` from the console).
pub const HAIR_COLOURS: [Rgb; 8] = [
    Rgb(0x3B2A20),
    Rgb(0x15110F),
    Rgb(0x6B4426),
    Rgb(0xA0703F),
    Rgb(0xD8B27A),
    Rgb(0xB3482A),
    Rgb(0x9C9A96),
    Rgb(0xE8E4DC),
];

/// Glasses (free, spec 4C): none, the sport wrap, round frames.
pub const GLASSES: [&str; 3] = ["None", "Sport wrap", "Round"];

/// The character part of a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Character {
    /// Body height (cm, from the rider profile).
    pub height_cm: u16,
    pub build: BodyBuild,
    /// Index into [`SKIN_TONES`].
    pub skin: u8,
    /// Index into [`FACES`].
    pub face: u8,
    /// Index into [`HAIR_STYLES`].
    pub hair: u8,
    pub hair_colour: Rgb,
    /// Index into [`GLASSES`].
    pub glasses: u8,
}

impl Default for Character {
    /// The generated rider as shipped: 1.78 m, medium build, the fourth
    /// skin tone, short dark-brown hair, sport glasses.
    fn default() -> Self {
        Self {
            height_cm: DEFAULT_HEIGHT_CM,
            build: BodyBuild::Medium,
            skin: 3,
            face: 0,
            hair: 0,
            hair_colour: HAIR_COLOURS[0],
            glasses: 1,
        }
    }
}

impl Character {
    /// Values from the network or a file, made valid: indices past the
    /// palettes become the default's, the height is clamped to the drawn
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
            build: self.build,
            skin: pick(self.skin, SKIN_TONES.len(), d.skin),
            face: pick(self.face, FACES.len(), d.face),
            hair: pick(self.hair, HAIR_STYLES.len(), d.hair),
            hair_colour: Rgb::from_wire(self.hair_colour.0),
            glasses: pick(self.glasses, GLASSES.len(), d.glasses),
        }
    }

    /// The scale of the rider model's lengths (1 = 1.78 m).
    pub fn scale(&self) -> f32 {
        f32::from(self.validated().height_cm) / f32::from(DEFAULT_HEIGHT_CM)
    }

    pub fn skin_colour(&self) -> Rgb {
        SKIN_TONES[self.validated().skin as usize]
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

    /// "1.78 m, medium build, skin 4 of 12, short hair #3B2A20, sport wrap".
    pub fn summary(&self) -> String {
        let c = self.validated();
        format!(
            "{:.2} m, {} build, skin {} of 12, {} face, {} hair {}, glasses: {}",
            f32::from(c.height_cm) / 100.0,
            c.build.id(),
            c.skin + 1,
            FACES[c.face as usize].to_lowercase(),
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

    #[test]
    fn palettes_and_defaults() {
        assert_eq!(SKIN_TONES.len(), 12, "a 12-step palette (spec 4C)");
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
        assert_eq!(d.skin_colour(), Rgb(0xD9A98C), "the shipped skin colour");
        for b in BodyBuild::ALL {
            assert_eq!(BodyBuild::parse(b.id()), Some(b));
            assert_eq!(BodyBuild::from_wire(b.wire()), b);
        }
        assert_eq!(BodyBuild::from_wire(9), BodyBuild::Medium);
        assert!(BodyBuild::Slim.girth() < 1.0 && BodyBuild::Strong.girth() > 1.0);
    }

    #[test]
    fn bad_values_are_made_valid() {
        let c = Character {
            height_cm: 400,
            skin: 99,
            face: 40,
            hair: 7,
            hair_colour: Rgb(0xFF00_1234),
            glasses: 9,
            ..Default::default()
        }
        .validated();
        let d = Character::default();
        assert_eq!(c.height_cm, HEIGHT_RANGE_CM.1);
        assert_eq!(
            (c.skin, c.face, c.hair, c.glasses),
            (d.skin, d.face, d.hair, d.glasses)
        );
        assert_eq!(c.hair_colour, Rgb(0x001234));
        assert_eq!(
            Character { height_cm: 0, ..d }.validated().height_cm,
            DEFAULT_HEIGHT_CM
        );
        assert_eq!(d.with_height(Some(191.4)).height_cm, 191);
        assert_eq!(d.with_height(None).height_cm, DEFAULT_HEIGHT_CM);
        assert_eq!(d.with_height(Some(120.0)).height_cm, HEIGHT_RANGE_CM.0);
        assert!(
            d.summary().starts_with("1.78 m, medium build"),
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
