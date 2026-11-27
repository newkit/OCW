//! The public server (spec 11.5 *The public server as the default*): the
//! game server on Fly.io that every app offers first **for now** (the
//! user, 2026-11-27: "insert the url of the fly.io server as default for
//! all apps for now"). Bevy-free and wasm-safe; no network here – the apps
//! fetch the join info themselves.
//!
//! - [`PUBLIC_JOIN_INFO`] / [`PUBLIC_VOICE`]: the one place the address is
//!   written in the Rust code (the kiosk scripts, the browser page and the
//!   preview site carry the same text; a test keeps them equal).
//! - [`SETTING`] (`net.public_server`): `default` (this constant), `off`
//!   (no public server: the rows and `join public` go away) or another
//!   join info URL (`https://…/join.json`) – changed or cleared without a
//!   rebuild.
//! - [`JoinInfo::parse`]: the server's `/join.json` (untrusted: only
//!   checked values come out), as the kiosk reads it.
//! - [`match_world`]: is the server's world installed here – by its
//!   SHA-256, else by the manifest's id and version, as the kiosk checks.

use std::path::{Path, PathBuf};

/// The public server's join info (spec 11.5 *Join info*).
pub const PUBLIC_JOIN_INFO: &str = "https://ocw-server.fly.dev/join.json";
/// The public server's voice (LiveKit over TLS on 443, spec 11.5); the
/// server's grants name it, so the apps need it only where a page names
/// its voice server (the browser's `config.json`).
pub const PUBLIC_VOICE: &str = "wss://ocw-server.fly.dev";
/// The setting that changes or clears [`PUBLIC_JOIN_INFO`].
pub const SETTING: &str = "net.public_server";
/// The largest join info read (the server's is about 600 bytes).
pub const MAX_JOIN_INFO_BYTES: u64 = 65_536;
/// Seconds to wait for the join info.
pub const FETCH_TIMEOUT_S: u64 = 10;

/// The join info URL `net.public_server` names: `default` (or empty) →
/// [`PUBLIC_JOIN_INFO`], `off` / `none` → none, an `https://` URL → it.
pub fn resolve(setting: &str) -> Result<Option<String>, String> {
    let s = setting.trim();
    match s.to_ascii_lowercase().as_str() {
        "" | "default" => Ok(Some(PUBLIC_JOIN_INFO.to_owned())),
        "off" | "none" => Ok(None),
        _ if valid_url(s) => Ok(Some(s.to_owned())),
        _ => Err(format!(
            "{SETTING}: {s:?} is not default, off or an https:// join info URL"
        )),
    }
}

/// [`resolve`], a bad value counting as `default` (the registry checks it
/// when it is set).
pub fn resolve_or_default(setting: &str) -> Option<String> {
    resolve(setting).unwrap_or_else(|_| Some(PUBLIC_JOIN_INFO.to_owned()))
}

fn valid_url(s: &str) -> bool {
    s.strip_prefix("https://").is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty()
            && rest.len() < 300
            && !s.contains(|c: char| c.is_whitespace() || c == '"' || c.is_control())
    })
}

/// The server's host for people: `ocw-server.fly.dev`.
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// The row of the Join page and the start choice: "Public server
/// (ocw-server.fly.dev)".
pub fn label(url: &str) -> String {
    format!("Public server ({})", host_of(url))
}

/// What a server's `/join.json` says (spec 11.5 *Join info*).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoinInfo {
    /// The link the desktop joins: `join` (`ocw://IPV4?cert=…&world=…`,
    /// QUIC), else the WebTransport URL with the current certificate hash.
    pub link: String,
    /// The world's id (`liechtenstein`), its version and SHA-256; `None`:
    /// a test route (no package needed).
    pub world: Option<String>,
    pub world_version: Option<u64>,
    pub world_sha256: Option<String>,
    /// A manifest URL to download the world from (`world_manifest`, when
    /// the server names one; it names none yet).
    pub world_manifest: Option<String>,
    /// The test route of a server without a world.
    pub route: Option<String>,
    pub voice: Option<String>,
}

