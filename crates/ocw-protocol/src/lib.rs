//! The Open Cycling World game protocol (spec §6.2).
//!
//! Messages are defined in `proto/ocw/v1/messages.proto` and generated with
//! `prost`. This crate also converts between wire types and simulation types,
//! including the quantization of rider state. It is transport-independent:
//! the in-process local server and the QUIC transport (Phase 5) carry the same
//! encoded bytes.

pub mod v1 {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/ocw.v1.rs"));
}

mod weather;
pub use weather::{
    WeatherChange, WeatherInfo, event_weather_from_wire, event_weather_to_wire, session_weather,
    weather_change, weather_from_wire, weather_info, weather_request, weather_to_wire,
};

use ocw_core::RiderId;
use ocw_sim::{Event, EventKind, EventPhase, EventSpec, RiderState, Standing, TurnChoice};
use ocw_world::graph::{EdgeId, VertexId};
use ocw_world::{RoadNetwork, Segment, TurnReason};
use prost::Message;

/// Version exchanged in `Hello` / `Welcome`.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("message has no content")]
    Empty,
}

pub fn encode<M: Message>(msg: &M) -> Vec<u8> {
    msg.encode_to_vec()
}

pub fn decode_client(bytes: &[u8]) -> Result<v1::client_message::Msg, ProtocolError> {
    v1::ClientMessage::decode(bytes)?
        .msg
        .ok_or(ProtocolError::Empty)
}

pub fn decode_server(bytes: &[u8]) -> Result<v1::server_message::Msg, ProtocolError> {
    v1::ServerMessage::decode(bytes)?
        .msg
        .ok_or(ProtocolError::Empty)
}

pub fn client_message(msg: v1::client_message::Msg) -> v1::ClientMessage {
    v1::ClientMessage { msg: Some(msg) }
}

pub fn server_message(msg: v1::server_message::Msg) -> v1::ServerMessage {
    v1::ServerMessage { msg: Some(msg) }
}

impl From<ocw_core::RiderProfile> for v1::RiderProfile {
    fn from(p: ocw_core::RiderProfile) -> Self {
        Self {
            rider_mass_kg: p.rider_mass_kg as f32,
            bike_mass_kg: p.bike_mass_kg as f32,
            cda_m2: p.cda_m2 as f32,
            crr: p.crr as f32,
        }
    }
}

impl From<v1::RiderProfile> for ocw_core::RiderProfile {
    fn from(p: v1::RiderProfile) -> Self {
        Self {
            rider_mass_kg: f64::from(p.rider_mass_kg),
            bike_mass_kg: f64::from(p.bike_mass_kg),
            cda_m2: f64::from(p.cda_m2),
            crr: f64::from(p.crr),
            ..Self::default()
        }
    }
}

/// The full look with the character (Hello, BikeChange, RiderBuild).
impl From<ocw_core::RiderLook> for v1::RiderLook {
    fn from(l: ocw_core::RiderLook) -> Self {
        let c = l.character.validated();
        Self {
            height_cm: Some(u32::from(c.height_cm)),
            body_build: Some(c.build.wire()),
            skin: Some(u32::from(c.skin)),
            face: Some(u32::from(c.face)),
            hair: Some(u32::from(c.hair)),
            hair_rgb: Some(c.hair_colour.0),
            glasses: Some(u32::from(c.glasses)),
            ..snapshot_look(&l)
        }
    }
}

/// The look as the snapshots carry it: only the colours and the pattern
/// (the character comes once per change in `RiderBuild.look`, spec §14
/// Phase 4, 4C).
pub fn snapshot_look(l: &ocw_core::RiderLook) -> v1::RiderLook {
    v1::RiderLook {
        frame_rgb: l.frame.0,
        jersey_rgb: l.jersey.0,
        helmet_rgb: l.helmet.0,
        kit_pattern: l.pattern.wire(),
        ..Default::default()
    }
}

/// The character of a look from the network, validated (absent fields:
/// the default's).
pub fn character(l: &v1::RiderLook) -> ocw_core::Character {
    let d = ocw_core::Character::default();
    let small = |v: Option<u32>, d: u8| v.map_or(d, |v| v.min(255) as u8);
    ocw_core::Character {
        height_cm: l
            .height_cm
            .map_or(d.height_cm, |h| h.min(u32::from(u16::MAX)) as u16),
        build: l.body_build.map_or(d.build, ocw_core::BodyBuild::from_wire),
        skin: small(l.skin, d.skin),
        face: small(l.face, d.face),
        hair: small(l.hair, d.hair),
        hair_colour: l.hair_rgb.map_or(d.hair_colour, ocw_core::Rgb::from_wire),
        glasses: small(l.glasses, d.glasses),
    }
    .validated()
}

/// A look from the network, validated (spec §2.5): colours are cut to
/// 24-bit sRGB, unknown patterns are plain.
impl From<&v1::RiderLook> for ocw_core::RiderLook {
    fn from(l: &v1::RiderLook) -> Self {
        Self {
            frame: ocw_core::Rgb::from_wire(l.frame_rgb),
            jersey: ocw_core::Rgb::from_wire(l.jersey_rgb),
            helmet: ocw_core::Rgb::from_wire(l.helmet_rgb),
            pattern: ocw_core::KitPattern::from_wire(l.kit_pattern),
            character: character(l),
        }
    }
}

/// The bike (physics profile, which picks the model) and look of a
/// received rider update; `None` for the look when the server sent none.
pub fn rider_look(u: &v1::RiderUpdate) -> (ocw_core::ProfileId, Option<ocw_core::RiderLook>) {
    (
        ocw_core::ProfileId::from_wire(Some(&u.bike_profile)),
        u.look.as_ref().map(ocw_core::RiderLook::from),
    )
}

