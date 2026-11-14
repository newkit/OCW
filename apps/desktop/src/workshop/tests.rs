//! The workshop in the app (spec §11.4, §14 Phase 4, 4J): the menu opens
//! it, every device reaches every screen, leaving applies the bike, the
//! console commands change the wallet. Headless: the stepped local
//! server, the virtual trainer, no window.

use std::time::Duration;

use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::time::TimeUpdateStrategy;
use clap::Parser;
use ocw_render::console::ConsoleCorePlugin;
use ocw_world::Route;

use super::*;
use crate::connection::ConnectionPlugin;
use crate::menu::MenuPlugin;

fn app() -> App {
    let capture = std::env::temp_dir().join(format!("ocw-workshop-test-{}", std::process::id()));
    let args = Args::parse_from([
        "ocw-desktop",
        "--bots",
        "0",
        "--virtual",
        "--capture",
        capture.to_str().unwrap(),
    ]);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            20,
        )))
        .add_message::<AppExit>()
        .add_message::<TouchInput>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(args)
        .insert_resource(RideView::new(Route::test_loop()))
        .insert_resource(crate::connection::Ghosts(None))
        .add_plugins((
            ConnectionPlugin {
                route: Route::test_loop(),
            },
            crate::devices::DevicesPlugin,
            crate::input::InputPlugin,
            MenuPlugin,
            WorkshopPlugin,
        ));
    app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
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

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn run(app: &mut App, line: &str) {
    app.world_mut().resource_mut::<ConsoleState>().queue(line);
    frames(app, 3);
}

fn view(app: &App) -> WorkshopView {
    app.world().resource::<WorkshopView>().clone()
}

fn scene(app: &App) -> Workshop {
    app.world()
        .resource::<WorkshopState>()
        .scene
        .clone()
        .expect("open")
}

fn balance(app: &App) -> i64 {
    app.world()
        .resource::<WorkshopState>()
        .wallet
        .balance("Rider")
}

/// Opens the menu and chooses Workshop (Resume, Ride, World, Workshop).
fn open_from_menu(app: &mut App) {
    tap(app, KeyCode::Escape);
    for _ in 0..3 {
        tap(app, KeyCode::ArrowDown);
    }
    tap(app, KeyCode::Enter);
    frames(app, 3);
}

#[test]
fn the_menu_opens_the_scene_and_leaving_returns_to_it() {
    let mut app = app();
    open_from_menu(&mut app);
    let v = view(&app);
    assert!(v.open, "the scene is open");
    assert_eq!(v.bikes.len(), 6, "the six starter bikes");
    assert_eq!(v.hearts, "500 ♥");
    assert!(!app.world().resource::<ocw_render::menu::MenuView>().open);
    assert_eq!(
        *app.world().resource::<InputContext>(),
        InputContext::Workshop
    );
    // The ride goes on under it in autodrive.
    let c = &app.world().resource::<ClientState>().0;
    assert!(c.autodrive() && c.steering_held());
    // Esc in the overview leaves to the menu's top page; the ride's
    // steering comes back.
    tap(&mut app, KeyCode::Escape);
    frames(&mut app, 3);
    assert!(!view(&app).open);
    let menu = app.world().resource::<ocw_render::menu::MenuView>().clone();
    assert!(menu.open && menu.title == "Open Cycling World", "{menu:?}");
}

#[test]
fn keyboard_cycles_swaps_and_leaving_applies_the_bike() {
    let mut app = app();
    run(&mut app, "workshop open");
    assert!(view(&app).open);
    tap(&mut app, KeyCode::ArrowRight);
    assert_eq!(scene(&app).focus, 1);
    assert_eq!(view(&app).focus, Some(1));
    tap(&mut app, KeyCode::Enter);
    assert!(view(&app).swap.is_some(), "the swap plays");
    frames(&mut app, 80);
    let s = scene(&app);
    assert_eq!(s.stand, 1);
    let id = s.bikes[1].clone();
    let st = app.world().resource::<WorkshopState>();
    assert_eq!(st.wallet.get("Rider").unwrap().active, id);
    // The server still rides the old bike: changes apply when leaving.
    let c = &app.world().resource::<ClientState>().0;
    assert_eq!(c.profile().bike, ProfileId::Road);
    tap(&mut app, KeyCode::Escape);
    frames(&mut app, 5);
    let c = &app.world().resource::<ClientState>().0;
    assert_eq!(c.profile().bike.id(), id, "applied on leaving");
}

#[test]
fn the_stand_hotspots_and_the_picker_with_the_keyboard() {
    let mut app = app();
    run(&mut app, "hearts.grant 1000");
    assert_eq!(balance(&app), 1500);
    run(&mut app, "workshop open");
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 40);
    let v = view(&app);
    assert_eq!(v.hotspots.len(), ocw_assets::bikes::HOTSPOT_SLOTS.len());
    assert_eq!(v.hotspots.iter().filter(|h| h.focused).count(), 1);
    // E / Q step through the slots.
    let before = scene(&app).hotspot;
    tap(&mut app, KeyCode::KeyE);
    assert_eq!(scene(&app).hotspot, before + 1);
    tap(&mut app, KeyCode::KeyQ);
    assert_eq!(scene(&app).hotspot, before);
    // Enter opens the picker of the focused slot.
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 2);
    let p = view(&app).picker.expect("picker");
    assert_eq!(p.rows[0].group.as_deref(), Some("Equipped"));
    assert!(!p.detail.is_empty());
    tap(&mut app, KeyCode::Escape);
    assert!(view(&app).picker.is_none());
}

