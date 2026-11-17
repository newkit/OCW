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
        .insert_resource(RiderProfiles {
            riders: Default::default(),
            path: None,
            riding: Default::default(),
        })
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
    // A part slot's picker shows tiles: the fitted part first.
    assert!(p.tiles.is_some());
    assert!(p.rows[0].right.contains("equipped"), "{:?}", p.rows[0]);
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

/// Into the character editor: Down, Down (bench, mirror), Confirm.
fn to_mirror(app: &mut App) {
    run(app, "workshop open");
    tap(app, KeyCode::ArrowDown);
    tap(app, KeyCode::ArrowDown);
    assert_eq!(scene(app).view(), camera::View::Mirror);
    tap(app, KeyCode::Enter);
    frames(app, 40);
    assert_eq!(
        scene(app).view(),
        camera::View::Dress {
            front: false,
            face: false
        }
    );
}

/// The changing corner with the keyboard (spec 4C, W4): the rider's five
/// hotspots, E / Q through the slots, the helmet picker (owned / shop /
/// locked), buying and wearing the TT helmet, a free choice applies at
/// once, *Try on the bike* and back; leaving sends the look and the kit.
#[test]
fn the_character_corner_with_the_keyboard() {
    let mut app = app();
    to_mirror(&mut app);
    let v = view(&app);
    let rider = v.rider.clone().expect("the rider at the mirror");
    assert!(!rider.on_stand && rider.kit.is_empty(), "the base kit");
    assert!(v.mirror, "the reflection");
    assert_eq!(v.hotspots.len(), 5, "head, torso, hands, legs, feet");
    assert_eq!(
        scene(&app).dress.slot(),
        picker::MirrorSlot::Kit(Slot::Helmet)
    );
    // E steps to the cap, Q back to the helmet.
    tap(&mut app, KeyCode::KeyE);
    assert_eq!(scene(&app).dress.slot(), picker::MirrorSlot::Kit(Slot::Cap));
    tap(&mut app, KeyCode::KeyQ);
    // Down in screen space: from the head to the torso.
    tap(&mut app, KeyCode::ArrowDown);
    assert_ne!(scene(&app).dress.hotspot, 0);
    tap(&mut app, KeyCode::ArrowUp);
    assert_eq!(scene(&app).dress.hotspot, 0);
    // The helmet picker: the TT helmet (500 ♥) is locked at 500 − 1 ♥.
    run(&mut app, "hearts.grant -1");
    tap(&mut app, KeyCode::Enter);
    let p = view(&app).picker.expect("the helmet picker");
    assert!(p.title.starts_with("Helmet"), "{}", p.title);
    assert_eq!(p.rows[0].group.as_deref(), Some("Equipped"));
    let tt = p
        .rows
        .iter()
        .position(|r| r.left.starts_with("Time-trial helmet"))
        .expect("the TT helmet in the shop");
    assert!(
        p.rows[tt].locked && p.rows[tt].right.contains("needs 1 ♥ more"),
        "{:?}",
        p.rows[tt]
    );
    run(&mut app, "hearts.grant 1");
    // Focus it (the picker's tiles: → one tile, spec §14 Phase 4, 4N):
    // the live preview wears it; the card shows its benefit.
    for _ in 0..tt {
        tap(&mut app, KeyCode::ArrowRight);
    }
    let v = view(&app);
    assert_eq!(v.rider.as_ref().unwrap().kit, vec!["helmet-tt".to_string()]);
    let d = v.picker.unwrap().detail;
    assert!(
        d.iter()
            .any(|l| l.contains("W @25") && l.contains("pays off above")),
        "{d:?}"
    );
    // Buy: the dialog with Cancel focused; Up + Enter buys.
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::ArrowUp);
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 3);
    assert_eq!(balance(&app), 0);
    let st = app.world().resource::<WorkshopState>();
    let g = st.wallet.get("Rider").unwrap();
    assert_eq!(g.kit_on(cat(), Slot::Helmet).unwrap().id, "helmet.tt.top");
    // E in the picker: the next slot's picker (free: glasses after the cap).
    tap(&mut app, KeyCode::KeyE);
    tap(&mut app, KeyCode::KeyE);
    let p = view(&app).picker.unwrap();
    assert!(p.title.starts_with("Glasses"), "{}", p.title);
    assert_eq!(p.rows[0].right, "free");
    // A free choice applies at once: round glasses (the next tile).
    tap(&mut app, KeyCode::ArrowRight);
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 3);
    let riding = &app.world().resource::<RiderProfiles>().riding;
    assert_eq!(riding.character().glasses, 2);
    // Back, then *Try on the bike* (the last slot).
    tap(&mut app, KeyCode::Escape);
    while scene(&app).dress.slot() != picker::MirrorSlot::TryOn {
        tap(&mut app, KeyCode::KeyE);
    }
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 40);
    assert_eq!(scene(&app).view(), camera::View::TryOn);
    assert!(view(&app).rider.unwrap().on_stand);
    tap(&mut app, KeyCode::Escape);
    frames(&mut app, 40);
    assert!(!view(&app).rider.unwrap().on_stand, "Back: standing again");
    // Leaving applies: the server rides with the TT helmet (Open, free
    // ride) and has the character.
    for _ in 0..3 {
        tap(&mut app, KeyCode::Escape);
    }
    frames(&mut app, 10);
    assert!(!view(&app).open);
    let c = &app.world().resource::<ClientState>().0;
    let me = c.rider_id().unwrap();
    assert!(c.rider_parts(me).contains(&"helmet.tt.top".to_string()));
    let effect = c.effective_profile();
    assert!(
        effect.cda_m2 < ProfileId::Road.profile().cda_m2,
        "the TT helmet counts in Open"
    );
}