/// Quantizes a simulation snapshot entry for the wire.
pub fn quantize(s: &ocw_sim::RiderSnapshot) -> v1::RiderUpdate {
    v1::RiderUpdate {
        rider_id: s.id.0,
        distance_cm: (s.state.distance * 100.0)
            .round()
            .clamp(0.0, u32::MAX as f64) as u32,
        speed_mm_s: (s.state.speed * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32,
        lateral_offset_cm: (s.state.lateral_offset * 100.0).round() as i32,
        animation_state: 0,
        last_input_seq: s.last_input_seq,
        power_w: s.power_w.round().max(0.0) as u32,
        display_name: s.name.clone(),
        bike_profile: s.bike.id().into(),
        look: Some(snapshot_look(&s.look)),
        heading_mrad: (s.state.heading * 1000.0).round() as i32,
        turn_curvature_e4: (s.state.turn_curvature * 1e4).round() as i32,
        mode: if s.god.is_some() {
            v1::RiderMode::God
        } else {
            v1::RiderMode::Riding
        } as i32,
        god: s.god.as_ref().map(|g| god_pose_to_wire(g, 0, false)),
        // The server marks its Echo rider (spec §7 *Echo bot*).
        echo: false,
        crash: s.crash.as_ref().map(crash_to_wire),
        // The server adds the credited hearts (spec §14 Phase 4, 4E).
        hearts: 0,
        build_hash: s.build_hash,
        bump: s.bump.as_ref().map(|b| v1::Bump {
            tick: b.tick,
            strength_mrad: (b.strength * 1000.0).round().max(0.0) as u32,
            side: if b.side < 0.0 { -1 } else { 1 },
        }),
    }
}

/// The collision model on the wire (spec §14 Phase 12A).
pub fn collision_mode_to_wire(mode: ocw_sim::collide::Mode) -> i32 {
    use ocw_sim::collide::Mode;
    (match mode {
        Mode::Off => v1::CollisionMode::Off,
        Mode::Soft => v1::CollisionMode::Soft,
        Mode::Full => v1::CollisionMode::Full,
    }) as i32
}

/// A received collision model (unknown or absent: an older server, off).
pub fn collision_mode_from_wire(v: i32) -> ocw_sim::collide::Mode {
    use ocw_sim::collide::Mode;
    match v1::CollisionMode::try_from(v) {
        Ok(v1::CollisionMode::Soft) => Mode::Soft,
        Ok(v1::CollisionMode::Full) => Mode::Full,
        _ => Mode::Off,
    }
}

fn contact_kind_to_wire(k: ocw_sim::collide::ContactKind) -> v1::ContactKind {
    use ocw_sim::collide::ContactKind as K;
    match k {
        K::Side => v1::ContactKind::Side,
        K::WheelToWheel => v1::ContactKind::WheelToWheel,
        K::WheelTouch => v1::ContactKind::WheelTouch,
        K::RearEnd => v1::ContactKind::RearEnd,
        K::HeadOn => v1::ContactKind::HeadOn,
        K::Object => v1::ContactKind::Object,
    }
}

fn contact_kind_from_wire(v: i32) -> ocw_sim::collide::ContactKind {
    use ocw_sim::collide::ContactKind as K;
    match v1::ContactKind::try_from(v) {
        Ok(v1::ContactKind::WheelToWheel) => K::WheelToWheel,
        Ok(v1::ContactKind::WheelTouch) => K::WheelTouch,
        Ok(v1::ContactKind::RearEnd) => K::RearEnd,
        Ok(v1::ContactKind::HeadOn) => K::HeadOn,
        Ok(v1::ContactKind::Object) => K::Object,
        _ => K::Side,
    }
}

/// A crash for the wire (spec §14 Phase 12C).
pub fn crash_to_wire(c: &ocw_sim::collide::CrashView) -> v1::CrashState {
    use ocw_sim::collide::CrashPhase as P;
    let ms = |s: f64| (s * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32;
    let mm = |v: f64| (v * 1000.0).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    v1::CrashState {
        phase: (match c.phase {
            P::Falling => v1::CrashPhase::Falling,
            P::Down => v1::CrashPhase::Down,
            P::Remount => v1::CrashPhase::Remount,
        }) as i32,
        elapsed_ms: ms(c.elapsed_s),
        down_ms: ms(c.down_s),
        kind: contact_kind_to_wire(c.kind) as i32,
        by_rider: c.by.map_or(0, |r| r.0),
        impulse_along_mm_s: mm(c.impulse[0]),
        impulse_right_mm_s: mm(c.impulse[1]),
        speed_mm_s: ms(c.speed),
    }
}

/// A received rider's crash; `None` while riding (or from older servers).
pub fn rider_crash(u: &v1::RiderUpdate) -> Option<ocw_sim::collide::CrashView> {
    use ocw_sim::collide::CrashPhase as P;
    let c = u.crash.as_ref()?;
    let phase = match v1::CrashPhase::try_from(c.phase).ok()? {
        v1::CrashPhase::None => return None,
        v1::CrashPhase::Falling => P::Falling,
        v1::CrashPhase::Down => P::Down,
        v1::CrashPhase::Remount => P::Remount,
    };
    Some(ocw_sim::collide::CrashView {
        phase,
        elapsed_s: f64::from(c.elapsed_ms) / 1000.0,
        down_s: f64::from(c.down_ms) / 1000.0,
        kind: contact_kind_from_wire(c.kind),
        by: (c.by_rider != 0).then_some(RiderId(c.by_rider)),
        impulse: [
            f64::from(c.impulse_along_mm_s) / 1000.0,
            f64::from(c.impulse_right_mm_s) / 1000.0,
        ],
        speed: f64::from(c.speed_mm_s) / 1000.0,
    })
}

/// A received rider's last soft bump.
pub fn rider_bump(u: &v1::RiderUpdate) -> Option<ocw_sim::collide::Bump> {
    let b = u.bump.as_ref()?;
    Some(ocw_sim::collide::Bump {
        tick: b.tick,
        strength: f64::from(b.strength_mrad) / 1000.0,
        side: if b.side < 0 { -1.0 } else { 1.0 },
    })
}

/// The rules request (spec §14 Phase 12A: `ride.collisions`,
/// `ride.chaotic_bots`).
pub fn rules_request(collisions: Option<bool>, chaotic_bots: Option<bool>) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Rules(v1::RulesRequest {
        collisions,
        chaotic_bots,
    }))
}

/// A staged contact (`dev.bump` / `dev.crash`, local servers only).
pub fn stage_contact(crash: bool) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Dev(v1::DevCommand {
        cmd: Some(v1::dev_command::Cmd::Stage(v1::StageContact { crash })),
    }))
}

/// A unicorn pose for the wire (spec §14 Phase 13E).
pub fn god_pose_to_wire(p: &ocw_sim::god::GodPose, seq: u32, teleport: bool) -> v1::GodPose {
    v1::GodPose {
        seq,
        x_m: p.pos[0],
        y_m: p.pos[1],
        z_m: p.pos[2],
        yaw_rad: p.yaw as f32,
        pitch_rad: p.pitch as f32,
        speed_mps: p.speed as f32,
        teleport,
    }
}

/// A received unicorn pose.
pub fn god_pose_from_wire(p: &v1::GodPose) -> ocw_sim::god::GodPose {
    ocw_sim::god::GodPose {
        pos: [p.x_m, p.y_m, p.z_m],
        yaw: f64::from(p.yaw_rad),
        pitch: f64::from(p.pitch_rad),
        speed: f64::from(p.speed_mps),
    }
}

/// The unicorn's pose of a received rider update, if the rider is in god
/// mode.
pub fn rider_god(u: &v1::RiderUpdate) -> Option<ocw_sim::god::GodPose> {
    (u.mode == v1::RiderMode::God as i32)
        .then(|| u.god.as_ref().map(god_pose_from_wire).unwrap_or_default())
}

/// God mode requests (spec §14 Phase 13): enter, or leave (land at the
/// road nearest (x, z), or fly back).
pub fn god_enter() -> v1::ClientMessage {
    client_message(v1::client_message::Msg::God(v1::GodRequest {
        req: Some(v1::god_request::Req::Enter(true)),
    }))
}

pub fn god_leave(fly_back: bool, x_m: f64, z_m: f64) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::God(v1::GodRequest {
        req: Some(v1::god_request::Req::Leave(v1::GodLeave {
            fly_back,
            x_m,
            z_m,
        })),
    }))
}

/// A free-ride path change for the wire (edges by stable ID).
pub fn path_update(c: &ocw_sim::PathChange, net: &RoadNetwork) -> v1::PathUpdate {
    v1::PathUpdate {
        rider_id: c.rider.0,
        reset: c.reset,
        first_index: c.first_index,
        start_m: c.start_m,
        steps: c
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Edge { edge, from_m, to_m } => v1::PathStep {
                    edge_id: net.edge(edge).id.0,
                    from_m,
                    to_m,
                    turnaround: 0,
                },
                Segment::Turnaround(r) => v1::PathStep {
                    edge_id: 0,
                    from_m: 0.0,
                    to_m: 0.0,
                    turnaround: match r {
                        TurnReason::DeadEnd => 1,
                        TurnReason::Boundary => 2,
                        TurnReason::Manual => 3,
                    },
                },
            })
            .collect(),
    }
}

/// The path segments of a received update; fails on edges this client's
/// world package does not have.
pub fn path_segments(u: &v1::PathUpdate, net: &RoadNetwork) -> Result<Vec<Segment>, String> {
    u.steps
        .iter()
        .map(|s| {
            Ok(match s.turnaround {
                0 => {
                    let edge = net
                        .edge_index(EdgeId(s.edge_id))
                        .ok_or_else(|| format!("unknown edge {}", s.edge_id))?;
                    let len = net.geometry(edge).length();
                    Segment::Edge {
                        edge,
                        from_m: s.from_m.clamp(0.0, len),
                        to_m: s.to_m.clamp(0.0, len),
                    }
                }
                1 => Segment::Turnaround(TurnReason::DeadEnd),
                2 => Segment::Turnaround(TurnReason::Boundary),
                _ => Segment::Turnaround(TurnReason::Manual),
            })
        })
        .collect()
}

/// An exit choice for the wire (`None` = no choice, the default exit).
pub fn turn_intent(
    seq: u32,
    junction: VertexId,
    choice: Option<&TurnChoice>,
    net: &RoadNetwork,
) -> v1::TurnIntent {
    v1::TurnIntent {
        seq,
        junction_id: junction.0,
        exit_edge_id: choice.map(|c| net.edge(c.edge).id.0),
        exit_forward: choice.is_some_and(|c| c.forward),
    }
}

/// The exit choice of a received intent (`None`: no choice, or an edge
/// this server does not know).
pub fn turn_choice(t: &v1::TurnIntent, net: &RoadNetwork) -> Option<TurnChoice> {
    let edge = net.edge_index(EdgeId(t.exit_edge_id?))?;
    ((t.junction_id as usize) < net.graph().vertices.len()).then_some(TurnChoice {
        junction: VertexId(t.junction_id),
        edge,
        forward: t.exit_forward,
    })
}

