//! Automatic port mapping on the router while hosting (spec §7.1.1
//! *Network*, §7.1.6 V2, §7.1.7 Q4): UPnP-IGD through the `igd-next` crate.
//!
//! A host asks the router (the Internet Gateway Device found by SSDP) to
//! forward the hosting's ports to this computer – the game's UDP port
//! (47800; 47801 when browsers join over WebTransport) and the bundled
//! voice server's TCP 7880, TCP 7881 and UDP 7882 – for the hosting's
//! lifetime, and removes the mappings again when the hosting stops
//! ([`PortMapping::close`], also on drop). The mappings are leased
//! ([`LEASE_S`]) and renewed at half the lease, so a crashed host leaves
//! nothing open for long; a router that takes only permanent leases gets
//! those (removed at the stop like the others). The router's external
//! address is the hosting's public address (join link, voice URL) when
//! it is a public one.
//!
//! **Never fatal:** no router answering, a router without UPnP, a refused
//! mapping – each is a [`MapState`] the hosting shows (mapped / not
//! available / failed and why), and the hosting goes on with the
//! firewall / port-forwarding hint.
//!
//! NAT-PMP / PCP (RFC 6886 / 6887) are not done here: the maintained crate
//! for both (`crab_nat`) needs the default gateway's address, which needs
//! one more crate for the routing table (spec §7.1.8 *Open*, §7.1.7 Q13).
//!
//! The router is behind the [`Igd`] trait: tests use a fake one (no real
//! router), and the real one ([`upnp`]) is tested against a fake IGD
//! (SSDP + description + SOAP on this machine) in `tests`.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::voice::launcher::VoicePorts;

/// The lease asked for each mapping (s); renewed at half of it.
pub const LEASE_S: u32 = 3600;
/// How long the router search waits for an answer.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(3);

/// TCP or UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proto {
    Tcp,
    Udp,
}

impl std::fmt::Display for Proto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        })
    }
}

/// One port of the hosting (the same number outside and inside).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPort {
    pub proto: Proto,
    pub port: u16,
    /// For people: "game", "voice signalling", …
    pub what: &'static str,
}

impl std::fmt::Display for HostPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} ({})", self.proto, self.port, self.what)
    }
}

/// The ports a hosting needs others to reach (spec §7.1.1 *Ports of a
/// host*): the game's UDP port, WebTransport's for browsers if it is
/// open, the voice server's three if it runs.
pub fn host_ports(game_udp: u16, web_udp: Option<u16>, voice: Option<VoicePorts>) -> Vec<HostPort> {
    let mut out = vec![HostPort {
        proto: Proto::Udp,
        port: game_udp,
        what: "game",
    }];
    if let Some(p) = web_udp.filter(|&p| p != game_udp) {
        out.push(HostPort {
            proto: Proto::Udp,
            port: p,
            what: "game, browsers",
        });
    }
    if let Some(v) = voice {
        out.extend([
            HostPort {
                proto: Proto::Tcp,
                port: v.signal,
                what: "voice signalling",
            },
            HostPort {
                proto: Proto::Tcp,
                port: v.tcp,
                what: "voice over TCP",
            },
            HostPort {
                proto: Proto::Udp,
                port: v.udp,
                what: "voice",
            },
        ]);
    }
    out
}