/// The same corner with the gamepad (LB / RB, A, B), the mouse (hover and
/// click on the rider's hotspots) and touch (a tap on a hotspot).
#[test]
fn the_character_corner_with_gamepad_mouse_and_touch() {
    use bevy::input::gamepad::{Gamepad, GamepadButton};
    let mut app = app();
    to_mirror(&mut app);
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
    press(&mut app, GamepadButton::RightTrigger);
    assert_eq!(scene(&app).dress.slot(), picker::MirrorSlot::Kit(Slot::Cap));
    press(&mut app, GamepadButton::LeftTrigger);
    press(&mut app, GamepadButton::LeftTrigger);
    assert_eq!(scene(&app).dress.slot(), picker::MirrorSlot::TryOn, "wraps");
    press(&mut app, GamepadButton::South);
    frames(&mut app, 40);
    assert_eq!(scene(&app).view(), camera::View::TryOn);
    assert!(view(&app).hint.contains("Ⓑ"), "{}", view(&app).hint);
    press(&mut app, GamepadButton::East);
    frames(&mut app, 40);
    assert_eq!(
        scene(&app).view(),
        camera::View::Dress {
            front: false,
            face: false
        }
    );
    // The hotspots' screen positions (the 1280 × 720 default window).
    let points = {
        let st = app.world().resource::<WorkshopState>();
        let s = st.scene.as_ref().unwrap();
        let layout = ocw_assets::workshop::layout(s.bikes.len());
        let pose = s.pose(&layout);
        layout
            .rider_hotspots(1.0)
            .map(|p| pose.to_screen(Vec2::new(1280.0, 720.0), p))
    };
    // Touch: a tap on the feet opens the shoes' picker.
    let feet = points[4].expect("the feet in view");
    touch(&mut app, TouchPhase::Started, 7, feet);
    touch(&mut app, TouchPhase::Ended, 7, feet);
    frames(&mut app, 2);
    assert_eq!(
        scene(&app).dress.slot(),
        picker::MirrorSlot::Kit(Slot::Shoes)
    );
    let p = view(&app).picker.expect("the shoes picker");
    assert!(p.title.starts_with("Shoes"), "{}", p.title);
    // Back with the on-screen button; then a mouse click on the torso.
    app.world_mut()
        .resource_mut::<WorkshopPointer>()
        .0
        .push(WorkshopPointerEvent::Back);
    frames(&mut app, 2);
    assert!(scene(&app).picker.is_none());
    let torso = points[1].unwrap();
    let mut s = app.world_mut().resource_mut::<WorkshopState>();
    s.queued.push(Act::Activate(torso));
    frames(&mut app, 2);
    assert_eq!(
        scene(&app).dress.slot(),
        picker::MirrorSlot::Kit(Slot::Jersey)
    );
    assert!(view(&app).picker.is_some());
}

/// `character` sets the free choices; `workshop.mirror off` gives the
/// direct front view (low graphics quality).
#[test]
fn the_character_command_and_the_front_view() {
    let mut app = app();
    run(
        &mut app,
        "character --shape ample --skin 9 --hair 4 --hair-colour #B3482A --beard full \
         --body c --age 50s",
    );
    let c = app.world().resource::<RiderProfiles>().riding.character();
    assert_eq!(c.shape, ocw_core::BodyShape::Ample);
    assert_eq!((c.skin, c.hair, c.beard), (8, 3, 4));
    assert_eq!(
        (c.base, c.age),
        (ocw_core::BaseBody::C, ocw_core::Age::Fifties)
    );
    assert_eq!(c.hair_colour, Rgb(0xB3482A));
    // Outside the workshop it applies at once: the hello's look.
    let look = app.world().resource::<ClientState>().0.view(0.0, 1.0)[0]
        .look
        .unwrap();
    assert_eq!(look.character.skin, 8);
    run(&mut app, "character --skin 17");
    assert_eq!(
        app.world()
            .resource::<RiderProfiles>()
            .riding
            .character()
            .skin,
        8,
        "out of the palette: refused"
    );
    run(&mut app, "workshop.mirror off");
    to_mirror_front(&mut app);
    assert!(!view(&app).mirror);
}

