//! The in-game menu's Bevy glue, the same for the desktop and the browser
//! (spec §3.2.1; §14 Phase 10, B6: one menu). Esc / Start opens it over
//! the running ride. The ride never pauses: the trainer, the ride time,
//! the recording and a video keep going; while the menu is open the
//! steering falls back to autodrive and junctions take their default exit
//! (`ClientCore::hold_steering`), restored on closing.
//!
//! Input (keys, gamepads – on wasm32 the browser's Gamepad API through
//! Bevy's gilrs –, mouse and touch through `ocw_render::menu::MenuPointer`)
//! → the Bevy-free model (`ocw_client::menu::model`) → [`MenuView`] (drawn
//! by `ocw_render::menu`); menu functions run as console command lines.
//!
//! The rows of the shared pages (settings, audio, voice, time, weather,
//! the rider list) come from the shared resources; the app's own pages
//! (the desktop's world catalog, routes, rides, rider profiles, the
//! browser's world and workouts) from its [`MenuHooks`].

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::menu::model::{Action, Effect, Menu, Page};
use ocw_client::menu::pages::{self as sub, AudioDevices, Row, WorldHome};
use ocw_client::riders::Riders;
use ocw_client::settings::{Settings, applies};
use ocw_client::voice::VoiceMenuInfo;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::{InputContext, MenuCorePlugin, MenuPointer, MenuPointerEvent, MenuView};

use crate::controls::{self, NavRepeat};
use crate::riders_page::RiderList;
use crate::settings::UserSettings;
use crate::timemachine::TimeMachine;
use crate::weather::WeatherMenu;
use crate::{Platform, UiPlatform};

/// The voice call's state for the Voice pages (refreshed by the app's
/// voice glue).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct VoiceMenu(pub VoiceMenuInfo);

/// The sign-in state for the Account page (spec §9.1; refreshed by the
/// app's sign-in glue).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct AccountMenu(pub ocw_client::account::AccountInfo);

/// How the rider rides now – alone, on another server, hosting – for the
/// Ride page's items (set by the app's connection glue).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Deref, DerefMut)]
pub struct MenuRideMode(pub ocw_client::menu::model::RideMode);

/// The audio outputs for the Audio page (refreshed by the app's audio
/// glue).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct AudioDeviceMenu(pub AudioDevices);

/// Settings → Trainer → Devices (spec §3.2.1 *Devices*): the trainers and
/// sensors as the app's device layer sees them (the desktop's Bluetooth
/// scan, the browser's Web Bluetooth devices), written by the app.
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct DevicesMenu(pub ocw_client::menu::devices::DevicesInfo);

/// What an app adds to the shared menu: its own pages.
#[derive(Resource, Clone, Copy)]
pub struct MenuHooks {
    /// The rows of the app's own pages (the desktop's world catalog,
    /// routes, rides, GPX, workouts; the browser's workouts); `None`: a
    /// shared page.
    pub rows: fn(&World, sub::Page) -> Option<Vec<Row>>,
    /// Text of a page beyond its fixed description (a data source's notice).
    pub text: fn(&World, Page) -> Vec<String>,
    /// The rider profiles (the Riders, Rider, Bike and Workshop pages).
    pub riders: for<'w> fn(&'w World) -> Option<&'w Riders>,
    /// Esc while the menu is closed: `true` if the app used it (after
    /// god mode's captured pointer, `crate::god::free_pointer`).
    pub escape: fn(&mut World) -> bool,
}

impl Default for MenuHooks {
    fn default() -> Self {
        Self {
            rows: |_, _| None,
            text: |_, _| Vec::new(),
            riders: |_| None,
            escape: |_| false,
        }
    }
}