/// Why a mapping was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The router takes only permanent mappings (lease 0).
    PermanentOnly,
    /// Another computer (or another program) has this port.
    InUse,
    Other(String),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermanentOnly => f.write_str("the router takes only permanent mappings"),
            Self::InUse => f.write_str("the router forwards this port elsewhere already"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

/// A router that maps ports (UPnP-IGD; a fake in tests).
pub trait Igd: Send {
    /// The router, for people and logs (its address).
    fn name(&self) -> String;
    /// This computer's address towards the router (where mapped traffic
    /// goes).
    fn local_ip(&self) -> Result<IpAddr, String>;
    /// The router's address on the internet side.
    fn external_ip(&self) -> Result<IpAddr, String>;
    fn add(&self, port: &HostPort, to: IpAddr, lease_s: u32) -> Result<(), AddError>;
    fn remove(&self, port: &HostPort) -> Result<(), String>;
}

/// Finds the router (blocking, at most a few seconds).
pub type Discover = Box<dyn FnOnce() -> Result<Box<dyn Igd>, String> + Send>;

/// What the port mapping did, for the hosting's pages and logs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MapState {
    /// Switched off (*Open the router's ports automatically*, `--upnp`).
    #[default]
    Off,
    /// Looking for the router.
    Searching,
    /// No router answered, or it does not map ports.
    NotAvailable(String),
    /// The router answered but mapped nothing.
    Failed(String),
    /// Mapped (all, or some: `failed` says which not and why).
    Mapped {
        router: String,
        /// The router's external address (shown even when not public).
        external: Option<IpAddr>,
        /// The external address is not public (double NAT, carrier-grade
        /// NAT): the mappings do not make this computer reachable.
        not_public: bool,
        mapped: Vec<HostPort>,
        failed: Vec<(HostPort, String)>,
        /// The leases are permanent (the router takes no others).
        permanent: bool,
    },
}

impl MapState {
    /// One line for people.
    pub fn summary(&self) -> String {
        match self {
            Self::Off => "router ports: not opened automatically (switched off)".into(),
            Self::Searching => "router ports: looking for the router (UPnP) …".into(),
            Self::NotAvailable(why) => format!("router ports: not available – {why}"),
            Self::Failed(why) => format!("router ports: failed – {why}"),
            Self::Mapped {
                router,
                external,
                not_public,
                mapped,
                failed,
                ..
            } => {
                let list = mapped
                    .iter()
                    .map(|p| format!("{} {}", p.proto, p.port))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut s = format!("router ports: opened by UPnP on {router}: {list}");
                if let Some(ip) = external {
                    s.push_str(&format!("; the router's address {ip}"));
                    if *not_public {
                        s.push_str(
                            " is not public (double NAT or carrier-grade NAT): riders from the internet still cannot reach this computer",
                        );
                    }
                }
                for (p, why) in failed {
                    s.push_str(&format!("; {p} not opened: {why}"));
                }
                s
            }
        }
    }

    /// The router's public address, when the mapping makes this computer
    /// reachable through it (the hosting's public address, §7.1.7 Q5).
    pub fn public_ip(&self) -> Option<IpAddr> {
        match self {
            Self::Mapped {
                external: Some(ip),
                not_public: false,
                mapped,
                ..
            } if !mapped.is_empty() => Some(*ip),
            _ => None,
        }
    }
}

/// Whether `ip` is an address others on the internet reach.
pub fn is_public(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    !(ip.is_unspecified()
        || ip.is_multicast()
        || crate::voice::is_local_network(ip)
        || matches!(ip, IpAddr::V4(v4) if v4.is_broadcast() || v4.is_documentation()))
}

/// The hosting's mappings on the router; removed by [`PortMapping::close`]
/// or when dropped.
pub struct PortMapping {
    igd: Box<dyn Igd>,
    to: IpAddr,
    mapped: Vec<HostPort>,
    /// The lease the router took (0: permanent).
    lease_s: u32,
    renewed: Instant,
}

impl std::fmt::Debug for PortMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortMapping")
            .field("router", &self.igd.name())
            .field("to", &self.to)
            .field("mapped", &self.mapped)
            .field("lease_s", &self.lease_s)
            .finish()
    }
}

