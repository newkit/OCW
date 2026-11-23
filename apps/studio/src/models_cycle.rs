//! The unicorn's flight cycles in the Models viewer (spec §14 Phase 16,
//! 16B, stage E3): a model with the unicorn's skeleton
//! (`creatures/unicorn-winged.glb`, Phase 15 M4) plays the cycles the game
//! flies it with – the wing beat of a climb, the gallop of slow flight, the
//! cruise with its flap-glide bursts and the landing (braking flaps, then
//! folded) – with play / pause, a speed and a seek.
//!
//! No pose code of its own: each cycle is a fixed flight ([`Cycle::view`],
//! a [`UnicornView`] as the game's flight makes one) fed to the game's
//! [`AnimState::advance`], [`pose`] and [`root_transform`]
//! (`ocw_render::unicorn`), and the joints' rotations go onto the file's
//! nodes (`<joint>` and `<joint>_lod1`) as the game's renderer puts them
//! on its rig.
//!
//! The console has it as `studio cycle wing|gallop|flight|landing|rest |
//! play | pause | speed X | at SECONDS | record DIR [SECONDS] [FPS] |
//! info` (the command [`crate::edit::StudioCommand::Cycle`]).

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::egui;
use ocw_assets::model::Model;
use ocw_assets::unicorn::{Joint, skeleton};
use ocw_render::UnicornView;
use ocw_render::unicorn::{AnimState, Pose, pose, root_transform};

use crate::models_view::{ModelView, ModelsActive, Part, ViewNode};

/// A flight cycle of the unicorn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    /// Climbing at 12 m/s, 4 m/s up: a deep beat all the time.
    WingBeat,
    /// Slow level flight (8 m/s): the legs gallop, the wings beat.
    Gallop,
    /// Cruising at 30 m/s: bursts of beats and glides between them.
    Flight,
    /// Flying in at 15 m/s for a second, then landing: braking flaps,
    /// slowing down, the wings folding (a loop of [`LANDING_LOOP_S`]).
    Landing,
}

/// The landing cycle's loop: [`LANDING_FLY_S`] of flight, then landing.
pub const LANDING_LOOP_S: f32 = 4.0;
pub const LANDING_FLY_S: f32 = 1.0;

/// The step of a seek and of the playback (s of cycle time at most).
const STEP_S: f32 = 1.0 / 60.0;

impl Cycle {
    pub const ALL: [Self; 4] = [Self::WingBeat, Self::Gallop, Self::Flight, Self::Landing];

    pub fn name(self) -> &'static str {
        match self {
            Self::WingBeat => "wing",
            Self::Gallop => "gallop",
            Self::Flight => "flight",
            Self::Landing => "landing",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WingBeat => "Wing beat (climb)",
            Self::Gallop => "Gallop (slow flight)",
            Self::Flight => "Flight (cruise, flap-gliding)",
            Self::Landing => "Landing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = match s {
            "wingbeat" | "wing-beat" | "beat" => "wing",
            "cruise" | "fly" => "flight",
            "land" => "landing",
            s => s,
        };
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The flight at `t` seconds into the cycle, as the game's flight
    /// gives it to the renderer (the unicorn at the origin, looking along
    /// −z, fully grown).
    pub fn view(self, t: f32) -> UnicornView {
        let (speed, climb, landing) = match self {
            Self::WingBeat => (12.0, 4.0, false),
            Self::Gallop => (8.0, 0.0, false),
            Self::Flight => (30.0, 0.0, false),
            Self::Landing => {
                let u = t.rem_euclid(LANDING_LOOP_S);
                if u < LANDING_FLY_S {
                    (15.0, 0.0, false)
                } else {
                    // Slowing down to a stop over the landing.
                    let k = (u - LANDING_FLY_S) / (LANDING_LOOP_S - LANDING_FLY_S);
                    (15.0 * (1.0 - k as f64).max(0.0), -1.0, true)
                }
            }
        };
        UnicornView {
            id: ocw_core::RiderId(1),
            name: String::new(),
            is_local: true,
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed_mps: speed,
            climb_mps: climb,
            turn_rate: 0.0,
            accel: 0.0,
            age_s: 10.0,
            landing,
            colour: ocw_core::Rgb(0xFF_FFFF),
        }
    }
}

