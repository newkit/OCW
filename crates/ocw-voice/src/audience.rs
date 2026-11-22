//! Who hears the rider and whom the rider listens to (spec §7.1.3,
//! §7.1.7 Q6, Q8, Q16): the Voice page's two rows *My voice* (on / off ·
//! Everybody / Selected; [`MyVoice`], setting `voice.my_voice`, the on /
//! off is the microphone) and *I hear* (everybody / selected / nobody;
//! [`Listen`], setting `voice.i_hear`), what they mean on the wire
//! ([`HeardBy`], [`Listen`]) and what the game server answers
//! ([`Hearing`]: whose voice the rider may receive and who may receive
//! the rider's). Pure.
//!
//! **The mapping (spec §7.1.7 Q16):**
//!
//! | My voice          | microphone      | *Who hears me* on the wire |
//! |-------------------|-----------------|----------------------------|
//! | off (any)         | muted (X mouth) | `NO_ONE`                   |
//! | on · Everybody    | on              | `NEARBY` (within range)    |
//! | on · Selected     | on              | `SELECTED`                 |
//!
//! Every ride starts with My voice off (Q7, the on / off is never
//! remembered); the audience (Everybody / Selected) is. *I hear*:
//! everybody → `EVERYBODY`, selected → `SELECTED`, nobody → `NOBODY`.
//! Old settings ([`MyVoice::from_heard_by`], `ocw_client::settings`):
//! `voice.heard_by` `no_one` → Everybody (and off, as every ride starts),
//! `nearby` → Everybody, `selected` → Selected; `voice.room` → *I hear*
//! (everybody / selected); `voice.enabled = off` → *I hear: nobody* (and
//! My voice off): both rows off, the old voice off.

use std::collections::BTreeSet;

use ocw_core::RiderId;
use ocw_protocol::v1;

/// Who receives the rider's voice, as the game server is told (spec
/// §7.1.7 Q8): from *My voice* by [`MyVoice::heard_by`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeardBy {
    /// Nobody (the default).
    #[default]
    NoOne,
    /// Everybody within hearing range.
    Nearby,
    /// The selected riders within hearing range.
    Selected,
}

impl HeardBy {
    pub const ALL: [Self; 3] = [Self::NoOne, Self::Nearby, Self::Selected];

    /// The setting's value (`voice.heard_by`).
    pub fn key(self) -> &'static str {
        match self {
            Self::NoOne => "no_one",
            Self::Nearby => "nearby",
            Self::Selected => "selected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.key() == s)
    }

    /// The menu's words.
    pub fn label(self) -> &'static str {
        match self {
            Self::NoOne => "No one",
            Self::Nearby => "All nearby",
            Self::Selected => "Selected riders",
        }
    }

    pub fn to_wire(self) -> v1::VoiceHeardBy {
        match self {
            Self::NoOne => v1::VoiceHeardBy::NoOne,
            Self::Nearby => v1::VoiceHeardBy::Nearby,
            Self::Selected => v1::VoiceHeardBy::Selected,
        }
    }

    pub fn from_wire(w: v1::VoiceHeardBy) -> Self {
        match w {
            v1::VoiceHeardBy::NoOne => Self::NoOne,
            v1::VoiceHeardBy::Nearby => Self::Nearby,
            v1::VoiceHeardBy::Selected => Self::Selected,
        }
    }
}

/// *My voice*'s audience (spec §7.1.7 Q16): whom the rider's voice
/// reaches while it is on (setting `voice.my_voice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MyVoice {
    /// Everybody within hearing range (the default; the old *All nearby*).
    #[default]
    Everybody,
    /// The selected riders (the one list of *Choose riders*, Q19).
    Selected,
}

impl MyVoice {
    pub const ALL: [Self; 2] = [Self::Everybody, Self::Selected];

    /// The setting's value (`voice.my_voice`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Everybody => "everybody",
            Self::Selected => "selected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.key() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Everybody => "Everybody",
            Self::Selected => "Selected",
        }
    }

    /// Who receives the voice: nobody while it is off (the microphone
    /// muted), else everybody nearby or the selected riders.
    pub fn heard_by(self, on: bool) -> HeardBy {
        match (on, self) {
            (false, _) => HeardBy::NoOne,
            (true, Self::Everybody) => HeardBy::Nearby,
            (true, Self::Selected) => HeardBy::Selected,
        }
    }

    /// The other way (an old `voice.heard_by`): whether it was on and the
    /// audience. *No one* is My voice off with the default audience.
    pub fn from_heard_by(h: HeardBy) -> (bool, Self) {
        match h {
            HeardBy::NoOne => (false, Self::Everybody),
            HeardBy::Nearby => (true, Self::Everybody),
            HeardBy::Selected => (true, Self::Selected),
        }
    }
}

/// *I hear*: whom the rider listens to (spec §7.1.3, §7.1.7 Q16; setting
/// `voice.i_hear`, before `voice.room`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Listen {
    /// Everybody within hearing range (10 m, the default).
    #[default]
    Everybody,
    /// Only the selected riders, at any distance (spec §7.1.7 Q20).
    Selected,
    /// Nobody (spec §7.1.7 Q16): no voice is received, and every rider in
    /// the call sees the headphones; others may still hear the rider.
    Nobody,
}

