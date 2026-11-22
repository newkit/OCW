//! The headphones per viewer (spec §2.6 *Rider status signs*, §7.1.7
//! Q11, Q12, Q17): which riders cannot hear a given viewer.
//!
//! Each client tells the server in its status (`RiderStatus`) whether it
//! hears nobody (`hears_nobody`: the speaker muted, *I hear: nobody*),
//! whom it chose not to hear (`not_hearing`: muted for itself, not in its
//! *I hear: selected*) and whom it set to 0 % volume (`silenced`, spec
//! §7.1.7 Q18); the reasons are worked out by the client
//! (`ocw_client::voice::not_hearing_reasons`). The server adds the blocks
//! it knows (`VoiceBlocks`, Q18: the blocker shows headphones to the
//! rider it blocked – this overrides 7.1.10's "the blocked rider is not
//! told" for the sign). Q17 (b): the server keeps those lists and sends
//! each viewer only one yes / no per rider – the riders who cannot hear
//! *that viewer* ([`cannot_hear`], sent as `NotHearingYou`) – and strips
//! the lists from the snapshots ([`relayed`]). The server also knows *I
//! hear: nobody* from the rider's audience (`VoiceAudience.listen`, spec
//! §7.1.7 Q16) and counts it even before the rider's status says so.
//! Distance and *My voice* (who hears the rider) are no reasons (Q12,
//! Q16): they never show headphones.

use std::collections::{BTreeMap, BTreeSet};

use ocw_core::RiderId;
use ocw_protocol::v1;

/// The riders who cannot hear `viewer`: they hear nobody (by their
/// status, or `hear_nobody`: their audience says *I hear: nobody*), have
/// the viewer in their `not_hearing` (muted for themselves, not
/// selected) or `silenced` (0 % volume), or blocked the viewer
/// (`blocked`: blocker → the riders it blocked; Q18). Never the viewer
/// itself; nobody while the viewer is not in the voice call
/// (`viewer_in_voice`, from its own status – there is nothing to hear
/// then).
pub fn cannot_hear(
    viewer: RiderId,
    viewer_in_voice: bool,
    statuses: &BTreeMap<RiderId, v1::RiderStatus>,
    hear_nobody: &BTreeSet<RiderId>,
    blocked: &BTreeMap<RiderId, BTreeSet<RiderId>>,
) -> BTreeSet<RiderId> {
    if !viewer_in_voice {
        return BTreeSet::new();
    }
    statuses
        .iter()
        .filter(|(id, st)| {
            **id != viewer
                && (st.hears_nobody
                    || st.not_hearing.contains(&viewer.0)
                    || st.silenced.contains(&viewer.0))
        })
        .map(|(id, _)| *id)
        .chain(hear_nobody.iter().copied())
        .chain(
            blocked
                .iter()
                .filter(|(_, set)| set.contains(&viewer))
                .map(|(blocker, _)| *blocker),
        )
        .filter(|r| *r != viewer)
        .collect()
}

