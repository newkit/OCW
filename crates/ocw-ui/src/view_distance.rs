//! The render distance in both apps (spec §14 Phase 10, 10G): the
//! settings `display.view_distance`, `display.view_distance_auto` and
//! `display.target_fps`, the console's `view`, the dynamic mode
//! (`ocw_client::view_distance::Controller`, fed with the measured frame
//! rate every frame) and what the distance drives: the renderer's preset
//! (`QualityPreset::with_view_distance`: far plane and fog, trees,
//! houses), the near tile ring (`ocw_render::package::StreamRadius`; the
//! browser's tile streaming follows it) and the FPS counter's note.

use bevy::prelude::*;
use bevy::window::{Monitor, PrimaryMonitor};
use clap::{Arg, Command};
use ocw_client::view_distance::{
    AppKind, Controller, DistanceSetting, Input, TargetSetting, ViewInfo, km_text, read_settings,
    target_fps,
};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::frame_rate::{FpsBadgeNote, FrameLimit, RenderRate};
use ocw_render::package::StreamRadius;
use ocw_render::quality::{QualityPreset, RenderQuality, stream_radius_for};

use crate::quality::QualityChoice;

/// What the app tells about itself.
#[derive(Resource, Debug, Clone, Default)]
pub struct ViewDistanceContext {
    /// The dynamic mode holds the distance at the slider: a reproducible
    /// picture (the desktop's `--screenshot` and `--capture` runs, whose
    /// frame rate is the software renderer's or the capture's).
    pub frozen: bool,
}

/// The tile ring changes at most this often (s): no streaming thrash.
pub const RADIUS_INTERVAL_S: f64 = 10.0;
/// A rider moving farther in one frame jumped (a teleport, a new world).
const JUMP_M: f64 = 200.0;
/// The world clock moving more than this in one frame jumped (ms; the
/// time machine).
const CLOCK_JUMP_MS: i64 = 15 * 60 * 1000;

/// The render distance's state.
#[derive(Resource, Debug, Clone)]
pub struct ViewDistanceState {
    pub setting: DistanceSetting,
    pub auto: bool,
    pub target: TargetSetting,
    pub controller: Controller,
    /// The target that runs and why.
    pub target_fps: f64,
    pub target_reason: &'static str,
    /// The primary display's refresh (Hz), where the window system says.
    pub refresh_hz: Option<f64>,
    /// The distance that runs (m; `None`: unlimited).
    pub distance: Option<f32>,
    /// The upper limit it ran with (a change resets the controller).
    ceiling: Option<Option<f32>>,
    /// The tile ring and the wall-clock time it was set.
    radius: Option<(i32, f64)>,
    last_position: Option<[f64; 2]>,
    last_clock_ms: Option<i64>,
    now_s: f64,
}

impl Default for ViewDistanceState {
    fn default() -> Self {
        Self {
            setting: DistanceSetting::Preset,
            auto: true,
            target: TargetSetting::Auto,
            controller: Controller::new(None),
            target_fps: ocw_client::view_distance::FALLBACK_FPS,
            target_reason: "",
            refresh_hz: None,
            distance: None,
            ceiling: None,
            radius: None,
            last_position: None,
            last_clock_ms: None,
            now_s: 0.0,
        }
    }
}

pub struct ViewDistancePlugin;

impl Plugin for ViewDistancePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewDistanceState>()
            .init_resource::<ViewDistanceContext>()
            .init_resource::<StreamRadius>()
            .init_resource::<FpsBadgeNote>()
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("view")
                    .about("Render distance: view [KM|unlimited|preset] [auto on|off] [target FPS|display|auto]; without arguments: what runs and why (spec §14 Phase 10, 10G)")
                    .arg(Arg::new("words").num_args(0..)),
                |world, m| {
                    let words: Vec<String> = m
                        .get_many::<String>("words")
                        .map(|v| v.cloned().collect())
                        .unwrap_or_default();
                    view_command(world, &words)?;
                    Ok(describe(world))
                },
            ))
            .add_systems(PostStartup, read_at_start)
            .add_systems(Update, (read_refresh, drive).chain());
    }
}