/// The browser's own pages (and a default for tests): the world of the
/// page, the bundled workouts.
pub fn world_home(name: impl Into<String>) -> WorldHome {
    WorldHome {
        name: name.into(),
        ..WorldHome::default()
    }
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuCorePlugin>() {
            app.add_plugins(MenuCorePlugin);
        }
        crate::configure_sets(app);
        // Rebinding, accessibility and the training history (spec §3.2.1
        // M9, §14 Phase 7): pages of this menu.
        if !app.is_plugin_added::<crate::bindings::BindingsPlugin>() {
            app.add_plugins(crate::bindings::BindingsPlugin);
        }
        if !app.is_plugin_added::<crate::access::AccessPlugin>() {
            app.add_plugins(crate::access::AccessPlugin);
        }
        if !app.is_plugin_added::<crate::history::HistoryPlugin>() {
            app.add_plugins(crate::history::HistoryPlugin);
        }
        // The trust dialog and the trusted world keys (spec §4.7).
        if !app.is_plugin_added::<crate::trust::TrustPlugin>() {
            app.add_plugins(crate::trust::TrustPlugin);
        }
        // Ride → Join (spec §3.2.1, §6.2 *Join links*).
        if !app.is_plugin_added::<crate::join::JoinPlugin>() {
            app.add_plugins(crate::join::JoinPlugin);
        }
        // Workshop → Character: the side panel's live rider (spec §14
        // Phase 4, 4N *Visual editor*).
        if !app.is_plugin_added::<crate::character::CharacterEditorPlugin>() {
            app.add_plugins(crate::character::CharacterEditorPlugin);
        }
        app.init_resource::<MenuState>()
            .init_resource::<VoiceMenu>()
            .init_resource::<AccountMenu>()
            .init_resource::<AudioDeviceMenu>()
            .init_resource::<DevicesMenu>()
            .init_resource::<MenuRideMode>()
            .add_systems(
                Update,
                (menu_input, crate::calendar::prepare, write_view)
                    .chain()
                    .after(crate::ViewExtrasSet),
            )
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("menu")
                    .about("Open, close or drive the in-game menu (Esc / gamepad Start)")
                    .arg(
                        Arg::new("action")
                            .value_parser([
                                "open",
                                "close",
                                "toggle",
                                "up",
                                "down",
                                "left",
                                "right",
                                "select",
                                "back",
                                "tab-next",
                                "tab-prev",
                                "workshop-page",
                                "ride",
                                "strava-waiting",
                                "strava",
                                "voice",
                                "season",
                                "bridge",
                            ])
                            .default_value("toggle"),
                    )
                    .arg(
                        Arg::new("n")
                            .value_parser(clap::value_parser!(usize))
                            .help("ride: the training history's ride (1 = the newest)"),
                    ),
                |world, m| {
                    let mut state = world.resource_mut::<MenuState>();
                    let open = state.menu.is_open();
                    let action = match m.get_one::<String>("action").map(String::as_str) {
                        Some("open") => Action::Open,
                        Some("close") => Action::Close,
                        Some("up") => Action::Up,
                        Some("down") => Action::Down,
                        Some("left") => Action::Left,
                        Some("right") => Action::Right,
                        Some("tab-next") => Action::TabNext,
                        Some("tab-prev") => Action::TabPrev,
                        Some("select") => Action::Confirm,
                        Some("back") => Action::Back,
                        Some("workshop-page") => Action::RiderPage,
                        // A ride's page of the training history.
                        Some("ride") => Action::OpenSub(sub::Page::HistoryRide(
                            m.get_one::<usize>("n").copied().unwrap_or(1).max(1) - 1,
                        )),
                        // The question about Strava's waiting recordings.
                        Some("strava-waiting") => Action::OpenSub(sub::Page::StravaWaiting),
                        // The Strava page (Navi → Strava; spec 2.8.1).
                        Some("strava") => Action::OpenSub(sub::Page::Strava),
                        // Settings → Voice (spec 7.1.7 Q16; scenes, tests).
                        Some("voice") => Action::OpenSub(sub::Page::Voice),
                        // World → Time → Season (spec 14 Phase 9 Q13; scenes).
                        Some("season") => Action::OpenSub(sub::Page::Season),
                        // Devices → the sensor bridge (spec 14 Phase 10,
                        // 10D.2; scenes, the bridge test).
                        Some("bridge") => Action::OpenSub(sub::Page::Bridge),
                        _ if open => Action::Close,
                        _ => Action::Open,
                    };
                    state.queued.push(action);
                    Ok(String::new())
                },
            ));
    }
}

/// The menu model and its input state.
#[derive(Resource, Default)]
pub struct MenuState {
    pub menu: Menu,
    repeat: NavRepeat,
    /// Actions from the console command `menu`, applied next frame.
    queued: Vec<Action>,
}

impl MenuState {
    /// Applies `action` next frame, as if it came from the input (the
    /// steering is held and released as the menu opens and closes).
    pub fn queue(&mut self, action: Action) {
        self.queued.push(action);
    }
}

/// The settings to show (the defaults without the settings plugin).
fn settings(world: &World) -> &Settings {
    static DEFAULT: std::sync::OnceLock<Settings> = std::sync::OnceLock::new();
    world
        .get_resource::<UserSettings>()
        .map_or_else(|| DEFAULT.get_or_init(Settings::default), |s| &s.settings)
}

fn platform(world: &World) -> Platform {
    world
        .get_resource::<UiPlatform>()
        .map_or(Platform::Desktop, |p| p.0)
}

/// The setting a row changes (`set KEY …` on ← / → / Confirm).
pub fn row_setting(row: &Row) -> Option<&str> {
    let line = row
        .left
        .as_deref()
        .or(row.right.as_deref())
        .or(match &row.confirm {
            Some(sub::Activate::Command(l)) => Some(l.as_str()),
            _ => None,
        })?;
    let rest = line.strip_prefix("set ")?;
    rest.split_whitespace().next()
}

/// Leaves out the rows of settings and pages this platform does not have.
pub fn for_platform(rows: Vec<Row>, platform: Platform) -> Vec<Row> {
    rows.into_iter()
        .filter(|r| row_setting(r).is_none_or(|k| applies(k, platform)))
        .filter(|r| !matches!(r.confirm, Some(sub::Activate::Open(p)) if !p.on(platform)))
        .collect()
}

