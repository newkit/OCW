//! Joining a shared ride at runtime (spec §3.2.1 *Ride → Join*, §6.2
//! *Join links*), Bevy-free and shared by both apps:
//!
//! - [`JoinFlow`]: the state machine of one join – validate the link,
//!   connect in the background (timeout [`CONNECT_TIMEOUT_S`]), learn the
//!   server's course from its `Welcome`, load it, switch – and why it
//!   failed, in words. The app owns the connection and the loading; it
//!   tells the flow what happened.
//! - [`ServerCourse`]: what the server rides, from its `Welcome`.
//! - [`RecentJoins`]: the last [`MAX_RECENT`] servers joined (one link per
//!   line, the world after a tab), kept by the apps in a text store.
//! - [`probe_hello`]: the rider's hello without the world (the server's
//!   course is not known yet; the server then does not compare graphs and
//!   answers with its `Welcome`). The same connection becomes the ride's
//!   link once the course is loaded (the server ignores a second hello).

use ocw_protocol::join::JoinLink;
use ocw_protocol::v1::client_message::Msg as C;
use ocw_protocol::v1::server_message::Msg as S;
use ocw_protocol::{client_message, decode_client, decode_server, encode};

/// Seconds to wait for the server's `Welcome`.
pub const CONNECT_TIMEOUT_S: f64 = 10.0;
/// Servers in the recent list.
pub const MAX_RECENT: usize = 5;

/// What a server rides, from its `Welcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCourse {
    /// A world package's road graph (free ride or one of its routes).
    pub world: Option<(String, u64)>,
    pub free_ride: bool,
    /// The package route ridden on the graph (empty: free ride).
    pub graph_route: String,
    /// The route id of a route ride (test route, route file).
    pub route_id: String,
}

impl ServerCourse {
    pub fn from_welcome(w: &ocw_protocol::v1::Welcome) -> Self {
        Self {
            world: w.world.as_ref().map(|p| (p.name.clone(), p.graph_hash)),
            free_ride: w.free_ride,
            graph_route: w.graph_route.clone(),
            route_id: w.route_id.clone(),
        }
    }

    /// "free ride in Liechtenstein", "route Malbun in …", "route loop".
    pub fn describe(&self) -> String {
        match &self.world {
            Some((name, _)) if !self.graph_route.is_empty() => {
                format!("route {} in {name}", self.graph_route)
            }
            Some((name, _)) => format!("free ride in {name}"),
            None => format!("route {}", self.route_id),
        }
    }
}

/// Why a join failed (the current ride goes on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinFailure {
    /// The link (or the fingerprint) is not valid.
    Invalid(String),
    /// No `Welcome` within [`CONNECT_TIMEOUT_S`].
    Timeout,
    /// The server's certificate is not the one in the link.
    Certificate(String),
    /// The server refused the rider; `auth`: sign-in needed or refused.
    Refused { reason: String, auth: bool },
    /// The server's world is not installed here (or not this version).
    WorldMissing(String),
    /// The world here differs from the server's.
    WorldMismatch(String),
    /// This app cannot reach that server (a browser and an `ocw://` link).
    Unsupported(String),
    /// The connection ended for another reason.
    Closed(String),
    /// Loading the server's course failed.
    Load(String),
    /// The rider cancelled.
    Cancelled,
}

impl JoinFailure {
    /// The reason for people.
    pub fn message(&self) -> String {
        match self {
            Self::Invalid(e) => e.clone(),
            Self::Timeout => format!(
                "no answer from the server within {CONNECT_TIMEOUT_S:.0} s (is it running, is UDP open on the way?)"
            ),
            Self::Certificate(e) => format!(
                "the server's certificate does not match the link's fingerprint ({e}); ask its host for the current link"
            ),
            Self::Refused { reason, auth: true } => {
                format!("the server wants you signed in: Settings -> Account ({reason})")
            }
            Self::Refused { reason, .. } => format!("the server refused: {reason}"),
            Self::WorldMissing(e) | Self::WorldMismatch(e) | Self::Unsupported(e) => e.clone(),
            Self::Closed(e) => format!("the connection ended: {e}"),
            Self::Load(e) => format!("cannot load the server's ride: {e}"),
            Self::Cancelled => "cancelled".into(),
        }
    }