impl PortMapping {
    /// Finds the router and maps `ports` to this computer. Never an error:
    /// the state says what happened; `None` when nothing is mapped.
    pub fn open(discover: Discover, ports: &[HostPort], lease_s: u32) -> (Option<Self>, MapState) {
        let igd = match discover() {
            Ok(igd) => igd,
            Err(why) => return (None, MapState::NotAvailable(why)),
        };
        let router = igd.name();
        let to = match igd.local_ip() {
            Ok(ip) => ip,
            Err(why) => {
                return (
                    None,
                    MapState::Failed(format!("no address towards {router}: {why}")),
                );
            }
        };
        let external = match igd.external_ip() {
            Ok(ip) => Some(ip),
            Err(why) => {
                tracing::debug!(%why, "the router's external address");
                None
            }
        };
        let mut lease = lease_s;
        let mut mapped = Vec::new();
        let mut failed = Vec::new();
        for p in ports {
            let mut r = igd.add(p, to, lease);
            if r == Err(AddError::PermanentOnly) && lease != 0 {
                lease = 0;
                r = igd.add(p, to, 0);
            }
            match r {
                Ok(()) => mapped.push(*p),
                Err(e) => failed.push((*p, e.to_string())),
            }
        }
        if mapped.is_empty() {
            let why = failed.first().map_or_else(
                || "no ports to open".to_owned(),
                |(p, e)| format!("{p}: {e}"),
            );
            return (None, MapState::Failed(format!("{router} refused: {why}")));
        }
        let state = MapState::Mapped {
            router,
            external,
            not_public: external.is_some_and(|ip| !is_public(ip)),
            mapped: mapped.clone(),
            failed,
            permanent: lease == 0,
        };
        let m = Self {
            igd,
            to,
            mapped,
            lease_s: lease,
            renewed: Instant::now(),
        };
        (Some(m), state)
    }

    /// The mapped ports.
    pub fn mapped(&self) -> &[HostPort] {
        &self.mapped
    }

    /// Renews the leases when half of one has passed (call it now and
    /// then); the ports whose renewal failed, with why.
    pub fn renew_if_due(&mut self) -> Vec<(HostPort, String)> {
        if self.lease_s == 0
            || self.renewed.elapsed() < Duration::from_secs(u64::from(self.lease_s / 2))
        {
            return Vec::new();
        }
        self.renew()
    }

    /// Renews every lease now.
    pub fn renew(&mut self) -> Vec<(HostPort, String)> {
        self.renewed = Instant::now();
        self.mapped
            .iter()
            .filter_map(|p| {
                self.igd
                    .add(p, self.to, self.lease_s)
                    .err()
                    .map(|e| (*p, e.to_string()))
            })
            .collect()
    }

    /// Removes the mappings of the ports `which` picks (a voice server
    /// that did not start); what could not be removed.
    pub fn remove_where(&mut self, which: impl Fn(&HostPort) -> bool) -> Vec<(HostPort, String)> {
        let (gone, keep): (Vec<HostPort>, Vec<HostPort>) =
            self.mapped.iter().partition(|p| which(p));
        self.mapped = keep;
        gone.iter()
            .filter_map(|p| self.igd.remove(p).err().map(|e| (*p, e)))
            .collect()
    }

    /// Removes every mapping (also on drop); what could not be removed.
    pub fn close(&mut self) -> Vec<(HostPort, String)> {
        let out = self
            .mapped
            .iter()
            .filter_map(|p| self.igd.remove(p).err().map(|e| (*p, e)))
            .collect();
        self.mapped.clear();
        out
    }
}

impl Drop for PortMapping {
    fn drop(&mut self) {
        for (p, e) in self.close() {
            tracing::warn!(port = %p, %e, "a router port mapping could not be removed");
        }
    }
}

/// This computer's address on the way to `to` (no packet is sent).
pub fn local_ip_towards(to: SocketAddr) -> Result<IpAddr, String> {
    let bind: SocketAddr = if to.is_ipv6() {
        "[::]:0".parse().expect("valid")
    } else {
        "0.0.0.0:0".parse().expect("valid")
    };
    let s = UdpSocket::bind(bind).map_err(|e| e.to_string())?;
    s.connect(to).map_err(|e| e.to_string())?;
    let ip = s.local_addr().map_err(|e| e.to_string())?.ip();
    if ip.is_unspecified() {
        return Err("no route".into());
    }
    Ok(ip)
}

