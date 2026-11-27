//! The traffic switches (spec 14 Phase 18, 18A, 18F): the words of the
//! `[traffic]` settings, the console command and the protocol, shared by
//! the server, the clients and the traffic core. Pure, wasm.
//!
//! Two kinds of traffic (18A *Multiplayer*): **road traffic** touches the
//! ride (it overtakes riders, can hit them with collisions on), so it is a
//! session rule the game server decides ([`RoadTraffic`], [`DriverManners`]:
//! the host in a shared ride, the organiser in an event, Q2); **aircraft
//! and UFOs** never touch a rider, so each rider decides them for their
//! own screen ([`AirTraffic`], [`Ufos`]) and every client simulates the
//! same flights from the session's air seed.

/// How much road traffic the session has (`traffic.road`, 18A; Q4 adds
/// the lunatic level). *Normal* follows the time of day and the day of
/// the week (18B *Demand*); *light* is a quarter of it, *busy* 1.5 x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum RoadTraffic {
    /// Today's empty roads (Q1: the default on a first start).
    #[default]
    Off,
    Light,
    Normal,
    Busy,
    /// Q4: drivers speed, cut corners and overtake in stupid and dangerous
    /// places (blind bends, crests, with oncoming traffic, too close to
    /// riders) and can cause crashes – with collisions on (damage only with
    /// `ride.damage on`).
    Lunatic,
}

impl RoadTraffic {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Light,
        Self::Normal,
        Self::Busy,
        Self::Lunatic,
    ];

    /// The word in settings and the console.
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Normal => "normal",
            Self::Busy => "busy",
            Self::Lunatic => "lunatic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|v| v.key().eq_ignore_ascii_case(s))
    }

    /// Whether there are cars at all.
    pub fn is_on(self) -> bool {
        self != Self::Off
    }

    /// The demand factor on the *normal* volume (18A): 0, 0.25, 1, 1.5;
    /// the lunatic drivers come on a normal day's roads.
    pub fn demand(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Light => 0.25,
            Self::Normal | Self::Lunatic => 1.0,
            Self::Busy => 1.5,
        }
    }
}

/// How drivers behave around riders (`traffic.manners`, Q8: all three
/// selectable; the host's choice in group rides, like the road traffic).
/// The lunatic mode is its own road-traffic level ([`RoadTraffic::Lunatic`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum DriverManners {
    /// Overtaking only with the legal distance (1.5 m in town, 2 m outside
    /// in Germany; 1.5 m in Spain, Switzerland and Liechtenstein) and a free
    /// oncoming lane, otherwise waiting patiently; no horns.
    #[default]
    Polite,
    /// Polite, with an occasional honk at a rider riding in the middle of
    /// the road for a long time.
    MostlyPolite,
    /// Now and then a close pass that never touches.
    SomeRealism,
}

impl DriverManners {
    pub const ALL: [Self; 3] = [Self::Polite, Self::MostlyPolite, Self::SomeRealism];

    pub fn key(self) -> &'static str {
        match self {
            Self::Polite => "polite",
            Self::MostlyPolite => "mostly_polite",
            Self::SomeRealism => "some_realism",
        }
    }

    /// For people: "mostly polite".
    pub fn label(self) -> &'static str {
        match self {
            Self::Polite => "polite",
            Self::MostlyPolite => "mostly polite",
            Self::SomeRealism => "some realism",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().replace([' ', '-'], "_");
        Self::ALL
            .into_iter()
            .find(|v| v.key().eq_ignore_ascii_case(&s))
    }
}

/// Aircraft in the rider's sky (`traffic.air`, 18C; a view setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AirTraffic {
    Off,
    /// From the session's air seed and the world clock (Q1: on by default).
    #[default]
    Simulated,
    /// Real positions through the game server (Q6, T10); until that is
    /// built, and whenever the feed is unreachable, the simulated sky.
    Live,
}

impl AirTraffic {
    pub const ALL: [Self; 3] = [Self::Off, Self::Simulated, Self::Live];

    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Simulated => "simulated",
            Self::Live => "live",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|v| v.key().eq_ignore_ascii_case(s))
    }
}

/// UFOs (`traffic.ufo`, 18C, Q7; a view setting, never in races or events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Ufos {
    #[default]
    Off,
    Rare,
    Often,
}

impl Ufos {
    pub const ALL: [Self; 3] = [Self::Off, Self::Rare, Self::Often];

    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rare => "rare",
            Self::Often => "often",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|v| v.key().eq_ignore_ascii_case(s))
    }
}