    /// Sorts a closed link's reason: a certificate problem, a timeout,
    /// something else.
    pub fn from_closed(reason: &str) -> Self {
        let r = reason.to_ascii_lowercase();
        if r.contains("certificate")
            || r.contains("unknownissuer")
            || r.contains("fingerprint")
            || r.contains("does not match")
        {
            Self::Certificate(reason.to_owned())
        } else if r.contains("no answer") || r.contains("timed out") {
            Self::Timeout
        } else {
            Self::Closed(reason.to_owned())
        }
    }
}

/// Where a join is.
#[derive(Debug, Clone, PartialEq)]
pub enum JoinState {
    Idle,
    /// Connecting since `since_s`, waiting for the `Welcome`.
    Connecting {
        link: JoinLink,
        since_s: f64,
    },
    /// The server's course is known and being loaded here.
    Loading {
        link: JoinLink,
        course: ServerCourse,
    },
    /// Riding on the server.
    Joined {
        link: JoinLink,
        course: ServerCourse,
    },
    Failed {
        link: Option<JoinLink>,
        why: JoinFailure,
    },
}

/// One join, step by step (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct JoinFlow {
    pub state: JoinState,
}

impl Default for JoinFlow {
    fn default() -> Self {
        Self {
            state: JoinState::Idle,
        }
    }
}

impl JoinFlow {
    /// Validates `link` (parsed, the fingerprint rule) and starts
    /// connecting; the app opens the connection when this is `Ok`.
    pub fn start(
        &mut self,
        link: &str,
        fingerprint: &str,
        now_s: f64,
    ) -> Result<JoinLink, JoinFailure> {
        if self.busy() {
            return Err(JoinFailure::Invalid(
                "a join is already in progress (cancel it first)".into(),
            ));
        }
        let parsed = JoinLink::parse(link)
            .and_then(|l| l.with_fingerprint(fingerprint))
            .and_then(|l| l.check_trust().map(|()| l));
        match parsed {
            Ok(l) => {
                self.state = JoinState::Connecting {
                    link: l.clone(),
                    since_s: now_s,
                };
                Ok(l)
            }
            Err(e) => {
                let why = JoinFailure::Invalid(e);
                self.state = JoinState::Failed {
                    link: None,
                    why: why.clone(),
                };
                Err(why)
            }
        }
    }

