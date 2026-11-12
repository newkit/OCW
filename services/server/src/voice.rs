//! Voice rooms and LiveKit access tokens (spec §7, §2.4, §12).
//!
//! Voice runs on a self-hosted LiveKit SFU, never over the game connection.
//! The game server only decides **which room** a rider may join and signs a
//! short-lived access token for it (a JWT, HS256 with the LiveKit API
//! secret). The client then connects to LiveKit directly.
//!
//! Rooms (all prefixed with [`VoiceConfig::room_prefix`] and the server
//! instance, so two servers sharing one LiveKit never share rooms):
//! - session: `PREFIX-INSTANCE` – everyone in the session; clients
//!   attenuate by distance on the road and hear only the nearest riders,
//! - event: `PREFIX-INSTANCE-event-ID` – the entrants of the running event,
//! - group: `PREFIX-INSTANCE-group-NAME` – a named group (private room).
//!
//! **Dev-mode assumption (no accounts before Phase 5):** a token is bound
//! to the rider id of the connection (identity `rider-<id>`,
//! [`ocw_protocol::voice_identity`]) and its display name, not to an
//! account; a group room is open to everyone in the session who knows its
//! name. With accounts (OIDC, §12) the identity becomes the account and
//! groups get invitations.
//!
//! The LiveKit URL, API key and secret come from the environment
//! (`OCW_LIVEKIT_URL`, `OCW_LIVEKIT_API_KEY`, `OCW_LIVEKIT_API_SECRET`) or
//! the server's command line; they are never part of the repository.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ocw_core::RiderId;
use ocw_protocol::v1;
use sha2::{Digest, Sha256};

/// Default lifetime of an access token. LiveKit checks it when a client
/// joins (and refreshes it itself for a connected client), so it only
/// bounds how long a token can be used to join; clients ask again before
/// reconnecting.
pub const DEFAULT_TOKEN_TTL_S: u64 = 10 * 60;
/// A connection gets at most one token per this many seconds.
pub const MIN_REQUEST_INTERVAL_S: u64 = 2;
/// Environment variables of the LiveKit server and its API key pair.
pub const ENV_URL: &str = "OCW_LIVEKIT_URL";
pub const ENV_API_KEY: &str = "OCW_LIVEKIT_API_KEY";
pub const ENV_API_SECRET: &str = "OCW_LIVEKIT_API_SECRET";

/// Unix time in seconds (a function, so tests can fix it).
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    })
}

/// LiveKit connection data of a game server.
#[derive(Clone)]
pub struct VoiceConfig {
    /// The LiveKit server clients connect to (`wss://…` or `ws://…`).
    pub url: String,
    pub api_key: String,
    pub api_secret: String,
    pub token_ttl_s: u64,
    /// First part of every room name.
    pub room_prefix: String,
    /// This server instance (part of every room name).
    pub instance: String,
    pub clock: Clock,
}

impl std::fmt::Debug for VoiceConfig {
    // Never print the secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceConfig")
            .field("url", &self.url)
            .field("api_key", &self.api_key)
            .field("token_ttl_s", &self.token_ttl_s)
            .field("room_prefix", &self.room_prefix)
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

impl VoiceConfig {
    /// A configuration with the default lifetime, room prefix `ocw`, a new
    /// instance id and the system clock.
    pub fn new(
        url: impl Into<String>,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            token_ttl_s: DEFAULT_TOKEN_TTL_S,
            room_prefix: "ocw".into(),
            instance: new_instance_id(),
            clock: system_clock(),
        }
    }