/// `view …`: applies the words to the state.
pub fn view_command(world: &mut World, words: &[String]) -> Result<(), String> {
    let mut state = world.resource::<ViewDistanceState>().clone();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].to_ascii_lowercase();
        let value = || {
            words
                .get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{w}: a value is missing"))
        };
        match w.as_str() {
            "auto" => {
                let v = value()?;
                state.auto = match v.to_ascii_lowercase().as_str() {
                    "on" | "true" | "yes" | "1" => true,
                    "off" | "false" | "no" | "0" => false,
                    _ => return Err(format!("auto: on or off, not {v:?}")),
                };
                i += 2;
            }
            "target" => {
                let v = value()?;
                state.target = TargetSetting::parse(&v)
                    .ok_or_else(|| format!("target: auto, display or 20 – 360, not {v:?}"))?;
                i += 2;
            }
            other => {
                state.setting = DistanceSetting::parse(other).ok_or_else(|| {
                    format!(
                        "{other:?}: a distance in km (1 – 60), unlimited or preset; auto on|off; target FPS"
                    )
                })?;
                i += 1;
            }
        }
    }
    let before = world.resource::<ViewDistanceState>();
    let restart = before.setting != state.setting || before.auto != state.auto;
    if restart {
        // What the rider asked for (at once, the tiles too), then down if
        // it is too slow.
        let ceiling = state.setting.ceiling(base_preset(world).view_m);
        state.controller.reset(ceiling);
        state.ceiling = Some(ceiling);
        state.radius = None;
    }
    world.insert_resource(state);
    apply(world);
    Ok(())
}

/// The settings at the start (their apply commands run only for values
/// that are not the default).
fn read_at_start(world: &mut World) {
    let Some(settings) = world.get_resource::<crate::settings::UserSettings>() else {
        apply(world);
        return;
    };
    let (setting, auto, target) = read_settings(&settings.settings);
    let mut state = world.resource_mut::<ViewDistanceState>();
    state.setting = setting;
    state.auto = auto;
    state.target = target;
    state.ceiling = None;
    apply(world);
}

/// The primary display's refresh rate (Bevy's monitors; none in the
/// browser and on headless outputs: the target falls back to 60).
fn read_refresh(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    mut state: ResMut<ViewDistanceState>,
) {
    let mut best: Option<(bool, f64)> = None;
    for (m, primary) in &monitors {
        if let Some(mhz) = m.refresh_rate_millihertz {
            let hz = f64::from(mhz) / 1000.0;
            if best.is_none_or(|(p, _)| primary && !p) {
                best = Some((primary, hz));
            }
        }
    }
    let hz = best.map(|(_, hz)| hz);
    if state.refresh_hz != hz {
        state.refresh_hz = hz;
    }
}

fn app_kind(world: &World) -> AppKind {
    match world.get_resource::<crate::UiPlatform>().map(|p| p.0) {
        Some(crate::Platform::Browser) => AppKind::Browser,
        _ => AppKind::Desktop,
    }
}

/// The quality preset's own numbers (before the render distance).
fn base_preset(world: &World) -> QualityPreset {
    world
        .get_resource::<QualityChoice>()
        .map(|c| QualityPreset::of(c.level))
        .unwrap_or_default()
}

/// The preset with the render distance that runs, for the renderer
/// (`ocw_ui::quality::set_quality` uses it too).
pub fn adjusted(world: &World, base: QualityPreset) -> QualityPreset {
    match world.get_resource::<ViewDistanceState>() {
        Some(s) => {
            let ceiling = s.setting.ceiling(base.view_m);
            // A new limit (another preset) starts at the limit.
            let d = if s.auto && !frozen(world) && s.ceiling == Some(ceiling) {
                match (s.controller.distance(), ceiling) {
                    (Some(d), Some(c)) => Some(d.min(c)),
                    (d, None) => d,
                    (None, c) => c,
                }
            } else {
                ceiling
            };
            base.with_view_distance(d)
        }
        None => base,
    }
}

fn frozen(world: &World) -> bool {
    world
        .get_resource::<ViewDistanceContext>()
        .is_some_and(|c| c.frozen)
}

