//! The commented-out examples of `ocw.toml` and `secrets.toml` (spec
//! 3.2.4, *Commented examples*): every section and key the app knows, one
//! line each – what it is, its default and its values or units.
//!
//! This is the one table of them: `[settings]` comes from the settings
//! registry (`ocw_client::settings::SETTINGS`: description, default,
//! values), the other sections from [`hand`]. A new file gets them all at
//! its end ([`block`]); an existing file gets the missing ones appended
//! once at the start ([`fill`]): never a value changed, never one twice,
//! the rider's comments kept. The tests fail when a key of a section's
//! struct (`Rider`, `KnownDevices`, the Strava and ANT+ secrets, …) has no
//! example, and when a secret's example is not one of the made-up
//! [`PLACEHOLDERS`].

use ocw_client::settings::{Kind, SETTINGS, Scope, Value};
use toml_edit::{DocumentMut, Item};

/// Which file an example belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File {
    /// `ocw.toml`.
    Config,
    /// `secrets.toml`.
    Secrets,
}

/// One commented-out example line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub file: File,
    /// The table header as written: `[settings.audio]`,
    /// `[[devices.device]]`.
    pub header: String,
    pub key: String,
    /// The value as TOML (`30`, `"chase"`, `true`).
    pub value: String,
    /// One line: what it is, the default, the values / units.
    pub note: String,
}

/// The made-up values the secrets' examples use – never a real key,
/// token or client id (CLAUDE.md, spec 3.2.4).
pub const PLACEHOLDERS: &[&str] = &[
    "\"12345\"",
    "\"made-up-client-secret\"",
    "\"made-up-access-token\"",
    "\"made-up-refresh-token\"",
    // The ANT+ network key: obviously not Garmin's (the real one comes
    // from Garmin's ANT+ developer page).
    "\"0A1B2C3D4E5F6071\"",
    r#"'{"issuer":"https://accounts.example.org","access_token":"made-up-access-token"}'"#,
];

/// The line that starts the examples block (also how [`fill`] knows the
/// block is there).
pub const BLOCK_MARK: &str =
    "# ---- Examples: every section and key the app knows, commented out ----";

/// A line exactly like this anywhere in the file: no examples are added.
pub const NO_EXAMPLES: &str = "# ocw: no examples";

fn intro(file: File) -> String {
    let what = match file {
        File::Config => "settings",
        File::Secrets => "secrets (all values below are made-up placeholders)",
    };
    format!(
        "{BLOCK_MARK}\n\
         # The known {what}, one line each: what it is, its default, its values.\n\
         # To use one, remove the \"# \" before its section line and its key line\n\
         # (a section only once in the file: it may be above already). The app\n\
         # adds examples of new keys at its start and never changes your values;\n\
         # a line \"{NO_EXAMPLES}\" stops that.\n"
    )
}

/// A TOML string literal.
fn s(v: &str) -> String {
    toml_edit::Value::from(v).to_string().trim().to_owned()
}

fn ex(file: File, header: &str, key: &str, value: String, note: String) -> Example {
    Example {
        file,
        header: header.into(),
        key: key.into(),
        value,
        note,
    }
}

/// The value of a registry default as TOML.
fn toml_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Text(t) => s(t),
    }
}

/// `[settings.<group>]` from the registry, in its order; the keys kept in
/// another section (`[strava] auto_upload`) there.
fn registry() -> Vec<Example> {
    let mut out = Vec::new();
    for d in SETTINGS {
        let (group, name) = d.key.split_once('.').expect("keys are group.name");
        let header = match super::SETTINGS_ELSEWHERE
            .iter()
            .find(|(g, k, _)| *g == group && *k == name)
        {
            Some((_, _, section)) => format!("[{section}]"),
            None => format!("[settings.{group}]"),
        };
        let values = match d.kind {
            Kind::Text => "free text".to_string(),
            _ => d.kind_text(),
        };
        let when = match d.scope {
            Scope::Live => "",
            Scope::NextStart => "; at the next start",
        };
        let note = format!(
            "{}; default {}; {values}{when}",
            d.description.replace('\n', " "),
            if d.default.is_empty() {
                "empty"
            } else {
                d.default
            },
        );
        out.push(ex(
            File::Config,
            &header,
            name,
            toml_value(&d.default_value()),
            note,
        ));
    }
    out
}

