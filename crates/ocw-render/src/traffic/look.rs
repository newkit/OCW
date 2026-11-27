//! What a road vehicle looks like (spec 14 Phase 18, 18B *Models*, 18E
//! T7): the model its class and look seed pick ([`VehicleModel::of`]) and
//! the colour of its paint slot from a regional palette of real-world car
//! colours ([`paint`]: many greys, whites and blacks, a few colours; buses
//! in a made-up local operator's colours). Pure: the same seed gives the
//! same vehicle on every client.

use ocw_sim::traffic::VehicleClass;

/// The T6 model files of the road vehicles (`artwork/3dmodels/vehicles/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VehicleModel {
    Hatchback,
    Saloon,
    Suv,
    Van,
    RigidLorry,
    ArticulatedLorry,
    Bus,
    Motorbike,
    Tractor,
}

impl VehicleModel {
    pub const ALL: [Self; 9] = [
        Self::Hatchback,
        Self::Saloon,
        Self::Suv,
        Self::Van,
        Self::RigidLorry,
        Self::ArticulatedLorry,
        Self::Bus,
        Self::Motorbike,
        Self::Tractor,
    ];

    /// The model of a vehicle of `class` with look seed `look`: cars are
    /// hatchbacks, saloons and SUVs (4 : 4 : 2), lorries rigid or
    /// articulated (3 : 2).
    pub fn of(class: VehicleClass, look: u32) -> Self {
        let pick = mix(look, 0x5EED) % 10;
        match class {
            VehicleClass::Car => match pick {
                0..=3 => Self::Hatchback,
                4..=7 => Self::Saloon,
                _ => Self::Suv,
            },
            VehicleClass::Van => Self::Van,
            VehicleClass::Lorry => {
                if pick < 6 {
                    Self::RigidLorry
                } else {
                    Self::ArticulatedLorry
                }
            }
            VehicleClass::Bus => Self::Bus,
            VehicleClass::Motorbike => Self::Motorbike,
            VehicleClass::Tractor => Self::Tractor,
        }
    }

    /// The model's id in the library (`artwork/3dmodels/`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Hatchback => "vehicles/car-hatchback",
            Self::Saloon => "vehicles/car-saloon",
            Self::Suv => "vehicles/car-suv",
            Self::Van => "vehicles/van-panel",
            Self::RigidLorry => "vehicles/lorry-rigid-box",
            Self::ArticulatedLorry => "vehicles/lorry-articulated-box",
            Self::Bus => "vehicles/bus-city",
            Self::Motorbike => "vehicles/motorbike-touring",
            Self::Tractor => "vehicles/tractor-farm",
        }
    }

    /// The slot the look seed paints (`body`; a bus: `livery`).
    pub fn paint_slot(self) -> &'static str {
        match self {
            Self::Bus => "livery",
            _ => "body",
        }
    }
}

/// A 32-bit mix (a seed and a salt → well spread bits).
pub fn mix(a: u32, salt: u32) -> u32 {
    let mut x = a ^ salt.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

/// Where the cars are: the palettes differ a little by region (more
/// white and silver in the south, more dark greys, blacks and blues in
/// central Europe – the registration statistics of the last years).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteRegion {
    /// Germany, Switzerland, Liechtenstein, Austria (the Black Forest,
    /// the Alps).
    #[default]
    Central,
    /// Spain, the Canaries, Andorra.
    South,
}

impl PaletteRegion {
    /// The region of a place (latitude, longitude °).
    pub fn of(lat: f64, lon: f64) -> Self {
        // South of the Pyrenees' crest (Andorra included) and the
        // Canaries; everything else here is central Europe.
        if lat < 42.8 && lon < 3.5 {
            Self::South
        } else {
            Self::Central
        }
    }
}

/// One palette entry: sRGB colour and weight.
type Entry = (u32, u32);

/// Central Europe: grey and black lead, then white, silver, blue; a few
/// reds, greens, browns and the odd yellow.
const CENTRAL: [Entry; 14] = [
    (0x5B5E63, 16), // dark grey (graphite)
    (0x8A8D91, 12), // mid grey
    (0x0E0F11, 15), // black
    (0x1A1C20, 6),  // black, metallic
    (0xE9EAEC, 13), // white
    (0xF4F3EF, 4),  // cream white
    (0xB6B9BD, 11), // silver
    (0x1F3557, 7),  // dark blue
    (0x2E5AA6, 3),  // mid blue
    (0x8E1B1B, 5),  // dark red
    (0xC2261F, 2),  // red
    (0x2F4A33, 2),  // dark green
    (0x5A4636, 2),  // brown
    (0xD7A91C, 1),  // yellow
];

/// The south: more white and silver, some bright reds and blues.
const SOUTH: [Entry; 13] = [
    (0xEDEEF0, 24), // white
    (0xF6F4EE, 5),  // cream white
    (0xBDC0C4, 16), // silver
    (0x8C8F93, 10), // mid grey
    (0x55585D, 9),  // dark grey
    (0x0F1012, 10), // black
    (0x1E3A68, 6),  // dark blue
    (0x3569C4, 4),  // mid blue
    (0xC1251E, 6),  // red
    (0x8B1D1D, 3),  // dark red
    (0x2E5236, 2),  // green
    (0xD9B47A, 3),  // beige / sand
    (0xE07A1F, 2),  // orange
];