impl Listen {
    pub const ALL: [Self; 3] = [Self::Everybody, Self::Selected, Self::Nobody];

    /// The setting's value (`voice.i_hear`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Everybody => "everybody",
            Self::Selected => "selected",
            Self::Nobody => "nobody",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.key() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Everybody => "Everybody",
            Self::Selected => "Selected",
            Self::Nobody => "Nobody",
        }
    }

    pub fn to_wire(self) -> v1::VoiceListen {
        match self {
            Self::Everybody => v1::VoiceListen::Everybody,
            Self::Selected => v1::VoiceListen::Selected,
            Self::Nobody => v1::VoiceListen::Nobody,
        }
    }

    pub fn from_wire(w: v1::VoiceListen) -> Self {
        match w {
            v1::VoiceListen::Everybody => Self::Everybody,
            v1::VoiceListen::Selected => Self::Selected,
            v1::VoiceListen::Nobody => Self::Nobody,
        }
    }
}

/// The game server's answer (`VoiceHearing`, spec §7.1.7 Q6): the riders
/// whose voice this rider may receive and those who may receive this
/// rider's (range, both audiences and the blocks already applied).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hearing {
    pub may_hear: BTreeSet<RiderId>,
    pub heard_by: BTreeSet<RiderId>,
}

impl Hearing {
    pub fn from_wire(h: &v1::VoiceHearing) -> Self {
        Self {
            may_hear: h.may_hear.iter().copied().map(RiderId).collect(),
            heard_by: h.heard_by.iter().copied().map(RiderId).collect(),
        }
    }

    pub fn to_wire(&self) -> v1::VoiceHearing {
        v1::VoiceHearing {
            may_hear: self.may_hear.iter().map(|r| r.0).collect(),
            heard_by: self.heard_by.iter().map(|r| r.0).collect(),
        }
    }
}

/// The audience message for the game server: the two settings and the
/// selected riders (account ids, names on this server).
pub fn audience_message(
    heard_by: HeardBy,
    listen: Listen,
    selected: (Vec<String>, Vec<String>),
) -> v1::VoiceAudience {
    v1::VoiceAudience {
        heard_by: heard_by.to_wire() as i32,
        listen: listen.to_wire() as i32,
        selected_account_ids: selected.0,
        selected_names: selected.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_values_labels_and_the_wire() {
        for h in HeardBy::ALL {
            assert_eq!(HeardBy::parse(h.key()), Some(h));
            assert_eq!(HeardBy::from_wire(h.to_wire()), h);
        }
        for l in Listen::ALL {
            assert_eq!(Listen::parse(l.key()), Some(l));
            assert_eq!(Listen::from_wire(l.to_wire()), l);
        }
        assert_eq!(HeardBy::default(), HeardBy::NoOne, "Q8: default no one");
        assert_eq!(Listen::default(), Listen::Everybody);
        assert_eq!(HeardBy::parse("all"), None);
        // The wire's default (0) is the decided default: a client that
        // never said is heard by no one.
        assert_eq!(
            v1::VoiceAudience::default().heard_by(),
            v1::VoiceHeardBy::NoOne
        );
        assert_eq!(
            v1::VoiceAudience::default().listen(),
            v1::VoiceListen::Everybody
        );
        let h = Hearing {
            may_hear: [RiderId(2), RiderId(5)].into(),
            heard_by: [RiderId(5)].into(),
        };
        assert_eq!(Hearing::from_wire(&h.to_wire()), h);
        let m = audience_message(
            HeardBy::Selected,
            Listen::Selected,
            (vec!["acc-1".into()], vec!["Bea Climber".into()]),
        );
        assert_eq!(m.heard_by(), v1::VoiceHeardBy::Selected);
        assert_eq!(m.listen(), v1::VoiceListen::Selected);
        assert_eq!(m.selected_names, ["Bea Climber"]);
    }

    /// Spec §7.1.7 Q16: My voice off is heard by no one whatever the
    /// audience; on · Everybody is the old *All nearby*, on · Selected the
    /// old *Selected riders*; and back: an old value gives the row it
    /// meant (no one → off).
    #[test]
    fn my_voice_maps_both_ways() {
        for m in MyVoice::ALL {
            assert_eq!(MyVoice::parse(m.key()), Some(m));
            assert_eq!(m.heard_by(false), HeardBy::NoOne, "{m:?} off");
        }
        assert_eq!(MyVoice::Everybody.heard_by(true), HeardBy::Nearby);
        assert_eq!(MyVoice::Selected.heard_by(true), HeardBy::Selected);
        for h in HeardBy::ALL {
            let (on, m) = MyVoice::from_heard_by(h);
            assert_eq!(m.heard_by(on), h, "round trip of {h:?}");
        }
        assert_eq!(
            MyVoice::from_heard_by(HeardBy::NoOne),
            (false, MyVoice::Everybody)
        );
        assert_eq!(MyVoice::default(), MyVoice::Everybody);
        assert_eq!(Listen::parse("nobody"), Some(Listen::Nobody));
        assert_eq!(Listen::Nobody.to_wire(), v1::VoiceListen::Nobody);
        assert_eq!(
            audience_message(HeardBy::Nearby, Listen::Nobody, Default::default()).listen(),
            v1::VoiceListen::Nobody
        );
    }
}
