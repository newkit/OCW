//! Rider profiles (spec §3.2.1, Settings → Riders; chosen in the
//! Workshop): name, weight, optional height and FTP, and the bike and
//! look (spec §2.5, Workshop → Bike). Stored in `riders.toml` next to
//! `settings.toml`:
//!
//! ```toml
//! active = "Alex"
//!
//! [[rider]]
//! name = "Alex"
//! weight_kg = 78.0
//! height_cm = 182.0
//! ftp_w = 250
//! bike = "road-aero"
//! frame_color = "#1E88E5"
//! jersey_color = "#FF1472"
//! helmet_color = "#F2F2EE"
//! kit = "band"
//! body_build = "slim"
//! skin = 6
//! face = 1
//! hair = 3
//! hair_color = "#6B4426"
//! glasses = 2
//! kit_second = "#FDD835"
//! kit_third = "#1E1E22"
//! race_number = 42
//! helmet_pattern = true
//! shorts_color = "#3949AB"
//! helmet_finish = "matte"
//! ```
//!
//! The bike and look keys are optional (files from before Phase 4 have
//! none: the default road bike and colours). The active rider's weight is
//! the rider mass of the physics (sent to the server in the hello, §6.1)
//! and, with the bike, the trainer's SIM mass; `bike` names one of the
//! curated physics profiles (`ocw_core::ProfileId`), which also picks the
//! bike model; the colours and kit are cosmetic only. The character keys
//! (`body_build`, `skin`, `face`, `hair`, `hair_color`, `glasses`; with
//! `height_cm` the body height, spec §14 Phase 4, 4C) are free looks and
//! never physics. The kit design keys (`kit_second`, `kit_third`,
//! `race_number`, `helmet_pattern`, `shorts_color`, `socks_color`,
//! `shoes_color`, `gloves_color`, `helmet_finish`, `shoe_finish`; spec §14
//! Phase 8 T6, the jersey designer) are free looks too; unset: the plain
//! kit. Bevy-free.

use std::path::Path;

use ocw_core::{
    BaseBody, BodyBuild, BodyShape, Character, Finish, KitDesign, KitPattern, MAX_RIDER_WEIGHT_KG,
    MIN_RIDER_WEIGHT_KG, ProfileId, Rgb, RiderLook, RiderProfile,
};
use serde::{Deserialize, Serialize};

/// Accepted body heights in cm.
pub const HEIGHT_RANGE_CM: (f64, f64) = (100.0, 250.0);
/// Accepted FTP values in W.
pub const FTP_RANGE_W: (u32, u32) = (30, 700);
/// Longest rider name in characters.
pub const MAX_NAME_CHARS: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rider {
    pub name: String,
    pub weight_kg: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_cm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_w: Option<u32>,
    /// Bike and look (spec §2.5); `None` = the default. Kept as the
    /// file's text and checked on loading ([`Riders::parse`] drops
    /// invalid values with a warning); read them with [`Rider::bike`] and
    /// [`Rider::look`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bike: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jersey_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helmet_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    /// The character (spec 4C; `None` = the default's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hair: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hair_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glasses: Option<u8>,
    /// The refined character (spec §14 Phase 4, 4N N11): version 2 of
    /// the character keys, readable ids. `body_build`, `face` and `hair`
    /// above stay written for older apps (the nearest build, the W4 face
    /// of the facial hair, the style's index); a file without
    /// `character_version` is version 1 and is migrated on reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_version: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hair_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greying: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moustache: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beard: Option<String>,
    /// The eye colour's id (4N *Faces*: "brown", "dark-brown", "hazel",
    /// "amber", "green", "blue", "grey"; absent: brown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eyes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facial_hair_color: Option<String>,
    /// The skin tone in the 16-step palette (spec §14 Phase 4, 4N, v4.51);
    /// `skin` above keeps the 12-step palette's nearest for older apps,
    /// and a file without `skin_tone` is migrated from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin_tone: Option<u8>,
    /// The kit design (spec §14 Phase 8 T6; `None` = the plain kit's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_second: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_third: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_number: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helmet_pattern: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shorts_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoes_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gloves_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helmet_finish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoe_finish: Option<String>,
}

impl Default for Rider {
    /// The default rider: 75 kg, as the physics default (§3.2.1).
    fn default() -> Self {
        Self {
            name: "Rider".into(),
            weight_kg: RiderProfile::default().rider_mass_kg,
            height_cm: None,
            ftp_w: None,
            bike: None,
            frame_color: None,
            jersey_color: None,
            helmet_color: None,
            kit: None,
            body_build: None,
            skin: None,
            face: None,
            hair: None,
            hair_color: None,
            glasses: None,
            character_version: None,
            body: None,
            shape: None,
            age: None,
            hair_style: None,
            greying: None,
            moustache: None,
            beard: None,
            eyes: None,
            facial_hair_color: None,
            skin_tone: None,
            kit_second: None,
            kit_third: None,
            race_number: None,
            helmet_pattern: None,
            shorts_color: None,
            socks_color: None,
            shoes_color: None,
            gloves_color: None,
            helmet_finish: None,
            shoe_finish: None,
        }
    }
}

impl Rider {
    /// The physics profile this rider rides with (spec §2.5): the curated
    /// profile of the bike with the rider's (clamped) weight.
    pub fn physics(&self) -> RiderProfile {
        RiderProfile::curated(self.bike(), self.weight_kg)
    }

