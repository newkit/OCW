//! The weather as the client shows it (spec §3.2.1 World → Weather, §14
//! Phase 9). Phase 9 S4 (weather simulation) and S5 (weather machine) are
//! not implemented yet: until then the world is always clear, and this is
//! only the hook for the menu and, later, the HUD and the renderer – a
//! client view model, no server weather. With S4 the server's session
//! weather (in `Welcome` / `Snapshot`, like the world clock) fills it.

use bevy::prelude::*;

use crate::settings::pages::Row;

/// What the sky does (S4 adds clouds, rain, snow, fog, wind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Condition {
    #[default]
    Clear,
}

impl Condition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "clear",
        }
    }
}

/// The weather at the rider's place, for the menu (and later the HUD).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct WeatherState {
    pub condition: Condition,
    /// Where it comes from: `false` until the weather simulation (S4)
    /// sends real state; the menu then says that it follows.
    pub simulated: bool,
}

impl WeatherState {
    /// "clear" (with S4: "rain, 9 °C, wind 4 m/s from SW").
    pub fn summary(&self) -> String {
        self.condition.label().to_owned()
    }
}

/// The text of World → Weather while there is no simulation.
pub const PENDING_NOTE: &str = "Clear (weather simulation follows, Phase 9 S4). The weather machine (presets, transitions, real weather) follows with S5.";

/// The rows of World → Weather: the weather now, and the weather machine
/// (disabled until S5: shown, not selectable).
pub fn rows(w: &WeatherState) -> Vec<Row> {
    let info = |label: &str, value: String| Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: None,
    };
    vec![
        info(
            "Now",
            if w.simulated {
                w.summary()
            } else {
                format!("{} (no simulation yet)", w.summary())
            },
        ),
        info("Weather machine", "disabled until Phase 9 S5".into()),
    ]
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeatherState>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_s4_the_weather_is_clear_and_the_machine_disabled() {
        let w = WeatherState::default();
        assert_eq!(w.condition, Condition::Clear);
        let r = rows(&w);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].value.as_deref(), Some("clear (no simulation yet)"));
        assert_eq!(r[1].label, "Weather machine");
        // Nothing to select or change: disabled.
        assert!(
            r.iter()
                .all(|r| r.confirm.is_none() && r.left.is_none() && r.right.is_none())
        );
        assert!(PENDING_NOTE.contains("clear") || PENDING_NOTE.starts_with("Clear"));
        assert!(PENDING_NOTE.contains("Phase 9 S4"));
        let sim = WeatherState {
            simulated: true,
            ..w
        };
        assert_eq!(rows(&sim)[0].value.as_deref(), Some("clear"));
    }
}
