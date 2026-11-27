//! The model contract per kind (spec §14 Phase 15, 15B): where a kind's
//! files live in `artwork/3dmodels/`, where its origin is, its triangle
//! budgets and the file-name pattern (15H: function first, then shape).
//! `artwork/3dmodels/README.md` has the same table for artists.

use serde::{Deserialize, Serialize};

/// What a model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A bike (one per physics profile; `profile` names it).
    Bike,
    /// The rider (shared rig).
    Rider,
    /// Worn on the rider at an anchor (helmets, glasses, caps).
    Wearable,
    /// A catalogue part, placed at a bike's `hs_<slot>` hotspot.
    Part,
    /// Placed in the world: bollards, gates, lamps, benches.
    Prop,
    /// A plant species' shape variant.
    Plant,
    /// The unicorn and other creatures.
    Creature,
    /// A piece of the workshop room.
    Workshop,
    /// Traffic (spec 14 Phase 18, 18B / 18E T6): cars, vans, lorries,
    /// buses, motorbikes, tractors, aircraft and the UFO; its class
    /// ([`VehicleClass`], `extras.ocw.class`) picks the budget and the
    /// required nodes.
    Vehicle,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Bike,
        Kind::Rider,
        Kind::Wearable,
        Kind::Part,
        Kind::Prop,
        Kind::Plant,
        Kind::Creature,
        Kind::Workshop,
        Kind::Vehicle,
    ];

    /// The manifest's name of the kind.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bike => "bike",
            Kind::Rider => "rider",
            Kind::Wearable => "wearable",
            Kind::Part => "part",
            Kind::Prop => "prop",
            Kind::Plant => "plant",
            Kind::Creature => "creature",
            Kind::Workshop => "workshop",
            Kind::Vehicle => "vehicle",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The folder of `artwork/3dmodels/` (15C).
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Bike => "bikes",
            Kind::Rider => "riders",
            Kind::Wearable => "wear",
            Kind::Part => "parts",
            Kind::Prop => "props",
            Kind::Plant => "flora",
            Kind::Creature => "creatures",
            Kind::Workshop => "workshop",
            Kind::Vehicle => "vehicles",
        }
    }

    /// Where the model's origin is (15B).
    pub fn origin(self) -> &'static str {
        match self {
            Kind::Bike => "ground contact under the bottom bracket",
            Kind::Rider => "between the sit bones",
            Kind::Wearable => "its anchor on the rider (`head`, `face`, `foot_l`, …)",
            Kind::Part => "its hotspot `hs_<slot>` on the bike",
            Kind::Prop => "the foot centre, on the ground",
            Kind::Plant => "the stem base",
            Kind::Creature => "under the body's centre",
            Kind::Workshop => {
                "the point where `workshop::room` places it: on the floor, its wall or the ceiling"
            }
            Kind::Vehicle => {
                "on the ground under the centre of the wheelbase (aircraft, balloon, UFO: under the body's centre)"
            }
        }
    }

    /// Triangle budgets: the whole file, and its `lod1` (15B; `None`: no
    /// far model).
    pub fn budget(self) -> (usize, Option<usize>) {
        match self {
            Kind::Bike => (15_000, Some(4_000)),
            Kind::Rider => (8_000, Some(4_000)),
            // The unicorn (Phase 13C: "about 3k near, 600 far") is 3,014 / 682.
            Kind::Creature => (3_500, Some(800)),
            Kind::Wearable => (1_500, None),
            Kind::Part => (1_500, None),
            Kind::Prop => (1_500, Some(300)),
            Kind::Plant => (2_000, Some(200)),
            Kind::Workshop => (10_000, None),
            // The largest class's (lorry, bus, airliner); the validator
            // checks each file against its class ([`VehicleClass::budget`]).
            Kind::Vehicle => (3_000, Some(400)),
        }
    }

    /// Whether a file of this kind must have a `lod1` node.
    pub fn needs_lod1(self) -> bool {
        matches!(self, Kind::Plant | Kind::Vehicle)
    }
}

/// The class of a vehicle file (`extras.ocw.class`, spec 18B *Models*):
/// its triangle budget and the nodes the game moves or lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VehicleClass {
    Car,
    Van,
    Lorry,
    Bus,
    Motorbike,
    Tractor,
    Airliner,
    SmallPlane,
    Glider,
    Helicopter,
    Balloon,
    Ufo,
}

