//! Who hears the rider and whom the rider listens to (spec §7.1.3,
//! §7.1.7 Q6, Q8): the settings *Who hears me* (`voice.heard_by`: No one –
//! the default –, All nearby, Selected riders) and *Voice room*
//! (`voice.room`: Everybody – the default –, Selected riders), and what the
//! game server answers ([`Hearing`]: whose voice the rider may receive and
//! who may receive the rider's). Pure.

use std::collections::BTreeSet;

use ocw_core::RiderId;
use ocw_protocol::v1;

/// *Who hears me* (spec §7.1.7 Q8).
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

/// *Voice room*: whom the rider listens to (spec §7.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Listen {
    /// Everybody within hearing range (the default).
    #[default]
    Everybody,
    /// Only the selected riders (within hearing range).
    Selected,
}

impl Listen {
    pub const ALL: [Self; 2] = [Self::Everybody, Self::Selected];

    /// The setting's value (`voice.room`).
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
            Self::Selected => "Selected riders",
        }
    }

    pub fn to_wire(self) -> v1::VoiceListen {
        match self {
            Self::Everybody => v1::VoiceListen::Everybody,
            Self::Selected => v1::VoiceListen::Selected,
        }
    }

    pub fn from_wire(w: v1::VoiceListen) -> Self {
        match w {
            v1::VoiceListen::Everybody => Self::Everybody,
            v1::VoiceListen::Selected => Self::Selected,
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
}