    /// The bike's physics profile (the default road bike if none is set).
    pub fn bike(&self) -> ProfileId {
        self.bike
            .as_deref()
            .and_then(ProfileId::parse)
            .unwrap_or_default()
    }

    /// The cosmetic look; unset parts are the default look's.
    pub fn look(&self) -> RiderLook {
        let d = RiderLook::default();
        let colour = |v: &Option<String>, d| v.as_deref().and_then(Rgb::parse).unwrap_or(d);
        RiderLook {
            frame: colour(&self.frame_color, d.frame),
            jersey: colour(&self.jersey_color, d.jersey),
            helmet: colour(&self.helmet_color, d.helmet),
            pattern: self
                .kit
                .as_deref()
                .and_then(KitPattern::parse)
                .unwrap_or(d.pattern),
            character: self.character(),
            design: self.design(),
        }
    }

    /// The kit design (spec §14 Phase 8 T6): unset keys are the plain
    /// kit's.
    pub fn design(&self) -> KitDesign {
        let colour = |v: &Option<String>| v.as_deref().and_then(Rgb::parse);
        let finish = |v: &Option<String>| v.as_deref().and_then(Finish::parse).unwrap_or_default();
        KitDesign {
            second: colour(&self.kit_second),
            third: colour(&self.kit_third),
            number: self.race_number.unwrap_or(0),
            helmet_pattern: self.helmet_pattern.unwrap_or(false),
            shorts: colour(&self.shorts_color),
            socks: colour(&self.socks_color),
            shoes: colour(&self.shoes_color),
            gloves: colour(&self.gloves_color),
            helmet_finish: finish(&self.helmet_finish),
            shoe_finish: finish(&self.shoe_finish),
        }
        .validated()
    }

    /// Sets the kit design; default values leave their keys out.
    pub fn set_design(&mut self, d: KitDesign) {
        let d = d.validated();
        let colour = |c: Option<Rgb>| c.map(|c| c.to_string());
        let finish = |f: Finish| (f != Finish::default()).then(|| f.id().to_string());
        self.kit_second = colour(d.second);
        self.kit_third = colour(d.third);
        self.race_number = (d.number > 0).then_some(d.number);
        self.helmet_pattern = d.helmet_pattern.then_some(true);
        self.shorts_color = colour(d.shorts);
        self.socks_color = colour(d.socks);
        self.shoes_color = colour(d.shoes);
        self.gloves_color = colour(d.gloves);
        self.helmet_finish = finish(d.helmet_finish);
        self.shoe_finish = finish(d.shoe_finish);
    }

    /// Whether the character keys are version 2 (4N N11).
    fn character_v2(&self) -> bool {
        self.character_version.is_some_and(|v| v >= 2)
    }

    /// The character (spec 4C, 4N): the height from the profile, the rest
    /// from the character keys (unset: the default's); a version-1 file
    /// is migrated (N11: the build to the shape, the face's facial hair
    /// to moustache and beard). Any shape (4N Q23: weight and height only
    /// propose one, [`Rider::weight_shape`]).
    pub fn character(&self) -> Character {
        use ocw_core::character::{self as ch, Age, BEARDS, MOUSTACHES};
        let d = Character::default();
        let v2 = self.character_v2();
        let shape = self
            .shape
            .as_deref()
            .and_then(BodyShape::parse)
            .or_else(|| {
                self.body_build
                    .as_deref()
                    .and_then(BodyBuild::parse)
                    .map(BodyShape::from_build)
            })
            .unwrap_or(d.shape);
        let (moustache, beard) = if v2 {
            let pick = |v: &Option<String>, list: &[&str]| {
                v.as_deref()
                    .and_then(|v| ch::list_parse(list, v))
                    .unwrap_or(0)
            };
            (
                pick(&self.moustache, &MOUSTACHES),
                pick(&self.beard, &BEARDS),
            )
        } else {
            ch::legacy_face(self.face.unwrap_or(0))
        };
        Character {
            height_cm: d.height_cm,
            base: self
                .body
                .as_deref()
                .and_then(BaseBody::parse)
                .unwrap_or(d.base),
            shape,
            age: self.age.as_deref().and_then(Age::parse).unwrap_or(d.age),
            skin: self
                .skin_tone
                .or_else(|| {
                    self.skin
                        .filter(|s| usize::from(*s) < ocw_core::character::LEGACY_SKIN_TONES.len())
                        .map(ocw_core::character::skin_from_legacy)
                })
                .unwrap_or(d.skin),
            face: if v2 {
                self.face.unwrap_or(d.face)
            } else {
                d.face
            },
            hair: self
                .hair_style
                .as_deref()
                .and_then(ch::hair_parse)
                .or(self.hair)
                .unwrap_or(d.hair),
            hair_colour: self
                .hair_color
                .as_deref()
                .and_then(Rgb::parse)
                .unwrap_or(d.hair_colour),
            greying: self.greying.unwrap_or(d.greying),
            moustache,
            beard,
            facial_hair_colour: self.facial_hair_color.as_deref().and_then(Rgb::parse),
            glasses: self.glasses.unwrap_or(d.glasses),
            eyes: self
                .eyes
                .as_deref()
                .and_then(|s| ch::list_parse(&ch::EYES, s))
                .unwrap_or(d.eyes),
        }
        .with_height(self.height_cm)
    }

