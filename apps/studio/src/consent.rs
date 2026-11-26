//! Asking before the via-points go to a BRouter server (spec 14 Phase 16,
//! 16D; 16H Q21 (a); privacy, 12): the answer is asked once per server and
//! remembered in the planner settings (`<config dir>/ocw/studio/planner.json`,
//! `route.planner.brouter_consent`), keyed by the server – scheme, host and
//! port – so another path on the same server needs no new answer, and a
//! changed server is asked again. A "send" is always remembered; a "don't
//! send" only when the user ticks *remember*, otherwise it holds for this
//! session and the next one asks again. *Forget* (in the planner settings,
//! or `studio planner forget [SERVER|all]`) drops a remembered answer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The answer for one server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Answer {
    /// The via-points may be sent to it.
    Send,
    /// They may not.
    DontSend,
}

/// The answers: the remembered ones (saved with the planner settings) and
/// this session's unremembered "don't send"s.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConsentBook {
    /// By server key (`scheme://host:port`).
    remembered: BTreeMap<String, Answer>,
    /// This session only (not saved).
    #[serde(skip)]
    session: BTreeMap<String, Answer>,
}

/// The key of a BRouter server: `scheme://host:port` with the scheme and
/// host in lower case and the scheme's port filled in (http 80, https 443);
/// user info, path, query and fragment left out. `None` for what is not an
/// http(s) URL with a host.
pub fn server_key(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(inner) = host_port.strip_prefix('[') {
        // [IPv6]:port
        let (h, after) = inner.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return None,
        };
        (format!("[{}]", h.to_ascii_lowercase()), port)
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => (h.to_ascii_lowercase(), Some(p)),
            None => (host_port.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() || host == "[]" {
        return None;
    }
    let port: u16 = match port {
        None | Some("") => default_port,
        Some(p) => p.parse().ok()?,
    };
    Some(format!("{scheme}://{host}:{port}"))
}

impl ConsentBook {
    /// The answer for the server of `url`: this session's, else the
    /// remembered one; `None`: not asked yet (ask before sending).
    pub fn answer(&self, url: &str) -> Option<Answer> {
        let key = server_key(url)?;
        self.session
            .get(&key)
            .or_else(|| self.remembered.get(&key))
            .copied()
    }

    /// Whether the via-points may go to `url`'s server.
    pub fn may_send(&self, url: &str) -> bool {
        self.answer(url) == Some(Answer::Send)
    }

    /// Whether the answer for `url`'s server is remembered (saved).
    pub fn is_remembered(&self, url: &str) -> bool {
        server_key(url).is_some_and(|k| self.remembered.contains_key(&k))
    }

    /// Records the user's answer for `url`'s server: "send" is always
    /// remembered; "don't send" only with `remember`, otherwise it holds
    /// for this session (and a remembered "send" is dropped, so the next
    /// session asks again). Returns whether the saved answers changed.
    pub fn decide(&mut self, url: &str, answer: Answer, remember: bool) -> Result<bool, String> {
        let key = server_key(url).ok_or("the BRouter server is an http:// or https:// URL")?;
        let before = self.remembered.clone();
        self.session.remove(&key);
        match (answer, remember) {
            (Answer::Send, _) | (Answer::DontSend, true) => {
                self.remembered.insert(key, answer);
            }
            (Answer::DontSend, false) => {
                self.remembered.remove(&key);
                self.session.insert(key, Answer::DontSend);
            }
        }
        Ok(self.remembered != before)
    }

    /// Forgets the answer for `url`'s server (remembered and this
    /// session's): it is asked again. Whether there was one.
    pub fn forget(&mut self, url: &str) -> bool {
        let Some(key) = server_key(url) else {
            return false;
        };
        let a = self.remembered.remove(&key).is_some();
        let b = self.session.remove(&key).is_some();
        a || b
    }

    /// Forgets every answer; how many servers had one.
    pub fn forget_all(&mut self) -> usize {
        let mut keys: Vec<&String> = self.remembered.keys().collect();
        keys.extend(self.session.keys());
        keys.sort();
        keys.dedup();
        let n = keys.len();
        self.remembered.clear();
        self.session.clear();
        n
    }

    /// The remembered answers, by server key.
    pub fn remembered(&self) -> impl Iterator<Item = (&str, Answer)> {
        self.remembered.iter().map(|(k, a)| (k.as_str(), *a))
    }

    /// A new session: the unremembered answers are gone (for tests; the
    /// app starts with a fresh book read from the file).
    #[cfg(test)]
    pub fn next_session(&self) -> Self {
        Self {
            remembered: self.remembered.clone(),
            session: BTreeMap::new(),
        }
    }
}

/// The answer in words (the settings list, `studio planner`).
pub fn answer_text(a: Answer) -> &'static str {
    match a {
        Answer::Send => "send the via-points",
        Answer::DontSend => "don't send them",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "http://brouter.example:17777";

    #[test]
    fn the_server_key_is_scheme_host_and_port() {
        let k = |u| server_key(u);
        assert_eq!(
            k("http://BRouter.Example:17777/brouter?x=1").as_deref(),
            Some("http://brouter.example:17777")
        );
        assert_eq!(
            k("https://plan.example").as_deref(),
            Some("https://plan.example:443")
        );
        assert_eq!(
            k("http://plan.example/").as_deref(),
            Some("http://plan.example:80")
        );
        assert_eq!(
            k("HTTP://ada:secret@plan.example:8080#x").as_deref(),
            Some("http://plan.example:8080")
        );
        assert_eq!(
            k("http://[::1]:17777/").as_deref(),
            Some("http://[::1]:17777")
        );
        assert_eq!(k("http://[::1]").as_deref(), Some("http://[::1]:80"));
        assert_eq!(
            k("http://127.0.0.1:17777").as_deref(),
            Some("http://127.0.0.1:17777")
        );
        // The default port and the written one are the same server.
        assert_eq!(
            k("https://plan.example:443"),
            k("https://plan.example/brouter")
        );
        for bad in [
            "",
            "plan.example",
            "ftp://plan.example",
            "http://",
            "http://:80",
            "http://h:port",
        ] {
            assert_eq!(k(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_send_is_remembered_per_server_and_survives_the_session() {
        let mut b = ConsentBook::default();
        assert_eq!(b.answer(HOME), None, "not asked yet");
        assert!(b.decide(HOME, Answer::Send, false).unwrap(), "saved");
        assert!(b.may_send(HOME));
        assert!(b.is_remembered(HOME));
        // Another path on the same server: the same answer.
        assert!(b.may_send("http://brouter.example:17777/brouter"));
        // The next session (read back from the file) still has it.
        let text = serde_json::to_string(&b).unwrap();
        let back: ConsentBook = serde_json::from_str(&text).unwrap();
        assert!(back.may_send(HOME));
        assert!(b.next_session().may_send(HOME));
    }

    #[test]
    fn a_changed_server_is_asked_again() {
        let mut b = ConsentBook::default();
        b.decide(HOME, Answer::Send, false).unwrap();
        for other in [
            "http://brouter.example:17778",
            "https://brouter.example:17777",
            "http://other.example:17777",
        ] {
            assert_eq!(b.answer(other), None, "{other}");
            assert!(!b.may_send(other));
        }
    }

    #[test]
    fn a_decline_without_remember_lasts_one_session() {
        let mut b = ConsentBook::default();
        assert!(
            !b.decide(HOME, Answer::DontSend, false).unwrap(),
            "nothing saved"
        );
        assert_eq!(b.answer(HOME), Some(Answer::DontSend));
        assert!(!b.is_remembered(HOME));
        assert_eq!(serde_json::to_string(&b).unwrap(), "{}");
        // The next session asks again.
        assert_eq!(b.next_session().answer(HOME), None);
        // Declining a remembered "send" without remember: dropped, so the
        // next session asks again too.
        b.decide(HOME, Answer::Send, false).unwrap();
        assert!(b.decide(HOME, Answer::DontSend, false).unwrap());
        assert!(!b.may_send(HOME));
        assert_eq!(b.next_session().answer(HOME), None);
    }

    #[test]
    fn a_decline_with_remember_is_kept() {
        let mut b = ConsentBook::default();
        assert!(b.decide(HOME, Answer::DontSend, true).unwrap());
        let back: ConsentBook = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(back.answer(HOME), Some(Answer::DontSend));
        assert!(back.is_remembered(HOME));
        // Changing one's mind: a send replaces it.
        b.decide(HOME, Answer::Send, false).unwrap();
        assert!(b.next_session().may_send(HOME));
    }

    #[test]
    fn forget_drops_the_answer_of_one_server_or_all() {
        let mut b = ConsentBook::default();
        b.decide(HOME, Answer::Send, false).unwrap();
        b.decide("https://plan.example", Answer::DontSend, true)
            .unwrap();
        b.decide("http://third.example", Answer::DontSend, false)
            .unwrap();
        assert!(b.forget("http://BROUTER.example:17777/brouter"));
        assert_eq!(b.answer(HOME), None, "asked again");
        assert!(!b.forget(HOME), "nothing left to forget");
        assert!(!b.forget("not a url"));
        assert_eq!(b.remembered().count(), 1);
        assert_eq!(b.forget_all(), 2);
        assert_eq!(b.remembered().count(), 0);
        assert_eq!(b.answer("http://third.example"), None);
    }

    #[test]
    fn the_file_holds_the_answers_by_server() {
        let mut b = ConsentBook::default();
        b.decide("http://brouter.example:17777/x", Answer::Send, false)
            .unwrap();
        b.decide("https://plan.example", Answer::DontSend, true)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&b).unwrap(),
            serde_json::json!({
                "http://brouter.example:17777": "send",
                "https://plan.example:443": "dont-send",
            })
        );
        assert!(
            ConsentBook::default()
                .decide("brouter.example", Answer::Send, false)
                .is_err()
        );
    }
}
