//! The page's configuration (spec §14 Phase 10 B8a): `config.json` next to
//! `index.html`, read by the page before the module starts and handed to
//! the app as `window.ocwConfig` (its text). It replaces the localhost
//! assumptions of the dev server for a hosted page:
//!
//! ```json
//! { "channel": "preview",
//!   "worlds": "worlds/",
//!   "server": "", "voice": "", "signin": "",
//!   "start": [ { "title": "…", "query": "route=hills" } ] }
//! ```
//!
//! - `worlds`: where worlds named by id are found (`world-builder publish`
//!   layout; a path relative to the page or an `https://` URL; default
//!   `worlds/`).
//! - `server`, `voice`, `signin`: the hosted game server (WebTransport),
//!   voice (LiveKit) and sign-in (§9.1). A key left out changes nothing (the
//!   dev server's pages); an **empty** value says this page has none: on a
//!   public page multiplayer (`?server=`) and voice are then off, with the
//!   note [`HOSTED_NOTE`] (the stage-1 preview on GitHub Pages).
//!   A `server` ending in `.json` is the public server's **join info**
//!   (spec 11.5 *Join info*, `https://ocw-server.fly.dev/join.json`):
//!   index.html fetches it for `?together=1` before the module starts and
//!   puts the server's current WebTransport URL, certificate hashes and
//!   world into the query (`server=`, `cert-hash=`, `world=`), which the
//!   app reads as from a join link ([`PageConfig::join_info`]). A page
//!   whose configuration has no `server` key takes the built-in public
//!   server, [`ocw_client::public::PUBLIC_JOIN_INFO`] (for now the one on
//!   Fly.io, spec 11.5 *The public server as the default*); `""` is none.
//! - `channel` (`dev`, `preview`) and `start` (the start page's rides) are
//!   the page's own (index.html); the app ignores them and unknown keys.
//!
//! Development pages (localhost, `crate::is_dev_host`) keep every option:
//! the page configuration only ever turns things off on public pages.

use ocw_client::public::PUBLIC_JOIN_INFO;
use serde_json::Value;

/// What a public page without a hosted server says about multiplayer,
/// voice and sign-in.
pub const HOSTED_NOTE: &str = "coming in the hosted version";

/// The page's configuration (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageConfig {
    /// `dev` (`make web`), `preview` (`make web-preview`), or empty.
    pub channel: String,
    /// The worlds' base (ends in `/`), `None`: the default
    /// (`crate::world::WORLDS_BASE`).
    pub worlds: Option<String>,
    /// The hosted game server, voice and sign-in; `Some("")`: none.
    pub server: Option<String>,
    pub voice: Option<String>,
    pub signin: Option<String>,
}

