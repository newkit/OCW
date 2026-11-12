//! Curated physics profiles and the cosmetic look of a rider and bike
//! (spec §2.5, §8.2).
//!
//! Physics comes only from the small fixed table [`PHYSICS_PROFILES`]:
//! a client names a profile by its id (`"road-aero"`), the server looks
//! it up in its own copy of this table (an unknown id is the default
//! profile) and never takes numbers from the client. Everything else a
//! rider chooses – colours, kit pattern – is [`RiderLook`], purely
//! cosmetic: it is passed through to the other clients for rendering
//! and never reaches the physics.

/// The bike families (spec §2.5): each has its own procedural model and
/// one or more physics profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BikeType {
    /// Drop-bar road bike.
    Road,
    /// Time-trial bike: aero bars, disc rear wheel.
    TimeTrial,
    /// Gravel bike: drop bars, wider tyres.
    Gravel,
    /// Mountain bike: flat bar, fat tyres, suspension fork.
    Mountain,
}

impl BikeType {
    pub const ALL: [BikeType; 4] = [
        BikeType::Road,
        BikeType::TimeTrial,
        BikeType::Gravel,
        BikeType::Mountain,
    ];

    /// Id in files and console commands.
    pub fn id(self) -> &'static str {
        match self {
            Self::Road => "road",
            Self::TimeTrial => "tt",
            Self::Gravel => "gravel",
            Self::Mountain => "mtb",
        }
    }

    /// Name for the menu.
    pub fn name(self) -> &'static str {
        match self {
            Self::Road => "Road",
            Self::TimeTrial => "Time trial",
            Self::Gravel => "Gravel",
            Self::Mountain => "Mountain bike",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.id().eq_ignore_ascii_case(s.trim()))
    }

    /// The profile a newly chosen bike of this type rides with.
    pub fn default_profile(self) -> ProfileId {
        match self {
            Self::Road => ProfileId::Road,
            Self::TimeTrial => ProfileId::TimeTrial,
            Self::Gravel => ProfileId::Gravel,
            Self::Mountain => ProfileId::Mountain,
        }
    }

    /// The profiles of this bike type, the default first.
    pub fn profiles(self) -> impl Iterator<Item = ProfileId> {
        ProfileId::ALL.into_iter().filter(move |p| p.bike() == self)
    }
}

/// One of the curated physics profiles (spec §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum ProfileId {
    /// Road bike, standard: the default for everyone.
    #[default]
    Road,
    /// Road bike, aero frame and deep rims: less drag, a little heavier.
    RoadAero,
    /// Road bike, light climbing frame: lighter, a little more drag.
    RoadClimbing,
    /// Time-trial bike: least drag, heavier, worse off the asphalt.
    TimeTrial,
    /// Gravel bike: wider tyres roll better on gravel and grass.
    Gravel,
    /// Mountain bike: best off-road, heaviest and slowest on asphalt.
    Mountain,
}

impl ProfileId {
    pub const ALL: [ProfileId; 6] = [
        ProfileId::Road,
        ProfileId::RoadAero,
        ProfileId::RoadClimbing,
        ProfileId::TimeTrial,
        ProfileId::Gravel,
        ProfileId::Mountain,
    ];

    /// The profile's parameters.
    pub fn profile(self) -> &'static PhysicsProfile {
        &PHYSICS_PROFILES[self as usize]
    }

    /// Id on the wire (`Hello.bike_profile`), in files and commands.
    pub fn id(self) -> &'static str {
        self.profile().id
    }

    pub fn name(self) -> &'static str {
        self.profile().name
    }

    pub fn bike(self) -> BikeType {
        self.profile().bike
    }

    /// The profile with this id (case does not matter), if it exists.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(s.trim()))
    }

    /// The profile a server uses for an id from the network: an unknown or
    /// missing id is the default profile (spec §2.5).
    pub fn from_wire(s: Option<&str>) -> Self {
        s.and_then(Self::parse).unwrap_or_default()
    }
}

/// Parameters of a curated physics profile: everything about the bike
/// that the physics uses (the rider adds only their weight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub bike: BikeType,
    /// Mass of the bike in kg.
    pub bike_mass_kg: f64,
    /// Drag area of rider and bike (Cd·A) in m².
    pub cda_m2: f64,
    /// Rolling resistance coefficient on asphalt.
    pub crr: f64,
    /// Share of the pedal power that reaches the rear wheel.
    pub drivetrain_efficiency: f64,
    /// How much of a rough surface's extra rolling resistance this bike
    /// feels (1 = all of it, the road bike; wider tyres less), see
    /// [`surface_rolling_factor`].
    pub rough_surface_scale: f64,
}

