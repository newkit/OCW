//! The workshop scene as a Bevy-free model (spec §14 Phase 4, 4A, 4B):
//! the bikes on the stand and the wall, the zones, the camera states, the
//! swap, the hotspots and the part picker. Input arrives as abstract
//! [`Act`]s; what the app has to do comes back as [`Out`]: console
//! command lines (the menu's rule, §3.2.1: menu, console and scripts
//! share one code path) and leaving.
//!
//! Navigation: in the room ← / → cycle the bikes (the stand's first,
//! then the hooks left to right, wrapping), ↑ / ↓ step through the zones
//! wall – stand – bench – mirror; Confirm on a wall bike swaps it onto
//! the stand ([`SWAP_S`]), on the stand bike goes to the stand view with
//! the part hotspots; there the directions move between the hotspots in
//! screen space, LB / RB step through the slots, Confirm opens the
//! picker. Back goes one level up and leaves from the overview; leaving
//! applies the changes ([`Out::Leave`]) – never before (4L Q1).
//!
//! The changing corner (W4, spec 4C): Confirm at the mirror opens the
//! character editor ([`View::Dress`]): the rider stands in front of the
//! mirror; the directions move between the rider's hotspots (head,
//! torso, hands, legs, feet) in screen space, LB / RB step through the
//! slots ([`MirrorSlot::ALL`]: kit and free character slots), Confirm
//! opens the slot's picker (free choices apply at once as a `character`
//! line; kit is bought / worn / sold like parts) or, on *Try on the
//! bike*, puts the rider on the stand bike ([`View::TryOn`]); Back steps
//! out again.

use bevy::math::{Vec2, Vec3};
use ocw_assets::parts::{Catalogue, Slot};
use ocw_assets::workshop::Layout;
use ocw_core::Rgb;

use super::camera::{CameraMachine, Orbit, Pose, Poses, View};
use super::input::Act;
use super::nav::{self, Dir, Rect};
use super::picker::{self, Kind, MirrorSlot, Picker, Region};
use super::wallet::{Bike, Garage};

/// Length of the bike swap (s): the stand's bike back onto its hook, the
/// chosen one down onto the stand.
pub const SWAP_S: f32 = 1.2;

/// A swap under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swap {
    /// The bike going back to its hook, and the one coming to the stand.
    pub from: usize,
    pub to: usize,
    pub t: f32,
}

impl Swap {
    pub fn progress(&self) -> f32 {
        (self.t / SWAP_S).clamp(0.0, 1.0)
    }
}

/// What the app does after an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Run this console command line.
    Line(String),
    /// Leave the workshop; `apply`: the bike changed (send `BikeChange`);
    /// `then`: a line to run afterwards (the menu page of the mirror).
    Leave { apply: bool, then: Option<String> },
}

/// The zones of the room (spec §14 Phase 4, 4A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Wall,
    Stand,
    Bench,
    Mirror,
}

/// What the model needs to know about the world this frame.
pub struct Ctx<'a> {
    pub cat: &'a Catalogue,
    pub garage: &'a Garage,
    pub balance: i64,
    pub layout: &'a Layout,
    /// Window size (logical px).
    pub size: Vec2,
    /// The hotspot anchors of the stand bike (world), per slot.
    pub anchors: &'a [(Slot, Vec3)],
    /// The corners of each bike's box (world), by bike index (on its hook
    /// or on the stand, wherever it is).
    pub boxes: &'a [[Vec3; 8]],
    /// The rider's character now (spec 4C).
    pub character: ocw_core::Character,
    /// The standing rider's hotspots (world; [`Region::ALL`] order).
    pub rider_spots: &'a [Vec3],
}

/// The character editor's state (spec 4C, W4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dress {
    /// Index into [`MirrorSlot::ALL`].
    pub slot: usize,
    /// The focused hotspot ([`Region::ALL`]).
    pub hotspot: usize,
    /// The rider turned about the vertical (orbit, rad).
    pub yaw: f32,
}

impl Default for Dress {
    fn default() -> Self {
        Self {
            slot: 0,
            hotspot: 0,
            yaw: 0.0,
        }
    }
}

impl Dress {
    pub fn slot(&self) -> MirrorSlot {
        MirrorSlot::ALL[self.slot % MirrorSlot::ALL.len()]
    }

    /// LB / RB: the next slot, the hotspot following its region.
    fn step(&mut self, d: isize) {
        let n = MirrorSlot::ALL.len() as isize;
        self.slot = (self.slot as isize + d).rem_euclid(n) as usize;
        if let Some(r) = self.slot().region() {
            self.hotspot = Region::ALL.iter().position(|x| *x == r).unwrap_or(0);
        }
    }

    /// A hotspot focused: its region's first slot.
    fn focus(&mut self, i: usize) {
        self.hotspot = i.min(Region::ALL.len() - 1);
        self.slot = Region::ALL[self.hotspot].first_slot();
    }
}

