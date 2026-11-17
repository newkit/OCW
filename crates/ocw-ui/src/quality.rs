//! The quality preset in both apps (spec §14 Phase 10 B7): the console's
//! `quality [auto|low|medium|high]` (what `display.quality` runs) and
//! `gpu` (the GPU API, the adapter and the quality that runs). The level
//! comes from `ocw_client::quality::resolve` – `auto` by the app's
//! [`QualityContext`] (the GPU API and the device) – and goes to the
//! renderer as `ocw_render::quality::RenderQuality`.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::quality::{DeviceInfo, GpuApi, QualitySetting, resolve};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::quality::{QualityLevel, QualityPreset, RenderQuality};

/// What `auto` decides by: the app's GPU API and the device (the desktop:
/// native, nothing known about the device).
#[derive(Resource, Debug, Clone, Default)]
pub struct QualityContext {
    pub api: GpuApi,
    pub device: DeviceInfo,
}

/// The chosen setting and the level it resolved to.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct QualityChoice {
    pub setting: QualitySetting,
    pub level: QualityLevel,
    pub reason: &'static str,
}

impl Default for QualityChoice {
    fn default() -> Self {
        Self {
            setting: QualitySetting::Auto,
            level: QualityLevel::High,
            reason: "auto: desktop",
        }
    }
}

pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QualityContext>()
            .init_resource::<QualityChoice>()
            .init_resource::<RenderQuality>()
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("quality")
                    .about("Graphics quality: auto (by GPU and device), low, medium or high; without an argument: what runs (spec §14 Phase 10 B7)")
                    .arg(Arg::new("level").value_parser(QualitySetting::CHOICES.to_vec())),
                |world, m| {
                    if let Some(word) = m.get_one::<String>("level") {
                        let setting = QualitySetting::parse(word).ok_or("unknown level")?;
                        set_quality(world, setting);
                    }
                    Ok(describe(world))
                },
            ))
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("gpu")
                    .about("The GPU API (WebGPU, WebGL2 or the desktop's), the adapter and the quality preset"),
                |world, _| Ok(gpu_text(world)),
            ))
            // The settings' start commands run only for values that are not
            // the default: `auto` is resolved here.
            .add_systems(PostStartup, apply_setting_at_start);
        // The render distance (10G) goes with the preset in both apps.
        if !app.is_plugin_added::<crate::view_distance::ViewDistancePlugin>() {
            app.add_plugins(crate::view_distance::ViewDistancePlugin);
        }
    }
}

/// Resolves `display.quality` once the app has set its [`QualityContext`].
fn apply_setting_at_start(world: &mut World) {
    let setting = world
        .get_resource::<crate::settings::UserSettings>()
        .and_then(|s| QualitySetting::parse(&s.value("display.quality").to_string()))
        .unwrap_or_default();
    set_quality(world, setting);
}

/// Applies a setting: resolves it and hands the preset to the renderer.
pub fn set_quality(world: &mut World, setting: QualitySetting) {
    let ctx = world
        .get_resource::<QualityContext>()
        .cloned()
        .unwrap_or_default();
    let (level, reason) = resolve(setting, ctx.api, &ctx.device);
    world.insert_resource(QualityChoice {
        setting,
        level,
        reason,
    });
    // With the render distance that runs (spec §14 Phase 10, 10G; at the
    // preset's own distance its numbers unchanged).
    let preset = crate::view_distance::adjusted(world, QualityPreset::of(level));
    if world.get_resource::<RenderQuality>().map(|q| q.0) != Some(preset) {
        world.insert_resource(RenderQuality(preset));
    }
}

/// "quality: low (auto: WebGL 2) – shadows …".
pub fn describe(world: &World) -> String {
    let c = world
        .get_resource::<QualityChoice>()
        .copied()
        .unwrap_or_default();
    let preset = world
        .get_resource::<RenderQuality>()
        .map(|q| q.0)
        .unwrap_or_default();
    let why = match c.setting {
        QualitySetting::Auto => c.reason.to_string(),
        QualitySetting::Fixed(_) => "chosen".into(),
    };
    format!("quality: {} ({why}) – {}", c.level, preset.summary())
}

/// The console's `gpu`: "gpu: WebGL2 – adapter …; quality: …".
pub fn gpu_text(world: &World) -> String {
    let api = world
        .get_resource::<QualityContext>()
        .map(|c| c.api)
        .unwrap_or_default();
    let adapter = world
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .map(|a| {
            let mut s = format!("{} ({:?}", a.name, a.backend);
            if !a.driver.is_empty() {
                s.push_str(&format!(", {}", a.driver));
            }
            s.push(')');
            s
        })
        .unwrap_or_else(|| "no adapter (headless)".into());
    let what = match api {
        GpuApi::Native => "native".to_string(),
        other => format!("{} (browser build)", other.name()),
    };
    format!("gpu: {what} – adapter {adapter}\n{}", describe(world))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};

    fn app(ctx: QualityContext) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin, QualityPlugin))
            .insert_resource(ctx);
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue(line.to_string());
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    /// `quality` resolves `auto` by the app's context and hands the preset
    /// to the renderer; a level by name wins; `gpu` says the API.
    #[test]
    fn the_console_sets_the_preset() {
        let mut web = app(QualityContext {
            api: GpuApi::WebGl2,
            device: DeviceInfo::default(),
        });
        let out = run(&mut web, "quality auto");
        assert!(out.contains("quality: low (auto: WebGL 2)"), "{out}");
        assert_eq!(
            web.world().resource::<RenderQuality>().0,
            QualityPreset::of(QualityLevel::Low)
        );
        let out = run(&mut web, "quality high");
        assert!(out.contains("quality: high (chosen)"), "{out}");
        assert_eq!(
            web.world().resource::<RenderQuality>().0.level,
            QualityLevel::High
        );
        let out = run(&mut web, "gpu");
        assert!(out.contains("gpu: WebGL2 (browser build)"), "{out}");
        assert!(run(&mut web, "quality ultra").contains("error"));

        // The desktop: auto is High, the look it always had.
        let mut desktop = app(QualityContext::default());
        let out = run(&mut desktop, "quality auto");
        assert!(out.contains("quality: high (auto: desktop)"), "{out}");
        assert_eq!(
            desktop.world().resource::<RenderQuality>().0,
            QualityPreset::default()
        );
        assert!(run(&mut desktop, "gpu").contains("gpu: native"));
    }
}