/// The nodes of a four-wheeled road vehicle (18B): the wheels the game
/// spins and steers, its lamps and the collision box.
pub const ROAD_NODES: [&str; 13] = [
    "wheel_fl",
    "wheel_fr",
    "wheel_rl",
    "wheel_rr",
    "light_head_l",
    "light_head_r",
    "light_brake_l",
    "light_brake_r",
    "light_indicator_fl",
    "light_indicator_fr",
    "light_indicator_rl",
    "light_indicator_rr",
    "col_box",
];

/// A motorbike's nodes: two wheels, one head and one brake lamp, four
/// indicators, the rider placeholder and the collision box.
pub const MOTORBIKE_NODES: [&str; 10] = [
    "wheel_front",
    "wheel_rear",
    "light_head",
    "light_brake",
    "light_indicator_fl",
    "light_indicator_fr",
    "light_indicator_rl",
    "light_indicator_rr",
    "rider",
    "col_box",
];

/// The navigation lights of a powered aircraft: red on the left wing tip,
/// green on the right, white at the tail, the red beacon.
pub const NAV_NODES: [&str; 4] = ["light_nav_l", "light_nav_r", "light_tail", "light_beacon"];

/// The rim lights a UFO has at least (`light_rim_<n>`).
pub const UFO_RIM_LIGHTS: usize = 8;

impl VehicleClass {
    pub const ALL: [VehicleClass; 12] = [
        VehicleClass::Car,
        VehicleClass::Van,
        VehicleClass::Lorry,
        VehicleClass::Bus,
        VehicleClass::Motorbike,
        VehicleClass::Tractor,
        VehicleClass::Airliner,
        VehicleClass::SmallPlane,
        VehicleClass::Glider,
        VehicleClass::Helicopter,
        VehicleClass::Balloon,
        VehicleClass::Ufo,
    ];

    /// The name in `extras.ocw.class`.
    pub fn name(self) -> &'static str {
        match self {
            VehicleClass::Car => "car",
            VehicleClass::Van => "van",
            VehicleClass::Lorry => "lorry",
            VehicleClass::Bus => "bus",
            VehicleClass::Motorbike => "motorbike",
            VehicleClass::Tractor => "tractor",
            VehicleClass::Airliner => "airliner",
            VehicleClass::SmallPlane => "small_plane",
            VehicleClass::Glider => "glider",
            VehicleClass::Helicopter => "helicopter",
            VehicleClass::Balloon => "balloon",
            VehicleClass::Ufo => "ufo",
        }
    }

    pub fn parse(s: &str) -> Option<VehicleClass> {
        VehicleClass::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Triangles near (the file without `lod1`) and in `lod1` (18B).
    pub fn budget(self) -> (usize, usize) {
        match self {
            VehicleClass::Car | VehicleClass::Van => (2_000, 300),
            VehicleClass::Lorry | VehicleClass::Bus => (3_000, 400),
            VehicleClass::Motorbike => (1_500, 250),
            VehicleClass::Tractor => (2_000, 300),
            VehicleClass::Airliner => (3_000, 300),
            VehicleClass::SmallPlane | VehicleClass::Glider => (1_500, 200),
            VehicleClass::Helicopter => (2_500, 300),
            VehicleClass::Balloon => (1_000, 150),
            VehicleClass::Ufo => (1_500, 200),
        }
    }

    /// Whether it drives on the road (wheels on the ground, lamps, a
    /// collision box for 12B).
    pub fn on_road(self) -> bool {
        matches!(
            self,
            VehicleClass::Car
                | VehicleClass::Van
                | VehicleClass::Lorry
                | VehicleClass::Bus
                | VehicleClass::Motorbike
                | VehicleClass::Tractor
        )
    }

    /// The nodes a file of this class must have (besides `lod1`).
    pub fn required_nodes(self) -> Vec<&'static str> {
        match self {
            VehicleClass::Car
            | VehicleClass::Van
            | VehicleClass::Lorry
            | VehicleClass::Bus
            | VehicleClass::Tractor => ROAD_NODES.to_vec(),
            VehicleClass::Motorbike => MOTORBIKE_NODES.to_vec(),
            VehicleClass::Airliner => NAV_NODES.to_vec(),
            VehicleClass::SmallPlane => [NAV_NODES.as_slice(), &["prop"]].concat(),
            VehicleClass::Helicopter => {
                [NAV_NODES.as_slice(), &["rotor_main", "rotor_tail"]].concat()
            }
            VehicleClass::Glider | VehicleClass::Ufo => Vec::new(),
            VehicleClass::Balloon => vec!["light_burner"],
        }
    }

    /// The paint slot the game varies per vehicle by its look seed (18B:
    /// `body`; a bus wears its made-up operator's `livery`).
    pub fn paint_slot(self) -> &'static str {
        match self {
            VehicleClass::Bus => "livery",
            _ => "body",
        }
    }
}

