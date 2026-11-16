//! Game audio (spec §3.3, menu increment M5 in §3.2.1) on the desktop.
//!
//! The sounds, their parameters and the mixer are the shared crate
//! `ocw-audio` (its `mix`, `ride`, `spatial`, `others`, `weather`, `cues`,
//! `mixer` and `dsp`; [`cues`], [`mix`] and [`mixer`] re-exported); what they hear of
//! the ride is `ocw_client::audio::AudioFeed` – both shared with the browser
//! app (spec §14 Phase 10 B5). Desktop-only here:
//!
//! - [`record`]: the game sound of video recordings (device tap, or
//!   rendered in step with game time for frame capture), WAV and mux;
//! - [`engine`]: the output backend trait and the engine thread (device
//!   choice by name, fallback to the default, hot-plugging); [`cpal_backend`]
//!   is the real output, [`engine::NullBackend`] none;
//! - this module: the Bevy glue (the feed's messages → the engine) and the
//!   console commands `audio.*`.
//!
//! Every sound is synthesised in code – our own work, no sample files and
//! no third-party recordings (spec §13). Without an output device
//! (headless runs, CI) the app runs silently and logs one line.

pub mod cpal_backend;
pub mod engine;
pub mod record;
pub use ocw_audio::{cues, mix, mixer};

use std::time::Instant;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::audio::{AudioFeed, FeedFrame, mix_line, on_off};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::menu::MenuView;

use crate::connection::ClientState;
use crate::settings::UserSettings;
use crate::settings::pages::AudioDevices;
use cues::Cue;
use engine::{AudioEngine, AudioStatus, NullBackend, Output};
use mix::{Bus, Mix};
use mixer::MixerMsg;

/// Which output the plugin uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The system's audio devices (cpal).
    Device,
    /// No sound (`--no-audio`, frame capture, tests).
    Null,
}

pub struct GameAudioPlugin {
    pub backend: Backend,
}

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BackendChoice(self.backend))
            .init_resource::<AudioDeviceList>()
            .add_systems(Startup, start_engine)
            .add_systems(
                Update,
                (sync_status, feed, refresh_on_audio_page).run_if(resource_exists::<GameAudio>),
            );
        register_commands(app);
    }
}

#[derive(Resource)]
struct BackendChoice(Backend);

/// The output devices for the Audio page (from the engine's status;
/// shared with the browser app: `ocw_ui::menu::AudioDeviceMenu`).
pub use ocw_ui::menu::AudioDeviceMenu as AudioDeviceList;

/// The running engine and the mix it plays with.
#[derive(Resource)]
pub struct GameAudio {
    engine: AudioEngine,
    pub mix: Mix,
    pub output: Output,
    status: AudioStatus,
}

impl GameAudio {
    fn send_gains(&self) {
        self.engine.send(MixerMsg::Gains(self.mix.gains()));
    }

    pub fn play(&self, cue: Cue) {
        self.engine.send(MixerMsg::Cue(cue, Instant::now()));
    }

    /// Any mixer message (the ride photographer's shutter and drone).
    pub fn send(&self, msg: MixerMsg) {
        self.engine.send(msg);
    }

    #[cfg(test)]
    pub fn status(&self) -> &AudioStatus {
        &self.status
    }

    /// The mixer, and whether an output device plays it now (a video
    /// recording then taps the device's buffers, else renders it itself).
    pub fn recording_source(&self) -> (engine::SharedMixer, bool) {
        (self.engine.mixer().clone(), self.status.current.is_some())
    }
}

/// The mix and output from the settings (`audio.*`; the mix is the shared
/// `ocw_client::audio::settings_mix`).
pub fn settings_mix(settings: &crate::settings::registry::Settings) -> (Mix, Output) {
    let (mix, output) = ocw_client::audio::settings_mix(settings);
    (mix, Output::from_setting(&output))
}

fn start_engine(world: &mut World) {
    let (mix, output) = world
        .get_resource::<UserSettings>()
        .map(|s| settings_mix(&s.settings))
        .unwrap_or((Mix::default(), Output::Default));
    let backend = world.resource::<BackendChoice>().0;
    let gains = mix.gains();
    let engine = match backend {
        Backend::Device => {
            AudioEngine::start(cpal_backend::CpalBackend::new, output.clone(), gains)
        }
        Backend::Null => {
            info!("audio: off (no output for this run)");
            AudioEngine::start(|| NullBackend, output.clone(), gains)
        }
    };
    world.insert_resource(GameAudio {
        engine,
        mix,
        output,
        status: AudioStatus::default(),
    });
}