/// The curated physics profiles, in [`ProfileId`] order. Differences are
/// capped (spec §2.5): at 250 W on flat asphalt every profile is within
/// ±3 % of the road bike's speed (tested in `ocw-sim`); off the asphalt
/// the wider tyres of gravel and mountain bikes pay off.
pub const PHYSICS_PROFILES: [PhysicsProfile; 6] = [
    PhysicsProfile {
        id: "road",
        name: "Road - standard",
        bike: BikeType::Road,
        bike_mass_kg: 8.0,
        cda_m2: 0.32,
        crr: 0.004,
        drivetrain_efficiency: 0.976,
        rough_surface_scale: 1.0,
    },
    PhysicsProfile {
        id: "road-aero",
        name: "Road - aero",
        bike: BikeType::Road,
        bike_mass_kg: 8.6,
        cda_m2: 0.305,
        crr: 0.004,
        drivetrain_efficiency: 0.976,
        rough_surface_scale: 1.0,
    },
    PhysicsProfile {
        id: "road-climbing",
        name: "Road - climbing",
        bike: BikeType::Road,
        bike_mass_kg: 6.8,
        cda_m2: 0.33,
        crr: 0.004,
        drivetrain_efficiency: 0.976,
        rough_surface_scale: 1.0,
    },
    PhysicsProfile {
        id: "tt",
        name: "Time trial",
        bike: BikeType::TimeTrial,
        bike_mass_kg: 9.2,
        cda_m2: 0.295,
        crr: 0.0039,
        drivetrain_efficiency: 0.976,
        rough_surface_scale: 1.3,
    },
    PhysicsProfile {
        id: "gravel",
        name: "Gravel",
        bike: BikeType::Gravel,
        bike_mass_kg: 9.0,
        cda_m2: 0.325,
        crr: 0.0045,
        drivetrain_efficiency: 0.974,
        rough_surface_scale: 0.5,
    },
    PhysicsProfile {
        id: "mtb",
        name: "Mountain bike",
        bike: BikeType::Mountain,
        bike_mass_kg: 11.0,
        cda_m2: 0.33,
        crr: 0.0048,
        drivetrain_efficiency: 0.972,
        rough_surface_scale: 0.3,
    },
];

/// The factor on a profile's rolling resistance on a surface whose own
/// factor (for the road bike) is `surface_factor` (asphalt 1, gravel 2.5,
/// grass 6, `ocw_world::Surface::rolling_factor`): only the part above
/// asphalt is scaled, so asphalt stays 1 for every bike.
pub fn surface_rolling_factor(rough_surface_scale: f64, surface_factor: f64) -> f64 {
    1.0 + (surface_factor - 1.0) * rough_surface_scale
}

/// Kit patterns of the jersey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KitPattern {
    #[default]
    Plain,
    /// A band around the chest.
    Band,
    /// A diagonal sash.
    Sash,
}

impl KitPattern {
    pub const ALL: [KitPattern; 3] = [KitPattern::Plain, KitPattern::Band, KitPattern::Sash];

    pub fn id(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Band => "band",
            Self::Sash => "sash",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(s.trim()))
    }

    /// Number on the wire (`RiderLook.kit_pattern`).
    pub fn wire(self) -> u32 {
        self as u32
    }

    /// The pattern for a wire number; unknown numbers are plain.
    pub fn from_wire(n: u32) -> Self {
        Self::ALL.get(n as usize).copied().unwrap_or_default()
    }
}

/// An sRGB colour, 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u32);

impl Rgb {
    /// A colour from the wire: only the low 24 bits count.
    pub fn from_wire(v: u32) -> Self {
        Self(v & 0xFF_FFFF)
    }

    /// Parses `#RRGGBB` or `RRGGBB`.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Self)
    }

    /// The channels as 0–255.
    pub fn channels(self) -> [u8; 3] {
        [(self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8]
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:06X}", self.0 & 0xFF_FFFF)
    }
}

/// The cosmetic look of a rider and bike (spec §2.5: never physics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RiderLook {
    pub frame: Rgb,
    pub jersey: Rgb,
    pub helmet: Rgb,
    pub pattern: KitPattern,
}

impl Default for RiderLook {
    /// The app's own colours: a #FF1472 jersey, a dark frame, a white
    /// helmet.
    fn default() -> Self {
        Self {
            frame: Rgb(0x1E1E22),
            jersey: Rgb(0xFF1472),
            helmet: Rgb(0xF2F2EE),
            pattern: KitPattern::Plain,
        }
    }
}

