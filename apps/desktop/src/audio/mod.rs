//! Game audio (spec §3.3, menu increment M5 in §3.2.1).
//!
//! - [`mix`]: the buses (master × music / effects / voice) and the volume
//!   maths, pure;
//! - [`ride`]: speed / cadence / power / surface → ride sound parameters
//!   (pure) and their synthesis (wind, tyres, freewheel, chain);
//! - [`spatial`]: where a sound is heard (distance, direction, air,
//!   Doppler), pure;
//! - [`others`]: the other riders' bikes – character per rider, nearest
//!   voices and the group layer (pure), and their synthesis;
//! - [`cues`]: one-shot sounds (UI, countdown, finish, workout steps, gear
//!   shift, test tones) and when they play (pure);
//! - [`mixer`]: renders all of it into a device buffer;
//! - [`record`]: the game sound of video recordings (device tap, or
//!   rendered in step with game time for frame capture), WAV and mux;
//! - [`engine`]: the output backend trait and the engine thread (device
//!   choice by name, fallback to the default, hot-plugging); [`cpal_backend`]
//!   is the real output, [`engine::NullBackend`] none;
//! - this module: the Bevy glue (ride state and UI changes → mixer
//!   messages) and the console commands `audio.*`.
//!
//! Every sound is synthesised in code here – our own work, no sample files
//! and no third-party recordings (spec §13). Without an output device
//! (headless runs, CI) the app runs silently and logs one line.

pub mod cpal_backend;
pub mod cues;
#[cfg(test)]
mod demo;
pub mod dsp;
pub mod engine;
pub mod mix;
pub mod mixer;
pub mod others;
pub mod record;
pub mod ride;
pub mod spatial;

use std::time::Instant;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::menu::MenuView;

use crate::connection::ClientState;
use crate::settings::UserSettings;
use crate::settings::pages::AudioDevices;
use cues::{
    ContactRider, ContactWatch, Cue, MenuSnapshot, contact_cues, countdown_cue, finish_cue,
    menu_cue, workout_cue,
};
use engine::{AudioEngine, AudioStatus, NullBackend, Output};
use mix::{Bus, Mix};
use mixer::MixerMsg;
use ride::{RideInput, RideParams, ride_params};

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
                (sync_status, feed_ride, feed_others, play_cues)
                    .run_if(resource_exists::<GameAudio>),
            );
        register_commands(app);
    }
}

#[derive(Resource)]
struct BackendChoice(Backend);

/// The output devices for the Audio page (from the engine's status).
#[derive(Resource, Default, PartialEq)]
pub struct AudioDeviceList(pub AudioDevices);

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

