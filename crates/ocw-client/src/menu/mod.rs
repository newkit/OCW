//! The in-game menu (spec §3.2.1), Bevy-free and shared by the desktop and
//! browser apps (spec §14 Phase 10, B6: one menu, Q6 full parity):
//!
//! - [`model`]: pages, the page stack, focus, paging (six rows and Back,
//!   the ten-foot rule) and the [`model::Platform`] filter,
//! - [`controls`]: keys and gamepad buttons → menu actions (the apps read
//!   their input into [`controls::Pressed`]),
//! - [`pages`]: the Settings / Workshop / Workouts / World rows (each a
//!   console line), [`weather`] the weather machine, [`time`] the time
//!   machine, [`voice`] the voice pages,
//! - [`hint`] and [`status_line`]: the text under and over a page.
//!
//! The Bevy glue (input → model → `ocw_render::menu::MenuView`) is
//! `ocw_ui::menu`, the same for both apps.

pub mod controls;
pub mod history;
pub mod model;
pub mod pages;
pub mod time;
pub mod voice;
pub mod weather;

use ocw_core::mps_to_kmh;
use ocw_render::RideView;

use model::Page;

/// The status strip of the top page: power, speed, trainer and steering.
pub fn status_line(view: &RideView) -> String {
    match view.local() {
        Some(r) => format!(
            "{:.0} W   {:.1} km/h   {}",
            r.power_w,
            mps_to_kmh(r.state.speed),
            view.status
        ),
        None => view.status.clone(),
    }
}

/// The control hint at the bottom of `page` (`list_pages` pages of a
/// list, `depth` pages open).
pub fn hint(page: Page, list_pages: usize, depth: usize) -> String {
    let back = if depth > 1 { "back" } else { "close" };
    match page {
        // The preset grid (spec §14 Phase 9, S5): the arrows move.
        Page::Sub(p) if p.grid_columns().is_some() => {
            return format!("Arrows move   Enter/A choose   Esc/B {back}");
        }
        // A binding row: Enter waits for the new key (spec §3.2.1 M9).
        Page::Sub(pages::Page::BindGroup(_)) => {
            return format!("Enter/A new key or button   Left/Right step   Esc/B {back}");
        }
        Page::Sub(p) if p.is_history_page() => {
            return format!(
                "Up/Down move   {}Enter/A select   Esc/B {back}",
                if list_pages > 1 {
                    "Left/Right page   "
                } else {
                    ""
                }
            );
        }
        Page::Sub(p) if p.is_weather_page() => {
            // One line at 16:9 (the generic hint is longer).
            return format!("↑/↓ move   ←/→ change   Enter/A select   Esc/B {back}");
        }
        // The rider list: information, paged with Left / Right.
        Page::Sub(pages::Page::RiderList) => {
            return format!(
                "Up/Down move   {}Esc/B {back}",
                if list_pages > 1 {
                    "Left/Right page   "
                } else {
                    ""
                }
            );
        }
        _ => {}
    }
    format!(
        "Up/Down move   {}Enter/A select   Esc/B {}",
        match page {
            Page::Sub(p) if p.is_ride_page() && list_pages > 1 => "Left/Right page   ",
            Page::Sub(p) if p.is_ride_page() => "",
            Page::Sub(_) => "Left/Right change   Q/E, LB/RB tab   ",
            _ => "",
        },
        back
    )
}