/// Copies the engine's status (devices, current output) for the menu.
fn sync_status(mut audio: ResMut<GameAudio>, mut devices: ResMut<AudioDeviceList>) {
    let s = audio.engine.status();
    if s.generation == audio.status.generation {
        return;
    }
    devices.set_if_neq(AudioDeviceList(AudioDevices {
        names: s.devices.clone(),
        current: s.current.clone(),
        fell_back: s.fell_back,
    }));
    audio.status = s;
}

/// The game sounds' input from the ride, every frame (the shared feed:
/// changes only).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn feed(
    time: Res<Time>,
    client: Option<Res<ClientState>>,
    view: Option<Res<RideView>>,
    tunnels: Option<Res<ocw_render::tunnel::Tunnels>>,
    menu: Option<Res<MenuView>>,
    workouts: Option<Res<crate::workout::Workouts>>,
    audio: Res<GameAudio>,
    mut feed: Local<AudioFeed>,
) {
    let msgs = feed.frame(&FeedFrame {
        client: client.as_deref().map(|c| &c.0),
        view: view.as_deref(),
        tunnels: tunnels.as_deref(),
        menu: menu.as_deref(),
        workout_laps: workouts.as_deref().map(|w| w.0.lap_count()),
        dt_s: time.delta_secs_f64(),
    });
    for m in msgs {
        audio.engine.send(m);
    }
}

/// The Audio page shows the devices: read them again when it opens.
fn refresh_on_audio_page(menu: Option<Res<MenuView>>, audio: Res<GameAudio>, mut was: Local<bool>) {
    let on = menu.is_some_and(|m| m.open && m.title == "Audio");
    if on && !*was {
        audio.engine.refresh();
    }
    *was = on;
}