/// The mix and output from the settings (`audio.*`).
pub fn settings_mix(settings: &crate::settings::registry::Settings) -> (Mix, Output) {
    let int = |key: &str| match settings.get(key) {
        Ok(crate::settings::registry::Value::Int(v)) => v.clamp(0, 100) as u8,
        _ => 0,
    };
    let mix = Mix {
        master: int("audio.master"),
        music: int("audio.music"),
        effects: int("audio.effects"),
        voice: int("audio.voice"),
        others: int("audio.others"),
        mute: settings
            .get("audio.mute")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let output = Output::from_setting(
        &settings
            .get("audio.output")
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    (mix, output)
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

/// The local rider's ride sounds, every frame (only sent when they change).
fn feed_ride(client: Option<Res<ClientState>>, audio: Res<GameAudio>, mut last: Local<RideParams>) {
    let params = client
        .as_deref()
        .filter(|c| c.0.rider_id().is_some())
        .map_or_else(RideParams::default, |c| {
            let core = &c.0;
            // Unicorn mode: the wind of the flight (spec §14 Phase 13C).
            if let Some(u) = core.god().unicorn(0.0) {
                return ride::flight_params(u.pose.speed);
            }
            let state = core.state();
            ride_params(&RideInput {
                speed_mps: state.speed,
                cadence_rpm: core.cadence_rpm(),
                power_w: core.power_w(),
                surface: ocw_sim::ride::surface(core.track(), &state),
            })
        });
    if params != *last {
        audio.engine.send(MixerMsg::Ride(params));
        *last = params;
    }
}

/// The other riders' sounds (spec §3.3): every rider in the view but the
/// own (bots, remote riders; the ghost is not in `riders` and stays
/// silent), heard at the own rider's head. Sent every frame while anyone
/// is around, once more (empty, to fade them out) when nobody is.
fn feed_others(
    view: Option<Res<RideView>>,
    audio: Res<GameAudio>,
    mut playing: Local<Vec<ocw_core::RiderId>>,
    mut sent_empty: Local<bool>,
) {
    let frame = view
        .as_deref()
        .and_then(|v| {
            let me = v.local()?;
            let listener = others::listener(v.track(me.id), &me.state);
            let riders: Vec<others::OtherRider> = v
                .riders
                .iter()
                .filter(|r| !r.is_local)
                .map(|r| {
                    let track = v.track(r.id);
                    let (pos, velocity) =
                        others::on_track(track, &r.state, others::SOURCE_HEIGHT_M);
                    others::OtherRider {
                        id: r.id,
                        pos,
                        velocity,
                        speed_mps: r.state.speed,
                        power_w: r.power_w,
                        cadence_rpm: r.cadence_rpm,
                        surface: ocw_sim::ride::surface(track, &r.state),
                    }
                })
                .collect();
            (!riders.is_empty()).then(|| others::frame(&listener, &riders, &playing))
        })
        .unwrap_or_default();
    let empty = frame.voices.is_empty() && frame.group.riders == 0;
    if empty && *sent_empty {
        return;
    }
    *sent_empty = empty;
    *playing = frame.voices.iter().map(|v| v.id).collect();
    audio.engine.send(MixerMsg::Others(frame));
}

/// What the cue detection saw last frame.
#[derive(Default)]
struct CueWatch {
    menu: MenuSnapshot,
    countdown: Option<String>,
    results: bool,
    workout: Option<u64>,
    /// Unicorn mode: flying, and the wing beat's phase.
    flying: bool,
    wing: f64,
    /// The hoof beats' phase.
    hoof: f64,
    /// Bumps, crashes, remounts, the proximity cue (spec §14 Phase 12C).
    contacts: ContactWatch,
}

/// UI sounds from the menu's changes, the countdown beeps and the finish
/// chime from the event view, the step cue from the workout.
fn play_cues(
    time: Res<Time>,
    menu: Option<Res<MenuView>>,
    ride: Option<Res<RideView>>,
    workouts: Option<Res<crate::workout::Workouts>>,
    audio: Res<GameAudio>,
    mut watch: Local<CueWatch>,
) {
    let mut cues = Vec::new();
    if let Some(m) = menu {
        let now = MenuSnapshot {
            open: m.open,
            title: m.title.clone(),
            focused: m.focused,
            label: m.items.get(m.focused).cloned().unwrap_or_default(),
        };
        cues.extend(menu_cue(&watch.menu, &now));
        // The Audio page shows the devices: read them again.
        if now.open && now.title == "Audio" && watch.menu.title != "Audio" {
            audio.engine.refresh();
        }
        watch.menu = now;
    }
    if let Some(r) = ride {
        let me = r.local().map_or(0.0, |l| l.state.distance);
        let riders: Vec<ContactRider> = r
            .riders
            .iter()
            .map(|v| ContactRider {
                id: v.id.0,
                is_local: v.is_local,
                distance_m: v.state.distance - me,
                wobble: v.wobble,
                crash: v
                    .crash
                    .map(|c| c.phase == ocw_sim::collide::CrashPhase::Remount),
            })
            .collect();
        cues.extend(contact_cues(&mut watch.contacts, &riders, r.proximity));
        let event = r.event.as_ref();
        let countdown = event.and_then(|e| e.countdown.clone());
        cues.extend(countdown_cue(
            watch.countdown.as_deref(),
            countdown.as_deref(),
        ));
        let results = event.is_some_and(|e| e.results.is_some());
        cues.extend(finish_cue(watch.results, results));
        watch.countdown = countdown;
        watch.results = results;
        // Unicorn mode: the chime of the transformation (in and out), soft
        // wing beats while the wings beat (spec §14 Phase 13C).
        let flying = r.flight.is_some();
        if flying != watch.flying {
            cues.push(Cue::GodChime);
            watch.flying = flying;
        }
        let local = r.unicorns.iter().find(|u| u.is_local && !u.landing);
        let beat = local.map_or(0.0, |u| {
            ocw_sim::god::wing_beat_hz(u.climb_mps, u.accel, u.speed_mps)
        });
        if beat > 0.0 {
            let before = watch.wing;
            watch.wing += beat * time.delta_secs_f64();
            if watch.wing.floor() > before.floor() {
                cues.push(Cue::WingBeat);
            }
        }
        // Muffled hooves on air while slow (spec §14 Phase 13C).
        let hooves = local.map_or(0.0, |u| ocw_sim::god::hoof_beat_hz(u.speed_mps));
        if hooves > 0.0 {
            let before = watch.hoof;
            watch.hoof += hooves * time.delta_secs_f64();
            if watch.hoof.floor() > before.floor() {
                cues.push(Cue::Hoof);
            }
        }
    }
    if let Some(w) = workouts {
        let n = w.0.lap_count();
        if let Some(prev) = watch.workout {
            cues.extend(workout_cue(prev, n));
        }
        watch.workout = Some(n);
    }
    for c in cues {
        audio.play(c);
    }
}

fn on_off(s: &str) -> Result<Option<bool>, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(Some(true)),
        "off" | "false" | "no" | "0" => Ok(Some(false)),
        "toggle" => Ok(None),
        _ => Err(format!("expected on, off or toggle, not {s:?}")),
    }
}

fn mix_line(m: &Mix) -> String {
    format!(
        "master {} %, effects {} %, others {} %, music {} %, voice {} %{}",
        m.master,
        m.effects,
        m.others,
        m.music,
        m.voice,
        if m.mute { ", muted" } else { "" }
    )
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