/// The real router: UPnP-IGD (`igd-next`, blocking).
pub mod upnp {
    use super::*;
    use igd_next::{AddPortError, PortMappingProtocol, SearchOptions};

    /// A router found by SSDP.
    pub struct UpnpGateway(pub igd_next::Gateway);

    fn proto(p: Proto) -> PortMappingProtocol {
        match p {
            Proto::Tcp => PortMappingProtocol::TCP,
            Proto::Udp => PortMappingProtocol::UDP,
        }
    }

    /// The mapping's description on the router's page.
    pub fn description(p: &HostPort) -> String {
        format!("Open Cycling World {}", p.what)
    }

    impl Igd for UpnpGateway {
        fn name(&self) -> String {
            format!("the router {}", self.0.addr.ip())
        }

        fn local_ip(&self) -> Result<IpAddr, String> {
            local_ip_towards(self.0.addr)
        }

        fn external_ip(&self) -> Result<IpAddr, String> {
            self.0.get_external_ip().map_err(|e| e.to_string())
        }

        fn add(&self, p: &HostPort, to: IpAddr, lease_s: u32) -> Result<(), AddError> {
            self.0
                .add_port(
                    proto(p.proto),
                    p.port,
                    SocketAddr::new(to, p.port),
                    lease_s,
                    &description(p),
                )
                .map_err(|e| match e {
                    AddPortError::OnlyPermanentLeasesSupported => AddError::PermanentOnly,
                    AddPortError::PortInUse => AddError::InUse,
                    e => AddError::Other(e.to_string()),
                })
        }

        fn remove(&self, p: &HostPort) -> Result<(), String> {
            self.0
                .remove_port(proto(p.proto), p.port)
                .map_err(|e| e.to_string())
        }
    }

    /// Searches the router with SSDP at `ssdp` (the UPnP multicast
    /// address `239.255.255.250:1900` normally; a fake IGD in tests).
    pub fn discover_at(ssdp: SocketAddr, bind: SocketAddr, timeout: Duration) -> Discover {
        Box::new(move || {
            let opts = SearchOptions {
                bind_addr: bind,
                broadcast_address: ssdp,
                timeout: Some(timeout),
                single_search_timeout: Some(timeout),
            };
            match igd_next::search_gateway(opts) {
                Ok(g) => Ok(Box::new(UpnpGateway(g)) as Box<dyn Igd>),
                Err(e) => Err(search_error(&e.to_string(), timeout)),
            }
        })
    }

    /// The router of this network (UPnP-IGD on the local network).
    pub fn discover() -> Discover {
        discover_at(
            "239.255.255.250:1900".parse().expect("valid"),
            "0.0.0.0:0".parse().expect("valid"),
            SEARCH_TIMEOUT,
        )
    }

    /// A failed search in words: most often no router answered in time
    /// (UPnP off on the router, or no router at all).
    pub fn search_error(e: &str, timeout: Duration) -> String {
        let lower = e.to_ascii_lowercase();
        if lower.contains("timed out")
            || lower.contains("would block")
            || lower.contains("temporarily unavailable")
            || lower.contains("os error 11")
            || lower.contains("os error 10060")
            || lower.contains("os error 35")
            || lower.contains("no response")
        {
            format!(
                "no UPnP router answered within {} s (UPnP may be off on the router)",
                timeout.as_secs_f32()
            )
        } else {
            format!("the router search failed: {e}")
        }
    }
}

/// What a [`MemoryRouter`] holds: (protocol, port) → (to, lease).
pub type RouterTable =
    std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<(String, u16), (IpAddr, u32)>>>;