/// The session's road traffic (what the server decides and sends).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoadRules {
    pub level: RoadTraffic,
    pub manners: DriverManners,
}

impl RoadRules {
    /// "busy, mostly polite" / "off".
    pub fn summary(&self) -> String {
        if self.level.is_on() && self.level != RoadTraffic::Lunatic {
            format!("{}, {}", self.level.key(), self.manners.label())
        } else {
            self.level.key().to_owned()
        }
    }

    /// Whether drivers honk at riders (Q8: mostly polite and some realism;
    /// lunatics always; never the polite ones; spec 18E T7).
    pub fn honks(&self) -> bool {
        self.level.is_on()
            && (self.level == RoadTraffic::Lunatic || self.manners != DriverManners::Polite)
    }
}

/// What the rider's screen shows of the traffic (18A): the master switch
/// turns everything off; aircraft and UFOs are the rider's own choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewChoice {
    /// `traffic.enabled`.
    pub enabled: bool,
    pub air: AirTraffic,
    pub ufo: Ufos,
    /// `traffic.sound`.
    pub sound: bool,
}

impl Default for ViewChoice {
    /// Q1: aircraft on (simulated), UFOs off; the sounds on.
    fn default() -> Self {
        Self {
            enabled: true,
            air: AirTraffic::Simulated,
            ufo: Ufos::Off,
            sound: true,
        }
    }
}

impl ViewChoice {
    /// The aircraft drawn (off with the master switch off). *Live* is
    /// simulated until the live feed is built (Q6, T10).
    pub fn air_shown(&self) -> AirTraffic {
        if self.enabled {
            self.air
        } else {
            AirTraffic::Off
        }
    }

    /// The UFOs drawn: none with the master switch off, and never in a
    /// race or event (`in_event`, 18C).
    pub fn ufos_shown(&self, in_event: bool) -> Ufos {
        if self.enabled && !in_event {
            self.ufo
        } else {
            Ufos::Off
        }
    }
}

/// The road traffic an event has when its organiser did not set one
/// (Q2): none – races and time trials never get cars unless the organiser
/// asks, and neither do group rides.
pub const EVENT_DEFAULT: RoadTraffic = RoadTraffic::Off;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_round_trip() {
        for v in RoadTraffic::ALL {
            assert_eq!(RoadTraffic::parse(v.key()), Some(v));
        }
        for v in DriverManners::ALL {
            assert_eq!(DriverManners::parse(v.key()), Some(v));
            assert_eq!(DriverManners::parse(v.label()), Some(v));
        }
        for v in AirTraffic::ALL {
            assert_eq!(AirTraffic::parse(v.key()), Some(v));
        }
        for v in Ufos::ALL {
            assert_eq!(Ufos::parse(v.key()), Some(v));
        }
        assert_eq!(RoadTraffic::parse("BUSY"), Some(RoadTraffic::Busy));
        assert_eq!(RoadTraffic::parse("jam"), None);
    }

    #[test]
    fn first_start_defaults_follow_q1() {
        assert_eq!(RoadTraffic::default(), RoadTraffic::Off);
        assert_eq!(DriverManners::default(), DriverManners::Polite);
        let v = ViewChoice::default();
        assert!(v.enabled && v.sound);
        assert_eq!(v.air_shown(), AirTraffic::Simulated);
        assert_eq!(v.ufos_shown(false), Ufos::Off);
        assert_eq!(EVENT_DEFAULT, RoadTraffic::Off);
    }

    #[test]
    fn the_master_switch_and_events_hide_the_sky() {
        let v = ViewChoice {
            enabled: false,
            ufo: Ufos::Often,
            ..Default::default()
        };
        assert_eq!(v.air_shown(), AirTraffic::Off);
        assert_eq!(v.ufos_shown(false), Ufos::Off);
        let on = ViewChoice {
            ufo: Ufos::Often,
            ..Default::default()
        };
        assert_eq!(on.ufos_shown(false), Ufos::Often);
        assert_eq!(on.ufos_shown(true), Ufos::Off, "never in events");
    }

    #[test]
    fn demand_and_summary() {
        assert_eq!(RoadTraffic::Light.demand(), 0.25);
        assert_eq!(RoadTraffic::Busy.demand(), 1.5);
        let r = RoadRules {
            level: RoadTraffic::Busy,
            manners: DriverManners::MostlyPolite,
        };
        assert_eq!(r.summary(), "busy, mostly polite");
        assert_eq!(RoadRules::default().summary(), "off");
    }
}