/// The rows of a sub page: the shared pages from the shared resources,
/// the app's own from its hooks.
pub fn sub_rows(world: &World, page: sub::Page) -> Vec<Row> {
    let s = settings(world);
    let hooks = world
        .get_resource::<MenuHooks>()
        .copied()
        .unwrap_or_default();
    let rows = if page.is_history_page() {
        ocw_client::menu::history::rows(page, &crate::history::info(world))
    } else if page.is_binding_page() {
        sub::binding_rows(page, s, platform(world) == Platform::Desktop)
    } else if page == sub::Page::RiderList {
        world
            .get_resource::<RiderList>()
            .map(|l| l.menu_rows())
            .unwrap_or_default()
    } else if page == sub::Page::Account {
        let empty = AccountMenu::default();
        let info = world.get_resource::<AccountMenu>().unwrap_or(&empty);
        let mut rows = ocw_client::account::rows(info, platform(world) == Platform::Desktop);
        // The world keys this rider trusts (spec §4.7 *User keys*).
        let n = world
            .get_resource::<crate::trust::TrustedKeys>()
            .map_or(0, |k| k.keys.keys.len());
        rows.push(sub::open_row(
            "Trusted world keys",
            Some(n.to_string()),
            sub::Page::TrustedKeys,
        ));
        rows
    } else if matches!(page, sub::Page::TrustedKeys | sub::Page::TrustedKey(_)) {
        let empty = crate::trust::TrustedKeys::default();
        let k = world
            .get_resource::<crate::trust::TrustedKeys>()
            .unwrap_or(&empty);
        ocw_client::menu::trust::rows(page, &k.keys.keys, k.warning.as_deref())
    } else if page == sub::Page::Join {
        world
            .get_resource::<crate::join::JoinUi>()
            .map(|u| ocw_client::menu::join::rows(page, &u.info))
            .unwrap_or_default()
    } else if page == sub::Page::ViewDistance {
        // The preset's distance and what runs (spec §14 Phase 10, 10G).
        ocw_client::view_distance::rows(s, &crate::view_distance::info(world))
    } else if page == sub::Page::TimeMachine {
        let info = world
            .get_resource::<TimeMachine>()
            .map(|t| t.0.clone())
            .unwrap_or_default();
        ocw_client::menu::time::rows(&info, calendar_info(world))
    } else if let sub::Page::Calendar(c) = page {
        ocw_client::menu::calendar::rows(&c, calendar_info(world))
    } else if let sub::Page::CalendarDates(c) = page {
        ocw_client::menu::calendar::dates_rows(&c, calendar_info(world))
    } else if page == sub::Page::Season {
        ocw_client::menu::calendar::season_rows(calendar_info(world))
    } else if page.is_weather_page() {
        let w = world
            .get_resource::<WeatherMenu>()
            .map(|w| w.0.clone())
            .unwrap_or_default();
        ocw_client::menu::weather::rows(page, &w)
    } else if page.is_voice_page() {
        let empty = VoiceMenuInfo::default();
        let info = world.get_resource::<VoiceMenu>().map_or(&empty, |v| &v.0);
        ocw_client::menu::voice::rows(page, s, info)
    } else if matches!(
        page,
        sub::Page::Devices | sub::Page::Device(_) | sub::Page::Bridge
    ) {
        let empty = DevicesMenu::default();
        let info = world.get_resource::<DevicesMenu>().unwrap_or(&empty);
        ocw_client::menu::devices::rows(page, info)
    } else if page == sub::Page::Audio {
        let empty = AudioDevices::default();
        let devices = world
            .get_resource::<AudioDeviceMenu>()
            .map_or(&empty, |d| &d.0);
        sub::audio_rows(s, devices)
    } else if let Some(rows) = (hooks.rows)(world, page) {
        rows
    } else {
        let default = Riders::default();
        let riders = (hooks.riders)(world).unwrap_or(&default);
        let mut rows = sub::rows(page, s, riders);
        // The Trainer page's Devices row shows what is connected.
        if page == sub::Page::Trainer
            && let Some(info) = world.get_resource::<DevicesMenu>()
            && let Some(r) = rows
                .iter_mut()
                .find(|r| r.confirm == Some(sub::Activate::Open(sub::Page::Devices)))
        {
            r.value = Some(info.trainer_value());
        }
        rows
    };
    for_platform(rows, platform(world))
}

/// The calendar's knowledge of the clock and the place (empty without the
/// time machine's plugin).
fn calendar_info(world: &World) -> &ocw_client::menu::calendar::CalendarInfo {
    static EMPTY: std::sync::OnceLock<ocw_client::menu::calendar::CalendarInfo> =
        std::sync::OnceLock::new();
    world
        .get_resource::<crate::calendar::CalendarMenu>()
        .map_or_else(|| EMPTY.get_or_init(Default::default), |c| &c.0)
}

/// Text of a page beyond its fixed description.
fn page_text(world: &World, page: Page) -> Vec<String> {
    if page == Page::Sub(sub::Page::Join) {
        return world
            .get_resource::<crate::join::JoinUi>()
            .map(|u| ocw_client::menu::join::text(&u.info, crate::now(world)))
            .unwrap_or_default();
    }
    // Where the seasons are weak, the Season page says so (spec §14 Phase
    // 9 Q13).
    if page == Page::Sub(sub::Page::Season) {
        return ocw_client::menu::calendar::season_note(calendar_info(world))
            .into_iter()
            .collect();
    }
    world
        .get_resource::<MenuHooks>()
        .map_or_else(Vec::new, |h| (h.text)(world, page))
}