fn to_mirror_front(app: &mut App) {
    run(app, "workshop open");
    tap(app, KeyCode::ArrowDown);
    tap(app, KeyCode::ArrowDown);
    tap(app, KeyCode::Enter);
    frames(app, 40);
    assert_eq!(
        scene(app).view(),
        camera::View::Dress {
            front: true,
            face: false
        }
    );
}

/// The bench (spec 4D) in the app: `workshop plus` opens the catalogue on
/// the "+" hook; `bikes.buy` hangs the new bike there – the scene and the
/// view get a hook more –, `bikes.sell` frees it again.
#[test]
fn bikes_are_bought_onto_the_plus_hook_and_sold() {
    let mut app = app();
    run(&mut app, "hearts.grant 10000");
    run(&mut app, "workshop open");
    run(&mut app, "workshop plus");
    let v = view(&app);
    assert!(v.plus_focused);
    let p = v.picker.expect("the catalogue");
    assert!(p.title.starts_with("The bench"), "{}", p.title);
    assert!(p.rows.len() <= 6);
    run(
        &mut app,
        "bikes.buy frame.road-aero.top --with groupset.road-electronic.mid --with wheels.road-alloy.mid",
    );
    frames(&mut app, 2);
    assert_eq!(scene(&app).bikes.len(), 7);
    assert_eq!(view(&app).bikes.len(), 7);
    assert_eq!(view(&app).bikes[6].profile, ProfileId::RoadAero);
    assert_eq!(view(&app).bikes[6].card[0], "Road - aero 2");
    assert_eq!(balance(&app), 10_500 - 5_279 - 1_817 - 600);
    // A starter bike is never sold; the bought one is.
    run(&mut app, "bikes.sell road");
    assert_eq!(scene(&app).bikes.len(), 7);
    run(&mut app, "bikes.sell road-aero-2");
    frames(&mut app, 2);
    assert_eq!(view(&app).bikes.len(), 6);
    assert!(balance(&app) > 10_500 - 5_279 - 1_817 - 600);
}

/// The kit's colours (spec 4C) go into the garage with `kit.design`, also
/// for the default rider without a rider profile; looks only.
#[test]
fn kit_colours_go_into_the_garage() {
    let mut app = app();
    let before = riding_parts(app.world(), "Rider", ProfileId::Road);
    run(&mut app, "kit.design --shorts #3949AB --gloves #E53935");
    let g = app
        .world()
        .resource::<WorkshopState>()
        .wallet
        .get("Rider")
        .unwrap()
        .clone();
    let c = g.kit_colours.expect("kept in the garage");
    assert_eq!(c.shorts.as_deref(), Some("#3949AB"));
    assert_eq!(c.gloves.as_deref(), Some("#E53935"));
    assert_eq!(c.shoes, None);
    let look = crate::settings::local_rider(app.world()).look();
    assert_eq!(look.design.gloves, Some(Rgb(0xE53935)));
    assert_eq!(riding_parts(app.world(), "Rider", ProfileId::Road), before);
}

/// The stand bike's variants as shown.
fn stand_variants(app: &App) -> Vec<String> {
    let v = view(app);
    let mut m = v.bikes[v.stand].variants.clone();
    m.sort();
    m
}

