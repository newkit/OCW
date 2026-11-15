//! The big plants of a region (spec §4.6.1, §14 Phase 8 T8): which species
//! a tree, shrub or plantation plant is, from what OSM maps
//! (`genus` / `species` / `taxon` / `leaf_type` of single trees and tree
//! rows, `trees=*` / `produce=*` / `crop=*` of plantations), the measured
//! leaf type (section `FRST`) and the regional rules of the spec's
//! vegetation tables by altitude, land use and – on Tenerife – the
//! windward (north) or leeward side of the island.
//!
//! The rules do not decide the leaf type itself where the package measured
//! it or where the seasons' regional mix decides it (`ocw_sky::season::
//! LeafMix`, which the far canopy also uses): [`pick_species`] chooses a
//! species within a [`LeafGroup`], so the near trees, the far canopy and
//! the seasons agree. Pure and deterministic (the picks are the trees' own
//! hashes); wasm-safe. The renderer draws each species with its own
//! low-poly model (`ocw-render::flora`).

use crate::landuse::LandClass;
use crate::trees::DominantLeaf;

/// A plant species (or a group drawn alike). The codes are stored in
/// packages (section `VEGE`, planted points): only add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Species {
    /// Not known: the regional rules decide by the place.
    #[default]
    Unknown = 0,
    // Conifers.
    NorwaySpruce = 1,
    SilverFir = 2,
    DouglasFir = 3,
    ScotsPine = 4,
    /// Mountain pine (*Pinus mugo*), the krummholz near the tree line.
    MountainPine = 5,
    Larch = 6,
    // Deciduous broadleaves.
    Beech = 7,
    SycamoreMaple = 8,
    Ash = 9,
    Oak = 10,
    Alder = 11,
    Willow = 12,
    /// Green alder (*Alnus viridis*), shrubs in avalanche tracks near the
    /// tree line.
    GreenAlder = 13,
    Birch = 14,
    /// Lime (linden), plane and horse chestnut: the big town trees.
    Lime = 15,
    /// Apple, pear, cherry, plum, walnut, service tree: orchards and the
    /// lone fruit trees of the meadows.
    FruitTree = 16,
    /// Sweet chestnut (*Castanea sativa*).
    Chestnut = 17,
    // The Canary Islands and subtropical gardens.
    /// Canary Island pine (*Pinus canariensis*).
    CanaryPine = 18,
    /// The laurel forest (laurisilva: *Laurus novocanariensis*, *Persea
    /// indica*, *Ocotea foetens*), also evergreen town trees (*Ficus*).
    Laurel = 19,
    /// Fayal-brezal: *Myrica faya* and tree heath *Erica arborea*.
    FayaHeath = 20,
    /// Canary Island date palm (*Phoenix canariensis*).
    DatePalm = 21,
    /// Slender palms: *Washingtonia*, *Roystonea*, *Wodyetia*, *Archontophoenix*.
    SlenderPalm = 22,
    /// Dragon tree (*Dracaena draco*).
    DragonTree = 23,
    /// Teide broom (*Spartocytisus supranubius*) and codeso (*Adenocarpus*).
    Broom = 24,
    /// Tabaiba (*Euphorbia balsamifera*, *E. lamarckii*) and the other
    /// low succulent shrubs of the coast.
    Tabaiba = 25,
    /// Cardón (*Euphorbia canariensis*), candelabra mounds.
    Cardon = 26,
    /// Prickly pear (*Opuntia*).
    PricklyPear = 27,
    Agave = 28,
    Eucalyptus = 29,
    /// Banana plants (*Musa*).
    Banana = 30,
    /// Evergreen fruit trees of warm climates: avocado, citrus, mango.
    SubtropicalFruit = 31,
}

/// How a species carries its leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Habit {
    /// Evergreen needles.
    Conifer,
    /// Needles that turn golden and fall.
    Larch,
    /// Leaves that come in spring and fall in autumn.
    Deciduous,
    /// Evergreen leaves.
    EvergreenBroadleaf,
    /// Palms and banana plants: evergreen fronds.
    Palm,
    /// Succulents and dry-country shrubs.
    Succulent,
    /// Shrubs that flower (broom).
    Shrub,
}

/// The leaf type group a tree belongs to, as the seasons' mix decides it
/// (`ocw_sky::season::LeafType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafGroup {
    Evergreen,
    Deciduous,
    Larch,
}

impl Species {
    pub const ALL: [Species; 32] = [
        Species::Unknown,
        Species::NorwaySpruce,
        Species::SilverFir,
        Species::DouglasFir,
        Species::ScotsPine,
        Species::MountainPine,
        Species::Larch,
        Species::Beech,
        Species::SycamoreMaple,
        Species::Ash,
        Species::Oak,
        Species::Alder,
        Species::Willow,
        Species::GreenAlder,
        Species::Birch,
        Species::Lime,
        Species::FruitTree,
        Species::Chestnut,
        Species::CanaryPine,
        Species::Laurel,
        Species::FayaHeath,
        Species::DatePalm,
        Species::SlenderPalm,
        Species::DragonTree,
        Species::Broom,
        Species::Tabaiba,
        Species::Cardon,
        Species::PricklyPear,
        Species::Agave,
        Species::Eucalyptus,
        Species::Banana,
        Species::SubtropicalFruit,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }

    /// The name for logs and the inspector.
    pub fn name(self) -> &'static str {
        match self {
            Species::Unknown => "unknown",
            Species::NorwaySpruce => "Norway spruce",
            Species::SilverFir => "silver fir",
            Species::DouglasFir => "Douglas fir",
            Species::ScotsPine => "Scots pine",
            Species::MountainPine => "mountain pine",
            Species::Larch => "larch",
            Species::Beech => "beech",
            Species::SycamoreMaple => "sycamore maple",
            Species::Ash => "ash",
            Species::Oak => "oak",
            Species::Alder => "alder",
            Species::Willow => "willow",
            Species::GreenAlder => "green alder",
            Species::Birch => "birch",
            Species::Lime => "lime",
            Species::FruitTree => "fruit tree",
            Species::Chestnut => "sweet chestnut",
            Species::CanaryPine => "Canary Island pine",
            Species::Laurel => "laurel forest tree",
            Species::FayaHeath => "faya and tree heath",
            Species::DatePalm => "Canary Island date palm",
            Species::SlenderPalm => "palm",
            Species::DragonTree => "dragon tree",
            Species::Broom => "Teide broom",
            Species::Tabaiba => "tabaiba",
            Species::Cardon => "cardón",
            Species::PricklyPear => "prickly pear",
            Species::Agave => "agave",
            Species::Eucalyptus => "eucalyptus",
            Species::Banana => "banana",
            Species::SubtropicalFruit => "avocado / citrus",
        }
    }

    /// How it carries its leaves (`Unknown`: as a deciduous tree).
    pub fn habit(self) -> Habit {
        use Species::*;
        match self {
            NorwaySpruce | SilverFir | DouglasFir | ScotsPine | MountainPine | CanaryPine => {
                Habit::Conifer
            }
            Larch => Habit::Larch,
            Unknown | Beech | SycamoreMaple | Ash | Oak | Alder | Willow | GreenAlder | Birch
            | Lime | FruitTree | Chestnut => Habit::Deciduous,
            Laurel | FayaHeath | Eucalyptus | SubtropicalFruit => Habit::EvergreenBroadleaf,
            DatePalm | SlenderPalm | Banana => Habit::Palm,
            DragonTree | Tabaiba | Cardon | PricklyPear | Agave => Habit::Succulent,
            Broom => Habit::Shrub,
        }
    }

    /// The leaf group of the seasons' mix it belongs to.
    pub fn leaf_group(self) -> LeafGroup {
        match self.habit() {
            Habit::Larch => LeafGroup::Larch,
            Habit::Deciduous => LeafGroup::Deciduous,
            _ => LeafGroup::Evergreen,
        }
    }

    /// Typical height of a grown plant (m) at scale 1 (the tree placement
    /// scales it by 0.45 … 1.5; spec §4.6.1 tables, moderated a little
    /// below the old trees of the inventories).
    pub fn height_m(self) -> f32 {
        use Species::*;
        match self {
            NorwaySpruce => 24.0,
            SilverFir => 23.0,
            DouglasFir => 27.0,
            ScotsPine => 19.0,
            MountainPine => 2.4,
            Larch => 22.0,
            Beech => 21.0,
            SycamoreMaple => 18.0,
            Ash => 20.0,
            Oak => 18.0,
            Alder => 13.0,
            Willow => 10.0,
            GreenAlder => 2.6,
            Birch => 15.0,
            Unknown | Lime => 16.0,
            FruitTree => 7.5,
            Chestnut => 15.0,
            CanaryPine => 21.0,
            Laurel => 14.0,
            FayaHeath => 7.0,
            DatePalm => 11.0,
            SlenderPalm => 14.0,
            DragonTree => 7.0,
            Broom => 1.8,
            Tabaiba => 1.0,
            Cardon => 1.8,
            PricklyPear => 1.8,
            Agave => 1.3,
            Eucalyptus => 26.0,
            Banana => 4.0,
            SubtropicalFruit => 6.5,
        }
    }

    /// The widest radius of its crown (m) at scale 1.
    pub fn crown_radius_m(self) -> f32 {
        use Species::*;
        match self {
            NorwaySpruce => 2.6,
            SilverFir => 3.2,
            DouglasFir => 3.4,
            ScotsPine => 3.4,
            MountainPine => 1.6,
            Larch => 2.8,
            Beech => 5.5,
            SycamoreMaple => 5.0,
            Ash => 4.6,
            Oak => 6.0,
            Alder => 3.0,
            Willow => 3.8,
            GreenAlder => 1.6,
            Birch => 3.0,
            Unknown | Lime => 5.0,
            FruitTree => 3.4,
            Chestnut => 5.5,
            CanaryPine => 3.8,
            Laurel => 4.0,
            FayaHeath => 2.4,
            DatePalm => 4.2,
            SlenderPalm => 2.4,
            DragonTree => 3.6,
            Broom => 1.3,
            Tabaiba => 0.9,
            Cardon => 1.5,
            PricklyPear => 1.1,
            Agave => 1.0,
            Eucalyptus => 5.0,
            Banana => 2.2,
            SubtropicalFruit => 3.0,
        }
    }

    /// Whether a rider in wild mode hits it as a trunk (spec §14 Phase 12
    /// K3): trees and palms; shrubs, succulents and banana plants are soft.
    pub fn solid(self) -> bool {
        !matches!(
            self,
            Species::MountainPine
                | Species::GreenAlder
                | Species::Broom
                | Species::Tabaiba
                | Species::Cardon
                | Species::PricklyPear
                | Species::Agave
                | Species::Banana
        )
    }

    /// The species of an OSM tree from its tags: `species`, `taxon` or
    /// `genus` (scientific names; the genus is the first word), else the
    /// `leaf_type` (`palm`: a palm; `needleleaved` / `broadleaved`:
    /// unknown, decided by the rules within that group, [`OsmLeaf`]).
    pub fn from_osm(
        genus: Option<&str>,
        species: Option<&str>,
        taxon: Option<&str>,
    ) -> Option<Species> {
        let name = species.or(taxon).or(genus)?.trim();
        let mut words = name.split_whitespace();
        let g = words.next()?.to_ascii_lowercase();
        let s = words.next().map(str::to_ascii_lowercase);
        let s = s.as_deref();
        use Species::*;
        Some(match g.as_str() {
            "picea" => NorwaySpruce,
            "abies" => SilverFir,
            "pseudotsuga" => DouglasFir,
            "pinus" => match s {
                Some("canariensis") => CanaryPine,
                Some("mugo" | "montana" | "uncinata") => MountainPine,
                _ => ScotsPine,
            },
            "larix" => Larch,
            "fagus" => Beech,
            "acer" => SycamoreMaple,
            "fraxinus" => Ash,
            "quercus" => Oak,
            "alnus" => match s {
                Some("viridis") => GreenAlder,
                _ => Alder,
            },
            "salix" | "populus" => Willow,
            "betula" => Birch,
            "tilia" | "platanus" | "aesculus" | "ulmus" | "carpinus" | "robinia" | "gleditsia"
            | "celtis" | "sophora" | "styphnolobium" | "liquidambar" | "liriodendron" => Lime,
            "malus" | "pyrus" | "prunus" | "juglans" | "sorbus" | "cydonia" | "mespilus"
            | "ficus"
                if g != "ficus" || s == Some("carica") =>
            {
                FruitTree
            }
            "castanea" => Chestnut,
            "laurus" | "persea" if s != Some("americana") => Laurel,
            "persea" => SubtropicalFruit,
            "ocotea" | "apollonias" | "ilex" | "ficus" | "magnolia" | "schinus" | "grevillea"
            | "brachychiton" | "ceratonia" | "olea" | "metrosideros" | "tipuana" | "jacaranda"
            | "delonix" | "spathodea" | "tabebuia" | "handroanthus" => Laurel,
            "myrica" | "morella" | "erica" | "arbutus" | "viburnum" => FayaHeath,
            "phoenix" => DatePalm,
            "washingtonia" | "roystonea" | "wodyetia" | "archontophoenix" | "areca" | "dypsis"
            | "caryota" | "livistona" | "syagrus" | "cocos" | "trachycarpus" | "chamaerops"
            | "howea" | "ravenea" => SlenderPalm,
            "dracaena" => DragonTree,
            "spartocytisus" | "cytisus" | "adenocarpus" | "retama" | "genista" => Broom,
            "euphorbia" => match s {
                Some("canariensis") => Cardon,
                _ => Tabaiba,
            },
            "opuntia" => PricklyPear,
            "agave" => Agave,
            "eucalyptus" => Eucalyptus,
            "musa" => Banana,
            "citrus" | "mangifera" | "carica" | "annona" | "psidium" | "eriobotrya" => {
                SubtropicalFruit
            }
            _ => return None,
        })
    }
}

