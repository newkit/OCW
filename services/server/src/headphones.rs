//! The headphones per viewer (spec §2.6 *Rider status signs*, §7.1.7
//! Q11, Q12, Q17): which riders cannot hear a given viewer.
//!
//! Each client tells the server in its status (`RiderStatus`) whether it
//! hears nobody (`hears_nobody`: the speaker muted, voice turned off) and
//! whom it chose not to hear (`not_hearing`: muted for itself, not in its
//! *Voice room: Selected riders*; the reasons are worked out by the
//! client, `ocw_client::voice::not_hearing_reasons`). Q17 (b): the server
//! keeps those lists and sends each viewer only one yes / no per rider –
//! the riders who cannot hear *that viewer* ([`cannot_hear`], sent as
//! `NotHearingYou`) – and strips the lists from the snapshots
//! ([`relayed`]). Distance, *Who hears me* and blocks are no reasons
//! (Q12, Q18): they never show headphones.

use std::collections::{BTreeMap, BTreeSet};

use ocw_core::RiderId;
use ocw_protocol::v1;

/// The riders who cannot hear `viewer`: they hear nobody or have the
/// viewer in their `not_hearing`. Never the viewer itself; nobody while
/// the viewer is not in the voice call (`viewer_in_voice`, from its own
/// status – there is nothing to hear then).
pub fn cannot_hear(
    viewer: RiderId,
    viewer_in_voice: bool,
    statuses: &BTreeMap<RiderId, v1::RiderStatus>,
) -> BTreeSet<RiderId> {
    if !viewer_in_voice {
        return BTreeSet::new();
    }
    statuses
        .iter()
        .filter(|(id, st)| {
            **id != viewer && (st.hears_nobody || st.not_hearing.contains(&viewer.0))
        })
        .map(|(id, _)| *id)
        .collect()
}

/// A rider's status as the snapshots carry it to everybody: the signs
/// everybody sees, without the list of whom the rider does not hear
/// (Q17: it never leaves the server).
pub fn relayed(st: &v1::RiderStatus) -> v1::RiderStatus {
    v1::RiderStatus {
        not_hearing: Vec::new(),
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
        assert_eq!(ids(&cannot_hear(RiderId(1), true, &statuses)), [2, 4]);
        assert_eq!(ids(&cannot_hear(RiderId(3), true, &statuses)), [2, 4]);
        assert_eq!(ids(&cannot_hear(RiderId(2), true, &statuses)), [4]);
        assert_eq!(
            ids(&cannot_hear(RiderId(4), true, &statuses)),
            Vec::<u32>::new(),
            "never on the own rider"
        );
        assert!(cannot_hear(RiderId(1), false, &statuses).is_empty());
        // A viewer without a status of its own (a bot) is not in the call.
        assert!(cannot_hear(RiderId(9), false, &statuses).is_empty());
    }

    /// The snapshots carry every sign but the list.
    #[test]
    fn the_relayed_status_has_no_list() {
        let full = v1::RiderStatus {
            voice_muted: true,
            in_menu: true,
            in_vr: true,
            in_voice: true,
            hears_nobody: true,
            not_hearing: vec![3, 9],
        };
        let r = relayed(&full);
        assert!(r.not_hearing.is_empty());
        assert_eq!(
            r,
            v1::RiderStatus {
                not_hearing: Vec::new(),
                ..full
            }
        );
    }
}