/// A router in memory (tests here and in the apps: no real router).
#[derive(Clone, Default)]
pub struct MemoryRouter {
    pub table: RouterTable,
    /// What it says its external address is.
    pub external: Option<IpAddr>,
    /// Takes only permanent leases.
    pub permanent_only: bool,
    /// Ports someone else has.
    pub taken: Vec<u16>,
    /// Every add call: (port, lease).
    pub adds: std::sync::Arc<std::sync::Mutex<Vec<(u16, u32)>>>,
}

impl MemoryRouter {
    /// Finds this router.
    pub fn discover(self) -> Discover {
        Box::new(move || Ok(Box::new(self) as Box<dyn Igd>))
    }

    fn table(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<(String, u16), (IpAddr, u32)>> {
        self.table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Igd for MemoryRouter {
    fn name(&self) -> String {
        "the router 192.168.1.1".into()
    }
    fn local_ip(&self) -> Result<IpAddr, String> {
        Ok(IpAddr::from([192, 168, 1, 20]))
    }
    fn external_ip(&self) -> Result<IpAddr, String> {
        self.external.ok_or_else(|| "no address".into())
    }
    fn add(&self, p: &HostPort, to: IpAddr, lease_s: u32) -> Result<(), AddError> {
        self.adds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((p.port, lease_s));
        if self.permanent_only && lease_s != 0 {
            return Err(AddError::PermanentOnly);
        }
        if self.taken.contains(&p.port) {
            return Err(AddError::InUse);
        }
        self.table()
            .insert((p.proto.to_string(), p.port), (to, lease_s));
        Ok(())
    }
    fn remove(&self, p: &HostPort) -> Result<(), String> {
        self.table()
            .remove(&(p.proto.to_string(), p.port))
            .map(|_| ())
            .ok_or_else(|| "no such mapping".into())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    pub(crate) fn found(igd: MemoryRouter) -> Discover {
        igd.discover()
    }

    type Table = RouterTable;

    fn ports() -> Vec<HostPort> {
        host_ports(47800, None, Some(VoicePorts::default()))
    }

    #[test]
    fn the_hosting_ports_are_game_and_voice() {
        let p = ports();
        let shown: Vec<String> = p.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "UDP 47800 (game)",
                "TCP 7880 (voice signalling)",
                "TCP 7881 (voice over TCP)",
                "UDP 7882 (voice)"
            ]
        );
        // WebTransport for browsers when it is open; no voice ports without voice.
        let p = host_ports(47800, Some(47801), None);
        assert_eq!(p.len(), 2);
        assert_eq!((p[1].proto, p[1].port), (Proto::Udp, 47801));
    }

    #[test]
    fn mapped_for_the_lifetime_and_removed_after() {
        let igd = MemoryRouter {
            external: Some("203.0.113.7".parse().unwrap()),
            ..MemoryRouter::default()
        };
        let table = igd.table.clone();
        let (m, state) = PortMapping::open(found(igd), &ports(), LEASE_S);
        let mut m = m.expect("mapped");
        assert_eq!(m.mapped().len(), 4);
        assert_eq!(table.lock().unwrap().len(), 4);
        let to: IpAddr = "192.168.1.20".parse().unwrap();
        assert!(
            table
                .lock()
                .unwrap()
                .values()
                .all(|&(ip, lease)| ip == to && lease == LEASE_S)
        );
        // The documentation range is not reachable from the internet …
        assert_eq!(state.public_ip(), None);
        assert!(
            state
                .summary()
                .contains("UDP 47800, TCP 7880, TCP 7881, UDP 7882")
        );
        // … removed at the end.
        assert!(m.close().is_empty());
        assert!(table.lock().unwrap().is_empty());
        // Dropped: removed too.
        let igd = MemoryRouter {
            external: Some("81.2.69.160".parse().unwrap()),
            ..MemoryRouter::default()
        };
        let table = igd.table.clone();
        let (m, state) = PortMapping::open(found(igd), &ports(), LEASE_S);
        assert_eq!(state.public_ip(), Some("81.2.69.160".parse().unwrap()));
        assert_eq!(table.lock().unwrap().len(), 4);
        drop(m);
        assert!(table.lock().unwrap().is_empty());
    }

    #[test]
    fn a_router_with_permanent_leases_only_gets_those() {
        let igd = MemoryRouter {
            permanent_only: true,
            ..MemoryRouter::default()
        };
        let adds = igd.adds.clone();
        let table = igd.table.clone();
        let (m, state) = PortMapping::open(found(igd), &ports(), LEASE_S);
        let mut m = m.unwrap();
        assert!(matches!(
            state,
            MapState::Mapped {
                permanent: true,
                ..
            }
        ));
        // One refused try with the lease, then all permanent.
        let adds = adds.lock().unwrap().clone();
        assert_eq!(adds[0], (47800, LEASE_S));
        assert!(adds[1..].iter().all(|&(_, l)| l == 0), "{adds:?}");
        // Permanent leases are never renewed, but still removed.
        assert!(m.renew_if_due().is_empty());
        drop(m);
        assert!(table.lock().unwrap().is_empty());
    }

    #[test]
    fn leases_are_renewed_at_half() {
        let igd = MemoryRouter::default();
        let adds = igd.adds.clone();
        let (m, _) = PortMapping::open(found(igd), &ports(), 2);
        let mut m = m.unwrap();
        assert_eq!(adds.lock().unwrap().len(), 4);
        assert!(m.renew_if_due().is_empty());
        assert_eq!(adds.lock().unwrap().len(), 4, "not yet due");
        std::thread::sleep(Duration::from_millis(1100));
        assert!(m.renew_if_due().is_empty());
        assert_eq!(adds.lock().unwrap().len(), 8, "renewed");
    }

    #[test]
    fn no_router_or_refusals_are_states_not_errors() {
        let none: Discover = Box::new(|| Err("no UPnP router answered within 3 s".into()));
        let (m, state) = PortMapping::open(none, &ports(), LEASE_S);
        assert!(m.is_none());
        assert_eq!(
            state.summary(),
            "router ports: not available – no UPnP router answered within 3 s"
        );
        // Everything taken: failed, and why.
        let igd = MemoryRouter {
            taken: vec![47800, 7880, 7881, 7882],
            ..MemoryRouter::default()
        };
        let (m, state) = PortMapping::open(found(igd), &ports(), LEASE_S);
        assert!(m.is_none());
        assert!(
            matches!(&state, MapState::Failed(w) if w.contains("forwards this port elsewhere")),
            "{state:?}"
        );
        // Some taken: the others are mapped, the summary names the rest.
        let igd = MemoryRouter {
            taken: vec![7881],
            external: Some("100.64.3.4".parse().unwrap()),
            ..MemoryRouter::default()
        };
        let (m, state) = PortMapping::open(found(igd), &ports(), LEASE_S);
        assert_eq!(m.unwrap().mapped().len(), 3);
        let s = state.summary();
        assert!(s.contains("TCP 7881 (voice over TCP) not opened"), "{s}");
        // Carrier-grade NAT: the router's address is not public.
        assert!(s.contains("is not public"), "{s}");
        assert_eq!(state.public_ip(), None);
        assert_eq!(MapState::Off.public_ip(), None);
    }

    #[test]
    fn public_addresses() {
        for ip in ["81.2.69.160", "2a00:1450:4001:80b::200e"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "192.168.1.1",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "0.0.0.0",
            "203.0.113.7",
            "fd00::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn search_errors_in_words() {
        let t = Duration::from_secs(3);
        assert_eq!(
            upnp::search_error(
                "IO error: Resource temporarily unavailable (os error 11)",
                t
            ),
            "no UPnP router answered within 3 s (UPnP may be off on the router)"
        );
        assert!(upnp::search_error("Invalid response", t).starts_with("the router search failed"));
    }

    // ------------------------------------------- the real client, a fake IGD

    /// A fake Internet Gateway Device on this machine: answers the SSDP
    /// search on a UDP port and serves its description, its service
    /// description and the SOAP actions over HTTP – what a router does.
    pub(crate) struct FakeRouter {
        pub ssdp: SocketAddr,
        pub table: Table,
        /// The SOAP actions it got, in order.
        pub actions: Arc<Mutex<Vec<String>>>,
    }

    const DESC: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0"><device>
<deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
<deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
<deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
<serviceList><service>
<serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
<serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>
<SCPDURL>/wanip.xml</SCPDURL><controlURL>/ctl/IPConn</controlURL>
</service></serviceList></device></deviceList></device></deviceList></device></root>"#;

    fn scpd() -> String {
        let action = |name: &str, args: &[&str]| {
            let a: String = args
                .iter()
                .map(|n| format!("<argument><name>{n}</name><direction>in</direction></argument>"))
                .collect();
            format!("<action><name>{name}</name><argumentList>{a}</argumentList></action>")
        };
        format!(
            r#"<?xml version="1.0"?><scpd xmlns="urn:schemas-upnp-org:service-1-0"><actionList>{}{}{}</actionList></scpd>"#,
            action(
                "AddPortMapping",
                &[
                    "NewRemoteHost",
                    "NewExternalPort",
                    "NewProtocol",
                    "NewInternalPort",
                    "NewInternalClient",
                    "NewEnabled",
                    "NewPortMappingDescription",
                    "NewLeaseDuration"
                ]
            ),
            action(
                "DeletePortMapping",
                &["NewRemoteHost", "NewExternalPort", "NewProtocol"]
            ),
            action("GetExternalIPAddress", &[]),
        )
    }

    fn tag<'a>(body: &'a str, name: &str) -> Option<&'a str> {
        let start = body.find(&format!("<{name}>"))? + name.len() + 2;
        let end = body[start..].find(&format!("</{name}>"))? + start;
        Some(&body[start..end])
    }

    fn soap(inner: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body>{inner}</s:Body></s:Envelope>"#
        )
    }