/// A rider's status as the snapshots carry it to everybody: the signs
/// everybody sees, without the list of whom the rider does not hear
/// (Q17: it never leaves the server).
pub fn relayed(st: &v1::RiderStatus) -> v1::RiderStatus {
    v1::RiderStatus {
        not_hearing: Vec::new(),
        silenced: Vec::new(),
        ..st.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(hears_nobody: bool, not_hearing: &[u32]) -> v1::RiderStatus {
        v1::RiderStatus {
            in_voice: true,
            hears_nobody,
            not_hearing: not_hearing.to_vec(),
            ..Default::default()
        }
    }

    fn ids(set: &BTreeSet<RiderId>) -> Vec<u32> {
        set.iter().map(|r| r.0).collect()
    }

    /// A muted viewer sees headphones on the rider who muted it, nobody
    /// else does; a rider who hears nobody shows them to every viewer in
    /// the call; never on the own rider; nothing outside the call.
    #[test]
    fn per_viewer_from_mutes_and_hears_nobody() {
        let statuses: BTreeMap<RiderId, v1::RiderStatus> = [
            (RiderId(1), st(false, &[])),
            (RiderId(2), st(false, &[1, 3])),
            (RiderId(3), st(false, &[])),
            (RiderId(4), st(true, &[])),
        ]
        .into();
        let none = BTreeSet::new();
        let no_blocks = BTreeMap::new();
        assert_eq!(
            ids(&cannot_hear(RiderId(1), true, &statuses, &none, &no_blocks)),
            [2, 4]
        );
        assert_eq!(
            ids(&cannot_hear(RiderId(3), true, &statuses, &none, &no_blocks)),
            [2, 4]
        );
        assert_eq!(
            ids(&cannot_hear(RiderId(2), true, &statuses, &none, &no_blocks)),
            [4]
        );
        assert_eq!(
            ids(&cannot_hear(RiderId(4), true, &statuses, &none, &no_blocks)),
            Vec::<u32>::new(),
            "never on the own rider"
        );
        assert!(cannot_hear(RiderId(1), false, &statuses, &none, &no_blocks).is_empty());
        // A viewer without a status of its own (a bot) is not in the call.
        assert!(cannot_hear(RiderId(9), false, &statuses, &none, &no_blocks).is_empty());
    }

    /// *I hear: nobody* (spec §7.1.7 Q16) from the audience: every viewer
    /// in the call sees the headphones on that rider, the rider itself
    /// none; a viewer outside the call none.
    #[test]
    fn i_hear_nobody_shows_to_every_viewer() {
        let statuses: BTreeMap<RiderId, v1::RiderStatus> = [
            (RiderId(1), st(false, &[])),
            (RiderId(2), st(false, &[])),
            (RiderId(3), st(false, &[])),
        ]
        .into();
        let nobody: BTreeSet<RiderId> = [RiderId(3)].into();
        let no_blocks = BTreeMap::new();
        assert_eq!(
            ids(&cannot_hear(
                RiderId(1),
                true,
                &statuses,
                &nobody,
                &no_blocks
            )),
            [3]
        );
        assert_eq!(
            ids(&cannot_hear(
                RiderId(2),
                true,
                &statuses,
                &nobody,
                &no_blocks
            )),
            [3]
        );
        assert!(cannot_hear(RiderId(3), true, &statuses, &nobody, &no_blocks).is_empty());
        assert!(cannot_hear(RiderId(1), false, &statuses, &nobody, &no_blocks).is_empty());
    }

    /// Spec §7.1.7 Q18: a rider shows headphones to a viewer it muted for
    /// itself, set to 0 % volume or blocked – each only to that viewer;
    /// the blocked rider sees them on the blocker, the blocker none on the
    /// blocked rider (who did not block back); never on the own rider.
    #[test]
    fn mute_zero_volume_and_block_each_count_for_that_viewer_only() {
        let none = BTreeSet::new();
        let no_blocks = BTreeMap::new();
        // Ada (1) muted Bea (2) for herself; Bea set Cleo (3) to 0 %;
        // Cleo and Dan (4) did nothing.
        let mut statuses: BTreeMap<RiderId, v1::RiderStatus> = [
            (RiderId(1), st(false, &[2])),
            (RiderId(2), st(false, &[])),
            (RiderId(3), st(false, &[])),
            (RiderId(4), st(false, &[])),
        ]
        .into();
        statuses.get_mut(&RiderId(2)).unwrap().silenced = vec![3];
        let see = |v: u32, blocks: &BTreeMap<RiderId, BTreeSet<RiderId>>| {
            ids(&cannot_hear(RiderId(v), true, &statuses, &none, blocks))
        };
        assert_eq!(see(2, &no_blocks), [1], "muted for me");
        assert_eq!(see(3, &no_blocks), [2], "0 % volume");
        assert!(see(1, &no_blocks).is_empty() && see(4, &no_blocks).is_empty());
        // Dan blocked Ada.
        let blocks: BTreeMap<RiderId, BTreeSet<RiderId>> =
            [(RiderId(4), [RiderId(1)].into())].into();
        assert_eq!(see(1, &blocks), [4], "the blocked rider sees the blocker's");
        assert!(see(4, &blocks).is_empty(), "the blocker sees none on Ada");
        assert_eq!(see(2, &blocks), [1], "the others unchanged");
        assert_eq!(see(3, &blocks), [2]);
        // A rider who blocked itself (a name clash) never shows to itself.
        let own: BTreeMap<RiderId, BTreeSet<RiderId>> = [(RiderId(4), [RiderId(4)].into())].into();
        assert!(see(4, &own).is_empty());
        // Outside the call: nothing.
        assert!(cannot_hear(RiderId(1), false, &statuses, &none, &blocks).is_empty());
    }

    /// The snapshots carry every sign but the lists.
    #[test]
    fn the_relayed_status_has_no_list() {
        let full = v1::RiderStatus {
            voice_muted: true,
            in_menu: true,
            in_vr: true,
            in_voice: true,
            hears_nobody: true,
            not_hearing: vec![3, 9],
            silenced: vec![4],
        };
        let r = relayed(&full);
        assert!(r.not_hearing.is_empty() && r.silenced.is_empty());
        assert_eq!(
            r,
            v1::RiderStatus {
                not_hearing: Vec::new(),
                silenced: Vec::new(),
                ..full
            }
        );
    }
}
