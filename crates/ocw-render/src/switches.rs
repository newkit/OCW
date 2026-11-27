//! Switches that each turn off one part of the renderer, to find what
//! makes a phone's browser close the page (spec 14 Phase 10, 10G
//! *Bisecting a phone's crash*): the iPhone of 2026-11-27 lost the page
//! within two seconds of the first frames, on the test route at Low.
//!
//! The browser app sets them from its address (`nosee=1`, `noshadow=1`, …,
//! `ios=safe` for all, `ocw_web::bisect`); the desktop leaves them off.
//! [`RenderSwitches`] is inserted before [`crate::OcwRenderPlugins`]; the
//! material extensions read the shader switches through [`install`] (their
//! shader functions have no world).
//!
//! Besides, the pipeline cache's compile budget and compile log
//! (`vendor/bevy_render`, a small patch): at most N pipelines start
//! compiling per frame (WebGL 2 compiles them on the page's thread, all of
//! a frame's at once before), and each compile is logged with its time.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::render::render_resource::{PipelineCache, PipelineCompileReport};
use bevy::render::{Render, RenderApp, RenderSystems};

/// What is turned off (all `false` / `None`: everything as built).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderSwitches {
    /// Trees without their see-through shader (`see_through.wesl`): Bevy's
    /// standard material shaders (no fade, sway or snow).
    pub no_see: bool,
    /// Road, ground, houses and gear without their texture-array shaders
    /// (`road.wesl`, `ground.wesl`, `building.wesl`, `gear.wesl`): the
    /// standard shaders of their base colours.
    pub no_arrays: bool,
    /// Water and the far terrain with the standard shaders too (with
    /// [`Self::no_see`] and [`Self::no_arrays`]: no shader of ours in the
    /// world).
    pub plain: bool,
    /// No shadow maps (sun, moon, lamps).
    pub no_shadow: bool,
    /// No multisampling on any camera.
    pub no_msaa: bool,
    /// The simplest sky: no cloud dome, no night sky (stars, moon).
    pub no_sky: bool,
    /// No egui (the console's overlay): commands still run (the page's
    /// `window.ocw.console`).
    pub no_egui: bool,
    /// At most this many pipelines start compiling per frame (`None`: all
    /// at once).
    pub compile_budget: Option<usize>,
    /// Log every pipeline compile (name, shader, ms).
    pub log_pipelines: bool,
    /// The cameras stop drawing after this many frames (`Some(0)`: none
    /// is drawn).
    pub frames: Option<u32>,
}

impl RenderSwitches {
    /// The switches that are on, as the address names them (for the log).
    pub fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (on, name) in [
            (self.no_see, "nosee"),
            (self.no_arrays, "noarrays"),
            (self.plain, "plain"),
            (self.no_shadow, "noshadow"),
            (self.no_msaa, "nomsaa"),
            (self.no_sky, "nosky"),
            (self.no_egui, "noegui"),
        ] {
            if on {
                out.push(name.to_string());
            }
        }
        if let Some(n) = self.compile_budget {
            out.push(format!("{n} pipeline(s) per frame"));
        }
        if let Some(n) = self.frames {
            out.push(format!("frames={n}"));
        }
        out
    }
}

static PLAIN_TREES: AtomicBool = AtomicBool::new(false);
static PLAIN_ARRAYS: AtomicBool = AtomicBool::new(false);
static PLAIN_OTHER: AtomicBool = AtomicBool::new(false);

/// Hands the shader switches to the material extensions (call before the
/// render plugins are added; the browser app does).
pub fn install(s: &RenderSwitches) {
    PLAIN_TREES.store(s.no_see || s.plain, Ordering::Relaxed);
    PLAIN_ARRAYS.store(s.no_arrays || s.plain, Ordering::Relaxed);
    PLAIN_OTHER.store(s.plain, Ordering::Relaxed);
}

/// The trees take Bevy's standard shaders ([`RenderSwitches::no_see`]).
pub fn plain_trees() -> bool {
    PLAIN_TREES.load(Ordering::Relaxed)
}

/// Road, ground, houses and gear take the standard shaders
/// ([`RenderSwitches::no_arrays`]).
pub fn plain_arrays() -> bool {
    PLAIN_ARRAYS.load(Ordering::Relaxed)
}