    fn fault(code: u16, text: &str) -> String {
        soap(&format!(
            r#"<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>{code}</errorCode><errorDescription>{text}</errorDescription></UPnPError></detail></s:Fault>"#
        ))
    }

    impl FakeRouter {
        /// Starts it; `external` is what GetExternalIPAddress answers.
        pub(crate) fn start(external: &str) -> Self {
            use std::io::{BufRead, BufReader, Read, Write};
            let http = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let http_port = http.local_addr().unwrap().port();
            let ssdp = UdpSocket::bind("127.0.0.1:0").unwrap();
            let ssdp_addr = ssdp.local_addr().unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1500];
                while let Ok((n, from)) = ssdp.recv_from(&mut buf) {
                    if String::from_utf8_lossy(&buf[..n]).starts_with("M-SEARCH") {
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nUSN: uuid:fake-igd::urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: http://127.0.0.1:{http_port}/rootDesc.xml\r\n\r\n"
                        );
                        let _ = ssdp.send_to(reply.as_bytes(), from);
                    }
                }
            });
            let table: Table = Arc::default();
            let actions: Arc<Mutex<Vec<String>>> = Arc::default();
            let (t, a) = (table.clone(), actions.clone());
            let external = external.to_owned();
            std::thread::spawn(move || {
                for stream in http.incoming() {
                    let Ok(mut s) = stream else { break };
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap_or_default();
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                    let mut len = 0usize;
                    let mut action = String::new();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        let lower = h.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        if lower.starts_with("soapaction:") {
                            action = h
                                .rsplit('#')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .trim_matches('"')
                                .to_owned();
                        }
                    }
                    let mut body = vec![0u8; len];
                    r.read_exact(&mut body).unwrap_or_default();
                    let body = String::from_utf8_lossy(&body).into_owned();
                    let reply = match path.as_str() {
                        "/rootDesc.xml" => DESC.to_owned(),
                        "/wanip.xml" => scpd(),
                        _ => {
                            a.lock().unwrap().push(action.clone());
                            let key = || {
                                (
                                    tag(&body, "NewProtocol").unwrap_or("").to_owned(),
                                    tag(&body, "NewExternalPort")
                                        .and_then(|p| p.parse().ok())
                                        .unwrap_or(0),
                                )
                            };
                            match action.as_str() {
                                "GetExternalIPAddress" => soap(&format!(
                                    r#"<u:GetExternalIPAddressResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"><NewExternalIPAddress>{external}</NewExternalIPAddress></u:GetExternalIPAddressResponse>"#
                                )),
                                "AddPortMapping" => {
                                    let to = tag(&body, "NewInternalClient")
                                        .and_then(|c| c.parse().ok());
                                    let lease = tag(&body, "NewLeaseDuration")
                                        .and_then(|l| l.parse().ok())
                                        .unwrap_or(0);
                                    match to {
                                        Some(to) => {
                                            t.lock().unwrap().insert(key(), (to, lease));
                                            soap(
                                                r#"<u:AddPortMappingResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"/>"#,
                                            )
                                        }
                                        None => fault(402, "Invalid Args"),
                                    }
                                }
                                "DeletePortMapping" => match t.lock().unwrap().remove(&key()) {
                                    Some(_) => soap(
                                        r#"<u:DeletePortMappingResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"/>"#,
                                    ),
                                    None => fault(714, "NoSuchEntryInArray"),
                                },
                                _ => fault(401, "Invalid Action"),
                            }
                        }
                    };
                    let _ = write!(
                        s,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                }
            });
            Self {
                ssdp: ssdp_addr,
                table,
                actions,
            }
        }

        pub(crate) fn discover(&self) -> Discover {
            upnp::discover_at(
                self.ssdp,
                "127.0.0.1:0".parse().unwrap(),
                Duration::from_secs(5),
            )
        }
    }

    #[test]
    fn the_upnp_client_maps_and_removes_on_a_fake_igd() {
        let router = FakeRouter::start("81.2.69.160");
        let (m, state) = PortMapping::open(router.discover(), &ports(), LEASE_S);
        let m = m.unwrap_or_else(|| panic!("{state:?}"));
        assert_eq!(state.public_ip(), Some("81.2.69.160".parse().unwrap()));
        assert!(
            state
                .summary()
                .starts_with("router ports: opened by UPnP on the router 127.0.0.1"),
            "{}",
            state.summary()
        );
        {
            let t = router.table.lock().unwrap();
            assert_eq!(
                t.keys().cloned().collect::<Vec<_>>(),
                [
                    ("TCP".to_owned(), 7880),
                    ("TCP".to_owned(), 7881),
                    ("UDP".to_owned(), 7882),
                    ("UDP".to_owned(), 47800)
                ]
            );
            // To this computer's address towards the router, leased.
            assert!(
                t.values()
                    .all(|&(to, lease)| to.is_loopback() && lease == LEASE_S)
            );
        }
        drop(m);
        assert!(
            router.table.lock().unwrap().is_empty(),
            "removed at the end"
        );
        let actions = router.actions.lock().unwrap().clone();
        assert_eq!(actions[0], "GetExternalIPAddress");
        assert_eq!(actions.iter().filter(|a| *a == "AddPortMapping").count(), 4);
        assert_eq!(
            actions.iter().filter(|a| *a == "DeletePortMapping").count(),
            4
        );
    }

    #[test]
    fn no_igd_answering_is_not_available() {
        // A UDP port nobody answers on.
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let d = upnp::discover_at(
            silent.local_addr().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            Duration::from_millis(300),
        );
        let (m, state) = PortMapping::open(d, &ports(), LEASE_S);
        assert!(m.is_none());
        assert!(
            matches!(&state, MapState::NotAvailable(w) if w.starts_with("no UPnP router answered")),
            "{state:?}"
        );
    }
}