/// The workshop scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Workshop {
    /// Garage bike ids in hook order (hook i holds bike i).
    pub bikes: Vec<String>,
    pub stand: usize,
    pub focus: usize,
    /// The "+" hook is focused (pointer only).
    pub plus: bool,
    pub zone: Zone,
    pub swap: Option<Swap>,
    pub hotspot: usize,
    pub picker: Option<Picker>,
    /// A short text for the bench / mirror / "+" (or after an action).
    pub caption: Option<String>,
    pub cam: CameraMachine,
    pub orbit: Orbit,
    /// Something changed that leaving applies.
    pub changed: bool,
    /// The character editor (spec 4C).
    pub dress: Dress,
    /// The mirror shows no reflection (low graphics quality,
    /// `workshop.mirror off`): the editor looks straight at the rider.
    pub front_view: bool,
}

impl Workshop {
    /// Opens the workshop with the garage's bikes, the active one on the
    /// stand, in the overview.
    pub fn open(garage: &Garage, layout: &Layout) -> Self {
        let bikes: Vec<String> = garage.bikes.iter().map(|b| b.id.clone()).collect();
        let stand = bikes.iter().position(|b| *b == garage.active).unwrap_or(0);
        let orbit = Orbit::default();
        let at = Poses { layout }.pose(View::Overview, &orbit);
        Self {
            bikes,
            stand,
            focus: stand,
            plus: false,
            zone: Zone::Stand,
            swap: None,
            hotspot: 0,
            picker: None,
            caption: None,
            cam: CameraMachine::new(View::Overview, at),
            orbit,
            changed: false,
            dress: Dress::default(),
            front_view: false,
        }
    }

    /// The rider is on the stand bike (*Try on the bike*).
    pub fn trying_on(&self) -> bool {
        self.view() == View::TryOn
    }

    /// The character corner is open: editor, picker there, or trying on.
    pub fn dressing(&self) -> bool {
        matches!(self.view(), View::Dress { .. } | View::TryOn)
    }

    fn dress_view(&self) -> View {
        View::Dress {
            front: self.front_view,
        }
    }

    /// The rider hotspots' screen positions.
    pub fn rider_points(&self, ctx: &Ctx) -> Vec<Option<Vec2>> {
        let pose = self.pose(ctx.layout);
        ctx.rider_spots
            .iter()
            .map(|p| pose.to_screen(ctx.size, *p))
            .collect()
    }

    /// Opens the picker of the editor's slot (not *Try on the bike*).
    fn open_mirror_picker(&mut self, ctx: &Ctx) {
        self.picker = match self.dress.slot() {
            MirrorSlot::Char(c) => Some(Picker::character(c, &ctx.character)),
            MirrorSlot::Kit(slot) => Some(Picker::new(
                slot,
                picker::kit_rows(ctx.cat, ctx.garage, slot),
            )),
            MirrorSlot::TryOn => None,
        };
    }

    /// The look the rider shows now (spec 4C): the character with the
    /// picker's focused free choice, and the worn kit's models with the
    /// focused kit row in its slot (the live preview).
    pub fn rider_look(&self, ctx: &Ctx) -> (ocw_core::Character, Vec<String>) {
        let mut character = ctx.character;
        let mut kit: Vec<(Slot, String)> = Slot::KIT
            .into_iter()
            .filter_map(|s| {
                let p = ctx.garage.kit_on(ctx.cat, s)?;
                (!p.is_base()).then(|| (s, p.model.clone()))
            })
            .collect();
        if let Some(p) = &self.picker {
            if let Some(f) = p.preview_free() {
                character = f.apply(character);
            } else if let Some(part) = p.preview().and_then(|id| ctx.cat.get(id))
                && part.slot.is_kit()
            {
                kit.retain(|(s, _)| *s != part.slot);
                if !part.is_base() {
                    kit.push((part.slot, part.model.clone()));
                }
            }
        }
        (character, kit.into_iter().map(|(_, m)| m).collect())
    }

    pub fn view(&self) -> View {
        self.cam.view
    }

    /// The camera pose shown now.
    pub fn pose(&self, layout: &Layout) -> Pose {
        let goal = Poses { layout }.pose(self.cam.view, &self.orbit);
        self.cam.pose(goal)
    }

    fn go(&mut self, view: View, layout: &Layout) {
        let now = self.pose(layout);
        self.cam.go(view, now);
    }