fn ids<T: Copy>(all: &[T], id: impl Fn(T) -> &'static str) -> String {
    all.iter().map(|x| id(*x)).collect::<Vec<_>>().join(", ")
}

/// Every section that is not the registry's.
pub fn hand() -> Vec<Example> {
    use File::{Config as C, Secrets as S};
    use ocw_core::character::{self as ch, Age, BaseBody, BodyBuild, BodyShape};
    use ocw_core::kit::Finish;
    use ocw_core::{KitPattern, ProfileId};
    let colour = |what: &str| format!("{what} as \"#RRGGBB\"; default: the default look's");
    let rider = "[[riders.rider]]";
    let mut v = vec![
        // Voice: single riders' volume and Block (spec 7.1.7 Q9).
        ex(
            C,
            "[settings.voice_riders.\"name:ride.example.org/Bea Climber\"]",
            "name",
            s("Bea Climber"),
            "Voice chat: your choice for one other rider (the table's name is the rider's key, written by the app); the name shown".into(),
        ),
        ex(
            C,
            "[settings.voice_riders.\"name:ride.example.org/Bea Climber\"]",
            "volume",
            "150".into(),
            "Their voice volume in %; default 100; 0 to 200 in steps of 10".into(),
        ),
        ex(
            C,
            "[settings.voice_riders.\"name:ride.example.org/Bea Climber\"]",
            "blocked",
            "false".into(),
            "Blocked: you never hear them; default false; true / false".into(),
        ),
        ex(
            C,
            "[settings.voice_riders.\"name:ride.example.org/Bea Climber\"]",
            "selected",
            "true".into(),
            "Picked in the voice riders' list; default false; true / false".into(),
        ),
        // Local riders (spec 3.2.4 Q4, Settings -> Riders).
        ex(
            C,
            "[riders]",
            "active",
            s("Ada Rider"),
            "The rider *Ride locally* rides: a name of a [[riders.rider]] below; default: the first one".into(),
        ),
        ex(
            C,
            rider,
            "name",
            s("Ada Rider"),
            format!(
                "The rider's name (one [[riders.rider]] per rider); required; at most {} characters",
                ocw_client::riders::MAX_NAME_CHARS
            ),
        ),
        ex(
            C,
            rider,
            "weight_kg",
            "61.0".into(),
            format!(
                "Weight in kg, the physics' rider mass; required (the default rider: 75.0); {} to {}",
                ocw_core::MIN_RIDER_WEIGHT_KG,
                ocw_core::MAX_RIDER_WEIGHT_KG
            ),
        ),
        ex(
            C,
            rider,
            "height_cm",
            "168.0".into(),
            format!(
                "Body height in cm (a look, never physics); default none; {} to {}",
                ocw_client::riders::HEIGHT_RANGE_CM.0,
                ocw_client::riders::HEIGHT_RANGE_CM.1
            ),
        ),
        ex(
            C,
            rider,
            "ftp_w",
            "230".into(),
            format!(
                "FTP in W (workouts' targets); default none; {} to {}",
                ocw_client::riders::FTP_RANGE_W.0,
                ocw_client::riders::FTP_RANGE_W.1
            ),
        ),
        ex(
            C,
            rider,
            "bike",
            s(ProfileId::ALL[0].id()),
            format!(
                "The bike's physics profile (also its model); default {}; {}",
                ProfileId::default().id(),
                ids(&ProfileId::ALL, ProfileId::id)
            ),
        ),
        ex(C, rider, "frame_color", s("#1E88E5"), colour("Frame colour")),
        ex(C, rider, "jersey_color", s("#FF1472"), colour("Jersey colour")),
        ex(C, rider, "helmet_color", s("#F2F2EE"), colour("Helmet colour")),
        ex(
            C,
            rider,
            "kit",
            s(KitPattern::Band.id()),
            format!(
                "The jersey's pattern; default plain; {}",
                ids(&KitPattern::ALL, KitPattern::id)
            ),
        ),
        ex(
            C,
            rider,
            "body_build",
            s(BodyBuild::Medium.id()),
            format!(
                "Body build (version 1, kept for older apps); default medium; {}",
                ids(&BodyBuild::ALL, BodyBuild::id)
            ),
        ),
        ex(
            C,
            rider,
            "skin",
            "6".into(),
            format!(
                "Skin (version 1 palette, kept for older apps); default: the default look's; 0 to {}",
                ch::LEGACY_SKIN_TONES.len() - 1
            ),
        ),
        ex(
            C,
            rider,
            "face",
            "0".into(),
            format!(
                "Face (version 1: {}); default 0; 0 to {}",
                ch::LEGACY_FACES.join(", "),
                ch::LEGACY_FACES.len() - 1
            ),
        ),
        ex(
            C,
            rider,
            "hair",
            "3".into(),
            format!(
                "Hair style's number (version 1, kept for older apps); default: the default look's; 0 to {}",
                ch::HAIR_STYLES.len() - 1
            ),
        ),
        ex(C, rider, "hair_color", s("#6B4426"), colour("Hair colour")),
        ex(
            C,
            rider,
            "glasses",
            "0".into(),
            format!(
                "Glasses; default 0; {}",
                ch::GLASSES
                    .iter()
                    .enumerate()
                    .map(|(i, g)| format!("{i} {g}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ex(
            C,
            rider,
            "character_version",
            "2".into(),
            "Which character keys count: 2 = the keys below (body … skin_tone); default: 1 when missing (migrated on reading)".into(),
        ),
        ex(
            C,
            rider,
            "body",
            s(BaseBody::ALL[0].id()),
            format!(
                "Base body; default: the default look's; {}",
                ids(&BaseBody::ALL, BaseBody::id)
            ),
        ),
        ex(
            C,
            rider,
            "shape",
            s(BodyShape::ALL[0].id()),
            format!(
                "Body shape; default: the default look's; {}",
                ids(&BodyShape::ALL, BodyShape::id)
            ),
        ),
        ex(
            C,
            rider,
            "age",
            s(Age::ALL[0].id()),
            format!(
                "Age look; default: the default look's; {}",
                ids(&Age::ALL, Age::id)
            ),
        ),
        ex(
            C,
            rider,
            "hair_style",
            s(&ch::hair_id(3)),
            format!(
                "Hair style; default: the default look's; {}",
                (0..ch::HAIR_STYLES.len() as u8)
                    .map(ch::hair_id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ex(
            C,
            rider,
            "greying",
            "false".into(),
            "Greying hair; default false; true / false".into(),
        ),
        ex(
            C,
            rider,
            "moustache",
            s(&ch::list_id(&ch::MOUSTACHES, 0)),
            format!(
                "Moustache; default: none; {}",
                (0..ch::MOUSTACHES.len() as u8)
                    .map(|i| ch::list_id(&ch::MOUSTACHES, i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ex(
            C,
            rider,
            "beard",
            s(&ch::list_id(&ch::BEARDS, 0)),
            format!(
                "Beard; default: none; {}",
                (0..ch::BEARDS.len() as u8)
                    .map(|i| ch::list_id(&ch::BEARDS, i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ex(
            C,
            rider,
            "facial_hair_color",
            s("#6B4426"),
            "Facial hair colour as \"#RRGGBB\"; default: the hair colour".into(),
        ),
        ex(
            C,
            rider,
            "skin_tone",
            "8".into(),
            format!(
                "Skin tone (16-step palette); default: the default look's; 0 to {}",
                ch::SKIN_TONES.len() - 1
            ),
        ),
        ex(C, rider, "kit_second", s("#FDD835"), colour("The kit's second colour")),
        ex(C, rider, "kit_third", s("#1E1E22"), colour("The kit's third colour")),
        ex(
            C,
            rider,
            "race_number",
            "42".into(),
            format!(
                "Race number on the jersey; default none; 1 to {}",
                ocw_core::kit::MAX_RACE_NUMBER
            ),
        ),
        ex(
            C,
            rider,
            "helmet_pattern",
            "true".into(),
            "The kit's pattern on the helmet too; default false; true / false".into(),
        ),
        ex(C, rider, "shorts_color", s("#3949AB"), colour("Shorts colour")),
        ex(C, rider, "socks_color", s("#F2F2EE"), colour("Socks colour")),
        ex(C, rider, "shoes_color", s("#1E1E22"), colour("Shoes colour")),
        ex(C, rider, "gloves_color", s("#1E1E22"), colour("Gloves colour")),
        ex(
            C,
            rider,
            "helmet_finish",
            s(Finish::Matte.id()),
            format!(
                "Helmet finish; default gloss; {}",
                ids(&Finish::ALL, Finish::id)
            ),
        ),
        ex(
            C,
            rider,
            "shoe_finish",
            s(Finish::Gloss.id()),
            format!(
                "Shoe finish; default gloss; {}",
                ids(&Finish::ALL, Finish::id)
            ),
        ),
        // Remembered devices (spec 3.4, Settings -> Trainer -> Devices).
        ex(
            C,
            "[devices]",
            "heart_rate",
            s("ant:120:23456:1"),
            "The device of *Use for heart rate* (with two straps only its heart rate counts); default: the first that sends; a device id below".into(),
        ),
        ex(
            C,
            "[devices]",
            "cadence",
            s("ant:121:34567:1"),
            "The device of *Use for cadence*; default: the first that sends; a device id below".into(),
        ),
        ex(
            C,
            "[devices]",
            "trainer",
            s("hci0/AA:BB:CC:DD:EE:01"),
            "The trainer of *Use as trainer* (only its power counts, only it gets SIM / ERG); default: the first; a device id below".into(),
        ),
        ex(
            C,
            "[[devices.device]]",
            "id",
            s("hci0/AA:BB:CC:DD:EE:01"),
            "A remembered device's id (one [[devices.device]] each; written by the app's Connect): Bluetooth's id, or ant:<type>:<number>:<transmission type>".into(),
        ),
        ex(
            C,
            "[[devices.device]]",
            "name",
            s("Made-up trainer"),
            "Its name as the Devices page shows it".into(),
        ),
        ex(
            C,
            "[[devices.device]]",
            "kind",
            s("trainer"),
            "What it is; default: unknown until it connects; trainer, power, cadence, hr".into(),
        ),
        ex(
            C,
            "[[devices.device]]",
            "transport",
            s("ant"),
            "How it is reached; default: Bluetooth (no key); ant = an ANT+ sensor through the ANT USB stick".into(),
        ),
        // Recent servers and the start choice.
        ex(
            C,
            "[[servers.recent]]",
            "link",
            s("ocw://rides.example.org:47800"),
            "A recent server of *Connect to a server* (one [[servers.recent]] each, newest first; written by the app): its join link".into(),
        ),
        ex(
            C,
            "[[servers.recent]]",
            "world",
            s("route hills"),
            "What that server rode (shown next to the link)".into(),
        ),
        ex(
            C,
            "[start]",
            "choice",
            s(ocw_client::menu::start::Choice::Local.id()),
            "The last start choice, preselected at the next start; default local; public, server, local".into(),
        ),
        // secrets.toml.
        ex(
            S,
            "[strava]",
            "client_id",
            "\"12345\"".into(),
            "Strava upload: the Client ID of your own Strava API app (strava.com/settings/api, callback domain 127.0.0.1; then strava.connect); default none: Strava off; OCW_STRAVA_CLIENT_ID wins".into(),
        ),
        ex(
            S,
            "[strava]",
            "client_secret",
            "\"made-up-client-secret\"".into(),
            "Its Client Secret; default none: Strava off; OCW_STRAVA_CLIENT_SECRET wins".into(),
        ),
        ex(
            S,
            "[strava_token]",
            "access_token",
            "\"made-up-access-token\"".into(),
            "Strava's token, written by strava.connect and refreshed by the app (do not type it)".into(),
        ),
        ex(
            S,
            "[strava_token]",
            "refresh_token",
            "\"made-up-refresh-token\"".into(),
            "Strava's refresh token, written by the app".into(),
        ),
        ex(
            S,
            "[strava_token]",
            "expires_at",
            "1800000000".into(),
            "When the access token expires, in Unix seconds; written by the app".into(),
        ),
        ex(
            S,
            "[strava_token]",
            "athlete",
            s("Ada Rider"),
            "The linked athlete's name as Strava gave it; written by the app".into(),
        ),
        ex(
            S,
            "[account]",
            "session",
            PLACEHOLDERS[5].into(),
            "The sign-in session (JSON) when no system keyring keeps it; written by Settings -> Account (do not type it)".into(),
        ),
        ex(
            S,
            "[ant]",
            "network_key",
            "\"0A1B2C3D4E5F6071\"".into(),
            "The ANT+ network key: 16 hex digits from Garmin's ANT+ developer page (thisisant.com, after accepting its terms), never shipped with OCW; this value is a made-up placeholder; default none: ANT+ off; or enter it in Devices -> ANT+ (USB stick)".into(),
        ),
    ];
    // The rider's look keys as the app writes them (the version-1 keys
    // must match the version-2 ones, `Riders::parse` checks that).
    let sample = sample_rider();
    for e in v.iter_mut().filter(|e| e.header == rider) {
        if let Some(value) = sample.get(&e.key) {
            e.value = value.to_string();
        }
    }
    v
}

/// A rider written the app's way: the default character and look.
fn sample_rider() -> toml::Table {
    let mut r = ocw_client::riders::Rider {
        name: "Ada Rider".into(),
        weight_kg: 61.0,
        height_cm: Some(168.0),
        ftp_w: Some(230),
        ..Default::default()
    };
    r.set_bike(ocw_core::ProfileId::ALL[0]);
    r.set_character(ocw_core::Character::default());
    r.set_look(ocw_core::RiderLook::default());
    toml::Table::try_from(&r).expect("a rider serializes")
}

/// Every example of `file`, in file order.
pub fn all(file: File) -> Vec<Example> {
    let mut v: Vec<Example> = match file {
        File::Config => {
            let mut v = registry();
            v.extend(hand());
            v
        }
        File::Secrets => hand(),
    };
    v.retain(|e| e.file == file);
    v
}

/// The lines of these examples, grouped by header in first-seen order.
pub fn render(examples: &[Example]) -> String {
    let mut headers: Vec<&str> = Vec::new();
    for e in examples {
        if !headers.contains(&e.header.as_str()) {
            headers.push(&e.header);
        }
    }
    let mut out = String::new();
    for h in headers {
        out.push_str(&format!("#\n# {h}\n"));
        for e in examples.iter().filter(|e| e.header == h) {
            out.push_str(&format!("# {} = {}   # {}\n", e.key, e.value, e.note));
        }
    }
    out
}

/// The whole examples block of a new file.
pub fn block(file: File) -> String {
    format!("\n{}{}", intro(file), render(&all(file)))
}

/// `[a.b."c d"]` / `[[a.b]]` → `["a", "b", "c d"]`.
fn header_path(header: &str) -> Vec<String> {
    let inner = header.trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in inner.chars() {
        match c {
            '"' => quoted = !quoted,
            '.' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Whether `doc` has `key` under `path` (any table of an array of tables).
fn has_value(item: &Item, path: &[String], key: &str) -> bool {
    match item {
        Item::ArrayOfTables(a) => a
            .iter()
            .any(|t| has_value(&Item::Table(t.clone()), path, key)),
        _ => {
            let Some(t) = item.as_table_like() else {
                return false;
            };
            match path.split_first() {
                None => t.contains_key(key),
                Some((first, rest)) => t.get(first).is_some_and(|i| has_value(i, rest, key)),
            }
        }
    }
}

/// The (header, key) pairs written as commented-out examples in `text`.
fn commented(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut header = String::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.starts_with('[') {
            // `[x.y]` up to its closing bracket(s).
            let end = if rest.starts_with("[[") {
                rest.find("]]").map(|i| i + 2)
            } else {
                // The header may hold a quoted key with brackets: the last `]`
                // before a comment.
                rest.split(" #")
                    .next()
                    .and_then(|h| h.rfind(']').map(|i| i + 1))
            };
            if let Some(end) = end {
                header = rest[..end].to_string();
            }
        } else if let Some((k, _)) = rest.split_once(" = ")
            && !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            out.push((header.clone(), k.to_string()));
        }
    }
    out
}

/// The examples of `file` that `text` (a whole file) has neither as a
/// value nor as a commented-out line. Err: the file does not parse.
pub fn missing(file: File, text: &str) -> Result<Vec<Example>, String> {
    let doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let root = Item::Table(doc.as_table().clone());
    let have = commented(text);
    Ok(all(file)
        .into_iter()
        .filter(|e| {
            !has_value(&root, &header_path(&e.header), &e.key)
                && !have.iter().any(|(h, k)| *h == e.header && *k == e.key)
        })
        .collect())
}

/// `text` with the missing examples appended (`None`: nothing to add, or
/// the file says [`NO_EXAMPLES`]). The text before stays byte for byte.
pub fn fill(file: File, text: &str) -> Result<Option<String>, String> {
    if text.lines().any(|l| l.trim() == NO_EXAMPLES) {
        return Ok(None);
    }
    let add = missing(file, text)?;
    if add.is_empty() {
        return Ok(None);
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if text.lines().any(|l| l == BLOCK_MARK) {
        out.push_str("#\n# (added by a newer version of OCW)\n");
    } else {
        out.push('\n');
        out.push_str(&intro(file));
    }
    out.push_str(&render(&add));
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field names a serde struct reads (its `#[derive(Deserialize)]`
    /// list, renames included) – so a new field without an example fails
    /// the test below.
    fn fields<T: serde::de::DeserializeOwned>() -> &'static [&'static str] {
        use serde::de::{Error, Visitor};
        struct Probe<'a>(&'a mut &'static [&'static str]);
        impl<'de> serde::Deserializer<'de> for Probe<'_> {
            type Error = serde::de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
                Err(Error::custom("probe"))
            }
            fn deserialize_struct<V: Visitor<'de>>(
                self,
                _: &'static str,
                fields: &'static [&'static str],
                _: V,
            ) -> Result<V::Value, Self::Error> {
                *self.0 = fields;
                Err(Error::custom("probe"))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        let mut f: &'static [&'static str] = &[];
        let _ = T::deserialize(Probe(&mut f));
        assert!(!f.is_empty(), "{} is a struct", std::any::type_name::<T>());
        f
    }

    fn has(file: File, header: &str, key: &str) -> bool {
        all(file).iter().any(|e| e.header == header && e.key == key)
    }

    /// The rule (CLAUDE.md, spec 3.2.4): every key the app reads from
    /// `ocw.toml` or `secrets.toml` has an example. A new field of one of
    /// these structs, a new setting or a new section fails here until its
    /// example is in the table.
    #[test]
    fn every_setting_and_secret_has_an_example() {
        use File::{Config as C, Secrets as S};
        let mut lacking = Vec::new();
        let mut need = |file: File, header: &str, keys: &[&str]| {
            for k in keys {
                if !has(file, header, k) {
                    lacking.push(format!("{file:?} {header} {k}"));
                }
            }
        };
        // The registry: every setting, in [settings.<group>] or its own
        // section.
        for d in SETTINGS {
            let (g, k) = d.key.split_once('.').unwrap();
            let header = super::super::SETTINGS_ELSEWHERE
                .iter()
                .find(|(gg, kk, _)| *gg == g && *kk == k)
                .map_or(format!("[settings.{g}]"), |(_, _, s)| format!("[{s}]"));
            need(C, &header, &[k]);
        }
        let voice = "[settings.voice_riders.\"name:ride.example.org/Bea Climber\"]";
        need(C, voice, &["name", "volume", "blocked", "selected"]);
        // [riders]: the struct's fields (`rider` is the array).
        let riders = fields::<ocw_client::riders::Riders>();
        assert_eq!(riders, ["active", "rider"]);
        need(C, "[riders]", &["active"]);
        need(C, "[[riders.rider]]", fields::<ocw_client::riders::Rider>());
        // [devices].
        let devices: Vec<&str> = fields::<crate::known_devices::KnownDevices>()
            .iter()
            .copied()
            .filter(|f| *f != "device")
            .collect();
        need(C, "[devices]", &devices);
        need(
            C,
            "[[devices.device]]",
            fields::<crate::known_devices::KnownDevice>(),
        );
        need(C, "[[servers.recent]]", &["link", "world"]);
        need(C, "[start]", &["choice"]);
        // Strava: the credentials are secrets, `auto_upload` is the setting
        // `strava.auto_upload` (in ocw.toml, checked above).
        for f in fields::<crate::strava::auth::StravaConfig>() {
            if *f != "auto_upload" {
                need(S, "[strava]", &[*f]);
            }
        }
        need(S, "[strava_token]", fields::<crate::strava::auth::Token>());
        need(S, "[account]", &["session"]);
        need(S, "[ant]", fields::<crate::devices::AntSecrets>());
        assert!(lacking.is_empty(), "keys without an example: {lacking:#?}");
        // Every section of both files has examples.
        for s in super::super::SECTIONS {
            assert!(
                all(C).iter().any(|e| header_path(&e.header)[0] == *s),
                "[{s}]"
            );
        }
        for s in super::super::SECRET_SECTIONS {
            assert!(
                all(S).iter().any(|e| header_path(&e.header)[0] == *s),
                "[{s}]"
            );
        }
        // Each example once.
        for f in [C, S] {
            let a = all(f);
            for (i, e) in a.iter().enumerate() {
                assert!(
                    !a[..i]
                        .iter()
                        .any(|o| o.header == e.header && o.key == e.key),
                    "twice: {} {}",
                    e.header,
                    e.key
                );
                assert!(!e.note.contains('\n'), "{}: one line", e.key);
            }
        }
    }

    /// Secrets' examples are made-up placeholders only (never a real key,
    /// token or client id), and ocw.toml's examples hold no secret key.
    #[test]
    fn secret_examples_are_placeholders_only() {
        for e in all(File::Secrets) {
            if super::super::SECRET_KEYS.contains(&e.key.as_str()) {
                assert!(
                    PLACEHOLDERS.contains(&e.value.as_str()),
                    "{} {} = {} is not a known placeholder",
                    e.header,
                    e.key,
                    e.value
                );
            }
        }
        for e in all(File::Config) {
            assert!(
                !super::super::SECRET_KEYS.contains(&e.key.as_str()),
                "a secret key in ocw.toml's examples: {} {}",
                e.header,
                e.key
            );
        }
        // The ANT+ key example is the test key and says where the real
        // one comes from.
        let ant = all(File::Secrets)
            .into_iter()
            .find(|e| e.key == "network_key")
            .unwrap();
        assert_eq!(ant.value, "\"0A1B2C3D4E5F6071\"");
        assert!(ant.note.contains("Garmin") && ant.note.contains("placeholder"));
    }

    /// Every example line, uncommented, is valid: the file parses and each
    /// section reads without a warning.
    #[test]
    fn the_examples_uncommented_parse_and_read_without_warnings() {
        let uncomment = |text: &str| -> String {
            text.lines()
                .map(|l| match l.strip_prefix("# ") {
                    Some(r)
                        if r.starts_with('[')
                            || r.split_once(" = ").is_some_and(|(k, _)| {
                                !k.is_empty()
                                    && k.chars()
                                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                            }) =>
                    {
                        r.to_string()
                    }
                    _ => l.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let config = uncomment(&block(File::Config));
        let t: toml::Table = toml::from_str(&config).unwrap_or_else(|e| panic!("{e}\n{config}"));
        let section = |k: &str| toml::to_string(&t[k]).unwrap();
        let (_, w) = ocw_client::settings::Settings::parse(&section("settings")).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let (r, w) = ocw_client::riders::Riders::parse(&section("riders")).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(r.active().name, "Ada Rider");
        let d: crate::known_devices::KnownDevices = toml::from_str(&section("devices")).unwrap();
        assert_eq!(d.devices.len(), 1);
        assert!(t["servers"]["recent"].as_array().is_some());
        assert_eq!(t["start"]["choice"].as_str(), Some("local"));
        assert_eq!(t["strava"]["auto_upload"].as_bool(), Some(true));
        let secrets = uncomment(&block(File::Secrets));
        let t: toml::Table = toml::from_str(&secrets).unwrap_or_else(|e| panic!("{e}\n{secrets}"));
        let st: crate::strava::auth::StravaConfig =
            toml::from_str(&toml::to_string(&t["strava"]).unwrap()).unwrap();
        assert_eq!(st.client_id.as_deref(), Some("12345"));
        let _: crate::strava::auth::Token =
            toml::from_str(&toml::to_string(&t["strava_token"]).unwrap()).unwrap();
        let session = t["account"]["session"].as_str().unwrap();
        serde_json::from_str::<serde_json::Value>(session).unwrap();
        let ant: crate::devices::AntSecrets =
            toml::from_str(&toml::to_string(&t["ant"]).unwrap()).unwrap();
        assert!(ocw_devices::ant::NetworkKey::parse(&ant.network_key.unwrap()).is_ok());
        // Commented out, the blocks hold no value at all.
        for f in [File::Config, File::Secrets] {
            let t: toml::Table = toml::from_str(&block(f)).unwrap();
            assert!(t.is_empty(), "{t:?}");
        }
    }

    #[test]
    fn fill_adds_only_what_is_missing_once_and_keeps_the_text() {
        let old = "# my notes\n[settings.audio]\n# quieter at night\nmaster = 30\n\n[start]\nchoice = \"server\"\n";
        let new = fill(File::Config, old).unwrap().unwrap();
        // The file before stays byte for byte; the block follows.
        assert!(new.starts_with(old), "{new}");
        assert_eq!(new.matches(BLOCK_MARK).count(), 1);
        // Keys with a value get no example; the others do.
        assert!(!new.contains("# master = "), "{new}");
        assert!(!new.contains("# choice = "), "{new}");
        assert!(new.contains("# [settings.audio]\n"), "{new}");
        assert!(new.contains("\n# effects = "), "{new}");
        assert!(
            new.contains("# [[riders.rider]]\n# name = \"Ada Rider\""),
            "{new}"
        );
        // The values are untouched.
        let t: toml::Table = toml::from_str(&new).unwrap();
        assert_eq!(t["settings"]["audio"]["master"].as_integer(), Some(30));
        assert_eq!(t["start"]["choice"].as_str(), Some("server"));
        // Idempotent.
        assert_eq!(fill(File::Config, &new).unwrap(), None);
        // A new key later (one example taken out): only it comes back.
        let without = new.replace(
            &new.lines()
                .find(|l| l.starts_with("# effects = "))
                .unwrap()
                .to_string(),
            "",
        );
        let again = fill(File::Config, &without).unwrap().unwrap();
        assert!(again.starts_with(&without));
        let added = &again[without.len()..];
        assert_eq!(commented(added).len(), 1, "{added}");
        assert!(
            added.contains("# [settings.audio]\n# effects = "),
            "{added}"
        );
        assert_eq!(again.matches(BLOCK_MARK).count(), 1);
        // A real value under an array of tables counts too.
        let devices = "[[devices.device]]\nid = \"x\"\nname = \"Made-up meter\"\n";
        let d = fill(File::Config, devices).unwrap().unwrap();
        assert!(!d.contains("# id = ") && d.contains("# kind = "), "{d}");
        // Opted out; a broken file is not touched.
        assert_eq!(
            fill(File::Config, &format!("{NO_EXAMPLES}\n")).unwrap(),
            None
        );
        assert!(fill(File::Config, "[[x\n").is_err());
        // A file without a newline at its end.
        let s = fill(File::Secrets, "[ant]\nnetwork_key = \"0A1B2C3D4E5F6071\"")
            .unwrap()
            .unwrap();
        assert!(
            s.starts_with("[ant]\nnetwork_key = \"0A1B2C3D4E5F6071\"\n\n"),
            "{s}"
        );
        assert!(!s.contains("# network_key"), "{s}");
        assert!(s.contains("# client_secret = "), "{s}");
    }

    /// The secrets example shipped in the release (`packaging/
    /// secrets.example.toml`) has every secrets example.
    #[test]
    fn the_packaged_secrets_example_has_every_example() {
        let text = include_str!("../../../../packaging/secrets.example.toml");
        let m = missing(File::Secrets, text).unwrap();
        assert!(
            m.is_empty(),
            "add to packaging/secrets.example.toml:\n{}",
            render(&m)
        );
        let t: toml::Table = toml::from_str(text).unwrap();
        assert!(t.is_empty(), "all commented out: {t:?}");
    }
}