/// The bike stand's tiles in the app (spec 4B / 4G *Tiles*): the wheels'
/// tiles with thumbnails (keyed by part and paint), the gain against the
/// base part on the flat and the climb and the weight, the bike's line
/// under the title; the keyboard, the gamepad, the mouse and a tap move
/// the focus and preview on the stand bike at once, a tap opens the
/// dialog, Esc cancels it and then restores the fitted wheels.
#[test]
fn the_bike_stand_tiles_with_every_device() {
    use bevy::input::gamepad::{Gamepad, GamepadButton};
    let mut app = app();
    run(&mut app, "workshop open");
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 40);
    let wheels = ocw_assets::bikes::HOTSPOT_SLOTS
        .iter()
        .position(|s| *s == "wheels")
        .unwrap();
    while scene(&app).hotspot != wheels {
        tap(&mut app, KeyCode::KeyE);
    }
    let fitted = stand_variants(&app);
    tap(&mut app, KeyCode::Enter);
    frames(&mut app, 2);
    let p = view(&app).picker.expect("the wheels' picker");
    let t = p.tiles.expect("tiles");
    assert_eq!((t.cols, t.first), (3, 0));
    assert_eq!(t.shown, p.rows.len().min(6));
    assert!(
        p.subtitle
            .as_deref()
            .is_some_and(|s| s.starts_with("This bike: ±0.0 % flat · ±0.0 % climb vs stock")),
        "{:?}",
        p.subtitle
    );
    for r in &p.rows {
        let Some(ocw_render::thumbs::ThumbKey::Part(k)) = r.thumb.as_ref() else {
            panic!("a part's thumbnail: {r:?}");
        };
        assert_eq!((k.profile, k.slot.as_str()), (ProfileId::Road, "wheels"));
        assert!(
            r.gain.contains(" flat · ") && r.gain.contains(" climb"),
            "{r:?}"
        );
        assert!(
            r.weight.ends_with(" g") || r.weight == "same weight",
            "{r:?}"
        );
    }
    // The base wheels: nothing; the carbon wheels: faster on both.
    assert_eq!(p.rows[0].gain, "±0.0 % flat · ±0.0 % climb");
    let carbon = p
        .rows
        .iter()
        .find(|r| {
            matches!(r.thumb.as_ref(), Some(ocw_render::thumbs::ThumbKey::Part(k)) if k.model == "wheel-carbon-50")
        })
        .expect("the top carbon wheels");
    assert!(
        carbon.gain.starts_with('+') && carbon.weight == "−650 g",
        "{carbon:?}"
    );
    assert!(
        p.detail
            .iter()
            .any(|l| l.starts_with("Against the base part: "))
    );
    // Regression (2026-11-17, the card ran out at the panel's bottom):
    // under the tiles the card starts with the benefit and the bike (the
    // name is on the focused tile), where parts count comes last, and what
    // the renderer shows of it is a part of it, in order, at 16:9 and 4:3.
    assert!(
        p.detail[0].starts_with("Benefit:") || p.detail[0].starts_with("On the bike now"),
        "{:?}",
        p.detail
    );
    assert!(p.detail.iter().all(|l| *l != p.rows[p.focus].left));
    assert_eq!(
        p.detail.last().map(String::as_str),
        Some(picker::WHERE_PARTS_COUNT)
    );
    for aspect in [16.0 / 9.0, 4.0 / 3.0] {
        let shown = ocw_render::workshop::fitted_card(&p, aspect);
        assert!(!shown.is_empty(), "{aspect}");
        let mut rest = p.detail.iter();
        assert!(
            shown.iter().all(|l| rest.any(|d| d == l)),
            "{aspect}: {shown:?}"
        );
    }
    // Keyboard: → previews the next tile on the stand bike.
    tap(&mut app, KeyCode::ArrowRight);
    assert_eq!(view(&app).picker.unwrap().focus, 1);
    assert_ne!(stand_variants(&app), fitted, "the preview");
    // Gamepad: the pad's ↓ one row down.
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::DPadDown);
    app.update();
    {
        let mut g = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        g.digital_mut().release(GamepadButton::DPadDown);
        g.digital_mut().clear();
    }
    app.update();
    assert_eq!(view(&app).picker.unwrap().focus, 4);
    // Mouse hover over tile 2, then a tap / click on tile 1: the dialog.
    app.world_mut()
        .resource_mut::<WorkshopPointer>()
        .0
        .push(WorkshopPointerEvent::Row(2));
    frames(&mut app, 2);
    assert_eq!(view(&app).picker.unwrap().focus, 2);
    app.world_mut()
        .resource_mut::<WorkshopPointer>()
        .0
        .push(WorkshopPointerEvent::RowClick(1));
    frames(&mut app, 2);
    let p = view(&app).picker.unwrap();
    assert_eq!(p.focus, 1);
    assert!(p.dialog.is_some(), "apply asks first");
    // Esc: the dialog goes, the preview stays; Esc again restores.
    tap(&mut app, KeyCode::Escape);
    let p = view(&app).picker.expect("still the picker");
    assert!(p.dialog.is_none());
    assert_ne!(stand_variants(&app), fitted);
    tap(&mut app, KeyCode::Escape);
    frames(&mut app, 2);
    assert!(view(&app).picker.is_none());
    assert_eq!(stand_variants(&app), fitted, "cancel restores");
}