/// The world package identity of a road network (Hello / Welcome, spec
/// §6.5): its package name and graph fingerprint.
pub fn world_id(net: &RoadNetwork) -> v1::WorldPackageId {
    v1::WorldPackageId {
        name: net.package.clone(),
        graph_hash: net.fingerprint(),
    }
}

/// Converts a received rider update back to simulation units.
pub fn dequantize(u: &v1::RiderUpdate) -> (RiderId, RiderState) {
    (
        RiderId(u.rider_id),
        RiderState {
            distance: f64::from(u.distance_cm) / 100.0,
            speed: f64::from(u.speed_mm_s) / 1000.0,
            lateral_offset: f64::from(u.lateral_offset_cm) / 100.0,
            heading: f64::from(u.heading_mrad) / 1000.0,
            turn_curvature: f64::from(u.turn_curvature_e4) / 1e4,
        },
    )
}

/// The race mode on the wire (spec §14 Phase 4, 4G).
pub fn physics_mode_to_wire(mode: ocw_sim::PhysicsMode) -> i32 {
    use ocw_sim::PhysicsMode as M;
    (match mode {
        M::Open => v1::PhysicsMode::Open,
        M::Profile => v1::PhysicsMode::Profile,
        M::Equal => v1::PhysicsMode::Equal,
    }) as i32
}

/// A received race mode (`None`: unspecified or unknown).
pub fn physics_mode_from_wire(v: i32) -> Option<ocw_sim::PhysicsMode> {
    use ocw_sim::PhysicsMode as M;
    match v1::PhysicsMode::try_from(v).ok()? {
        v1::PhysicsMode::Open => Some(M::Open),
        v1::PhysicsMode::Profile => Some(M::Profile),
        v1::PhysicsMode::Equal => Some(M::Equal),
        v1::PhysicsMode::Unspecified => None,
    }
}

/// An event request with the organiser's race mode (`None`: by its kind).
pub fn event_start_with(
    spec: &EventSpec,
    bots: Option<u32>,
    physics: Option<ocw_sim::PhysicsMode>,
) -> v1::EventStart {
    v1::EventStart {
        physics: physics.map(physics_mode_to_wire),
        ..event_start(spec, bots)
    }
}

/// The organiser's weather of an event request (spec §14 Phase 9, S5;
/// `None`: the session's weather stays).
pub fn event_weather(s: &v1::EventStart) -> Option<ocw_weather::machine::EventWeather> {
    s.weather.as_ref().and_then(event_weather_from_wire)
}

/// The organiser's race mode of an event request (`None`: by its kind).
pub fn event_physics(s: &v1::EventStart) -> Option<ocw_sim::PhysicsMode> {
    s.physics.and_then(physics_mode_from_wire)
}

/// A rider's bike build for the wire (spec §14 Phase 4, 4G).
pub fn rider_build(
    rider: RiderId,
    profile: ocw_core::ProfileId,
    build: Option<&ocw_sim::Build>,
    look: Option<&ocw_core::RiderLook>,
) -> v1::RiderBuild {
    let stock = ocw_sim::Build::stock(profile);
    let b = build.unwrap_or(&stock);
    let e4 = |x: f64| (x * 1e4).round() as i32;
    v1::RiderBuild {
        rider_id: rider.0,
        build_hash: b.hash(),
        bike_profile: b.profile.id().into(),
        parts: b.parts.clone(),
        flat_gain_e4: e4(b.flat_gain),
        climb_gain_e4: e4(b.climb_gain),
        bike_mass_g: (b.params.bike_mass_kg * 1000.0).round().max(0.0) as u32,
        capped: b.capped(),
        look: look.map(|l| (*l).into()),
    }
}

/// A received bike build.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildInfo {
    pub rider: RiderId,
    pub hash: u32,
    pub profile: ocw_core::ProfileId,
    pub parts: Vec<String>,
    /// The server's effect against the profile's stock bike (4G).
    pub effect: ocw_sim::build::Effect,
    /// The rider's full look with the character (4C; `None`: an older
    /// server).
    pub look: Option<ocw_core::RiderLook>,
}

/// Decodes a bike build.
pub fn build_info(b: &v1::RiderBuild) -> BuildInfo {
    BuildInfo {
        rider: RiderId(b.rider_id),
        hash: b.build_hash,
        profile: ocw_core::ProfileId::from_wire(Some(&b.bike_profile)),
        parts: b.parts.clone(),
        effect: ocw_sim::build::Effect {
            flat: f64::from(b.flat_gain_e4) / 1e4,
            climb: f64::from(b.climb_gain_e4) / 1e4,
            mass_kg: f64::from(b.bike_mass_g) / 1000.0,
            capped: b.capped,
        },
        look: b.look.as_ref().map(ocw_core::RiderLook::from),
    }
}

/// An event request for the wire (`bots`: the field size, if given).
pub fn event_start(spec: &EventSpec, bots: Option<u32>) -> v1::EventStart {
    let len = |l: Option<f64>| l.map(|m| m.round().clamp(0.0, u32::MAX as f64) as u32);
    let (kind, laps, length_m, target_w) = match *spec {
        EventSpec::GroupRide { target_w } => (
            v1::EventKind::GroupRide,
            None,
            None,
            target_w.map(|w| w.round().max(0.0) as u32),
        ),
        EventSpec::Race { laps, length_m } => (v1::EventKind::Race, laps, len(length_m), None),
        EventSpec::TimeTrial { laps, length_m } => {
            (v1::EventKind::TimeTrial, laps, len(length_m), None)
        }
    };
    v1::EventStart {
        kind: kind as i32,
        laps,
        length_m,
        bots,
        target_w,
        collisions: None,
        physics: None,
        weather: None,
    }
}

/// The request of a received event start (`None`: unknown kind).
pub fn event_spec(s: &v1::EventStart) -> Option<EventSpec> {
    let length_m = s.length_m.map(f64::from);
    Some(match v1::EventKind::try_from(s.kind).ok()? {
        v1::EventKind::GroupRide => EventSpec::GroupRide {
            target_w: s.target_w.map(f64::from),
        },
        v1::EventKind::Race => EventSpec::Race {
            laps: s.laps,
            length_m,
        },
        v1::EventKind::TimeTrial => EventSpec::TimeTrial {
            laps: s.laps,
            length_m,
        },
        v1::EventKind::Unspecified => return None,
    })
}