fn menu_input(world: &mut World) {
    let platform = platform(world);
    {
        let mut state = world.resource_mut::<MenuState>();
        if state.menu.platform() != platform && !state.menu.is_open() {
            state.menu = Menu::for_platform(platform);
        }
    }
    let mode = world
        .get_resource::<MenuRideMode>()
        .map(|m| m.0)
        .unwrap_or_default();
    world.resource_mut::<MenuState>().menu.set_ride_mode(mode);
    // A text field of Ride → Join is typed into: this frame's keys are its.
    if crate::join::capture(world) {
        world.resource_mut::<MenuState>().repeat.reset();
        world.resource_mut::<MenuPointer>().0.clear();
        return;
    }
    // A binding row waits for its new key or button (spec §3.2.1 M9):
    // this frame's input is that, not the menu's.
    if crate::bindings::capture(world) {
        world.resource_mut::<MenuState>().repeat.reset();
        world.resource_mut::<MenuPointer>().0.clear();
        return;
    }
    let swap = settings(world)
        .get("controls.pad_confirm")
        .is_ok_and(|v| v.to_string() == "east");
    let mut pressed = {
        let keys = world.get_resource::<ButtonInput<KeyCode>>().cloned();
        let mut q = world.query::<&Gamepad>();
        controls::pressed(keys.as_ref(), q.iter(world), swap)
    };
    let console_open = world.resource::<ConsoleState>().is_open();
    let open = world.resource::<MenuState>().menu.is_open();
    // God mode (spec §14 Phase 13A, both apps): Esc first frees the
    // captured pointer; the next Esc opens the menu. Then the app's own
    // use of Esc, if any.
    if pressed.escape && !open && !console_open {
        let escape = world
            .get_resource::<MenuHooks>()
            .map(|h| h.escape)
            .unwrap_or(|_| false);
        if crate::god::free_pointer(world) || escape(world) {
            pressed.escape = false;
        }
    }
    let now = crate::now(world);
    let dt = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.delta_secs_f64());
    let context = *world.resource::<InputContext>();
    let moved = world
        .get_resource::<AccumulatedMouseMotion>()
        .is_some_and(|m| m.delta != Vec2::ZERO);
    let pointer_events = std::mem::take(&mut world.resource_mut::<MenuPointer>().0);
    let calendar_open = matches!(
        world.resource::<MenuState>().menu.page(),
        Some(Page::Sub(sub::Page::Calendar(_)))
    );
    let mut actions = std::mem::take(&mut world.resource_mut::<MenuState>().queued);
    let results =
        !open && !console_open && crate::client(world).is_some_and(|c| c.results_showing(now));
    if results && (pressed.escape || pressed.confirm || pressed.back) {
        // An event's results panel is up: Esc / Enter / B close it first
        // (spec §2.3), the menu opens on the next press.
        if let Some(mut c) = crate::client_mut(world) {
            c.dismiss_results(now);
        }
        world.resource_mut::<MenuState>().repeat.reset();
    } else if console_open {
        // The console has the keys; Esc closes it first (never the app).
        if pressed.escape {
            world.resource_mut::<ConsoleState>().set_open(false);
        }
        world.resource_mut::<MenuState>().repeat.reset();
    } else if (!open && context != InputContext::Workshop) || context == InputContext::Menu {
        // Navigation starts once the frame began in the menu's context:
        // the key that opened the menu does not also move its focus.
        let step = if open {
            world
                .resource_mut::<MenuState>()
                .repeat
                .update(pressed.held, pressed.tapped, dt)
                + pressed.step
        } else {
            0
        };
        actions.extend(controls::actions(&pressed, step, open));
        // Hover only when the mouse moved: rows appearing under a resting
        // pointer (a new page) must not take the focus.
        actions.extend(pointer_events.into_iter().filter_map(|e| match e {
            MenuPointerEvent::Hover(i) => moved.then_some(Action::Hover(i)),
            MenuPointerEvent::Click(i) => Some(Action::Click(i)),
            MenuPointerEvent::Step(i, dir) => Some(Action::Step(i, dir)),
            MenuPointerEvent::Back => Some(Action::Back),
            // A swipe over the calendar: the next / previous month.
            MenuPointerEvent::Swipe(dir) => calendar_open.then_some(if dir > 0 {
                Action::TabNext
            } else {
                Action::TabPrev
            }),
        }));
    }
    let mut lines = Vec::new();
    world.resource_scope(|world, mut state: Mut<MenuState>| {
        let rows = |p| sub_rows(world, p);
        for action in actions {
            if let Effect::Command(line) = state.menu.apply_with(action, rows) {
                lines.push(line);
            }
        }
    });
    {
        let mut console = world.resource_mut::<ConsoleState>();
        for line in lines {
            console.queue(line);
        }
    }
    let now_open = world.resource::<MenuState>().menu.is_open();
    if now_open != open {
        if !now_open {
            world.resource_mut::<MenuState>().repeat.reset();
        }
        info!("menu {}", if now_open { "opened" } else { "closed" });
        if let Some(mut c) = crate::client_mut(world) {
            if now_open {
                c.hold_steering();
            } else {
                c.release_steering();
            }
        }
    }
}

