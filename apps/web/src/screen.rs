//! The phone's screen in the browser (spec §3.2.3 TS4): the safe-area
//! insets and full screen. The page (`index.html`, `window.ocwScreen`)
//! measures `env(safe-area-inset-*)` – on load, on `resize` and on a turn
//! of the phone – and the app follows them every frame into
//! [`SafeArea`]: the HUD and the touch zones keep clear of the notch and
//! the home bar. `fullscreen` (Captures → *Full screen*) asks the page for
//! the Fullscreen API; on the iPhone, which has none, the page's answer
//! says how to add OCW to the home screen (the manifest's `fullscreen`).

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::input::touch::Insets;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::safe_area::SafeArea;

/// The page's insets (its JSON, `{"top": …, "right": …, "bottom": …,
/// "left": …}` in CSS px) as the HUD's safe area.
pub fn safe_area_of(json: &str) -> SafeArea {
    let i = Insets::from_json(json);
    SafeArea {
        top: i.top,
        right: i.right,
        bottom: i.bottom,
        left: i.left,
    }
}

/// The page's last insets text (only parsed when it changes).
#[derive(Resource, Default)]
struct PageInsets(String);

/// Reads the page's insets (`window.ocwScreen.safeArea()`).
fn page_insets() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::browser::call_page("ocwScreen", "safeArea", &[])
            .ok()
            .and_then(|v| v.as_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

fn follow_page_insets(mut last: ResMut<PageInsets>, mut safe: ResMut<SafeArea>) {
    let Some(text) = page_insets() else { return };
    if text == last.0 {
        return;
    }
    let s = safe_area_of(&text);
    info!("safe area from the page: {text}");
    last.0 = text;
    safe.set_if_neq(s);
}

/// `fullscreen toggle|on|off|status`: the page's Fullscreen API.
#[cfg(target_arch = "wasm32")]
fn fullscreen(what: &str) -> Result<String, String> {
    crate::browser::call_page("ocwScreen", "fullscreen", &[what.into()])
        .map(|v| v.as_string().unwrap_or_default())
}

#[cfg(not(target_arch = "wasm32"))]
fn fullscreen(_what: &str) -> Result<String, String> {
    Err("full screen is the browser page's (window.ocwScreen)".into())
}

pub fn build(app: &mut App) {
    app.init_resource::<SafeArea>()
        .init_resource::<PageInsets>()
        .add_systems(First, follow_page_insets);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("fullscreen")
            .about("Full screen where the browser has it (the menu's Captures -> Full screen); on the iPhone: Share -> Add to Home Screen")
            .arg(
                Arg::new("what")
                    .default_value("toggle")
                    .value_parser(["toggle", "on", "off", "status"]),
            ),
        |_, m| fullscreen(m.get_one::<String>("what").map_or("toggle", String::as_str)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_s_insets_become_the_safe_area() {
        let s = safe_area_of(r#"{"top": 0, "right": 59, "bottom": 21, "left": 59}"#);
        assert_eq!(
            s,
            SafeArea {
                top: 0.0,
                right: 59.0,
                bottom: 21.0,
                left: 59.0
            }
        );
        assert!(safe_area_of("").is_zero());
        assert!(safe_area_of("{\"top\": -3}").is_zero());
    }
}