    /// Connecting or loading.
    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            JoinState::Connecting { .. } | JoinState::Loading { .. }
        )
    }

    pub fn joined(&self) -> Option<&JoinLink> {
        match &self.state {
            JoinState::Joined { link, .. } => Some(link),
            _ => None,
        }
    }

    pub fn link(&self) -> Option<&JoinLink> {
        match &self.state {
            JoinState::Connecting { link, .. }
            | JoinState::Loading { link, .. }
            | JoinState::Joined { link, .. } => Some(link),
            JoinState::Failed { link, .. } => link.as_ref(),
            JoinState::Idle => None,
        }
    }

    /// A server message on the probe connection: the `Welcome` gives the
    /// course (→ loading), a `Reject` ends the join.
    pub fn on_message(&mut self, bytes: &[u8]) -> Option<ServerCourse> {
        let JoinState::Connecting { link, .. } = &self.state else {
            return None;
        };
        match decode_server(bytes) {
            Ok(S::Welcome(w)) => {
                let course = ServerCourse::from_welcome(&w);
                self.state = JoinState::Loading {
                    link: link.clone(),
                    course: course.clone(),
                };
                Some(course)
            }
            Ok(S::Reject(r)) => {
                self.fail(JoinFailure::Refused {
                    reason: r.reason,
                    auth: r.auth,
                });
                None
            }
            _ => None,
        }
    }

    /// The probe connection closed before the switch.
    pub fn on_closed(&mut self, reason: &str) {
        if self.busy() {
            self.fail(JoinFailure::from_closed(reason));
        }
    }

    /// The clock: `true` once the join timed out (no `Welcome` in time).
    pub fn tick(&mut self, now_s: f64) -> bool {
        if let JoinState::Connecting { since_s, .. } = self.state
            && now_s - since_s > CONNECT_TIMEOUT_S
        {
            self.fail(JoinFailure::Timeout);
            return true;
        }
        false
    }

    /// The course is loaded and the session switched.
    pub fn switched(&mut self) {
        if let JoinState::Loading { link, course } = &self.state {
            self.state = JoinState::Joined {
                link: link.clone(),
                course: course.clone(),
            };
        }
    }

    /// Ends the join (or the shared ride) with `why`.
    pub fn fail(&mut self, why: JoinFailure) {
        let link = self.link().cloned();
        self.state = JoinState::Failed { link, why };
    }

    /// Left the shared ride (or never joined): idle again.
    pub fn leave(&mut self) {
        self.state = JoinState::Idle;
    }

    /// The status line of the Join page.
    pub fn status_line(&self, now_s: f64) -> Option<String> {
        match &self.state {
            JoinState::Idle => None,
            JoinState::Connecting { link, since_s } => Some(format!(
                "Connecting to {} … {:.0} s",
                link.server_spec(),
                (now_s - since_s).max(0.0)
            )),
            JoinState::Loading { course, .. } => {
                Some(format!("Loading the server's {} …", course.describe()))
            }
            JoinState::Joined { link, course } => Some(format!(
                "Riding on {}: {}",
                link.server_spec(),
                course.describe()
            )),
            JoinState::Failed { why, .. } => Some(format!("Not joined: {}", why.message())),
        }
    }
}

/// The rider's encoded hello without the world (see the module docs);
/// other messages unchanged.
pub fn probe_hello(hello: &[u8]) -> Vec<u8> {
    match decode_client(hello) {
        Ok(C::Hello(mut h)) => {
            h.world = None;
            encode(&client_message(C::Hello(h)))
        }
        _ => hello.to_vec(),
    }
}

/// Whether to keep a message received before the switch: all reliable
/// ones; snapshots only the newest few (the app keeps the last
/// [`KEEP_SNAPSHOTS`]).
pub fn is_snapshot(bytes: &[u8]) -> bool {
    matches!(decode_server(bytes), Ok(S::Snapshot(_)))
}

/// Snapshots kept from before the switch.
pub const KEEP_SNAPSHOTS: usize = 20;

/// The messages received on the probe connection, for the new client.
#[derive(Debug, Default, Clone)]
pub struct Replay {
    pub messages: Vec<Vec<u8>>,
}

impl Replay {
    /// Keeps `bytes`; old snapshots beyond [`KEEP_SNAPSHOTS`] are dropped.
    pub fn push(&mut self, bytes: Vec<u8>) {
        if is_snapshot(&bytes) {
            let n = self.messages.iter().filter(|m| is_snapshot(m)).count();
            if n >= KEEP_SNAPSHOTS
                && let Some(i) = self.messages.iter().position(|m| is_snapshot(m))
            {
                self.messages.remove(i);
            }
        }
        self.messages.push(bytes);
    }
}

/// One server of the recent list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentJoin {
    pub link: String,
    /// What it rode the last time (`ServerCourse::describe`), if known.
    pub world: String,
}

impl RecentJoin {
    /// "host:port – world" for the menu.
    pub fn label(&self) -> String {
        let host = JoinLink::parse(&self.link).map_or_else(
            |_| self.link.clone(),
            |l| l.name.clone().unwrap_or_else(|| l.server_spec()),
        );
        if self.world.is_empty() {
            host
        } else {
            format!("{host} – {}", self.world)
        }
    }
}

/// The last servers joined, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentJoins {
    pub entries: Vec<RecentJoin>,
}