/// The `extras.ocw.schema` this code writes and reads.
pub const SCHEMA: u64 = 1;

/// The validator's warning size for one model file (15C).
pub const WARN_FILE_BYTES: usize = 2 * 1024 * 1024;

/// The id of a model file: its path in `artwork/3dmodels/` without
/// `.glb` / `.gltf`.
pub fn id_of(file: &str) -> &str {
    file.strip_suffix(".glb")
        .or_else(|| file.strip_suffix(".gltf"))
        .unwrap_or(file)
}

/// Checks a file path against the naming rule of 15H:
/// `<kind dir>/[<slot>/]<function>-<shape>[-<detail>][-<variant>].glb`,
/// lower-case ASCII letters and digits with hyphens, no colours or
/// versions in it (colours are paint slots).
pub fn check_name(file: &str, kind: Kind) -> Result<(), String> {
    let Some(stem) = file.strip_suffix(".glb") else {
        return Err("a model file ends in `.glb`".into());
    };
    let parts: Vec<&str> = stem.split('/').collect();
    let depth = if kind == Kind::Part { 3 } else { 2 };
    if parts.len() != depth || parts[0] != kind.dir() {
        let want = if kind == Kind::Part {
            format!("{}/<slot>/<name>.glb", kind.dir())
        } else {
            format!("{}/<name>.glb", kind.dir())
        };
        return Err(format!("a {} lives in {want}", kind.name()));
    }
    let name = parts[depth - 1];
    let words: Vec<&str> = name.split('-').collect();
    let plain = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    if words.len() < 2 || !words.iter().all(|w| plain(w)) {
        return Err(format!(
            "`{name}`: function first, then shape (`function-shape[-detail][-variant]`), lower-case letters and digits with hyphens"
        ));
    }
    const COLOURS: [&str; 10] = [
        "red", "green", "blue", "black", "white", "yellow", "orange", "grey", "gray", "pink",
    ];
    if let Some(w) = words.iter().find(|w| COLOURS.contains(w)) {
        return Err(format!(
            "`{name}`: no colours in names (`{w}`): colours are paint slots"
        ));
    }
    if words
        .iter()
        .any(|w| w.len() > 1 && w.starts_with('v') && w[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("`{name}`: no version numbers in names"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_function_then_shape() {
        assert!(check_name("bikes/bike-road-endurance.glb", Kind::Bike).is_ok());
        assert!(check_name("flora/tree-spruce-conical-a.glb", Kind::Plant).is_ok());
        assert!(check_name("parts/wheel/wheel-carbon-deep-60mm.glb", Kind::Part).is_ok());
        assert!(check_name("bikes/bike.glb", Kind::Bike).is_err());
        assert!(check_name("riders/bike-road.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/Bike-Road.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike-road-red.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike-road-v2.glb", Kind::Bike).is_err());
        assert!(check_name("bikes/bike_road.glb", Kind::Bike).is_err());
        assert!(check_name("parts/wheel-carbon.glb", Kind::Part).is_err());
        assert_eq!(
            id_of("wear/helmet-tt-teardrop.glb"),
            "wear/helmet-tt-teardrop"
        );
    }

    #[test]
    fn every_kind_has_a_folder_and_a_budget() {
        let mut dirs: Vec<&str> = Kind::ALL.iter().map(|k| k.dir()).collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), Kind::ALL.len());
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
            assert!(k.budget().0 > 0 && !k.origin().is_empty());
        }
    }

    /// The budgets of 18B per class, within the kind's; names round-trip.
    #[test]
    fn every_vehicle_class_has_its_budget_and_nodes() {
        let (near, far) = Kind::Vehicle.budget();
        for c in VehicleClass::ALL {
            assert_eq!(VehicleClass::parse(c.name()), Some(c));
            let (n, f) = c.budget();
            assert!(n <= near && f <= far.unwrap() && f < n, "{c:?}");
            if c.on_road() {
                assert!(c.required_nodes().contains(&"col_box"), "{c:?}");
            }
        }
        assert_eq!(VehicleClass::Car.budget(), (2_000, 300));
        assert_eq!(VehicleClass::Bus.budget(), (3_000, 400));
        assert_eq!(VehicleClass::Motorbike.budget(), (1_500, 250));
        assert_eq!(VehicleClass::Balloon.budget(), (1_000, 150));
        assert_eq!(VehicleClass::Ufo.budget(), (1_500, 200));
        assert!(VehicleClass::parse("tank").is_none());
        assert!(check_name("vehicles/car-hatchback.glb", Kind::Vehicle).is_ok());
    }
}