impl PageConfig {
    /// Parses `config.json`; an empty text is the empty configuration
    /// (a page without the file).
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        let v: Value =
            serde_json::from_str(json).map_err(|e| format!("config.json: not JSON ({e})"))?;
        let obj = v
            .as_object()
            .ok_or("config.json: not a JSON object".to_string())?;
        let text = |key: &str| -> Result<Option<String>, String> {
            match obj.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
                Some(other) => Err(format!("config.json: {key} must be a string, not {other}")),
            }
        };
        let worlds = match text("worlds")? {
            Some(w) if w.is_empty() => None,
            Some(w) => Some(worlds_base(&w)?),
            None => None,
        };
        let url = |key: &str| -> Result<Option<String>, String> {
            match text(key)? {
                Some(u)
                    if !u.is_empty() && !u.starts_with("https://") && !u.starts_with("wss://") =>
                {
                    Err(format!(
                        "config.json: {key} must be an https:// or wss:// URL (or empty)"
                    ))
                }
                other => Ok(other),
            }
        };
        Ok(Self {
            channel: text("channel")?.unwrap_or_default(),
            worlds,
            server: url("server")?,
            voice: url("voice")?,
            signin: url("signin")?,
        })
    }

    /// Whether this is the public preview's page (`channel: preview`).
    pub fn is_preview(&self) -> bool {
        self.channel == "preview"
    }

    /// The public server's join info URL, which index.html reads for
    /// `?together=1`: `server` ending in `.json`; without a `server` key the
    /// built-in [`PUBLIC_JOIN_INFO`]; none for `""` or a plain server URL.
    pub fn join_info(&self) -> Option<&str> {
        match self.server.as_deref() {
            None => Some(PUBLIC_JOIN_INFO),
            Some(s) => Some(s).filter(|s| s.starts_with("https://") && s.ends_with(".json")),
        }
    }

    /// Why multiplayer is off on this page (`None`: it is not): a public
    /// page whose configuration names no game server.
    pub fn multiplayer_off(&self, dev_page: bool) -> Option<String> {
        off(&self.server, dev_page).then(|| format!("multiplayer is {HOSTED_NOTE}"))
    }

    /// Why voice is off on this page (`None`: it is not).
    pub fn voice_off(&self, dev_page: bool) -> Option<String> {
        off(&self.voice, dev_page).then(|| format!("voice chat is {HOSTED_NOTE}"))
    }

    /// Why sign-in is off on this page (`None`: it is not; there is no
    /// sign-in in the browser yet, spec §9.1).
    pub fn signin_off(&self, dev_page: bool) -> Option<String> {
        off(&self.signin, dev_page).then(|| format!("sign-in is {HOSTED_NOTE}"))
    }

    /// Applies the configuration to the page's options: the worlds' base,
    /// and on a public page without a server no `?server=` (with a
    /// warning); the voice note for the app.
    pub fn apply(
        &self,
        config: &mut crate::SessionConfig,
        dev_page: bool,
        warnings: &mut Vec<String>,
    ) {
        if let (Some(base), Some(w)) = (&self.worlds, config.world.as_mut()) {
            w.base = base.clone();
        }
        if let Some(why) = self.multiplayer_off(dev_page)
            && let Some(r) = config.remote.take()
        {
            warnings.push(format!("server={}: ignored ({why})", r.url));
        }
        config.voice_off = self.voice_off(dev_page);
    }
}

/// `Some("")` on a public page.
fn off(value: &Option<String>, dev_page: bool) -> bool {
    !dev_page && value.as_deref().is_some_and(str::is_empty)
}

/// A worlds base: a relative path (no scheme, no `//`) or an `https://`
/// URL, ending in `/`.
fn worlds_base(w: &str) -> Result<String, String> {
    let ok = if let Some(rest) = w.strip_prefix("https://") {
        !rest.is_empty()
    } else {
        !w.contains("://") && !w.starts_with("//") && !w.contains(':')
    };
    if !ok {
        return Err(format!(
            "config.json: worlds {w:?} must be a path relative to the page or an https:// URL"
        ));
    }
    Ok(if w.ends_with('/') {
        w.to_string()
    } else {
        format!("{w}/")
    })
}

