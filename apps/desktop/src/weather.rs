//! The weather on the desktop (spec §3.2.1 World → Weather, §14 Phase 9,
//! S4 / S5 / S7): shared with the browser app since Phase 10 B6 – the
//! menu's state and rows are `ocw_client::menu::weather`, the Bevy glue
//! and the `weather …` commands `ocw_ui::weather`. Here: when a desktop
//! ride starts in real weather (`--real-weather`, scripted runs).

pub use ocw_ui::weather::*;

/// Whether a new ride starts in real weather (spec §14 Phase 9, 9A, S7):
/// `--real-weather BOOL`, else the setting `weather.real` (on by
/// default) – except screenshot and capture runs (reproducible) and unit
/// tests (no network).
pub fn real_at_start(world: &bevy::prelude::World, args: &crate::Args) -> bool {
    if cfg!(test) {
        return false;
    }
    ocw_ui::weather::real_at_start(
        world,
        args.real_weather,
        args.screenshot.is_some() || args.capture.is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, Menu, Page as MenuPage};
    use crate::settings::pages::Page;
    use ocw_weather::presets::preset;

    fn state(name: &str, may: bool) -> WeatherState {
        let w = preset(name).unwrap();
        WeatherState {
            now: Some(w),
            ground: ocw_weather::Ground::settled(&w),
            mode: Some(ocw_weather::Mode::Fixed),
            target: None,
            control: if may { "you" } else { "the host" }.into(),
            may_change: may,
            event: None,
            sim_seed: 0,
            real: None,
            physics: Some(true),
            machine: Machine::default(),
        }
    }

    /// The menu: World → Weather → Presets (a grid: the arrows move in two
    /// directions) and Custom, with the rows above.
    #[test]
    fn the_menu_reaches_every_page_of_the_weather_machine() {
        let w = state("clear", true);
        let rows_of = |p: Page| rows(p, &w);
        let mut m = Menu::default();
        m.apply_with(Action::Open, rows_of);
        // Top → World → Weather.
        m.apply_with(Action::Click(2), rows_of);
        let world_rows = |p: Page| {
            if p.is_weather_page() {
                rows(p, &w)
            } else {
                crate::ride::world::rows(p, &Default::default(), &Default::default(), &w)
            }
        };
        m.apply_with(Action::Click(4), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Weather)));
        // Presets: a grid of 17 and Back.
        m.apply_with(Action::Click(2), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherPresets)));
        assert_eq!(m.item_count(), 18);
        assert_eq!(m.grid_columns(), Some(GRID_COLUMNS));
        m.apply_with(Action::Right, world_rows);
        m.apply_with(Action::Down, world_rows);
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 11, "two rows down, one right: hail");
        assert_eq!(
            m.apply_with(Action::Confirm, world_rows),
            Effect::Command("weather hail in 60".into())
        );
        // Down from the last row wraps to the top, Up from the top to the
        // last row, in the same column.
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 16);
        m.apply_with(Action::Down, world_rows);
        assert_eq!(m.focus(), 1);
        m.apply_with(Action::Up, world_rows);
        assert_eq!(m.focus(), 16);
        m.apply_with(Action::Left, world_rows);
        m.apply_with(Action::Left, world_rows);
        assert_eq!(m.focus(), 14);
        // Back is the last cell.
        m.apply_with(Action::Click(17), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Weather)));
        // Custom → Wind and temperature.
        m.apply_with(Action::Click(3), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherCustom)));
        assert_eq!(
            m.apply_with(Action::Right, world_rows),
            Effect::Command("weather clouds=0.1 in 60".into())
        );
        // The ‹ arrow of row 4 (fog), clicked or tapped.
        assert_eq!(
            m.apply_with(Action::Step(4, -1), world_rows),
            Effect::Command("weather fog=20000 in 60".into())
        );
        assert_eq!(m.focus(), 4);
        m.apply_with(Action::Click(5), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherAir)));
        // Wind, gust, direction, temperature, weather physics and Back.
        assert_eq!(m.labels().len(), 6);
        m.apply_with(Action::Back, world_rows);
        m.apply_with(Action::Back, world_rows);
        // The transition and the seed.
        m.apply_with(Action::Hover(4), world_rows);
        assert_eq!(
            m.apply_with(Action::Right, world_rows),
            Effect::Command("weather.transition 300".into())
        );
        m.apply_with(Action::Hover(5), world_rows);
        assert_eq!(
            m.apply_with(Action::Confirm, world_rows),
            Effect::Command("weather random 1 in 60".into())
        );
        // Real weather: its page (S7), Enter turns it on.
        m.apply_with(Action::Click(1), world_rows);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::WeatherReal)));
        assert_eq!(
            m.apply_with(Action::Confirm, world_rows),
            Effect::Command("weather.real on".into())
        );
        assert_eq!(
            m.apply_with(Action::Step(1, 1), world_rows),
            Effect::Command("weather.provider meteoswiss".into())
        );
        m.apply_with(Action::Back, world_rows);
        // A dimmed row does nothing (the guest's).
        let guest = state("clear", false);
        let guest_rows = |p: Page| rows(p, &guest);
        m.apply_with(Action::Hover(0), guest_rows);
        assert_eq!(m.apply_with(Action::Confirm, guest_rows), Effect::None);
        assert!(m.dimmed()[0]);
    }
}
