//! Where each kind of model switches from its near model to its far one
//! (`lod1`, spec 15B) – the one table of the game and of OCW Studio's
//! *auto* level (spec 14 Phase 16, 16H Q24 (a)): bikes, riders and their
//! wear at [`crate::riders::LOD_DISTANCE_M`] (40 m), the unicorn at 40 m
//! like the riders (it was 80 m, Phase 13C), props (lamps, posts,
//! bollards, gates, …) at [`PROP_LOD_M`] (60 m), plants in the middle of
//! [`crate::vegetation::NEAR_FADE_M`] (270 m); workshop pieces and bike
//! parts never (only seen up close, or drawn with their bike).
//!
//! Each switch cross-fades over `at_m ∓ fade_m` (Bevy's dithered
//! `VisibilityRange`, as the riders always did): the near model fades out
//! while the far one fades in, the middle of the fade is the switch.

use std::ops::Range;

use bevy::camera::visibility::VisibilityRange;
use ocw_assets::kinds::Kind;

/// A kind's switch from the near model to `lod1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodSwitch {
    /// The middle of the cross-fade (m from the camera).
    pub at_m: f32,
    /// Half its width (m).
    pub fade_m: f32,
}

/// Props switch here (m).
pub const PROP_LOD_M: f32 = 60.0;

/// Bikes, riders and their wear (`riders::near_range` / `far_range`).
pub const RIDER: LodSwitch = LodSwitch {
    at_m: crate::riders::LOD_DISTANCE_M,
    fade_m: crate::riders::LOD_FADE_M,
};
/// The unicorn: like the riders (spec 16H Q24 (a)).
pub const CREATURE: LodSwitch = RIDER;
/// Props: lamps, posts, bollards, gates, barriers (a tenth either side,
/// like the riders' 4 m of 40 m).
pub const PROP: LodSwitch = LodSwitch {
    at_m: PROP_LOD_M,
    fade_m: 6.0,
};
/// Plants: the near models give way to the far ones over
/// `vegetation::NEAR_FADE_M` (230 – 310 m).
pub const PLANT: LodSwitch = LodSwitch {
    at_m: 0.5 * (crate::vegetation::NEAR_FADE_M.start + crate::vegetation::NEAR_FADE_M.end),
    fade_m: 0.5 * (crate::vegetation::NEAR_FADE_M.end - crate::vegetation::NEAR_FADE_M.start),
};

/// The switch of every kind (`None`: never, the near model at any
/// distance).
pub const TABLE: [(Kind, Option<LodSwitch>); 8] = [
    (Kind::Bike, Some(RIDER)),
    (Kind::Rider, Some(RIDER)),
    (Kind::Wearable, Some(RIDER)),
    (Kind::Part, None),
    (Kind::Prop, Some(PROP)),
    (Kind::Plant, Some(PLANT)),
    (Kind::Creature, Some(CREATURE)),
    (Kind::Workshop, None),
];

/// The switch of `kind` ([`TABLE`]).
pub fn switch(kind: Kind) -> Option<LodSwitch> {
    TABLE.iter().find(|(k, _)| *k == kind).and_then(|(_, s)| *s)
}

/// Where `kind` switches to `lod1` (m), `None` for never.
pub fn distance_m(kind: Kind) -> Option<f32> {
    switch(kind).map(|s| s.at_m)
}

impl LodSwitch {
    /// The cross-fade (m from the camera).
    pub fn fade(self) -> Range<f32> {
        self.at_m - self.fade_m..self.at_m + self.fade_m
    }

    /// Whether the far model is the one shown at `distance_m` (beyond the
    /// middle of the fade).
    pub fn far_at(self, distance_m: f32) -> bool {
        distance_m > self.at_m
    }

    /// The near model's range: up to the switch.
    pub fn near_range(self) -> VisibilityRange {
        VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: self.fade(),
            use_aabb: false,
        }
    }

    /// The far model's range: from the switch to `end` (m, faded out over
    /// its span; `None`: no end).
    pub fn far_range(self, end: Option<Range<f32>>) -> VisibilityRange {
        VisibilityRange {
            start_margin: self.fade(),
            end_margin: end.unwrap_or(1.0e7..1.0e7),
            use_aabb: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distance per kind (spec 16H Q24 (a)): riders, bikes and wear
    /// 40 m, the unicorn 40 m, props 60 m, plants 270 m, workshop pieces
    /// and parts never – every kind in the table once.
    #[test]
    fn the_switch_distance_of_every_kind() {
        let want = [
            (Kind::Bike, Some(40.0)),
            (Kind::Rider, Some(40.0)),
            (Kind::Wearable, Some(40.0)),
            (Kind::Part, None),
            (Kind::Prop, Some(60.0)),
            (Kind::Plant, Some(270.0)),
            (Kind::Creature, Some(40.0)),
            (Kind::Workshop, None),
        ];
        for (k, d) in want {
            assert_eq!(distance_m(k), d, "{k:?}");
        }
        for k in Kind::ALL {
            assert_eq!(TABLE.iter().filter(|(t, _)| *t == k).count(), 1, "{k:?}");
        }
        // The riders' own ranges are this table's.
        assert_eq!(crate::riders::near_range().end_margin, RIDER.fade());
        assert_eq!(crate::riders::far_range().start_margin, RIDER.fade());
        // The plants fade over the vegetation's range.
        assert_eq!(PLANT.fade(), crate::vegetation::NEAR_FADE_M);
    }

    /// The two models cross-fade over the same band: the near one fades
    /// out where the far one fades in, the switch in its middle.
    #[test]
    fn near_and_far_meet_in_one_fade() {
        for (_, s) in TABLE.iter().filter_map(|(k, s)| s.map(|s| (k, s))) {
            assert_eq!(s.near_range().end_margin, s.far_range(None).start_margin);
            assert!(s.fade().contains(&s.at_m));
            assert!(!s.far_at(s.at_m - 1.0) && s.far_at(s.at_m + 1.0));
        }
        let end = PROP.far_range(Some(320.0..350.0));
        assert_eq!(end.end_margin, 320.0..350.0);
        assert_eq!(PROP.fade(), 54.0..66.0);
    }
}