/// The event state for the wire at server tick `tick` (`None`: no event).
pub fn event_state(ev: Option<&Event>, standings: &[Standing], tick: u64) -> v1::EventState {
    let Some(ev) = ev else {
        return v1::EventState {
            server_tick: tick,
            ..Default::default()
        };
    };
    let (kind, target_w) = match ev.kind {
        EventKind::GroupRide { target_w } => (v1::EventKind::GroupRide, target_w.round() as u32),
        EventKind::Race { .. } => (v1::EventKind::Race, 0),
        EventKind::TimeTrial { .. } => (v1::EventKind::TimeTrial, 0),
    };
    let ms = |s: f64| (s * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32;
    v1::EventState {
        event_id: ev.id,
        kind: kind as i32,
        phase: match ev.phase {
            EventPhase::Scheduled => v1::EventPhase::Scheduled,
            EventPhase::Countdown => v1::EventPhase::Countdown,
            EventPhase::Running => v1::EventPhase::Running,
            EventPhase::Finished => v1::EventPhase::Finished,
        } as i32,
        countdown_tick: ev.countdown_tick,
        go_tick: ev.go_tick,
        server_tick: tick,
        start_line_m: ev.start_line_m,
        length_m: ev.kind.length_m().unwrap_or(0.0),
        laps: ev.kind.laps().unwrap_or(0),
        target_w,
        standings: standings
            .iter()
            .map(|s| v1::EventStanding {
                rider_id: s.id.0,
                display_name: s.name.clone(),
                position: s.position,
                progress_cm: (s.progress_m * 100.0)
                    .round()
                    .clamp(i32::MIN as f64, i32::MAX as f64) as i32,
                finish_ms: s.finish_s.map(ms),
                dnf: s.dnf,
                gap_ahead_ms: s.gap_ahead_s.map(ms),
            })
            .collect(),
        finished_tick: ev.finished_tick,
        // The event's own rule (the server puts the model in force here).
        collisions: collision_mode_to_wire(ev.collisions.unwrap_or(match ev.kind {
            EventKind::GroupRide { .. } => ocw_sim::collide::Mode::Soft,
            _ => ocw_sim::collide::Mode::Full,
        })),
        physics: physics_mode_to_wire(ev.physics),
        // The organiser's weather: set by the server (it owns the weather).
        weather: None,
    }
}

/// A received event state, in simulation units.
#[derive(Debug, Clone, PartialEq)]
pub struct EventInfo {
    pub id: u32,
    pub kind: EventKind,
    pub phase: EventPhase,
    pub countdown_tick: u64,
    pub go_tick: u64,
    /// The server tick the state is from.
    pub server_tick: u64,
    pub start_line_m: f64,
    pub finished_tick: Option<u64>,
    pub standings: Vec<Standing>,
    /// The collision model of the event (spec §14 Phase 12A).
    pub collisions: ocw_sim::collide::Mode,
    /// The race mode (spec §14 Phase 4, 4G; an older server: open).
    pub physics: ocw_sim::PhysicsMode,
    /// The organiser's weather of the event, for its info ("thunderstorm";
    /// spec §14 Phase 9, S5; `None`: the session's weather).
    pub weather: Option<String>,
}

impl EventInfo {
    /// Seconds since GO at server tick `tick` (negative before).
    pub fn race_time_s(&self, tick: f64) -> f64 {
        (tick - self.go_tick as f64) * ocw_core::SIM_DT
    }
}

/// Decodes an event state; `None` = no event (none, or cancelled).
pub fn event_info(e: &v1::EventState) -> Option<EventInfo> {
    let phase = match v1::EventPhase::try_from(e.phase).ok()? {
        v1::EventPhase::None => return None,
        v1::EventPhase::Scheduled => EventPhase::Scheduled,
        v1::EventPhase::Countdown => EventPhase::Countdown,
        v1::EventPhase::Running => EventPhase::Running,
        v1::EventPhase::Finished => EventPhase::Finished,
    };
    let laps = (e.laps > 0).then_some(e.laps);
    let kind = match v1::EventKind::try_from(e.kind).ok()? {
        v1::EventKind::GroupRide => EventKind::GroupRide {
            target_w: f64::from(e.target_w),
        },
        v1::EventKind::Race => EventKind::Race {
            length_m: e.length_m,
            laps,
        },
        v1::EventKind::TimeTrial => EventKind::TimeTrial {
            length_m: e.length_m,
            laps,
        },
        v1::EventKind::Unspecified => return None,
    };
    Some(EventInfo {
        id: e.event_id,
        kind,
        phase,
        countdown_tick: e.countdown_tick,
        go_tick: e.go_tick,
        server_tick: e.server_tick,
        start_line_m: e.start_line_m,
        finished_tick: e.finished_tick,
        collisions: collision_mode_from_wire(e.collisions),
        physics: physics_mode_from_wire(e.physics).unwrap_or_default(),
        weather: e
            .weather
            .as_ref()
            .and_then(weather::event_weather_from_wire)
            .map(|w| w.label()),
        standings: e
            .standings
            .iter()
            .map(|s| Standing {
                id: RiderId(s.rider_id),
                name: s.display_name.clone(),
                position: s.position,
                progress_m: f64::from(s.progress_cm) / 100.0,
                finish_s: s.finish_ms.map(|m| f64::from(m) / 1000.0),
                dnf: s.dnf,
                gap_ahead_s: s.gap_ahead_ms.map(|m| f64::from(m) / 1000.0),
            })
            .collect(),
    })
}

/// Voice chat (spec §7): the LiveKit participant identity of a rider.
/// Server and clients derive it from the rider id, so a client maps the
/// voices it hears to riders on the road (spatial audio).
pub fn voice_identity(rider: RiderId) -> String {
    format!("rider-{}", rider.0)
}

/// The rider behind a LiveKit participant identity ([`voice_identity`]);
/// `None` for other participants.
pub fn rider_of_voice_identity(identity: &str) -> Option<RiderId> {
    let id = identity.strip_prefix("rider-")?;
    // No sign, no leading zeros: exactly the form `voice_identity` writes.
    if id.is_empty() || id.starts_with('+') || (id.len() > 1 && id.starts_with('0')) {
        return None;
    }
    id.parse().ok().map(RiderId)
}

/// Longest voice group name ([`v1::VoiceTokenRequest::group`]).
pub const MAX_VOICE_GROUP_LEN: usize = 32;

/// Checks a voice group name: 1 to [`MAX_VOICE_GROUP_LEN`] characters of
/// `a-z`, `0-9` and `-` (it becomes part of the LiveKit room name).
pub fn valid_voice_group(name: &str) -> bool {
    (1..=MAX_VOICE_GROUP_LEN).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A voice token request (spec §7).
pub fn voice_request(channel: v1::VoiceChannel, group: &str) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Voice(v1::VoiceTokenRequest {
        channel: channel as i32,
        group: group.to_owned(),
    }))
}

/// The echo test's request (spec §7 *Echo bot*): the Echo rider on the
/// road (`here`: 20 m ahead of the asker) or away.
pub fn echo_request(on: bool, here: bool) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Echo(v1::EchoRequest { on, here }))
}

/// The workshop's bike when the rider leaves it (spec §14 Phase 4, 4A):
/// garage bike id, frame profile, look and the part ids on the bike.
pub fn bike_change(
    bike_id: &str,
    profile: ocw_core::ProfileId,
    look: ocw_core::RiderLook,
    parts: &[String],
) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::BikeChange(v1::BikeChange {
        bike_id: bike_id.into(),
        bike_profile: profile.id().into(),
        look: Some(look.into()),
        parts: parts.to_vec(),
    }))
}

/// The session's Echo rider in a snapshot, if there is one.
pub fn echo_rider(s: &v1::Snapshot) -> Option<RiderId> {
    s.riders
        .iter()
        .find(|u| u.echo)
        .map(|u| RiderId(u.rider_id))
}

/// Who may change the world clock (spec §14 Phase 9, 9D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockAccess {
    /// Every rider: a local server, where the single player is the host.
    Anyone,
    /// Only the host (the rider connected longest) in a shared session.
    #[default]
    Host,
    /// Nobody (set by the organizer on the server).
    Nobody,
}

impl ClockAccess {
    /// Whether `rider` may do it, with `host` the session's host.
    pub fn allows(self, rider: Option<RiderId>, host: Option<RiderId>) -> bool {
        match self {
            ClockAccess::Anyone => true,
            ClockAccess::Host => rider.is_some() && rider == host,
            ClockAccess::Nobody => false,
        }
    }
}

/// [`ClockAccess`] on the wire (`WorldClock.access`, `Welcome.world_access`).
pub fn access_to_wire(access: ClockAccess) -> i32 {
    (match access {
        ClockAccess::Anyone => v1::ClockAccess::Anyone,
        ClockAccess::Host => v1::ClockAccess::Host,
        ClockAccess::Nobody => v1::ClockAccess::Nobody,
    }) as i32
}

/// A received access value (unknown values: anyone, as an old server's 0).
pub fn access_from_wire(v: i32) -> ClockAccess {
    match v1::ClockAccess::try_from(v) {
        Ok(v1::ClockAccess::Host) => ClockAccess::Host,
        Ok(v1::ClockAccess::Nobody) => ClockAccess::Nobody,
        _ => ClockAccess::Anyone,
    }
}

/// A world switch request (spec §3.2.1): package and route (empty: free ride).
pub fn world_request(world: &str, route: Option<&str>) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::World(v1::WorldRequest {
        world: world.to_owned(),
        route: route.unwrap_or_default().to_owned(),
    }))
}

/// The world clock message for session time `t` (s).
pub fn world_clock(
    c: &ocw_sky::WorldClock,
    t: f64,
    access: ClockAccess,
    host: Option<RiderId>,
) -> v1::WorldClock {
    v1::WorldClock {
        world_unix_ms: c.now(t),
        rate: c.rate() as f32,
        mode: match c.mode() {
            ocw_sky::ClockMode::Real => v1::ClockMode::Real,
            ocw_sky::ClockMode::Fixed => v1::ClockMode::Fixed,
            ocw_sky::ClockMode::Offset => v1::ClockMode::Offset,
        } as i32,
        speed: c.speed() as f32,
        paused: c.paused(),
        offset_ms: c.offset_ms(t),
        access: access_to_wire(access),
        host_rider_id: host.map_or(0, |h| h.0),
    }
}