/// The view of the menu as it is now.
pub fn view_of(world: &World, menu: &Menu) -> MenuView {
    let Some(page) = menu.page() else {
        return MenuView::default();
    };
    let def = page.def();
    let weather = world.get_resource::<WeatherMenu>();
    let rider_list = world.get_resource::<RiderList>();
    let history = match page {
        Page::Sub(p) if p.is_history_page() => Some(crate::history::info(world)),
        Page::ConfirmDelete => Some(crate::history::info(world)),
        _ => None,
    };
    // The ride a delete asks about (spec 14 Phase 7, 7A).
    let deleting = (page == Page::ConfirmDelete)
        .then(|| {
            menu.pending()
                .and_then(ocw_client::menu::history::delete_index)
        })
        .flatten();
    let history_text = match (page, &history) {
        (Page::Sub(p), Some(h)) => ocw_client::menu::history::text(p, h),
        (Page::ConfirmDelete, Some(h)) => deleting
            .map(|k| ocw_client::menu::history::delete_text(k, h))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let (trust_status, trust_text, emphasis) = trust_view(world, menu, page);
    let mut view = MenuView {
        open: true,
        // The trust dialog's warning, world and data sources all fit.
        wide: page == Page::ConfirmTrust,
        title: def.title.into(),
        emphasis,
        status: match (page, world.get_resource::<RideView>(), weather) {
            _ if !trust_status.is_empty() => trust_status,
            (Page::Top, Some(r), _) => ocw_client::menu::status_line(r),
            // The weather now and who sets it (spec §14 Phase 9, S5).
            (Page::Sub(p), _, Some(w)) if p.is_weather_page() => {
                ocw_client::menu::weather::status(p, w, menu.focus())
            }
            (Page::Sub(sub::Page::RiderList), _, _) => {
                rider_list.map(|l| l.summary()).unwrap_or_default()
            }
            (Page::Sub(sub::Page::Account), _, _) => world
                .get_resource::<AccountMenu>()
                .map(|a| a.status())
                .unwrap_or_default(),
            (Page::ConfirmDelete, _, _) => match (deleting, &history) {
                (Some(k), Some(h)) => {
                    ocw_client::menu::history::status(sub::Page::HistoryRide(k), h)
                }
                _ => String::new(),
            },
            (Page::Sub(p), _, _) if history.is_some() => {
                ocw_client::menu::history::status(p, history.as_ref().expect("some"))
            }
            (Page::Sub(p @ (sub::Page::Devices | sub::Page::Device(_) | sub::Page::Bridge)), _, _) => world
                .get_resource::<DevicesMenu>()
                .map(|d| ocw_client::menu::devices::status(p, d))
                .unwrap_or_default(),
            (Page::Sub(sub::Page::BindGroup(g)), _, _) => crate::bindings::status(world, Some(g)),
            // Who sets the time; the calendar's focused day (spec §14
            // Phase 9 S9).
            (Page::Sub(sub::Page::TimeMachine), _, _) => world
                .get_resource::<TimeMachine>()
                .map(|t| ocw_client::menu::time::status(t))
                .unwrap_or_default(),
            (Page::Sub(sub::Page::Calendar(c)), _, _) => {
                ocw_client::menu::calendar::status(&c, calendar_info(world), menu.focus())
            }
            (Page::Sub(sub::Page::Bindings), _, _) => crate::bindings::status(world, None),
            // The focused tile's name under pictures without names (4N).
            (Page::Sub(sub::Page::CharacterPick(a)), _, _) => {
                let hooks = world
                    .get_resource::<MenuHooks>()
                    .copied()
                    .unwrap_or_default();
                let rider = (hooks.riders)(world)
                    .map_or_else(ocw_client::riders::Rider::default, Riders::active);
                ocw_client::menu::character::status(a, &rider, menu.focus())
            }
            _ => String::new(),
        },
        text: def
            .text
            .iter()
            .map(|&s| s.to_owned())
            .chain(page_text(world, page))
            // The device flow's code and URL, the last sign-in message.
            .chain(match page {
                Page::Sub(sub::Page::Account) => world
                    .get_resource::<AccountMenu>()
                    .map(|a| a.text())
                    .unwrap_or_default(),
                _ => Vec::new(),
            })
            .chain(history_text)
            .chain(trust_text)
            .collect(),
        items: menu.labels(),
        focused: menu.focus(),
        hint: ocw_client::menu::hint(page, menu.list_pages(), menu.depth()),
        dimmed: menu.dimmed(),
        grid: match page {
            Page::Sub(p @ sub::Page::WeatherPresets) if p.grid_columns().is_some() => {
                let clear = WeatherMenu::default();
                Some(ocw_client::menu::weather::grid(weather.unwrap_or(&clear)))
            }
            // The character editor's tiles (spec §14 Phase 4, 4N).
            Page::Sub(sub::Page::CharacterPick(a)) => {
                let hooks = world
                    .get_resource::<MenuHooks>()
                    .copied()
                    .unwrap_or_default();
                let rider = (hooks.riders)(world)
                    .map_or_else(ocw_client::riders::Rider::default, Riders::active);
                // The pictures show the rider's own kit colours and the
                // kit it wears (spec §14 Phase 4, 4N).
                let local = world
                    .get_resource::<RideView>()
                    .and_then(|v| v.riders.iter().find(|r| r.is_local));
                let look = local.and_then(|r| r.look).unwrap_or_default();
                let kit = local
                    .map(|r| ocw_render::riders::kit_variants(&r.parts))
                    .unwrap_or_default();
                Some(ocw_client::menu::character::grid_for(
                    a, &rider, &look, &kit,
                ))
            }
            _ => None,
        },
        // The character editor: the panel at the side, the rider beside it.
        side: matches!(page, Page::Sub(p) if ocw_client::menu::character::is_editor_page(p)),
        // The rider list's unicorns and damage dots (13E, 12F); the
        // warning dot on a binding in conflict (M9).
        marks: match (page, rider_list) {
            (Page::Sub(sub::Page::RiderList), Some(l)) => menu
                .shown_rows()
                .into_iter()
                .map(|r| r.and_then(|i| l.mark(i)).unwrap_or_default())
                .collect(),
            (Page::Sub(sub::Page::BindGroup(g)), _) => {
                let actions = crate::bindings::group_actions(world, g);
                let warn = ocw_sim::damage::Status::Warning;
                menu.shown_rows()
                    .into_iter()
                    .map(|r| ocw_render::menu::RowMark {
                        unicorn: false,
                        dot: r
                            .and_then(|i| actions.get(i))
                            .filter(|a| crate::bindings::in_conflict(world, **a))
                            .map(|_| (warn.icon().to_owned(), warn.rgb())),
                    })
                    .collect()
            }
            _ => Vec::new(),
        },
        // The power curve (spec §14 Phase 7).
        chart: match (page, &history) {
            (Page::Sub(p), Some(h)) => ocw_client::menu::history::chart(p, h),
            _ => None,
        },
        // The ride's track beside it (spec 14 Phase 7, 7A).
        map: match (page, &history) {
            (Page::Sub(p), Some(h)) => ocw_client::menu::history::map(p, h),
            _ => None,
        },
        // The month grid (spec §14 Phase 9 S9a).
        calendar: match page {
            Page::Sub(p @ sub::Page::Calendar(c)) => Some(ocw_client::menu::calendar::view(
                &c,
                calendar_info(world),
                &sub_rows(world, p),
            )),
            _ => None,
        },
    };
    // The calendar's title is its month.
    if let Some(c) = &view.calendar {
        view.title = c.title.clone();
    }
    view
}

/// The trust pages' status, text and large fingerprint (spec §4.7).
fn trust_view(world: &World, menu: &Menu, page: Page) -> (String, Vec<String>, Vec<String>) {
    use ocw_client::menu::trust as t;
    let empty = crate::trust::TrustedKeys::default();
    let keys = world
        .get_resource::<crate::trust::TrustedKeys>()
        .unwrap_or(&empty);
    match page {
        Page::ConfirmTrust => match world
            .get_resource::<crate::trust::TrustDialog>()
            .and_then(|d| d.request.as_ref().map(|r| (r, d.shared)))
        {
            Some((r, shared)) => (
                t::dialog_status(r),
                t::dialog_text(r, shared),
                t::dialog_emphasis(r),
            ),
            None => (
                String::new(),
                vec!["No world is waiting for a decision.".into()],
                Vec::new(),
            ),
        },
        Page::ConfirmForget => match t::pending_key(menu.pending(), &keys.keys.keys) {
            Some(k) => (
                format!("'{}' (unverified name)", k.name),
                Vec::new(),
                k.fingerprint().lines(t::GROUPS_PER_LINE),
            ),
            None => Default::default(),
        },
        Page::Sub(p @ ocw_client::menu::pages::Page::TrustedKey(_)) => (
            String::new(),
            Vec::new(),
            t::key_emphasis(p, &keys.keys.keys),
        ),
        _ => Default::default(),
    }
}

fn write_view(world: &mut World) {
    // Values changed by the menu's commands (or the console) show at once.
    let new = world.resource_scope(|world, mut state: Mut<MenuState>| {
        state.menu.refresh(&|p| sub_rows(world, p));
        view_of(world, &state.menu)
    });
    world.resource_mut::<MenuView>().set_if_neq(new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;

    /// The menu alone over a stepped local server, as `platform`.
    fn app(platform: Platform) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(UiPlatform(platform))
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .add_plugins((
                crate::settings::SettingsPlugin,
                crate::weather::WeatherPlugin,
                crate::timemachine::TimeMachinePlugin,
                crate::riders_page::RidersPagePlugin,
                MenuPlugin,
            ));
        app.update();
        app
    }

    fn tap(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
        app.update();
    }

    fn view(app: &App) -> MenuView {
        app.world().resource::<MenuView>().clone()
    }

    /// Workshop → Character (spec §14 Phase 4, 4N N10): the same pages and
    /// rows in both apps (one model, B6), each change a `character` line.
    #[test]
    fn the_character_pages_are_the_same_in_both_apps() {
        let desktop = app(Platform::Desktop);
        let browser = app(Platform::Browser);
        let workshop = sub_rows(browser.world(), sub::Page::Workshop);
        assert!(
            workshop
                .iter()
                .any(|r| r.confirm == Some(sub::Activate::Open(sub::Page::Character))),
            "the browser's Workshop has the Character row"
        );
        for page in std::iter::once(sub::Page::Character).chain(ocw_client::menu::character::PAGES)
        {
            let d = sub_rows(desktop.world(), page);
            let b = sub_rows(browser.world(), page);
            assert!(!d.is_empty(), "{page:?}");
            assert_eq!(d, b, "{page:?}");
            for r in &d {
                for line in [&r.left, &r.right].into_iter().flatten() {
                    assert!(line.starts_with("character "), "{page:?}: {line}");
                }
            }
        }
    }

    /// The character editor's live view (spec §14 Phase 4, 4N *Visual
    /// editor*), the same in both apps: the editor's pages dock the panel
    /// at the side; on a tile page the rider previews the focused tile at
    /// once (moved by the keys, the pad's arrows or the pointer's hover)
    /// and the camera frames the face; leaving puts the camera back and
    /// the saved character on the rider.
    #[test]
    fn the_character_editor_shows_the_rider_and_previews_the_focused_tile() {
        use ocw_client::menu::character::Attr;
        use ocw_render::camera::CameraOrbit;
        for platform in [Platform::Desktop, Platform::Browser] {
            let mut app = app(platform);
            app.insert_resource(CameraOrbit::default());
            app.init_resource::<ocw_render::camera::CloseUp>();
            let go = |app: &mut App, a: Action| {
                app.world_mut().resource_mut::<MenuState>().queue(a);
                app.update();
                app.update();
            };
            go(&mut app, Action::RiderPage);
            let workshop = sub_rows(app.world(), sub::Page::Workshop);
            let at = workshop
                .iter()
                .position(|r| r.confirm == Some(sub::Activate::Open(sub::Page::Character)))
                .expect("the Character row");
            go(&mut app, Action::Click(at));
            assert!(view(&app).side, "{platform:?}: the panel at the side");
            assert!(view(&app).open);
            go(&mut app, Action::Click(2)); // Hair
            go(&mut app, Action::Click(0)); // Hair style
            let v = view(&app);
            assert_eq!(v.title, "Hair style");
            let grid = v.grid.expect("tiles");
            assert_eq!(grid.cells.len(), 22, "21 styles and Back");
            assert!(grid.cells[0].marked, "the current style (short)");
            // The camera turned to the face.
            let orbit = *app.world().resource::<CameraOrbit>();
            assert_eq!(
                orbit,
                crate::character::editor_orbit(sub::Page::CharacterPick(Attr::HairStyle))
            );
            assert!(orbit.yaw > 2.0, "in front of the rider");
            assert_eq!(
                app.world().resource::<ocw_render::camera::CloseUp>().0,
                Some(ocw_render::camera::CloseUpShot::THREE_QUARTER),
                "{platform:?}: the head in a close-up"
            );
            assert!(
                crate::character::helmet_off(app.world()),
                "{platform:?}: the helmet off for the hair (4N Q28)"
            );
            // The focused tile is previewed, by the arrows and the pointer.
            go(&mut app, Action::Right);
            assert_eq!(
                crate::character::previewed(app.world()).map(|c| c.hair),
                Some(1)
            );
            go(&mut app, Action::Hover(16));
            assert_eq!(
                crate::character::previewed(app.world()).map(|c| c.hair),
                Some(16)
            );
            // Back to the category, then out: no preview, the camera back.
            go(&mut app, Action::Back);
            assert_eq!(crate::character::previewed(app.world()), None);
            go(&mut app, Action::Close);
            assert!(!view(&app).side);
            assert_eq!(
                app.world().resource::<ocw_render::camera::CloseUp>().0,
                None
            );
            assert!(
                !crate::character::helmet_off(app.world()),
                "back on when it closes"
            );
            assert_eq!(
                *app.world().resource::<CameraOrbit>(),
                CameraOrbit::default(),
                "{platform:?}: the camera put back"
            );
        }
    }

    /// Settings → Trainer → Devices: the rows from the app's devices
    /// (spec §3.2.1 *Devices*), the Trainer page's row shows what is
    /// connected.
    #[test]
    fn the_devices_pages_come_from_the_apps_devices() {
        use ocw_client::menu::devices::{DevicesInfo, Entry, Link, Role, Scan};
        let mut app = app(Platform::Desktop);
        // Nothing yet: scan, "no devices".
        let rows = sub_rows(app.world(), sub::Page::Devices);
        assert_eq!(rows[0].label, "Scan for devices");
        assert_eq!(rows[1].label, "No devices yet");
        app.world_mut().resource_mut::<DevicesMenu>().0 = DevicesInfo {
            scan: Scan::Scanning { left_s: 4 },
            entries: vec![Entry {
                id: "hci0/dev_00_00_00_00_00_01".into(),
                name: "KICKR".into(),
                role: Role::Trainer,
                rssi: Some(-58),
                link: Link::Connected("Wahoo KICKR".into()),
                remembered: true,
            }],
            ..Default::default()
        };
        let rows = sub_rows(app.world(), sub::Page::Devices);
        assert_eq!(rows[0].label, "Stop the scan");
        assert_eq!(rows[1].label, "KICKR");
        assert_eq!(
            rows[1].confirm,
            Some(sub::Activate::Open(sub::Page::Device(0)))
        );
        let trainer = sub_rows(app.world(), sub::Page::Trainer);
        let d = trainer.iter().find(|r| r.label == "Devices").unwrap();
        assert_eq!(d.value.as_deref(), Some("KICKR"));
        assert!(!sub_rows(app.world(), sub::Page::Device(0)).is_empty());
        assert!(sub_rows(app.world(), sub::Page::Device(1)).is_empty());
    }

    #[test]
    fn the_browser_menu_has_no_quit_and_its_own_captures() {
        let mut app = app(Platform::Browser);
        tap(&mut app, KeyCode::Escape);
        let v = view(&app);
        assert!(v.open);
        assert_eq!(
            v.items,
            [
                "Resume", "Ride", "World", "Workshop", "Navi", "Settings", "Captures"
            ]
        );
        // Captures: screenshot, video, FIT, the folder, full screen.
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Captures");
        assert_eq!(v.items.len(), 6);
        assert!(v.items[2].contains("FIT"));
        assert_eq!(v.items[4], "Full screen");
        // Back; the workshop is locked (Q5).
        tap(&mut app, KeyCode::Escape);
        for _ in 0..3 {
            tap(&mut app, KeyCode::ArrowUp);
        }
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Workshop");
        assert!(v.text[0].contains("log in"), "{:?}", v.text);
        assert_eq!(v.items, ["Back"]);
    }

    /// The browser's own pages fit a 16:9 window with their text, like
    /// the desktop's (`menu::tests` in the desktop app).
    #[test]
    fn the_browser_pages_fit_a_16_9_window() {
        use ocw_render::menu::{PANEL_MAX_VH, fitted_paragraphs, page_height_vh};
        let app = app(Platform::Browser);
        for path in [&[][..], &[6], &[3], &[1], &[4], &[5]] {
            let mut m = Menu::for_platform(Platform::Browser);
            m.apply(Action::Open);
            for &i in path {
                m.apply(Action::Click(i));
            }
            let v = view_of(app.world(), &m);
            let h = page_height_vh(&v, 16.0 / 9.0, v.text.len());
            assert!(h <= PANEL_MAX_VH, "{}: {h:.1} vh", v.title);
            assert_eq!(
                fitted_paragraphs(&v, 16.0 / 9.0),
                v.text.len(),
                "{}",
                v.title
            );
        }
    }

    #[test]
    fn the_desktop_menu_keeps_quit() {
        let mut app = app(Platform::Desktop);
        tap(&mut app, KeyCode::Escape);
        assert_eq!(view(&app).items.last().unwrap(), "Quit");
    }

    #[test]
    fn browser_settings_pages_leave_out_what_it_does_not_have() {
        let mut app = app(Platform::Browser);
        tap(&mut app, KeyCode::Escape);
        // Settings: no Riders (no profiles for the anonymous rider).
        for _ in 0..5 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Settings");
        assert_eq!(
            v.items,
            [
                "Account",
                "Controls, input mapping",
                "Trainer",
                "Audio",
                "Display",
                "Voice",
                "Back"
            ]
        );
        // Display → Graphics without fullscreen (B7: fullscreen moved
        // there).
        let rows = |app: &App, p| sub_rows(app.world(), p);
        let display = rows(&app, sub::Page::Display);
        assert!(!display.is_empty());
        let graphics = rows(&app, sub::Page::Graphics);
        assert!(
            graphics
                .iter()
                .any(|r| row_setting(r) == Some("display.quality"))
        );
        assert!(
            display
                .iter()
                .chain(&graphics)
                .all(|r| row_setting(r) != Some("display.fullscreen"))
        );
        // God mode's rows are there (spec §14 Phase 13 G5: the browser
        // flies too), on Controls -> Unicorn mode (M9: Controls is full).
        let controls = rows(&app, sub::Page::Controls);
        let unicorn_row = Some(sub::Activate::Open(sub::Page::UnicornControls));
        assert!(controls.iter().any(|r| r.confirm == unicorn_row));
        let unicorn = rows(&app, sub::Page::UnicornControls);
        for key in ["controls.god_flight", "controls.invert_y"] {
            assert!(unicorn.iter().any(|r| row_setting(r) == Some(key)), "{key}");
        }
        let desktop = for_platform(
            sub::rows(
                sub::Page::Graphics,
                &Settings::default(),
                &Riders::default(),
            ),
            Platform::Desktop,
        );
        assert!(
            desktop
                .iter()
                .any(|r| row_setting(r) == Some("display.fullscreen"))
        );
    }

    /// The user (2026-11-19): damage has its row in the Settings section
    /// of both apps – Settings → Controls, beside Collisions – and the
    /// separate damage-indicator switch is gone from every page.
    #[test]
    fn damage_is_reachable_from_settings_in_both_apps() {
        for platform in [Platform::Desktop, Platform::Browser] {
            let mut app = app(platform);
            tap(&mut app, KeyCode::Escape);
            for _ in 0..5 {
                tap(&mut app, KeyCode::ArrowDown);
            }
            tap(&mut app, KeyCode::Enter);
            let v = view(&app);
            assert_eq!(v.title, "Settings", "{platform:?}");
            assert!(
                v.items.iter().any(|i| i == "Controls, input mapping"),
                "{platform:?}"
            );
            let controls = sub_rows(app.world(), sub::Page::Controls);
            assert!(controls.len() <= 6, "the ten-foot rule: {controls:?}");
            let i = controls
                .iter()
                .position(|r| row_setting(r) == Some("ride.damage"))
                .unwrap_or_else(|| panic!("{platform:?}: no Damage row"));
            assert_eq!(controls[i].label, "Damage");
            assert_eq!(row_setting(&controls[i - 1]), Some("ride.collisions"));
            // Chaotic bots moved to the bots and aids page.
            let aids = sub_rows(app.world(), sub::Page::RidingAids);
            assert!(
                aids.iter()
                    .any(|r| row_setting(r) == Some("ride.chaotic_bots"))
            );
            for p in sub::Page::TABS.into_iter().chain([
                sub::Page::RidingAids,
                sub::Page::Hud,
                sub::Page::Accessibility,
            ]) {
                assert!(
                    sub_rows(app.world(), p)
                        .iter()
                        .all(|r| row_setting(r) != Some("display.damage")),
                    "{p:?}"
                );
            }
        }
    }

    #[test]
    fn a_setting_row_changes_the_setting_through_the_console() {
        let mut app = app(Platform::Browser);
        tap(&mut app, KeyCode::Escape);
        for _ in 0..5 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        // Display: the first row is the HUD switch.
        // (Account comes first since the sign-in, spec §9.1.)
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Display");
        // HUD: its own page (the HUD switch, the navigation map, …).
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "HUD");
        assert_eq!(view(&app).items[0], "HUD:  < on >");
        tap(&mut app, KeyCode::ArrowRight);
        app.update();
        let us = app.world().resource::<UserSettings>();
        assert_eq!(us.value("display.hud").as_bool(), Some(false));
    }
}