    fn stand_bike<'g>(&self, g: &'g Garage) -> Option<&'g Bike> {
        g.bike(self.bikes.get(self.stand)?)
    }

    /// Advances time: the camera move, the orbit's inertia, the swap.
    pub fn tick(&mut self, dt: f32) -> Vec<Out> {
        self.cam.update(dt);
        self.orbit.update(dt);
        let mut out = Vec::new();
        if let Some(s) = &mut self.swap {
            s.t += dt;
            if s.t >= SWAP_S {
                let to = s.to;
                self.swap = None;
                self.stand = to;
                self.focus = to;
                self.hotspot = 0;
                self.changed = true;
                out.push(Out::Line(format!("bikes.select {}", self.bikes[to])));
            }
        }
        out
    }

    /// The order ← / → cycle through: the stand's bike, then the hooks.
    fn cycle_order(&self) -> Vec<usize> {
        std::iter::once(self.stand)
            .chain((0..self.bikes.len()).filter(|i| *i != self.stand))
            .collect()
    }

    fn cycle(&mut self, d: isize, layout: &Layout) {
        let order = self.cycle_order();
        let at = order.iter().position(|i| *i == self.focus).unwrap_or(0) as isize;
        self.focus = order[(at + d).rem_euclid(order.len() as isize) as usize];
        self.plus = false;
        self.focus_bike(self.focus, layout);
    }

    fn focus_bike(&mut self, i: usize, layout: &Layout) {
        self.focus = i;
        if i == self.stand {
            self.zone = Zone::Stand;
            self.go(View::Overview, layout);
        } else {
            self.zone = Zone::Wall;
            self.go(View::Wall(i), layout);
        }
    }

    fn enter_zone(&mut self, z: Zone, layout: &Layout) {
        self.caption = None;
        self.zone = z;
        match z {
            Zone::Wall => {
                if self.focus == self.stand {
                    self.focus = self.cycle_order().get(1).copied().unwrap_or(self.stand);
                }
                let v = if self.focus == self.stand {
                    View::Overview
                } else {
                    View::Wall(self.focus)
                };
                self.go(v, layout);
            }
            Zone::Stand => {
                self.focus = self.stand;
                self.go(View::Overview, layout);
            }
            Zone::Bench => {
                self.caption = Some(
                    "The bench: buying bikes and spare parts comes with the bench catalogue (spec 4D)."
                        .into(),
                );
                self.go(View::Bench, layout);
            }
            Zone::Mirror => {
                self.caption = Some(
                    "The changing corner: Confirm to dress your rider – body, skin, face, hair, glasses (free) and kit (hearts) – and to try it on the bike."
                        .into(),
                );
                self.go(View::Mirror, layout);
            }
        }
    }

    fn start_swap(&mut self, to: usize, layout: &Layout) {
        if to == self.stand || to >= self.bikes.len() {
            return;
        }
        self.swap = Some(Swap {
            from: self.stand,
            to,
            t: 0.0,
        });
        self.zone = Zone::Stand;
        self.go(View::Overview, layout);
    }

    fn open_stand(&mut self, layout: &Layout) {
        self.zone = Zone::Stand;
        self.focus = self.stand;
        self.caption = None;
        self.go(View::Stand, layout);
    }

    fn open_picker(&mut self, ctx: &Ctx) {
        let Some((slot, anchor)) = ctx.anchors.get(self.hotspot).copied() else {
            return;
        };
        let Some(bike) = self.stand_bike(ctx.garage) else {
            return;
        };
        self.picker = Some(Picker::new(
            slot,
            picker::rows(ctx.cat, ctx.garage, bike, slot),
        ));
        self.go(View::Part(anchor), ctx.layout);
    }

    /// The hotspots' screen positions (`None`: behind the camera).
    pub fn hotspot_points(&self, ctx: &Ctx) -> Vec<Option<Vec2>> {
        let pose = self.pose(ctx.layout);
        ctx.anchors
            .iter()
            .map(|(_, p)| pose.to_screen(ctx.size, *p))
            .collect()
    }

    /// The bikes' screen boxes.
    pub fn bike_rects(&self, ctx: &Ctx) -> Vec<Option<Rect>> {
        let pose = self.pose(ctx.layout);
        ctx.boxes
            .iter()
            .map(|c| nav::bounds(c.iter().map(|p| pose.to_screen(ctx.size, *p))))
            .collect()
    }

    fn zone_rect(&self, ctx: &Ctx, at: Vec3, half: Vec3) -> Option<Rect> {
        let pose = self.pose(ctx.layout);
        let corners = (0..8).map(|i| {
            let s = Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            pose.to_screen(ctx.size, at + half * s)
        });
        nav::bounds(corners)
    }

    /// Applies the actions of one frame.
    pub fn update(&mut self, acts: &[Act], ctx: &Ctx) -> Vec<Out> {
        let mut out = Vec::new();
        for a in acts {
            if self.swap.is_some() {
                // The swap plays out; input waits.
                continue;
            }
            if let Some(o) = self.act(*a, ctx) {
                out.push(o);
            }
        }
        // The picker's rows follow the garage (after a purchase …).
        let bike = self
            .bikes
            .get(self.stand)
            .and_then(|id| ctx.garage.bike(id));
        if let (Some(p), Some(bike)) = (&mut self.picker, bike)
            && p.character.is_none()
        {
            p.refresh(picker::rows(ctx.cat, ctx.garage, bike, p.slot));
        }
        self.hotspot = self.hotspot.min(ctx.anchors.len().saturating_sub(1));
        out
    }

    fn line(&mut self, line: String) -> Option<Out> {
        self.changed = true;
        Some(Out::Line(line))
    }

    fn leave(&mut self, then: Option<String>) -> Option<Out> {
        Some(Out::Leave {
            apply: self.changed,
            then,
        })
    }

    fn act(&mut self, a: Act, ctx: &Ctx) -> Option<Out> {
        let l = ctx.layout;
        if a == Act::ResetView {
            self.orbit = Orbit::default();
            return None;
        }
        // The picker and its dialog.
        if let Some(p) = &mut self.picker {
            if let Some(d) = &mut p.dialog {
                let n = d.buttons.len();
                match a {
                    Act::Up | Act::Left => d.focus = (d.focus + n - 1) % n,
                    Act::Down | Act::Right => d.focus = (d.focus + 1) % n,
                    Act::Button(i) if i < n => {
                        d.focus = i;
                        return self.press_dialog();
                    }
                    Act::Confirm => return self.press_dialog(),
                    Act::Back | Act::BackButton => p.dialog = None,
                    _ => {}
                }
                return None;
            }
            match a {
                Act::Up => p.step(-1),
                Act::Down => p.step(1),
                Act::Row(i) if i < p.rows.len() => p.focus = i,
                Act::RowClick(i) if i < p.rows.len() => {
                    p.focus = i;
                    self.open_dialog(ctx);
                }
                Act::Confirm | Act::Action => {
                    // A free choice applies at once (no dialog, nothing to pay).
                    if let Some(f) = p.preview_free() {
                        return self.line(f.line());
                    }
                    self.open_dialog(ctx)
                }
                Act::Compare => p.toggle_pin(),
                Act::SlotPrev | Act::SlotNext if self.dressing() => {
                    let d = if a == Act::SlotNext { 1 } else { -1 };
                    self.dress.step(d);
                    if self.dress.slot() == MirrorSlot::TryOn {
                        self.dress.step(d);
                    }
                    self.open_mirror_picker(ctx);
                }
                Act::SlotPrev | Act::SlotNext => {
                    let n = ctx.anchors.len().max(1);
                    let d = if a == Act::SlotNext { 1 } else { n - 1 };
                    self.hotspot = (self.hotspot + d) % n;
                    self.open_picker(ctx);
                }
                Act::Orbit(d, _) if self.dressing() => self.dress.yaw += d.x,
                Act::Back | Act::BackButton => {
                    // Leaving the picker restores the equipped part (the
                    // look: what the rider wears).
                    self.picker = None;
                    if self.dressing() {
                        let v = self.dress_view();
                        self.go(v, l);
                    } else {
                        self.go(View::Stand, l);
                    }
                }
                _ => {}
            }
            return None;
        }
        match (self.view(), a) {
            // The character editor (spec 4C).
            (View::Dress { .. }, Act::Back | Act::BackButton) => self.go(View::Mirror, l),
            (View::TryOn, Act::Back | Act::BackButton) => {
                let v = self.dress_view();
                self.go(v, l);
            }
            (View::TryOn, Act::Orbit(d, _)) => self.dress.yaw += d.x,
            (View::TryOn, _) => {}
            (View::Dress { .. }, Act::Left | Act::Right | Act::Up | Act::Down) => {
                let dir = match a {
                    Act::Left => Dir::Left,
                    Act::Right => Dir::Right,
                    Act::Up => Dir::Up,
                    _ => Dir::Down,
                };
                if let Some(i) =
                    nav::next_in_direction(&self.rider_points(ctx), self.dress.hotspot, dir)
                {
                    self.dress.focus(i);
                }
            }
            (View::Dress { .. }, Act::SlotPrev) => self.dress.step(-1),
            (View::Dress { .. }, Act::SlotNext) => self.dress.step(1),
            (View::Dress { .. }, Act::Confirm) => self.dress_confirm(ctx),
            (View::Dress { .. }, Act::Point(at)) => {
                if let Some(i) = nav::hit(&self.rider_points(ctx), at, 0.0) {
                    self.dress.focus(i);
                }
            }
            (View::Dress { .. }, Act::Activate(at)) => {
                if let Some(i) = nav::hit(&self.rider_points(ctx), at, 0.0) {
                    self.dress.focus(i);
                    self.dress_confirm(ctx);
                }
            }
            (View::Dress { .. }, Act::Orbit(d, _)) => self.dress.yaw += d.x,
            (View::Dress { .. }, _) => {}
            (_, Act::BackButton) | (_, Act::Back) => match self.view() {
                View::Overview => return self.leave(None),
                View::Stand
                | View::Part(_)
                | View::Wall(_)
                | View::Bench
                | View::Mirror
                | View::Dress { .. }
                | View::TryOn => {
                    self.caption = None;
                    self.focus = self.stand;
                    self.zone = Zone::Stand;
                    self.go(View::Overview, l);
                }
            },
            // The stand view: hotspots and the orbit.
            (View::Stand, Act::Left | Act::Right | Act::Up | Act::Down) => {
                let dir = match a {
                    Act::Left => Dir::Left,
                    Act::Right => Dir::Right,
                    Act::Up => Dir::Up,
                    _ => Dir::Down,
                };
                if let Some(i) =
                    nav::next_in_direction(&self.hotspot_points(ctx), self.hotspot, dir)
                {
                    self.hotspot = i;
                }
            }
            (View::Stand, Act::SlotPrev | Act::SlotNext) => {
                let n = ctx.anchors.len().max(1);
                let d = if a == Act::SlotNext { 1 } else { n - 1 };
                self.hotspot = (self.hotspot + d) % n;
            }
            (View::Stand, Act::Confirm) => self.open_picker(ctx),
            (View::Stand, Act::Point(at)) => {
                if let Some(i) = nav::hit(&self.hotspot_points(ctx), at, 0.0) {
                    self.hotspot = i;
                }
            }
            (View::Stand, Act::Activate(at)) => {
                if let Some(i) = nav::hit(&self.hotspot_points(ctx), at, 0.0) {
                    self.hotspot = i;
                    self.open_picker(ctx);
                }
            }
            (View::Stand, Act::Orbit(d, dt)) => self.orbit.turn(d, dt),
            (View::Stand, Act::Zoom(z)) => self.orbit.zoom(z),
            // The room: bikes and zones.
            (View::Overview | View::Wall(_), Act::Left) => self.cycle(-1, l),
            (View::Overview | View::Wall(_), Act::Right) => self.cycle(1, l),
            (_, Act::Swipe(d)) if !matches!(self.view(), View::Stand | View::Part(_)) => {
                self.cycle(if d > 0 { -1 } else { 1 }, l)
            }
            (_, Act::Up) => {
                let z = match self.zone {
                    Zone::Wall | Zone::Stand => Zone::Wall,
                    Zone::Bench => Zone::Stand,
                    Zone::Mirror => Zone::Bench,
                };
                self.enter_zone(z, l);
            }
            (_, Act::Down) => {
                let z = match self.zone {
                    Zone::Wall => Zone::Stand,
                    Zone::Stand => Zone::Bench,
                    Zone::Bench | Zone::Mirror => Zone::Mirror,
                };
                self.enter_zone(z, l);
            }
            (View::Overview | View::Wall(_), Act::Confirm) => {
                if self.plus {
                    self.caption =
                        Some("More bikes: from the bench catalogue (spec 4D, later).".into());
                } else if self.focus == self.stand {
                    self.open_stand(l);
                } else {
                    self.start_swap(self.focus, l);
                }
            }
            (View::Mirror, Act::Confirm) => {
                self.caption = None;
                self.zone = Zone::Mirror;
                let v = self.dress_view();
                self.go(v, l);
            }
            (View::Overview | View::Wall(_) | View::Bench | View::Mirror, Act::Point(at)) => {
                if let Some(i) = nav::hit_rect(&self.bike_rects(ctx), at) {
                    self.focus = i;
                    self.plus = false;
                } else if self.plus_rect(ctx).is_some_and(|r| inside(r, at)) {
                    self.plus = true;
                }
            }
            (View::Overview | View::Wall(_) | View::Bench | View::Mirror, Act::Activate(at)) => {
                if let Some(i) = nav::hit_rect(&self.bike_rects(ctx), at) {
                    self.plus = false;
                    if i == self.stand {
                        self.open_stand(l);
                    } else {
                        self.focus = i;
                        self.start_swap(i, l);
                    }
                } else if self.plus_rect(ctx).is_some_and(|r| inside(r, at)) {
                    self.plus = true;
                    self.caption =
                        Some("More bikes: from the bench catalogue (spec 4D, later).".into());
                } else if self
                    .zone_rect(ctx, l.bench, Vec3::new(0.5, 0.6, 1.0))
                    .is_some_and(|r| inside(r, at))
                {
                    self.enter_zone(Zone::Bench, l);
                } else if self
                    .zone_rect(ctx, l.mirror, Vec3::new(0.3, 0.9, 0.6))
                    .is_some_and(|r| inside(r, at))
                {
                    self.enter_zone(Zone::Mirror, l);
                }
            }
            _ => {}
        }
        None
    }

    /// Confirm in the editor: the slot's picker, or *Try on the bike*.
    fn dress_confirm(&mut self, ctx: &Ctx) {
        if self.dress.slot() == MirrorSlot::TryOn {
            self.picker = None;
            self.go(View::TryOn, ctx.layout);
        } else {
            self.open_mirror_picker(ctx);
        }
    }

    fn plus_rect(&self, ctx: &Ctx) -> Option<Rect> {
        let h = ctx.layout.hooks.last()?;
        self.zone_rect(ctx, h.bike.0, Vec3::new(0.4, 0.6, 0.3))
    }

    fn open_dialog(&mut self, ctx: &Ctx) {
        let Some(bike) = self.stand_bike(ctx.garage) else {
            return;
        };
        let Some(p) = &mut self.picker else { return };
        let Some(row) = p.focused().cloned() else {
            return;
        };
        p.dialog = picker::dialog(ctx.cat, ctx.garage, bike, p.slot, &row, ctx.balance);
        if p.dialog.is_none()
            && let Kind::Shop = row.kind
        {
            self.caption = Some("Not enough hearts yet: they are earned by riding.".into());
        }
    }

    fn press_dialog(&mut self) -> Option<Out> {
        let p = self.picker.as_mut()?;
        let d = p.dialog.take()?;
        let line = d.buttons.get(d.focus)?.line.clone()?;
        self.line(line)
    }

    /// The parts' models and paint to show on the stand bike: what is on
    /// it, with the picker's focused row in its slot (the live preview).
    pub fn stand_look(&self, ctx: &Ctx) -> (Vec<String>, Option<Rgb>) {
        let Some(bike) = self.stand_bike(ctx.garage) else {
            return (Vec::new(), None);
        };
        let mut models: Vec<(Slot, String)> = Slot::ALL
            .into_iter()
            .filter_map(|s| {
                let p = ctx.garage.part_on(ctx.cat, bike, s)?;
                Some((s, p.model.clone()))
            })
            .collect();
        let mut paint = bike.paint();
        if let Some(p) = &self.picker {
            if let Some(c) = p.preview_paint() {
                paint = c;
            } else if let Some(part) = p.preview().and_then(|id| ctx.cat.get(id)) {
                models.retain(|(s, _)| *s != part.slot);
                models.push((part.slot, part.model.clone()));
            }
        }
        (models.into_iter().map(|(_, m)| m).collect(), paint)
    }
}