    /// From explicit values, falling back to the environment variables
    /// [`ENV_URL`], [`ENV_API_KEY`], [`ENV_API_SECRET`]. `Ok(None)`: none of
    /// them is set (voice off); an error: only some are.
    pub fn from_parts(
        url: Option<String>,
        api_key: Option<String>,
        api_secret: Option<String>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, String> {
        let pick = |v: Option<String>, name: &str| {
            v.or_else(|| env(name)).filter(|s| !s.trim().is_empty())
        };
        let url = pick(url, ENV_URL);
        let key = pick(api_key, ENV_API_KEY);
        let secret = pick(api_secret, ENV_API_SECRET);
        match (url, key, secret) {
            (None, None, None) => Ok(None),
            (Some(u), Some(k), Some(s)) => {
                if !(u.starts_with("ws://")
                    || u.starts_with("wss://")
                    || u.starts_with("http://")
                    || u.starts_with("https://"))
                {
                    return Err(format!(
                        "LiveKit URL {u:?}: expected ws://, wss://, http:// or https://"
                    ));
                }
                Ok(Some(Self::new(u, k, s)))
            }
            (u, k, s) => {
                let missing: Vec<&str> = [
                    (u.is_none(), ENV_URL),
                    (k.is_none(), ENV_API_KEY),
                    (s.is_none(), ENV_API_SECRET),
                ]
                .iter()
                .filter(|(m, _)| *m)
                .map(|(_, n)| *n)
                .collect();
                Err(format!(
                    "voice: LiveKit configuration incomplete, missing {}",
                    missing.join(", ")
                ))
            }
        }
    }

    /// The session room.
    pub fn session_room(&self) -> String {
        format!("{}-{}", self.room_prefix, self.instance)
    }

    pub fn event_room(&self, event_id: u32) -> String {
        format!("{}-event-{event_id}", self.session_room())
    }

    /// A group room (`name` checked with [`ocw_protocol::valid_voice_group`]).
    pub fn group_room(&self, name: &str) -> String {
        format!("{}-group-{name}", self.session_room())
    }

    /// A LiveKit access token for `identity` in `room`, valid from now for
    /// [`VoiceConfig::token_ttl_s`]: join, publish the microphone only,
    /// subscribe; no data, no video, no room administration.
    pub fn access_token(&self, room: &str, identity: &str, name: &str, now_unix_s: u64) -> String {
        let claims = serde_json::json!({
            "iss": self.api_key,
            "sub": identity,
            "name": name,
            "nbf": now_unix_s,
            "exp": now_unix_s + self.token_ttl_s,
            "video": {
                "room": room,
                "roomJoin": true,
                "canPublish": true,
                "canSubscribe": true,
                "canPublishData": false,
                "canPublishSources": ["microphone"],
                "canUpdateOwnMetadata": false,
            },
        });
        jwt_hs256(&claims, self.api_secret.as_bytes())
    }
}

/// A server instance id: hex of a hash of the time and process id (no
/// randomness needed: it only keeps the rooms of servers apart).
fn new_instance_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut h = Sha256::new();
    h.update(nanos.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What a rider asked for and is allowed into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomChoice {
    Session,
    Event(u32),
    Group(String),
}

/// Decides the room for a request: the session room (also for an
/// unspecified channel), the running event's room for its entrants, or a
/// valid group name.
pub fn choose_room(
    req: &v1::VoiceTokenRequest,
    rider: RiderId,
    event: Option<&ocw_sim::Event>,
) -> Result<RoomChoice, String> {
    match req.channel() {
        v1::VoiceChannel::Unspecified | v1::VoiceChannel::Session => Ok(RoomChoice::Session),
        v1::VoiceChannel::Event => match event {
            Some(ev) if ev.entrants.iter().any(|e| e.id == rider) => Ok(RoomChoice::Event(ev.id)),
            Some(_) => Err("you are not an entrant of the current event".into()),
            None => Err("there is no event in this session".into()),
        },
        v1::VoiceChannel::Group => {
            if ocw_protocol::valid_voice_group(&req.group) {
                Ok(RoomChoice::Group(req.group.clone()))
            } else {
                Err(format!(
                    "invalid group name {:?}: 1 to {} characters of a-z, 0-9 and -",
                    req.group,
                    ocw_protocol::MAX_VOICE_GROUP_LEN
                ))
            }
        }
    }
}

/// A refused grant.
pub fn refused(channel: i32, error: impl Into<String>) -> v1::VoiceGrant {
    v1::VoiceGrant {
        channel,
        error: error.into(),
        ..Default::default()
    }
}

/// Issues the grant for `rider` (name `name`) if allowed.
pub fn grant(
    config: &VoiceConfig,
    req: &v1::VoiceTokenRequest,
    rider: RiderId,
    name: &str,
    event: Option<&ocw_sim::Event>,
    now_unix_s: u64,
) -> v1::VoiceGrant {
    let room = match choose_room(req, rider, event) {
        Ok(RoomChoice::Session) => config.session_room(),
        Ok(RoomChoice::Event(id)) => config.event_room(id),
        Ok(RoomChoice::Group(g)) => config.group_room(&g),
        Err(e) => return refused(req.channel, e),
    };
    let identity = ocw_protocol::voice_identity(rider);
    let channel = match req.channel() {
        v1::VoiceChannel::Unspecified => v1::VoiceChannel::Session as i32,
        _ => req.channel,
    };
    v1::VoiceGrant {
        url: config.url.clone(),
        token: config.access_token(&room, &identity, name, now_unix_s),
        room,
        identity,
        expires_unix_s: now_unix_s + config.token_ttl_s,
        channel,
        error: String::new(),
    }
}

// ------------------------------------------------------------- JWT (HS256)

/// A compact JWT with header `{"alg":"HS256","typ":"JWT"}`.
pub fn jwt_hs256(claims: &serde_json::Value, secret: &[u8]) -> String {
    let header = base64url(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = base64url(claims.to_string().as_bytes());
    let signing_input = format!("{header}.{payload}");
    let sig = hmac_sha256(secret, signing_input.as_bytes());
    format!("{signing_input}.{}", base64url(&sig))
}

/// Verifies an HS256 JWT and returns its claims (tests and diagnostics).
pub fn verify_jwt_hs256(token: &str, secret: &[u8]) -> Result<serde_json::Value, String> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("not a compact JWT".into());
    };
    let header: serde_json::Value =
        serde_json::from_slice(&base64url_decode(h)?).map_err(|e| format!("header: {e}"))?;
    if header["alg"] != "HS256" {
        return Err(format!("unexpected algorithm {}", header["alg"]));
    }
    let expected = hmac_sha256(secret, format!("{h}.{p}").as_bytes());
    let sig = base64url_decode(s)?;
    // Constant-time comparison.
    if sig.len() != expected.len()
        || sig.iter().zip(&expected).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0
    {
        return Err("bad signature".into());
    }
    serde_json::from_slice(&base64url_decode(p)?).map_err(|e| format!("claims: {e}"))
}

/// HMAC-SHA256 (RFC 2104) with `sha2`; small enough not to need another
/// dependency.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (RFC 4648 §5), as JWTs use it.
pub fn base64url(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..chunk.len() + 1 {
            out.push(B64URL[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

pub fn base64url_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = B64URL
            .iter()
            .position(|&x| x == c)
            .ok_or_else(|| format!("invalid base64url character {:?}", c as char))?
            as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if text.len() % 4 == 1 {
        return Err("truncated base64url".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn config(now: u64) -> VoiceConfig {
        VoiceConfig {
            instance: "abc123".into(),
            clock: Arc::new(move || now),
            ..VoiceConfig::new(
                "wss://voice.example.org",
                "devkey",
                "devsecret-devsecret-devsecret-00",
            )
        }
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn base64url_roundtrip_and_known_values() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        for n in 0..70u8 {
            let data: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base64url_decode(&base64url(&data)).unwrap(), data);
        }
        assert!(base64url_decode("a").is_err());
        assert!(base64url_decode("ab=c").is_err());
    }

    #[test]
    fn jwt_matches_a_reference_token() {
        // jwt.io's HS256 example (claims serialized in the same key order).
        let claims: serde_json::Value =
            serde_json::from_str(r#"{"sub":"1234567890","name":"John Doe","iat":1516239022}"#)
                .unwrap();
        let t = jwt_hs256(&claims, b"your-256-bit-secret");
        let (signing_input, _) = t.rsplit_once('.').unwrap();
        assert_eq!(
            hex(&hmac_sha256(
                b"your-256-bit-secret",
                signing_input.as_bytes()
            )),
            hex(&base64url_decode(t.rsplit('.').next().unwrap()).unwrap())
        );
        assert_eq!(
            verify_jwt_hs256(&t, b"your-256-bit-secret").unwrap(),
            claims
        );
        assert_eq!(verify_jwt_hs256(&t, b"wrong").unwrap_err(), "bad signature");
    }

    #[test]
    fn access_token_has_livekit_claims() {
        let c = config(1_800_000_000);
        let t = c.access_token("ocw-abc123", "rider-7", "Rider Seven", 1_800_000_000);
        let claims = verify_jwt_hs256(&t, c.api_secret.as_bytes()).unwrap();
        assert_eq!(claims["iss"], "devkey");
        assert_eq!(claims["sub"], "rider-7");
        assert_eq!(claims["name"], "Rider Seven");
        assert_eq!(claims["nbf"], 1_800_000_000u64);
        assert_eq!(claims["exp"], 1_800_000_000u64 + DEFAULT_TOKEN_TTL_S);
        let v = &claims["video"];
        assert_eq!(v["room"], "ocw-abc123");
        assert_eq!(v["roomJoin"], true);
        assert_eq!(v["canPublish"], true);
        assert_eq!(v["canSubscribe"], true);
        assert_eq!(v["canPublishData"], false);
        assert_eq!(v["canPublishSources"], serde_json::json!(["microphone"]));
        // No admin rights.
        assert!(v.get("roomAdmin").is_none() && v.get("roomCreate").is_none());
        // Signed with the secret, not the key.
        assert!(verify_jwt_hs256(&t, b"devkey").is_err());
    }

    #[test]
    fn config_from_parts_and_environment() {
        let none = |_: &str| None;
        assert!(
            VoiceConfig::from_parts(None, None, None, none)
                .unwrap()
                .is_none()
        );
        let env = |k: &str| match k {
            ENV_URL => Some("ws://127.0.0.1:7880".to_owned()),
            ENV_API_KEY => Some("k".to_owned()),
            ENV_API_SECRET => Some("s".to_owned()),
            _ => None,
        };
        let c = VoiceConfig::from_parts(None, None, None, env)
            .unwrap()
            .unwrap();
        assert_eq!(
            (c.url.as_str(), c.api_key.as_str(), c.api_secret.as_str()),
            ("ws://127.0.0.1:7880", "k", "s")
        );
        // Explicit values win over the environment.
        let c = VoiceConfig::from_parts(Some("wss://x".into()), None, None, env)
            .unwrap()
            .unwrap();
        assert_eq!(c.url, "wss://x");
        let e = VoiceConfig::from_parts(Some("wss://x".into()), None, None, none).unwrap_err();
        assert!(
            e.contains(ENV_API_KEY) && e.contains(ENV_API_SECRET) && !e.contains(ENV_URL),
            "{e}"
        );
        assert!(
            VoiceConfig::from_parts(
                Some("ftp://x".into()),
                Some("k".into()),
                Some("s".into()),
                none
            )
            .is_err()
        );
        // The secret never shows in debug output.
        let c = VoiceConfig::new("wss://x", "key", "very-secret");
        assert!(!format!("{c:?}").contains("very-secret"));
    }

    #[test]
    fn instance_ids_differ() {
        let a = new_instance_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_ne!(a, new_instance_id());
        assert_eq!(a.len(), 12);
    }

    fn request(channel: v1::VoiceChannel, group: &str) -> v1::VoiceTokenRequest {
        v1::VoiceTokenRequest {
            channel: channel as i32,
            group: group.into(),
        }
    }

    #[test]
    fn rooms_by_channel() {
        let c = config(1000);
        let r = RiderId(3);
        let ev = ocw_sim::Event::new(
            9,
            ocw_sim::EventKind::GroupRide { target_w: 150.0 },
            0,
            0.0,
            vec![(RiderId(3), "A".into()), (RiderId(4), "B".into())],
        );
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Unspecified, ""),
            r,
            "A",
            None,
            1000,
        );
        assert_eq!(
            (g.room.as_str(), g.channel),
            ("ocw-abc123", v1::VoiceChannel::Session as i32)
        );
        assert_eq!(g.identity, "rider-3");
        assert_eq!(g.url, "wss://voice.example.org");
        assert_eq!(g.expires_unix_s, 1000 + DEFAULT_TOKEN_TTL_S);
        assert!(g.error.is_empty());
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Event, ""),
            r,
            "A",
            Some(&ev),
            1000,
        );
        assert_eq!(g.room, "ocw-abc123-event-9");
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Event, ""),
            RiderId(5),
            "C",
            Some(&ev),
            1000,
        );
        assert!(
            g.token.is_empty() && g.error.contains("not an entrant"),
            "{g:?}"
        );
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Event, ""),
            r,
            "A",
            None,
            1000,
        );
        assert!(g.token.is_empty() && g.error.contains("no event"));
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Group, "team-7"),
            r,
            "A",
            None,
            1000,
        );
        assert_eq!(g.room, "ocw-abc123-group-team-7");
        let claims = verify_jwt_hs256(&g.token, c.api_secret.as_bytes()).unwrap();
        assert_eq!(claims["video"]["room"], "ocw-abc123-group-team-7");
        let g = grant(
            &c,
            &request(v1::VoiceChannel::Group, "Team 7"),
            r,
            "A",
            None,
            1000,
        );
        assert!(g.token.is_empty() && g.error.contains("invalid group name"));
        assert!(g.room.is_empty() && g.url.is_empty());
    }
}
