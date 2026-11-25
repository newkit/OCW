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
//! out again. The jersey designer's pages (spec §14 Phase 8 T6) are slots
//! too: ↑ / ↓ pick a field, ← / → change it on a draft shown live,
//! Confirm applies the design (a `kit.design` line).

use bevy::math::{Vec2, Vec3};
use ocw_assets::parts::{Catalogue, Slot};
use ocw_assets::workshop::Layout;
use ocw_core::Rgb;

use super::bench::{self, BenchPage};
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
    /// The rider's whole look now (the designer's start, Phase 8 T6).
    pub look: ocw_core::RiderLook,
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
    /// The rider's height against the default (the face close-up's head).
    pub rider_scale: f32,
}

impl Workshop {
    /// Opens the workshop with the garage's bikes, the active one on the
    /// stand, in the overview.
    pub fn open(garage: &Garage, layout: &Layout) -> Self {
        let bikes: Vec<String> = garage.bikes.iter().map(|b| b.id.clone()).collect();
        let stand = bikes.iter().position(|b| *b == garage.active).unwrap_or(0);
        let orbit = Orbit::default();
        let at = Poses { layout, scale: 1.0 }.pose(View::Overview, &orbit);
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
            rider_scale: 1.0,
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
            face: false,
        }
    }

    /// The bare head at the mirror (spec §14 Phase 4, 4N Q28): while a
    /// hair, facial-hair, face or glasses slot is chosen in the editor
    /// (not the helmet's and the cap's: they show the helmet), the helmet
    /// hangs on its hook.
    pub fn helmet_off(&self) -> bool {
        use picker::CharSlot as C;
        matches!(self.view(), View::Dress { .. })
            && matches!(
                self.dress.slot(),
                MirrorSlot::Char(
                    C::Hair
                        | C::HairColour
                        | C::Moustache
                        | C::Beard
                        | C::Age
                        | C::Glasses
                        | C::Skin
                )
            )
    }

    /// The editor's view now: the face close-up while a picker of a head
    /// slot is open (spec §14 Phase 4, 4N), else the whole rider.
    fn dress_view_now(&self) -> View {
        View::Dress {
            front: self.front_view,
            face: self.picker.is_some() && self.dress.slot().frames_face(),
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
            MirrorSlot::Kit(slot) => Some(Picker::kit(
                slot,
                picker::kit_rows(ctx.cat, ctx.garage, slot),
                ctx.look,
            )),
            MirrorSlot::Design(page) => Some(Picker::designer(page, ctx.look)),
            MirrorSlot::TryOn => None,
        };
    }

    /// The jersey designer's draft look (shown on the rider), if open.
    pub fn design_preview(&self) -> Option<ocw_core::RiderLook> {
        self.picker.as_ref()?.design.map(|(_, l)| l)
    }

    /// The look the rider shows now (spec 4C): the character with the
    /// picker's focused free choice, and the worn kit's models with the
    /// focused kit row in its slot (the live preview).
    pub fn rider_look(&self, ctx: &Ctx) -> (ocw_core::Character, Vec<String>) {
        self.rider_look_with(ctx, self.picker.as_ref().and_then(|p| p.focused()))
    }

    /// [`Self::rider_look`] with the picker's row `row` instead of the
    /// focused one (the picture of each tile, spec §14 Phase 4, 4N).
    pub fn rider_look_with(
        &self,
        ctx: &Ctx,
        row: Option<&picker::Row>,
    ) -> (ocw_core::Character, Vec<String>) {
        let mut character = ctx.character;
        let mut kit: Vec<(Slot, String)> = Slot::KIT
            .into_iter()
            .filter_map(|s| {
                let p = ctx.garage.kit_on(ctx.cat, s)?;
                (!p.is_base()).then(|| (s, p.model.clone()))
            })
            .collect();
        if let Some(row) = row {
            if let Kind::Free(f) = row.kind {
                character = f.apply(character);
            } else if let Some(part) = row.part.as_deref().and_then(|id| ctx.cat.get(id))
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
        let goal = Poses {
            layout,
            scale: self.rider_scale,
        }
        .pose(self.cam.view, &self.orbit);
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
                    "The bench: Confirm for the catalogue – framesets as new bikes, complete builds, the spare parts on the shelf, selling a bike."
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
        self.rider_scale = ctx.character.scale();
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
        // The hooks follow the garage (a bike bought or sold at the bench).
        self.sync_bikes(ctx.garage);
        // The picker's rows follow the garage (after a purchase …).
        let bike = self
            .bikes
            .get(self.stand)
            .and_then(|id| ctx.garage.bike(id));
        if let Some(p) = &mut self.picker
            && let Some(page) = p.bench
        {
            p.refresh(bench::rows(ctx.cat, ctx.garage, page));
        } else if let (Some(p), Some(bike)) = (&mut self.picker, bike)
            && p.character.is_none()
            && !p.is_designer()
        {
            p.refresh(picker::rows(ctx.cat, ctx.garage, bike, p.slot));
        }
        self.hotspot = self.hotspot.min(ctx.anchors.len().saturating_sub(1));
        // The editor frames the head while a head slot's picker is open.
        if matches!(self.view(), View::Dress { .. }) {
            let want = self.dress_view_now();
            if want != self.view() {
                self.go(want, ctx.layout);
            }
        }
        out
    }

    /// The hooks after the garage changed (spec 4D): a bike bought hangs on
    /// a new hook at the end of the row (where the "+" was), a sold one's
    /// hook goes; the stand keeps its bike (the garage's active one if its
    /// own was sold).
    fn sync_bikes(&mut self, g: &Garage) {
        let ids: Vec<String> = g.bikes.iter().map(|b| b.id.clone()).collect();
        if ids == self.bikes || self.swap.is_some() {
            return;
        }
        let on_stand = self.bikes.get(self.stand).cloned();
        let focused = self.bikes.get(self.focus).cloned();
        let mut list: Vec<String> = self
            .bikes
            .iter()
            .filter(|b| ids.contains(b))
            .cloned()
            .collect();
        let new: Vec<String> = ids.iter().filter(|b| !list.contains(b)).cloned().collect();
        list.extend(new.iter().cloned());
        self.bikes = list;
        let at = |id: &Option<String>| {
            id.as_ref()
                .and_then(|i| self.bikes.iter().position(|b| b == i))
        };
        self.stand = at(&on_stand)
            .or_else(|| self.bikes.iter().position(|b| *b == g.active))
            .unwrap_or(0);
        self.focus = at(&focused).unwrap_or(self.stand);
        if let Some(id) = new.last()
            && let Some(b) = g.bike(id)
        {
            self.caption = Some(format!(
                "The {} hangs on a new hook – Confirm on it to put it on the stand and build it up.",
                b.label()
            ));
        }
        self.hotspot = 0;
    }

    /// Opens a page of the bench catalogue (spec 4D).
    fn open_bench(&mut self, page: BenchPage, ctx: &Ctx) {
        let pinned = self.picker.as_ref().and_then(|p| p.pinned.clone());
        let mut p = Picker::bench(page, bench::rows(ctx.cat, ctx.garage, page));
        p.pinned = pinned;
        self.picker = Some(p);
    }

    /// The bench's catalogue from the bench, or from the "+" hook (the
    /// camera on the hook, where the new bike will hang).
    fn open_catalogue(&mut self, from_plus: bool, ctx: &Ctx) {
        self.caption = None;
        if from_plus {
            self.zone = Zone::Wall;
            self.go(View::Wall(self.bikes.len()), ctx.layout);
        } else {
            self.zone = Zone::Bench;
            self.go(View::Bench, ctx.layout);
        }
        self.open_bench(BenchPage::Top, ctx);
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
        if let Act::Catalogue { plus } = a {
            self.picker = None;
            self.plus = plus;
            self.open_catalogue(plus, ctx);
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
            // The bench catalogue (spec 4D): pages, then the dialogs.
            if let Some(page) = p.bench {
                match a {
                    Act::Up => p.step(-1),
                    Act::Down => p.step(1),
                    Act::Row(i) if i < p.rows.len() => p.focus = i,
                    Act::Compare => p.toggle_pin(),
                    Act::Confirm | Act::Action | Act::RowClick(_) => {
                        if let Act::RowClick(i) = a {
                            if i >= p.rows.len() {
                                return None;
                            }
                            p.focus = i;
                        }
                        if let Some(Kind::Page(next)) = p.focused().map(|r| r.kind.clone()) {
                            self.open_bench(next, ctx);
                        } else {
                            self.open_dialog(ctx);
                        }
                    }
                    Act::Back | Act::BackButton => match page.parent() {
                        Some(up) => {
                            self.open_bench(up, ctx);
                            if let Some(p) = &mut self.picker {
                                // The row of the page we came from.
                                p.focus = p
                                    .rows
                                    .iter()
                                    .position(|r| r.kind == Kind::Page(page))
                                    .unwrap_or(0);
                            }
                        }
                        None => self.picker = None,
                    },
                    _ => {}
                }
                return None;
            }
            // A kit picker's swatch row above its tiles (4N Q30 (a)): the
            // arrows move between the row and the tiles and along each,
            // the selected colour shows live on the rider; Confirm on the
            // row applies it (free), the pointer's hover selects, a click
            // applies.
            if p.swatches.is_some() {
                use ocw_client::part_tiles::Dir;
                let dir = match a {
                    Act::Up => Some(Dir::Up),
                    Act::Down => Some(Dir::Down),
                    Act::Left => Some(Dir::Left),
                    Act::Right => Some(Dir::Right),
                    _ => None,
                };
                let apply = |w: &mut Self| {
                    let draft = w.picker.as_ref()?.design?.1;
                    w.caption =
                        Some("Colour applied – free, looks only, and seen by everyone.".into());
                    w.line(super::designer::line(&draft))
                };
                match a {
                    _ if dir.is_some() => {
                        p.step_swatched(dir.expect("checked"));
                        return None;
                    }
                    Act::Swatch(i) => {
                        p.focus_swatch(i);
                        return None;
                    }
                    Act::SwatchClick(i) => {
                        p.focus_swatch(i);
                        return apply(self);
                    }
                    Act::Confirm | Act::Action if p.on_swatches() => return apply(self),
                    Act::Row(i) if i < p.rows.len() => {
                        p.focus_tile(i);
                        return None;
                    }
                    Act::RowClick(i) if i < p.rows.len() => p.focus_tile(i),
                    _ => {}
                }
            }
            // The jersey designer: ← / → change the focused field.
            if let Some((_, draft)) = p.design.filter(|_| p.swatches.is_none()) {
                let on_field = p.design_field().is_some();
                match a {
                    Act::Left if on_field => {
                        p.step_design(-1);
                        return None;
                    }
                    Act::Right if on_field => {
                        p.step_design(1);
                        return None;
                    }
                    Act::RowClick(i)
                        if i < p.rows.len() && matches!(p.rows[i].kind, Kind::Design(_)) =>
                    {
                        p.focus = i;
                        p.step_design(1);
                        return None;
                    }
                    Act::Confirm | Act::Action if on_field => {
                        self.caption = Some(if p.slot.is_kit() {
                            "Colour applied – free, looks only, and seen by everyone.".into()
                        } else {
                            "Design applied – free, and seen by everyone.".into()
                        });
                        return self.line(super::designer::line(&draft));
                    }
                    _ => {}
                }
            }
            match a {
                // The tiles (spec 4B *Tiles*): the focus moves over the
                // grid; the stand bike previews the focused tile.
                Act::Up | Act::Down | Act::Left | Act::Right if p.tiles() => {
                    use ocw_client::part_tiles::Dir;
                    p.step_tiles(match a {
                        Act::Up => Dir::Up,
                        Act::Down => Dir::Down,
                        Act::Left => Dir::Left,
                        _ => Dir::Right,
                    });
                }
                Act::Up => p.step(-1),
                Act::Down => p.step(1),
                Act::Row(i) if i < p.rows.len() => p.focus = i,
                Act::RowClick(i) if i < p.rows.len() => {
                    p.focus = i;
                    // A free choice's tile applies at once, as Confirm.
                    if let Some(f) = p.preview_free() {
                        return self.line(f.line());
                    }
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
            (View::Bench, Act::Confirm) => self.open_catalogue(false, ctx),
            (View::Overview | View::Wall(_), Act::Confirm) => {
                if self.plus {
                    self.open_catalogue(true, ctx);
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
                    self.open_catalogue(true, ctx);
                } else if self
                    .zone_rect(ctx, l.bench, Vec3::new(0.5, 0.6, 1.0))
                    .is_some_and(|r| inside(r, at))
                {
                    if self.view() == View::Bench {
                        self.open_catalogue(false, ctx);
                    } else {
                        self.enter_zone(Zone::Bench, l);
                    }
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
        p.dialog = if p.bench.is_some() {
            bench::dialog(ctx.cat, ctx.garage, Some(bike), &row, ctx.balance)
        } else {
            picker::dialog(ctx.cat, ctx.garage, bike, p.slot, &row, ctx.balance)
        };
        if p.dialog.is_none() && matches!(row.kind, Kind::Shop | Kind::Frameset | Kind::Build(_)) {
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
        // The bench's rows are no preview on the stand bike (a frameset is
        // a new bike).
        if let Some(p) = self.picker.as_ref().filter(|p| p.bench.is_none()) {
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
                look: ocw_core::RiderLook::default(),
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

        /// `f` with the frame's context.
        pub fn with_ctx<T>(&self, w: &Workshop, f: impl FnOnce(&Workshop, &Ctx) -> T) -> T {
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
                look: ocw_core::RiderLook::default(),
                rider_spots: &self.spots,
            };
            f(w, &ctx)
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
                look: ocw_core::RiderLook::default(),
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
        assert_eq!(
            w.view(),
            View::Dress {
                front: false,
                face: false
            }
        );
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

    /// The face close-up at the mirror (spec §14 Phase 4, 4N): a picker
    /// of a head slot (helmet, glasses, hair, beard, age …, and the skin)
    /// frames the head, the body's and the kit's below the head frame the
    /// whole rider; closing the picker goes back to the whole rider.
    #[test]
    fn head_pickers_frame_the_face_in_the_mirror() {
        let world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Down, Act::Down, Act::Down]);
        world.step(&mut w, &[Act::Confirm]);
        world.settle(&mut w);
        for (i, slot) in MirrorSlot::ALL.iter().enumerate() {
            w.dress.slot = i;
            if *slot == MirrorSlot::TryOn {
                continue;
            }
            // The helmet off for the hair, the face and the glasses, on for
            // the helmet's and the cap's own pickers and the rest (Q28).
            let bare = matches!(
                slot,
                MirrorSlot::Char(
                    picker::CharSlot::Glasses
                        | picker::CharSlot::Hair
                        | picker::CharSlot::HairColour
                        | picker::CharSlot::Moustache
                        | picker::CharSlot::Beard
                        | picker::CharSlot::Age
                        | picker::CharSlot::Skin
                )
            );
            assert_eq!(w.helmet_off(), bare, "{slot:?}");
            world.step(&mut w, &[Act::Confirm]);
            assert!(w.picker.is_some(), "{slot:?}");
            assert_eq!(w.helmet_off(), bare, "{slot:?} picker");
            let face = matches!(
                slot,
                MirrorSlot::Kit(Slot::Helmet | Slot::Cap)
                    | MirrorSlot::Char(
                        picker::CharSlot::Glasses
                            | picker::CharSlot::Hair
                            | picker::CharSlot::HairColour
                            | picker::CharSlot::Moustache
                            | picker::CharSlot::Beard
                            | picker::CharSlot::Age
                            | picker::CharSlot::Skin
                    )
            );
            assert_eq!(w.view(), View::Dress { front: false, face }, "{slot:?}");
            world.settle(&mut w);
            let pose = w.pose(&world.layout);
            if face {
                assert!(
                    pose.fov < crate::workshop::camera::FOV_Y,
                    "{slot:?}: closer"
                );
            }
            // Back closes the picker: the whole rider again.
            world.step(&mut w, &[Act::Back]);
            assert!(w.picker.is_none(), "{slot:?}");
            assert_eq!(
                w.view(),
                View::Dress {
                    front: false,
                    face: false
                },
                "{slot:?}"
            );
            world.settle(&mut w);
        }
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

    /// The bench (spec 4D): Confirm there opens the catalogue; its pages
    /// open with Confirm and close with Back; a frameset's dialog buys it
    /// as a new bike, which hangs on a new hook where the "+" was.
    #[test]
    fn the_bench_catalogue_buys_a_bike_onto_a_new_hook() {
        let mut world = World::new();
        world.wallet.grant("Ann", 5_000);
        let mut w = world.open();
        world.step(&mut w, &[Act::Down]);
        assert_eq!(w.view(), View::Bench);
        world.step(&mut w, &[Act::Confirm]);
        let p = w.picker.as_ref().expect("the catalogue");
        assert_eq!(p.bench, Some(BenchPage::Top));
        assert!(p.rows.len() <= 6);
        // Road bikes: the framesets page.
        world.step(&mut w, &[Act::Confirm]);
        let p = w.picker.as_ref().unwrap();
        assert_eq!(p.bench, Some(BenchPage::Type(ocw_core::BikeType::Road)));
        let carbon = p
            .rows
            .iter()
            .position(|r| r.part.as_deref() == Some("frame.road.upper"))
            .unwrap();
        world.step(&mut w, &[Act::Row(carbon), Act::Confirm]);
        let d = w
            .picker
            .as_ref()
            .unwrap()
            .dialog
            .clone()
            .expect("buy dialog");
        assert_eq!(d.buttons[d.focus].label, "Cancel");
        let out = world.step(&mut w, &[Act::Up, Act::Confirm]);
        assert_eq!(out, vec![Out::Line("bikes.buy frame.road.upper".into())]);
        // The app runs the line: the garage has the bike, the wall a hook.
        world
            .wallet
            .buy_bike(
                ocw_assets::shipped::catalogue(),
                "Ann",
                "frame.road.upper",
                &[],
            )
            .unwrap();
        world.step(&mut w, &[]);
        assert_eq!(w.bikes.len(), 7);
        assert_eq!(w.bikes[6], "road-2");
        assert_eq!(w.stand, 0, "the stand keeps its bike");
        assert!(w.caption.as_deref().unwrap().contains("new hook"));
        // Back: the top page (on the Road row), Back: closed, at the bench.
        world.step(&mut w, &[Act::Back]);
        let p = w.picker.as_ref().unwrap();
        assert_eq!((p.bench, p.focus), (Some(BenchPage::Top), 0));
        world.step(&mut w, &[Act::Back]);
        assert!(w.picker.is_none());
        assert_eq!(w.view(), View::Bench);
        // The "+" hook opens the same catalogue, the camera on the hook.
        world.step(&mut w, &[Act::Catalogue { plus: true }]);
        assert_eq!(w.view(), View::Wall(7));
        assert_eq!(w.picker.as_ref().unwrap().bench, Some(BenchPage::Top));
        // Selling it from the bench's last page frees the hook.
        let sell = w.picker.as_ref().unwrap().rows.len() - 1;
        world.step(&mut w, &[Act::RowClick(sell)]);
        let p = w.picker.as_ref().unwrap();
        assert_eq!(p.bench, Some(BenchPage::Sell));
        assert_eq!(p.rows[0].kind, Kind::SellBike("road-2".into()));
        world.step(&mut w, &[Act::Confirm, Act::Up]);
        let out = world.step(&mut w, &[Act::Confirm]);
        assert_eq!(out, vec![Out::Line("bikes.sell road-2".into())]);
        world.wallet.sell_bike("Ann", "road-2").unwrap();
        world.step(&mut w, &[]);
        assert_eq!(w.bikes.len(), 6);
    }

    /// Opens the mirror's picker of kit `slot`.
    fn open_kit(world: &World, w: &mut Workshop, slot: Slot) {
        world.step(w, &[Act::Down, Act::Down, Act::Confirm]);
        world.settle(w);
        w.dress.slot = MirrorSlot::ALL
            .iter()
            .position(|s| *s == MirrorSlot::Kit(slot))
            .unwrap();
        world.step(w, &[Act::Confirm]);
    }

    /// The kit's colours (spec §14 Phase 4, 4N Q30 (a)): the shorts,
    /// socks, shoes and gloves are picked from item tiles with a row of
    /// colour swatches above them – ↑ from the first tile row to the
    /// swatches, ← / → along them (a draft shown live on the rider),
    /// Confirm applies it as a `kit.design` line, ↓ back to the same
    /// tile; the pointer's hover selects a swatch, a click applies it; the
    /// item tiles still preview and buy; Back restores the saved look.
    #[test]
    fn kit_pickers_are_tiles_with_a_swatch_row() {
        use super::super::designer::Field;
        use ocw_core::bike::PALETTE;
        for (slot, field) in [
            (Slot::Shorts, Field::Shorts),
            (Slot::Socks, Field::Socks),
            (Slot::Shoes, Field::Shoes),
            (Slot::Gloves, Field::Gloves),
        ] {
            let world = World::new();
            let mut w = world.open();
            open_kit(&world, &mut w, slot);
            let p = w.picker.as_ref().expect("kit picker");
            assert!(
                p.tiles() && p.of_the_rider() && !p.is_designer(),
                "{slot:?}"
            );
            let s = p.swatches.as_ref().expect("a swatch row");
            assert_eq!(s.field, field);
            assert_eq!(s.colours.len(), PALETTE.len() + 1, "standard + palette");
            assert_eq!(s.focus.swatch, 0, "the standard colour selected");
            assert!(!p.on_swatches(), "the focus starts on the worn item");
            assert!(
                p.rows.iter().all(|r| r.part.is_some()),
                "{slot:?}: only items among the tiles"
            );
        }
        let world = World::new();
        let mut w = world.open();
        open_kit(&world, &mut w, Slot::Gloves);
        let gloves = |w: &Workshop| w.design_preview().unwrap().design.gloves;
        let focus = |w: &Workshop| w.picker.as_ref().unwrap().focus;
        // → on the tiles moves over the items, the colour stays.
        world.step(&mut w, &[Act::Right]);
        assert_eq!(focus(&w), 1);
        assert_eq!(gloves(&w), None);
        let worn = world.with_ctx(&w, |w, ctx| w.rider_look(ctx).1);
        let item = w.picker.as_ref().unwrap().rows[1].part.clone().unwrap();
        let model = &ocw_assets::shipped::catalogue().get(&item).unwrap().model;
        assert!(worn.contains(model), "the focused item on the rider: {worn:?}");
        // ↑ to the swatches, → twice: the second palette colour, live.
        world.step(&mut w, &[Act::Up]);
        assert!(w.picker.as_ref().unwrap().on_swatches());
        world.step(&mut w, &[Act::Right, Act::Right]);
        assert_eq!(gloves(&w), Some(PALETTE[1]));
        world.step(&mut w, &[Act::Left]);
        assert_eq!(gloves(&w), Some(PALETTE[0]));
        // ↓ back to the same item tile, the colour kept on the draft.
        world.step(&mut w, &[Act::Down]);
        let p = w.picker.as_ref().unwrap();
        assert!(!p.on_swatches());
        assert_eq!(p.focus, 1);
        assert_eq!(gloves(&w), Some(PALETTE[0]));
        // Confirm on the swatch row applies the colour.
        world.step(&mut w, &[Act::Up, Act::Right]);
        let out = world.step(&mut w, &[Act::Confirm]);
        let [Out::Line(l)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(
            l.starts_with("kit.design ") && l.contains(&format!("--gloves {}", PALETTE[1])),
            "{l}"
        );
        // The view: the selected swatch outlined, the row focused, named.
        let v =
            super::super::swatch_row_view(w.picker.as_ref().unwrap().swatches.as_ref().unwrap());
        assert_eq!((v.selected, v.focused), (2, true));
        assert_eq!(v.colours.len(), 13);
        assert_eq!(v.colours[2], PALETTE[1]);
        assert_eq!(v.colours[0], Field::Gloves.standard_colour());
        assert_eq!(v.label, format!("Colour of the gloves: {}", PALETTE[1]));
        // The pointer: hovering a swatch selects it, a click applies it.
        world.step(&mut w, &[Act::Swatch(5)]);
        assert_eq!(gloves(&w), Some(PALETTE[4]));
        let out = world.step(&mut w, &[Act::SwatchClick(7)]);
        let [Out::Line(l)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(l.contains(&format!("--gloves {}", PALETTE[6])), "{l}");
        // Hovering an item tile moves the focus there; a click on a shop
        // tile opens its buy dialog as before.
        let shop = w
            .picker
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .position(|r| r.kind == Kind::Shop)
            .expect("gloves in the shop");
        world.step(&mut w, &[Act::Row(shop)]);
        let p = w.picker.as_ref().unwrap();
        assert!(!p.on_swatches());
        assert_eq!(p.focus, shop);
        world.step(&mut w, &[Act::RowClick(shop)]);
        assert!(w.picker.as_ref().unwrap().dialog.is_some());
        world.step(&mut w, &[Act::Back]);
        assert!(w.picker.as_ref().unwrap().dialog.is_none());
        // Back restores the saved look (no draft).
        world.step(&mut w, &[Act::Back]);
        assert!(w.picker.is_none());
        assert!(w.design_preview().is_none());
        // The helmet and the jersey have no swatch row here (their
        // colours are the designer's).
        let helmet = Picker::kit(Slot::Helmet, Vec::new(), ocw_core::RiderLook::default());
        assert!(helmet.swatches.is_none() && helmet.tiles());
        let helmet = picker::kit_rows(
            ocw_assets::shipped::catalogue(),
            world.wallet.get("Ann").unwrap(),
            Slot::Helmet,
        );
        assert!(helmet.iter().all(|r| !matches!(r.kind, Kind::Design(_))));
    }

    /// A colour already chosen is the selected swatch when the picker
    /// opens (4N Q30 (a)).
    #[test]
    fn the_swatch_row_selects_the_worn_colour() {
        use super::super::designer::Field;
        let mut look = ocw_core::RiderLook::default();
        look.design.socks = Some(ocw_core::bike::PALETTE[3]);
        let p = Picker::kit(Slot::Socks, Vec::new(), look);
        let s = p.swatches.as_ref().unwrap();
        assert_eq!(s.focus.swatch, 4);
        assert_eq!(s.selected(), Some(ocw_core::bike::PALETTE[3]));
        assert_eq!(s.field, Field::Socks);
        assert_eq!(s.name(0), "standard (white)");
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
                look: ocw_core::RiderLook::default(),
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
            look: ocw_core::RiderLook::default(),
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

    /// The models and paint on the stand bike (with the live preview).
    fn stand_models(world: &World, w: &Workshop) -> (Vec<String>, Option<Rgb>) {
        let boxes = world.boxes(w);
        let ctx = Ctx {
            cat: ocw_assets::shipped::catalogue(),
            garage: world.wallet.get("Ann").unwrap(),
            balance: world.wallet.balance("Ann"),
            layout: &world.layout,
            size: Vec2::new(1920.0, 1080.0),
            anchors: &world.anchors,
            boxes: &boxes,
            character: ocw_core::Character::default(),
            look: ocw_core::RiderLook::default(),
            rider_spots: &world.spots,
        };
        w.stand_look(&ctx)
    }

    /// Opens the stand bike's picker of `slot`.
    fn open_slot(world: &World, w: &mut Workshop, slot: Slot) {
        world.step(w, &[Act::Confirm]);
        world.settle(w);
        w.hotspot = world.anchors.iter().position(|(s, _)| *s == slot).unwrap();
        world.step(w, &[Act::Confirm]);
    }

    /// The bike stand's tiles (spec 4B *Tiles*): the part pickers and the
    /// paint are tile grids, the kit, the bench and the character lists;
    /// the focus moves over the grid with the arrows (keyboard and the
    /// gamepad's pad / stick give the same actions), the mouse's hover and
    /// a click or tap; the stand bike shows the focused tile at once, a
    /// confirm (or click / tap) applies through the dialog, Cancel keeps
    /// the picker, Back restores the fitted part.
    #[test]
    fn tiles_move_preview_apply_and_cancel_with_every_input() {
        let world = World::new();
        let mut w = world.open();
        open_slot(&world, &mut w, Slot::Wheels);
        let p = w.picker.as_ref().unwrap();
        assert!(p.tiles());
        // Equipped base wheels, then four road wheelsets in the shop.
        assert_eq!(p.rows.len(), 5, "{:?}", p.rows);
        let fitted = stand_models(&world, &w);
        let model_of = |w: &Workshop| {
            let id = w.picker.as_ref().unwrap().preview().unwrap().to_string();
            ocw_assets::shipped::catalogue()
                .get(&id)
                .unwrap()
                .model
                .clone()
        };
        // Keyboard / gamepad: → one tile, ↓ one row (3 columns), ← back,
        // ↑ at the top stays.
        world.step(&mut w, &[Act::Right]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 1);
        assert!(
            stand_models(&world, &w).0.contains(&model_of(&w)),
            "preview"
        );
        world.step(&mut w, &[Act::Down]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 4);
        world.step(&mut w, &[Act::Down]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 4, "the last row holds");
        world.step(&mut w, &[Act::Left]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 3);
        world.step(&mut w, &[Act::Up]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 0);
        world.step(&mut w, &[Act::Up]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 0);
        assert_eq!(stand_models(&world, &w), fitted, "the base tile");
        // Mouse hover: focus and preview at once.
        world.step(&mut w, &[Act::Row(2)]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 2);
        let previewed = model_of(&w);
        assert!(stand_models(&world, &w).0.contains(&previewed));
        // A click / tap on a tile: focused and the dialog (apply); Cancel
        // keeps the picker and the preview.
        world.step(&mut w, &[Act::RowClick(1)]);
        let p = w.picker.as_ref().unwrap();
        assert_eq!(p.focus, 1);
        let d = p.dialog.clone().expect("the buy dialog");
        assert_eq!(d.buttons[d.focus].label, "Cancel");
        world.step(&mut w, &[Act::Confirm]);
        assert!(w.picker.as_ref().unwrap().dialog.is_none());
        assert!(stand_models(&world, &w).0.contains(&model_of(&w)));
        // Confirm on the focused tile, then the buy button: applied.
        let part = w.picker.as_ref().unwrap().preview().unwrap().to_string();
        world.step(&mut w, &[Act::Confirm]);
        let out = world.step(&mut w, &[Act::Up, Act::Confirm]);
        assert_eq!(
            out,
            vec![Out::Line(format!("workshop.buy {part} --bike road"))]
        );
        // Back: the fitted part again (nothing was bought in this test).
        world.step(&mut w, &[Act::Right, Act::Back]);
        assert!(w.picker.is_none());
        assert_eq!(stand_models(&world, &w), fitted, "cancel restores");
    }

    /// The paint is tiles too, previewed on the frame; the bench and the
    /// jersey designer stay lists; the mirror's kit and character pickers
    /// are tiles of the rider (spec §14 Phase 4, 4N).
    #[test]
    fn paint_tiles_preview_the_colour() {
        let world = World::new();
        let mut w = world.open();
        open_slot(&world, &mut w, Slot::Paint);
        let p = w.picker.as_ref().unwrap();
        assert!(p.tiles());
        let before = stand_models(&world, &w).1;
        world.step(&mut w, &[Act::Right]);
        let after = stand_models(&world, &w).1;
        assert_ne!(before, after);
        assert_eq!(
            Some(after),
            w.picker.as_ref().unwrap().preview_paint(),
            "the focused colour on the frame"
        );
        world.step(&mut w, &[Act::Back]);
        assert_eq!(stand_models(&world, &w).1, before);
        let kit = Picker::kit(Slot::Helmet, Vec::new(), ocw_core::RiderLook::default());
        assert!(kit.tiles() && kit.of_the_rider());
        let mut catalogue = Picker::new(Slot::Frame, Vec::new());
        catalogue.bench = Some(bench::BenchPage::Top);
        assert!(!catalogue.tiles());
        let body = Picker::character(picker::CharSlot::Body, &ocw_core::Character::default());
        assert!(body.tiles() && body.of_the_rider());
        let designer = Picker::designer(
            super::super::designer::DesignPage::Jersey,
            ocw_core::RiderLook::default(),
        );
        assert!(!designer.tiles());
    }

    /// The mirror's pickers as tiles (spec §14 Phase 4, 4N), with every
    /// input: the arrows (keyboard and pad: three columns) move the focus
    /// over the hair styles and the rider shows the focused one at once;
    /// the pointer's hover focuses and previews; Confirm and a click keep
    /// a choice (one `character` line each); Back restores the saved look.
    /// Each tile's look is the rider's with that row (the picture's key).
    #[test]
    fn the_mirror_picker_is_tiles_with_every_input() {
        let world = World::new();
        let mut w = world.open();
        world.step(&mut w, &[Act::Down, Act::Down, Act::Down]);
        world.step(&mut w, &[Act::Confirm]);
        world.settle(&mut w);
        let hair = MirrorSlot::ALL
            .iter()
            .position(|s| *s == MirrorSlot::Char(picker::CharSlot::Hair))
            .unwrap();
        w.dress.slot = hair;
        world.step(&mut w, &[Act::Confirm]);
        let ctx_char = ocw_core::Character::default();
        let shown = |w: &Workshop| world.with_ctx(w, |w, ctx| w.rider_look(ctx).0.hair);
        let p = w.picker.as_ref().unwrap();
        assert!(p.tiles() && p.of_the_rider());
        assert_eq!(p.rows.len(), ocw_core::character::HAIR_STYLES.len());
        assert_eq!(p.focus, ctx_char.hair as usize, "the current style");
        // Each tile's own look: that row's style.
        for (i, r) in p.rows.iter().enumerate() {
            let c = world.with_ctx(&w, |w, ctx| w.rider_look_with(ctx, Some(r)).0);
            assert_eq!(c.hair as usize, i);
        }
        // Keyboard / pad arrows on the grid.
        world.step(&mut w, &[Act::Right]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 1);
        assert_eq!(shown(&w), 1, "previewed at once");
        world.step(&mut w, &[Act::Down]);
        assert_eq!(
            w.picker.as_ref().unwrap().focus,
            1 + ocw_client::part_tiles::COLUMNS
        );
        world.step(&mut w, &[Act::Left, Act::Up]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 0);
        // Hover: focus and preview.
        world.step(&mut w, &[Act::Row(9)]);
        assert_eq!(shown(&w), 9);
        // Confirm keeps it.
        let out = world.step(&mut w, &[Act::Confirm]);
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0], Out::Line(l) if l.starts_with("character --hair")),
            "{out:?}"
        );
        // A click on a tile keeps that one.
        let out = world.step(&mut w, &[Act::RowClick(5)]);
        assert_eq!(w.picker.as_ref().unwrap().focus, 5);
        assert!(
            matches!(&out[0], Out::Line(l) if l.starts_with("character --hair")),
            "{out:?}"
        );
        // Back: the picker closes, the saved look shows again.
        world.step(&mut w, &[Act::Right, Act::Back]);
        assert!(w.picker.is_none());
        assert_eq!(shown(&w), ctx_char.hair, "Back restores");
        // The helmets' picker is tiles too: each row its helmet on the rider.
        let helmet = MirrorSlot::ALL
            .iter()
            .position(|s| *s == MirrorSlot::Kit(Slot::Helmet))
            .unwrap();
        w.dress.slot = helmet;
        world.step(&mut w, &[Act::Confirm]);
        let p = w.picker.as_ref().unwrap();
        assert!(p.tiles());
        let kits: Vec<Vec<String>> = p
            .rows
            .iter()
            .filter(|r| r.part.is_some())
            .map(|r| world.with_ctx(&w, |w, ctx| w.rider_look_with(ctx, Some(r)).1))
            .collect();
        assert!(kits.len() > 1);
        assert!(kits.windows(2).any(|k| k[0] != k[1]), "{kits:?}");
    }
}