impl RecentJoins {
    /// Reads the stored text (unreadable lines are left out).
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .filter_map(|l| {
                let (link, world) = l.split_once('\t').unwrap_or((l, ""));
                JoinLink::parse(link).ok().map(|_| RecentJoin {
                    link: link.trim().to_owned(),
                    world: world.trim().to_owned(),
                })
            })
            .take(MAX_RECENT)
            .collect();
        Self { entries }
    }

    pub fn to_text(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("{}\t{}\n", e.link, e.world))
            .collect()
    }

    /// Puts `link` at the front (the same server once: host and port).
    pub fn push(&mut self, link: &JoinLink, world: &str) {
        let spec = link.server_spec();
        self.entries
            .retain(|e| !JoinLink::parse(&e.link).is_ok_and(|l| l.server_spec() == spec));
        self.entries.insert(
            0,
            RecentJoin {
                link: link.to_link(),
                world: world.to_owned(),
            },
        );
        self.entries.truncate(MAX_RECENT);
    }

    /// The list for `join` without a link.
    pub fn listing(&self) -> String {
        if self.entries.is_empty() {
            return "no servers joined yet: join LINK (ocw://HOST:PORT?cert=…)".into();
        }
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| format!("{}. {}\n   {}", i + 1, e.label(), e.link))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `join N`: the N-th server (1 = the newest).
    pub fn get(&self, n: usize) -> Option<&RecentJoin> {
        n.checked_sub(1).and_then(|i| self.entries.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::v1;

    const FP: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn welcome(world: Option<(&str, u64)>, route: &str, free: bool) -> Vec<u8> {
        encode(&ocw_protocol::server_message(S::Welcome(v1::Welcome {
            rider_id: 3,
            route_id: route.into(),
            free_ride: free,
            world: world.map(|(n, h)| v1::WorldPackageId {
                name: n.into(),
                graph_hash: h,
            }),
            ..Default::default()
        })))
    }

    #[test]
    fn validate_connect_welcome_load_switch_leave() {
        let mut f = JoinFlow::default();
        let link = f
            .start(&format!("ocw://10.0.0.9?cert={FP}"), "", 1.0)
            .unwrap();
        assert_eq!(link.server_spec(), "10.0.0.9:47800");
        assert!(f.busy());
        assert!(
            f.status_line(4.2)
                .unwrap()
                .contains("Connecting to 10.0.0.9:47800 … 3 s")
        );
        assert!(!f.tick(5.0));
        let c = f
            .on_message(&welcome(Some(("Liechtenstein", 7)), "", true))
            .unwrap();
        assert_eq!(c.describe(), "free ride in Liechtenstein");
        assert!(matches!(f.state, JoinState::Loading { .. }));
        // No timeout while loading.
        assert!(!f.tick(100.0));
        f.switched();
        assert_eq!(f.joined().unwrap().host, "10.0.0.9");
        assert!(
            f.status_line(0.0)
                .unwrap()
                .starts_with("Riding on 10.0.0.9:47800")
        );
        f.leave();
        assert_eq!(f.state, JoinState::Idle);
    }

    #[test]
    fn invalid_links_and_the_localhost_rule_fail_at_once() {
        let mut f = JoinFlow::default();
        let e = f.start("ocw://10.0.0.9", "", 0.0).unwrap_err();
        assert!(e.message().contains("fingerprint"), "{}", e.message());
        assert!(!f.busy());
        let e = f.start("10.0.0.9", "abc", 0.0).unwrap_err();
        assert!(e.message().contains("64 hex digits; got 3"));
        // The fingerprint entered separately.
        assert!(f.start("10.0.0.9", FP, 0.0).is_ok());
        assert!(
            f.start("localhost", "", 0.0)
                .unwrap_err()
                .message()
                .contains("already")
        );
        let mut f = JoinFlow::default();
        assert!(f.start("localhost:47800", "", 0.0).is_ok());
    }

    #[test]
    fn timeout_refusal_and_certificate_mismatch() {
        let mut f = JoinFlow::default();
        f.start("localhost", "", 0.0).unwrap();
        assert!(f.tick(CONNECT_TIMEOUT_S + 0.5));
        assert!(f.status_line(0.0).unwrap().contains("no answer"));
        let mut f = JoinFlow::default();
        f.start("localhost", "", 0.0).unwrap();
        let reject = encode(&ocw_protocol::server_message(S::Reject(v1::Reject {
            reason: "sign in first".into(),
            auth: true,
            name_taken: false,
        })));
        assert!(f.on_message(&reject).is_none());
        assert!(f.status_line(0.0).unwrap().contains("Settings -> Account"));
        let mut f = JoinFlow::default();
        f.start("localhost", "", 0.0).unwrap();
        f.on_closed("cannot connect: the cryptographic handshake failed: invalid peer certificate: certificate fingerprint mismatch");
        assert!(matches!(
            f.state,
            JoinState::Failed {
                why: JoinFailure::Certificate(_),
                ..
            }
        ));
        assert!(JoinFailure::from_closed("no answer from the server") == JoinFailure::Timeout);
        assert!(matches!(
            JoinFailure::from_closed("the server closed the connection"),
            JoinFailure::Closed(_)
        ));
        // A world that is not here: the app says so.
        let mut f = JoinFlow::default();
        f.start("localhost", "", 0.0).unwrap();
        f.on_message(&welcome(None, "loop-1", false)).unwrap();
        f.fail(JoinFailure::WorldMissing(
            "the server rides X, not installed".into(),
        ));
        assert!(f.status_line(0.0).unwrap().contains("not installed"));
        assert!(f.link().is_some());
    }

    #[test]
    fn the_probe_hello_has_no_world_and_the_replay_keeps_few_snapshots() {
        let h = encode(&client_message(C::Hello(v1::Hello {
            display_name: "Ada".into(),
            world: Some(v1::WorldPackageId {
                name: "x".into(),
                graph_hash: 1,
            }),
            ..Default::default()
        })));
        match decode_client(&probe_hello(&h)) {
            Ok(C::Hello(h)) => assert!(h.world.is_none() && h.display_name == "Ada"),
            other => panic!("{other:?}"),
        }
        let mut r = Replay::default();
        r.push(welcome(None, "loop", false));
        let snap = encode(&ocw_protocol::server_message(S::Snapshot(
            v1::Snapshot::default(),
        )));
        for _ in 0..50 {
            r.push(snap.clone());
        }
        assert_eq!(r.messages.len(), KEEP_SNAPSHOTS + 1);
        assert!(!is_snapshot(&r.messages[0]), "the welcome stays first");
    }

    #[test]
    fn the_recent_list_keeps_five_servers_newest_first() {
        let mut r = RecentJoins::default();
        for i in 0..7 {
            let mut l = JoinLink::new(format!("10.0.0.{i}"), 47800);
            l.cert = Some(FP.into());
            r.push(&l, &format!("world {i}"));
        }
        assert_eq!(r.entries.len(), MAX_RECENT);
        assert_eq!(r.get(1).unwrap().label(), "10.0.0.6:47800 – world 6");
        // The same server again moves to the front, once.
        let mut l = JoinLink::new("10.0.0.3", 47800);
        l.cert = Some(FP.into());
        r.push(&l, "free ride in Tenerife");
        assert_eq!(r.entries.len(), MAX_RECENT);
        assert_eq!(r.get(1).unwrap().world, "free ride in Tenerife");
        assert_eq!(
            r.entries
                .iter()
                .filter(|e| e.link.contains("10.0.0.3"))
                .count(),
            1
        );
        let back = RecentJoins::parse(&(r.to_text() + "garbage line\n"));
        assert_eq!(back, r);
        assert!(
            r.listing()
                .starts_with("1. 10.0.0.3:47800 – free ride in Tenerife")
        );
        assert!(RecentJoins::default().listing().contains("no servers"));
    }
}
