//! The browser's autoplay rule (spec §14 Phase 10 B5): a page may not
//! start sound before the user has interacted with it. An `AudioContext`
//! made earlier starts *suspended*; `resume()` works from a user gesture
//! (click, tap, key). Safari also suspends ("interrupted") a running
//! context – a call, another app's audio – and wants a gesture again.
//!
//! [`AudioGate`] is that rule as a state machine: the app tells it the
//! context's state and every gesture; it says when to call `resume()` (in
//! the gesture's handler: only there it counts everywhere), whether the
//! mixer plays, and the hint the page shows while it waits. Pure; the
//! browser side only calls it.

/// `AudioContext.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextState {
    Suspended,
    Running,
    Closed,
    /// Safari: suspended by the system.
    Interrupted,
}

impl ContextState {
    /// From the `state` string (unknown words count as suspended).
    pub fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "closed" => Self::Closed,
            "interrupted" => Self::Interrupted,
            _ => Self::Suspended,
        }
    }
}

/// Where the sound is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unlock {
    /// No Web Audio here (or the context could not be made); why.
    Unavailable(String),
    /// Waiting for a gesture.
    Locked,
    /// `resume()` was called; waiting for the context.
    Resuming,
    /// Playing.
    Running,
    /// The context was closed (the page is going away).
    Closed,
}

/// The hint while the sound waits for a gesture.
pub const HINT: &str = "Sound: click or press a key to start it";

/// See the module docs.
#[derive(Debug, Clone)]
pub struct AudioGate {
    state: Unlock,
    gestures: u32,
    resumes: u32,
    last_error: Option<String>,
}

impl AudioGate {
    /// A gate for a context that could be made (`Ok`) or not.
    pub fn new(context: Result<ContextState, String>) -> Self {
        let mut g = Self {
            state: Unlock::Locked,
            gestures: 0,
            resumes: 0,
            last_error: None,
        };
        match context {
            Ok(s) => g.on_state(s),
            Err(e) => g.state = Unlock::Unavailable(e),
        }
        g
    }

    pub fn state(&self) -> &Unlock {
        &self.state
    }

    /// The context reported its state (`statechange`, or read each frame).
    pub fn on_state(&mut self, s: ContextState) {
        self.state = match (&self.state, s) {
            (Unlock::Unavailable(_), _) | (Unlock::Closed, _) => return,
            (_, ContextState::Running) => Unlock::Running,
            (_, ContextState::Closed) => Unlock::Closed,
            // Still waiting for the resume to take effect.
            (Unlock::Resuming, ContextState::Suspended) => Unlock::Resuming,
            // Suspended or interrupted by the browser: a gesture again.
            (_, ContextState::Suspended | ContextState::Interrupted) => Unlock::Locked,
        };
    }

    /// A user gesture happened: whether to call `resume()` now (in the
    /// handler). Every gesture while not running asks again – a resume
    /// that was not allowed leaves the context suspended.
    pub fn on_gesture(&mut self) -> bool {
        self.gestures += 1;
        match self.state {
            Unlock::Locked | Unlock::Resuming => {
                self.state = Unlock::Resuming;
                self.resumes += 1;
                true
            }
            _ => false,
        }
    }

    /// `resume()` failed (its promise was rejected).
    pub fn on_resume_failed(&mut self, error: String) {
        if self.state == Unlock::Resuming {
            self.state = Unlock::Locked;
        }
        self.last_error = Some(error);
    }

    /// The mixer renders and plays.
    pub fn plays(&self) -> bool {
        self.state == Unlock::Running
    }

    /// The page's hint while the sound waits for a gesture.
    pub fn hint(&self) -> Option<&'static str> {
        matches!(self.state, Unlock::Locked | Unlock::Resuming).then_some(HINT)
    }

    /// Gestures seen and `resume()` calls asked for.
    pub fn counts(&self) -> (u32, u32) {
        (self.gestures, self.resumes)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// One word for the status: unavailable, locked, resuming, running,
    /// closed.
    pub fn word(&self) -> &'static str {
        match self.state {
            Unlock::Unavailable(_) => "unavailable",
            Unlock::Locked => "locked",
            Unlock::Resuming => "resuming",
            Unlock::Running => "running",
            Unlock::Closed => "closed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_locked_and_unlocks_on_the_first_gesture() {
        let mut g = AudioGate::new(Ok(ContextState::Suspended));
        assert_eq!(g.state(), &Unlock::Locked);
        assert!(!g.plays());
        assert_eq!(g.hint(), Some(HINT));
        // Nothing until a gesture.
        g.on_state(ContextState::Suspended);
        assert_eq!(g.state(), &Unlock::Locked);
        assert!(g.on_gesture(), "resume in the handler");
        assert_eq!(g.state(), &Unlock::Resuming);
        // Still suspended a moment later: still resuming.
        g.on_state(ContextState::Suspended);
        assert_eq!(g.state(), &Unlock::Resuming);
        g.on_state(ContextState::Running);
        assert!(g.plays());
        assert_eq!(g.hint(), None);
        // Further gestures do nothing.
        assert!(!g.on_gesture());
        assert_eq!(g.counts(), (2, 1));
        assert_eq!(g.word(), "running");
    }

    /// Autoplay allowed (the site was used before, or a kiosk flag): the
    /// context runs at once, no gesture needed.
    #[test]
    fn a_running_context_needs_no_gesture() {
        let g = AudioGate::new(Ok(ContextState::Running));
        assert!(g.plays());
        assert_eq!(g.hint(), None);
    }

    #[test]
    fn a_refused_resume_waits_for_the_next_gesture() {
        let mut g = AudioGate::new(Ok(ContextState::Suspended));
        assert!(g.on_gesture());
        g.on_resume_failed("NotAllowedError".into());
        assert_eq!(g.state(), &Unlock::Locked);
        assert_eq!(g.last_error(), Some("NotAllowedError"));
        assert!(g.on_gesture(), "asked again");
        g.on_state(ContextState::Running);
        assert!(g.plays());
    }

    /// Safari interrupts a running context: locked again, the next
    /// gesture resumes it; a closed context stays closed.
    #[test]
    fn interruptions_and_closing() {
        let mut g = AudioGate::new(Ok(ContextState::Running));
        g.on_state(ContextState::Interrupted);
        assert_eq!(g.state(), &Unlock::Locked);
        assert!(g.on_gesture());
        g.on_state(ContextState::Running);
        g.on_state(ContextState::Suspended);
        assert_eq!(g.state(), &Unlock::Locked, "suspended by the browser");
        g.on_state(ContextState::Closed);
        assert_eq!(g.state(), &Unlock::Closed);
        g.on_state(ContextState::Running);
        assert_eq!(g.state(), &Unlock::Closed);
        assert!(!g.on_gesture());
        assert_eq!(ContextState::parse("running"), ContextState::Running);
        assert_eq!(ContextState::parse("weird"), ContextState::Suspended);
    }

    #[test]
    fn without_web_audio_nothing_plays() {
        let mut g = AudioGate::new(Err("no AudioContext".into()));
        assert!(!g.on_gesture());
        g.on_state(ContextState::Running);
        assert_eq!(g.state(), &Unlock::Unavailable("no AudioContext".into()));
        assert_eq!(g.hint(), None);
        assert_eq!(g.word(), "unavailable");
    }
}