/// The page's `window.ocwConfig` (set by index.html from `config.json`):
/// the configuration and what was wrong with it (an unreadable file is
/// the empty configuration, with a warning).
#[cfg(target_arch = "wasm32")]
pub fn from_page() -> (PageConfig, Option<String>) {
    let text = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"ocwConfig".into()).ok())
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    match PageConfig::parse(&text) {
        Ok(c) => (c, None),
        Err(e) => (PageConfig::default(), Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREVIEW: &str = r#"{
        "channel": "preview",
        "worlds": "worlds",
        "server": "", "voice": "", "signin": "",
        "start": [{"title": "Hills", "query": "route=hills"}],
        "comment": "unknown keys are the page's"
    }"#;

    #[test]
    fn parses_the_preview_config() {
        let c = PageConfig::parse(PREVIEW).unwrap();
        assert!(c.is_preview());
        assert_eq!(c.worlds.as_deref(), Some("worlds/"));
        assert_eq!(c.server.as_deref(), Some(""));
        // A public page: all three off, with the note.
        assert_eq!(
            c.multiplayer_off(false).as_deref(),
            Some("multiplayer is coming in the hosted version")
        );
        assert!(c.voice_off(false).unwrap().contains(HOSTED_NOTE));
        assert!(c.signin_off(false).unwrap().contains(HOSTED_NOTE));
        // On localhost nothing is off.
        assert!(c.multiplayer_off(true).is_none() && c.voice_off(true).is_none());
    }

    #[test]
    fn missing_keys_change_nothing() {
        for text in ["", "  ", "{}", r#"{"channel": "dev", "worlds": ""}"#] {
            let c = PageConfig::parse(text).unwrap();
            assert!(!c.is_preview());
            assert!(c.worlds.is_none(), "{text}");
            assert!(c.multiplayer_off(false).is_none() && c.voice_off(false).is_none());
            // No server named: the built-in public server (spec 11.5).
            assert_eq!(c.join_info(), Some(PUBLIC_JOIN_INFO), "{text}");
        }
        // `"server": ""`: none.
        assert_eq!(PageConfig::parse(PREVIEW).unwrap().join_info(), None);
        // A hosted server named: multiplayer stays on.
        let c = PageConfig::parse(
            r#"{"server": "https://ocw.example:47801", "voice": "wss://lk.example"}"#,
        )
        .unwrap();
        assert!(c.multiplayer_off(false).is_none() && c.voice_off(false).is_none());
        assert_eq!(
            PageConfig::parse(r#"{"worlds": "https://cdn.example/w"}"#)
                .unwrap()
                .worlds
                .as_deref(),
            Some("https://cdn.example/w/")
        );
    }

    /// The preview with the public server (spec 11.5 *Join info*):
    /// multiplayer and voice on, sign-in still off; the join info named.
    #[test]
    fn the_public_server_turns_multiplayer_and_voice_on() {
        let c = PageConfig::parse(
            r#"{"channel": "preview", "worlds": "worlds/",
                "server": "https://ocw-server.fly.dev/join.json",
                "voice": "wss://ocw-server.fly.dev", "signin": ""}"#,
        )
        .unwrap();
        assert_eq!(c.join_info(), Some("https://ocw-server.fly.dev/join.json"));
        assert!(c.multiplayer_off(false).is_none() && c.voice_off(false).is_none());
        assert!(c.signin_off(false).is_some());
        // The query index.html makes from it is the app's usual one.
        let q = format!(
            "together=1&server=https%3A%2F%2F203.0.113.7%3A47801&cert-hash={}%2C{}&world=liechtenstein&name=Ada",
            "ab".repeat(32),
            "cd".repeat(32)
        );
        let (mut s, mut w) = crate::parse_query(&q, false);
        c.apply(&mut s, false, &mut w);
        assert!(w.is_empty(), "{w:?}");
        let r = s.remote.expect("the server stays");
        assert_eq!(r.url, "https://203.0.113.7:47801/ocw/1");
        assert_eq!(r.cert_hashes.len(), 2);
        assert!(s.voice_off.is_none());
        // A plain server URL is no join info.
        assert!(
            PageConfig::parse(r#"{"server": "https://ocw.example:47801"}"#)
                .unwrap()
                .join_info()
                .is_none()
        );
    }

    #[test]
    fn bad_configs_are_refused() {
        for text in [
            "[1]",
            "{",
            r#"{"worlds": 3}"#,
            r#"{"worlds": "http://plain.example/w/"}"#,
            r#"{"worlds": "//cdn.example/w/"}"#,
            r#"{"worlds": "javascript:alert(1)"}"#,
            r#"{"server": "http://x"}"#,
            r#"{"voice": "ws://x"}"#,
            r#"{"channel": false}"#,
        ] {
            assert!(PageConfig::parse(text).is_err(), "{text}");
        }
    }

    /// The preview's page: its worlds' base, no `?server=` on a public
    /// page, the voice note; localhost keeps the server.
    #[test]
    fn applies_to_the_page_options() {
        let c = PageConfig::parse(PREVIEW).unwrap();
        let q = "world=liechtenstein&server=https://127.0.0.1:47801";
        let (mut s, mut w) = crate::parse_query(q, false);
        assert!(w.is_empty(), "{w:?}");
        c.apply(&mut s, false, &mut w);
        assert!(s.remote.is_none());
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("multiplayer is coming in the hosted version"));
        assert_eq!(s.world.as_ref().unwrap().base, "worlds/");
        assert!(s.voice_off.as_deref().unwrap().contains("voice chat"));
        let (mut s, mut w) = crate::parse_query(q, true);
        c.apply(&mut s, true, &mut w);
        assert!(s.remote.is_some() && w.is_empty() && s.voice_off.is_none());
    }
}