#[test]
fn console_commands_buy_sell_and_list() {
    let mut app = app();
    run(&mut app, "hearts.grant 500");
    run(&mut app, "workshop.buy tyres.road.performance --bike road");
    assert_eq!(balance(&app), 1000 - 164);
    let st = app.world().resource::<WorkshopState>();
    let g = st.wallet.get("Rider").unwrap();
    let item = g.equipped("road", Slot::Tyres).expect("fitted").id;
    run(&mut app, &format!("workshop.sell {item}"));
    assert_eq!(balance(&app), 1000, "full refund before the first ride");
    // Not fitting, not affordable: refused, nothing booked.
    run(&mut app, "workshop.buy tyres.mtb.xc --bike road");
    run(&mut app, "workshop.buy frame.tt.top --bike tt");
    assert_eq!(balance(&app), 1000);
    run(&mut app, "workshop.paint #43A047 --bike gravel");
    let st = app.world().resource::<WorkshopState>();
    let g = st.wallet.get("Rider").unwrap();
    assert_eq!(g.bike("gravel").unwrap().paint(), Some(Rgb(0x43A047)));
    // bikes.select outside the workshop applies at once.
    run(&mut app, "bikes.select mtb");
    let c = &app.world().resource::<ClientState>().0;
    assert_eq!(c.profile().bike, ProfileId::Mountain);
}

#[test]
fn virtual_power_earns_no_hearts() {
    let mut app = app();
    run(&mut app, "sim.power 900");
    frames(&mut app, 50 * 30);
    let c = &app.world().resource::<ClientState>().0;
    assert_eq!(c.server_hearts(), 0, "the virtual trainer earns nothing");
    assert_eq!(balance(&app), 500);
}

fn touch(app: &mut App, phase: TouchPhase, id: u64, at: Vec2) {
    app.world_mut().write_message(TouchInput {
        phase,
        position: at,
        window: Entity::PLACEHOLDER,
        force: None,
        id,
    });
    app.update();
}

#[test]
fn touch_swipes_through_the_bikes_and_two_fingers_go_back() {
    let mut app = app();
    run(&mut app, "workshop open");
    // A fast swipe to the left: the next bike.
    let y = 400.0;
    touch(&mut app, TouchPhase::Started, 1, Vec2::new(900.0, y));
    touch(&mut app, TouchPhase::Moved, 1, Vec2::new(700.0, y));
    touch(&mut app, TouchPhase::Moved, 1, Vec2::new(500.0, y));
    touch(&mut app, TouchPhase::Ended, 1, Vec2::new(480.0, y));
    assert_eq!(scene(&app).focus, 1);
    assert!(view(&app).touch, "touch hints and the Back button");
    assert!(view(&app).hint.contains("Swipe"));
    // A two-finger tap: back (from the wall to the overview).
    touch(&mut app, TouchPhase::Started, 2, Vec2::new(300.0, y));
    touch(&mut app, TouchPhase::Started, 3, Vec2::new(400.0, y));
    touch(&mut app, TouchPhase::Ended, 2, Vec2::new(300.0, y));
    touch(&mut app, TouchPhase::Ended, 3, Vec2::new(400.0, y));
    assert_eq!(scene(&app).view(), camera::View::Overview);
    // The on-screen Back button leaves.
    app.world_mut()
        .resource_mut::<WorkshopPointer>()
        .0
        .push(WorkshopPointerEvent::Back);
    frames(&mut app, 3);
    assert!(!view(&app).open);
}

#[test]
fn the_gamepad_reaches_every_screen() {
    use bevy::input::gamepad::{Gamepad, GamepadButton};
    let mut app = app();
    run(&mut app, "workshop open");
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    let press = |app: &mut App, b: GamepadButton| {
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .press(b);
        app.update();
        let mut g = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        g.digital_mut().release(b);
        g.digital_mut().clear();
        app.update();
    };
    press(&mut app, GamepadButton::DPadRight);
    assert_eq!(scene(&app).focus, 1);
    press(&mut app, GamepadButton::DPadLeft);
    assert_eq!(scene(&app).focus, 0);
    assert!(view(&app).hint.contains("Ⓐ"), "{}", view(&app).hint);
    // Down: the bench, the mirror; Up back to the stand.
    press(&mut app, GamepadButton::DPadDown);
    press(&mut app, GamepadButton::DPadDown);
    assert_eq!(scene(&app).view(), camera::View::Mirror);
    press(&mut app, GamepadButton::DPadUp);
    press(&mut app, GamepadButton::DPadUp);
    assert_eq!(scene(&app).view(), camera::View::Overview);
    // A: the stand; RB: next slot; A: the picker; Y pins; B, B, B leave.
    press(&mut app, GamepadButton::South);
    frames(&mut app, 40);
    assert_eq!(scene(&app).view(), camera::View::Stand);
    press(&mut app, GamepadButton::RightTrigger);
    assert_eq!(scene(&app).hotspot, 1);
    press(&mut app, GamepadButton::South);
    assert!(scene(&app).picker.is_some());
    press(&mut app, GamepadButton::North);
    assert!(scene(&app).picker.as_ref().unwrap().pinned.is_some());
    press(&mut app, GamepadButton::East);
    press(&mut app, GamepadButton::East);
    press(&mut app, GamepadButton::East);
    frames(&mut app, 3);
    assert!(!view(&app).open);
}
