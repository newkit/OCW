//! What the game sounds hear of the ride (spec §3.3), shared by the
//! desktop and the browser app (spec §14 Phase 10 B5): every frame the
//! [`AudioFeed`] turns the client state and the render view into the
//! [`MixerMsg`]s of [`ocw_audio::mixer::Mixer`] – the own bike, the other
//! riders, the weather and its thunder, and the cues (menu, countdown,
//! finish, workout steps, bumps and crashes, the buckled wheel's rub, the
//! unicorn's chime, wings and hooves). Only changes are sent. The apps
//! hand the messages to their output: the cpal engine thread on the
//! desktop, the Web Audio mixer in the browser.

use std::collections::BTreeSet;

use ocw_audio::cues::{
    ContactRider, ContactWatch, Cue, MenuSnapshot, contact_cues, countdown_cue, finish_cue,
    menu_cue, rub_hz, workout_cue,
};
use ocw_audio::mix::Mix;
use ocw_audio::mixer::MixerMsg;
use ocw_audio::others;
use ocw_audio::ride::{self, RideInput, RideParams, ride_params};
use ocw_audio::spatial::{self, Placement};
use ocw_audio::weather::{self, ThunderSound, WeatherParams};
use ocw_core::RiderId;
use ocw_render::RideView;
use ocw_render::menu::MenuView;
use ocw_render::traffic::VehicleCueKind;
use ocw_render::tunnel::Tunnels;
use ocw_sim::traffic::VehicleClass;
use web_time::Instant;

use crate::client::ClientCore;
use crate::settings::{Settings, Value};

/// One frame's inputs.
#[derive(Clone, Copy, Default)]
pub struct FeedFrame<'a> {
    /// The client (the own rider's speed, cadence, power, surface).
    pub client: Option<&'a ClientCore>,
    /// The ride as rendered (the others, the weather, events, crashes).
    pub view: Option<&'a RideView>,
    /// The tunnels (sheltered from the rain inside).
    pub tunnels: Option<&'a Tunnels>,
    /// The menu, for the UI cues.
    pub menu: Option<&'a MenuView>,
    /// The workout's change counter (`WorkoutSession::lap_count`).
    pub workout_laps: Option<u64>,
    /// Seconds since the last frame (the rhythm of the rub, wings and
    /// hooves).
    pub dt_s: f64,
}

/// What the cue detection saw last frame.
#[derive(Debug, Default)]
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
    /// The last road bump played (its `seq`, spec Phase 8 Q12).
    road_bump: u64,
    /// The last vehicle cue played (its `seq`, spec 18E T7).
    vehicle_cue: u64,
    /// A buckled wheel's rub phase (spec §14 Phase 12E).
    rub: f64,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct AudioFeed {
    ride: RideParams,
    others_playing: Vec<RiderId>,
    others_sent_empty: bool,
    weather: WeatherParams,
    heard_strikes: BTreeSet<u32>,
    watch: CueWatch,
    /// Aircraft were heard in the last message (spec 14 Phase 18, T8).
    aircraft_heard: bool,
    /// Road vehicles were heard in the last message, and which (T7).
    vehicles_heard: bool,
    vehicles_playing: Vec<u32>,
    /// UFOs were heard in the last message (T9), and the ones darting
    /// away already whooshed.
    ufos_heard: bool,
    ufos_away: Vec<u64>,
}

/// The mix from the settings (`audio.master` … `audio.mute`) and the
/// output's setting text (`audio.output`: a device name or `default`).
pub fn settings_mix(settings: &Settings) -> (Mix, String) {
    let int = |key: &str| match settings.get(key) {
        Ok(Value::Int(v)) => v.clamp(0, 100) as u8,
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
    let output = settings
        .get("audio.output")
        .map(|v| v.to_string())
        .unwrap_or_default();
    (mix, output)
}

/// The mix as the `audio.*` commands say it.
pub fn mix_line(m: &Mix) -> String {
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

/// An `on` / `off` / `toggle` argument (`None` = toggle).
pub fn on_off(s: &str) -> Result<Option<bool>, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(Some(true)),
        "off" | "false" | "no" | "0" => Ok(Some(false)),
        "toggle" => Ok(None),
        _ => Err(format!("expected on, off or toggle, not {s:?}")),
    }
}