/// A frame-stepped recording of the playing cycle (`studio cycle record`):
/// one screenshot of the window per frame, the cycle advancing exactly
/// 1 / fps per frame however slowly the view renders.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub dir: PathBuf,
    pub frames: u32,
    pub fps: u32,
    pub frame: u32,
    /// Frames shown before the first one is taken (the window and the
    /// model settle: a recording asked for at startup).
    pub warmup: u32,
}

/// The frames a recording waits before it starts.
pub const RECORD_WARMUP: u32 = 30;

/// The cycle player of the Models viewer.
#[derive(Debug, Clone, PartialEq)]
pub struct CyclePlayer {
    /// The cycle shown (`None`: the file's rest pose).
    pub cycle: Option<Cycle>,
    pub playing: bool,
    /// Playback speed (1 = the game's).
    pub speed: f32,
    /// Seconds into the cycle.
    pub t: f32,
    pub anim: AnimState,
    pub record: Option<Recording>,
}

impl Default for CyclePlayer {
    fn default() -> Self {
        Self {
            cycle: None,
            playing: true,
            speed: 1.0,
            t: 0.0,
            anim: AnimState::default(),
            record: None,
        }
    }
}

/// The playback speeds offered.
pub const SPEEDS: (f32, f32) = (0.05, 3.0);

impl CyclePlayer {
    /// Shows a cycle (or the rest pose) from its start.
    pub fn set(&mut self, cycle: Option<Cycle>) {
        self.cycle = cycle;
        self.restart();
    }

    pub fn restart(&mut self) {
        self.t = 0.0;
        self.anim = AnimState::default();
    }

    /// Advances the cycle by `dt` seconds of its own time, in the game's
    /// steps (at most 1/60 s).
    pub fn advance(&mut self, dt: f32) {
        let Some(c) = self.cycle else { return };
        let mut left = dt.max(0.0);
        while left > 0.0 {
            let h = left.min(STEP_S);
            let v = c.view(self.t);
            self.anim.advance(&v, h);
            self.t += h;
            left -= h;
        }
    }

    /// A frame of `dt` real seconds: advanced while playing, at the speed.
    pub fn step(&mut self, dt: f32) {
        if self.playing {
            self.advance(dt * self.speed);
        }
    }

    /// Goes to `t` seconds into the cycle (from its start, so the same
    /// time always gives the same pose).
    pub fn seek(&mut self, t: f32) {
        self.restart();
        let mut n = (t.max(0.0) / STEP_S).round() as u32;
        while n > 0 {
            self.advance(STEP_S);
            n -= 1;
        }
    }

    /// The game's pose now and the body's transform (at the origin).
    pub fn pose(&self) -> Option<(Pose, Transform)> {
        let c = self.cycle?;
        let v = c.view(self.t);
        let p = pose(&self.anim, &v);
        let body = root_transform(&v, &p);
        Some((p, body))
    }

    pub fn describe(&self) -> String {
        let Some(c) = self.cycle else {
            return "rest pose".into();
        };
        let a = &self.anim;
        format!(
            "{} at {:.2} s ({}, ×{:.2}): wing phase {:.2}, beat depth {:.2}, spread {:.2}{}, gallop phase {:.2}",
            c.label(),
            self.t,
            if self.playing { "playing" } else { "paused" },
            self.speed,
            a.wing,
            a.flap,
            a.spread,
            if a.gliding() { ", gliding" } else { "" },
            a.gallop
        )
    }
}

/// Whether a model has the cycles: the unicorn's skeleton (every joint's
/// node).
pub fn has_cycles(m: &Model) -> bool {
    skeleton().iter().all(|d| m.node(&d.joint.node()).is_some())
}