impl JoinInfo {
    /// Parses the join info; only values of the expected form are taken
    /// (as the kiosk's reader: no quotes, spaces or control characters).
    pub fn parse(json: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("not JSON ({e})"))?;
        let obj = v.as_object().ok_or("not a join info object")?;
        let text = |k: &str| obj.get(k).and_then(|v| v.as_str()).map(str::trim);
        let linkish = |s: &str| {
            !s.is_empty()
                && s.len() <= 1000
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._~:/?&=%@,+[]-".contains(c))
        };
        let world = text("world").filter(|w| {
            !w.is_empty()
                && w.len() <= 100
                && w.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "/_.-".contains(c))
        });
        let world_version = match obj.get("world_version") {
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            Some(serde_json::Value::String(s)) => s.parse().ok(),
            _ => None,
        };
        let world_sha256 = text("world_sha256")
            .filter(|s| s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        let hex64 =
            |s: &str| s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
        let route = text("route").filter(|r| linkish(r)).map(str::to_owned);
        let mut link = text("join")
            .filter(|l| l.starts_with("ocw://") && l.contains('?') && linkish(l))
            .map(str::to_owned);
        if link.is_none() {
            let web = text("webtransport").filter(|w| {
                w.strip_prefix("https://").is_some_and(|h| {
                    !h.is_empty()
                        && h.chars()
                            .all(|c| c.is_ascii_alphanumeric() || ".:[]-".contains(c))
                })
            });
            let hash = obj
                .get("cert_hashes")
                .and_then(|h| h.as_array())
                .and_then(|h| h.first())
                .and_then(|h| h.as_str())
                .filter(|h| hex64(h));
            if let (Some(web), Some(hash)) = (web, hash) {
                let mut l = format!("{web}?cert-hash={hash}");
                if let Some(w) = world {
                    l.push_str(&format!("&world={w}"));
                    if let Some(v) = world_version {
                        l.push_str(&format!("@{v}"));
                    }
                }
                if let Some(r) = &route {
                    l.push_str(&format!("&route={r}"));
                }
                link = Some(l);
            }
        }
        let link = link.ok_or(
            "no join link in it (neither join nor webtransport with cert_hashes)".to_string(),
        )?;
        Ok(Self {
            link,
            world: world.map(str::to_owned),
            world_version,
            world_sha256: world_sha256.map(str::to_owned),
            world_manifest: text("world_manifest")
                .filter(|u| valid_url(u))
                .map(str::to_owned),
            route,
            voice: text("voice")
                .filter(|v| (v.starts_with("wss://") || v.starts_with("ws://")) && linkish(v))
                .map(str::to_owned),
        })
    }

    /// `liechtenstein@10` (or the id alone).
    pub fn world_spec(&self) -> Option<String> {
        let w = self.world.as_ref()?;
        Some(match self.world_version {
            Some(v) => format!("{w}@{v}"),
            None => w.clone(),
        })
    }
}

/// An installed world package (from the app's world list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// What `--world` takes (a path or `id@version`).
    pub spec: String,
    pub id: String,
    pub version: u64,
    pub path: Option<PathBuf>,
}

/// Whether the server's world is here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldMatch {
    /// The server rides a test route: nothing to install.
    NotNeeded,
    /// This package is the server's.
    Installed(String),
    /// Not installed (or another build): why, and where it could be
    /// downloaded from (the join info's manifest URL), if anywhere.
    Missing {
        why: String,
        download: Option<String>,
    },
}

/// The last part of a manifest id (`europe/liechtenstein` →
/// `liechtenstein`): the join info names the short id.
fn same_id(installed: &str, wanted: &str) -> bool {
    installed == wanted || installed.rsplit('/').next() == Some(wanted)
}

/// Finds the server's world among `installed` like the kiosk does: a
/// package of the same id whose SHA-256 (`sha_of`, the app hashes the
/// file) is the server's; without a SHA-256 in the join info the same id
/// and version. A package of that id and version with another SHA-256 is
/// another build (its road graph would differ).
pub fn match_world(
    info: &JoinInfo,
    installed: &[Installed],
    sha_of: &mut dyn FnMut(&Path) -> Option<String>,
) -> WorldMatch {
    let Some(id) = &info.world else {
        return WorldMatch::NotNeeded;
    };
    let spec = info.world_spec().unwrap_or_else(|| id.clone());
    let same: Vec<&Installed> = installed.iter().filter(|i| same_id(&i.id, id)).collect();
    let version_ok = |i: &Installed| info.world_version.is_none_or(|v| i.version == v);
    let mut other_build = false;
    if let Some(sha) = &info.world_sha256 {
        for i in &same {
            if let Some(p) = &i.path
                && let Some(got) = sha_of(p)
            {
                if &got == sha {
                    return WorldMatch::Installed(i.spec.clone());
                }
                other_build |= version_ok(i);
            }
        }
    } else if let Some(i) = same.iter().find(|i| version_ok(i)) {
        return WorldMatch::Installed(i.spec.clone());
    }
    let have: Vec<String> = same
        .iter()
        .map(|i| format!("{}@{}", i.id, i.version))
        .collect();
    let why = if other_build {
        format!(
            "the public server rides {spec}, and the package of that version here is another build (its SHA-256 differs)"
        )
    } else if have.is_empty() {
        format!("the public server rides the world {spec}, which is not installed here")
    } else {
        format!(
            "the public server rides {spec}; installed here: {}",
            have.join(", ")
        )
    };
    WorldMatch::Missing {
        why,
        download: info.world_manifest.clone(),
    }
}

