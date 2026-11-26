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
//!
//! Both apps (spec 14 Phase 4, 4O part 3; B6): the module moved from the
//! desktop into `ocw-ui`; each app inserts its [`WorkshopHost`] (how the
//! ride runs, the wallet's folder, the account) and reaches its client and
//! server through [`crate::RideAccess`] ([`RideNow`]). Signed in, the
//! workshop is the account's (the desktop's `ocw-account` thread, the
//! browser's `fetch()` calls); signed out, the desktop's local test wallet,
//! while the browser's anonymous rider stays locked (Phase 10 Q5).
//!
//! W4 (spec 4C): the mirror is the character editor – the free character
//! (`character` command: build, skin, face, hair, glasses; the height
//! comes from the rider profile) and the kit (bought and worn like parts,
//! `workshop.buy helmet.tt.top`), *Try on the bike*; the look and the kit
//! ids go to the server with the bike when the workshop closes.
//!
//! Phase 8 T6: the jersey designer ([`designer`]; `kit.design` command:
//! pattern, three colours, race number, the pattern on the helmet, the
//! colours of shorts, socks, shoes and gloves, helmet and shoe finish –
//! free, saved in the rider profile, sent with the look).

pub mod bench;
pub mod camera;
pub mod designer;
pub mod host;
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
use ocw_core::{ProfileId, Rgb, RiderLook};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::InputContext;
use ocw_render::thumbs::{PartThumb, ThumbKey};
use ocw_render::workshop::{
    BikeModels, BikeView, DialogView, HotspotView, PickerRowView, PickerView, RiderShow, TilesView,
    WorkshopCorePlugin, WorkshopPointer, WorkshopPointerEvent, WorkshopView, bike_corners,
    bike_pose,
};
use wallet::shown as hearts;

use crate::profiles::RiderProfiles;
pub use host::{ActAnswer, WorkshopHost, WorkshopUpdate, host};
use input::{Act, ActiveInput, Gestures, HintCtx, MouseState, Pad, StickLatch, Touch};
use model::{Ctx, Out, Workshop};
use ocw_account_api::workshop::Action;
use wallet::{AccountView, Garage, Wallet};

pub struct WorkshopPlugin;

/// The workshop's state in the app.
#[derive(Resource)]
pub struct WorkshopState {
    pub wallet: Wallet,
    pub scene: Option<Workshop>,
    /// Actions from the console command `workshop`, applied next frame
    /// (public for the apps' tests).
    #[doc(hidden)]
    pub queued: Vec<Act>,
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
    /// The mirror's reflection (off: the low-quality direct front view,
    /// `workshop.mirror off`).
    pub mirror: bool,
    /// Booked hearts are saved at most this often (s).
    dirty: bool,
    save_in: f64,
    /// The session whose damage is kept, and the bike it started on (the
    /// server's damage is that bike's, also after a swap in the workshop).
    damage_session: f64,
    ride_bike: Option<String>,
    /// The server's damage last kept (per mille: rider, parts, spare):
    /// only a change of it is written, so a repair in the workshop is not
    /// overwritten before the server has it.
    last_damage: Option<Vec<u32>>,
    /// Signed in (spec 14 Phase 4, 4O): the local test wallet set aside
    /// while `wallet` is the account's; back when signed out.
    local: Option<Wallet>,
    /// The account workshop's generation last taken.
    account_gen: u64,
    /// Signed in, the account's workshop not known yet: "connecting".
    pub connecting: bool,
    /// Account actions sent and not answered: id → apply the bike after.
    pending: std::collections::BTreeMap<u64, bool>,
    next_action: u64,
    /// The ride's damage for the account, sent when it answers (the
    /// newest replaces an unsent one).
    damage_report: Option<Action>,
    /// Actions sent without a word in the console (the damage reports).
    quiet: std::collections::BTreeSet<u64>,
    /// The account's workshop as last synced (the garage again under a
    /// new rider name, W5b).
    account_state: Option<ocw_account_api::workshop::WorkshopState>,
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
            mirror: true,
            dirty: false,
            save_in: 0.0,
            damage_session: f64::NAN,
            ride_bike: None,
            last_damage: None,
            local: None,
            account_gen: 0,
            connecting: false,
            pending: std::collections::BTreeMap::new(),
            next_action: 0,
            damage_report: None,
            quiet: std::collections::BTreeSet::new(),
            account_state: None,
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
    let r = crate::profiles::local_rider(world);
    (r.name.clone(), r.bike())
}

/// The part ids on the bike `rider` rides (the garage's active bike, if
/// it is of the riding `profile`) and the worn kit, for the hello (spec
/// §14 Phase 4, 4G, 4C).
pub fn riding_parts(world: &World, rider: &str, profile: ProfileId) -> Vec<String> {
    let Some(g) = world
        .get_resource::<WorkshopState>()
        .and_then(|s| s.wallet.get(rider))
    else {
        return Vec::new();
    };
    if g.active_bike().profile() == profile {
        g.riding_ids(g.active_bike())
    } else {
        g.kit_ids()
    }
}

/// The condition the next ride carries over (spec §14 Phase 12 Q6): the
/// rider's injury healed over the real time since, and the parts of the
/// garage's active bike; `None`: fresh.
pub fn riding_damage(world: &World, rider: &str) -> Option<ocw_protocol::v1::DamageState> {
    let s = world.get_resource::<WorkshopState>()?;
    let g = s.wallet.get(rider)?;
    let carried = g.carried(g.active_bike());
    ocw_client::damage::Carried::hello(carried.as_ref(), false, s.wallet.clock())
}

/// How the garage works in this session (spec 3.2.4 Q4).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// The wallet's rules: hearts, prices, locks (Phase 4, 4E; the tests,
    /// and the base of Campaign mode, postponed: one old rusty city bike at
    /// the start, the rest earned).
    #[default]
    Hearts,
    /// *Ride locally*: every bike and all gear free, no hearts.
    Training,
}

/// Whether the server's damage goes into the garage (spec §14 Phase 12
/// Q6): never in captures (scripted runs) or with `--no-hearts` (the
/// kiosk / show mode); from local and remote servers alike.
pub fn keeps_damage(capture: bool, no_hearts: bool) -> bool {
    !capture && !no_hearts
}

impl Plugin for WorkshopPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<WorkshopCorePlugin>() {
            app.add_plugins(WorkshopCorePlugin);
        }
        // Ride locally rides in Training mode (spec 3.2.4 Q4): its own
        // garage, everything free. Campaign mode (postponed) will use the
        // wallet with its hearts.
        let training = app.world().get_resource::<PlayMode>() == Some(&PlayMode::Training);
        // The app's wallet folder (none: in memory – the browser, tests).
        let (mut wallet, warnings) = match (host(app.world()).wallet_dir)() {
            Some(dir) if training => Wallet::load(&dir.join(wallet::TRAINING_DIR)),
            Some(dir) => Wallet::load(&dir),
            None => (Wallet::in_memory(), Vec::new()),
        };
        wallet.training = training;
        for w in warnings {
            warn!("workshop: {w}");
        }
        let mut state = WorkshopState::new(wallet);
        let (name, bike) = rider(app.world());
        // The kit's colours live with the garage (spec 4C): also for the
        // default rider, who has no rider profile to keep them.
        let colours = state.wallet.garage(&name, bike).kit_colours.clone();
        if let Some(c) = colours
            && let Some(mut r) = app.world_mut().get_resource_mut::<RiderProfiles>()
        {
            let mut l = r.riding.look();
            c.apply(&mut l.design);
            r.riding.set_look(l);
        }
        app.insert_resource(state)
            .init_resource::<RideNow>()
            // (Both are there in the apps; headless tests may lack them.)
            .init_resource::<InputContext>()
            .add_message::<TouchInput>()
            .add_systems(
                Update,
                (
                    read_ride,
                    follow_account,
                    earn_hearts,
                    keep_damage,
                    workshop_input,
                    write_ride,
                    write_view,
                )
                    .chain(),
            );
        register_commands(app);
        register_apply(app);
    }
}