fn inside(r: Rect, at: Vec2) -> bool {
    at.x >= r.0.x && at.x <= r.1.x && at.y >= r.0.y && at.y <= r.1.y
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::workshop::wallet::Wallet;
    use ocw_core::ProfileId;

    pub(crate) struct World {
        pub wallet: Wallet,
        pub layout: Layout,
        pub anchors: Vec<(Slot, Vec3)>,
        pub spots: Vec<Vec3>,
    }

    impl World {
        pub fn new() -> Self {
            let mut wallet = Wallet::in_memory();
            wallet.now_unix = Some(1_794_000_000);
            wallet.garage("Ann", ProfileId::Road);
            let layout = ocw_assets::workshop::layout(6);
            // A ring of anchors around the stand bike's middle.
            let c = layout.stand_bike.0 + Vec3::Y * 0.55;
            let anchors = Slot::ALL
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let a = i as f32 / Slot::ALL.len() as f32 * std::f32::consts::TAU;
                    (*s, c + Vec3::new(a.cos() * 0.5, a.sin() * 0.3, 0.0))
                })
                .collect();
            let spots = layout.rider_hotspots(1.0).to_vec();
            Self {
                wallet,
                layout,
                anchors,
                spots,
            }
        }

        pub fn boxes(&self, w: &Workshop) -> Vec<[Vec3; 8]> {
            (0..w.bikes.len())
                .map(|i| {
                    let c = if i == w.stand {
                        self.layout.stand_bike.0 + Vec3::Y * 0.5
                    } else {
                        self.layout.hooks[i].bike.0
                    };
                    let h = Vec3::new(0.35, 0.45, 0.35);
                    std::array::from_fn(|k| {
                        c + h * Vec3::new(
                            if k & 1 == 0 { -1.0 } else { 1.0 },
                            if k & 2 == 0 { -1.0 } else { 1.0 },
                            if k & 4 == 0 { -1.0 } else { 1.0 },
                        )
                    })
                })
                .collect()
        }

        pub fn open(&self) -> Workshop {
            Workshop::open(self.wallet.get("Ann").unwrap(), &self.layout)
        }

        /// One frame: the actions, then 1/60 s.
        pub fn step(&self, w: &mut Workshop, acts: &[Act]) -> Vec<Out> {
            let boxes = self.boxes(w);
            let ctx = Ctx {
                cat: ocw_assets::shipped::catalogue(),
                garage: self.wallet.get("Ann").unwrap(),
                balance: self.wallet.balance("Ann"),
                layout: &self.layout,
                size: Vec2::new(1920.0, 1080.0),
                anchors: &self.anchors,
                boxes: &boxes,
                character: ocw_core::Character::default(),
                rider_spots: &self.spots,
            };
            let mut out = w.update(acts, &ctx);
            out.extend(w.tick(1.0 / 60.0));
            out
        }

        /// Frames until the camera stopped (and a swap ended).
        pub fn settle(&self, w: &mut Workshop) -> Vec<Out> {
            let mut out = Vec::new();
            for _ in 0..120 {
                out.extend(self.step(w, &[]));
            }
            out
        }

        pub fn ctx_points(&self, w: &Workshop) -> (Vec<Option<Vec2>>, Vec<Option<Rect>>) {
            let boxes = self.boxes(w);
            let ctx = Ctx {
                cat: ocw_assets::shipped::catalogue(),
                garage: self.wallet.get("Ann").unwrap(),
                balance: self.wallet.balance("Ann"),
                layout: &self.layout,
                size: Vec2::new(1920.0, 1080.0),
                anchors: &self.anchors,
                boxes: &boxes,
                character: ocw_core::Character::default(),
                rider_spots: &self.spots,
            };
            (w.hotspot_points(&ctx), w.bike_rects(&ctx))
        }
    }

    #[test]
    fn left_right_cycle_the_bikes_stand_first_and_wrap() {
        let world = World::new();
        let mut w = world.open();
        assert_eq!(w.bikes.len(), 6);
        assert_eq!((w.stand, w.focus, w.view()), (0, 0, View::Overview));
        world.step(&mut w, &[Act::Right]);
        assert_eq!((w.focus, w.view()), (1, View::Wall(1)));
        for _ in 0..4 {
            world.step(&mut w, &[Act::Right]);
        }
        assert_eq!(w.focus, 5);
        world.step(&mut w, &[Act::Right]);
        assert_eq!(
            (w.focus, w.view()),
            (0, View::Overview),
            "wraps to the stand"
        );
        world.step(&mut w, &[Act::Left]);
        assert_eq!(w.focus, 5);
        // Touch: a swipe to the left shows the next bike.
        world.step(&mut w, &[Act::Swipe(-1)]);
        assert_eq!(w.focus, 0);
    }

    #[test]
    fn confirm_swaps_a_wall_bike_onto_the_stand() {
        let world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Right, Act::Right]);
        assert_eq!(w.focus, 2);
        let out = world.step(&mut w, &[Act::Confirm]);
        assert!(out.is_empty());
        let s = w.swap.expect("swapping");
        assert_eq!((s.from, s.to), (0, 2));
        // Input waits during the swap.
        world.step(&mut w, &[Act::Right, Act::Back]);
        assert!(w.swap.is_some());
        let out = world.settle(&mut w);
        assert_eq!(out, vec![Out::Line(format!("bikes.select {}", w.bikes[2]))]);
        assert_eq!((w.stand, w.focus, w.view()), (2, 2, View::Overview));
        // Now the old stand bike is on the wall: ← / → reach it.
        let order = w.cycle_order();
        assert_eq!(order[0], 2);
        assert!(order.contains(&0));
        // Confirm on the stand bike: the stand view.
        world.step(&mut w, &[Act::Confirm]);
        assert_eq!(w.view(), View::Stand);
    }

    #[test]
    fn zones_up_and_down_and_back_out() {
        let world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Up]);
        assert_eq!((w.zone, w.view()), (Zone::Wall, View::Wall(1)));
        world.step(&mut w, &[Act::Down]);
        assert_eq!((w.zone, w.view()), (Zone::Stand, View::Overview));
        world.step(&mut w, &[Act::Down]);
        assert_eq!((w.zone, w.view()), (Zone::Bench, View::Bench));
        assert!(w.caption.as_deref().unwrap().contains("bench"));
        world.step(&mut w, &[Act::Down]);
        assert_eq!((w.zone, w.view()), (Zone::Mirror, View::Mirror));
        // Confirm at the mirror: the character editor (W4).
        let out = world.step(&mut w, &[Act::Confirm]);
        assert!(out.is_empty());
        assert_eq!(w.view(), View::Dress { front: false });
        // Back: the corner; Back from a zone: the overview; Back there: leave.
        world.step(&mut w, &[Act::Back]);
        assert_eq!(w.view(), View::Mirror);
        world.step(&mut w, &[Act::Back]);
        assert_eq!(w.view(), View::Overview);
        let out = world.step(&mut w, &[Act::Back]);
        assert_eq!(
            out,
            vec![Out::Leave {
                apply: false,
                then: None
            }]
        );
    }

    #[test]
    fn changes_apply_only_when_leaving() {
        let world = World::new();
        let mut w = world.open();
        // Nothing changed: leaving applies nothing.
        assert_eq!(
            world.step(&mut w, &[Act::Back]),
            vec![Out::Leave {
                apply: false,
                then: None
            }]
        );
        let mut w = world.open();
        world.step(&mut w, &[Act::Right, Act::Confirm]);
        let out = world.settle(&mut w);
        // The swap only selects the bike in the garage: no BikeChange yet.
        assert!(
            out.iter()
                .all(|o| matches!(o, Out::Line(l) if l.starts_with("bikes.select")))
        );
        assert_eq!(
            world.step(&mut w, &[Act::Back]),
            vec![Out::Leave {
                apply: true,
                then: None
            }]
        );
    }

    #[test]
    fn stand_hotspots_by_direction_slot_keys_and_pointer() {
        let world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Confirm]);
        world.settle(&mut w);
        assert_eq!(w.view(), View::Stand);
        let n = world.anchors.len();
        // LB / RB step through the slot list, wrapping.
        world.step(&mut w, &[Act::SlotPrev]);
        assert_eq!(w.hotspot, n - 1);
        world.step(&mut w, &[Act::SlotNext, Act::SlotNext]);
        assert_eq!(w.hotspot, 1);
        // Directions move in screen space: to the nearest one that way.
        let (pts, _) = world.ctx_points(&w);
        let from = w.hotspot;
        world.step(&mut w, &[Act::Left]);
        let to = w.hotspot;
        assert_ne!(to, from);
        assert!(pts[to].unwrap().x < pts[from].unwrap().x);
        // Mouse hover focuses, a click opens the picker on that slot.
        let (pts, _) = world.ctx_points(&w);
        let target = (from + 3) % n;
        world.step(
            &mut w,
            &[Act::Point(pts[target].unwrap() + Vec2::new(5.0, 0.0))],
        );
        assert_eq!(w.hotspot, target);
        world.step(&mut w, &[Act::Activate(pts[target].unwrap())]);
        let p = w.picker.as_ref().expect("picker open");
        assert_eq!(p.slot, world.anchors[target].0);
        assert!(matches!(w.view(), View::Part(_)));
        // Back closes the picker: the stand view again.
        world.step(&mut w, &[Act::Back]);
        assert!(w.picker.is_none());
        assert_eq!(w.view(), View::Stand);
        // Orbit and zoom only move the camera.
        let before = w.orbit;
        world.step(
            &mut w,
            &[Act::Orbit(Vec2::new(0.2, 0.1), 0.0), Act::Zoom(1.0)],
        );
        assert!(w.orbit.yaw > before.yaw && w.orbit.dist < before.dist);
        world.step(&mut w, &[Act::ResetView]);
        assert_eq!(w.orbit, Orbit::default());
    }

    #[test]
    fn a_click_on_a_wall_bike_swaps_it_and_on_the_stand_bike_opens_the_stand() {
        let world = World::new();
        let mut w = world.open();
        world.settle(&mut w);
        let (_, rects) = world.ctx_points(&w);
        let r = rects[3].expect("bike 3 in view");
        let centre = (r.0 + r.1) / 2.0;
        world.step(&mut w, &[Act::Point(centre)]);
        assert_eq!(w.focus, 3, "hover focuses");
        world.step(&mut w, &[Act::Activate(centre)]);
        assert_eq!(w.swap.map(|s| s.to), Some(3));
        world.settle(&mut w);
        let (_, rects) = world.ctx_points(&w);
        let r = rects[3].unwrap();
        world.step(&mut w, &[Act::Activate((r.0 + r.1) / 2.0)]);
        assert_eq!(w.view(), View::Stand);
    }

    #[test]
    fn picker_preview_buy_and_dialog_flow() {
        let mut world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Confirm]);
        world.settle(&mut w);
        let tyres = world
            .anchors
            .iter()
            .position(|(s, _)| *s == Slot::Tyres)
            .unwrap();
        w.hotspot = tyres;
        world.step(&mut w, &[Act::Confirm]);
        let base_look = {
            let boxes = world.boxes(&w);
            let ctx = Ctx {
                cat: ocw_assets::shipped::catalogue(),
                garage: world.wallet.get("Ann").unwrap(),
                balance: 500,
                layout: &world.layout,
                size: Vec2::new(1920.0, 1080.0),
                anchors: &world.anchors,
                boxes: &boxes,
                character: ocw_core::Character::default(),
                rider_spots: &world.spots,
            };
            w.stand_look(&ctx)
        };
        // Down: the first shop row, previewed on the bike.
        world.step(&mut w, &[Act::Down]);
        let p = w.picker.as_ref().unwrap();
        let part = p.preview().unwrap().to_string();
        let boxes = world.boxes(&w);
        let ctx = Ctx {
            cat: ocw_assets::shipped::catalogue(),
            garage: world.wallet.get("Ann").unwrap(),
            balance: 500,
            layout: &world.layout,
            size: Vec2::new(1920.0, 1080.0),
            anchors: &world.anchors,
            boxes: &boxes,
            character: ocw_core::Character::default(),
            rider_spots: &world.spots,
        };
        let model = ocw_assets::shipped::catalogue()
            .get(&part)
            .unwrap()
            .model
            .clone();
        let (models, _) = w.stand_look(&ctx);
        assert!(
            models.contains(&model) && !base_look.0.contains(&model),
            "live preview"
        );
        // Confirm: the buy dialog with Cancel focused; Confirm on Cancel
        // does nothing; Up + Confirm buys.
        world.step(&mut w, &[Act::Confirm]);
        let d = w.picker.as_ref().unwrap().dialog.clone().expect("dialog");
        assert_eq!(d.buttons[d.focus].label, "Cancel");
        assert!(world.step(&mut w, &[Act::Confirm]).is_empty());
        assert!(w.picker.as_ref().unwrap().dialog.is_none());
        world.step(&mut w, &[Act::Action]);
        let out = world.step(&mut w, &[Act::Up, Act::Confirm]);
        assert_eq!(
            out,
            vec![Out::Line(format!("workshop.buy {part} --bike road"))]
        );
        world
            .wallet
            .buy(ocw_assets::shipped::catalogue(), "Ann", "road", &part)
            .unwrap();
        world.step(&mut w, &[]);
        assert!(w.changed);
        // Compare: pin, move, unpin.
        world.step(&mut w, &[Act::Compare, Act::Down]);
        assert!(w.picker.as_ref().unwrap().pinned.is_some());
        world.step(&mut w, &[Act::Compare]);
        assert!(w.picker.as_ref().unwrap().pinned.is_none());
        // LB / RB in the picker: the next slot's picker.
        world.step(&mut w, &[Act::SlotNext]);
        assert_eq!(w.picker.as_ref().unwrap().slot, world.anchors[tyres + 1].0);
        // Back twice: picker closed, overview; leaving applies.
        world.step(&mut w, &[Act::Back, Act::Back]);
        assert_eq!(w.view(), View::Overview);
        assert_eq!(
            world.step(&mut w, &[Act::Back]),
            vec![Out::Leave {
                apply: true,
                then: None
            }]
        );
    }
}