/// The transforms of the model's joint nodes (near and `_lod1`) in the
/// pose: each joint's rotation as the game's renderer sets it on its rig,
/// the body moved by the root transform (bob, pitch, bank) over its place
/// in the file.
pub fn node_transforms(m: &Model, p: &Pose, body: Transform) -> Vec<(usize, Transform)> {
    let mut out = Vec::new();
    for (i, n) in m.nodes.iter().enumerate() {
        let Some((joint, _far)) = Joint::parse(&n.name) else {
            continue;
        };
        let rest = Transform {
            translation: n.translation,
            rotation: n.rotation,
            scale: n.scale,
        };
        let t = if joint == Joint::Body {
            Transform {
                translation: rest.translation + body.translation,
                rotation: body.rotation * rest.rotation,
                scale: rest.scale * body.scale,
            }
        } else if let Some(q) = p.joints.get(&joint) {
            Transform {
                rotation: *q,
                ..rest
            }
        } else {
            continue;
        };
        out.push((i, t));
    }
    out
}

/// A `studio cycle …` command.
#[derive(Debug, Clone, PartialEq)]
pub enum CycleCommand {
    /// A cycle, or the rest pose.
    Show(Option<Cycle>),
    Play(bool),
    Speed(f32),
    Seek(f32),
    Record {
        dir: PathBuf,
        seconds: f32,
        fps: u32,
    },
    Info,
}

/// Parses the words after `studio cycle`.
pub fn parse(words: &[String]) -> Result<CycleCommand, String> {
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let num = |s: Option<&&str>, what: &str| -> Result<f32, String> {
        s.ok_or_else(|| format!("{what} is missing"))?
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{what} must be a number"))
    };
    match w.as_slice() {
        [] | ["info"] => Ok(CycleCommand::Info),
        ["rest" | "off" | "none"] => Ok(CycleCommand::Show(None)),
        ["play"] => Ok(CycleCommand::Play(true)),
        ["pause"] => Ok(CycleCommand::Play(false)),
        ["speed", rest @ ..] => Ok(CycleCommand::Speed(num(rest.first(), "the speed")?)),
        ["at", rest @ ..] => Ok(CycleCommand::Seek(num(rest.first(), "the time")?)),
        ["record", dir, rest @ ..] => {
            let seconds = match rest.first() {
                Some(_) => num(rest.first(), "the seconds")?,
                None => 4.0,
            };
            let fps = match rest.get(1) {
                Some(_) => num(rest.get(1), "the fps")? as u32,
                None => 25,
            };
            if !(0.1..=60.0).contains(&seconds) || !(1..=60).contains(&fps) {
                return Err("record: 0.1 … 60 s at 1 … 60 fps".into());
            }
            Ok(CycleCommand::Record {
                dir: PathBuf::from(dir),
                seconds,
                fps,
            })
        }
        [name] => Cycle::parse(name)
            .map(|c| CycleCommand::Show(Some(c)))
            .ok_or_else(|| {
                format!("unknown cycle {name}: wing|gallop|flight|landing|rest|play|pause|speed X|at S|record DIR [S] [FPS]|info")
            }),
        _ => Err("usage: studio cycle wing|gallop|flight|landing|rest|play|pause|speed X|at S|record DIR [S] [FPS]|info".into()),
    }
}

/// Carries out a cycle command on the Models view.
pub fn command(view: &mut ModelView, c: CycleCommand) -> Result<String, String> {
    if c == CycleCommand::Info {
        return Ok(view.cycle.describe());
    }
    let o = view.open.as_ref().ok_or("no model is open")?;
    if !has_cycles(&o.model) {
        return Err(format!(
            "{} has no flight cycles (only the unicorn has)",
            o.id
        ));
    }
    let p = &mut view.cycle;
    match c {
        CycleCommand::Show(cycle) => {
            p.set(cycle);
            if cycle.is_none() {
                // Back to the file's rest pose.
                view.dirty = true;
            }
        }
        CycleCommand::Play(on) => p.playing = on,
        CycleCommand::Speed(s) => p.speed = s.clamp(SPEEDS.0, SPEEDS.1),
        CycleCommand::Seek(t) => {
            if p.cycle.is_none() {
                return Err("no cycle is shown".into());
            }
            p.playing = false;
            p.seek(t);
        }
        CycleCommand::Record { dir, seconds, fps } => {
            if p.cycle.is_none() {
                return Err("no cycle is shown".into());
            }
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let frames = (seconds * fps as f32).round().max(1.0) as u32;
            p.playing = false;
            let msg = format!(
                "recording {frames} frames at {fps} fps into {}",
                dir.display()
            );
            p.record = Some(Recording {
                dir,
                frames,
                fps,
                frame: 0,
                warmup: RECORD_WARMUP,
            });
            return Ok(msg);
        }
        CycleCommand::Info => unreachable!(),
    }
    Ok(view.cycle.describe())
}

