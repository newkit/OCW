//! Following the host of a shared ride to a world signed by a user key
//! (spec §4.7 *User keys*; §14 Phase 16, 16C *Signing and trust*).
//!
//! When the host switches the world (`WorldRequest` → the server's
//! `WorldChange`), every client loads its own copy of the package. A copy
//! signed by a user key this client does not trust is not ridden silently:
//! the client asks with the trust dialog ([`Follow::Ask`]); a rider who
//! declines **leaves the shared ride** ([`leave_message`]) – they never
//! ride a world they did not accept. A damaged or tampered copy is refused
//! ([`Follow::Refuse`]), never asked about.
//!
//! The server checks its own trust: the keys its operator configured
//! (`ocw-server --trust-key`, its account's `trusted-keys.json`); a client
//! never makes the server trust a key. The desktop does the same with the
//! files (`ocw_worldstore::consent_needed`).

use ocw_protocol::v1;
use ocw_world::manifest::{SignedManifest, TrustStore};
use ocw_worldstore::userkeys::{Decision, TrustRequest, decide};

/// What a client does with the host's world change.
#[derive(Debug, Clone, PartialEq)]
pub enum Follow {
    /// Load the copy and join again.
    Load,
    /// Ask the rider first (the trust dialog, shared: Cancel leaves).
    Ask(Box<TrustRequest>),
    /// Never ride this copy: leave the shared ride with the reason.
    Refuse(String),
}

/// The decision for `change` with the client's own copy's manifest
/// `local` under its `trust` at `now_unix_s`.
pub fn follow(
    change: &v1::WorldChange,
    local: &SignedManifest,
    trust: &TrustStore,
    now_unix_s: i64,
) -> Follow {
    let m = &local.manifest;
    if m.id != change.package_id || m.version != change.version {
        return Follow::Refuse(format!(
            "your copy is {}@{}, the host rides {}@{}",
            m.id, m.version, change.package_id, change.version
        ));
    }
    match decide(local, trust, now_unix_s) {
        // Unsigned or another trust failure: as before (the content hash
        // still has to match the server's).
        Decision::Load(_) | Decision::Policy(_) => Follow::Load,
        Decision::Ask(r) => Follow::Ask(r),
        Decision::Refuse(e) => Follow::Refuse(e.to_string()),
    }
}

/// The message of a rider who declined the host's world.
pub fn leave_message(world_name: &str) -> String {
    format!("You left the shared ride: you did not trust the key that signed {world_name}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientCore, Notice};
    use ocw_core::RiderProfile;
    use ocw_server::{GameServer, ServerConfig};
    use ocw_world::Route;
    use ocw_world::manifest::{Attribution, Manifest, RootKeyPair, SigningKeyPair};

    const NOW: i64 = 1_800_000_000;

    /// A throwaway user key (test only; fictional names) and a manifest it
    /// signed.
    fn ada_world() -> (RootKeyPair, SignedManifest) {
        let root = RootKeyPair::from_seed("Ada Rider", [111; 32]);
        let cert = root.certify(
            "Ada Rider",
            SigningKeyPair::public_key_of([112; 32]),
            NOW - 100,
            NOW + 100_000,
        );
        let key = SigningKeyPair::new([112; 32], cert)
            .unwrap()
            .with_root(root.public())
            .unwrap();
        let mut m = Manifest::new(
            "user/ada/strip",
            "Ada's Strip",
            1,
            NOW,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("strip.ocwpkg", b"package bytes").unwrap();
        (root, SignedManifest::signed(&m, &key))
    }

    /// Joins `client` on `server` and lets a few ticks pass.
    fn join(server: &mut GameServer, client: &mut ClientCore, conn: ocw_server::ConnectionId) {
        for r in server.handle(conn, &client.hello()) {
            client.on_server_message(&r, 0.0);
        }
    }

    #[test]
    fn a_rider_who_does_not_trust_the_hosts_world_leaves_the_ride() {
        let (ada, copy) = ada_world();
        let config = ServerConfig {
            world_access: ocw_protocol::ClockAccess::Host,
            ..Default::default()
        };
        let mut server = GameServer::new(Route::test_loop(), config);
        let (hc, gc) = (server.connect(), server.connect());
        let mut host = ClientCore::new(Route::test_loop(), "Host", RiderProfile::default());
        let mut guest = ClientCore::new(Route::test_loop(), "Guest", RiderProfile::default());
        join(&mut server, &mut host, hc);
        join(&mut server, &mut guest, gc);
        for b in (0..5).flat_map(|_| server.tick()) {
            host.on_server_message(&b, 0.1);
            guest.on_server_message(&b, 0.1);
        }
        assert_eq!(server.connections().count(), 2);
        // The host switched to Ada's world (the server trusts her key: its
        // operator configured it; the server loaded the package).
        let change = v1::WorldChange {
            package_id: "user/ada/strip".into(),
            version: 1,
            name: "Ada's Strip".into(),
            ..Default::default()
        };
        let (conns, bytes) = server.switch_world(Route::test_loop(), change);
        assert_eq!(conns.len(), 2);
        let notice = |c: &mut ClientCore| match c.on_server_message(&bytes, 0.2) {
            Some(Notice::WorldChange(w)) => w,
            n => panic!("{n:?}"),
        };
        let (hw, gw) = (notice(&mut host), notice(&mut guest));
        // The host trusts the key: follows and joins again.
        let host_trust =
            ocw_worldstore::userkeys::with_user_keys(TrustStore::default(), &[ada.public()]);
        assert_eq!(follow(&hw, &copy, &host_trust, NOW), Follow::Load);
        join(&mut server, &mut host, hc);
        // The guest does not: asked, with the claimed name and the key.
        let Follow::Ask(r) = follow(&gw, &copy, &TrustStore::default(), NOW) else {
            panic!("the guest is asked");
        };
        assert_eq!(r.claimed(), "signed by 'Ada Rider' (unverified name)");
        assert_eq!(r.fingerprint(), ada.public().fingerprint());
        // Declined: the guest leaves – no hello, never in the new session.
        let msg = leave_message(&gw.name);
        assert!(msg.contains("left the shared ride") && msg.contains("Ada's Strip"));
        for b in (0..5).flat_map(|_| server.tick()) {
            host.on_server_message(&b, 0.3);
        }
        let riding: Vec<_> = server.connections().collect();
        assert_eq!(riding, [hc], "only the host rides the new world");
        assert!(host.rider_id().is_some());
        assert_eq!(guest.rider_id(), None);
        // Another package under the same name, or a tampered copy:
        // refused, never asked about.
        let other = v1::WorldChange {
            version: 2,
            ..gw.clone()
        };
        assert!(matches!(
            follow(&other, &copy, &TrustStore::default(), NOW),
            Follow::Refuse(_)
        ));
        let tampered =
            SignedManifest::parse(&copy.to_json().replacen("Ada's Strip", "Ada's Strap", 1))
                .unwrap();
        assert!(matches!(
            follow(&gw, &tampered, &TrustStore::default(), NOW),
            Follow::Refuse(e) if e.contains("signature")
        ));
    }
}