/// Water and the far terrain take the standard shaders
/// ([`RenderSwitches::plain`]).
pub fn plain_other() -> bool {
    PLAIN_OTHER.load(Ordering::Relaxed)
}

/// `shader`, or Bevy's default where `plain` says so.
pub fn shader_or_default(plain: bool, shader: bevy::shader::ShaderRef) -> bevy::shader::ShaderRef {
    if plain {
        bevy::shader::ShaderRef::Default
    } else {
        shader
    }
}

pub struct SwitchesPlugin;

impl Plugin for SwitchesPlugin {
    fn build(&self, app: &mut App) {
        let s = *app
            .world_mut()
            .get_resource_or_insert_with(RenderSwitches::default);
        install(&s);
        app.init_resource::<FramesDrawn>().add_systems(
            PostUpdate,
            apply_switches.after(crate::quality::apply_quality),
        );
        if s.no_sky {
            app.add_systems(Update, no_clouds);
        }
        if s.compile_budget.is_some() || s.log_pipelines {
            let (budget, log) = (s.compile_budget, s.log_pipelines);
            if let Some(render) = app.get_sub_app_mut(RenderApp) {
                render.add_systems(
                    Render,
                    (move |cache: Option<ResMut<PipelineCache>>| {
                        if let Some(mut cache) = cache
                            && (cache.compile_budget != budget
                                || cache.compile_log.is_some() != log)
                        {
                            cache.compile_budget = budget;
                            cache.compile_log =
                                log.then_some(log_compile as fn(&PipelineCompileReport));
                        }
                    })
                    .in_set(RenderSystems::PrepareAssets),
                );
            }
        }
        if !s.names().is_empty() {
            let names = s.names().join(", ");
            app.add_systems(Startup, move || info!("render switches: {names}"));
        }
    }
}

/// The frames the app has run (for [`RenderSwitches::frames`]).
#[derive(Resource, Debug, Default)]
pub struct FramesDrawn(pub u32);

/// Shadows, multisampling and the cameras, every frame after the quality
/// preset (which would turn them on again).
#[allow(clippy::type_complexity)] // a Bevy system query
fn apply_switches(
    s: Res<RenderSwitches>,
    mut frames: ResMut<FramesDrawn>,
    mut commands: Commands,
    mut suns: Query<&mut DirectionalLight>,
    mut points: Query<&mut PointLight>,
    mut spots: Query<&mut SpotLight>,
    mut cams: Query<(Entity, &mut Camera, Option<&Msaa>)>,
) {
    frames.0 = frames.0.saturating_add(1);
    if s.no_shadow {
        for mut l in &mut suns {
            if l.shadow_maps_enabled {
                l.shadow_maps_enabled = false;
            }
        }
        for mut l in &mut points {
            if l.shadow_maps_enabled {
                l.shadow_maps_enabled = false;
            }
        }
        for mut l in &mut spots {
            if l.shadow_maps_enabled {
                l.shadow_maps_enabled = false;
            }
        }
    }
    let stop = s.frames.is_some_and(|n| frames.0 > n);
    if stop && s.frames.is_some_and(|n| frames.0 == n + 1) {
        info!(
            "frames={}: the cameras stop drawing",
            s.frames.unwrap_or_default()
        );
    }
    for (e, mut cam, msaa) in &mut cams {
        if s.no_msaa && msaa != Some(&Msaa::Off) {
            commands.entity(e).insert(Msaa::Off);
        }
        if stop && cam.is_active {
            cam.is_active = false;
        }
    }
}

/// No cloud dome ([`RenderSwitches::no_sky`]).
fn no_clouds(fx: Option<ResMut<crate::weather::WeatherFx>>) {
    if let Some(mut fx) = fx
        && fx.clouds
    {
        fx.clouds = false;
    }
}

/// How long every pipeline compile is logged (s from the first).
pub const LOG_ALL_S: f32 = 30.0;

/// The pipelines already reported as waiting for a shader (once each).
static WAITING: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