    /// The *Match my weight* shape of this profile (4N Q13, Q23): a
    /// proposal – a new rider starts with it –, never a limit.
    pub fn weight_shape(&self) -> BodyShape {
        ocw_core::character::weight_shape(self.weight_kg, self.height_cm)
    }

    /// Sets the character's free choices (the height stays the profile's).
    /// Writes version 2 of the keys, and the W4 keys for older apps.
    pub fn set_character(&mut self, c: Character) {
        use ocw_core::character::{self as ch, BEARDS, MOUSTACHES};
        let c = c.validated().with_height(self.height_cm);
        self.character_version = Some(2);
        self.body = Some(c.base.id().into());
        self.shape = Some(c.shape.id().into());
        self.age = Some(c.age.id().into());
        self.hair_style = Some(ch::hair_id(c.hair));
        self.greying = (!c.greying).then_some(false);
        self.moustache = Some(ch::list_id(&MOUSTACHES, c.moustache));
        self.beard = Some(ch::list_id(&BEARDS, c.beard));
        self.eyes = (c.eyes != 0).then(|| ch::list_id(&ch::EYES, c.eyes));
        self.facial_hair_color = c.facial_hair_colour.map(|c| c.to_string());
        // For older apps (N11).
        self.body_build = Some(c.shape.nearest_build().id().into());
        self.skin = Some(ocw_core::character::skin_to_legacy(c.skin));
        self.skin_tone = Some(c.skin);
        self.face = Some(c.legacy_face());
        self.hair = Some(c.hair);
        self.hair_color = Some(c.hair_colour.to_string());
        self.glasses = Some(c.glasses);
    }

    /// Sets the bike's physics profile.
    pub fn set_bike(&mut self, bike: ProfileId) {
        self.bike = Some(bike.id().into());
    }

    /// Sets the look (all parts written to the file).
    pub fn set_look(&mut self, look: RiderLook) {
        self.frame_color = Some(look.frame.to_string());
        self.jersey_color = Some(look.jersey.to_string());
        self.helmet_color = Some(look.helmet.to_string());
        self.kit = Some(look.pattern.id().into());
        self.set_design(look.design);
    }

    /// "Road - aero, frame #1E88E5, jersey #FF1472 (band)".
    pub fn bike_summary(&self) -> String {
        let l = self.look();
        let mut s = format!(
            "{}, frame {}, jersey {}",
            self.bike().name(),
            l.frame,
            l.jersey
        );
        if l.pattern != KitPattern::Plain {
            s.push_str(&format!(" ({})", l.pattern.id()));
        }
        s
    }

    /// "78 kg, 182 cm, FTP 250 W".
    pub fn summary(&self) -> String {
        let mut s = format!("{} kg", fmt_num(self.weight_kg));
        if let Some(h) = self.height_cm {
            s.push_str(&format!(", {} cm", fmt_num(h)));
        }
        if let Some(f) = self.ftp_w {
            s.push_str(&format!(", FTP {f} W"));
        }
        s
    }

    /// Checks a profile entered by the user.
    pub fn validate(&self) -> Result<(), String> {
        check_name(&self.name)?;
        check_weight(self.weight_kg)?;
        if let Some(h) = self.height_cm
            && !(HEIGHT_RANGE_CM.0..=HEIGHT_RANGE_CM.1).contains(&h)
        {
            return Err(format!(
                "height must be {} to {} cm",
                HEIGHT_RANGE_CM.0, HEIGHT_RANGE_CM.1
            ));
        }
        if let Some(f) = self.ftp_w
            && !(FTP_RANGE_W.0..=FTP_RANGE_W.1).contains(&f)
        {
            return Err(format!(
                "FTP must be {} to {} W",
                FTP_RANGE_W.0, FTP_RANGE_W.1
            ));
        }
        Ok(())
    }
}

