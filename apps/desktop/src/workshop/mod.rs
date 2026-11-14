//! The workshop (spec §14 Phase 4, 4A–4F; milestones W1 *Scene* and W2
//! *Parts*): choosing *Workshop* in the menu opens a 3D bicycle workshop
//! over the running ride (autodrive meanwhile, the ride window shows it).
//! The selected bike stands on a repair stand, the rider's other bikes
//! hang on the wall; parts are chosen at their hotspots on the stand bike
//! and bought with hearts (♥) earned by riding.
//!
//! [`model`] is the Bevy-free scene (zones, bikes, swap, hotspots, picker),
//! [`camera`] its camera states, [`nav`] screen-space navigation, [`input`]
//! the four devices, [`picker`] the part picker, [`wallet`] the local
//! wallet and garage. This module is the Bevy glue: devices → actions →
//! model → [`WorkshopView`] (drawn by `ocw_render::workshop`); every
//! change is a console command line (`workshop.buy`, `bikes.select` …),
//! and leaving the workshop applies the bike (`BikeChange`, 4L Q1).

pub mod camera;
pub mod input;
pub mod model;
pub mod nav;
pub mod picker;
pub mod wallet;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use clap::{Arg, ArgAction, Command};
use ocw_assets::parts::{Catalogue, Slot};
use ocw_core::hearts::format as hearts;
use ocw_core::{ProfileId, Rgb, RiderLook};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::InputContext;
use ocw_render::workshop::{
    BikeModels, BikeView, DialogView, HotspotView, PickerRowView, PickerView, WorkshopCorePlugin,
    WorkshopPointer, WorkshopPointerEvent, WorkshopView, bike_corners, bike_pose,
};

use crate::Args;
use crate::connection::{ClientState, Link};
use crate::settings::RiderProfiles;
use input::{Act, ActiveInput, Gestures, HintCtx, MouseState, Pad, StickLatch, Touch};
use model::{Ctx, Out, Workshop};
use wallet::{Garage, Wallet};

pub struct WorkshopPlugin;

/// The workshop's state in the app.
#[derive(Resource)]
pub struct WorkshopState {
    pub wallet: Wallet,
    pub scene: Option<Workshop>,
    /// Actions from the console command `workshop`, applied next frame.
    queued: Vec<Act>,
    mouse: MouseState,
    latch: StickLatch,
    gestures: Gestures,
    pub active: ActiveInput,
    last_touch_s: f64,
    /// Hearts of this session already booked, and the session they belong to.
    credited: u32,
    session: f64,
    last_distance: Option<f64>,
    pub ride_window: bool,
    /// Booked hearts are saved at most this often (s).
    dirty: bool,
    save_in: f64,
}

impl WorkshopState {
    fn new(wallet: Wallet) -> Self {
        Self {
            wallet,
            scene: None,
            queued: Vec::new(),
            mouse: MouseState::default(),
            latch: StickLatch::default(),
            gestures: Gestures::default(),
            active: ActiveInput::Keyboard,
            last_touch_s: f64::NEG_INFINITY,
            credited: 0,
            session: f64::NAN,
            last_distance: None,
            ride_window: true,
            dirty: false,
            save_in: 0.0,
        }
    }

    fn save(&mut self) -> Result<(), String> {
        self.dirty = false;
        self.wallet.save()
    }
}

fn cat() -> &'static Catalogue {
    ocw_assets::shipped::catalogue()
}

/// The riding rider's name (the wallet's key) and bike profile.
fn rider(world: &World) -> (String, ProfileId) {
    let r = crate::settings::local_rider(world);
    (r.name.clone(), r.bike())
}

/// The part ids on the bike `rider` rides (the garage's active bike, if
/// it is of the riding `profile`), for the hello (spec §14 Phase 4, 4G).
pub fn riding_parts(world: &World, rider: &str, profile: ProfileId) -> Vec<String> {
    world
        .get_resource::<WorkshopState>()
        .and_then(|s| s.wallet.get(rider))
        .filter(|g| g.active_bike().profile() == profile)
        .map(|g| g.part_ids(g.active_bike()))
        .unwrap_or_default()
}