/// The own rider's ride sounds from the client (silent before the join).
pub fn ride_params_of(client: Option<&ClientCore>) -> RideParams {
    client
        .filter(|c| c.rider_id().is_some())
        .map_or_else(RideParams::default, |core| {
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
        })
}

impl AudioFeed {
    /// The messages for this frame (changes only; cues stamped now).
    pub fn frame(&mut self, f: &FeedFrame) -> Vec<MixerMsg> {
        let mut out = Vec::new();
        let ride = ride_params_of(f.client);
        if ride != self.ride {
            out.push(MixerMsg::Ride(ride));
            self.ride = ride;
        }
        if let Some(m) = self.others(f.view) {
            out.push(m);
        }
        self.weather(f, &mut out);
        self.aircraft(f.view, &mut out);
        self.vehicles(f.view, &mut out);
        self.ufos(f.view, &mut out);
        let now = Instant::now();
        out.extend(self.cues(f).into_iter().map(|(c, at)| match at {
            Some(p) => MixerMsg::SpatialCue(c, p, now),
            None => MixerMsg::Cue(c, now),
        }));
        out
    }

    /// The other riders' sounds (spec §3.3): every rider in the view but
    /// the own (bots, remote riders; the ghost is not in `riders` and
    /// stays silent), heard at the own rider's head. Sent every frame
    /// while anyone is around, once more (empty, to fade them out) when
    /// nobody is.
    fn others(&mut self, view: Option<&RideView>) -> Option<MixerMsg> {
        let frame = view
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
                (!riders.is_empty())
                    .then(|| others::frame(&listener, &riders, &self.others_playing))
            })
            .unwrap_or_default();
        let empty = frame.voices.is_empty() && frame.group.riders == 0;
        if empty && self.others_sent_empty {
            return None;
        }
        self.others_sent_empty = empty;
        self.others_playing = frame.voices.iter().map(|v| v.id).collect();
        Some(MixerMsg::Others(frame))
    }

    /// The aircraft heard (spec 14 Phase 18, T8): the loudest two of the
    /// view's, each from where its sound now arriving left it; sent every
    /// frame while any is heard, once empty to fade them out.
    fn aircraft(&mut self, view: Option<&RideView>, out: &mut Vec<MixerMsg>) {
        use ocw_audio::aircraft::{AircraftSound, Engine, loudest};
        use ocw_sim::air::AirKind;
        let sounds = view
            .and_then(|v| v.air.as_ref())
            .map(|a| {
                loudest(
                    a.heard
                        .iter()
                        .filter_map(|h| {
                            let speed =
                                (h.vel[0].powi(2) + h.vel[1].powi(2) + h.vel[2].powi(2)).sqrt();
                            let (engine, power) = match h.kind {
                                AirKind::Airliner => (
                                    Engine::Jet,
                                    if h.on_ground && speed < 1.0 { 0.4 } else { 1.0 },
                                ),
                                AirKind::SmallPlane => (Engine::Prop, h.rotor),
                                AirKind::Helicopter => (Engine::Rotor, h.rotor),
                                AirKind::Glider => (Engine::Glider, (speed / 30.0).min(1.0)),
                                AirKind::Balloon if h.burner => (Engine::Burner, 1.0),
                                AirKind::Balloon => return None,
                            };
                            AircraftSound::placed(
                                h.id,
                                engine,
                                a.listener,
                                a.listener_forward,
                                h.pos,
                                h.vel,
                                power,
                            )
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        if sounds.is_empty() && !self.aircraft_heard {
            return;
        }
        self.aircraft_heard = !sounds.is_empty();
        out.push(MixerMsg::Aircraft(sounds));
    }

    /// The UFOs heard (spec 14 Phase 18, T9; with `traffic.sound`): the
    /// hum of the two loudest, and a whoosh where one darts away.
    fn ufos(&mut self, view: Option<&RideView>, out: &mut Vec<MixerMsg>) {
        use ocw_audio::aircraft::{AircraftSound, Engine, loudest};
        let Some(u) = view.and_then(|v| v.ufo.as_ref()).filter(|u| u.heard) else {
            if self.ufos_heard {
                out.push(MixerMsg::Ufos(Vec::new()));
                self.ufos_heard = false;
            }
            self.ufos_away.clear();
            return;
        };
        let place = |id: u64, pos: [f64; 3], vel: [f64; 3], power: f64| {
            AircraftSound::placed(
                id,
                Engine::Ufo,
                u.listener,
                u.listener_forward,
                pos,
                vel,
                power,
            )
        };
        let sounds = loudest(
            u.ufos
                .iter()
                .filter(|x| !x.departing)
                .filter_map(|x| place(x.id, x.pos, x.vel, f64::from(x.presence)))
                .collect(),
        );
        let now = Instant::now();
        for x in u.ufos.iter().filter(|x| x.departing) {
            if self.ufos_away.contains(&x.id) {
                continue;
            }
            self.ufos_away.push(x.id);
            if let Some(s) = place(x.id, x.pos, [0.0; 3], 1.0) {
                let d = ((x.pos[0] - u.listener[0]).powi(2)
                    + (x.pos[1] - u.listener[1]).powi(2)
                    + (x.pos[2] - u.listener[2]).powi(2))
                .sqrt();
                let p = ocw_audio::spatial::Placement {
                    distance_m: d,
                    gain: s.level(),
                    pan: 0.0,
                    left: s.left,
                    right: s.right,
                    air_cutoff_hz: s.cutoff_hz,
                    doppler: 1.0,
                };
                out.push(MixerMsg::SpatialCue(
                    ocw_audio::cues::Cue::UfoWhoosh,
                    p,
                    now,
                ));
            }
        }
        if self.ufos_away.len() > 32 {
            self.ufos_away.remove(0);
        }
        if sounds.is_empty() && !self.ufos_heard {
            return;
        }
        self.ufos_heard = !sounds.is_empty();
        out.push(MixerMsg::Ufos(sounds));
    }

    /// The road vehicles heard (spec 14 Phase 18, T7): the eight loudest
    /// near the own rider's head, with `traffic.sound` on; quieter under
    /// rain and wind. Sent every frame while any is heard, once empty to
    /// fade them out.
    fn vehicles(&mut self, view: Option<&RideView>, out: &mut Vec<MixerMsg>) {
        use ocw_audio::vehicles::{VehicleSound, select};
        let sounds = view
            .filter(|v| v.vehicle_sound && !v.vehicles.is_empty())
            .and_then(|v| {
                let me = v.local()?;
                let listener = others::listener(v.track(me.id), &me.state);
                let wet = v.weather.as_ref().is_some_and(|w| w.ground.wetness() > 0.3);
                let mask = vehicle_mask(v);
                Some(select(
                    v.vehicles
                        .iter()
                        .filter_map(|c| {
                            VehicleSound::placed(
                                c.id,
                                vehicle_kind(c.class),
                                &listener,
                                [c.pos[0], c.pos[1] + 0.6, c.pos[2]],
                                [c.heading[0] * c.speed, 0.0, c.heading[1] * c.speed],
                                wet,
                                mask,
                            )
                        })
                        .collect(),
                    &self.vehicles_playing,
                ))
            })
            .unwrap_or_default();
        if sounds.is_empty() && !self.vehicles_heard {
            return;
        }
        self.vehicles_heard = !sounds.is_empty();
        self.vehicles_playing = sounds.iter().map(|s| s.id).collect();
        out.push(MixerMsg::Vehicles(sounds));
    }

    /// The weather's sounds (spec §14 Phase 9, S4): rain, hail, drizzle,
    /// the gusts and the wet tyres (sent when they change), and each
    /// strike's thunder once it has travelled to the rider (three seconds
    /// per kilometre after the flash; in game time, so frame captures keep
    /// it).
    fn weather(&mut self, f: &FeedFrame, out: &mut Vec<MixerMsg>) {
        let Some(view) = f.view else { return };
        let params = view.weather.as_ref().map_or_else(Default::default, |w| {
            let speed = view.local().map_or(0.0, |l| l.state.speed as f32);
            let sheltered = ocw_render::sky::daylight_at_rider(Some(view), f.tunnels) < 0.5;
            weather::weather_params(&w.state, w.wind_mps, w.ground.wetness(), speed, sheltered)
        });
        if params != self.weather {
            out.push(MixerMsg::Weather(params));
            self.weather = params;
        }
        let Some(w) = &view.weather else { return };
        for s in &w.strikes {
            let arrives = s.flash_at_s + s.strike.thunder_delay_s();
            if self.heard_strikes.contains(&s.strike.id) || view.elapsed_s < arrives {
                continue;
            }
            self.heard_strikes.insert(s.strike.id);
            // Not a thunder that arrived long ago (a late join).
            if view.elapsed_s - arrives < 2.0
                && let Some(t) = ThunderSound::of(&s.strike)
            {
                out.push(MixerMsg::Thunder(t));
            }
        }
        if self.heard_strikes.len() > 4096 {
            let keep: BTreeSet<u32> = w.strikes.iter().map(|s| s.strike.id).collect();
            self.heard_strikes.retain(|id| keep.contains(id));
        }
    }

    /// UI sounds from the menu's changes, the countdown beeps and the
    /// finish chime from the event view, the step cue from the workout,
    /// the contact sounds, the rub, the unicorn's sounds. Another rider's
    /// bump or crash comes with where it is heard (spec §14 Phase 12C:
    /// placed like the others' bikes); every other cue plays in the centre.
    fn cues(&mut self, f: &FeedFrame) -> Vec<(Cue, Option<Placement>)> {
        let watch = &mut self.watch;
        let mut cues = Vec::new();
        let mut placed = Vec::new();
        if let Some(m) = f.menu {
            let now = MenuSnapshot {
                open: m.open,
                title: m.title.clone(),
                focused: m.focused,
                label: m.items.get(m.focused).cloned().unwrap_or_default(),
            };
            cues.extend(menu_cue(&watch.menu, &now));
            watch.menu = now;
        }
        if let Some(r) = f.view {
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
            for c in contact_cues(&mut watch.contacts, &riders, r.proximity) {
                placed.push((c.cue, c.from.and_then(|id| contact_place(r, RiderId(id)))));
            }
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
            // Unicorn mode: the chime of the transformation (in and out),
            // soft wing beats while the wings beat (spec §14 Phase 13C).
            let flying = r.flight.is_some();
            if flying != watch.flying {
                cues.push(Cue::GodChime);
                watch.flying = flying;
            }
            let local = r.unicorns.iter().find(|u| u.is_local && !u.landing);
            let beat = local.map_or(0.0, |u| {
                ocw_sim::god::wing_beat_hz(u.climb_mps, u.accel, u.speed_mps)
            });
            if tick(&mut watch.wing, beat, f.dt_s) {
                cues.push(Cue::WingBeat);
            }
            // A buckled wheel rubs once per turn (spec §14 Phase 12E).
            let rub = r.local().map_or(0.0, |l| {
                let bent = l.damage.map_or(0.0, |d| d.bent[0].max(d.bent[1]));
                rub_hz(bent, l.state.speed)
            });
            if tick(&mut watch.rub, rub, f.dt_s) {
                cues.push(Cue::Rub);
            }
            // Muffled hooves on air while slow (spec §14 Phase 13C).
            let hooves = local.map_or(0.0, |u| ocw_sim::god::hoof_beat_hz(u.speed_mps));
            if tick(&mut watch.hoof, hooves, f.dt_s) {
                cues.push(Cue::Hoof);
            }
        }
        // What the vehicles did (spec 18E T7): a honk and the whoosh of a
        // pass where the vehicle is, the soft cue of one closing in from
        // behind; once each, with `traffic.sound` on.
        if let Some(r) = f.view {
            for c in &r.vehicle_cues {
                if c.seq <= watch.vehicle_cue {
                    continue;
                }
                watch.vehicle_cue = c.seq;
                if !r.vehicle_sound {
                    continue;
                }
                let v = r.vehicles.iter().find(|v| v.id == c.vehicle);
                let heavy =
                    v.is_some_and(|v| matches!(v.class, VehicleClass::Lorry | VehicleClass::Bus));
                let cue = match c.kind {
                    VehicleCueKind::Honk if heavy => Cue::AirHorn,
                    VehicleCueKind::Honk => Cue::Horn,
                    VehicleCueKind::Passed => Cue::PassWhoosh,
                    VehicleCueKind::Approaching { .. } => {
                        placed.push((Cue::VehicleBehind, None));
                        continue;
                    }
                };
                placed.push((cue, v.and_then(|v| vehicle_place(r, v))));
            }
        }
        // Over road damage (spec Phase 8 Q12): the own wheel's thump or
        // tick, once per bump.
        if let Some(seen) = f.client.and_then(|c| c.road_bumps().last())
            && seen.seq != watch.road_bump
        {
            watch.road_bump = seen.seq;
            cues.push(seen.cue());
        }
        if let Some(n) = f.workout_laps {
            if let Some(prev) = watch.workout {
                cues.extend(workout_cue(prev, n));
            }
            watch.workout = Some(n);
        }
        // The contact cues where they were (the others' placed), then the
        // rest.
        placed.extend(cues.into_iter().map(|c| (c, None)));
        placed
    }
}

/// Where another rider's bump or crash is heard by the own rider (spec
/// §14 Phase 12C): at that rider's bike on the road, low (the fall is on
/// the ground), heard at the own head like the others' bikes
/// ([`spatial::place`]: distance, direction, air). `None` without the own
/// rider (played in the centre then).
fn contact_place(view: &RideView, id: RiderId) -> Option<Placement> {
    let me = view.local()?;
    let other = view.riders.iter().find(|r| r.id == id)?;
    let listener = others::listener(view.track(me.id), &me.state);
    let (pos, velocity) = others::on_track(view.track(id), &other.state, CONTACT_HEIGHT_M);
    Some(spatial::place(&listener, pos, velocity))
}

/// The sound class of a road vehicle.
pub fn vehicle_kind(c: VehicleClass) -> ocw_audio::vehicles::Kind {
    use ocw_audio::vehicles::Kind;
    match c {
        VehicleClass::Car => Kind::Car,
        VehicleClass::Van => Kind::Van,
        VehicleClass::Lorry => Kind::Lorry,
        VehicleClass::Bus => Kind::Bus,
        VehicleClass::Motorbike => Kind::Motorbike,
        VehicleClass::Tractor => Kind::Tractor,
    }
}

/// How much of the vehicles is heard under the weather (spec 18A:
/// everything quieter under wind and rain): 1 on a calm dry day, down to
/// 0.45 in a downpour with a gale.
pub fn vehicle_mask(view: &RideView) -> f64 {
    view.weather.as_ref().map_or(1.0, |w| {
        let rain = if w.state.precipitating() {
            (f64::from(w.state.precip_mm_h) / 8.0).min(1.0)
        } else {
            0.0
        };
        let wind = (f64::from(w.wind_mps) / 15.0).min(1.0);
        (1.0 - 0.35 * rain - 0.2 * wind).clamp(0.45, 1.0)
    })
}

/// Where a vehicle's horn or whoosh is heard by the own rider.
fn vehicle_place(view: &RideView, v: &ocw_render::traffic::VehicleView) -> Option<Placement> {
    let me = view.local()?;
    let listener = others::listener(view.track(me.id), &me.state);
    Some(spatial::place(
        &listener,
        [v.pos[0], v.pos[1] + 0.8, v.pos[2]],
        [v.heading[0] * v.speed, 0.0, v.heading[1] * v.speed],
    ))
}

/// Height of a contact's sound above the road (m): the bikes' wheels and
/// a fall on the ground.
pub const CONTACT_HEIGHT_M: f64 = 0.4;

/// Advances a rhythm's phase by `hz × dt`: whether it crossed a whole
/// beat (nothing while the rate is 0).
fn tick(phase: &mut f64, hz: f64, dt_s: f64) -> bool {
    if hz <= 0.0 {
        return false;
    }
    let before = *phase;
    *phase += hz * dt_s;
    phase.floor() > before.floor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::RiderView;
    use ocw_world::Route;

    fn rider(id: u32, d: f64, speed: f64, local: bool) -> RiderView {
        RiderView {
            id: RiderId(id),
            name: format!("R{id}"),
            state: ocw_sim::RiderState {
                distance: d,
                speed,
                ..Default::default()
            },
            power_w: 200.0,
            cadence_rpm: Some(90.0),
            heart_rate_bpm: None,
            is_local: local,
            bike: Default::default(),
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        }
    }

    #[test]
    fn mix_defaults_match_the_settings_registry() {
        let (mix, output) = settings_mix(&Settings::default());
        assert_eq!(mix, Mix::default());
        assert_eq!(output, "default");
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
        assert_eq!(output, "USB Headset");
    }

    #[test]
    fn rhythm_ticks_once_per_beat() {
        let mut phase = 0.0;
        let beats = (0..123)
            .filter(|_| tick(&mut phase, 2.0, 1.0 / 60.0))
            .count();
        assert_eq!(beats, 4, "2 Hz for 2.05 s");
        assert!(!tick(&mut phase, 0.0, 1.0));
    }

    /// Without a client nothing rides; a view with others sends their
    /// frame every frame, the empty frame once.
    #[test]
    fn others_are_sent_while_around_and_faded_out_once() {
        let mut feed = AudioFeed::default();
        // Nobody around: one empty frame (fading out whatever played), then
        // nothing.
        let first = feed.frame(&FeedFrame::default());
        assert!(
            first
                .iter()
                .all(|m| matches!(m, MixerMsg::Others(f) if f.voices.is_empty())),
            "{first:?}"
        );
        assert!(feed.frame(&FeedFrame::default()).is_empty(), "silence");
        let mut view = RideView::new(Route::test_loop());
        view.riders = vec![rider(1, 100.0, 9.0, true), rider(2, 104.0, 9.0, false)];
        for _ in 0..3 {
            let msgs = feed.frame(&FeedFrame {
                view: Some(&view),
                ..Default::default()
            });
            assert!(
                msgs.iter()
                    .any(|m| matches!(m, MixerMsg::Others(f) if f.voices.len() == 1)),
                "{msgs:?}"
            );
        }
        view.riders.truncate(1);
        let fade = feed.frame(&FeedFrame {
            view: Some(&view),
            ..Default::default()
        });
        assert!(
            fade.iter()
                .any(|m| matches!(m, MixerMsg::Others(f) if f.voices.is_empty()))
        );
        let after = feed.frame(&FeedFrame {
            view: Some(&view),
            ..Default::default()
        });
        assert!(!after.iter().any(|m| matches!(m, MixerMsg::Others(_))));
    }

    /// The cues: a menu opening, a crash nearby, the workout's steps, a
    /// buckled wheel's rub at speed.
    #[test]
    fn cues_from_the_menu_crashes_workouts_and_the_rub() {
        let mut feed = AudioFeed::default();
        let cues = |msgs: Vec<MixerMsg>| -> Vec<Cue> {
            msgs.into_iter()
                .filter_map(|m| match m {
                    MixerMsg::Cue(c, _) | MixerMsg::SpatialCue(c, _, _) => Some(c),
                    _ => None,
                })
                .collect()
        };
        let mut menu = MenuView {
            open: true,
            title: "Audio".into(),
            ..Default::default()
        };
        let got = cues(feed.frame(&FeedFrame {
            menu: Some(&menu),
            workout_laps: Some(0),
            ..Default::default()
        }));
        assert_eq!(got, [Cue::UiConfirm]);
        menu.open = false;
        let got = cues(feed.frame(&FeedFrame {
            menu: Some(&menu),
            workout_laps: Some(1),
            ..Default::default()
        }));
        assert_eq!(got, [Cue::UiBack, Cue::WorkoutStep]);
        // Another rider goes down 10 m ahead.
        let mut view = RideView::new(Route::test_loop());
        view.riders = vec![rider(1, 100.0, 9.0, true), rider(2, 110.0, 9.0, false)];
        feed.frame(&FeedFrame {
            view: Some(&view),
            ..Default::default()
        });
        view.riders[1].wobble = 0.1;
        let got = cues(feed.frame(&FeedFrame {
            view: Some(&view),
            ..Default::default()
        }));
        assert_eq!(got, [Cue::Bump]);
        // A buckled front wheel at 9 m/s: one rub per 2.1 m.
        view.riders[1].wobble = 0.0;
        view.riders[0].damage = Some(ocw_sim::damage::Visible {
            bent: [0.5, 0.0],
            ..Default::default()
        });
        let rubs: usize = (0..60)
            .map(|_| {
                cues(feed.frame(&FeedFrame {
                    view: Some(&view),
                    dt_s: 1.0 / 60.0,
                    ..Default::default()
                }))
                .iter()
                .filter(|c| **c == Cue::Rub)
                .count()
            })
            .sum();
        assert!((4..=5).contains(&rubs), "{rubs} rubs in 1 s at 9 m/s");
    }

    /// Spec §14 Phase 12C: another rider's crash and bump are heard where
    /// they happen (left or right, nearer or farther), the own crash in
    /// the centre.
    #[test]
    fn the_others_crashes_and_bumps_are_placed_the_own_is_not() {
        let crash = Some(ocw_sim::collide::CrashView {
            phase: ocw_sim::collide::CrashPhase::Falling,
            elapsed_s: 0.1,
            down_s: 3.0,
            kind: ocw_sim::collide::ContactKind::Side,
            by: None,
            impulse: [0.0, 0.0],
            speed: 8.0,
        });
        let heard = |feed: &mut AudioFeed, view: &RideView| -> Vec<(Cue, Option<Placement>)> {
            feed.frame(&FeedFrame {
                view: Some(view),
                ..Default::default()
            })
            .into_iter()
            .filter_map(|m| match m {
                MixerMsg::Cue(c, _) => Some((c, None)),
                MixerMsg::SpatialCue(c, p, _) => Some((c, Some(p))),
                _ => None,
            })
            .collect()
        };
        let mut view = RideView::new(Route::test_loop());
        let mut left = rider(2, 106.0, 9.0, false);
        left.state.lateral_offset = -2.5;
        let mut right = rider(3, 112.0, 9.0, false);
        right.state.lateral_offset = 2.5;
        let far = rider(4, 135.0, 9.0, false);
        view.riders = vec![rider(1, 100.0, 9.0, true), left, right, far];
        let mut feed = AudioFeed::default();
        heard(&mut feed, &view);
        view.riders[1].crash = crash;
        view.riders[2].wobble = 0.08;
        view.riders[3].crash = crash;
        let got = heard(&mut feed, &view);
        assert_eq!(got.len(), 3, "{got:?}");
        let (c, p) = got[0];
        let p = p.expect("placed");
        assert_eq!(c, Cue::Crash);
        assert!(p.pan < -0.2 && p.left > p.right, "on the left: {p:?}");
        let (c, p) = got[1];
        let near = p.expect("placed");
        assert_eq!(c, Cue::Bump);
        assert!(
            near.pan > 0.1 && near.right > near.left,
            "on the right: {near:?}"
        );
        let (c, p) = got[2];
        let far = p.expect("placed");
        assert_eq!(c, Cue::Crash);
        assert!(far.gain < 0.5 * near.gain, "farther, quieter: {far:?}");
        assert!(far.air_cutoff_hz < near.air_cutoff_hz);
        // The own crash: in the centre (not placed).
        view.riders[0].crash = crash;
        let got = heard(&mut feed, &view);
        assert_eq!(got, [(Cue::Crash, None)]);
    }
}