/// What to do when the world is missing and there is nowhere to download
/// it from (the public server names no download source yet).
pub fn install_hint(spec: &str) -> String {
    format!(
        "install the package {spec} (it comes with the release zips and the browser preview's worlds; World -> Change world lists those found, or ocw-desktop --fetch-world MANIFEST-URL) and join again"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0ae1fb51a5eacb8fb1beb577b107b5fa5f7b177f89ccd4cae299c8e23a86ff77";
    const SHA: &str = "e5b3f612aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaafbf8";

    fn fly_info() -> String {
        format!(
            r#"{{"webtransport": "https://203.0.113.7:47801", "cert_hashes": ["{FP}"],
                "voice": "wss://ocw-server.fly.dev", "world": "liechtenstein",
                "world_version": 10, "world_sha256": "{SHA}",
                "join": "ocw://203.0.113.7?cert={FP}&world=liechtenstein@10"}}"#
        )
    }

    /// The default, a changed URL and the setting cleared.
    #[test]
    fn the_setting_resolves_to_the_public_server_or_none() {
        assert_eq!(
            resolve("default").unwrap().as_deref(),
            Some(PUBLIC_JOIN_INFO)
        );
        assert_eq!(resolve("").unwrap().as_deref(), Some(PUBLIC_JOIN_INFO));
        assert_eq!(resolve(" Off ").unwrap(), None);
        assert_eq!(resolve("none").unwrap(), None);
        assert_eq!(
            resolve("https://ride.example.org/join.json")
                .unwrap()
                .as_deref(),
            Some("https://ride.example.org/join.json")
        );
        for bad in [
            "http://ride.example.org/join.json",
            "ride.example.org",
            "https://",
            "https://a b",
        ] {
            assert!(resolve(bad).is_err(), "{bad}");
            assert_eq!(resolve_or_default(bad).as_deref(), Some(PUBLIC_JOIN_INFO));
        }
        assert_eq!(host_of(PUBLIC_JOIN_INFO), "ocw-server.fly.dev");
        assert_eq!(
            label(PUBLIC_JOIN_INFO),
            "Public server (ocw-server.fly.dev)"
        );
        assert_eq!(host_of(PUBLIC_VOICE), "ocw-server.fly.dev");
    }

    /// The registry's default is the built-in server; the setting's
    /// values are checked when they are set.
    #[test]
    fn the_registry_has_the_setting() {
        let d = crate::settings::def(SETTING).expect("registered");
        assert_eq!(d.default, "default");
        assert_eq!(
            resolve(d.default).unwrap().as_deref(),
            Some(PUBLIC_JOIN_INFO)
        );
        assert!(d.parse("off").is_ok());
        assert!(d.parse("https://ride.example.org/join.json").is_ok());
        assert!(d.parse("ride.example.org").is_err());
        // The browser takes its server from the page (config.json).
        assert!(!crate::settings::applies(
            SETTING,
            crate::menu::model::Platform::Browser
        ));
    }

    #[test]
    fn the_join_info_gives_the_desktop_link_and_the_world() {
        let i = JoinInfo::parse(&fly_info()).unwrap();
        assert_eq!(
            i.link,
            format!("ocw://203.0.113.7?cert={FP}&world=liechtenstein@10")
        );
        assert_eq!(i.world_spec().as_deref(), Some("liechtenstein@10"));
        assert_eq!(i.world_sha256.as_deref(), Some(SHA));
        assert_eq!(i.voice.as_deref(), Some(PUBLIC_VOICE));
        assert_eq!(i.world_manifest, None);
        assert!(ocw_protocol::join::JoinLink::parse(&i.link).is_ok());
        // Without `join`: the WebTransport URL with the hash.
        let old = fly_info().replace("\"join\"", "\"old_join\"");
        let i = JoinInfo::parse(&old).unwrap();
        assert_eq!(
            i.link,
            format!("https://203.0.113.7:47801?cert-hash={FP}&world=liechtenstein@10")
        );
        // A test route.
        let i = JoinInfo::parse(&format!(
            r#"{{"webtransport": "https://203.0.113.7:47801", "cert_hashes": ["{FP}"], "route": "loop"}}"#
        ))
        .unwrap();
        assert_eq!(i.world, None);
        assert!(i.link.ends_with("&route=loop"), "{}", i.link);
        // Refused: not JSON, no link, a link with quotes or spaces.
        for bad in [
            "[1]".to_string(),
            "{".into(),
            r#"{"world": "liechtenstein"}"#.into(),
            format!(r#"{{"join": "ocw://h?cert={FP}\" --exec x"}}"#),
            format!(r#"{{"webtransport": "https://h:1/x", "cert_hashes": ["{FP}"]}}"#),
        ] {
            assert!(JoinInfo::parse(&bad).is_err(), "{bad}");
        }
        // A bad world id is dropped, not used.
        let i = JoinInfo::parse(&fly_info().replace("\"liechtenstein\"", "\"../x y\"")).unwrap();
        assert_eq!(i.world, None);
    }

    fn pkg(id: &str, version: u64, path: &str) -> Installed {
        Installed {
            spec: format!("{id}@{version}"),
            id: id.into(),
            version,
            path: Some(PathBuf::from(path)),
        }
    }

    #[test]
    fn the_world_is_found_by_sha256_else_by_id_and_version() {
        let info = JoinInfo::parse(&fly_info()).unwrap();
        let installed = [
            pkg("europe/liechtenstein", 9, "/w/old.ocwpkg"),
            pkg("europe/liechtenstein", 10, "/w/li.ocwpkg"),
            pkg("blackforest", 3, "/w/bf.ocwpkg"),
        ];
        let mut hashed = Vec::new();
        let mut sha = |p: &Path| {
            hashed.push(p.to_path_buf());
            Some(if p.ends_with("li.ocwpkg") {
                SHA.to_string()
            } else {
                "0".repeat(64)
            })
        };
        assert_eq!(
            match_world(&info, &installed, &mut sha),
            WorldMatch::Installed("europe/liechtenstein@10".into())
        );
        // Only packages of the server's id are hashed.
        assert!(
            !hashed.iter().any(|p| p.ends_with("bf.ocwpkg")),
            "{hashed:?}"
        );
        // Another build of the same version.
        let m = match_world(&info, &installed, &mut |_| Some("1".repeat(64)));
        let WorldMatch::Missing {
            why,
            download: None,
        } = m
        else {
            panic!("{m:?}")
        };
        assert!(why.contains("another build"), "{why}");
        // Not installed at all: said so; no download source.
        let m = match_world(&info, &installed[2..], &mut |_| None);
        assert_eq!(
            m,
            WorldMatch::Missing {
                why: "the public server rides the world liechtenstein@10, which is not installed here".into(),
                download: None
            }
        );
        // Without a SHA-256: id and version.
        let mut no_sha = info.clone();
        no_sha.world_sha256 = None;
        assert_eq!(
            match_world(&no_sha, &installed, &mut |_| panic!("no hashing")),
            WorldMatch::Installed("europe/liechtenstein@10".into())
        );
        let m = match_world(&no_sha, &installed[..1], &mut |_| None);
        let WorldMatch::Missing { why, .. } = m else {
            panic!()
        };
        assert!(
            why.contains("installed here: europe/liechtenstein@9"),
            "{why}"
        );
        // A download source named by the server.
        let mut src = info.clone();
        src.world_manifest =
            Some("https://worlds.example.org/liechtenstein/10/manifest.json".into());
        let m = match_world(&src, &[], &mut |_| None);
        assert!(
            matches!(
                m,
                WorldMatch::Missing {
                    download: Some(_),
                    ..
                }
            ),
            "{m:?}"
        );
        // A test route needs nothing.
        let mut route = info;
        route.world = None;
        assert_eq!(
            match_world(&route, &[], &mut |_| None),
            WorldMatch::NotNeeded
        );
        assert!(install_hint("liechtenstein@10").contains("liechtenstein@10"));
    }

    /// The scripts and pages that cannot use the constant carry the same
    /// text (the kiosks, the browser page's fallback, the preview site).
    #[test]
    fn every_copy_of_the_address_is_the_same() {
        for (file, text) in [
            (
                "scripts/ocw-kiosk.sh",
                include_str!("../../../scripts/ocw-kiosk.sh"),
            ),
            (
                "scripts/ocw-kiosk.ps1",
                include_str!("../../../scripts/ocw-kiosk.ps1"),
            ),
            (
                "scripts/kiosk.env",
                include_str!("../../../scripts/kiosk.env"),
            ),
            (
                "apps/web/web/index.html",
                include_str!("../../../apps/web/web/index.html"),
            ),
            (
                "scripts/web-preview-site.py",
                include_str!("../../../scripts/web-preview-site.py"),
            ),
        ] {
            assert!(text.contains(PUBLIC_JOIN_INFO), "{file}");
        }
        let page = include_str!("../../../apps/web/web/index.html");
        assert!(
            page.contains(&format!(
                "const OCW_PUBLIC_JOIN_INFO = \"{PUBLIC_JOIN_INFO}\";"
            )),
            "index.html's fallback"
        );
        assert!(include_str!("../../../scripts/web-preview-site.py").contains(PUBLIC_VOICE));
    }
}