impl Plugin for WorkshopPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<WorkshopCorePlugin>() {
            app.add_plugins(WorkshopCorePlugin);
        }
        let (wallet, warnings) = match wallet::default_dir() {
            Some(dir) if !cfg!(test) => Wallet::load(&dir),
            _ => (Wallet::in_memory(), Vec::new()),
        };
        for w in warnings {
            warn!("workshop: {w}");
        }
        let mut state = WorkshopState::new(wallet);
        let (name, bike) = rider(app.world());
        state.wallet.garage(&name, bike);
        app.insert_resource(state)
            .add_systems(Update, (earn_hearts, workshop_input, write_view).chain());
        register_commands(app);
        register_apply(app);
    }
}

/// Console lines of the scene's actions (scripts, scenes, tests).
const ACTIONS: [&str; 14] = [
    "open",
    "close",
    "left",
    "right",
    "up",
    "down",
    "select",
    "back",
    "slot-next",
    "slot-prev",
    "compare",
    "action",
    "reset",
    "orbit",
];

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop")
            .about("Open, close or drive the workshop scene (menu: Workshop)")
            .arg(
                Arg::new("action")
                    .value_parser(ACTIONS)
                    .default_value("open"),
            )
            .arg(
                Arg::new("amount")
                    .help("orbit: degrees to turn")
                    .allow_negative_numbers(true)
                    .value_parser(clap::value_parser!(f32)),
            ),
        |world, m| {
            let action = m.get_one::<String>("action").map_or("open", String::as_str);
            match action {
                "open" => open(world),
                "close" => {
                    let mut s = world.resource_mut::<WorkshopState>();
                    if s.scene.is_none() {
                        return Err("the workshop is not open".into());
                    }
                    s.queued.push(Act::BackButton);
                    // Back until it leaves (at most picker, stand, overview).
                    s.queued
                        .extend([Act::Back, Act::Back, Act::Back, Act::Back]);
                    Ok(String::new())
                }
                a => {
                    let act = match a {
                        "left" => Act::Left,
                        "right" => Act::Right,
                        "up" => Act::Up,
                        "down" => Act::Down,
                        "select" => Act::Confirm,
                        "back" => Act::Back,
                        "slot-next" => Act::SlotNext,
                        "slot-prev" => Act::SlotPrev,
                        "compare" => Act::Compare,
                        "action" => Act::Action,
                        "reset" => Act::ResetView,
                        _ => {
                            let deg = m.get_one::<f32>("amount").copied().unwrap_or(30.0);
                            Act::Orbit(Vec2::new(deg.to_radians(), 0.0), 0.0)
                        }
                    };
                    let mut s = world.resource_mut::<WorkshopState>();
                    if s.scene.is_none() {
                        return Err("the workshop is not open (workshop open)".into());
                    }
                    s.queued.push(act);
                    Ok(String::new())
                }
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.ride_window")
            .about("The small live ride window in the workshop")
            .arg(Arg::new("on").value_parser(["on", "off"]).required(true)),
        |world, m| {
            let on = m.get_one::<String>("on").is_some_and(|v| v == "on");
            world.resource_mut::<WorkshopState>().ride_window = on;
            Ok(format!("ride window {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("hearts").about("Your hearts balance and the last ledger entries"),
        |world, _| {
            let (name, _) = rider(world);
            let s = world.resource::<WorkshopState>();
            let mut out = format!(
                "{name}: {} (1 per 10 kJ, 1 per 20 m climbed)",
                hearts(s.wallet.balance(&name))
            );
            for e in s.wallet.history(&name, 5) {
                out.push_str(&format!(
                    "\n  {}  {:>+6}  {} {}",
                    e.t, e.delta, e.reason, e.reference
                ));
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("hearts.grant")
            .about("Grant hearts (tests and screenshots; local server only)")
            .arg(
                Arg::new("n")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(clap::value_parser!(i64)),
            ),
        |world, m| {
            let n = *m.get_one::<i64>("n").expect("required");
            let (name, bike) = rider(world);
            let mut s = world.resource_mut::<WorkshopState>();
            s.wallet.garage(&name, bike);
            s.wallet.grant(&name, n);
            s.save()?;
            Ok(format!("balance {}", hearts(s.wallet.balance(&name))))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.parts")
            .about("The catalogue's parts that fit the bike (all slots, or one)")
            .arg(Arg::new("slot").value_parser(Slot::ALL.map(|s| s.id())))
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            let (name, _) = rider(world);
            let s = world.resource::<WorkshopState>();
            let g = s.wallet.get(&name).ok_or("no garage")?;
            let bike = bike_arg(g, m)?;
            let slots: Vec<Slot> = match m.get_one::<String>("slot") {
                Some(id) => Slot::parse(id).into_iter().collect(),
                None => Slot::ALL
                    .into_iter()
                    .filter(|s| *s != Slot::Paint)
                    .collect(),
            };
            let mut out = format!("{} – {}", bike.name(), hearts(s.wallet.balance(&name)));
            for slot in slots {
                for r in picker::rows(cat(), g, bike, slot) {
                    let (left, right, _) = picker::row_text(cat(), &r, s.wallet.balance(&name));
                    out.push_str(&format!(
                        "\n{:<12} {:<34} {left} – {right}",
                        slot.id(),
                        r.part.as_deref().unwrap_or("")
                    ));
                }
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.buy")
            .about("Buy a part with hearts and fit it (default: the bike on the stand)")
            .arg(Arg::new("part").required(true))
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            let part = m.get_one::<String>("part").expect("required").clone();
            garage_op(world, m, |w, name, bike| {
                let r = w.buy(cat(), name, bike, &part)?;
                Ok(format!(
                    "bought {} for {}; balance {}",
                    cat().get(&part).map_or(part.as_str(), |p| p.name.as_str()),
                    hearts(i64::from(r.paid)),
                    hearts(r.balance)
                ))
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.equip")
            .about("Fit an owned part (item number) on a bike")
            .arg(
                Arg::new("item")
                    .required(true)
                    .value_parser(clap::value_parser!(u32)),
            )
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            let item = *m.get_one::<u32>("item").expect("required");
            garage_op(world, m, |w, name, bike| w.equip(cat(), name, bike, item))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.unequip")
            .about("Back to the slot's base part; the part goes to the spares")
            .arg(
                Arg::new("slot")
                    .required(true)
                    .value_parser(Slot::ALL.map(|s| s.id())),
            )
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            let slot = Slot::parse(m.get_one::<String>("slot").expect("required")).expect("parsed");
            garage_op(world, m, |w, name, bike| {
                Ok(match w.unequip(name, bike, slot)? {
                    Some(i) => format!("item {i} is a spare part now"),
                    None => "the base part is on already".into(),
                })
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.sell")
            .about("Sell an owned part: the full price before its first ride, half after")
            .arg(
                Arg::new("item")
                    .required(true)
                    .value_parser(clap::value_parser!(u32)),
            ),
        |world, m| {
            let item = *m.get_one::<u32>("item").expect("required");
            garage_op(world, m, |w, name, _| {
                let refund = w.sell(name, item)?;
                Ok(format!(
                    "sold for {}; balance {}",
                    hearts(i64::from(refund)),
                    hearts(w.balance(name))
                ))
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.paint")
            .about("Paint a bike's frame (#RRGGBB, or default: the rider's frame colour; free)")
            .arg(Arg::new("colour").required(true))
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            let c = m.get_one::<String>("colour").expect("required");
            let colour = match c.as_str() {
                "default" => None,
                v => Some(Rgb::parse(v).ok_or_else(|| format!("not a colour: {v} (#RRGGBB)"))?),
            };
            garage_op(world, m, |w, name, bike| {
                w.paint(name, bike, colour)?;
                Ok(format!(
                    "{bike} painted {}",
                    colour.map_or("in the rider's colour".to_string(), |c| c.to_string())
                ))
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bikes.list").about("The bikes in your garage"),
        |world, _| {
            let (name, _) = rider(world);
            let s = world.resource::<WorkshopState>();
            let g = s.wallet.get(&name).ok_or("no garage")?;
            Ok(g.bikes
                .iter()
                .map(|b| {
                    format!(
                        "{} {:<14} {:<24} {:.2} kg, {} parts{}",
                        if b.id == g.active { "*" } else { " " },
                        b.id,
                        b.name(),
                        bike_kg(g, b),
                        g.part_ids(b).len(),
                        if b.starter { ", starter bike" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bikes.select")
            .about("Put a bike on the stand: the one you ride (applies when the workshop closes)")
            .arg(Arg::new("bike").required(true))
            .arg(
                Arg::new("now")
                    .long("now")
                    .action(ArgAction::SetTrue)
                    .help("apply at once (outside the workshop)"),
            ),
        |world, m| {
            let id = m.get_one::<String>("bike").expect("required").clone();
            let (name, _) = rider(world);
            let profile = {
                let mut s = world.resource_mut::<WorkshopState>();
                s.wallet.select(&name, &id)?;
                s.save()?;
                s.wallet
                    .get(&name)
                    .and_then(|g| g.bike(&id))
                    .map(|b| b.profile())
            };
            let profile = profile.unwrap_or_default();
            // The rider profile rides it from now on (riders.toml).
            let has_profile = world
                .get_resource::<RiderProfiles>()
                .is_some_and(|r| r.riders.find(&name).is_some());
            if has_profile {
                world
                    .resource_mut::<ConsoleState>()
                    .queue(format!("riders.edit \"{name}\" --bike {}", profile.id()));
            }
            let open = world.resource::<WorkshopState>().scene.is_some();
            if m.get_flag("now") || !open {
                apply_bike(world);
            }
            Ok(format!("{} on the stand", profile.name()))
        },
    ));
}

/// The bike of `--bike` (default: the active one).
fn bike_arg<'g>(g: &'g Garage, m: &clap::ArgMatches) -> Result<&'g wallet::Bike, String> {
    match m.try_get_one::<String>("bike").ok().flatten() {
        Some(id) => g
            .bike(id)
            .ok_or_else(|| format!("no bike {id} (bikes.list)")),
        None => Ok(g.active_bike()),
    }
}

/// Runs a garage change for the riding rider and saves the wallet.
fn garage_op(
    world: &mut World,
    m: &clap::ArgMatches,
    op: impl FnOnce(&mut Wallet, &str, &str) -> Result<String, String>,
) -> Result<String, String> {
    let (name, bike) = rider(world);
    let mut s = world.resource_mut::<WorkshopState>();
    s.wallet.garage(&name, bike);
    let id = {
        let g = s.wallet.get(&name).ok_or("no garage")?;
        bike_arg(g, m)?.id.clone()
    };
    let said = op(&mut s.wallet, &name, &id)?;
    s.save()?;
    if s.scene.is_none()
        && id
            == s.wallet
                .get(&name)
                .map_or(String::new(), |g| g.active.clone())
    {
        apply_bike(world);
    }
    Ok(said)
}

/// The bike's weight (kg): the profile's bike mass with the parts'
/// differences to the base parts.
pub fn bike_kg(g: &Garage, b: &wallet::Bike) -> f64 {
    let base = b.profile().profile().bike_mass_kg;
    let delta: i64 = Slot::ALL
        .into_iter()
        .filter_map(|s| {
            let on = g.part_on(cat(), b, s)?;
            let stock = match s {
                Slot::Frame => cat().base_frame(b.profile())?,
                _ => cat().base(s, b.profile().bike())?,
            };
            Some(i64::from(on.weight_g) - i64::from(stock.weight_g))
        })
        .sum();
    base + delta as f64 / 1000.0
}

/// The value of the parts on a bike (hearts paid).
fn bike_value(g: &Garage, b: &wallet::Bike) -> u32 {
    b.parts
        .values()
        .filter_map(|i| g.item(*i))
        .map(|i| i.paid)
        .sum()
}

/// Opens the workshop over the ride.
fn open(world: &mut World) -> Result<String, String> {
    let (name, bike) = rider(world);
    let layout = {
        let mut s = world.resource_mut::<WorkshopState>();
        if s.scene.is_some() {
            return Ok("the workshop is open".into());
        }
        let g = s.wallet.garage(&name, bike).clone();
        let layout = ocw_assets::workshop::layout(g.bikes.len());
        s.scene = Some(Workshop::open(&g, &layout));
        layout
    };
    let _ = layout;
    if let Some(mut c) = world.get_resource_mut::<ClientState>() {
        c.0.hold_steering();
    }
    info!("workshop opened");
    Ok(String::new())
}

/// Sends the active bike to the server (`BikeChange`) and makes it the
/// riding profile's bike (a ride switch keeps it).
fn apply_bike(world: &mut World) {
    let (name, _) = rider(world);
    let (bike_id, profile, paint, parts) = {
        let s = world.resource::<WorkshopState>();
        let Some(g) = s.wallet.get(&name) else { return };
        let b = g.active_bike();
        (b.id.clone(), b.profile(), b.paint(), g.part_ids(b))
    };
    let mut look: RiderLook = crate::settings::local_rider(world).look();
    if let Some(p) = paint {
        look.frame = p;
    }
    if let Some(mut r) = world.get_resource_mut::<RiderProfiles>() {
        r.riding.set_bike(profile);
    }
    // The server answers with the build's effect (printed when it
    // arrives); in an event that is not over it applies after the event.
    let msg = world
        .get_resource_mut::<ClientState>()
        .map(|mut c| c.0.change_bike(&bike_id, profile, look, &parts));
    if let (Some(m), Some(link)) = (msg, world.get_resource::<Link>()) {
        link.send_bytes(m);
        info!(bike = %bike_id, profile = profile.id(), parts = parts.len(), "bike change sent");
    }
}

/// Books the hearts the server credited (spec §14 Phase 4, 4E) and marks
/// the parts on the active bike as ridden.
fn earn_hearts(
    time: Res<Time>,
    args: Res<Args>,
    client: Option<Res<ClientState>>,
    session: Option<Res<crate::connection::SessionStart>>,
    riders: Option<Res<RiderProfiles>>,
    mut state: ResMut<WorkshopState>,
) {
    let Some(client) = client else { return };
    let name = riders.map_or_else(
        || crate::settings::riders::Rider::default().name,
        |r| r.riding.name.clone(),
    );
    let start = session.map_or(0.0, |s| s.0);
    if state.session != start {
        // A new session: the server counts from zero.
        state.session = start;
        state.credited = 0;
        state.last_distance = None;
    }
    let total = client.0.server_hearts();
    let earns = wallet::earns(
        args.server.is_none(),
        args.capture.is_some(),
        args.no_hearts,
    );
    if total > state.credited {
        let new = total - state.credited;
        state.credited = total;
        if earns {
            state.wallet.credit(&name, new, "ride");
            state.dirty = true;
        }
    }
    // Ridden parts (refund rule): the own rider's distance.
    let d = client.0.state().distance;
    if let Some(last) = state.last_distance {
        let step = d - last;
        if (0.0..50.0).contains(&step) && state.wallet.ride(&name, step) {
            state.dirty = true;
        }
    }
    state.last_distance = Some(d);
    state.save_in -= time.delta_secs_f64();
    if state.dirty && state.save_in <= 0.0 {
        state.save_in = 10.0;
        if let Err(e) = state.save() {
            warn!("workshop: saving the wallet: {e}");
        }
    }
}

/// The stand bike's hotspot anchors (world) and every bike's box.
fn geometry(
    w: &Workshop,
    g: &Garage,
    models: &BikeModels,
    layout: &ocw_assets::workshop::Layout,
    looks: &[(Vec<String>, Option<Rgb>)],
) -> (Vec<(Slot, Vec3)>, Vec<[Vec3; 8]>) {
    let mut anchors = Vec::new();
    let mut boxes = Vec::new();
    let swap = w.swap.map(|s| (s.from, s.to, s.progress()));
    for (i, id) in w.bikes.iter().enumerate() {
        let Some(b) = g.bike(id) else { continue };
        let built = models.get(b.profile(), &looks[i].0);
        let pose = bike_pose(layout, i, w.stand, swap, &built);
        boxes.push(bike_corners(pose, &built));
        if i == w.stand {
            for (slot, p) in &built.anchors {
                if let Some(s) = Slot::parse(slot) {
                    anchors.push((s, pose.0 + pose.1 * *p));
                }
            }
        }
    }
    (anchors, boxes)
}

/// The parts' models of every bike (the stand bike with the preview).
fn looks(
    w: &Workshop,
    g: &Garage,
    ctx_stand: Option<(Vec<String>, Option<Rgb>)>,
) -> Vec<(Vec<String>, Option<Rgb>)> {
    w.bikes
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if i == w.stand
                && let Some(l) = &ctx_stand
            {
                return l.clone();
            }
            let Some(b) = g.bike(id) else {
                return (Vec::new(), None);
            };
            let models = Slot::ALL
                .into_iter()
                .filter_map(|s| g.part_on(cat(), b, s).map(|p| p.model.clone()))
                .collect();
            (models, b.paint())
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn workshop_input(
    time: Res<Time>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mut touches: MessageReader<TouchInput>,
    pads: Query<&Gamepad>,
    windows: Query<&Window, With<PrimaryWindow>>,
    context: Res<InputContext>,
    mut pointer: ResMut<WorkshopPointer>,
    models: Res<BikeModels>,
    riders: Option<Res<RiderProfiles>>,
    mut console: ResMut<ConsoleState>,
    mut state: ResMut<WorkshopState>,
    mut client: Option<ResMut<ClientState>>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let touch_events: Vec<TouchInput> = touches.read().cloned().collect();
    let pointer_events = std::mem::take(&mut pointer.0);
    if state.scene.is_none() {
        return;
    }
    let st = &mut *state;
    let mut acts = std::mem::take(&mut st.queued);
    // Devices only in the workshop's context (not under the console).
    if *context == InputContext::Workshop {
        if let Some(k) = &keys {
            let a = input::keyboard(k, dt);
            if !a.is_empty() {
                st.active = ActiveInput::Keyboard;
            }
            acts.extend(a);
        }
        let mut latch = st.latch;
        for pad in &pads {
            let p = Pad::of(pad);
            if p.active() {
                st.active = ActiveInput::Gamepad;
            }
            acts.extend(input::gamepad(&p, &mut latch, dt));
        }
        st.latch = latch;
        for t in &touch_events {
            st.last_touch_s = now;
            st.active = ActiveInput::Touch;
            let e = match t.phase {
                TouchPhase::Started => Touch::Down {
                    id: t.id,
                    at: t.position,
                    t: now,
                },
                TouchPhase::Moved => Touch::Move {
                    id: t.id,
                    at: t.position,
                    t: now,
                },
                TouchPhase::Ended | TouchPhase::Canceled => Touch::Up {
                    id: t.id,
                    at: t.position,
                    t: now,
                },
            };
            acts.extend(st.gestures.event(e));
        }
        acts.extend(st.gestures.tick(now));
        // The mouse, unless a touch just happened (Windows' emulated mouse).
        if let (Some(b), Ok(w)) = (&buttons, windows.single())
            && now - st.last_touch_s > 0.5
        {
            let wheel = scroll.as_ref().map_or(0.0, |s| match s.unit {
                MouseScrollUnit::Line => s.delta.y,
                MouseScrollUnit::Pixel => s.delta.y / 40.0,
            });
            let moved = motion.as_ref().map_or(Vec2::ZERO, |m| m.delta);
            let m = input::Mouse {
                at: w.cursor_position(),
                moved,
                wheel,
            };
            let a = st.mouse.update(b, m, dt);
            if moved.length() > input::MOUSE_NUDGE_PX || b.get_just_pressed().next().is_some() {
                st.active = ActiveInput::Mouse;
            }
            acts.extend(a);
        }
        acts.extend(pointer_events.into_iter().map(|e| match e {
            WorkshopPointerEvent::Row(i) => Act::Row(i),
            WorkshopPointerEvent::RowClick(i) => Act::RowClick(i),
            WorkshopPointerEvent::Button(i) => Act::Button(i),
            WorkshopPointerEvent::Back => Act::BackButton,
        }));
    }
    let name = riders.as_ref().map_or_else(
        || crate::settings::riders::Rider::default().name,
        |r| r.riding.name.clone(),
    );
    let Some(g) = st.wallet.get(&name).cloned() else {
        return;
    };
    let balance = st.wallet.balance(&name);
    let size = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let scene = st.scene.as_mut().expect("open");
    let layout = ocw_assets::workshop::layout(scene.bikes.len());
    let looks0 = looks(scene, &g, None);
    let (anchors, boxes) = geometry(scene, &g, &models, &layout, &looks0);
    let ctx = Ctx {
        cat: cat(),
        garage: &g,
        balance,
        layout: &layout,
        size,
        anchors: &anchors,
        boxes: &boxes,
    };
    let mut out = scene.update(&acts, &ctx);
    out.extend(scene.tick(dt));
    for o in out {
        match o {
            Out::Line(l) => console.queue(l),
            Out::Leave { apply, then } => {
                st.scene = None;
                if let Some(c) = client.as_mut() {
                    c.0.release_steering();
                }
                if apply {
                    console.queue("workshop.apply");
                }
                console.queue(then.unwrap_or_else(|| "menu open".into()));
                info!("workshop closed");
                break;
            }
        }
    }
}

/// The scene → [`WorkshopView`].
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn write_view(
    state: Res<WorkshopState>,
    models: Res<BikeModels>,
    ride: Option<Res<RideView>>,
    riders: Option<Res<RiderProfiles>>,
    client: Option<Res<ClientState>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut view: ResMut<WorkshopView>,
) {
    let Some(scene) = &state.scene else {
        if view.open {
            *view = WorkshopView::default();
        }
        return;
    };
    let rider = riders
        .as_ref()
        .map(|r| r.riding.clone())
        .unwrap_or_default();
    let Some(g) = state.wallet.get(&rider.name) else {
        return;
    };
    let balance = state.wallet.balance(&rider.name);
    let layout = ocw_assets::workshop::layout(scene.bikes.len());
    let size = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    // The stand bike's look with the live preview.
    let looks0 = looks(scene, g, None);
    let (anchors, boxes) = geometry(scene, g, &models, &layout, &looks0);
    let ctx = Ctx {
        cat: cat(),
        garage: g,
        balance,
        layout: &layout,
        size,
        anchors: &anchors,
        boxes: &boxes,
    };
    let stand_look = scene.stand_look(&ctx);
    let all = looks(scene, g, Some(stand_look));
    let pose = scene.pose(&layout);
    let in_stand = matches!(scene.view(), camera::View::Stand) && !scene.cam.moving();
    let bikes = scene
        .bikes
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let b = g.bike(id)?;
            Some(BikeView {
                profile: b.profile(),
                variants: all[i].0.clone(),
                paint: all[i].1,
                card: vec![
                    b.name().to_string(),
                    format!("{} · {}", b.profile().bike().name(), b.profile().id()),
                    format!("{:.2} kg", bike_kg(g, b)),
                    format!("parts {}", hearts(i64::from(bike_value(g, b)))),
                    // The build physics (4G): what the parts do.
                    {
                        let built = picker::build_with(cat(), g, b, None);
                        built
                            .effect_from(&ocw_sim::Build::stock(built.profile))
                            .line()
                    },
                ],
            })
        })
        .collect();
    let hotspots = if in_stand {
        let stand = g.bike(&scene.bikes[scene.stand]);
        anchors
            .iter()
            .enumerate()
            .map(|(i, (slot, pos))| HotspotView {
                pos: *pos,
                label: format!(
                    "{} – {}",
                    slot.name(),
                    stand.and_then(|b| g.part_on(cat(), b, *slot)).map_or_else(
                        || {
                            stand
                                .and_then(|b| b.paint())
                                .map_or("rider's colour".into(), |c| c.to_string())
                        },
                        |p| p.name.clone()
                    )
                ),
                focused: i == scene.hotspot,
            })
            .collect()
    } else {
        Vec::new()
    };
    let picker = scene.picker.as_ref().map(|p| {
        let server = client.as_ref().and_then(|c| c.0.own_effect());
        picker_view(p, g, balance, scene, server)
    });
    let hint_ctx = if scene.picker.as_ref().is_some_and(|p| p.dialog.is_some()) {
        HintCtx::Dialog
    } else if scene.picker.is_some() {
        HintCtx::Picker
    } else {
        match scene.view() {
            camera::View::Stand => HintCtx::Stand,
            camera::View::Bench | camera::View::Mirror => HintCtx::Corner,
            _ => HintCtx::Room,
        }
    };
    let new = WorkshopView {
        open: true,
        eye: pose.eye,
        target: pose.target,
        fov_y: camera::FOV_Y,
        bikes,
        stand: scene.stand,
        swap: scene.swap.map(|s| (s.from, s.to, s.progress())),
        focus: (!scene.plus && scene.picker.is_none() && !in_stand).then_some(scene.focus),
        plus_focused: scene.plus,
        hotspots,
        picker,
        status: ride
            .as_deref()
            .map_or_else(String::new, crate::menu::status_line),
        hearts: hearts(balance),
        hint: input::hint(state.active, hint_ctx),
        caption: scene.caption.clone(),
        ride_window: state.ride_window,
        touch: state.active == ActiveInput::Touch,
        frame: rider.look().frame,
    };
    view.set_if_neq(new);
}

fn picker_view(
    p: &picker::Picker,
    g: &Garage,
    balance: i64,
    scene: &Workshop,
    server: Option<&ocw_protocol::BuildInfo>,
) -> PickerView {
    let bike = g.bike(&scene.bikes[scene.stand]);
    let mut last_group = "";
    let rows = p
        .rows
        .iter()
        .map(|r| {
            let gname = picker::group(&r.kind);
            let group = (gname != last_group).then(|| gname.to_string());
            last_group = gname;
            let (left, right, locked) = picker::row_text(cat(), r, balance);
            PickerRowView {
                group,
                left,
                right,
                locked,
            }
        })
        .collect();
    let equipped = bike.and_then(|b| g.part_on(cat(), b, p.slot));
    let focused = p.preview().and_then(|id| cat().get(id));
    let detail = match (p.slot, focused) {
        (Slot::Paint, _) => vec![
            picker::colour_name(p.preview_paint().flatten()),
            "Paint is free (special designs come later).".into(),
        ],
        (_, Some(f)) => {
            let ph = bike.map(|b| picker::CardPhysics::of(cat(), g, b, f));
            let mut d = picker::detail(f, equipped, ph.as_ref());
            // The server's word on the bike being ridden (its RiderBuild).
            if let (Some(e), Some(b)) = (server, bike)
                && b.profile() == e.profile
                && g.active_bike().id == b.id
            {
                d.insert(
                    d.len() - 1,
                    format!("Riding now (server): {}", e.effect.line()),
                );
            }
            d
        }
        _ => Vec::new(),
    };
    let compare = match (&p.pinned, focused) {
        (Some(a), Some(b)) => a.part.as_deref().and_then(|id| cat().get(id)).map(|a| {
            let builds = bike.map(|bk| {
                (
                    picker::build_with(cat(), g, bk, Some(a)),
                    picker::build_with(cat(), g, bk, Some(b)),
                )
            });
            picker::compare(a, b, builds.as_ref().map(|(x, y)| (x, y)))
        }),
        _ => None,
    };
    PickerView {
        title: format!("{} – {}", p.slot.name(), bike.map_or("", |b| b.name())),
        rows,
        focus: p.focus,
        detail,
        compare,
        dialog: p.dialog.as_ref().map(|d| DialogView {
            title: d.title.clone(),
            text: d.text.clone(),
            buttons: d.buttons.iter().map(|b| b.label.clone()).collect(),
            focus: d.focus,
        }),
    }
}

/// `workshop.apply`: run by the scene when it closes with a change.
pub fn register_apply(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.apply")
            .about("Ride the bike on the stand now (the workshop runs it when it closes)"),
        |world, _| {
            apply_bike(world);
            Ok(String::new())
        },
    ));
}

#[cfg(test)]
mod tests;