/// Colours offered on the Workshop's ← / → rows (any `#RRGGBB` can be set
/// from the console).
pub const PALETTE: [Rgb; 12] = [
    Rgb(0xFF1472),
    Rgb(0xE53935),
    Rgb(0xFF8F00),
    Rgb(0xFDD835),
    Rgb(0x43A047),
    Rgb(0x00897B),
    Rgb(0x1E88E5),
    Rgb(0x3949AB),
    Rgb(0x8E24AA),
    Rgb(0xF2F2EE),
    Rgb(0x9E9E9E),
    Rgb(0x1E1E22),
];

/// The palette colour `dir` steps from `c` (a colour not in the palette
/// steps from its start).
pub fn palette_step(c: Rgb, dir: i32) -> Rgb {
    let n = PALETTE.len() as i32;
    let i = PALETTE
        .iter()
        .position(|p| *p == c)
        .map_or(if dir > 0 { -1 } else { 0 }, |i| i as i32);
    PALETTE[(i + dir).rem_euclid(n) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_in_id_order_and_parse() {
        for (i, p) in ProfileId::ALL.into_iter().enumerate() {
            assert_eq!(p as usize, i);
            assert_eq!(ProfileId::parse(p.id()), Some(p));
            assert_eq!(ProfileId::parse(&p.id().to_uppercase()), Some(p));
            assert!(p.bike().profiles().any(|q| q == p));
        }
        assert_eq!(ProfileId::parse("rocket"), None);
        assert_eq!(ProfileId::from_wire(Some("rocket")), ProfileId::Road);
        assert_eq!(ProfileId::from_wire(None), ProfileId::Road);
        assert_eq!(ProfileId::from_wire(Some("mtb")), ProfileId::Mountain);
        let ids: std::collections::BTreeSet<_> = PHYSICS_PROFILES.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), PHYSICS_PROFILES.len());
    }

    #[test]
    fn bike_types_have_their_profiles() {
        for b in BikeType::ALL {
            assert_eq!(BikeType::parse(b.id()), Some(b));
            assert_eq!(b.profiles().next(), Some(b.default_profile()));
            assert_eq!(b.default_profile().bike(), b);
        }
        assert_eq!(BikeType::Road.profiles().count(), 3);
        assert_eq!(BikeType::parse("unicycle"), None);
    }

    #[test]
    fn parameters_are_plausible() {
        for p in &PHYSICS_PROFILES {
            assert!((5.0..=15.0).contains(&p.bike_mass_kg), "{}", p.id);
            assert!((0.2..=0.45).contains(&p.cda_m2), "{}", p.id);
            assert!((0.002..=0.008).contains(&p.crr), "{}", p.id);
            assert!((0.95..=0.99).contains(&p.drivetrain_efficiency), "{}", p.id);
            assert!((0.2..=1.5).contains(&p.rough_surface_scale), "{}", p.id);
        }
        // Asphalt is asphalt for every bike; gravel is easier on wide tyres.
        assert_eq!(surface_rolling_factor(0.3, 1.0), 1.0);
        assert_eq!(surface_rolling_factor(1.0, 2.5), 2.5);
        assert_eq!(surface_rolling_factor(0.5, 2.5), 1.75);
    }

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Rgb::parse("#FF1472"), Some(Rgb(0xFF1472)));
        assert_eq!(Rgb::parse("00ff00"), Some(Rgb(0x00FF00)));
        for bad in ["#FF147", "#GG1472", "red", "#FF14720", ""] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
        assert_eq!(Rgb(0x0A0B0C).to_string(), "#0A0B0C");
        assert_eq!(Rgb::from_wire(0xFFFF_FFFF), Rgb(0xFF_FFFF));
        assert_eq!(Rgb(0x123456).channels(), [0x12, 0x34, 0x56]);
        assert_eq!(RiderLook::default().jersey, Rgb(0xFF1472));
    }

    #[test]
    fn patterns_and_palette_steps() {
        for p in KitPattern::ALL {
            assert_eq!(KitPattern::from_wire(p.wire()), p);
            assert_eq!(KitPattern::parse(p.id()), Some(p));
        }
        assert_eq!(KitPattern::from_wire(99), KitPattern::Plain);
        assert_eq!(palette_step(PALETTE[0], 1), PALETTE[1]);
        assert_eq!(palette_step(PALETTE[0], -1), PALETTE[11]);
        assert_eq!(palette_step(PALETTE[11], 1), PALETTE[0]);
        // Off-palette colours start from the palette's ends.
        assert_eq!(palette_step(Rgb(0x010203), 1), PALETTE[0]);
        assert_eq!(palette_step(Rgb(0x010203), -1), PALETTE[11]);
    }
}