/// What OSM's `leaf_type` of a tree says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsmLeaf {
    Broadleaved,
    Needleleaved,
    Palm,
}

impl OsmLeaf {
    pub fn parse(v: &str) -> Option<Self> {
        match v {
            "broadleaved" => Some(Self::Broadleaved),
            "needleleaved" => Some(Self::Needleleaved),
            "palm" => Some(Self::Palm),
            _ => None,
        }
    }
}

/// A region with its own vegetation tables (spec §4.6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloraRegion {
    /// Liechtenstein and the Alpine Rhine valley.
    AlpineRhine,
    BlackForest,
    /// Tenerife (the Canary Islands).
    Tenerife,
    /// Elsewhere: the Central European mix.
    Other,
}

impl FloraRegion {
    /// The region of a place (the same boxes as the seasons' climates,
    /// `ocw_sky::season::REGIONS`).
    pub fn at(lat: f64, lon: f64) -> Self {
        let inside =
            |a: (f64, f64), b: (f64, f64)| (a.0..=a.1).contains(&lat) && (b.0..=b.1).contains(&lon);
        if inside((46.8, 47.7), (9.2, 10.0)) {
            FloraRegion::AlpineRhine
        } else if inside((47.4, 48.9), (7.4, 8.9)) {
            FloraRegion::BlackForest
        } else if inside((27.4, 29.5), (-18.4, -13.2)) {
            FloraRegion::Tenerife
        } else {
            FloraRegion::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FloraRegion::AlpineRhine => "alpine-rhine",
            FloraRegion::BlackForest => "black-forest",
            FloraRegion::Tenerife => "tenerife",
            FloraRegion::Other => "central-europe",
        }
    }

    /// The tree line (m): above it trees are stunted and then only
    /// shrubs grow ([`stature`]).
    pub fn tree_line_m(self) -> f64 {
        match self {
            FloraRegion::AlpineRhine | FloraRegion::Other => 1950.0,
            // The Feldberg's summit (1 493 m) is above the local tree line
            // (about 1 400 m, wind-shaped spruces).
            FloraRegion::BlackForest => 1430.0,
            // The upper edge of the Canary pine forest.
            FloraRegion::Tenerife => 2250.0,
        }
    }
}

/// Where a plant stands, for the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    Forest,
    Orchard,
    /// Towns: residential, commercial, industrial.
    Urban,
    Park,
    /// Lone trees in meadows and fields.
    Open,
    Scrub,
    Wetland,
}

impl Setting {
    pub fn of_class(class: Option<LandClass>) -> Self {
        match class {
            Some(LandClass::Forest) => Setting::Forest,
            Some(LandClass::Orchard) => Setting::Orchard,
            Some(LandClass::Residential | LandClass::Commercial | LandClass::Industrial) => {
                Setting::Urban
            }
            Some(LandClass::Park) => Setting::Park,
            Some(LandClass::Scrub) => Setting::Scrub,
            Some(LandClass::Wetland) => Setting::Wetland,
            _ => Setting::Open,
        }
    }
}