impl Rider {
    /// Drops bike and look values that are not valid (unknown bike or kit,
    /// not `#RRGGBB`) and writes valid ones in their canonical form;
    /// returns warnings for the dropped ones.
    fn repair_look(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let name = self.name.clone();
        let mut check = |field: &mut Option<String>,
                         what: &str,
                         canon: &dyn Fn(&str) -> Option<String>| {
            if let Some(v) = field.as_deref() {
                match canon(v) {
                    Some(c) => *field = Some(c),
                    None => {
                        warnings.push(format!("rider {name:?}: {what} {v:?} ignored (unknown)"));
                        *field = None;
                    }
                }
            }
        };
        check(&mut self.bike, "bike", &|v| {
            ProfileId::parse(v).map(|p| p.id().into())
        });
        let colour = |v: &str| Rgb::parse(v).map(|c| c.to_string());
        check(&mut self.frame_color, "frame colour", &colour);
        check(&mut self.jersey_color, "jersey colour", &colour);
        check(&mut self.helmet_color, "helmet colour", &colour);
        check(&mut self.kit, "kit", &|v| {
            KitPattern::parse(v).map(|k| k.id().into())
        });
        check(&mut self.body_build, "body build", &|v| {
            BodyBuild::parse(v).map(|b| b.id().into())
        });
        check(&mut self.hair_color, "hair colour", &colour);
        // The 4N keys (N11): unknown values are warned about and take the
        // default.
        {
            use ocw_core::character::{self as ch, Age, BEARDS, MOUSTACHES};
            check(&mut self.facial_hair_color, "facial hair colour", &colour);
            check(&mut self.body, "body", &|v| {
                BaseBody::parse(v).map(|b| b.id().into())
            });
            check(&mut self.shape, "body shape", &|v| {
                BodyShape::parse(v).map(|b| b.id().into())
            });
            check(&mut self.age, "age", &|v| {
                Age::parse(v).map(|a| a.id().into())
            });
            check(&mut self.hair_style, "hair style", &|v| {
                ch::hair_parse(v).map(ch::hair_id)
            });
            check(&mut self.moustache, "moustache", &|v| {
                ch::list_parse(&MOUSTACHES, v).map(|i| ch::list_id(&MOUSTACHES, i))
            });
            check(&mut self.beard, "beard", &|v| {
                ch::list_parse(&BEARDS, v).map(|i| ch::list_id(&BEARDS, i))
            });
            check(&mut self.eyes, "eye colour", &|v| {
                ch::list_parse(&ch::EYES, v).map(|i| ch::list_id(&ch::EYES, i))
            });
        }
        for (field, what) in [
            (&mut self.kit_second, "second kit colour"),
            (&mut self.kit_third, "third kit colour"),
            (&mut self.shorts_color, "shorts colour"),
            (&mut self.socks_color, "socks colour"),
            (&mut self.shoes_color, "shoes colour"),
            (&mut self.gloves_color, "gloves colour"),
        ] {
            check(field, what, &colour);
        }
        let finish = |v: &str| Finish::parse(v).map(|f| f.id().to_string());
        check(&mut self.helmet_finish, "helmet finish", &finish);
        check(&mut self.shoe_finish, "shoe finish", &finish);
        if let Some(n) = self.race_number
            && n > ocw_core::kit::MAX_RACE_NUMBER
        {
            warnings.push(format!(
                "rider {name:?}: race number {n} ignored (1 to 999)"
            ));
            self.race_number = None;
        }
        let c = self.character();
        let v = c.validated();
        // A version-1 face is the W4 face (0 … 3: the facial hair).
        let face_ok = if self.character_v2() {
            v.face
        } else {
            self.face
                .filter(|f| (*f as usize) < ocw_core::character::LEGACY_FACES.len())
                .unwrap_or(0)
        };
        let hair_ok = self
            .hair
            .filter(|h| (*h as usize) < HAIR_STYLES_LEN)
            .unwrap_or(v.hair);
        let skin_ok = self
            .skin
            .filter(|s| usize::from(*s) < ocw_core::character::LEGACY_SKIN_TONES.len())
            .unwrap_or(u8::MAX);
        for (what, field, ok) in [
            ("skin", &mut self.skin, skin_ok),
            ("skin tone", &mut self.skin_tone, v.skin),
            ("face", &mut self.face, face_ok),
            ("hair", &mut self.hair, hair_ok),
            ("glasses", &mut self.glasses, v.glasses),
        ] {
            if let Some(x) = *field
                && x != ok
            {
                warnings.push(format!("rider {name:?}: {what} {x} ignored (unknown)"));
                *field = None;
            }
        }
        warnings
    }
}

const HAIR_STYLES_LEN: usize = ocw_core::character::HAIR_STYLES.len();

/// A number without a trailing ".0".
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.trim() != name {
        return Err("a rider name must not be empty or start / end with spaces".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "a rider name has at most {MAX_NAME_CHARS} characters"
        ));
    }
    // Names are quoted in console command lines.
    if name.contains('"') || name.chars().any(char::is_control) {
        return Err("a rider name must not contain quotes".into());
    }
    Ok(())
}

pub fn check_weight(kg: f64) -> Result<(), String> {
    if (MIN_RIDER_WEIGHT_KG..=MAX_RIDER_WEIGHT_KG).contains(&kg) {
        Ok(())
    } else {
        Err(format!(
            "weight must be {MIN_RIDER_WEIGHT_KG} to {MAX_RIDER_WEIGHT_KG} kg"
        ))
    }
}

/// Changes to a rider (`riders.edit`); `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiderEdit {
    pub name: Option<String>,
    pub weight_kg: Option<f64>,
    /// `Some(None)` clears the height.
    pub height_cm: Option<Option<f64>>,
    pub ftp_w: Option<Option<u32>>,
    pub bike: Option<ProfileId>,
    pub frame: Option<Rgb>,
    pub jersey: Option<Rgb>,
    pub helmet: Option<Rgb>,
    pub pattern: Option<KitPattern>,
}

/// The rider profiles and which one is active.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Riders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default, rename = "rider")]
    pub riders: Vec<Rider>,
}