/// The cycle picker of the Models panel (shown for a model with cycles).
pub fn section(ui: &mut egui::Ui, view: &mut ModelView) {
    let mut cmd = None;
    let p = &view.cycle;
    ui.horizontal_wrapped(|ui| {
        if ui
            .selectable_label(p.cycle.is_none(), "Rest pose")
            .clicked()
        {
            cmd = Some(CycleCommand::Show(None));
        }
        for c in Cycle::ALL {
            if ui.selectable_label(p.cycle == Some(c), c.label()).clicked() {
                cmd = Some(CycleCommand::Show(Some(c)));
            }
        }
    });
    if p.cycle.is_some() {
        let mut speed = p.speed;
        ui.horizontal(|ui| {
            let label = if p.playing { "Pause" } else { "Play" };
            if ui.button(label).clicked() {
                cmd = Some(CycleCommand::Play(!p.playing));
            }
            if ui.button("Restart").clicked() {
                cmd = Some(CycleCommand::Seek(0.0));
            }
            ui.label("Speed");
            if ui
                .add(
                    egui::Slider::new(&mut speed, SPEEDS.0..=SPEEDS.1)
                        .logarithmic(true)
                        .suffix("×"),
                )
                .changed()
            {
                cmd = Some(CycleCommand::Speed(speed));
            }
        });
        ui.weak(p.describe());
    }
    ui.weak("The game's own animation (ocw-render::unicorn) on the file's joint nodes.");
    if let Some(c) = cmd {
        let play = matches!(c, CycleCommand::Seek(_)) && view.cycle.playing;
        let r = command(view, c);
        if play {
            view.cycle.playing = true;
        }
        if let Err(e) = r {
            view.status = e;
        }
    }
}

pub struct CyclePlugin;

impl Plugin for CyclePlugin {
    fn build(&self, app: &mut App) {
        // After the view's own systems (in Update): the joints are set on
        // the nodes the last rebuild spawned.
        app.add_systems(PostUpdate, play.before(TransformSystems::Propagate));
    }
}