fn audio_mut(world: &mut World) -> Result<Mut<'_, GameAudio>, String> {
    world
        .get_resource_mut::<GameAudio>()
        .ok_or_else(|| "audio is not running".to_string())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.volume")
            .about("Set a volume for this session (set audio.master … saves it)")
            .arg(
                Arg::new("bus")
                    .required(true)
                    .value_parser(["master", "music", "effects", "voice", "others"]),
            )
            .arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(value_parser!(u8).range(0..=100)),
            ),
        |world, m| {
            let bus = m.get_one::<String>("bus").expect("required").clone();
            let p = *m.get_one::<u8>("percent").expect("required");
            let mut a = audio_mut(world)?;
            match Bus::parse(&bus) {
                Some(b) => a.mix.set_volume(b, p),
                None => a.mix.master = p,
            }
            a.send_gains();
            Ok(format!("audio: {}", mix_line(&a.mix)))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.mute")
            .about("Mute or unmute all sound (set audio.mute … saves it)")
            .arg(Arg::new("state").default_value("toggle")),
        |world, m| {
            let state = on_off(m.get_one::<String>("state").expect("default"))?;
            let mut a = audio_mut(world)?;
            a.mix.mute = state.unwrap_or(!a.mix.mute);
            a.send_gains();
            Ok(format!("audio: {}", if a.mix.mute { "muted" } else { "on" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.output")
            .about("Play on this output device (a name from audio.devices, or default)")
            .arg(Arg::new("name").required(true).num_args(1..)),
        |world, m| {
            let name = m
                .get_many::<String>("name")
                .expect("required")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let mut a = audio_mut(world)?;
            a.output = Output::from_setting(&name);
            a.engine.set_output(a.output.clone());
            // Only a known device list can tell that it is missing.
            let missing = !a.status.devices.is_empty()
                && engine::choose_device(&a.output, &a.status.devices).1;
            Ok(if !missing {
                format!("audio: output {}", a.output.setting())
            } else {
                format!(
                    "audio: output {} (not found now: the default plays until it appears)",
                    a.output.setting()
                )
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.devices").about("List the output devices (* = playing now)"),
        |world, _| {
            let a = audio_mut(world)?;
            a.engine.refresh();
            let s = &a.status;
            let mut out: Vec<String> = s
                .devices
                .iter()
                .map(|d| {
                    let mark = if s.current.as_ref() == Some(d) { '*' } else { ' ' };
                    let default = if s.default.as_ref() == Some(d) {
                        " (default)"
                    } else {
                        ""
                    };
                    format!("{mark} {d}{default}")
                })
                .collect();
            if out.is_empty() {
                out.push("no output devices".into());
            }
            out.push(format!("output setting: {}", a.output.setting()));
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.status").about("Show the output device and the mix"),
        |world, _| {
            let a = audio_mut(world)?;
            let msg = if a.status.message.is_empty() {
                "starting".to_string()
            } else {
                a.status.message.clone()
            };
            Ok(format!("audio: {msg}\nmix: {}", mix_line(&a.mix)))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.play")
            .about("Play a sound: move, value, confirm, back, beep, go, finish, step, shift, test, test-music, test-effects, test-voice, test-others")
            .arg(Arg::new("sound").required(true)),
        |world, m| {
            let name = m.get_one::<String>("sound").expect("required");
            let cue = Cue::parse(name).ok_or_else(|| {
                let names: Vec<&str> = Cue::NAMES.iter().map(|(n, _)| *n).collect();
                format!("unknown sound {name:?}: {}", names.join(", "))
            })?;
            audio_mut(world)?.play(cue);
            Ok(String::new())
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::registry::Settings;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            ConsoleCorePlugin,
            GameAudioPlugin {
                backend: Backend::Null,
            },
        ));
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log().to_vec();
        let at = log.iter().rposition(|l| l == &format!("> {line}")).unwrap();
        log[at + 1..].to_vec()
    }

    #[test]
    fn mix_defaults_match_the_settings_registry() {
        let (mix, output) = settings_mix(&Settings::default());
        assert_eq!(mix, Mix::default());
        assert_eq!(output, Output::Default);
        let (s, _) = Settings::parse(
            "[audio]\nmaster = 40\nmusic = 0\neffects = 100\nvoice = 55\nothers = 35\nmute = true\noutput = \"USB Headset\"\n",
        )
        .unwrap();
        let (mix, output) = settings_mix(&s);
        assert_eq!(
            mix,
            Mix {
                master: 40,
                music: 0,
                effects: 100,
                voice: 55,
                others: 35,
                mute: true
            }
        );
        assert_eq!(output, Output::Named("USB Headset".into()));
    }

    #[test]
    fn console_commands_change_the_mix_and_output() {
        let mut app = app();
        assert_eq!(
            run(&mut app, "audio.volume effects 50")[0],
            "audio: master 80 %, effects 50 %, others 80 %, music 60 %, voice 100 %"
        );
        assert_eq!(app.world().resource::<GameAudio>().mix.effects, 50);
        assert!(run(&mut app, "audio.volume effects 101")[0].starts_with("error"));
        assert!(run(&mut app, "audio.volume drums 10")[0].starts_with("error"));
        run(&mut app, "audio.volume others 25");
        assert_eq!(app.world().resource::<GameAudio>().mix.others, 25);
        assert!(run(&mut app, "audio.play test-others").is_empty());
        run(&mut app, "audio.volume master 30");
        assert_eq!(app.world().resource::<GameAudio>().mix.master, 30);
        assert_eq!(run(&mut app, "audio.mute")[0], "audio: muted");
        assert_eq!(run(&mut app, "audio.mute off")[0], "audio: on");
        assert!(run(&mut app, "audio.mute maybe")[0].starts_with("error"));
        assert_eq!(
            run(&mut app, "audio.output \"USB Headset\"")[0],
            "audio: output USB Headset"
        );
        assert_eq!(
            app.world().resource::<GameAudio>().output,
            Output::Named("USB Headset".into())
        );
        assert_eq!(
            run(&mut app, "audio.output default")[0],
            "audio: output default"
        );
        let devices = run(&mut app, "audio.devices");
        assert_eq!(devices, ["no output devices", "output setting: default"]);
        assert!(run(&mut app, "audio.play finish").is_empty());
        assert!(run(&mut app, "audio.play trumpet")[0].starts_with("error: unknown sound"));
    }

    /// Without a device (the null backend, like a headless run) the app
    /// starts and says so.
    #[test]
    fn starts_silently_without_a_device() {
        let mut app = app();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.update();
            let s = app.world().resource::<GameAudio>().status().clone();
            if !s.message.is_empty() {
                assert_eq!(s.current, None);
                assert!(s.message.starts_with("no audio output"), "{}", s.message);
                break;
            }
            assert!(Instant::now() < deadline, "no status");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(run(&mut app, "audio.status")[0].starts_with("audio: no audio output"));
    }
}