impl Riders {
    /// Reads the profiles; returns them and warnings for entries that had to
    /// be repaired (weight clamped, nameless or duplicate riders dropped).
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let mut r: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        let warnings = r.repair();
        Ok((r, warnings))
    }

    /// Reads the file; a missing file means no profiles (the default rider).
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("riders serialize")
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_toml()).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn repair(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut kept: Vec<Rider> = Vec::new();
        for mut r in std::mem::take(&mut self.riders) {
            if let Err(e) = check_name(&r.name) {
                warnings.push(format!("rider {:?} dropped: {e}", r.name));
                continue;
            }
            if kept.iter().any(|k| k.name.eq_ignore_ascii_case(&r.name)) {
                warnings.push(format!(
                    "rider {:?} dropped: the name is used twice",
                    r.name
                ));
                continue;
            }
            if check_weight(r.weight_kg).is_err() {
                let w = ocw_core::clamp_rider_weight(r.weight_kg);
                warnings.push(format!(
                    "rider {:?}: weight {} kg out of range, using {w} kg",
                    r.name, r.weight_kg
                ));
                r.weight_kg = w;
            }
            if r.height_cm
                .is_some_and(|h| !(HEIGHT_RANGE_CM.0..=HEIGHT_RANGE_CM.1).contains(&h))
            {
                warnings.push(format!("rider {:?}: height ignored (out of range)", r.name));
                r.height_cm = None;
            }
            if r.ftp_w
                .is_some_and(|f| !(FTP_RANGE_W.0..=FTP_RANGE_W.1).contains(&f))
            {
                warnings.push(format!("rider {:?}: FTP ignored (out of range)", r.name));
                r.ftp_w = None;
            }
            warnings.extend(r.repair_look());
            kept.push(r);
        }
        self.riders = kept;
        if let Some(a) = &self.active
            && self.find(a).is_none()
        {
            warnings.push(format!("active rider {a:?} not found, using the first"));
            self.active = None;
        }
        warnings
    }

    /// Index of the rider named `name` (case does not matter).
    pub fn find(&self, name: &str) -> Option<usize> {
        self.riders
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// The active rider: the selected one, else the first, else the
    /// default rider (75 kg).
    pub fn active(&self) -> Rider {
        self.active
            .as_deref()
            .and_then(|a| self.find(a))
            .or(if self.riders.is_empty() {
                None
            } else {
                Some(0)
            })
            .map(|i| self.riders[i].clone())
            .unwrap_or_default()
    }

    /// Whether `name` is the active rider.
    pub fn is_active(&self, name: &str) -> bool {
        !self.riders.is_empty() && self.active().name.eq_ignore_ascii_case(name)
    }

    /// Adds a rider; the first one added becomes the active one. A new
    /// name follows the rules of `ocw_core::name` (spec §14 Phase 4, 4N
    /// N1: 2–20 characters of any script, no invisible characters, not
    /// reserved), normalized.
    pub fn add(&mut self, rider: Rider) -> Result<(), String> {
        let mut rider = rider;
        rider.name = ocw_core::name::check(&rider.name).map_err(|e| e.to_string())?;
        rider.validate()?;
        // A new rider starts with the shape weight and height propose
        // (4N Q23), unless it brings one.
        if rider.shape.is_none() && rider.body_build.is_none() {
            rider.shape = Some(rider.weight_shape().id().into());
        }
        if self.find(&rider.name).is_some() {
            return Err(format!("there is already a rider {:?}", rider.name));
        }
        if self.riders.is_empty() {
            self.active = Some(rider.name.clone());
        }
        self.riders.push(rider);
        Ok(())
    }

    /// Changes a rider; returns the changed profile.
    pub fn edit(&mut self, name: &str, edit: RiderEdit) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        let mut r = self.riders[i].clone();
        if let Some(n) = &edit.name {
            // A new name follows the 4N rules (old names still load).
            let n = ocw_core::name::check(n).map_err(|e| e.to_string())?;
            if self.find(&n).is_some_and(|j| j != i) {
                return Err(format!("there is already a rider {n:?}"));
            }
            r.name = n;
        }
        if let Some(w) = edit.weight_kg {
            r.weight_kg = w;
        }
        if let Some(h) = edit.height_cm {
            r.height_cm = h;
        }
        if let Some(f) = edit.ftp_w {
            r.ftp_w = f;
        }
        if let Some(b) = edit.bike {
            r.set_bike(b);
        }
        if edit.frame.is_some()
            || edit.jersey.is_some()
            || edit.helmet.is_some()
            || edit.pattern.is_some()
        {
            let l = r.look();
            r.set_look(RiderLook {
                frame: edit.frame.unwrap_or(l.frame),
                jersey: edit.jersey.unwrap_or(l.jersey),
                helmet: edit.helmet.unwrap_or(l.helmet),
                pattern: edit.pattern.unwrap_or(l.pattern),
                ..l
            });
        }
        r.validate()?;
        if self.is_active(name) {
            self.active = Some(r.name.clone());
        }
        self.riders[i] = r.clone();
        Ok(r)
    }

    /// Deletes a rider. Deleting the active rider makes the first one
    /// active.
    pub fn remove(&mut self, name: &str) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        let was_active = self.is_active(name);
        let r = self.riders.remove(i);
        if was_active {
            self.active = self.riders.first().map(|r| r.name.clone());
        }
        Ok(r)
    }

    /// Makes `name` the active rider.
    pub fn select(&mut self, name: &str) -> Result<Rider, String> {
        let i = self.find(name).ok_or_else(|| no_rider(name))?;
        self.active = Some(self.riders[i].name.clone());
        Ok(self.riders[i].clone())
    }

    /// A name for a new rider that is not taken: a made-up name from the
    /// dice (spec §14 Phase 4, 4N N1: "Swift Climber"; "Rider N" is the
    /// servers' reserved fallback).
    pub fn new_name(&self) -> String {
        (self.riders.len() as u64..)
            .map(ocw_core::name::dice)
            .find(|n| self.find(n).is_none())
            .expect("a free name")
    }

    /// The dice's proposal for renaming rider `i` (4N N1): free, and not
    /// the name *Add a rider* proposes.
    pub fn dice_name(&self, i: usize) -> String {
        let add = self.new_name();
        (1000 + 7 * i as u64..)
            .map(ocw_core::name::dice)
            .find(|n| self.find(n).is_none() && *n != add)
            .expect("a free name")
    }
}