/// Console lines of the scene's actions (scripts, scenes, tests).
const ACTIONS: [&str; 18] = [
    "bench",
    "plus",
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
    "zoom",
    "mirror",
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
                    .help("orbit: degrees to turn; zoom: steps closer (negative: away)")
                    .allow_negative_numbers(true)
                    .value_parser(clap::value_parser!(f32)),
            ),
        |world, m| {
            locked(world)?;
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
                        "bench" => Act::Catalogue { plus: false },
                        "plus" => Act::Catalogue { plus: true },
                        "zoom" => {
                            Act::Zoom(m.get_one::<f32>("amount").copied().unwrap_or(3.0))
                        }
                        "mirror" => {
                            // Straight to the character editor (scripts).
                            let mut s = world.resource_mut::<WorkshopState>();
                            if s.scene.is_none() {
                                return Err("the workshop is not open (workshop open)".into());
                            }
                            s.queued.extend([Act::Down, Act::Down, Act::Confirm]);
                            return Ok(String::new());
                        }
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
        Command::new("workshop.mirror")
            .about("The mirror's reflection in the changing corner (off: a direct front view, for slow graphics)")
            .arg(Arg::new("on").value_parser(["on", "off"]).required(true)),
        |world, m| {
            let on = m.get_one::<String>("on").is_some_and(|v| v == "on");
            let mut s = world.resource_mut::<WorkshopState>();
            s.mirror = on;
            Ok(format!(
                "mirror {}",
                if on { "reflection on" } else { "off: front view" }
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        kit_design_command(),
        |world, m| {
            locked(world)?;
            let mut l = crate::profiles::local_rider(world).look();
            let before = l;
            apply_design_args(&mut l, m);
            if l != before {
                set_look(world, l)?;
                if world.resource::<WorkshopState>().scene.is_none() {
                    apply_bike(world);
                }
            }
            Ok(design_summary(&l))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        // One definition for both apps (spec §14 Phase 4, 4N; B6).
        crate::character::command(),
        |world, m| {
            locked(world)?;
            let rider = crate::profiles::local_rider(world);
            let mut c = rider.character();
            let before = c;
            crate::character::apply(&mut c, m, rider.weight_kg, rider.height_cm)?;
            if c == before {
                return Ok(c.summary());
            }
            set_character(world, c)?;
            if world.resource::<WorkshopState>().scene.is_none() {
                apply_bike(world);
            }
            Ok(c.summary())
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
            locked(world)?;
            not_with_account(world, "hearts.grant")?;
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
            locked(world)?;
            let part = m.get_one::<String>("part").expect("required").clone();
            if let Some(r) = via_account(world, "buying", false, |g| {
                Ok(Action::BuyPart {
                    bike: bike_arg(g, m)?.id.clone(),
                    part: part.clone(),
                })
            }) {
                return r;
            }
            garage_op(world, m, |w, name, bike| {
                let r = w.buy(cat(), name, bike, &part)?;
                let with = if r.fitted.is_empty() {
                    String::new()
                } else {
                    format!(", fitted with its {}", r.fitted.join(", "))
                };
                Ok(format!(
                    "bought {} for {}{with}; balance {}",
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
            locked(world)?;
            let item = *m.get_one::<u32>("item").expect("required");
            if let Some(r) = via_account(world, "fitting", false, |g| {
                Ok(Action::Equip {
                    bike: bike_arg(g, m)?.id.clone(),
                    item,
                })
            }) {
                return r;
            }
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
            locked(world)?;
            let slot = Slot::parse(m.get_one::<String>("slot").expect("required")).expect("parsed");
            if let Some(r) = via_account(world, "taking it off", false, |g| {
                Ok(Action::Unequip {
                    bike: bike_arg(g, m)?.id.clone(),
                    slot: slot.id().into(),
                })
            }) {
                return r;
            }
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
            locked(world)?;
            let item = *m.get_one::<u32>("item").expect("required");
            if let Some(r) = via_account(world, "selling", false, |_| Ok(Action::Sell { item })) {
                return r;
            }
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
        Command::new("workshop.repair")
            .about("Repair a damaged part (or all) for hearts: ceil(price x (1 - condition) x 0.3)")
            .arg(
                Arg::new("part").required(true).value_parser(
                    ocw_sim::damage::Part::ALL
                        .map(|p| p.id())
                        .into_iter()
                        .chain(["all"])
                        .collect::<Vec<_>>(),
                ),
            )
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            locked(world)?;
            let part = m.get_one::<String>("part").expect("required").clone();
            // Signed in: the account repairs (its price, 4O part 4).
            if let Some(r) = via_account(world, "repairing", false, |g| {
                Ok(Action::Repair {
                    bike: bike_arg(g, m)?.id.clone(),
                    part: (part != "all").then(|| part.clone()),
                })
            }) {
                return r;
            }
            garage_op(world, m, |w, name, bike| {
                if part == "all" {
                    let (paid, n) = w.repair_all(cat(), name, bike)?;
                    return Ok(format!(
                        "repaired {n} part{} for {}; balance {}",
                        if n == 1 { "" } else { "s" },
                        hearts(i64::from(paid)),
                        hearts(w.balance(name))
                    ));
                }
                let p = ocw_sim::damage::Part::from_id(&part).expect("parsed");
                let paid = w.repair(cat(), name, bike, p)?;
                Ok(format!(
                    "repaired the {} for {}; balance {}",
                    p.label(),
                    hearts(i64::from(paid)),
                    hearts(w.balance(name))
                ))
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("workshop.damage")
            .about("Set a part's condition in the garage (rider: the injury; tests and screenshots)")
            .arg(
                Arg::new("part").required(true).value_parser(
                    ocw_sim::damage::Part::ALL
                        .map(|p| p.id())
                        .into_iter()
                        .chain(["rider"])
                        .collect::<Vec<_>>(),
                ),
            )
            .arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(clap::value_parser!(f64)),
            )
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            locked(world)?;
            let part = m.get_one::<String>("part").expect("required").clone();
            not_with_account(world, "workshop.damage")?;
            let c = (m.get_one::<f64>("percent").expect("required") / 100.0).clamp(0.0, 1.0);
            garage_op(world, m, |w, name, bike| {
                let now = w.clock();
                let g = w.get(name).ok_or("no garage")?;
                let b = g.bike(bike).ok_or("no such bike")?;
                let mut kept = ocw_client::damage::Carried {
                    rider: g.rider_condition(now),
                    parts: b.conditions(),
                    at_unix: now,
                };
                match ocw_sim::damage::Part::from_id(&part) {
                    Some(p) => kept.parts[p.index()] = c,
                    None => kept.rider = c,
                }
                w.keep_damage(name, bike, &kept);
                Ok(format!("{part} {:.0} %", c * 100.0))
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workshop.paint")
            .about("Paint a bike's frame (#RRGGBB, carbon: bare carbon under a clear coat, or default: the rider's frame colour; free)")
            .arg(Arg::new("colour").required(true))
            .arg(Arg::new("bike").long("bike")),
        |world, m| {
            locked(world)?;
            let c = m.get_one::<String>("colour").expect("required");
            let colour = match c.as_str() {
                "default" => None,
                "carbon" => Some(ocw_core::bike::BARE_CARBON),
                v => Some(Rgb::parse(v).ok_or_else(|| format!("not a colour: {v} (#RRGGBB)"))?),
            };
            if let Some(r) = via_account(world, "painting", true, |g| {
                Ok(Action::Paint {
                    bike: bike_arg(g, m)?.id.clone(),
                    colour: colour.map(|c| c.to_string()),
                })
            }) {
                return r;
            }
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
                        b.label(),
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
            locked(world)?;
            let id = m.get_one::<String>("bike").expect("required").clone();
            if let Some(r) = via_account(world, "selecting", true, |_| Ok(Action::Select { bike: id.clone() })) {
                return r;
            }
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
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bikes.buy")
            .about("Buy a frameset as a new bike on a new hook (base parts; --with: a complete build at the sum of the prices)")
            .arg(Arg::new("frame").required(true))
            .arg(
                Arg::new("with")
                    .long("with")
                    .action(ArgAction::Append)
                    .value_name("PART")
                    .help("A part bought with it (groupset, wheels …)"),
            ),
        |world, m| {
            locked(world)?;
            let frame = m.get_one::<String>("frame").expect("required").clone();
            let with: Vec<String> = m
                .get_many::<String>("with")
                .map(|v| v.cloned().collect())
                .unwrap_or_default();
            if let Some(r) = via_account(world, "buying the bike", false, |_| {
                Ok(Action::BuyBike {
                    frame: frame.clone(),
                    with: with.clone(),
                })
            }) {
                return r;
            }
            let (name, bike) = rider(world);
            let mut s = world.resource_mut::<WorkshopState>();
            s.wallet.garage(&name, bike);
            let r = s.wallet.buy_bike(cat(), &name, &frame, &with)?;
            s.save()?;
            let label = s
                .wallet
                .get(&name)
                .and_then(|g| g.bike(&r.bike))
                .map_or(r.bike.clone(), |b| b.label());
            let fitted = if r.fitted.is_empty() {
                String::new()
            } else {
                format!(", with the {}", r.fitted.join(", "))
            };
            Ok(format!(
                "bought the {label} ({}) for {}{fitted}; it hangs on a new hook; balance {}",
                r.bike,
                hearts(i64::from(r.paid)),
                hearts(r.balance)
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bikes.sell")
            .about("Sell a bike bought at the bench: the frameset's and its parts' refunds (not a starter bike, not the last)")
            .arg(Arg::new("bike").required(true)),
        |world, m| {
            locked(world)?;
            let id = m.get_one::<String>("bike").expect("required").clone();
            if let Some(r) = via_account(world, "selling the bike", true, |_| Ok(Action::SellBike { bike: id.clone() })) {
                return r;
            }
            let (name, bike) = rider(world);
            let was_active = {
                let mut s = world.resource_mut::<WorkshopState>();
                s.wallet.garage(&name, bike);
                let active = s.wallet.get(&name).is_some_and(|g| g.active == id);
                let refund = s.wallet.sell_bike(&name, &id)?;
                s.save()?;
                (active, refund)
            };
            let (active, refund) = was_active;
            // The stand's bike sold: the next one is ridden.
            if active && world.resource::<WorkshopState>().scene.is_none() {
                apply_bike(world);
            }
            let s = world.resource::<WorkshopState>();
            Ok(format!(
                "sold {id} for {}; balance {}",
                hearts(i64::from(refund)),
                hearts(s.wallet.balance(&name))
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bikes.hang")
            .about("Hang a spare frameset (item number) on a new hook as a bike with the base parts")
            .arg(
                Arg::new("item")
                    .required(true)
                    .value_parser(clap::value_parser!(u32)),
            ),
        |world, m| {
            locked(world)?;
            let item = *m.get_one::<u32>("item").expect("required");
            not_with_account(world, "bikes.hang")?;
            let (name, bike) = rider(world);
            let mut s = world.resource_mut::<WorkshopState>();
            s.wallet.garage(&name, bike);
            let id = s.wallet.hang_frame(cat(), &name, item)?;
            s.save()?;
            Ok(format!("{id} hangs on a new hook"))
        },
    ));
}

/// `kit.design`: the jersey designer's console form (spec §14 Phase 8 T6).
fn kit_design_command() -> Command {
    let colour = |s: &str| Rgb::parse(s).ok_or_else(|| format!("{s:?} is not a colour #RRGGBB"));
    // A colour, or the word that means "none" for this option.
    let opt_colour = |none: &'static str| {
        move |s: &str| -> Result<Option<Rgb>, String> {
            if s.eq_ignore_ascii_case(none) {
                Ok(None)
            } else {
                Rgb::parse(s)
                    .map(Some)
                    .ok_or_else(|| format!("{s:?} is neither a colour #RRGGBB nor {none}"))
            }
        }
    };
    let finish =
        |s: &str| ocw_core::Finish::parse(s).ok_or_else(|| format!("{s:?}: gloss or matte"));
    let mut c = Command::new("kit.design")
        .about("The jersey designer (free): pattern, colours, race number, kit colours, finishes (no options: the current design)")
        .arg(
            Arg::new("pattern")
                .long("pattern")
                .value_parser(|s: &str| {
                    ocw_core::KitPattern::parse(s).ok_or_else(|| {
                        "plain, band, sash, stripes, chevrons or gradient".to_string()
                    })
                })
                .help("plain, band, sash, stripes, chevrons, gradient"),
        )
        .arg(
            Arg::new("jersey")
                .long("jersey")
                .value_name("#RRGGBB")
                .value_parser(colour)
                .help("First (main) colour"),
        )
        .arg(
            Arg::new("second")
                .long("second")
                .value_name("#RRGGBB|auto")
                .value_parser(opt_colour("auto"))
                .help("The pattern's colour (auto: white on dark, black on light)"),
        )
        .arg(
            Arg::new("third")
                .long("third")
                .value_name("#RRGGBB|none")
                .value_parser(opt_colour("none"))
                .help("A third colour (none: two colours)"),
        )
        .arg(
            Arg::new("number")
                .long("number")
                .value_name("1-999|none")
                .value_parser(|s: &str| -> Result<u16, String> {
                    if s.eq_ignore_ascii_case("none") {
                        return Ok(0);
                    }
                    match s.parse::<u16>() {
                        Ok(n) if (1..=ocw_core::kit::MAX_RACE_NUMBER).contains(&n) => Ok(n),
                        _ => Err(format!("{s:?}: a race number 1 to 999, or none")),
                    }
                })
                .help("Race number on the back and the shorts"),
        )
        .arg(
            Arg::new("helmet_pattern")
                .long("helmet-pattern")
                .value_parser(["on", "off"])
                .help("The pattern on the helmet too"),
        )
        .arg(
            Arg::new("helmet")
                .long("helmet")
                .value_name("#RRGGBB")
                .value_parser(colour)
                .help("Helmet colour (without the pattern)"),
        );
    for (id, what) in [
        ("shorts", "Shorts"),
        ("socks", "Socks"),
        ("shoes", "Shoes"),
        ("gloves", "Gloves"),
    ] {
        c = c.arg(
            Arg::new(id)
                .long(id)
                .value_name("#RRGGBB|standard")
                .value_parser(opt_colour("standard"))
                .help(format!("{what} colour (standard: the kit's own)")),
        );
    }
    for (id, long) in [
        ("helmet_finish", "helmet-finish"),
        ("shoe_finish", "shoe-finish"),
    ] {
        c = c.arg(
            Arg::new(id)
                .long(long)
                .value_parser(finish)
                .help("gloss or matte"),
        );
    }
    c
}

/// The `kit.design` options in `m` applied to `l`.
fn apply_design_args(l: &mut RiderLook, m: &clap::ArgMatches) {
    if let Some(p) = m.get_one::<ocw_core::KitPattern>("pattern") {
        l.pattern = *p;
    }
    if let Some(c) = m.get_one::<Rgb>("jersey") {
        l.jersey = *c;
    }
    if let Some(c) = m.get_one::<Rgb>("helmet") {
        l.helmet = *c;
    }
    let d = &mut l.design;
    for (id, field) in [
        ("second", &mut d.second),
        ("third", &mut d.third),
        ("shorts", &mut d.shorts),
        ("socks", &mut d.socks),
        ("shoes", &mut d.shoes),
        ("gloves", &mut d.gloves),
    ] {
        if let Some(v) = m.get_one::<Option<Rgb>>(id) {
            *field = *v;
        }
    }
    if let Some(n) = m.get_one::<u16>("number") {
        d.number = *n;
    }
    if let Some(v) = m.get_one::<String>("helmet_pattern") {
        d.helmet_pattern = v == "on";
    }
    if let Some(f) = m.get_one::<ocw_core::Finish>("helmet_finish") {
        d.helmet_finish = *f;
    }
    if let Some(f) = m.get_one::<ocw_core::Finish>("shoe_finish") {
        d.shoe_finish = *f;
    }
}

/// "Stripes, jersey #1E88E5, helmet #F2F2EE; second #FFFFFF, number 42, …".
fn design_summary(l: &RiderLook) -> String {
    format!(
        "{}, jersey {}, helmet {}; {}",
        l.pattern.name(),
        l.jersey,
        l.helmet,
        l.design.summary()
    )
}

/// Sets the riding rider's look (and the saved profile's of the same
/// name, riders.toml): colours, pattern, kit design.
fn set_look(world: &mut World, l: RiderLook) -> Result<(), String> {
    let mut p = world
        .get_resource_mut::<RiderProfiles>()
        .ok_or("no rider profiles")?;
    p.riding.set_look(l);
    let name = p.riding.name.clone();
    let saved = if let Some(i) = p.riders.find(&name) {
        p.riders.riders[i].set_look(l);
        true
    } else {
        false
    };
    if saved && p.path.is_some() {
        p.save()?;
    }
    // The kit's colours with the garage, in workshop.toml (spec 4C).
    if let Some(mut s) = world.get_resource_mut::<WorkshopState>()
        && s.wallet.set_kit_colours(&name, &l.design)
    {
        s.save()?;
    }
    Ok(())
}

/// Sets the riding rider's character (and the saved profile's of the same
/// name, riders.toml).
fn set_character(world: &mut World, c: ocw_core::Character) -> Result<(), String> {
    let mut p = world
        .get_resource_mut::<RiderProfiles>()
        .ok_or("no rider profiles")?;
    p.riding.set_character(c);
    let name = p.riding.name.clone();
    let saved = if let Some(i) = p.riders.find(&name) {
        p.riders.riders[i].set_character(c);
        true
    } else {
        false
    };
    if saved && p.path.is_some() {
        p.save()?;
    }
    Ok(())
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

/// Signed in (spec 14 Phase 4, 4O): `make` the action from the account's
/// garage and send it to the backend – answered later, never waited for
/// ([`follow_account`] applies the answer); `None`: not signed in, the
/// local wallet does it. Offline the workshop is read-only (Q20 (a)).
fn via_account(
    world: &mut World,
    what: &str,
    apply: bool,
    make: impl FnOnce(&Garage) -> Result<Action, String>,
) -> Option<Result<String, String>> {
    let (name, _) = rider(world);
    let s = world.resource::<WorkshopState>();
    let view = s.wallet.account.clone()?;
    let action = match s
        .wallet
        .get(&name)
        .ok_or_else(|| "no garage".to_string())
        .and_then(make)
    {
        Ok(a) => a,
        Err(e) => return Some(Err(e)),
    };
    if let Err(e) = ocw_account_api::offline::allowed(&action, view.online) {
        return Some(Err(e));
    }
    let id = {
        let mut s = world.resource_mut::<WorkshopState>();
        s.next_action += 1;
        let id = s.next_action;
        s.pending.insert(id, apply);
        if let Some(a) = &mut s.wallet.account {
            a.pending += 1;
        }
        id
    };
    let sent = (host(world).send_action)(world, id, action);
    if !sent {
        world.resource_mut::<WorkshopState>().pending.remove(&id);
        return Some(Err("sign-in is not set up".into()));
    }
    Some(Ok(format!("{what}: sent to your account …")))
}

/// The browser's anonymous rider may not customise (spec 14 Phase 10
/// Q5): the workshop, the character and the garage need an account there.
fn locked(world: &World) -> Result<(), String> {
    let h = host(world);
    let anonymous = h.requires_account && !(h.signed_in)(world);
    if ocw_core::character::may_customize(anonymous) {
        Ok(())
    } else {
        Err(ocw_core::character::LOCKED_MIRROR.into())
    }
}

/// What the account's workshop cannot do yet (repairs, hanging a spare
/// frame, the dev commands; spec 14 Phase 4, 4O *Not yet*).
fn not_with_account(world: &World, what: &str) -> Result<(), String> {
    if world.resource::<WorkshopState>().wallet.is_account() {
        return Err(format!(
            "{what} is not available with an account yet (signed out it uses the local test wallet)"
        ));
    }
    Ok(())
}

/// The balance line of the workshop: the account's, offline or connecting,
/// or the local test wallet's, marked local (spec 14 Phase 4, 4O).
pub fn balance_label(s: &WorkshopState, balance: i64) -> String {
    match &s.wallet.account {
        Some(a) if !a.online => format!("{} · account, offline (read-only)", hearts(balance)),
        Some(a) if a.pending > 0 => format!("{} · account (saving …)", hearts(balance)),
        Some(_) => format!("{} · account", hearts(balance)),
        None if s.connecting => format!("{} · connecting to your account …", hearts(balance)),
        None if s.wallet.training => format!("{} · local", hearts(balance)),
        None => format!("{} · local test wallet", hearts(balance)),
    }
}

/// Follows the sign-in (spec 14 Phase 4, 4O): signed in, the workshop is
/// the account's (the local test wallet set aside); signed out, the local
/// one again. Applies the answers of the actions sent.
///
/// Signed in, the account's workshop replaces Training mode's free garage
/// too (3.2.4 Q4 is the signed-out case): what the rider owns and earns is
/// the account's.
fn follow_account(world: &mut World) {
    let (name, _) = rider(world);
    let Some(seen) = world.get_resource::<WorkshopState>().map(|s| s.account_gen) else {
        return;
    };
    let h = host(world);
    let signed_in_now = (h.signed_in)(world);
    let upd = (h.workshop_update)(world, seen);
    let mut said = Vec::new();
    let mut apply = false;
    {
        let mut s = world.resource_mut::<WorkshopState>();
        s.connecting = signed_in_now && !upd.signed_in;
        if upd.signed_in {
            if let Some(sy) = &upd.synced {
                let view = AccountView {
                    account: sy.state.account.clone(),
                    hearts: sy.state.hearts,
                    online: upd.online,
                    synced_at: sy.synced_at,
                    queued_rides: upd.queued,
                    pending: s.pending.len(),
                };
                if s.wallet.is_account() {
                    let before = s.wallet.get(&name).map(|g| g.active.clone());
                    s.wallet.account = Some(view);
                    s.wallet.adopt(&name, &sy.state);
                    apply |= s.wallet.get(&name).map(|g| g.active.clone()) != before;
                } else {
                    let w = Wallet::for_account(view, &name, &sy.state);
                    let local = std::mem::replace(&mut s.wallet, w);
                    s.local = Some(local);
                    said.push(format!(
                        "workshop: your account's workshop, {}{}",
                        hearts(sy.state.hearts),
                        if upd.online {
                            ""
                        } else {
                            " (offline: read-only, as last synced)"
                        }
                    ));
                    apply = true;
                }
            }
            // (Unchanged generation: nothing new from the thread – every
            // change of `online` or the queue comes with a new one.)
            s.account_gen = upd.generation;
            if let Some(sy) = upd.synced {
                s.account_state = Some(sy.state);
            }
            // The riding rider renamed (the account's rider, W5b): the
            // account's garage under the new name.
            if s.wallet.is_account()
                && s.wallet.get(&name).is_none()
                && let Some(st) = s.account_state.clone()
            {
                s.wallet.adopt(&name, &st);
                apply = true;
            }
        } else if s.wallet.is_account() && !signed_in_now {
            if let Some(local) = s.local.take() {
                s.wallet = local;
            }
            s.pending.clear();
            s.quiet.clear();
            s.damage_report = None;
            s.account_state = None;
            s.account_gen = 0;
            said.push(if h.requires_account {
                "workshop: signed out".into()
            } else {
                "workshop: signed out – the local test wallet".into()
            });
            apply = true;
        }
        for a in upd.answers {
            let after = s.pending.remove(&a.id).unwrap_or(false);
            if s.quiet.remove(&a.id) {
                match a.result {
                    Ok(ans) => s.wallet.adopt_answer(&name, &ans),
                    Err(e) => debug!("workshop: the ride's damage not kept: {e}"),
                }
                continue;
            }
            match a.result {
                Ok(ans) => {
                    s.wallet.adopt_answer(&name, &ans);
                    let fitted = if ans.fitted.is_empty() {
                        String::new()
                    } else {
                        format!(", with the {}", ans.fitted.join(", "))
                    };
                    said.push(format!(
                        "{}{fitted}; balance {}",
                        ans.message,
                        hearts(ans.hearts)
                    ));
                    apply |= after;
                }
                Err(e) => said.push(format!("workshop: {e}")),
            }
        }
        let pending = s.pending.len();
        if let Some(a) = &mut s.wallet.account {
            a.pending = pending;
        }
    }
    if !said.is_empty()
        && let Some(mut c) = world.get_resource_mut::<ConsoleState>()
    {
        for m in said {
            info!("{m}");
            c.print(m);
        }
    }
    // The ride's damage for the account, once it answers (one at a time).
    let report = {
        let mut s = world.resource_mut::<WorkshopState>();
        let online = s.wallet.account.as_ref().is_some_and(|a| a.online);
        if online && s.quiet.is_empty() {
            s.damage_report.take().map(|r| {
                s.next_action += 1;
                let id = s.next_action;
                s.pending.insert(id, false);
                s.quiet.insert(id);
                (id, r)
            })
        } else {
            None
        }
    };
    if let Some((id, r)) = report
        && !(h.send_action)(world, id, r)
    {
        let mut s = world.resource_mut::<WorkshopState>();
        s.pending.remove(&id);
        s.quiet.remove(&id);
    }
    if apply && world.resource::<WorkshopState>().scene.is_none() {
        apply_bike(world);
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
    if let Some(mut c) = crate::client_mut(world) {
        c.hold_steering();
    }
    // Signed in: the account's balance and garage fresh from the backend
    // (answered later; the cached one shows meanwhile).
    let h = host(world);
    if (h.signed_in)(world) {
        (h.sync_workshop)(world);
    }
    info!("workshop opened");
    Ok(String::new())
}

/// Sends the active bike to the server (`BikeChange`) and makes it the
/// riding profile's bike (a ride switch keeps it).
fn apply_bike(world: &mut World) {
    let (name, _) = rider(world);
    let (bike_id, profile, paint, parts, damage) = {
        let s = world.resource::<WorkshopState>();
        let Some(g) = s.wallet.get(&name) else { return };
        let b = g.active_bike();
        // The bike's condition (Phase 12 Q6, W6): repairs and a swap reach
        // the ride with the bike.
        let damage =
            ocw_protocol::carried_damage(g.rider_condition(s.wallet.clock()), &b.conditions());
        (
            b.id.clone(),
            b.profile(),
            b.paint(),
            g.riding_ids(b),
            damage,
        )
    };
    let mut look: RiderLook = crate::profiles::local_rider(world).look();
    if let Some(p) = paint {
        look.frame = p;
    }
    if let Some(mut r) = world.get_resource_mut::<RiderProfiles>() {
        r.riding.set_bike(profile);
    }
    // The server answers with the build's effect (printed when it
    // arrives); in an event that is not over it applies after the event.
    let now = crate::client(world).is_some_and(|c| !c.bike_change_waits());
    let msg = crate::client_mut(world)
        .map(|mut c| c.change_bike_with_damage(&bike_id, profile, look, &parts, Some(damage)));
    // The ride's damage is this bike's from now on (spec §14 Phase 12 Q6).
    if now && let Some(mut s) = world.get_resource_mut::<WorkshopState>() {
        s.ride_bike = Some(bike_id.clone());
        s.last_damage = None;
    }
    if let Some(m) = msg
        && crate::send(world, m)
    {
        info!(bike = %bike_id, profile = profile.id(), parts = parts.len(), "bike change sent");
    }
}

/// What the workshop reads of the running ride each frame (through
/// [`crate::RideAccess`], the same in both apps), and what it asks of it.
#[derive(Resource, Debug)]
pub struct RideNow {
    /// A ride runs.
    pub riding: bool,
    /// The hearts the server counted in this session.
    pub hearts: u32,
    /// The own rider's distance (m).
    pub distance: f64,
    /// The own rider's damage (spec 12E).
    pub damage: Option<ocw_sim::damage::Damage>,
    /// The server's answer to the bike (its build's effect).
    pub effect: Option<ocw_protocol::BuildInfo>,
    pub mode: ocw_sim::PhysicsMode,
    /// When the session started (game time, s).
    pub session_start: f64,
    /// The workshop closed: give the steering back.
    pub release_steering: bool,
}

impl Default for RideNow {
    fn default() -> Self {
        Self {
            riding: false,
            hearts: 0,
            distance: 0.0,
            damage: None,
            effect: None,
            mode: ocw_sim::PhysicsMode::Open,
            session_start: 0.0,
            release_steering: false,
        }
    }
}

/// Reads the ride into [`RideNow`].
fn read_ride(world: &mut World) {
    let session_start = (host(world).session_start)(world);
    let now = match crate::client(world) {
        Some(c) => RideNow {
            riding: true,
            hearts: c.server_hearts(),
            distance: c.state().distance,
            damage: c.own_damage().cloned(),
            effect: c.own_effect().cloned(),
            mode: c.physics_mode(),
            session_start,
            release_steering: false,
        },
        None => RideNow {
            session_start,
            ..RideNow::default()
        },
    };
    world.insert_resource(now);
}

/// Hands the ride what the workshop asked of it.
fn write_ride(world: &mut World) {
    let release = std::mem::take(&mut world.resource_mut::<RideNow>().release_steering);
    if release && let Some(mut c) = crate::client_mut(world) {
        c.release_steering();
    }
}

/// Books the hearts the server credited (spec §14 Phase 4, 4E) and marks
/// the parts on the active bike as ridden.
fn earn_hearts(
    time: Res<Time>,
    host: Option<Res<WorkshopHost>>,
    ride: Res<RideNow>,
    riders: Option<Res<RiderProfiles>>,
    mut state: ResMut<WorkshopState>,
) {
    if !ride.riding {
        return;
    }
    let host = host.map(|h| *h).unwrap_or_default();
    let name = riders.map_or_else(
        || ocw_client::riders::Rider::default().name,
        |r| r.riding.name.clone(),
    );
    let start = ride.session_start;
    if state.session != start {
        // A new session: the server counts from zero.
        state.session = start;
        state.credited = 0;
        state.last_distance = None;
    }
    let total = ride.hearts;
    let earns = wallet::earns(host.local_server, host.capture, host.no_hearts);
    if total > state.credited {
        let new = total - state.credited;
        state.credited = total;
        if earns {
            state.wallet.credit(&name, new, "ride");
            state.dirty = true;
        }
    }
    // Ridden parts (refund rule): the own rider's distance.
    let d = ride.distance;
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

/// Keeps the server's damage of the own rider in the garage (spec §14
/// Phase 12 Q6): the active bike's parts (not a spare bike's) and the
/// rider's injury; saved with the wallet (at most every 10 s).
fn keep_damage(
    host: Option<Res<WorkshopHost>>,
    ride: Res<RideNow>,
    riders: Option<Res<RiderProfiles>>,
    mut state: ResMut<WorkshopState>,
) {
    let host = host.map(|h| *h).unwrap_or_default();
    // Not while the workshop is open (its repairs reach the ride when it
    // closes).
    if !keeps_damage(host.capture, host.no_hearts) || state.scene.is_some() {
        return;
    }
    let Some(d) = ride.damage.as_ref() else {
        return;
    };
    let name = riders.map_or_else(
        || ocw_client::riders::Rider::default().name,
        |r| r.riding.name.clone(),
    );
    let now = state.wallet.clock();
    let start = ride.session_start;
    let Some(active) = state.wallet.get(&name).map(|g| g.active.clone()) else {
        return;
    };
    if state.damage_session != start {
        state.damage_session = start;
        state.ride_bike = Some(active.clone());
        state.last_damage = None;
    }
    // Only a change of the server's damage is kept.
    let sig: Vec<u32> = std::iter::once(ocw_sim::damage::per_mille(d.rider))
        .chain(d.parts.iter().map(|c| ocw_sim::damage::per_mille(*c)))
        .chain([u32::from(d.spare)])
        .collect();
    if state.last_damage.as_ref() == Some(&sig) {
        return;
    }
    state.last_damage = Some(sig);
    let bike = state.ride_bike.clone().unwrap_or(active);
    let Some(g) = state.wallet.get(&name) else {
        return;
    };
    let Some(b) = g.bike(&bike) else {
        return;
    };
    let before = g.carried(b);
    let Some(kept) = ocw_client::damage::Carried::keep(d, before.as_ref(), false, now) else {
        return;
    };
    if state.wallet.keep_damage(&name, &bike, &kept) {
        state.dirty = true;
        // Signed in: the account keeps it (4O part 4), sent by
        // `follow_account` while the backend answers.
        if state.wallet.is_account()
            && let Some(g) = state.wallet.get(&name)
            && let Some(b) = g.bike(&bike)
        {
            let report = Action::Damage {
                bike: bike.clone(),
                condition: b.condition.clone(),
                injury: g.injury.map(|i| ocw_account_api::workshop::Injury {
                    condition_pm: i.condition_pm,
                    at_unix: i.at_unix,
                }),
            };
            state.damage_report = Some(report);
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
    mut ride_now: ResMut<RideNow>,
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
            WorkshopPointerEvent::Swatch(i) => Act::Swatch(i),
            WorkshopPointerEvent::SwatchClick(i) => Act::SwatchClick(i),
            WorkshopPointerEvent::Button(i) => Act::Button(i),
            WorkshopPointerEvent::Back => Act::BackButton,
        }));
    }
    let name = riders.as_ref().map_or_else(
        || ocw_client::riders::Rider::default().name,
        |r| r.riding.name.clone(),
    );
    let Some(g) = st.wallet.get(&name).cloned() else {
        return;
    };
    let balance = st.wallet.balance(&name);
    let size = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let character = riders
        .as_ref()
        .map(|r| r.riding.character())
        .unwrap_or_default();
    let rider_look = riders.as_ref().map(|r| r.riding.look()).unwrap_or_default();
    let mirror = st.mirror;
    let scene = st.scene.as_mut().expect("open");
    scene.front_view = !mirror;
    let layout = ocw_assets::workshop::layout(scene.bikes.len());
    let spots = layout.rider_hotspots(character.scale()).to_vec();
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
        character,
        look: rider_look,
        rider_spots: &spots,
    };
    let mut out = scene.update(&acts, &ctx);
    out.extend(scene.tick(dt));
    for o in out {
        match o {
            Out::Line(l) => console.queue(l),
            Out::Leave { apply, then } => {
                st.scene = None;
                ride_now.release_steering = true;
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
    ride_now: Res<RideNow>,
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
    let character = rider.character();
    let rider_look = rider.look();
    let spots = layout.rider_hotspots(character.scale()).to_vec();
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
        character,
        look: rider_look,
        rider_spots: &spots,
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
                    b.label(),
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
    let dress = matches!(scene.view(), camera::View::Dress { .. }) && !scene.cam.moving();
    let rider_points_label = |i: usize, r: picker::Region| HotspotView {
        damage: None,
        pos: spots[i],
        label: if i == scene.dress.hotspot {
            let slot = scene.dress.slot();
            // *Try on the bike* belongs to no region: its name alone.
            if slot.region() == Some(r) {
                format!("{} – {}", r.name(), slot.name())
            } else {
                slot.name().to_string()
            }
        } else {
            r.name().to_string()
        },
        focused: i == scene.dress.hotspot,
    };
    let hotspots = if dress && scene.picker.is_none() {
        picker::Region::ALL
            .iter()
            .enumerate()
            .map(|(i, r)| rider_points_label(i, *r))
            .collect()
    } else if in_stand {
        let stand = g.bike(&scene.bikes[scene.stand]);
        anchors
            .iter()
            .enumerate()
            .map(|(i, (slot, pos))| {
                // Damage (W6): the worst part of the slot in its colour and
                // words.
                let damage = stand.and_then(|b| picker::slot_damage(b, *slot));
                let mut label = format!(
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
                );
                if let (Some(b), Some((p, _))) = (stand, damage) {
                    label.push_str(&format!("\n{}", picker::repair_line(cat(), g, b, p)));
                }
                HotspotView {
                    pos: *pos,
                    label,
                    focused: i == scene.hotspot,
                    damage: damage.map(|(_, c)| ocw_sim::damage::part_status(c)),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let picker = scene.picker.as_ref().map(|p| {
        let server = ride_now.effect.as_ref();
        let mode = ride_now.mode;
        let mut v = picker_view(p, g, balance, scene, server, &character, rider.weight_kg);
        if p.tiles() && p.of_the_rider() {
            fill_rider_tiles(&mut v, p, scene, &ctx);
        } else if p.tiles() {
            fill_tiles(&mut v, p, g, scene, mode, rider_look.frame);
        }
        v
    });
    // The rider in the changing corner (W4): standing at the mirror, or on
    // the stand bike; with the live preview of the picker.
    let rider_show = scene.dressing().then(|| {
        let (c, mut kit) = scene.rider_look(&ctx);
        // The jersey designer's draft (T6), else the rider's look.
        let mut look = scene.design_preview().unwrap_or(rider_look);
        look.character = c;
        // The bare head (4N Q28): the helmet on its hook.
        if scene.helmet_off() {
            kit.push(ocw_assets::rider::HELMET_OFF.to_string());
        }
        RiderShow {
            look,
            kit,
            on_stand: scene.trying_on(),
            yaw: scene.dress.yaw,
        }
    });
    let hint_ctx = if scene.picker.as_ref().is_some_and(|p| p.dialog.is_some()) {
        HintCtx::Dialog
    } else if scene.picker.is_some() {
        HintCtx::Picker
    } else {
        match scene.view() {
            camera::View::Stand => HintCtx::Stand,
            camera::View::Bench | camera::View::Mirror => HintCtx::Corner,
            camera::View::Dress { .. } => HintCtx::Dress,
            camera::View::TryOn => HintCtx::TryOn,
            _ => HintCtx::Room,
        }
    };
    // The condition card (W6): the stand bike's damage and its repairs; at
    // the mirror the rider's injury and the time to heal.
    let at_mirror = scene.dressing()
        || matches!(
            scene.view(),
            camera::View::Mirror | camera::View::Dress { .. } | camera::View::TryOn
        );
    let condition = if at_mirror {
        injury_card(g.rider_condition(state.wallet.clock()))
    } else {
        g.bike(&scene.bikes[scene.stand])
            .map(|b| repair_card(g, b, balance))
            .unwrap_or_default()
    };
    let new = WorkshopView {
        condition,
        open: true,
        eye: pose.eye,
        target: pose.target,
        fov_y: pose.fov,
        bikes,
        stand: scene.stand,
        swap: scene.swap.map(|s| (s.from, s.to, s.progress())),
        focus: (!scene.plus && scene.picker.is_none() && !in_stand).then_some(scene.focus),
        plus_focused: scene.plus,
        hotspots,
        picker,
        status: ride
            .as_deref()
            .map_or_else(String::new, ocw_client::menu::status_line),
        hearts: balance_label(&state, balance),
        hint: input::hint(state.active, hint_ctx),
        caption: scene.caption.clone(),
        ride_window: state.ride_window,
        touch: state.active == ActiveInput::Touch,
        frame: rider.look().frame,
        helmet_on_hook: rider_show
            .as_ref()
            .filter(|_| scene.helmet_off())
            .map(|r| r.kit.clone()),
        rider: rider_show,
        mirror: state.mirror,
    };
    view.set_if_neq(new);
}

/// The stand bike's condition card (W6): its damaged parts, worst first,
/// with the repair prices, and *Repair all*.
pub fn repair_card(g: &Garage, b: &wallet::Bike, balance: i64) -> Vec<String> {
    let damaged = b.damaged();
    if damaged.is_empty() {
        return Vec::new();
    }
    let mut d = ocw_sim::damage::Damage::fresh();
    d.parts = b.conditions();
    let mut out = vec![format!(
        "{}: {} {:.0} % ({})",
        b.name(),
        d.bike_level().status().icon(),
        d.bike() * 100.0,
        d.bike_level().label()
    )];
    for (p, _) in &damaged {
        out.push(picker::repair_line(cat(), g, b, *p));
    }
    let total: u32 = damaged
        .iter()
        .map(|(p, _)| wallet::repair_quote(cat(), g, b, *p))
        .sum();
    out.push(if i64::from(total) <= balance {
        format!(
            "Repair all: {} – at a part's hotspot, or workshop.repair all",
            hearts(i64::from(total))
        )
    } else {
        format!(
            "Repair all: {} ({} more needed)",
            hearts(i64::from(total)),
            hearts(i64::from(total) - balance)
        )
    });
    out
}

/// The rider's injury at the mirror (W6, Phase 12 Q6): the level and the
/// time to heal (real time; easy riding heals 1 % a minute).
pub fn injury_card(rider: f64) -> Vec<String> {
    if rider >= 1.0 {
        return Vec::new();
    }
    let level = ocw_sim::damage::Injury::of(rider);
    vec![
        format!(
            "Rider: {} {} ({:.0} %)",
            level.status().icon(),
            level.label(),
            rider * 100.0
        ),
        format!(
            "Heals in {} – or 1 % a minute of easy riding",
            ocw_sim::damage::heal_text(ocw_sim::damage::hours_to_heal(rider))
        ),
    ]
}

fn picker_view(
    p: &picker::Picker,
    g: &Garage,
    balance: i64,
    scene: &Workshop,
    server: Option<&ocw_protocol::BuildInfo>,
    character: &ocw_core::Character,
    weight_kg: f64,
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
            let (left, right, locked) = match (&r.kind, p.design) {
                (picker::Kind::Design(f), Some((_, draft))) => (
                    if p.slot.is_kit() {
                        format!("Colour of the {}", f.name().to_lowercase())
                    } else {
                        f.name().to_string()
                    },
                    format!("‹ {} ›", f.value(&draft)),
                    false,
                ),
                _ => bench::row_text(cat(), g, r, balance)
                    .unwrap_or_else(|| picker::row_text(cat(), r, balance)),
            };
            PickerRowView {
                group,
                left,
                right,
                locked,
                ..Default::default()
            }
        })
        .collect();
    let equipped = if p.slot.is_kit() {
        g.kit_on(cat(), p.slot)
    } else {
        bike.and_then(|b| g.part_on(cat(), b, p.slot))
    };
    let focused = p.preview().and_then(|id| cat().get(id));
    let detail = match (p.slot, focused) {
        _ if p.bench.is_some() => bench::card(
            cat(),
            g,
            bike,
            p.bench.unwrap_or(bench::BenchPage::Top),
            p.focused(),
        ),
        // On the swatch row of a kit picker (4N Q30 (a)): the colour.
        _ if p.on_swatches() => p.swatches.as_ref().map_or_else(Vec::new, |s| {
            vec![
                format!("{}: {}", s.field.name(), s.name(s.focus.swatch)),
                s.field.help().to_string(),
                "Free – ← / → choose (live in the mirror), Confirm applies; everyone sees it; ↓ the items."
                    .into(),
            ]
        }),
        _ if p.design_field().is_some() => {
            let draft = p.design.map(|(_, l)| l).unwrap_or_default();
            p.design_field().map_or_else(Vec::new, |f| {
                vec![
                    format!("{}: {}", f.name(), f.value(&draft)),
                    f.help().to_string(),
                    "Free – ← / → change it (live in the mirror), Confirm applies; everyone sees it."
                        .into(),
                ]
            })
        }
        _ if p.character.is_some() => {
            let f = p.preview_free();
            let mut d = vec![
                f.map_or_else(String::new, |f| f.name()),
                "Free – who you are never costs hearts, and never changes the physics.".into(),
            ];
            if matches!(
                p.character,
                Some(picker::CharSlot::Body | picker::CharSlot::Shape)
            ) {
                d.push(format!(
                    "Height {:.2} m: from the rider profile (Settings → Riders, riders.edit --height)",
                    f32::from(character.validated().height_cm) / 100.0
                ));
                if p.character == Some(picker::CharSlot::Shape) {
                    let proposal = ocw_core::character::weight_shape(
                        weight_kg,
                        Some(f64::from(character.validated().height_cm)),
                    );
                    d.push(format!(
                        "Match my weight: {} (what your weight and height propose; any shape is free)",
                        proposal.name()
                    ));
                }
            }
            if f.is_some_and(|f| matches!(f, picker::Free::Glasses(_)))
                && g.kit_on(cat(), Slot::Helmet)
                    .is_some_and(|h| h.model == "helmet-tt")
            {
                d.push("Under the time-trial helmet the visor replaces the glasses.".into());
            }
            d
        }
        (Slot::Paint, _) => vec![
            picker::colour_name(p.preview_paint().flatten()),
            "Paint is free (special designs come later).".into(),
        ],
        (_, Some(f)) => {
            let ph = bike.map(|b| picker::CardPhysics::of(cat(), g, b, f));
            let mut d = picker::detail(cat(), f, equipped, ph.as_ref());
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
    let title = match p.character {
        _ if p.bench.is_some() => p.bench.map(|b| b.title()).unwrap_or_default(),
        _ if p.is_designer() => format!(
            "{} – your kit",
            p.design.map_or("Jersey designer", |(page, _)| page.name())
        ),
        Some(c) => format!("{} – your rider", c.name()),
        None if p.slot.is_kit() => format!("{} – your kit", p.slot.name()),
        None => format!("{} – {}", p.slot.name(), bike.map_or("", |b| b.name())),
    };
    PickerView {
        title,
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
        tiles: None,
        subtitle: None,
        swatches: None,
    }
}

/// The picker as tiles (spec §14 Phase 4, 4B / 4G *Tiles*): each row's
/// thumbnail (the part's model on the bike's profile in its paint; a
/// paint row: the bike in that colour), its gain against the slot's base
/// part on this bike within the caps (flat and 6 % climb, or the race
/// mode's word that parts are cosmetic) and its weight, the grid scrolled
/// to the focus, and the whole bike against the stock bike under the
/// title.
fn fill_tiles(
    v: &mut PickerView,
    p: &picker::Picker,
    g: &Garage,
    scene: &Workshop,
    mode: ocw_sim::PhysicsMode,
    frame: ocw_core::Rgb,
) {
    use ocw_client::part_tiles::{self, COLUMNS, VISIBLE_ROWS};
    let Some(bike) = g.bike(&scene.bikes[scene.stand]) else {
        return;
    };
    let paint = bike.paint().unwrap_or(frame);
    for (row, r) in v.rows.iter_mut().zip(&p.rows) {
        row.group = None;
        match (&r.kind, r.part.as_deref().and_then(|id| cat().get(id))) {
            (picker::Kind::Paint(c), _) => {
                row.thumb = Some(ThumbKey::Part(PartThumb {
                    profile: bike.profile(),
                    slot: "paint".into(),
                    model: "stock".into(),
                    paint: c.unwrap_or(frame),
                }));
                let t = part_tiles::tile_text(&row.left, &row.right, false, None, mode);
                row.gain = t.gain;
            }
            (_, Some(part)) => {
                row.thumb = Some(ThumbKey::Part(PartThumb {
                    profile: bike.profile(),
                    slot: part.slot.id().into(),
                    model: part.model.clone(),
                    paint,
                }));
                // The tile's name without the weight and tier of the row.
                row.left = part.name.clone();
                let gain = picker::tile_gain(cat(), g, bike, part);
                let t =
                    part_tiles::tile_text(&row.left, &row.right, row.locked, gain.as_ref(), mode);
                row.gain = t.gain;
                row.weight = t.weight;
                row.capped = t.capped;
                row.right = format!("{} · {}", part.tier.name(), row.right);
            }
            _ => {}
        }
    }
    // The focused tile's numbers on the card too.
    if let Some(r) = v.rows.get(v.focus).filter(|r| !r.weight.is_empty()) {
        let line = format!("Against the base part: {} · {}", r.gain, r.weight);
        let at = v.detail.len().min(2);
        v.detail.insert(at, line);
    }
    // Under the tiles the renderer shows what fits of the card, in this
    // order, without the name (on the focused tile; bug 2026-11-17: the
    // card ran out at the panel's bottom).
    if !v.detail.is_empty() {
        v.detail.remove(0);
    }
    v.detail.sort_by_key(|l| picker::tile_card_rank(l));
    let len = v.rows.len();
    let first = part_tiles::first_row(v.focus, len, COLUMNS, VISIBLE_ROWS) * COLUMNS;
    v.tiles = Some(TilesView {
        cols: COLUMNS,
        first,
        shown: (VISIBLE_ROWS * COLUMNS).min(len.saturating_sub(first)),
    });
    if v.subtitle.is_none() && p.slot != Slot::Paint {
        let now = picker::build_with(cat(), g, bike, None);
        v.subtitle = Some(match part_tiles::cosmetic_now(mode) {
            Some(note) => format!("{} – parts: {note}", part_tiles::bike_gain_line(&now)),
            None => part_tiles::bike_gain_line(&now),
        });
    }
}

/// The mirror's pickers as tiles (spec §14 Phase 4, 4N; the same tile
/// model as the menu's editor): each row a rendering of the rider wearing
/// it – the face for the head's slots, the whole rider for the body's, the
/// hands for the gloves – keyed by the look with that row, the colours as
/// swatches; the focused tile previews on the rider at once, Confirm (or a
/// click) keeps it, Back restores the saved look.
fn fill_rider_tiles(v: &mut PickerView, p: &picker::Picker, scene: &Workshop, ctx: &Ctx) {
    use ocw_client::part_tiles::{self, COLUMNS};
    use ocw_render::thumbs::RiderThumb;
    let shot = scene.dress.slot().shot();
    // A kit picker with a swatch row shows its items in the selected
    // colour (4N Q30 (a)); the others the saved look.
    let look = p
        .swatches
        .as_ref()
        .and(scene.design_preview())
        .unwrap_or(ctx.look);
    for (row, r) in v.rows.iter_mut().zip(&p.rows) {
        match r.kind {
            picker::Kind::Free(picker::Free::Skin(i)) => {
                row.swatch = Some(
                    ocw_core::Character {
                        skin: i,
                        ..ctx.character
                    }
                    .skin_colour(),
                );
            }
            picker::Kind::Free(picker::Free::HairColour(c)) => row.swatch = Some(c),
            picker::Kind::Design(_) => {}
            _ => {
                let (character, mut kit) = scene.rider_look_with(ctx, Some(r));
                if scene.helmet_off() {
                    kit.push(ocw_assets::rider::HELMET_OFF.to_string());
                }
                row.thumb = Some(ThumbKey::Rider(RiderThumb {
                    look: ocw_core::RiderLook { character, ..look },
                    kit,
                    shot,
                }));
            }
        }
    }
    let len = v.rows.len();
    let first = part_tiles::first_row(v.focus, len, COLUMNS, RIDER_TILE_ROWS) * COLUMNS;
    v.tiles = Some(TilesView {
        cols: COLUMNS,
        first,
        shown: (RIDER_TILE_ROWS * COLUMNS).min(len.saturating_sub(first)),
    });
    v.swatches = p.swatches.as_ref().map(swatch_row_view);
}

/// A kit picker's swatch row as shown (spec §14 Phase 4, 4N Q30 (a)):
/// every colour (the standard one as the model's own), the selected one
/// outlined, the label naming it.
pub fn swatch_row_view(s: &picker::Swatches) -> ocw_render::workshop::SwatchRowView {
    ocw_render::workshop::SwatchRowView {
        label: format!(
            "Colour of the {}: {}",
            s.field.name().to_lowercase(),
            s.name(s.focus.swatch)
        ),
        colours: (0..s.colours.len()).map(|i| s.shown(i)).collect(),
        selected: s.focus.swatch,
        focused: s.focus.on_swatches,
        standard: s.colours.iter().position(Option::is_none),
    }
}

/// Rows of the rider's tiles shown at once (square pictures).
pub const RIDER_TILE_ROWS: usize = 2;

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
