//! `hr.test` (Dev): a made-up heart rate for scenes and tests of the HUD's
//! heart (spec 3.2 *Heart rate*) in both apps – a reading per second into
//! the ride client, as a strap would send it (`ocw_render::heart::HrProgram`):
//!
//! - `hr.test 142` – steady;
//! - `hr.test 110 165 30` – from 110 to 165 bpm over 30 s, then steady;
//! - `hr.test 120 165 10 --hold 6` – 120 bpm for 6 s first;
//! - `hr.test 140 140 0 --noise 1.5` – with a jitter of ± 1.5 bpm;
//! - `hr.test off` – stops (no heart rate: the heart goes).

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_devices::DeviceEvent;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::heart::HrProgram;

pub struct HrTestPlugin;

impl Plugin for HrTestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HrTest>()
            .add_systems(Update, feed.before(crate::RideViewSet));
        app.console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("hr.test")
                .about("A test heart rate (a reading per second): hr.test BPM | hr.test FROM TO SECONDS [--hold S] [--noise BPM] | hr.test off")
                .arg(Arg::new("from").required(true).help("bpm, or off"))
                .arg(Arg::new("to").help("bpm at the end of the ramp"))
                .arg(Arg::new("seconds").help("the ramp's length (s)"))
                .arg(
                    Arg::new("hold")
                        .long("hold")
                        .value_parser(clap::value_parser!(f64))
                        .help("seconds at FROM before the ramp"),
                )
                .arg(
                    Arg::new("noise")
                        .long("noise")
                        .value_parser(clap::value_parser!(f64))
                        .help("a jitter of ± this many bpm"),
                ),
            |world, m| {
                let num = |k: &str| -> Result<Option<f64>, String> {
                    m.get_one::<String>(k)
                        .map(|v| v.parse::<f64>().map_err(|_| format!("{k}: not a number: {v}")))
                        .transpose()
                };
                let from = m.get_one::<String>("from").expect("required");
                if from == "off" {
                    let was = world.resource_mut::<HrTest>().program.take();
                    if was.is_some()
                        && let Some(mut c) = crate::client_mut(world)
                    {
                        c.on_device_event(DeviceEvent::HeartRate {
                            bpm: 0.0,
                            timestamp: Default::default(),
                        });
                    }
                    return Ok("test heart rate off".into());
                }
                let from = num("from")?.expect("required");
                let to = num("to")?.unwrap_or(from);
                let ramp_s = num("seconds")?.unwrap_or(0.0).max(0.0);
                if !(30.0..=230.0).contains(&from) || !(30.0..=230.0).contains(&to) {
                    return Err("expected 30 to 230 bpm".into());
                }
                let noise = m.get_one::<f64>("noise").copied().unwrap_or(0.0).abs();
                let start = world.resource::<Time>().elapsed_secs_f64();
                let hold_s = m.get_one::<f64>("hold").copied().unwrap_or(0.0).max(0.0);
                let program = HrProgram {
                    hold_s,
                    from,
                    to,
                    ramp_s,
                    noise,
                };
                *world.resource_mut::<HrTest>() = HrTest {
                    program: Some((program, start)),
                    last: None,
                };
                Ok(if ramp_s > 0.0 {
                    format!("test heart rate {from:.0} → {to:.0} bpm over {ramp_s:.0} s")
                } else {
                    format!("test heart rate {from:.0} bpm")
                })
            },
        ));
    }
}

/// The running test heart rate: its program and start (game time, s), and
/// the last second sent.
#[derive(Resource, Debug, Default)]
pub struct HrTest {
    pub program: Option<(HrProgram, f64)>,
    last: Option<u64>,
}

fn feed(world: &mut World) {
    let Some((program, start)) = world.resource::<HrTest>().program else {
        return;
    };
    let t = (world.resource::<Time>().elapsed_secs_f64() - start).max(0.0);
    let second = t.floor() as u64;
    if world.resource::<HrTest>().last == Some(second) {
        return;
    }
    let Some(mut c) = crate::client_mut(world) else {
        return;
    };
    c.on_device_event(DeviceEvent::HeartRate {
        bpm: program.at(t),
        timestamp: std::time::Duration::from_secs_f64(t),
    });
    world.resource_mut::<HrTest>().last = Some(second);
}
