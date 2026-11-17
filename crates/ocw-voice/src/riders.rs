//! What the rider chose for single other riders (spec §7.1.7 Q9, built
//! §7.1.10): a **volume** (0 – 200 %, 10 % steps, on top of the spatial
//! gain and the voice volume) and **Block** (both directions, enforced by
//! the game server and the voice server). Remembered across sessions by a
//! stable key: the rider's signed-in account (`acct:ID`, the same on every
//! server) where there is one, else the rider's name on that server
//! (`name:SERVER/NAME`; names are unique on a server, spec §14 Phase 4
//! 4N). Pure; the apps keep the table in the settings (`[voice_riders]`).

use std::collections::BTreeMap;

/// The volume of a rider nobody changed.
pub const DEFAULT_VOLUME_PCT: u16 = 100;
/// The loudest per-rider volume.
pub const MAX_VOLUME_PCT: u16 = 200;
/// The menu's step and the grid every volume is rounded to.
pub const VOLUME_STEP_PCT: u16 = 10;
/// The server scope of rides on the app's own server (solo, hosting).
pub const LOCAL_SCOPE: &str = "local";

/// The rider's choices for one other rider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiderPref {
    /// The rider's name when last seen (the blocked list shows it).
    pub name: String,
    pub volume_pct: u16,
    pub blocked: bool,
}

impl RiderPref {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            volume_pct: DEFAULT_VOLUME_PCT,
            blocked: false,
        }
    }

    /// Nothing changed: not worth remembering.
    pub fn is_default(&self) -> bool {
        self.volume_pct == DEFAULT_VOLUME_PCT && !self.blocked
    }
}

/// Every remembered rider by key.
pub type RiderPrefs = BTreeMap<String, RiderPref>;

/// The key of a signed-in rider.
pub fn account_key(account_id: &str) -> String {
    format!("acct:{account_id}")
}

/// The key of a rider by name on a server.
pub fn name_key(scope: &str, name: &str) -> String {
    format!("name:{scope}/{name}")
}

/// The server part of a name key: the host the app joined (lower case,
/// without scheme, port, path or brackets), so the desktop app
/// (`ride.example.org:47800`) and the browser
/// (`https://ride.example.org:47801/ocw/1`) share it; [`LOCAL_SCOPE`] for
/// nothing.
pub fn server_scope(spec: &str) -> String {
    let s = spec.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(rest) = s.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if s.matches(':').count() > 1 {
        // A bare IPv6 address.
        s
    } else {
        s.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        LOCAL_SCOPE.into()
    } else {
        host.to_ascii_lowercase()
    }
}

/// The keys of one rider: by account if signed in, by name always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiderKeys {
    pub account: Option<String>,
    pub name: String,
}

impl RiderKeys {
    pub fn new(scope: &str, name: &str, account_id: Option<&str>) -> Self {
        Self {
            account: account_id.filter(|a| !a.is_empty()).map(account_key),
            name: name_key(scope, name),
        }
    }

    /// The key a new choice is remembered under: the account if any.
    pub fn primary(&self) -> &str {
        self.account.as_deref().unwrap_or(&self.name)
    }

    /// The remembered choice: the account's first, else the name's.
    pub fn lookup<'a>(&self, prefs: &'a RiderPrefs) -> Option<(&'a str, &'a RiderPref)> {
        self.account
            .iter()
            .chain(std::iter::once(&self.name))
            .find_map(|k| prefs.get_key_value(k).map(|(k, v)| (k.as_str(), v)))
    }

    /// Blocked under either key.
    pub fn blocked(&self, prefs: &RiderPrefs) -> bool {
        self.account
            .iter()
            .chain(std::iter::once(&self.name))
            .any(|k| prefs.get(k).is_some_and(|p| p.blocked))
    }

    pub fn volume_pct(&self, prefs: &RiderPrefs) -> u16 {
        self.lookup(prefs)
            .map_or(DEFAULT_VOLUME_PCT, |(_, p)| p.volume_pct)
    }
}

/// A volume as typed or stepped: clamped to 0 … 200 % and rounded to the
/// 10 % grid.
pub fn clamp_volume(pct: i64) -> u16 {
    let step = i64::from(VOLUME_STEP_PCT);
    let v = pct.clamp(0, i64::from(MAX_VOLUME_PCT));
    (((v + step / 2) / step) * step) as u16
}

/// The gain a voice is played at: the spatial gain (distance, 0 … 1) ×
/// the voice volume (0 … 2) × the rider's own volume (0 … 200 %).
pub fn gain(spatial: f32, voice_volume: f32, rider_pct: u16) -> f32 {
    spatial.max(0.0) * voice_volume.clamp(0.0, 2.0) * f32::from(rider_pct.min(MAX_VOLUME_PCT))
        / 100.0
}

