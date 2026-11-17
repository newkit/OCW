//! Voice blocks on the game server (spec §7.1.7 Q9, §7.1.10): who
//! exchanges no voice with whom.
//!
//! Each rider tells the server whom they blocked ([`v1::VoiceBlocks`]:
//! account ids and display names). A block works **both ways**: the
//! blocked rider does not hear the blocker and the blocker does not hear
//! the blocked rider. The server computes, per rider, the riders they are
//! *apart* from – a block in either direction – and sends each rider only
//! their own set ([`v1::VoiceApart`]), never who blocked whom. The clients
//! then neither subscribe to those riders' microphones nor let them
//! subscribe to their own (LiveKit's track subscription permissions, which
//! the voice server enforces against the other side's client).
//!
//! Pure: the game server keeps a [`Blocks`] per connection and calls
//! [`apart`] with the riders of the session.

use std::collections::{BTreeMap, BTreeSet};

use ocw_core::RiderId;
use ocw_protocol::v1;

/// At most this many entries of each kind are kept per rider.
pub const MAX_ENTRIES: usize = 256;
/// Longest name or account id kept (longer ones cannot match anyway).
pub const MAX_ENTRY_LEN: usize = 128;

/// The riders one rider blocked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blocks {
    pub account_ids: BTreeSet<String>,
    pub names: BTreeSet<String>,
}

impl Blocks {
    /// From the wire: trimmed, empty and over-long entries dropped, at
    /// most [`MAX_ENTRIES`] of each.
    pub fn from_wire(b: &v1::VoiceBlocks) -> Self {
        let clean = |list: &[String]| -> BTreeSet<String> {
            list.iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ENTRY_LEN)
                .take(MAX_ENTRIES)
                .map(str::to_owned)
                .collect()
        };
        Self {
            account_ids: clean(&b.account_ids),
            names: clean(&b.names),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.account_ids.is_empty() && self.names.is_empty()
    }

    /// Whether this list blocks `who`: by the account when signed in, by
    /// the display name always (a name block also holds when that rider
    /// signs in; blocking errs on the safe side).
    pub fn blocks(&self, who: &Who) -> bool {
        who.account_id
            .as_ref()
            .is_some_and(|a| self.account_ids.contains(a))
            || self.names.contains(&who.name)
    }
}

/// A rider as the block rules see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    pub rider: RiderId,
    pub name: String,
    pub account_id: Option<String>,
}

/// For every rider, the riders they exchange no voice with: those they
/// blocked and those who blocked them. Riders without any block are not
/// in the map.
pub fn apart<K: Ord + Copy>(
    riders: &BTreeMap<K, Who>,
    blocks: &BTreeMap<K, Blocks>,
) -> BTreeMap<K, BTreeSet<RiderId>> {
    let mut out: BTreeMap<K, BTreeSet<RiderId>> = BTreeMap::new();
    for (ka, a) in riders {
        let Some(list) = blocks.get(ka).filter(|b| !b.is_empty()) else {
            continue;
        };
        for (kb, b) in riders {
            if ka == kb || a.rider == b.rider || !list.blocks(b) {
                continue;
            }
            out.entry(*ka).or_default().insert(b.rider);
            out.entry(*kb).or_default().insert(a.rider);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(id: u32, name: &str, account: Option<&str>) -> Who {
        Who {
            rider: RiderId(id),
            name: name.into(),
            account_id: account.map(str::to_owned),
        }
    }

    fn set(ids: &[u32]) -> BTreeSet<RiderId> {
        ids.iter().copied().map(RiderId).collect()
    }

    /// A blocks B: A and B are apart from each other, C from nobody.
    #[test]
    fn a_block_works_both_ways_and_only_between_the_two() {
        let riders: BTreeMap<u8, Who> = [
            (1, who(1, "Ada Rider", None)),
            (2, who(2, "Bea Climber", None)),
            (3, who(3, "Cleo Sprinter", None)),
        ]
        .into();
        let blocks: BTreeMap<u8, Blocks> = [(
            1,
            Blocks::from_wire(&v1::VoiceBlocks {
                names: vec!["Bea Climber".into()],
                ..Default::default()
            }),
        )]
        .into();
        let a = apart(&riders, &blocks);
        assert_eq!(a.get(&1), Some(&set(&[2])));
        assert_eq!(a.get(&2), Some(&set(&[1])));
        assert_eq!(a.get(&3), None);
        // Both blocked each other: the same, once.
        let mut both = blocks.clone();
        both.insert(
            2,
            Blocks {
                names: ["Ada Rider".to_owned()].into(),
                ..Default::default()
            },
        );
        assert_eq!(apart(&riders, &both), a);
    }

    /// By account: matches the signed-in rider whatever the name; a name
    /// matches any rider of that name.
    #[test]
    fn accounts_and_names_match() {
        let riders: BTreeMap<u8, Who> = [
            (1, who(1, "Ada Rider", Some("acc-ada"))),
            (2, who(2, "Bea Renamed", Some("acc-bea"))),
            (3, who(3, "Cleo Sprinter", None)),
        ]
        .into();
        let blocks: BTreeMap<u8, Blocks> = [(
            3,
            Blocks::from_wire(&v1::VoiceBlocks {
                account_ids: vec!["acc-bea".into(), " ".into()],
                names: vec![" Ada Rider ".into(), "x".repeat(MAX_ENTRY_LEN + 1)],
            }),
        )]
        .into();
        let b = &blocks[&3];
        assert_eq!(b.names.len(), 1, "trimmed, the over-long one dropped");
        assert_eq!(b.account_ids.len(), 1, "the empty one dropped");
        let a = apart(&riders, &blocks);
        assert_eq!(a.get(&3), Some(&set(&[1, 2])));
        assert_eq!(a.get(&1), Some(&set(&[3])));
        assert_eq!(a.get(&2), Some(&set(&[3])));
        // Nobody blocks themselves (an own name in the list).
        let me: BTreeMap<u8, Blocks> = [(
            1,
            Blocks {
                names: ["Ada Rider".to_owned()].into(),
                ..Default::default()
            },
        )]
        .into();
        assert!(apart(&riders, &me).is_empty());
    }

    #[test]
    fn long_lists_are_cut() {
        let b = Blocks::from_wire(&v1::VoiceBlocks {
            names: (0..1000).map(|i| format!("Rider {i}")).collect(),
            ..Default::default()
        });
        assert_eq!(b.names.len(), MAX_ENTRIES);
    }
}