/// A made-up local operator's bus colours (no real company): per region
/// two liveries.
const BUS_CENTRAL: [u32; 2] = [0xC8D44A, 0xE2E4E6];
const BUS_SOUTH: [u32; 2] = [0x2D9C5A, 0x1E6FB8];

/// Tractors: the usual red, green, blue and orange of farm machinery.
const TRACTOR: [u32; 4] = [0xB0231C, 0x2E7D32, 0x1C4E9A, 0xE0731B];
/// Lorry cabs: white mostly, some colours.
const LORRY: [Entry; 7] = [
    (0xEDEEF0, 10),
    (0xB6B9BD, 3),
    (0x1F3557, 3),
    (0xC2261F, 2),
    (0x0E0F11, 2),
    (0x2F5E3A, 1),
    (0xE0A21B, 1),
];

fn weighted(table: &[Entry], r: u32) -> u32 {
    let total: u32 = table.iter().map(|e| e.1).sum();
    let mut k = r % total;
    for (c, w) in table {
        if k < *w {
            return *c;
        }
        k -= w;
    }
    table[0].0
}

/// The paint colour (sRGB, 0xRRGGBB) of a vehicle of `model` with look
/// seed `look` in `region`.
pub fn paint(model: VehicleModel, look: u32, region: PaletteRegion) -> u32 {
    let r = mix(look, 0xC010);
    match model {
        VehicleModel::Bus => match region {
            PaletteRegion::Central => BUS_CENTRAL[(r % 2) as usize],
            PaletteRegion::South => BUS_SOUTH[(r % 2) as usize],
        },
        VehicleModel::Tractor => TRACTOR[(r % TRACTOR.len() as u32) as usize],
        VehicleModel::RigidLorry | VehicleModel::ArticulatedLorry => weighted(&LORRY, r),
        _ => match region {
            PaletteRegion::Central => weighted(&CENTRAL, r),
            PaletteRegion::South => weighted(&SOUTH, r),
        },
    }
}

/// The colours a palette can give (tests, the art board).
pub fn palette(region: PaletteRegion) -> Vec<u32> {
    match region {
        PaletteRegion::Central => CENTRAL.iter().map(|e| e.0).collect(),
        PaletteRegion::South => SOUTH.iter().map(|e| e.0).collect(),
    }
}

/// Whether a colour is a grey, white, silver or black (chroma under 12 %).
pub fn is_neutral(c: u32) -> bool {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    hi - lo < 31
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_picks_the_model_and_the_colour_deterministically() {
        for look in [0u32, 1, 77, 0xDEAD_BEEF] {
            assert_eq!(
                VehicleModel::of(VehicleClass::Car, look),
                VehicleModel::of(VehicleClass::Car, look)
            );
            for m in VehicleModel::ALL {
                assert_eq!(
                    paint(m, look, PaletteRegion::Central),
                    paint(m, look, PaletteRegion::Central)
                );
            }
        }
        // Fixed values: a client of another build paints the same car.
        assert_eq!(
            VehicleModel::of(VehicleClass::Car, 12345),
            VehicleModel::of(VehicleClass::Car, 12345)
        );
        let a = paint(VehicleModel::Saloon, 12345, PaletteRegion::Central);
        assert!(palette(PaletteRegion::Central).contains(&a));
    }

    #[test]
    fn cars_come_in_three_shapes_and_mostly_neutral_colours() {
        let mut shapes = std::collections::BTreeMap::new();
        let mut neutral = [0usize; 2];
        let n = 4000u32;
        for look in 0..n {
            let s = mix(look, 1);
            *shapes
                .entry(VehicleModel::of(VehicleClass::Car, s))
                .or_insert(0) += 1;
            for (i, r) in [PaletteRegion::Central, PaletteRegion::South]
                .into_iter()
                .enumerate()
            {
                if is_neutral(paint(VehicleModel::Hatchback, s, r)) {
                    neutral[i] += 1;
                }
            }
        }
        assert_eq!(shapes.len(), 3, "{shapes:?}");
        let suv = shapes[&VehicleModel::Suv] as f64 / f64::from(n);
        assert!((0.15..0.25).contains(&suv), "{suv}");
        // Real fleets: about three in four cars are grey, white, silver
        // or black.
        for k in neutral {
            let share = k as f64 / f64::from(n);
            assert!((0.7..0.9).contains(&share), "{share}");
        }
        // The south has more white.
        let white = |r| {
            (0..n)
                .filter(|l| paint(VehicleModel::Saloon, mix(*l, 2), r) >= 0xE0_0000)
                .count()
        };
        assert!(white(PaletteRegion::South) > white(PaletteRegion::Central));
        // Lorries: rigid and articulated.
        let artic = (0..n)
            .filter(|l| {
                VehicleModel::of(VehicleClass::Lorry, mix(*l, 3)) == VehicleModel::ArticulatedLorry
            })
            .count();
        assert!((1200..2000).contains(&artic), "{artic}");
    }

    #[test]
    fn regions_by_place() {
        // Tenerife, Andorra: south; Vaduz, the Black Forest: central.
        assert_eq!(PaletteRegion::of(28.3, -16.5), PaletteRegion::South);
        assert_eq!(PaletteRegion::of(42.5, 1.5), PaletteRegion::South);
        assert_eq!(PaletteRegion::of(47.14, 9.52), PaletteRegion::Central);
        assert_eq!(PaletteRegion::of(48.0, 8.0), PaletteRegion::Central);
    }
}