/// The target frame rate (and why) from the setting, the app, the
/// display and the frame cap, into the state.
fn update_target(world: &mut World) -> f64 {
    let max_fps = world.get_resource::<FrameLimit>().and_then(|l| l.max_fps);
    let app = app_kind(world);
    let mut state = world.resource_mut::<ViewDistanceState>();
    let (target, reason) = target_fps(state.target, app, state.refresh_hz, max_fps);
    if state.target_fps != target || state.target_reason != reason {
        state.target_fps = target;
        state.target_reason = reason;
    }
    target
}

/// Every frame: the controller with the frame rate, then [`apply`].
fn drive(world: &mut World) {
    let dt = world.resource::<Time<Real>>().delta_secs_f64();
    let (fps, worst_ms) = world
        .get_resource::<RenderRate>()
        .map_or((None, None), |r| (r.meter.fps(), r.meter.worst_ms()));
    let view = world.get_resource::<ocw_render::RideView>();
    let focus = view.and_then(|v| ocw_render::flight_world::stream_focus(v).map(|f| f.centre));
    let clock = view.and_then(|v| v.sky.as_ref().map(|s| s.sky.instant));
    let menu_open = world
        .get_resource::<crate::menu::MenuState>()
        .is_some_and(|m| m.menu.is_open());
    let ceiling = world
        .resource::<ViewDistanceState>()
        .setting
        .ceiling(base_preset(world).view_m);
    let frozen = frozen(world);
    let target = update_target(world);
    let mut state = world.resource_mut::<ViewDistanceState>();
    state.now_s += dt;
    // A new limit (the slider, the preset, the mode switched on): start
    // there.
    if state.ceiling != Some(ceiling) {
        state.ceiling = Some(ceiling);
        state.controller.reset(ceiling);
    }
    // A jump (a world loaded, a teleport, the time machine): settle.
    let jumped = match (state.last_position, focus) {
        (None, Some(_)) => true,
        (Some(a), Some(b)) => (a[0] - b[0]).hypot(a[1] - b[1]) > JUMP_M,
        _ => false,
    } || matches!((state.last_clock_ms, clock), (Some(a), Some(b)) if (a - b).abs() > CLOCK_JUMP_MS);
    state.last_position = focus;
    state.last_clock_ms = clock;
    if jumped {
        state.controller.settle();
        // A new place: its tiles stream anyway.
        state.radius = None;
    }
    if state.auto
        && !frozen
        && let Some(step) = state.controller.update(Input {
            dt_s: dt,
            fps,
            worst_ms,
            target_fps: target,
            ceiling,
            // No rider yet (the loading screen) or the menu: the frame rate
            // says nothing about the ride's view.
            hold: menu_open || focus.is_none(),
        })
    {
        info!("view distance: {}", step.describe(target));
    }
    apply(world);
}

/// Hands the distance to the renderer: the preset, the tile ring (at most
/// every [`RADIUS_INTERVAL_S`], at once at a jump or the start) and the
/// FPS counter's note.
pub fn apply(world: &mut World) {
    update_target(world);
    let base = base_preset(world);
    let preset = adjusted(world, base);
    let auto = {
        let s = world.resource::<ViewDistanceState>();
        s.auto && !frozen(world)
    };
    if world.get_resource::<RenderQuality>().map(|q| q.0) != Some(preset) {
        world.insert_resource(RenderQuality(preset));
    }
    let mut state = world.resource_mut::<ViewDistanceState>();
    state.distance = preset.view_m;
    let now = state.now_s;
    let (ring, at) = next_ring(state.radius, stream_radius_for(preset.view_m), now);
    state.radius = Some((ring, at));
    let note = format!(
        "{}{}",
        km_text(preset.view_m),
        if auto { " auto" } else { "" }
    );
    if world.get_resource::<StreamRadius>().map(|r| r.0) != Some(ring) {
        world.insert_resource(StreamRadius(ring));
    }
    if world.get_resource::<FpsBadgeNote>().map(|n| n.0.as_str()) != Some(note.as_str()) {
        world.insert_resource(FpsBadgeNote(note));
    }
}