/// The pipeline cache's compile log ([`RenderSwitches::log_pipelines`]).
fn log_compile(r: &PipelineCompileReport) {
    // Every compile in the first LOG_ALL_S, later only slow ones and
    // failures (a new material in a long ride).
    static START: std::sync::OnceLock<bevy::platform::time::Instant> = std::sync::OnceLock::new();
    let early = START
        .get_or_init(bevy::platform::time::Instant::now)
        .elapsed()
        .as_secs_f32()
        < LOG_ALL_S;
    if !early && r.outcome != "error" && r.ms < 50.0 {
        return;
    }
    if r.outcome == "waiting" {
        let mut w = WAITING.lock().unwrap_or_else(|e| e.into_inner());
        if !w.get_or_insert_with(HashSet::new).insert(r.id) {
            return;
        }
    }
    info!("{}", compile_line(r));
}

/// One compile as a log line: "pipeline 12 ok in 85 ms: pbr_opaque_mesh_pipeline
/// (embedded://ocw_render/road.wesl), 4 more queued".
pub fn compile_line(r: &PipelineCompileReport) -> String {
    let what = if r.label.is_empty() {
        "(no label)"
    } else {
        r.label
    };
    let shader = if r.shader.is_empty() {
        String::new()
    } else {
        format!(" ({})", r.shader)
    };
    let outcome = match r.outcome {
        "waiting" => "waits for a shader or import".to_string(),
        o => format!("{o} in {:.0} ms", r.ms),
    };
    format!(
        "pipeline {} {outcome}: {what}{shader}, {} more queued",
        r.id, r.queued
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_off_by_default() {
        let s = RenderSwitches::default();
        assert!(s.names().is_empty());
        install(&s);
        assert!(!plain_trees() && !plain_arrays() && !plain_other());
        let custom = || bevy::shader::ShaderRef::Path("embedded://x.wesl".into());
        assert!(matches!(
            shader_or_default(false, custom()),
            bevy::shader::ShaderRef::Path(_)
        ));
        assert!(matches!(
            shader_or_default(true, custom()),
            bevy::shader::ShaderRef::Default
        ));
    }

    #[test]
    fn names_list_what_is_on() {
        let s = RenderSwitches {
            no_see: true,
            no_shadow: true,
            compile_budget: Some(1),
            frames: Some(10),
            ..default()
        };
        assert_eq!(
            s.names(),
            ["nosee", "noshadow", "1 pipeline(s) per frame", "frames=10"]
        );
    }

    #[test]
    fn a_compile_reads_as_one_line() {
        let r = PipelineCompileReport {
            id: 12,
            label: "pbr_opaque_mesh_pipeline",
            shader: "embedded://ocw_render/road.wesl",
            ms: 85.4,
            outcome: "ok",
            queued: 4,
        };
        assert_eq!(
            compile_line(&r),
            "pipeline 12 ok in 85 ms: pbr_opaque_mesh_pipeline (embedded://ocw_render/road.wesl), 4 more queued"
        );
        let w = PipelineCompileReport {
            outcome: "waiting",
            label: "",
            shader: "",
            ..r
        };
        assert_eq!(
            compile_line(&w),
            "pipeline 12 waits for a shader or import: (no label), 4 more queued"
        );
    }

    /// The switches act on the scene: shadows off, no MSAA, the cameras
    /// stop after N frames.
    #[test]
    fn shadows_msaa_and_frames() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RenderSwitches {
                no_shadow: true,
                no_msaa: true,
                frames: Some(2),
                ..default()
            })
            .init_resource::<FramesDrawn>()
            .add_systems(Update, apply_switches);
        let sun = app
            .world_mut()
            .spawn(DirectionalLight {
                shadow_maps_enabled: true,
                ..default()
            })
            .id();
        let cam = app
            .world_mut()
            .spawn((Camera::default(), Msaa::Sample4))
            .id();
        app.update();
        let w = app.world();
        assert!(!w.get::<DirectionalLight>(sun).unwrap().shadow_maps_enabled);
        assert_eq!(w.get::<Msaa>(cam), Some(&Msaa::Off));
        assert!(w.get::<Camera>(cam).unwrap().is_active);
        app.update();
        assert!(app.world().get::<Camera>(cam).unwrap().is_active);
        app.update();
        assert!(
            !app.world().get::<Camera>(cam).unwrap().is_active,
            "after 2 frames"
        );
    }
}