fn no_rider(name: &str) -> String {
    format!("no rider {name:?} (riders.list shows them)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider(name: &str, kg: f64) -> Rider {
        Rider {
            name: name.into(),
            weight_kg: kg,
            ..Rider::default()
        }
    }

    #[test]
    fn no_profiles_means_the_default_rider() {
        let r = Riders::default();
        assert_eq!(r.active(), Rider::default());
        assert_eq!(r.active().physics(), RiderProfile::default());
        assert!(!r.is_active("Rider"));
    }

    #[test]
    fn add_select_edit_remove() {
        let mut r = Riders::default();
        r.add(rider("Alex", 78.0)).unwrap();
        // The first rider becomes active.
        assert_eq!(r.active().name, "Alex");
        r.add(rider("Anna", 61.5)).unwrap();
        assert!(r.add(rider("anna", 60.0)).unwrap_err().contains("already"));
        assert!(r.add(rider("Heavy", 250.0)).unwrap_err().contains("weight"));
        assert!(r.add(rider("", 70.0)).is_err());
        assert!(r.add(rider("Say \"hi\"", 70.0)).is_err());
        assert_eq!(r.select("ANNA").unwrap().name, "Anna");
        assert_eq!(r.active().physics().rider_mass_kg, 61.5);
        assert_eq!(r.active().physics().total_mass_kg(), 69.5);
        // Renaming the active rider keeps it active.
        let e = r
            .edit(
                "anna",
                RiderEdit {
                    name: Some("Anna B".into()),
                    weight_kg: Some(62.0),
                    ftp_w: Some(Some(230)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(e.summary(), "62 kg, FTP 230 W");
        assert_eq!(r.active().name, "Anna B");
        let bad = RiderEdit {
            weight_kg: Some(10.0),
            ..Default::default()
        };
        assert!(r.edit("Anna B", bad).is_err());
        assert_eq!(r.active().weight_kg, 62.0);
        let clash = RiderEdit {
            name: Some("alex".into()),
            ..Default::default()
        };
        assert!(r.edit("Anna B", clash).is_err());
        assert!(r.edit("Nobody", RiderEdit::default()).is_err());
        // Removing the active rider activates the first.
        r.remove("Anna B").unwrap();
        assert_eq!(r.active().name, "Alex");
        r.remove("Alex").unwrap();
        assert_eq!(r.active(), Rider::default());
        assert!(r.remove("Alex").is_err());
    }

    #[test]
    fn toml_round_trip() {
        let mut r = Riders::default();
        r.add(Rider {
            name: "Alex".into(),
            weight_kg: 78.0,
            height_cm: Some(182.0),
            ftp_w: Some(250),
            ..Rider::default()
        })
        .unwrap();
        r.add(rider("Guest", 70.0)).unwrap();
        let text = r.to_toml();
        assert!(text.contains("active = \"Alex\""), "{text}");
        assert!(
            text.contains("[[rider]]") && text.contains("ftp_w = 250"),
            "{text}"
        );
        // No height / FTP keys for the guest.
        assert_eq!(text.matches("height_cm").count(), 1);
        let (again, warnings) = Riders::parse(&text).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(again, r);
        // No bike keys until a bike is chosen.
        assert!(!text.contains("bike"), "{text}");
    }

    /// Bike and look (spec §2.5): persisted per rider, read back, and only
    /// the bike reaches the physics.
    #[test]
    fn bike_and_look_round_trip_and_only_the_bike_is_physics() {
        let mut r = Riders::default();
        r.add(rider("Alex", 78.0)).unwrap();
        let plain = r.active().physics();
        let e = r
            .edit(
                "Alex",
                RiderEdit {
                    frame: Some(Rgb(0x1E88E5)),
                    jersey: Some(Rgb(0x43A047)),
                    pattern: Some(KitPattern::Band),
                    ..Default::default()
                },
            )
            .unwrap();
        // Cosmetic choices never change the physics profile.
        assert_eq!(e.physics(), plain);
        assert_eq!(e.look().frame, Rgb(0x1E88E5));
        assert_eq!(e.look().helmet, RiderLook::default().helmet);
        let e = r
            .edit(
                "Alex",
                RiderEdit {
                    bike: Some(ProfileId::Mountain),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            e.physics(),
            RiderProfile::curated(ProfileId::Mountain, 78.0)
        );
        assert_eq!(e.look().frame, Rgb(0x1E88E5), "the look stays");
        assert_eq!(
            e.bike_summary(),
            "Mountain bike, frame #1E88E5, jersey #43A047 (band)"
        );
        let text = r.to_toml();
        assert!(text.contains("bike = \"mtb\""), "{text}");
        assert!(text.contains("frame_color = \"#1E88E5\""), "{text}");
        assert!(text.contains("kit = \"band\""), "{text}");
        let (again, warnings) = Riders::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, r);
    }

    /// The jersey designer's look is saved in the rider profile (spec
    /// §14 Phase 8 T6): every field round-trips through riders.toml, the
    /// plain kit writes no keys, bad values are dropped with a warning.
    #[test]
    fn the_kit_design_round_trips_in_the_profile() {
        let mut r = Riders::default();
        r.add(rider("Alex", 78.0)).unwrap();
        let plain = r.to_toml();
        assert!(!plain.contains("race_number") && !plain.contains("kit_second"));
        let look = RiderLook {
            pattern: KitPattern::Gradient,
            design: KitDesign {
                second: Some(Rgb(0xFDD835)),
                third: Some(Rgb(0x1E1E22)),
                number: 42,
                helmet_pattern: true,
                shorts: Some(Rgb(0x3949AB)),
                socks: Some(Rgb(0xF2F2EE)),
                shoes: Some(Rgb(0x1E1E22)),
                gloves: Some(Rgb(0xE53935)),
                helmet_finish: Finish::Matte,
                shoe_finish: Finish::Matte,
            },
            ..r.active().look()
        };
        r.riders[0].set_look(look);
        let text = r.to_toml();
        assert!(text.contains("race_number = 42"), "{text}");
        assert!(text.contains("helmet_finish = \"matte\""), "{text}");
        assert!(text.contains("kit = \"gradient\""), "{text}");
        let (again, warnings) = Riders::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.active().look(), look);
        // Back to the plain kit: the keys go.
        let mut plain_again = again.clone();
        plain_again.riders[0].set_look(RiderLook {
            design: KitDesign::default(),
            ..look
        });
        assert!(!plain_again.to_toml().contains("race_number"));
        let bad = "[[rider]]\nname = \"B\"\nweight_kg = 70.0\nrace_number = 5000\n\
                   kit_second = \"teal\"\nhelmet_finish = \"satin\"\n";
        let (b, warnings) = Riders::parse(bad).unwrap();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(b.active().design(), KitDesign::default());
    }

    /// The skin tone (spec §14 Phase 4, 4N, v4.51): a file with only the
    /// 12-step `skin` index (versions 1 and 2) loads as the nearest of the
    /// 16 tones; saving writes `skin_tone` and the 12-step index for older
    /// apps; an unknown tone is dropped with a warning.
    #[test]
    fn skin_tones_migrate_from_the_twelve_step_palette() {
        use ocw_core::character::{SKIN_TONES, skin_from_legacy, skin_to_legacy};
        for old in 0..12u8 {
            let text = format!(
                "[[rider]]\nname = \"Ada Rider\"\nweight_kg = 70.0\ncharacter_version = 2\nskin = {old}\n"
            );
            let (r, warnings) = Riders::parse(&text).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(r.riders[0].character().skin, skin_from_legacy(old), "{old}");
        }
        for tone in 0..SKIN_TONES.len() as u8 {
            let mut rider = Rider {
                name: "Ada Rider".into(),
                ..Rider::default()
            };
            let c = Character {
                skin: tone,
                ..Character::default()
            };
            rider.set_character(c);
            assert_eq!(rider.skin_tone, Some(tone));
            assert_eq!(rider.skin, Some(skin_to_legacy(tone)));
            let riders = Riders {
                riders: vec![rider],
                ..Riders::default()
            };
            let (again, warnings) = Riders::parse(&riders.to_toml()).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(again.riders[0].character().skin, tone);
        }
        let bad = "[[rider]]\nname = \"Ada Rider\"\nweight_kg = 70.0\nskin_tone = 40\n";
        let (r, warnings) = Riders::parse(bad).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(r.riders[0].character().skin, Character::default().skin);
    }

    /// Files from before Phase 4 parse unchanged; invalid bike and look
    /// values are dropped with a warning, valid ones normalised.
    #[test]
    fn bike_values_are_checked_on_loading() {
        let old = "active = \"Alex\"\n[[rider]]\nname = \"Alex\"\nweight_kg = 78.0\n";
        let (r, warnings) = Riders::parse(old).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(r.active().bike(), ProfileId::Road);
        assert_eq!(r.active().look(), RiderLook::default());
        let text = "\
[[rider]]
name = \"Odd\"
weight_kg = 70.0
bike = \"Unicycle\"
frame_color = \"blue\"
jersey_color = \"#00ff00\"
kit = \"stars\"
helmet_color = \"#ABCDEF\"
";
        let (r, warnings) = Riders::parse(text).unwrap();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        let o = &r.riders[0];
        assert_eq!(o.bike, None);
        assert_eq!(o.frame_color, None);
        assert_eq!(o.jersey_color.as_deref(), Some("#00FF00"));
        assert_eq!(o.kit, None);
        assert_eq!(o.look().helmet, Rgb(0xABCDEF));
        let (r, _) =
            Riders::parse("[[rider]]\nname = \"A\"\nweight_kg = 70.0\nbike = \"TT\"\n").unwrap();
        assert_eq!(r.riders[0].bike.as_deref(), Some("tt"));
    }

    #[test]
    fn broken_entries_are_repaired_with_warnings() {
        let text = "\
active = \"Ghost\"
[[rider]]
name = \"Big\"
weight_kg = 500.0
height_cm = 20.0
[[rider]]
name = \"big\"
weight_kg = 70.0
[[rider]]
name = \"\"
weight_kg = 70.0
[[rider]]
name = \"Fit\"
weight_kg = 65.0
ftp_w = 5000
";
        let (r, warnings) = Riders::parse(text).unwrap();
        assert_eq!(warnings.len(), 6, "{warnings:?}");
        assert_eq!(r.riders.len(), 2);
        assert_eq!(r.riders[0].weight_kg, MAX_RIDER_WEIGHT_KG);
        assert_eq!(r.riders[0].height_cm, None);
        assert_eq!(r.riders[1].ftp_w, None);
        // The missing active rider: the first.
        assert_eq!(r.active().name, "Big");
        assert!(Riders::parse("[[rider]]\nname = 3").is_err());
    }

    #[test]
    fn new_names_are_free() {
        let mut r = Riders::default();
        let first = r.new_name();
        assert!(ocw_core::name::check(&first).is_ok(), "{first}");
        r.add(rider(&first, 70.0)).unwrap();
        let second = r.new_name();
        assert_ne!(first, second);
        r.add(rider(&second, 70.0)).unwrap();
        assert!(r.find(&r.new_name()).is_none());
    }

    /// The shape (spec §14 Phase 4, 4N Q23): a new rider starts with the
    /// shape weight and height propose; any shape can be set and is kept
    /// (saved and loaded) whatever the weight, also after the weight
    /// changes.
    #[test]
    fn any_shape_is_kept_and_a_new_rider_starts_with_the_proposal() {
        use ocw_core::BodyShape;
        let mut r = Riders::default();
        r.add(Rider {
            name: "Ada Rider".into(),
            weight_kg: 120.0,
            height_cm: Some(178.0),
            ..Rider::default()
        })
        .unwrap();
        assert_eq!(r.riders[0].character().shape, BodyShape::Obese);
        for s in BodyShape::ALL {
            let mut a = r.riders[0].clone();
            a.set_character(Character {
                shape: s,
                ..a.character()
            });
            assert_eq!(a.character().shape, s);
            let text = toml::to_string(&Riders {
                riders: vec![a.clone()],
                ..Riders::default()
            })
            .unwrap();
            let (back, _) = Riders::parse(&text).unwrap();
            assert_eq!(back.riders[0].character().shape, s, "{text}");
            a.weight_kg = 55.0;
            assert_eq!(a.character().shape, s, "the weight does not move it");
            assert_eq!(a.weight_shape(), BodyShape::Skinny);
        }
    }

    /// New names follow the 4N rules (spec §14 Phase 4, N1), normalized;
    /// files with old names (the 32-character rule) still load.
    #[test]
    fn new_names_follow_the_4n_rules() {
        let mut r = Riders::default();
        // "Rider N" is the servers' fallback, reserved.
        assert!(r.add(rider("Rider", 70.0)).is_err());
        assert!(r.add(rider("Rider 2", 70.0)).is_err());
        assert!(r.add(rider("Ada \u{1F6B4}", 70.0)).is_err());
        assert!(r.add(rider("An overly long rider name", 70.0)).is_err());
        r.add(rider("  Zoe\u{0308}  Fahrer ", 70.0)).unwrap();
        assert_eq!(r.riders[0].name, "Zoë Fahrer");
        r.add(rider("李小龍", 61.0)).unwrap();
        let rename = |to: &str| RiderEdit {
            name: Some(to.into()),
            ..RiderEdit::default()
        };
        assert!(r.edit("Zoë Fahrer", rename("Echo")).is_err());
        assert!(r.edit("Zoë Fahrer", rename("李小龍")).is_err());
        assert_eq!(
            r.edit("Zoë Fahrer", rename("Ada Rider")).unwrap().name,
            "Ada Rider"
        );
        let old = "[[rider]]\nname = \"Rider\"\nweight_kg = 70.0\n\
                   [[rider]]\nname = \"A rider with a long old name\"\nweight_kg = 71.0\n";
        let (loaded, warnings) = Riders::parse(old).unwrap();
        assert_eq!(loaded.riders.len(), 2, "{warnings:?}");
    }

    #[test]
    fn files_load_and_save() {
        let dir = std::env::temp_dir().join(format!("ocw-riders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("riders.toml");
        assert_eq!(Riders::load(&path).unwrap().0, Riders::default());
        let mut r = Riders::default();
        r.add(rider("Alex", 80.0)).unwrap();
        r.save(&path).unwrap();
        assert_eq!(Riders::load(&path).unwrap().0, r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The eye colour (4N *Faces*) is kept in the profile as `eyes`; a
    /// rider saved before has none and loads with brown eyes and the rest
    /// of the look as it was; brown is not written.
    #[test]
    fn the_eye_colour_is_saved_and_older_riders_load_with_brown() {
        let mut r = rider("Ada Rider", 62.0);
        let before = Character {
            beard: 4,
            moustache: 2,
            hair: 7,
            glasses: 2,
            ..r.character()
        };
        r.set_character(before);
        assert_eq!(r.eyes, None);
        let saved = toml::to_string(&r).unwrap();
        assert!(!saved.contains("eyes"), "{saved}");
        let old: Rider = toml::from_str(&saved).unwrap();
        assert_eq!(
            old.character(),
            before,
            "a rider saved before loads the same"
        );
        r.set_character(Character { eyes: 5, ..before });
        assert_eq!(r.eyes.as_deref(), Some("blue"));
        let saved = toml::to_string(&r).unwrap();
        let back: Rider = toml::from_str(&saved).unwrap();
        assert_eq!(back.character(), Character { eyes: 5, ..before });
        // A hand-edited file: any spelling of the list, anything else brown.
        let mut odd = back;
        odd.eyes = Some("Dark Brown".into());
        assert_eq!(odd.character().eyes, 1);
        odd.eyes = Some("purple".into());
        assert_eq!(odd.character().eyes, 0);
    }
}
