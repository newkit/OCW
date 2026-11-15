//! Unicorn mode in the browser (spec §14 Phase 13 G5, v3.38): the
//! desktop's god mode, shared as `ocw_ui::god` (keys by what they type –
//! the `KeyboardEvent.key` –, gamepad, mouse, touch on the left half,
//! inspection and pins, the HUD, the menu row and settings, the console
//! commands with their permissions; the server's rules in a shared ride
//! over WebTransport), plus what only the page has:
//!
//! - **Pointer Lock:** Bevy asks for it on the canvas when the flight
//!   captures the pointer (G, the menu row or a click are the user gesture
//!   the browser wants); the browser ends it itself on Esc and may refuse
//!   it, so [`ocw_ui::god::PointerLockWatch`] frees the captured pointer
//!   when `document.pointerLockElement` is gone – the next Esc opens the
//!   menu, a click locks again;
//! - **`window.ocw.god()`:** the flight as JSON for the page and the
//!   headless test ([`god_json`]).

use std::sync::Mutex;

use bevy::prelude::*;
use ocw_ui::god::{FlightPointer, GodStatus, PointerLockWatch};

pub struct WebGodPlugin;

impl Plugin for WebGodPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ocw_ui::god::GodPlugin>() {
            app.add_plugins(ocw_ui::god::GodPlugin);
        }
        app.add_systems(PreUpdate, watch_pointer_lock)
            .add_systems(Last, publish);
    }
}

/// Whether the page holds the Pointer Lock (natively: whatever the app
/// asked for, so the tests see the desktop's behaviour).
fn page_locked(captured: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = captured;
        crate::browser::pointer_locked()
    }
    #[cfg(not(target_arch = "wasm32"))]
    captured
}

/// Frees the captured pointer when the browser ended or refused the lock.
fn watch_pointer_lock(
    time: Res<Time>,
    pointer: Option<ResMut<FlightPointer>>,
    mut watch: Local<PointerLockWatch>,
) {
    let Some(mut pointer) = pointer else { return };
    let locked = page_locked(pointer.captured);
    let keep = watch.keep(pointer.captured, locked, time.elapsed_secs_f64());
    if pointer.captured && !keep {
        info!("unicorn mode: the browser released the pointer lock");
        pointer.captured = false;
    }
}

/// The flight as JSON for the page (`window.ocw.god()`).
static GOD_JSON: Mutex<String> = Mutex::new(String::new());

/// The last published flight state (JSON; empty before the first frame).
pub fn god_json() -> String {
    GOD_JSON.lock().expect("not poisoned").clone()
}

/// The flight's JSON: the phase, the pointer, the unicorn's position and
/// height, the inspection's panel and crosshair, the pins.
pub fn flight_json(
    status: &GodStatus,
    pointer: &FlightPointer,
    view: &ocw_render::RideView,
) -> String {
    let f = view.flight.as_ref();
    serde_json::json!({
        "active": status.active,
        "flying": status.flying,
        "asking": status.asking,
        "captured": pointer.captured,
        "pos": f.map(|f| f.pos),
        "height_m": f.map(|f| f.height_m),
        "speed_mps": f.map(|f| f.speed_mps),
        "first_person": f.map(|f| f.first_person),
        "panel": f.map(|f| f.panel.clone()).unwrap_or_default(),
        "crosshair": f.and_then(|f| f.crosshair.clone()),
        "pins": view.pins.len(),
        "unicorns": view.unicorns.len(),
    })
    .to_string()
}

fn publish(
    status: Option<Res<GodStatus>>,
    pointer: Option<Res<FlightPointer>>,
    view: Option<Res<ocw_render::RideView>>,
) {
    let (Some(status), Some(pointer), Some(view)) = (status, pointer, view) else {
        return;
    };
    *GOD_JSON.lock().expect("not poisoned") = flight_json(&status, &pointer, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flight_json_names_the_phase_height_and_inspection() {
        let mut view = ocw_render::RideView::new(ocw_world::Route::test_loop());
        let riding = flight_json(&GodStatus::default(), &FlightPointer::default(), &view);
        let v: serde_json::Value = serde_json::from_str(&riding).unwrap();
        assert_eq!(v["flying"], false);
        assert!(v["pos"].is_null() && v["crosshair"].is_null());
        view.flight = Some(ocw_render::FlightView {
            pos: [10.0, 520.0, -30.0],
            height_m: 80.0,
            panel: vec!["Position x 10 z -30 m".into()],
            crosshair: Some(vec!["Sky".into()]),
            ..Default::default()
        });
        let flying = flight_json(
            &GodStatus {
                active: true,
                flying: true,
                asking: false,
            },
            &FlightPointer { captured: true },
            &view,
        );
        let v: serde_json::Value = serde_json::from_str(&flying).unwrap();
        assert_eq!(v["flying"], true);
        assert_eq!(v["captured"], true);
        assert_eq!(v["pos"][1], 520.0);
        assert_eq!(v["height_m"], 80.0);
        assert_eq!(v["panel"][0], "Position x 10 z -30 m");
        assert_eq!(v["crosshair"][0], "Sky");
    }
}