/// The clock as a client sees it (from a [`v1::WorldClock`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockInfo {
    pub world_ms: i64,
    pub rate: f64,
    pub mode: ocw_sky::ClockMode,
    pub speed: f64,
    pub paused: bool,
    pub offset_ms: i64,
    pub access: ClockAccess,
    pub host: Option<RiderId>,
}

impl ClockInfo {
    /// Whether rider `me` may change the clock.
    pub fn may_change(&self, me: Option<RiderId>) -> bool {
        self.access.allows(me, self.host)
    }
}

pub fn clock_info(c: &v1::WorldClock) -> ClockInfo {
    ClockInfo {
        world_ms: c.world_unix_ms,
        rate: f64::from(c.rate),
        mode: match v1::ClockMode::try_from(c.mode) {
            Ok(v1::ClockMode::Fixed) => ocw_sky::ClockMode::Fixed,
            Ok(v1::ClockMode::Offset) => ocw_sky::ClockMode::Offset,
            _ => ocw_sky::ClockMode::Real,
        },
        speed: f64::from(c.speed),
        paused: c.paused,
        offset_ms: c.offset_ms,
        access: access_from_wire(c.access),
        host: (c.host_rider_id != 0).then_some(RiderId(c.host_rider_id)),
    }
}

/// A time-machine request (spec §14 Phase 9, 9D).
pub fn clock_request(change: ocw_sky::ClockChange) -> v1::ClientMessage {
    use ocw_sky::ClockChange as K;
    use v1::clock_request::Change;
    let change = match change {
        K::Real => Change::Real(true),
        K::Fixed(ms) => Change::FixedUnixMs(ms),
        K::Offset(ms) => Change::OffsetMs(ms),
        K::Step(ms) => Change::StepMs(ms),
        K::Speed(s) => Change::Speed(s as f32),
        K::Paused(p) => Change::Paused(p),
    };
    client_message(v1::client_message::Msg::Clock(v1::ClockRequest {
        change: Some(change),
    }))
}