/// The blocks that apply on the server of `scope`, for the game server
/// (`VoiceBlocks`): every blocked account and the names blocked on this
/// server.
pub fn blocks_for(prefs: &RiderPrefs, scope: &str) -> (Vec<String>, Vec<String>) {
    let prefix = format!("name:{scope}/");
    let mut accounts = Vec::new();
    let mut names = Vec::new();
    for (k, _) in prefs.iter().filter(|(_, p)| p.blocked) {
        if let Some(a) = k.strip_prefix("acct:") {
            accounts.push(a.to_owned());
        } else if let Some(n) = k.strip_prefix(&prefix) {
            names.push(n.to_owned());
        }
    }
    (accounts, names)
}

/// The blocked riders (key, name), by name.
pub fn blocked_list(prefs: &RiderPrefs) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = prefs
        .iter()
        .filter(|(_, p)| p.blocked)
        .map(|(k, p)| (k.clone(), p.name.clone()))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_of_server_addresses() {
        assert_eq!(server_scope("ride.example.org:47800"), "ride.example.org");
        assert_eq!(
            server_scope("https://Ride.Example.org:47801/ocw/1?x=1"),
            "ride.example.org"
        );
        assert_eq!(server_scope("192.168.1.20:47800"), "192.168.1.20");
        assert_eq!(server_scope("[fd00::20]:47800"), "fd00::20");
        assert_eq!(server_scope("fd00::20"), "fd00::20");
        assert_eq!(server_scope(""), LOCAL_SCOPE);
        assert_eq!(server_scope("  "), LOCAL_SCOPE);
    }

    #[test]
    fn keys_prefer_the_account() {
        let k = RiderKeys::new("ride.example.org", "Bea Climber", Some("acc-7"));
        assert_eq!(k.primary(), "acct:acc-7");
        assert_eq!(k.name, "name:ride.example.org/Bea Climber");
        let anon = RiderKeys::new("local", "Bea Climber", Some(""));
        assert_eq!(anon.primary(), "name:local/Bea Climber");
        let mut prefs = RiderPrefs::new();
        prefs.insert(
            k.name.clone(),
            RiderPref {
                volume_pct: 50,
                ..RiderPref::new("Bea Climber")
            },
        );
        // Only the name known: it counts.
        assert_eq!(k.volume_pct(&prefs), 50);
        prefs.insert(
            k.primary().to_owned(),
            RiderPref {
                volume_pct: 150,
                ..RiderPref::new("Bea Climber")
            },
        );
        assert_eq!(k.volume_pct(&prefs), 150, "the account's wins");
        assert!(!k.blocked(&prefs));
        prefs.get_mut(&k.name).unwrap().blocked = true;
        assert!(k.blocked(&prefs), "a block under either key holds");
        // Another server: the name key does not match there.
        let elsewhere = RiderKeys::new("other.example.org", "Bea Climber", None);
        assert!(!elsewhere.blocked(&prefs));
        assert_eq!(elsewhere.volume_pct(&prefs), DEFAULT_VOLUME_PCT);
    }

    #[test]
    fn volume_is_clamped_to_the_grid() {
        assert_eq!(clamp_volume(-20), 0);
        assert_eq!(clamp_volume(0), 0);
        assert_eq!(clamp_volume(104), 100);
        assert_eq!(clamp_volume(105), 110);
        assert_eq!(clamp_volume(170), 170);
        assert_eq!(clamp_volume(999), 200);
    }

    /// The gain maths: spatial × voice volume × the rider's volume.
    #[test]
    fn gain_on_top_of_the_spatial_gain() {
        assert_eq!(gain(1.0, 1.0, 100), 1.0);
        assert_eq!(gain(0.5, 1.0, 200), 1.0);
        assert_eq!(gain(0.5, 2.0, 200), 2.0);
        assert_eq!(gain(0.25, 1.0, 0), 0.0);
        assert!((gain(0.2, 0.5, 150) - 0.15).abs() < 1e-6);
        // Out of range inputs are held in range.
        assert_eq!(gain(1.0, 9.0, 900), 4.0);
        assert_eq!(gain(-1.0, 1.0, 100), 0.0);
    }

    #[test]
    fn blocks_for_this_server_only() {
        let mut prefs = RiderPrefs::new();
        let blocked = |name: &str| RiderPref {
            blocked: true,
            ..RiderPref::new(name)
        };
        prefs.insert("acct:acc-1".into(), blocked("Ada Rider"));
        prefs.insert(
            "name:ride.example.org/Bea Climber".into(),
            blocked("Bea Climber"),
        );
        prefs.insert(
            "name:other.example.org/Cleo Sprinter".into(),
            blocked("Cleo Sprinter"),
        );
        prefs.insert(
            "name:ride.example.org/Dan Descender".into(),
            RiderPref {
                volume_pct: 50,
                ..RiderPref::new("Dan Descender")
            },
        );
        let (a, n) = blocks_for(&prefs, "ride.example.org");
        assert_eq!(a, ["acc-1"]);
        assert_eq!(n, ["Bea Climber"]);
        let list = blocked_list(&prefs);
        let names: Vec<&str> = list.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["Ada Rider", "Bea Climber", "Cleo Sprinter"]);
        assert!(RiderPref::new("x").is_default());
        assert!(!blocked("x").is_default());
    }
}