/// The place of a plant, for the rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    pub region: FloraRegion,
    pub setting: Setting,
    pub alt_m: f64,
    /// The measured leaf type (section `FRST`), if any.
    pub measured: Option<DominantLeaf>,
    /// On the windward (north-east) side of Tenerife ([`windward`]);
    /// ignored elsewhere.
    pub windward: bool,
    /// Beside water (river banks, lake shores): alders and willows.
    pub near_water: bool,
}

/// A weighted choice: the first entry whose cumulated weight exceeds
/// `pick` (0 … 1) times the sum.
fn choose(table: &[(Species, f32)], pick: f32) -> Species {
    let sum: f32 = table.iter().map(|t| t.1).sum();
    let mut at = pick.clamp(0.0, 0.999_999) * sum;
    for &(s, w) in table {
        if at < w {
            return s;
        }
        at -= w;
    }
    table.last().map_or(Species::Unknown, |t| t.0)
}

/// Interpolates between altitude bands of tables by `alt` (bands
/// ascending by their lower bound; a band's table holds from its lower
/// bound up to the next one's).
fn by_altitude<'a>(bands: &'a [(f64, &'a [(Species, f32)])], alt: f64) -> &'a [(Species, f32)] {
    let mut out = bands[0].1;
    for &(lo, t) in bands {
        if alt >= lo {
            out = t;
        }
    }
    out
}

use Species::*;

// --- Liechtenstein and the Alpine Rhine (spec §4.6.1, table 1) -------------

const AR_CONIFERS: [(f64, &[(Species, f32)]); 5] = [
    (
        f64::MIN,
        &[(NorwaySpruce, 0.5), (SilverFir, 0.15), (ScotsPine, 0.35)],
    ),
    (
        650.0,
        &[(NorwaySpruce, 0.55), (SilverFir, 0.38), (ScotsPine, 0.07)],
    ),
    (1150.0, &[(NorwaySpruce, 0.75), (SilverFir, 0.25)]),
    (1600.0, &[(NorwaySpruce, 0.9), (MountainPine, 0.1)]),
    (1900.0, &[(NorwaySpruce, 0.35), (MountainPine, 0.65)]),
];
const AR_BROADLEAVES: [(f64, &[(Species, f32)]); 4] = [
    (
        f64::MIN,
        &[
            (Beech, 0.4),
            (Ash, 0.2),
            (SycamoreMaple, 0.14),
            (Oak, 0.1),
            (Lime, 0.06),
            (Alder, 0.05),
            (Birch, 0.05),
        ],
    ),
    (
        650.0,
        &[
            (Beech, 0.6),
            (SycamoreMaple, 0.2),
            (Ash, 0.15),
            (Birch, 0.05),
        ],
    ),
    (1100.0, &[(Beech, 0.45), (SycamoreMaple, 0.45), (Ash, 0.1)]),
    (
        1500.0,
        &[(SycamoreMaple, 0.4), (GreenAlder, 0.45), (Birch, 0.15)],
    ),
];

// --- The Black Forest (table 2) ---------------------------------------------

const BF_CONIFERS: [(f64, &[(Species, f32)]); 4] = [
    (
        f64::MIN,
        &[
            (NorwaySpruce, 0.4),
            (SilverFir, 0.24),
            (DouglasFir, 0.16),
            (ScotsPine, 0.2),
        ],
    ),
    (
        500.0,
        &[
            (NorwaySpruce, 0.55),
            (SilverFir, 0.3),
            (DouglasFir, 0.09),
            (ScotsPine, 0.06),
        ],
    ),
    (
        900.0,
        &[(NorwaySpruce, 0.6), (SilverFir, 0.35), (ScotsPine, 0.05)],
    ),
    (1250.0, &[(NorwaySpruce, 0.92), (MountainPine, 0.08)]),
];
const BF_BROADLEAVES: [(f64, &[(Species, f32)]); 3] = [
    (
        f64::MIN,
        &[
            (Beech, 0.35),
            (Oak, 0.3),
            (Ash, 0.1),
            (SycamoreMaple, 0.1),
            (Chestnut, 0.1),
            (Birch, 0.05),
        ],
    ),
    (
        500.0,
        &[
            (Beech, 0.7),
            (SycamoreMaple, 0.2),
            (Ash, 0.05),
            (Birch, 0.05),
        ],
    ),
    (900.0, &[(Beech, 0.6), (SycamoreMaple, 0.35), (Birch, 0.05)]),
];

// --- Central Europe outside the regions: the Black Forest's mix -------------

// --- Tenerife (table 3) -------------------------------------------------------

/// The evergreen broadleaves of the north (windward) slopes: the laurel
/// forest in the cloud belt, fayal-brezal above and where it was cut.
const TF_BROADLEAVES: [(f64, &[(Species, f32)]); 4] = [
    (
        f64::MIN,
        &[(Laurel, 0.5), (FayaHeath, 0.3), (SubtropicalFruit, 0.2)],
    ),
    (
        450.0,
        &[(Laurel, 0.75), (FayaHeath, 0.22), (Eucalyptus, 0.03)],
    ),
    (
        950.0,
        &[(FayaHeath, 0.62), (Laurel, 0.3), (Eucalyptus, 0.08)],
    ),
    (1450.0, &[(FayaHeath, 0.8), (CanaryPine, 0.2)]),
];
/// Lone trees and town trees in the warm lowlands of Tenerife.
const TF_TOWN_LOW: [(Species, f32); 6] = [
    (DatePalm, 0.34),
    (SlenderPalm, 0.14),
    (Laurel, 0.3),
    (DragonTree, 0.05),
    (SubtropicalFruit, 0.1),
    (Lime, 0.07),
];
/// Higher towns of Tenerife (La Esperanza, Vilaflor).
const TF_TOWN_HIGH: [(Species, f32); 5] = [
    (CanaryPine, 0.3),
    (Eucalyptus, 0.15),
    (Chestnut, 0.15),
    (Laurel, 0.25),
    (FruitTree, 0.15),
];
/// Lone trees in the fields of the lowlands.
const TF_OPEN_LOW: [(Species, f32); 6] = [
    (DatePalm, 0.38),
    (SubtropicalFruit, 0.25),
    (Laurel, 0.12),
    (DragonTree, 0.05),
    (Eucalyptus, 0.08),
    (FruitTree, 0.12),
];
/// Lone trees in the fields of the middle heights (north: chestnuts).
const TF_OPEN_MID: [(Species, f32); 5] = [
    (Chestnut, 0.3),
    (FayaHeath, 0.25),
    (Eucalyptus, 0.15),
    (FruitTree, 0.2),
    (CanaryPine, 0.1),
];
/// The succulent scrub of the coast (cardonal-tabaibal).
const TF_SUCCULENTS: [(Species, f32); 5] = [
    (Tabaiba, 0.48),
    (Cardon, 0.24),
    (PricklyPear, 0.14),
    (Agave, 0.08),
    (DatePalm, 0.06),
];
/// The scrub of the dry middle heights (south and west): a transition of
/// tabaibas, broom-like shrubs and young pines.
const TF_DRY_MID: [(Species, f32); 4] = [
    (Tabaiba, 0.3),
    (Broom, 0.35),
    (CanaryPine, 0.25),
    (PricklyPear, 0.1),
];
/// The summit scrub of Las Cañadas.
const TF_SUMMIT: [(Species, f32); 1] = [(Broom, 1.0)];

/// Whether a place (lat, lon) of Tenerife lies on the windward side of the
/// island: north-west of the line through the Teide (28.2723 N, 16.6422
/// W) at a bearing of 60° (the NE–SW ridge from Anaga to Teno), where the
/// trade winds' cloud belt waters the laurel forest (spec §4.6.1).
pub fn windward(lat: f64, lon: f64) -> bool {
    let (lat0, lon0): (f64, f64) = (28.2723, -16.6422);
    // Local km east / north of the Teide.
    let x = (lon - lon0) * 111.32 * lat0.to_radians().cos();
    let y = (lat - lat0) * 110.57;
    // The ridge's direction (bearing 60°: east-north-east) and its normal
    // pointing north-west.
    let (dx, dy) = (60f64.to_radians().sin(), 60f64.to_radians().cos());
    let (nx, ny) = (-dy, dx);
    x * nx + y * ny > 0.0
}

/// The species of a plant at `site` in leaf group `group` (decided by the
/// measured leaf type or the seasons' regional mix), `pick` its own hash
/// (0 … 1). Tables of spec §4.6.1.
pub fn pick_species(site: &Site, group: LeafGroup, pick: f32) -> Species {
    let alt = site.alt_m;
    if group == LeafGroup::Larch {
        return Larch;
    }
    match site.region {
        FloraRegion::Tenerife => tenerife(site, group, pick),
        FloraRegion::AlpineRhine => central(site, group, pick, &AR_CONIFERS, &AR_BROADLEAVES, alt),
        FloraRegion::BlackForest | FloraRegion::Other => {
            central(site, group, pick, &BF_CONIFERS, &BF_BROADLEAVES, alt)
        }
    }
}

fn central(
    site: &Site,
    group: LeafGroup,
    pick: f32,
    conifers: &[(f64, &[(Species, f32)])],
    broadleaves: &[(f64, &[(Species, f32)])],
    alt: f64,
) -> Species {
    let tree_line = site.region.tree_line_m();
    let wet = site.near_water || site.setting == Setting::Wetland;
    match group {
        LeafGroup::Evergreen => match site.setting {
            // Lone and town conifers: spruces, some pines.
            Setting::Urban | Setting::Park | Setting::Open | Setting::Orchard
                if alt < tree_line - 300.0 =>
            {
                choose(
                    &[(NorwaySpruce, 0.6), (ScotsPine, 0.25), (SilverFir, 0.15)],
                    pick,
                )
            }
            Setting::Scrub if alt > tree_line - 350.0 => MountainPine,
            _ => choose(by_altitude(conifers, alt), pick),
        },
        LeafGroup::Deciduous => {
            if wet && alt < 1200.0 {
                return choose(&[(Alder, 0.5), (Willow, 0.4), (Ash, 0.1)], pick);
            }
            match site.setting {
                Setting::Orchard => FruitTree,
                Setting::Urban | Setting::Park if alt < 1100.0 => choose(
                    &[
                        (Lime, 0.38),
                        (SycamoreMaple, 0.2),
                        (Ash, 0.08),
                        (Beech, 0.08),
                        (Birch, 0.1),
                        (Oak, 0.06),
                        (FruitTree, 0.1),
                    ],
                    pick,
                ),
                // The lone trees of the meadows: fruit trees (Streuobst)
                // in the valleys, maples and ashes higher up.
                Setting::Open if alt < 900.0 => choose(
                    &[
                        (FruitTree, 0.38),
                        (SycamoreMaple, 0.18),
                        (Ash, 0.16),
                        (Oak, 0.1),
                        (Lime, 0.08),
                        (Beech, 0.1),
                    ],
                    pick,
                ),
                Setting::Open => choose(
                    &[
                        (SycamoreMaple, 0.55),
                        (Ash, 0.2),
                        (Beech, 0.15),
                        (Birch, 0.1),
                    ],
                    pick,
                ),
                Setting::Scrub if alt > tree_line - 450.0 => GreenAlder,
                Setting::Scrub => choose(
                    &[(Birch, 0.3), (Willow, 0.3), (Alder, 0.2), (Ash, 0.2)],
                    pick,
                ),
                _ => choose(by_altitude(broadleaves, alt), pick),
            }
        }
        LeafGroup::Larch => Larch,
    }
}

fn tenerife(site: &Site, group: LeafGroup, pick: f32) -> Species {
    let alt = site.alt_m;
    let north = site.windward;
    // A deciduous tree on Tenerife (the seasons' mix has none in its
    // forests): chestnuts in the north's middle heights, else fruit trees.
    if group == LeafGroup::Deciduous {
        return if north && (500.0..1300.0).contains(&alt) {
            choose(&[(Chestnut, 0.6), (FruitTree, 0.4)], pick)
        } else {
            FruitTree
        };
    }
    match site.setting {
        Setting::Forest => match site.measured {
            Some(DominantLeaf::Coniferous) => CanaryPine,
            Some(DominantLeaf::Broadleaved) => {
                if north || alt > 600.0 {
                    choose(by_altitude(&TF_BROADLEAVES, alt), pick)
                } else {
                    // Thermophilous woodland and gardens of the dry side.
                    choose(
                        &[
                            (Laurel, 0.35),
                            (SubtropicalFruit, 0.25),
                            (DatePalm, 0.25),
                            (FayaHeath, 0.15),
                        ],
                        pick,
                    )
                }
            }
            // Unmeasured: the laurel forest and fayal-brezal in the north's
            // cloud belt, the Canary pine above it and on the south side.
            None => {
                if north && (350.0..1250.0).contains(&alt) {
                    choose(by_altitude(&TF_BROADLEAVES, alt), pick)
                } else {
                    CanaryPine
                }
            }
        },
        Setting::Urban | Setting::Park => {
            if alt < 700.0 {
                choose(&TF_TOWN_LOW, pick)
            } else {
                choose(&TF_TOWN_HIGH, pick)
            }
        }
        Setting::Orchard => {
            if alt < 700.0 {
                SubtropicalFruit
            } else {
                FruitTree
            }
        }
        Setting::Open | Setting::Wetland => {
            if alt < if north { 450.0 } else { 700.0 } {
                choose(&TF_OPEN_LOW, pick)
            } else if alt < 1500.0 {
                choose(&TF_OPEN_MID, pick)
            } else if alt < 2200.0 {
                CanaryPine
            } else {
                Broom
            }
        }
        Setting::Scrub => scrub_tenerife(alt, north, pick),
    }
}

/// The scrub of Tenerife by altitude and side: succulents on the coast,
/// heath (fayal-brezal) in the north's middle heights, dry scrub and young
/// pines in the south's, broom in Las Cañadas.
fn scrub_tenerife(alt: f64, north: bool, pick: f32) -> Species {
    let coast = if north { 300.0 } else { 650.0 };
    if alt < coast {
        choose(&TF_SUCCULENTS, pick)
    } else if alt < 1800.0 {
        if north {
            choose(&[(FayaHeath, 0.7), (Broom, 0.15), (CanaryPine, 0.15)], pick)
        } else {
            choose(&TF_DRY_MID, pick)
        }
    } else {
        choose(&TF_SUMMIT, pick)
    }
}

/// How tall a plant of `species` grows at `alt_m` in `region` (a factor on
/// its scale): trees shrink over the last 350 m below the tree line and
/// above it reach a third of their height (wind-shaped, krummholz);
/// shrubs and succulents keep their size, the Teide's broom gets smaller
/// towards the summit.
pub fn stature(region: FloraRegion, species: Species, alt_m: f64) -> f32 {
    let line = region.tree_line_m();
    match species.habit() {
        Habit::Shrub if region == FloraRegion::Tenerife => {
            (1.0 - 0.5 * ramp(alt_m, 2600.0, 3200.0)) as f32
        }
        Habit::Conifer | Habit::Larch | Habit::Deciduous | Habit::EvergreenBroadleaf
            if species.solid() =>
        {
            (1.0 - 0.6 * ramp(alt_m, line - 350.0, line + 50.0)) as f32
        }
        _ => 1.0,
    }
}

/// Whether plants of the scrub and lone trees still grow at `alt_m`: on
/// Tenerife the summit cone above 3 000 m is all but bare (only the Teide
/// violet), thinning out from 2 750 m; a factor on the chance.
pub fn alive(region: FloraRegion, alt_m: f64) -> f32 {
    match region {
        FloraRegion::Tenerife => (1.0 - ramp(alt_m, 2750.0, 3050.0)) as f32,
        _ => 1.0,
    }
}

fn ramp(x: f64, lo: f64, hi: f64) -> f64 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(region: FloraRegion, setting: Setting, alt_m: f64) -> Site {
        Site {
            region,
            setting,
            alt_m,
            measured: None,
            windward: true,
            near_water: false,
        }
    }

    /// The share of each species over 2 000 evenly spread picks.
    fn shares(site: &Site, group: LeafGroup) -> std::collections::BTreeMap<Species, f32> {
        let n = 2000;
        let mut m = std::collections::BTreeMap::new();
        for i in 0..n {
            let s = pick_species(site, group, (i as f32 + 0.5) / n as f32);
            *m.entry(s).or_insert(0.0) += 1.0 / n as f32;
        }
        m
    }

    #[test]
    fn codes_round_trip_and_habits_are_consistent() {
        for (i, s) in Species::ALL.iter().enumerate() {
            assert_eq!(s.code() as usize, i);
            assert_eq!(Species::from_code(s.code()), Some(*s));
            assert!(s.height_m() > 0.5 && s.crown_radius_m() > 0.3, "{s:?}");
            assert!(!s.name().is_empty());
        }
        assert_eq!(Species::from_code(200), None);
        assert_eq!(Species::Larch.leaf_group(), LeafGroup::Larch);
        assert_eq!(Species::Beech.leaf_group(), LeafGroup::Deciduous);
        assert_eq!(Species::CanaryPine.leaf_group(), LeafGroup::Evergreen);
        assert_eq!(Species::Laurel.habit(), Habit::EvergreenBroadleaf);
        assert!(Species::Beech.solid() && Species::DatePalm.solid());
        assert!(!Species::Broom.solid() && !Species::Banana.solid());
    }

    #[test]
    fn osm_names_map_to_species() {
        let f = |g: Option<&str>, s: Option<&str>| Species::from_osm(g, s, None);
        assert_eq!(f(None, Some("Phoenix canariensis")), Some(DatePalm));
        assert_eq!(f(None, Some("Phoenix Canariensis")), Some(DatePalm));
        assert_eq!(f(Some("Phoenix"), None), Some(DatePalm));
        assert_eq!(f(None, Some("Pinus canariensis")), Some(CanaryPine));
        assert_eq!(f(None, Some("Pinus mugo")), Some(MountainPine));
        assert_eq!(f(Some("Pinus"), None), Some(ScotsPine));
        assert_eq!(f(None, Some("Dracaena draco")), Some(DragonTree));
        assert_eq!(f(None, Some("Euphorbia canariensis")), Some(Cardon));
        assert_eq!(f(None, Some("Euphorbia balsamifera")), Some(Tabaiba));
        assert_eq!(f(None, Some("Washingtonia robusta")), Some(SlenderPalm));
        assert_eq!(f(None, Some("Ficus microcarpa")), Some(Laurel));
        assert_eq!(f(None, Some("Ficus carica")), Some(FruitTree));
        assert_eq!(f(None, Some("Persea americana")), Some(SubtropicalFruit));
        assert_eq!(f(None, Some("Persea indica")), Some(Laurel));
        assert_eq!(f(None, Some("Sorbus domestica")), Some(FruitTree));
        assert_eq!(f(None, Some("Malus domestica")), Some(FruitTree));
        assert_eq!(f(Some("Tilia"), None), Some(Lime));
        assert_eq!(f(None, Some("Platanus × hispanica")), Some(Lime));
        assert_eq!(f(None, Some("Fraxinus ornus")), Some(Ash));
        assert_eq!(f(None, Some("Alnus viridis")), Some(GreenAlder));
        assert_eq!(f(Some("Quercus"), None), Some(Oak));
        assert_eq!(
            Species::from_osm(None, None, Some("Fagus sylvatica")),
            Some(Beech)
        );
        assert_eq!(f(None, Some("Sequoiadendron giganteum")), None);
        assert_eq!(f(None, None), None);
        assert_eq!(OsmLeaf::parse("palm"), Some(OsmLeaf::Palm));
        assert_eq!(OsmLeaf::parse("mixed"), None);
    }

    #[test]
    fn regions_by_place() {
        assert_eq!(FloraRegion::at(47.14, 9.52), FloraRegion::AlpineRhine);
        assert_eq!(FloraRegion::at(47.99, 7.85), FloraRegion::BlackForest);
        assert_eq!(FloraRegion::at(28.27, -16.64), FloraRegion::Tenerife);
        assert_eq!(FloraRegion::at(52.5, 13.4), FloraRegion::Other);
        // Puerto de la Cruz, La Laguna, Anaga and Teno: windward; Los
        // Cristianos, Vilaflor, Güímar's south coast: leeward.
        assert!(windward(28.414, -16.549));
        assert!(windward(28.487, -16.316));
        assert!(windward(28.55, -16.20));
        assert!(windward(28.37, -16.86));
        assert!(!windward(28.052, -16.716));
        assert!(!windward(28.155, -16.635));
        assert!(!windward(28.10, -16.50));
    }

    #[test]
    fn liechtenstein_conifers_by_altitude() {
        // Vaduz: spruces with pines on the dry slopes and some firs; the
        // montane belt spruce and fir; Malbun spruce; above the tree line
        // mountain pine.
        let low = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 500.0),
            LeafGroup::Evergreen,
        );
        assert!(
            low[&ScotsPine] > 0.25 && low[&NorwaySpruce] > 0.4,
            "{low:?}"
        );
        let mid = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 1000.0),
            LeafGroup::Evergreen,
        );
        assert!(mid[&SilverFir] > 0.3 && !mid.contains_key(&MountainPine));
        let malbun = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 1700.0),
            LeafGroup::Evergreen,
        );
        assert!(malbun[&NorwaySpruce] > 0.8 && !malbun.contains_key(&SilverFir));
        let high = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 2000.0),
            LeafGroup::Evergreen,
        );
        assert!(high[&MountainPine] > 0.5);
        // Broadleaves: beech below, sycamore and green alder near the line.
        let b = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 800.0),
            LeafGroup::Deciduous,
        );
        assert!(b[&Beech] > 0.5);
        let b = shares(
            &site(FloraRegion::AlpineRhine, Setting::Forest, 1700.0),
            LeafGroup::Deciduous,
        );
        assert!(!b.contains_key(&Beech) && b[&GreenAlder] > 0.3);
        // Larches stay larches.
        assert_eq!(
            pick_species(
                &site(FloraRegion::AlpineRhine, Setting::Forest, 1500.0),
                LeafGroup::Larch,
                0.3
            ),
            Larch
        );
    }

    #[test]
    fn river_banks_orchards_and_meadows() {
        let mut s = site(FloraRegion::AlpineRhine, Setting::Forest, 440.0);
        s.near_water = true;
        let w = shares(&s, LeafGroup::Deciduous);
        assert!(w[&Alder] + w[&Willow] > 0.85);
        let o = shares(
            &site(FloraRegion::BlackForest, Setting::Orchard, 300.0),
            LeafGroup::Deciduous,
        );
        assert_eq!(o.get(&FruitTree), Some(&1.0));
        let m = shares(
            &site(FloraRegion::BlackForest, Setting::Open, 400.0),
            LeafGroup::Deciduous,
        );
        assert!(m[&FruitTree] > 0.3, "{m:?}");
        let u = shares(
            &site(FloraRegion::AlpineRhine, Setting::Urban, 460.0),
            LeafGroup::Deciduous,
        );
        assert!(u[&Lime] > 0.3);
    }

    #[test]
    fn black_forest_has_douglas_fir_low_and_spruce_high() {
        let low = shares(
            &site(FloraRegion::BlackForest, Setting::Forest, 400.0),
            LeafGroup::Evergreen,
        );
        assert!(low[&DouglasFir] > 0.1);
        let feldberg = shares(
            &site(FloraRegion::BlackForest, Setting::Forest, 1350.0),
            LeafGroup::Evergreen,
        );
        assert!(feldberg[&NorwaySpruce] > 0.85 && !feldberg.contains_key(&DouglasFir));
        let oak = shares(
            &site(FloraRegion::BlackForest, Setting::Forest, 300.0),
            LeafGroup::Deciduous,
        );
        assert!(oak[&Oak] > 0.2 && oak[&Chestnut] > 0.05);
    }

    #[test]
    fn tenerife_zones() {
        let t = |setting, alt, north, measured| {
            let mut s = site(FloraRegion::Tenerife, setting, alt);
            s.windward = north;
            s.measured = measured;
            s
        };
        // Measured conifers: Canary pine anywhere.
        let s = t(
            Setting::Forest,
            1500.0,
            true,
            Some(DominantLeaf::Coniferous),
        );
        assert_eq!(pick_species(&s, LeafGroup::Evergreen, 0.4), CanaryPine);
        // Measured broadleaves in the north's cloud belt: laurel forest,
        // fayal-brezal above.
        let laurel = shares(
            &t(
                Setting::Forest,
                700.0,
                true,
                Some(DominantLeaf::Broadleaved),
            ),
            LeafGroup::Evergreen,
        );
        assert!(laurel[&Laurel] > 0.6, "{laurel:?}");
        let faya = shares(
            &t(
                Setting::Forest,
                1200.0,
                true,
                Some(DominantLeaf::Broadleaved),
            ),
            LeafGroup::Evergreen,
        );
        assert!(faya[&FayaHeath] > 0.5);
        // Unmeasured forest: laurels in the north at 800 m, pines in the
        // south at 800 m and in the north at 1 600 m.
        let s = t(Setting::Forest, 800.0, false, None);
        assert_eq!(pick_species(&s, LeafGroup::Evergreen, 0.4), CanaryPine);
        let s = t(Setting::Forest, 1600.0, true, None);
        assert_eq!(pick_species(&s, LeafGroup::Evergreen, 0.4), CanaryPine);
        let s = shares(&t(Setting::Forest, 800.0, true, None), LeafGroup::Evergreen);
        assert!(s[&Laurel] > 0.6);
        // Scrub: succulents on the coast (higher up on the dry south side),
        // broom in Las Cañadas.
        let coast = shares(&t(Setting::Scrub, 100.0, false, None), LeafGroup::Evergreen);
        assert!(coast[&Tabaiba] > 0.4 && coast[&Cardon] > 0.2);
        let south = shares(&t(Setting::Scrub, 600.0, false, None), LeafGroup::Evergreen);
        assert!(south.contains_key(&Cardon));
        let north = shares(&t(Setting::Scrub, 600.0, true, None), LeafGroup::Evergreen);
        assert!(north[&FayaHeath] > 0.5 && !north.contains_key(&Cardon));
        let canadas = shares(
            &t(Setting::Scrub, 2200.0, false, None),
            LeafGroup::Evergreen,
        );
        assert_eq!(canadas.get(&Broom), Some(&1.0));
        // Towns: palms on the coast.
        let town = shares(&t(Setting::Urban, 50.0, true, None), LeafGroup::Evergreen);
        assert!(town[&DatePalm] > 0.3 && town.contains_key(&DragonTree));
        // The summit is bare, broom shrinks towards it.
        assert_eq!(alive(FloraRegion::Tenerife, 3300.0), 0.0);
        assert_eq!(alive(FloraRegion::Tenerife, 2200.0), 1.0);
        assert_eq!(alive(FloraRegion::AlpineRhine, 3300.0), 1.0);
        assert!(stature(FloraRegion::Tenerife, Broom, 3000.0) < 0.8);
    }

    #[test]
    fn trees_are_stunted_at_the_tree_line() {
        let r = FloraRegion::AlpineRhine;
        assert_eq!(stature(r, NorwaySpruce, 1200.0), 1.0);
        let near = stature(r, NorwaySpruce, 1850.0);
        let above = stature(r, NorwaySpruce, 2100.0);
        assert!(near < 1.0 && above < near && above >= 0.4 - 1e-6);
        // Shrubs keep their size.
        assert_eq!(stature(r, MountainPine, 2100.0), 1.0);
        // The Feldberg's tree line is lower.
        assert!(stature(FloraRegion::BlackForest, NorwaySpruce, 1450.0) < 0.6);
    }

    #[test]
    fn choices_are_deterministic_and_cover_the_table() {
        let s = site(FloraRegion::BlackForest, Setting::Forest, 700.0);
        for i in 0..100 {
            let p = i as f32 / 100.0;
            assert_eq!(
                pick_species(&s, LeafGroup::Evergreen, p),
                pick_species(&s, LeafGroup::Evergreen, p)
            );
        }
        // The edges of the pick range.
        assert_eq!(choose(&[(Beech, 1.0), (Oak, 1.0)], 0.0), Beech);
        assert_eq!(choose(&[(Beech, 1.0), (Oak, 1.0)], 1.0), Oak);
        assert_eq!(choose(&[(Beech, 1.0), (Oak, 1.0)], 0.6), Oak);
    }
}