/// The tile ring to draw: `want` at once without a ring yet (the start,
/// a jump, a change by the rider: they clear it), else only
/// [`RADIUS_INTERVAL_S`] after the last change (the dynamic mode's steps
/// never make the tiles stream back and forth). With the time it was set.
pub fn next_ring(current: Option<(i32, f64)>, want: i32, now_s: f64) -> (i32, f64) {
    match current {
        Some((r, at)) if r == want || now_s - at < RADIUS_INTERVAL_S => (r, at),
        _ => (want, now_s),
    }
}

/// "12 km, auto, target 60 fps" (the menu's Now row).
pub fn status(world: &World) -> String {
    let Some(s) = world.get_resource::<ViewDistanceState>() else {
        return String::new();
    };
    let mode = if frozen(world) && s.auto {
        "auto held (screenshot run)"
    } else if s.auto {
        "auto"
    } else {
        "fixed"
    };
    format!(
        "{}, {mode}, target {:.0} fps",
        km_text(s.distance),
        s.target_fps
    )
}

/// The menu page's facts.
pub fn info(world: &World) -> ViewInfo {
    ViewInfo {
        preset_m: base_preset(world).view_m,
        status: status(world),
    }
}

/// The console's `view`: the distance, the mode, the limit, the target and
/// why, what the distance drives, the last step.
pub fn describe(world: &World) -> String {
    let Some(s) = world.get_resource::<ViewDistanceState>() else {
        return "view: not available".into();
    };
    let base = base_preset(world);
    let preset = adjusted(world, base);
    let limit = match s.setting {
        DistanceSetting::Preset => format!("the {} preset's {}", base.level, km_text(base.view_m)),
        other => km_text(other.ceiling(base.view_m)),
    };
    let ring = s
        .radius
        .map_or(stream_radius_for(preset.view_m), |(r, _)| r);
    let mut out = format!(
        "view: {} ({}; limit {limit}; target {:.0} fps: {}) – trees {}, house LOD × {:.2}, tiles {n} × {n}",
        km_text(s.distance),
        if frozen(world) && s.auto {
            "auto, held for this screenshot / capture run"
        } else if s.auto {
            "auto"
        } else {
            "fixed"
        },
        s.target_fps,
        s.target_reason,
        km_text(preset.tree_view_m),
        preset.house_lod_scale,
        n = 2 * ring + 1,
    );
    if let Some(step) = s.controller.last() {
        out.push_str(&format!("\nlast step: {}", step.describe(s.target_fps)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_client::quality::{DeviceInfo, GpuApi};
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
    use ocw_render::quality::QualityLevel;

    fn app(api: GpuApi) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            ConsoleCorePlugin,
            crate::quality::QualityPlugin,
        ))
        .insert_resource(crate::quality::QualityContext {
            api,
            device: DeviceInfo::default(),
            vr: false,
        });
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue(line.to_string());
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    fn preset(app: &App) -> QualityPreset {
        app.world().resource::<RenderQuality>().0
    }

    /// `view` sets the distance, the mode and the target; the renderer's
    /// preset, the tile ring and the FPS note follow; the preset's own
    /// distance changes nothing.
    #[test]
    fn the_console_sets_the_render_distance() {
        let mut a = app(GpuApi::WebGl2);
        // Low (WebGL 2) at its preset distance: Low's own numbers.
        assert_eq!(preset(&a), QualityPreset::of(QualityLevel::Low));
        let out = run(&mut a, "view");
        assert!(
            out.contains("view: 8 km (auto; limit the low preset's 8 km"),
            "{out}"
        );
        let out = run(&mut a, "view 2 auto off");
        assert!(out.contains("view: 2 km (fixed"), "{out}");
        let p = preset(&a);
        assert_eq!(p.view_m, Some(2_000.0));
        assert_eq!(p.tree_view_m, Some(1_000.0));
        assert_eq!(a.world().resource::<StreamRadius>().0, 1, "3 × 3 tiles");
        assert_eq!(a.world().resource::<FpsBadgeNote>().0, "2 km");
        let out = run(&mut a, "view unlimited target 30");
        assert!(out.contains("view: unlimited (fixed"), "{out}");
        assert!(out.contains("target 30 fps: chosen"), "{out}");
        assert_eq!(preset(&a).view_m, None);
        // The rider's change: the ring at once.
        assert_eq!(
            a.world().resource::<StreamRadius>().0,
            ocw_render::package::STREAM_RADIUS
        );
        let out = run(&mut a, "view auto on preset");
        assert!(out.contains("(auto;"), "{out}");
        assert_eq!(preset(&a), QualityPreset::of(QualityLevel::Low));
        assert_eq!(a.world().resource::<FpsBadgeNote>().0, "8 km auto");
        for bad in [
            "view 0",
            "view 61",
            "view auto maybe",
            "view target",
            "view far",
        ] {
            assert!(run(&mut a, bad).contains("error"), "{bad}");
        }
        // The desktop's auto target: the display's refresh, 60 where it is
        // not known (headless).
        let mut d = app(GpuApi::Native);
        let out = run(&mut d, "view");
        assert!(
            out.contains("view: unlimited (auto; limit the high preset's unlimited"),
            "{out}"
        );
        assert!(
            out.contains("target 60 fps: the display's refresh is not known"),
            "{out}"
        );
    }

    /// A quality level changes the limit the dynamic mode starts from; a
    /// fixed distance overrides the preset's view distance both ways.
    #[test]
    fn the_quality_preset_and_the_render_distance() {
        let mut a = app(GpuApi::Native);
        run(&mut a, "quality medium");
        assert_eq!(preset(&a), QualityPreset::of(QualityLevel::Medium));
        run(&mut a, "view 60 auto off");
        let p = preset(&a);
        assert_eq!(p.view_m, Some(60_000.0));
        assert_eq!(p.tree_view_m, Some(3_000.0), "Medium's trees");
        assert_eq!(p.house_lod_scale, 0.75);
        run(&mut a, "quality low");
        let p = preset(&a);
        assert_eq!(p.view_m, Some(60_000.0), "the slider wins");
        assert_eq!(p.tree_view_m, Some(1_200.0), "Low's trees");
        assert_eq!(p.shadow_cascades, 1);
    }

    /// The settings drive it at the start and on a change, in both apps
    /// (the menu's page has the slider, the mode, the target and what
    /// runs).
    #[test]
    fn settings_and_the_menu_page() {
        let mut a = App::new();
        a.add_plugins((
            MinimalPlugins,
            ConsoleCorePlugin,
            crate::settings::SettingsPlugin,
            crate::quality::QualityPlugin,
        ));
        a.update();
        run(&mut a, "set display.view_distance 12");
        run(&mut a, "set display.view_distance_auto off");
        assert_eq!(preset(&a).view_m, Some(12_000.0));
        let s = a.world().resource::<ViewDistanceState>();
        assert_eq!(s.setting, DistanceSetting::Km(12));
        assert!(!s.auto);
        let rows = crate::menu::sub_rows(a.world(), ocw_client::menu::pages::Page::ViewDistance);
        assert_eq!(rows[0].value.as_deref(), Some("12 km"));
        assert_eq!(
            rows[3].value.as_deref(),
            Some("12 km, fixed, target 60 fps")
        );
        let graphics = crate::menu::sub_rows(a.world(), ocw_client::menu::pages::Page::Graphics);
        assert!(graphics.iter().any(|r| r.value.as_deref() == Some("12 km")));
    }

    /// The dynamic mode's steps move the tile ring at most every 10 s.
    #[test]
    fn the_tile_ring_waits_between_changes() {
        assert_eq!(next_ring(None, 1, 3.0), (1, 3.0));
        assert_eq!(next_ring(Some((2, 0.0)), 1, 4.0), (2, 0.0), "too soon");
        assert_eq!(next_ring(Some((2, 0.0)), 1, 10.0), (1, 10.0));
        assert_eq!(next_ring(Some((1, 10.0)), 1, 50.0), (1, 10.0));
    }

    /// A screenshot run holds the distance at the slider.
    #[test]
    fn frozen_runs_hold_the_slider() {
        let mut a = app(GpuApi::Native);
        a.insert_resource(ViewDistanceContext { frozen: true });
        let out = run(&mut a, "view 20");
        assert!(out.contains("held for this screenshot"), "{out}");
        assert_eq!(preset(&a).view_m, Some(20_000.0));
    }
}