/// The change a [`v1::ClockRequest`] asks for.
pub fn clock_change(r: &v1::ClockRequest) -> Option<ocw_sky::ClockChange> {
    use ocw_sky::ClockChange as K;
    use v1::clock_request::Change;
    Some(match r.change.as_ref()? {
        Change::Real(_) => K::Real,
        Change::FixedUnixMs(ms) => K::Fixed(*ms),
        Change::OffsetMs(ms) => K::Offset(*ms),
        Change::StepMs(ms) => K::Step(*ms),
        Change::Speed(s) => K::Speed(f64::from(*s)),
        Change::Paused(p) => K::Paused(*p),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use v1::client_message::Msg;

    #[test]
    fn world_requests_and_access_round_trip() {
        let bytes = encode(&world_request(
            "europe/liechtenstein@3",
            Some("Vaduz - Malbun"),
        ));
        let Msg::World(w) = decode_client(&bytes).unwrap() else {
            panic!("not a world request")
        };
        assert_eq!(
            (w.world.as_str(), w.route.as_str()),
            ("europe/liechtenstein@3", "Vaduz - Malbun")
        );
        let Msg::World(w) = decode_client(&encode(&world_request("x", None))).unwrap() else {
            panic!()
        };
        assert!(w.route.is_empty(), "free ride");
        for a in [ClockAccess::Anyone, ClockAccess::Host, ClockAccess::Nobody] {
            assert_eq!(access_from_wire(access_to_wire(a)), a);
        }
        assert_eq!(access_from_wire(99), ClockAccess::Anyone);
        let (me, other) = (Some(RiderId(1)), Some(RiderId(2)));
        assert!(ClockAccess::Anyone.allows(other, me));
        assert!(ClockAccess::Host.allows(me, me) && !ClockAccess::Host.allows(other, me));
        assert!(!ClockAccess::Host.allows(None, None));
        assert!(!ClockAccess::Nobody.allows(me, me));
    }

    #[test]
    fn voice_identity_roundtrip() {
        for id in [0, 1, 42, u32::MAX] {
            let s = voice_identity(RiderId(id));
            assert_eq!(rider_of_voice_identity(&s), Some(RiderId(id)), "{s}");
        }
        for bad in [
            "",
            "rider-",
            "rider-x",
            "rider--1",
            "rider-+1",
            "rider-01",
            "bot-1",
            "rider-4294967296",
        ] {
            assert_eq!(rider_of_voice_identity(bad), None, "{bad}");
        }
    }

    #[test]
    fn voice_group_names() {
        assert!(valid_voice_group("team-7"));
        assert!(valid_voice_group(&"a".repeat(MAX_VOICE_GROUP_LEN)));
        for bad in ["", "Team", "a b", "ä", "a_b", "../x"] {
            assert!(!valid_voice_group(bad), "{bad}");
        }
        assert!(!valid_voice_group(&"a".repeat(MAX_VOICE_GROUP_LEN + 1)));
    }

    #[test]
    fn voice_request_roundtrip() {
        let bytes = encode(&voice_request(v1::VoiceChannel::Group, "team-7"));
        match decode_client(&bytes).unwrap() {
            Msg::Voice(r) => {
                assert_eq!(r.channel(), v1::VoiceChannel::Group);
                assert_eq!(r.group, "team-7");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn event_messages_roundtrip() {
        for spec in [
            EventSpec::GroupRide {
                target_w: Some(180.0),
            },
            EventSpec::Race {
                laps: Some(3),
                length_m: None,
            },
            EventSpec::TimeTrial {
                laps: None,
                length_m: Some(4200.0),
            },
            EventSpec::Race {
                laps: None,
                length_m: None,
            },
        ] {
            let start = event_start(&spec, Some(4));
            let bytes = encode(&client_message(Msg::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(start)),
            })));
            let Msg::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(back)),
            }) = decode_client(&bytes).unwrap()
            else {
                panic!()
            };
            assert_eq!(back.bots, Some(4));
            assert_eq!(event_spec(&back), Some(spec));
        }
        assert_eq!(event_spec(&v1::EventStart::default()), None);
        // The state, with standings.
        let mut ev = Event::new(
            7,
            EventKind::Race {
                length_m: 8000.0,
                laps: Some(2),
            },
            100,
            4000.0,
            vec![(RiderId(1), "Ana".into()), (RiderId(2), "Bot 1".into())],
        );
        ev.update(100, &[]);
        let standings = vec![
            Standing {
                id: RiderId(2),
                name: "Bot 1".into(),
                position: 1,
                progress_m: 8000.0,
                finish_s: Some(612.345),
                dnf: false,
                gap_ahead_s: None,
            },
            Standing {
                id: RiderId(1),
                name: "Ana".into(),
                position: 2,
                progress_m: 7990.5,
                finish_s: None,
                dnf: false,
                gap_ahead_s: Some(1.25),
            },
        ];
        let wire = event_state(Some(&ev), &standings, 123);
        let info = event_info(&wire).unwrap();
        assert_eq!(info.id, 7);
        assert_eq!(info.kind, ev.kind);
        assert_eq!(info.phase, EventPhase::Countdown);
        assert_eq!((info.go_tick, info.server_tick), (ev.go_tick, 123));
        assert_eq!(info.start_line_m, 4000.0);
        assert_eq!(info.standings, standings);
        assert_eq!(info.race_time_s(ev.go_tick as f64 + 25.0), 0.5);
        // No event (cancelled).
        assert_eq!(event_info(&event_state(None, &[], 5)), None);
        let group = Event::new(1, EventKind::GroupRide { target_w: 150.0 }, 0, 0.0, vec![]);
        let info = event_info(&event_state(Some(&group), &[], 0)).unwrap();
        assert_eq!(info.kind, EventKind::GroupRide { target_w: 150.0 });
    }

    #[test]
    fn roundtrip_client_message() {
        let sample = v1::SensorSample {
            seq: 7,
            client_time_us: 123,
            power_w: Some(250.0),
            cadence_rpm: Some(90.0),
            heart_rate_bpm: None,
            brake: Some(true),
            virtual_power: None,
        };
        let bytes = encode(&client_message(Msg::Sample(sample)));
        match decode_client(&bytes).unwrap() {
            Msg::Sample(s) => assert_eq!(s, sample),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The world clock through the wire and back (spec §14 Phase 9).
    #[test]
    fn world_clock_round_trip() {
        use v1::server_message::Msg as S;
        let mut c = ocw_sky::WorldClock::new(1_781_000_000_000);
        c.set_speed(10.0, 600.0).unwrap();
        let msg = world_clock(&c, 20.0, ClockAccess::Host, Some(RiderId(3)));
        let snap = v1::Snapshot {
            server_tick: 1000,
            riders: Vec::new(),
            human_riders: 1,
            clock: Some(msg),
            ..Default::default()
        };
        let bytes = encode(&server_message(S::Snapshot(snap)));
        let S::Snapshot(s) = decode_server(&bytes).unwrap() else {
            panic!("a snapshot")
        };
        let info = clock_info(&s.clock.unwrap());
        assert_eq!(info.world_ms, c.now(20.0));
        assert_eq!(info.world_ms, 1_781_000_000_000 + 10_000 + 6_000_000);
        assert_eq!((info.rate, info.speed, info.paused), (600.0, 600.0, false));
        assert_eq!(info.mode, ocw_sky::ClockMode::Fixed);
        assert_eq!(info.offset_ms, 6_000_000 - 10_000);
        assert!(info.may_change(Some(RiderId(3))));
        assert!(!info.may_change(Some(RiderId(4))));
        assert!(!info.may_change(None));
        let anyone = ClockInfo {
            access: ClockAccess::Anyone,
            ..info
        };
        assert!(anyone.may_change(Some(RiderId(4))));
        let nobody = ClockInfo {
            access: ClockAccess::Nobody,
            ..info
        };
        assert!(!nobody.may_change(Some(RiderId(3))));
        // Requests.
        use ocw_sky::ClockChange as K;
        for change in [
            K::Real,
            K::Fixed(-5),
            K::Offset(7),
            K::Step(-9),
            K::Speed(60.0),
            K::Paused(true),
        ] {
            let bytes = encode(&clock_request(change));
            let Msg::Clock(r) = decode_client(&bytes).unwrap() else {
                panic!("a clock request")
            };
            assert_eq!(clock_change(&r), Some(change));
        }
        assert_eq!(clock_change(&v1::ClockRequest { change: None }), None);
    }

    #[test]
    fn roundtrip_server_snapshot() {
        use v1::server_message::Msg as S;
        let snapshot = v1::Snapshot {
            server_tick: 4242,
            riders: vec![
                quantize(&ocw_sim::RiderSnapshot {
                    id: RiderId(1),
                    name: "Ana".into(),
                    state: RiderState {
                        distance: 3949.99,
                        speed: 12.5,
                        lateral_offset: -1.2,
                        heading: 0.123,
                        turn_curvature: -0.0345,
                    },
                    power_w: 310.0,
                    last_input_seq: 77,
                    god: None,
                    crash: None,
                    bump: None,
                    bike: ocw_core::ProfileId::Gravel,
                    look: ocw_core::RiderLook {
                        frame: ocw_core::Rgb(0x123456),
                        pattern: ocw_core::KitPattern::Sash,
                        ..Default::default()
                    },
                    build_hash: 0,
                }),
                quantize(&ocw_sim::RiderSnapshot {
                    id: RiderId(2),
                    name: "Bot".into(),
                    state: RiderState {
                        distance: 0.0,
                        speed: 0.0,
                        lateral_offset: 1.2,
                        ..Default::default()
                    },
                    power_w: 0.0,
                    last_input_seq: 0,
                    god: None,
                    crash: None,
                    bump: None,
                    bike: Default::default(),
                    look: Default::default(),
                    build_hash: 0,
                }),
            ],
            human_riders: 2,
            clock: Some(v1::WorldClock {
                world_unix_ms: 1_781_000_000_000,
                rate: 600.0,
                mode: v1::ClockMode::Fixed as i32,
                speed: 600.0,
                paused: false,
                offset_ms: -3_600_000,
                access: v1::ClockAccess::Host as i32,
                host_rider_id: 1,
            }),
            collisions: v1::CollisionMode::Soft as i32,
            chaotic_bots: true,
            weather: None,
        };
        let bytes = encode(&server_message(S::Snapshot(snapshot.clone())));
        match decode_server(&bytes).unwrap() {
            S::Snapshot(s) => {
                assert_eq!(s, snapshot);
                let (id, st) = dequantize(&s.riders[0]);
                assert_eq!(id, RiderId(1));
                assert!((st.distance - 3949.99).abs() < 1e-9);
                assert!((st.lateral_offset + 1.2).abs() < 1e-9);
                assert_eq!(s.riders[0].last_input_seq, 77);
                let (bike, look) = rider_look(&s.riders[0]);
                assert_eq!(bike, ocw_core::ProfileId::Gravel);
                let look = look.unwrap();
                assert_eq!(look.frame, ocw_core::Rgb(0x123456));
                assert_eq!(look.pattern, ocw_core::KitPattern::Sash);
                assert_eq!(rider_look(&s.riders[1]).0, ocw_core::ProfileId::Road);
            }
            other => panic!("unexpected {other:?}"),
        }
        let welcome = v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: 9,
            tick_rate_hz: 50,
            broadcast_rate_hz: 10,
            route_id: "test-loop".into(),
            free_ride: false,
            world: Some(v1::WorldPackageId {
                name: "Liechtenstein".into(),
                graph_hash: 0x0123_4567_89ab_cdef,
            }),
            graph_route: "Rheintal".into(),
            voice: true,
            clock: None,
            world_access: 2,
            god_access: 1,
            echo_bot: true,
            ..Default::default()
        };
        let bytes = encode(&server_message(S::Welcome(welcome.clone())));
        assert_eq!(decode_server(&bytes).unwrap(), S::Welcome(welcome));
    }

    #[test]
    fn quantization_clamps_out_of_range_values() {
        let u = quantize(&ocw_sim::RiderSnapshot {
            id: RiderId(1),
            name: String::new(),
            state: RiderState {
                distance: -3.0,
                speed: -1.0,
                lateral_offset: 0.004,
                ..Default::default()
            },
            power_w: -5.0,
            last_input_seq: 0,
            god: None,
            crash: None,
            bump: None,
            bike: Default::default(),
            look: Default::default(),
            build_hash: 0,
        });
        assert_eq!((u.distance_cm, u.speed_mm_s, u.power_w), (0, 0, 0));
        assert_eq!(u.lateral_offset_cm, 0);
    }

    /// God mode (spec §14 Phase 13E): the mode and the unicorn's pose
    /// travel with the rider update; riders without it are riding.
    #[test]
    fn god_mode_roundtrips_in_rider_updates_and_requests() {
        let pose = ocw_sim::god::GodPose {
            pos: [1234.5, 812.25, -77.0],
            yaw: 1.25,
            pitch: -0.5,
            speed: 220.0,
        };
        let snap = |god| ocw_sim::RiderSnapshot {
            id: RiderId(3),
            name: "Uni".into(),
            state: RiderState::default(),
            power_w: 180.0,
            last_input_seq: 0,
            god,
            crash: None,
            bump: None,
            bike: Default::default(),
            look: Default::default(),
            build_hash: 0,
        };
        let u = quantize(&snap(Some(pose)));
        let bytes = encode(&u);
        let back = <v1::RiderUpdate as prost::Message>::decode(bytes.as_slice()).unwrap();
        assert_eq!(rider_god(&back), Some(pose));
        assert_eq!(rider_god(&quantize(&snap(None))), None);
        // An old server's update (no mode): riding.
        assert_eq!(rider_god(&v1::RiderUpdate::default()), None);
        let Msg::God(r) = decode_client(&encode(&god_leave(true, 1.0, 2.0))).unwrap() else {
            panic!("a god request")
        };
        assert_eq!(
            r.req,
            Some(v1::god_request::Req::Leave(v1::GodLeave {
                fly_back: true,
                x_m: 1.0,
                z_m: 2.0
            }))
        );
        assert!(matches!(
            decode_client(&encode(&god_enter())).unwrap(),
            Msg::God(v1::GodRequest {
                req: Some(v1::god_request::Req::Enter(true))
            })
        ));
        let w = god_pose_to_wire(&pose, 7, true);
        assert_eq!((w.seq, w.teleport), (7, true));
        assert_eq!(god_pose_from_wire(&w), pose);
    }

    #[test]
    fn empty_message_is_rejected() {
        let bytes = encode(&v1::ClientMessage { msg: None });
        assert!(matches!(decode_client(&bytes), Err(ProtocolError::Empty)));
        assert!(decode_client(&[0xff, 0xff]).is_err());
        let bytes = encode(&v1::ServerMessage { msg: None });
        assert!(matches!(decode_server(&bytes), Err(ProtocolError::Empty)));
    }

    #[test]
    fn free_ride_paths_and_turns_round_trip() {
        use ocw_world::graph::{Edge, Oneway, RoadClass, RoadGraph};
        let edge = |id: u64, a: u32, b: u32, p: [[f64; 2]; 2]| Edge {
            id: EdgeId(id),
            from: VertexId(a),
            to: VertexId(b),
            points: p.to_vec(),
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 1,
            profile: Vec::new(),
        };
        let net = RoadNetwork::new(RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 80.0]],
            edges: vec![
                edge(u64::MAX - 7, 0, 1, [[0.0, 0.0], [100.0, 0.0]]),
                edge(42, 1, 2, [[100.0, 0.0], [100.0, 80.0]]),
            ],
            boundary_vertices: Default::default(),
        });
        let change = ocw_sim::PathChange {
            rider: RiderId(4),
            reset: true,
            first_index: 17,
            start_m: 1234.5,
            segments: vec![
                Segment::Edge {
                    edge: 0,
                    from_m: 12.25,
                    to_m: 100.0,
                },
                Segment::Edge {
                    edge: 1,
                    from_m: 0.0,
                    to_m: 70.0,
                },
                Segment::Turnaround(TurnReason::Manual),
                Segment::Edge {
                    edge: 1,
                    from_m: 70.0,
                    to_m: 0.0,
                },
            ],
        };
        let u = path_update(&change, &net);
        assert_eq!(u.steps[0].edge_id, u64::MAX - 7);
        let bytes = encode(&server_message(v1::server_message::Msg::Path(u.clone())));
        let v1::server_message::Msg::Path(back) = decode_server(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(back, u);
        assert_eq!(path_segments(&back, &net).unwrap(), change.segments);
        // Unknown edges are refused.
        let mut bad = back.clone();
        bad.steps[1].edge_id = 999;
        assert!(path_segments(&bad, &net).unwrap_err().contains("999"));
        // Turn intents.
        let c = TurnChoice {
            junction: VertexId(1),
            edge: 1,
            forward: true,
        };
        let t = turn_intent(3, VertexId(1), Some(&c), &net);
        assert_eq!(t.exit_edge_id, Some(42));
        assert_eq!(turn_choice(&t, &net), Some(c));
        let none = turn_intent(4, VertexId(1), None, &net);
        assert_eq!(none.exit_edge_id, None);
        assert_eq!(turn_choice(&none, &net), None);
    }

    #[test]
    fn quantization_keeps_centimetres() {
        let snap = ocw_sim::RiderSnapshot {
            id: RiderId(3),
            name: "x".into(),
            state: RiderState {
                distance: 1234.567,
                speed: 9.8765,
                lateral_offset: -1.234,
                heading: -0.2004,
                turn_curvature: 0.01234,
            },
            power_w: 201.6,
            last_input_seq: 9,
            god: None,
            crash: None,
            bump: None,
            bike: Default::default(),
            look: Default::default(),
            build_hash: 0,
        };
        let u = quantize(&snap);
        assert_eq!(u.power_w, 202);
        let (id, s) = dequantize(&u);
        assert_eq!(id, RiderId(3));
        assert!((s.distance - 1234.57).abs() < 1e-9);
        assert!((s.speed - 9.877).abs() < 1e-9);
        assert!((s.lateral_offset + 1.23).abs() < 1e-9);
        assert!((s.heading + 0.2).abs() < 1e-9);
        assert!((s.turn_curvature - 0.0123).abs() < 1e-9);
    }

    /// The character (spec §14 Phase 4, 4C, W4) round-trips in the full
    /// look (Hello, BikeChange, RiderBuild), is left out of the snapshots'
    /// looks, and is validated on arrival.
    #[test]
    fn the_character_round_trips_once_per_change_not_in_snapshots() {
        let look = ocw_core::RiderLook {
            jersey: ocw_core::Rgb(0x43A047),
            character: ocw_core::Character {
                height_cm: 191,
                build: ocw_core::BodyBuild::Strong,
                skin: 9,
                face: 1,
                hair: 3,
                hair_colour: ocw_core::Rgb(0xB3482A),
                glasses: 2,
            },
            ..Default::default()
        };
        let wire: v1::RiderLook = look.into();
        assert_eq!(ocw_core::RiderLook::from(&wire), look);
        let snap = snapshot_look(&look);
        assert_eq!(snap.height_cm, None);
        assert_eq!(snap.skin, None);
        let back = ocw_core::RiderLook::from(&snap);
        assert_eq!(back.jersey, look.jersey);
        assert_eq!(back.character, ocw_core::Character::default());
        // A RiderBuild carries it, with or without parts.
        let msg = rider_build(RiderId(3), ocw_core::ProfileId::Road, None, Some(&look));
        let bytes = encode(&server_message(v1::server_message::Msg::RiderBuild(msg)));
        let v1::server_message::Msg::RiderBuild(b) = decode_server(&bytes).unwrap() else {
            panic!("a rider build")
        };
        assert_eq!(build_info(&b).look, Some(look));
        // Out of range from the wire: the default's values, height clamped.
        let odd = v1::RiderLook {
            height_cm: Some(900),
            skin: Some(40),
            hair: Some(99),
            body_build: Some(7),
            ..Default::default()
        };
        let c = character(&odd);
        let d = ocw_core::Character::default();
        assert_eq!(c.height_cm, ocw_core::character::HEIGHT_RANGE_CM.1);
        assert_eq!((c.skin, c.hair, c.build), (d.skin, d.hair, d.build));
    }

    /// Looks from the network are validated: colours to 24 bits, unknown
    /// patterns plain, unknown bikes the default profile.
    #[test]
    fn looks_from_the_wire_are_validated() {
        let wire = v1::RiderLook {
            frame_rgb: 0xFF00_00FF,
            jersey_rgb: 0x00AB_CDEF,
            helmet_rgb: 0,
            kit_pattern: 77,
            ..Default::default()
        };
        let l = ocw_core::RiderLook::from(&wire);
        assert_eq!(l.frame, ocw_core::Rgb(0x0000FF));
        assert_eq!(l.jersey, ocw_core::Rgb(0xABCDEF));
        assert_eq!(l.pattern, ocw_core::KitPattern::Plain);
        let u = v1::RiderUpdate {
            bike_profile: "hovercraft".into(),
            ..Default::default()
        };
        assert_eq!(rider_look(&u), (ocw_core::ProfileId::Road, None));
        let back = v1::RiderLook::from(ocw_core::RiderLook::default());
        assert_eq!(
            ocw_core::RiderLook::from(&back),
            ocw_core::RiderLook::default()
        );
    }

    /// The workshop's `BikeChange` and the hearts total survive the wire
    /// (spec §14 Phase 4, 4A, 4E).
    #[test]
    fn bike_change_and_hearts_round_trip() {
        let look = ocw_core::RiderLook {
            frame: ocw_core::Rgb(0x1E88E5),
            ..Default::default()
        };
        let parts = vec!["wheels.road-carbon.top".to_string(), "lights.set".into()];
        let m = bike_change("tt", ocw_core::ProfileId::TimeTrial, look, &parts);
        let back = decode_client(&encode(&m)).unwrap();
        let v1::client_message::Msg::BikeChange(c) = back else {
            panic!("{back:?}")
        };
        assert_eq!(c.bike_id, "tt");
        assert_eq!(
            ocw_core::ProfileId::from_wire(Some(&c.bike_profile)),
            ocw_core::ProfileId::TimeTrial
        );
        assert_eq!(ocw_core::RiderLook::from(&c.look.unwrap()), look);
        assert_eq!(c.parts, parts);
        let u = v1::RiderUpdate {
            hearts: 1239,
            ..Default::default()
        };
        let snap = v1::Snapshot {
            riders: vec![u],
            ..Default::default()
        };
        let bytes = encode(&server_message(v1::server_message::Msg::Snapshot(snap)));
        let Ok(v1::server_message::Msg::Snapshot(s)) = decode_server(&bytes) else {
            panic!()
        };
        assert_eq!(s.riders[0].hearts, 1239);
        // Old clients' samples have no virtual-power flag: real power.
        let sample = v1::SensorSample::default();
        assert_eq!(sample.virtual_power, None);
    }

    /// The echo test's messages survive the wire (spec §7 *Echo bot*).
    #[test]
    fn echo_bot_round_trip() {
        let m = echo_request(true, true);
        let back = decode_client(&encode(&m)).unwrap();
        assert_eq!(
            back,
            v1::client_message::Msg::Echo(v1::EchoRequest {
                on: true,
                here: true
            })
        );
        let snap = v1::Snapshot {
            riders: vec![
                v1::RiderUpdate {
                    rider_id: 3,
                    ..Default::default()
                },
                v1::RiderUpdate {
                    rider_id: 7,
                    display_name: "Echo".into(),
                    echo: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let msg = server_message(v1::server_message::Msg::Snapshot(snap));
        let Ok(v1::server_message::Msg::Snapshot(back)) = decode_server(&encode(&msg)) else {
            panic!("no snapshot")
        };
        assert_eq!(echo_rider(&back), Some(RiderId(7)));
        assert!(!back.riders[0].echo, "others are not");
        let w = v1::Welcome {
            echo_bot: true,
            ..Default::default()
        };
        let msg = server_message(v1::server_message::Msg::Welcome(w));
        let Ok(v1::server_message::Msg::Welcome(back)) = decode_server(&encode(&msg)) else {
            panic!("no welcome")
        };
        assert!(back.echo_bot);
        // Older peers: absent means no Echo rider, no echo bot.
        assert_eq!(echo_rider(&v1::Snapshot::default()), None);
    }

    #[test]
    fn crashes_bumps_and_collision_modes_round_trip() {
        use ocw_sim::collide::{Bump, ContactKind, CrashPhase, CrashView, Mode};
        let crash = CrashView {
            phase: CrashPhase::Down,
            elapsed_s: 1.234,
            down_s: 4.5,
            kind: ContactKind::WheelTouch,
            by: Some(RiderId(7)),
            impulse: [-1.25, 0.75],
            speed: 9.876,
        };
        let snap = ocw_sim::RiderSnapshot {
            id: RiderId(3),
            name: "R".into(),
            bike: ocw_core::ProfileId::default(),
            look: ocw_core::RiderLook::default(),
            state: RiderState::default(),
            power_w: 0.0,
            last_input_seq: 0,
            god: None,
            crash: Some(crash),
            bump: Some(Bump {
                tick: 99,
                strength: 0.08,
                side: -1.0,
            }),
            build_hash: 0,
        };
        let bytes = encode(&quantize(&snap));
        let u = v1::RiderUpdate::decode(bytes.as_slice()).unwrap();
        assert_eq!(rider_crash(&u), Some(crash));
        let b = rider_bump(&u).unwrap();
        assert_eq!((b.tick, b.side), (99, -1.0));
        assert!((b.strength - 0.08).abs() < 1e-9);
        // Riding: no crash; an old server's update has none either.
        let riding = quantize(&ocw_sim::RiderSnapshot {
            crash: None,
            bump: None,
            ..snap
        });
        assert_eq!(rider_crash(&riding), None);
        assert_eq!(rider_bump(&riding), None);
        for m in [Mode::Off, Mode::Soft, Mode::Full] {
            assert_eq!(collision_mode_from_wire(collision_mode_to_wire(m)), m);
        }
        assert_eq!(collision_mode_from_wire(0), Mode::Off, "older servers");
        // The rules request and the staged contact decode.
        let m = decode_client(&encode(&rules_request(Some(true), None))).unwrap();
        assert!(
            matches!(m, v1::client_message::Msg::Rules(r) if r.collisions == Some(true) && r.chaotic_bots.is_none())
        );
        let m = decode_client(&encode(&stage_contact(true))).unwrap();
        assert!(
            matches!(m, v1::client_message::Msg::Dev(v1::DevCommand { cmd: Some(v1::dev_command::Cmd::Stage(s)) }) if s.crash)
        );
    }

    /// W3 (spec §14 Phase 4, 4G): the race mode in the event request and
    /// state, the build of a rider, its hash in the snapshots and the
    /// parts in the hello survive the wire.
    #[test]
    fn race_modes_and_builds_round_trip() {
        use ocw_sim::PhysicsMode as M;
        for m in M::ALL {
            assert_eq!(physics_mode_from_wire(physics_mode_to_wire(m)), Some(m));
            let spec = EventSpec::Race {
                laps: None,
                length_m: Some(2000.0),
            };
            let start = event_start_with(&spec, Some(2), Some(m));
            let back = v1::EventStart::decode(encode(&start).as_slice()).unwrap();
            assert_eq!(event_physics(&back), Some(m));
            assert_eq!(event_spec(&back), Some(spec));
            let mut ev = Event::new(
                1,
                EventKind::Race {
                    length_m: 2000.0,
                    laps: None,
                },
                0,
                0.0,
                vec![],
            );
            ev.physics = m;
            let state =
                v1::EventState::decode(encode(&event_state(Some(&ev), &[], 0)).as_slice()).unwrap();
            assert_eq!(event_info(&state).unwrap().physics, m);
        }
        // Absent: by the kind on the server; an older server's state: open.
        assert_eq!(
            event_physics(&event_start(&EventSpec::GroupRide { target_w: None }, None)),
            None
        );
        let old = v1::EventState {
            phase: v1::EventPhase::Running as i32,
            kind: v1::EventKind::Race as i32,
            ..Default::default()
        };
        assert_eq!(event_info(&old).unwrap().physics, M::Open);
        assert_eq!(physics_mode_from_wire(99), None);

        let parts: Vec<String> = ["wheels.road-carbon.top", "tyres.road.race"]
            .map(String::from)
            .to_vec();
        let b = ocw_sim::build::build(
            ocw_sim::PartTable::shipped(),
            ocw_core::ProfileId::Road,
            &parts,
        );
        let wire = rider_build(RiderId(4), ocw_core::ProfileId::Road, Some(&b), None);
        let msg = server_message(v1::server_message::Msg::RiderBuild(wire));
        let v1::server_message::Msg::RiderBuild(back) = decode_server(&encode(&msg)).unwrap()
        else {
            panic!("a rider build");
        };
        let info = build_info(&back);
        assert_eq!(info.rider, RiderId(4));
        assert_eq!(info.hash, b.hash());
        assert_eq!(info.profile, ocw_core::ProfileId::Road);
        assert_eq!(info.parts, b.parts);
        assert!((info.effect.flat - b.flat_gain).abs() < 1e-4);
        assert!((info.effect.climb - b.climb_gain).abs() < 1e-4);
        assert!((info.effect.mass_kg - b.params.bike_mass_kg).abs() < 1e-3);
        assert_eq!(info.effect.capped, b.capped());
        // The stock bike: hash 0, no parts.
        let stock = build_info(&rider_build(
            RiderId(5),
            ocw_core::ProfileId::Gravel,
            None,
            None,
        ));
        assert_eq!((stock.hash, stock.parts.len()), (0, 0));
        assert_eq!(stock.profile, ocw_core::ProfileId::Gravel);
        // The hash rides in every rider update (6 bytes: a 2-byte tag and
        // 4), the parts not.
        let snap = ocw_sim::RiderSnapshot {
            id: RiderId(4),
            name: "A".into(),
            bike: ocw_core::ProfileId::Road,
            look: Default::default(),
            state: RiderState::default(),
            power_w: 0.0,
            last_input_seq: 0,
            god: None,
            crash: None,
            bump: None,
            build_hash: b.hash(),
        };
        let u = quantize(&snap);
        assert_eq!(u.build_hash, b.hash());
        let plain = quantize(&ocw_sim::RiderSnapshot {
            build_hash: 0,
            ..snap
        });
        assert_eq!(encode(&u).len(), encode(&plain).len() + 6);
        // Hello carries the start build.
        let hello = v1::Hello {
            parts: parts.clone(),
            ..Default::default()
        };
        assert_eq!(
            v1::Hello::decode(encode(&hello).as_slice()).unwrap().parts,
            parts
        );
        // A bot's bike for the dev tools.
        let dev = v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::BotBike(v1::BotBike {
                rider_id: 2,
                bike_profile: "road".into(),
                parts: parts.clone(),
                power_w: Some(300.0),
                lane_m: Some(-2.4),
                name: "Stock".into(),
            })),
        };
        assert_eq!(
            v1::DevCommand::decode(encode(&dev).as_slice()).unwrap(),
            dev
        );
    }
}