/// Advances the cycle and writes its pose into the open model's nodes;
/// a recording takes one screenshot per frame.
fn play(
    mut commands: Commands,
    time: Res<Time>,
    active: Res<ModelsActive>,
    mut view: ResMut<ModelView>,
    mut nodes: Query<(&ViewNode, &mut Transform)>,
) {
    if !active.0 || view.cycle.cycle.is_none() {
        return;
    }
    let Some(model) = view.open.as_ref().map(|o| o.model.clone()) else {
        return;
    };
    if !has_cycles(&model) {
        return;
    }
    if let Some(mut rec) = view.cycle.record.take() {
        if rec.warmup > 0 {
            rec.warmup -= 1;
            view.cycle.record = Some(rec);
        } else if rec.frame < rec.frames {
            if rec.frame > 0 {
                view.cycle.advance(1.0 / rec.fps as f32);
            }
            let path = rec.dir.join(format!("frame_{:05}.png", rec.frame));
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            rec.frame += 1;
            view.cycle.record = Some(rec);
        } else {
            info!(frames = rec.frames, dir = %rec.dir.display(), "cycle recording finished");
        }
    } else {
        let dt = time.delta_secs();
        view.cycle.step(dt);
    }
    let Some((p, body)) = view.cycle.pose() else {
        return;
    };
    let want = node_transforms(&model, &p, body);
    for (vn, mut t) in &mut nodes {
        if vn.part != Part::Main {
            continue;
        }
        if let Some((_, nt)) = want.iter().find(|(i, _)| *i == vn.index) {
            *t = *nt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_assets::library::ModelLibrary;
    use ocw_render::unicorn::{BRAKE_S, stroke};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn unicorn() -> ModelView {
        let mut v = ModelView::new(ModelLibrary::embedded(), "embedded".into());
        v.open("creatures/unicorn-winged").unwrap();
        v
    }

    #[test]
    fn console_words_parse() {
        assert_eq!(parse(&words("")), Ok(CycleCommand::Info));
        assert_eq!(
            parse(&words("wing")),
            Ok(CycleCommand::Show(Some(Cycle::WingBeat)))
        );
        assert_eq!(
            parse(&words("gallop")),
            Ok(CycleCommand::Show(Some(Cycle::Gallop)))
        );
        assert_eq!(
            parse(&words("landing")),
            Ok(CycleCommand::Show(Some(Cycle::Landing)))
        );
        assert_eq!(parse(&words("rest")), Ok(CycleCommand::Show(None)));
        assert_eq!(parse(&words("pause")), Ok(CycleCommand::Play(false)));
        assert_eq!(parse(&words("speed 0.5")), Ok(CycleCommand::Speed(0.5)));
        assert_eq!(parse(&words("at 1.25")), Ok(CycleCommand::Seek(1.25)));
        assert_eq!(
            parse(&words("record out 2 10")),
            Ok(CycleCommand::Record {
                dir: "out".into(),
                seconds: 2.0,
                fps: 10
            })
        );
        assert!(parse(&words("speed fast")).is_err());
        assert!(parse(&words("trot")).is_err());
        assert!(parse(&words("record out 100")).is_err());
    }

    #[test]
    fn only_the_unicorn_has_cycles() {
        let mut v = unicorn();
        assert!(has_cycles(&v.open.as_ref().unwrap().model));
        command(&mut v, CycleCommand::Show(Some(Cycle::Gallop))).unwrap();
        assert_eq!(v.cycle.cycle, Some(Cycle::Gallop));
        let mut bike = ModelView::new(ModelLibrary::embedded(), "embedded".into());
        bike.open("bikes/bike-tt").unwrap();
        assert!(!has_cycles(&bike.open.as_ref().unwrap().model));
        assert!(command(&mut bike, CycleCommand::Show(Some(Cycle::WingBeat))).is_err());
        // Back to rest rebuilds the file's pose.
        v.dirty = false;
        command(&mut v, CycleCommand::Show(None)).unwrap();
        assert!(v.dirty && v.cycle.cycle.is_none());
    }

    /// The viewer's joints are the game's: the same state gives the same
    /// rotations on the file's near and far nodes.
    #[test]
    fn the_nodes_get_the_games_pose() {
        let v = unicorn();
        let m = v.open.as_ref().unwrap().model.clone();
        let mut p = CyclePlayer::default();
        p.set(Some(Cycle::WingBeat));
        p.seek(1.3);
        // The game, stepped the same way.
        let mut a = AnimState::default();
        let mut t = 0.0;
        for _ in 0..(1.3f32 / STEP_S).round() as u32 {
            a.advance(&Cycle::WingBeat.view(t), STEP_S);
            t += STEP_S;
        }
        assert_eq!(p.anim, a);
        let game = pose(&a, &Cycle::WingBeat.view(t));
        let (mine, body) = p.pose().unwrap();
        assert_eq!(mine, game);
        let set = node_transforms(&m, &mine, body);
        for name in ["wing_l", "wing_l_lod1", "wrist_r", "hip_fl", "tail_3"] {
            let i = m.node(name).unwrap_or_else(|| panic!("node {name}"));
            let (j, _) = Joint::parse(name).unwrap();
            let (_, tr) = set.iter().find(|(k, _)| *k == i).unwrap();
            assert_eq!(tr.rotation, game.joints[&j], "{name}");
            assert_eq!(
                tr.translation, m.nodes[i].translation,
                "{name} keeps its pivot"
            );
        }
        // The body bobs over its place in the file.
        let b = m.node("body").unwrap();
        let (_, tb) = set.iter().find(|(k, _)| *k == b).unwrap();
        assert!((tb.translation.y - m.nodes[b].translation.y - game.bob).abs() < 1e-6);
    }

    #[test]
    fn the_cycles_move_as_in_the_game() {
        let sample = |c: Cycle, secs: f32| {
            let mut p = CyclePlayer::default();
            p.set(Some(c));
            let mut out = Vec::new();
            for _ in 0..(secs * 30.0) as u32 {
                p.advance(1.0 / 30.0);
                out.push((p.anim, p.pose().unwrap().0));
            }
            out
        };
        // Wing beat: beating deep all the time, the wings up and down.
        let w = sample(Cycle::WingBeat, 8.0);
        assert!(w[60..].iter().all(|(a, _)| a.flap > 0.9 && !a.gliding()));
        let flaps: Vec<f32> = w[60..].iter().map(|(_, p)| p.flap).collect();
        let (lo, hi) = flaps
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), f| (l.min(*f), h.max(*f)));
        assert!(hi > 0.5 && lo < -0.4, "{lo} … {hi}");
        // Gallop: the legs swing through the stride.
        let g = sample(Cycle::Gallop, 4.0);
        let hips: Vec<f32> = g
            .iter()
            .map(|(_, p)| p.joints[&Joint::Hip(0)].to_euler(EulerRot::XYZ).0)
            .collect();
        let swing = hips.iter().cloned().fold(f32::MIN, f32::max)
            - hips.iter().cloned().fold(f32::MAX, f32::min);
        assert!(swing > 0.8, "{swing}");
        // Flight: bursts with glides between them.
        let f = sample(Cycle::Flight, 12.0);
        assert!(f.iter().any(|(a, _)| a.gliding()));
        assert!(f.iter().any(|(a, _)| !a.gliding() && a.flap > 0.5));
        // Landing: braking flaps with spread wings, then folded; again
        // after the loop.
        let l = sample(Cycle::Landing, 2.0 * LANDING_LOOP_S);
        let at = |s: f32| &l[(s * 30.0) as usize - 1].0;
        assert!(at(LANDING_FLY_S + 0.5 * BRAKE_S).landing_s > 0.0);
        assert!(at(LANDING_FLY_S + 0.5 * BRAKE_S).spread > 0.8);
        assert!(at(LANDING_LOOP_S - 0.1).spread < 0.1);
        assert_eq!(at(LANDING_LOOP_S + 0.5).landing_s, 0.0);
        assert!(at(LANDING_LOOP_S + LANDING_FLY_S - 0.05).spread > 0.4);
    }

    #[test]
    fn seek_speed_and_pause() {
        let mut p = CyclePlayer::default();
        p.set(Some(Cycle::Gallop));
        p.seek(0.8);
        let a = p.anim;
        p.seek(0.8);
        assert_eq!(p.anim, a, "the same time is the same pose");
        p.playing = false;
        p.step(0.5);
        assert_eq!(p.anim, a, "paused");
        p.playing = true;
        p.speed = 0.5;
        let mut q = p.clone();
        p.step(0.4);
        q.advance(0.2);
        assert_eq!(p.anim, q.anim, "half speed");
        // The wing's stroke is the game's.
        assert!((stroke(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_commands_drive_the_player() {
        let mut v = unicorn();
        assert_eq!(command(&mut v, CycleCommand::Info).unwrap(), "rest pose");
        assert!(command(&mut v, CycleCommand::Seek(1.0)).is_err());
        command(&mut v, CycleCommand::Show(Some(Cycle::WingBeat))).unwrap();
        let msg = command(&mut v, CycleCommand::Seek(0.5)).unwrap();
        assert!(msg.contains("paused") && msg.contains("0.50 s"), "{msg}");
        command(&mut v, CycleCommand::Speed(10.0)).unwrap();
        assert_eq!(v.cycle.speed, SPEEDS.1);
        command(&mut v, CycleCommand::Play(true)).unwrap();
        assert!(v.cycle.playing);
    }
}
