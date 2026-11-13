//! Trainer control loop of the client (spec §5.3): turns the road under the
//! rider (SIM) or a target power (ERG) into device commands. Pure; the Bevy
//! system in `connection.rs` calls [`TrainerControl::update`] every frame.

use ocw_core::RiderProfile;
use ocw_devices::{DeviceCommand, SimulationParameters};
use ocw_sim::RiderState;
use ocw_sim::trainer::{ErgController, ErgSettings, SimController, SimSettings};
use ocw_world::Track;

/// How far ahead (in seconds of riding) SIM mode looks for the gradient.
pub const LOOK_AHEAD_S: f64 = 1.0;
/// Minimum look-ahead distance, so a slow rider still anticipates.
pub const MIN_LOOK_AHEAD_M: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrainerMode {
    /// No resistance control (free riding / power meter only).
    Off,
    /// Simulate the road (default).
    Sim,
    /// Hold a target power.
    Erg { target_w: f64 },
}

impl std::fmt::Display for TrainerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Sim => write!(f, "SIM"),
            Self::Erg { target_w } => write!(f, "ERG {target_w:.0} W"),
        }
    }
}

pub struct TrainerControl {
    mode: TrainerMode,
    profile: RiderProfile,
    sim: SimController,
    erg: ErgController,
    /// Last gradient sent in SIM mode (for the HUD).
    last_grade: Option<f64>,
    /// God mode (spec §14 Phase 13E): SIM simulates a flat asphalt road.
    flat: bool,
}

/// Open question Q4 (spec §14 Phase 13H): while flying in god mode the
/// trainer simulates a flat road (lowest resistance; the proposal). ERG
/// keeps its target either way. `false` would keep the frozen road's
/// gradient instead.
pub const FLAT_WHILE_FLYING: bool = true;

impl TrainerControl {
    pub fn new(profile: RiderProfile) -> Self {
        Self {
            mode: TrainerMode::Sim,
            profile,
            sim: SimController::new(SimSettings::default(), &profile),
            erg: ErgController::new(ErgSettings::default()),
            last_grade: None,
            flat: false,
        }
    }

    /// God mode: flying (SIM: a flat road while [`FLAT_WHILE_FLYING`]).
    pub fn set_flying(&mut self, flying: bool) {
        self.flat = flying && FLAT_WHILE_FLYING;
    }

    pub fn mode(&self) -> TrainerMode {
        self.mode
    }

    /// Switches the mode; the new mode's first update is sent immediately.
    pub fn set_mode(&mut self, mode: TrainerMode) {
        let kind_changed = std::mem::discriminant(&mode) != std::mem::discriminant(&self.mode);
        self.mode = mode;
        if kind_changed {
            let settings = *self.sim.settings();
            self.sim = SimController::new(settings, &self.profile);
            self.erg = ErgController::new(ErgSettings::default());
            self.last_grade = None;
        }
    }

    pub fn difficulty(&self) -> f64 {
        self.sim.settings().difficulty
    }

    /// Trainer difficulty 0.0–1.0 (spec §5.3).
    pub fn set_difficulty(&mut self, difficulty: f64) {
        let settings = SimSettings {
            difficulty: difficulty.clamp(0.0, 1.0),
            ..*self.sim.settings()
        };
        self.sim.set_settings(settings);
    }

    /// Advances to `now_s`; returns a command when one should be sent.
    pub fn update(
        &mut self,
        now_s: f64,
        route: &dyn Track,
        rider: RiderState,
        cadence_rpm: Option<f64>,
    ) -> Option<DeviceCommand> {
        match self.mode {
            TrainerMode::Off => None,
            TrainerMode::Sim => {
                let ahead_m = (rider.speed * LOOK_AHEAD_S).max(MIN_LOOK_AHEAD_M);
                let (here, ahead, surface) = if self.flat {
                    (0.0, 0.0, ocw_world::Surface::Asphalt)
                } else {
                    (
                        route.sample(rider.distance).grade,
                        route.sample(rider.distance + ahead_m).grade,
                        ocw_sim::ride::surface(route, &rider),
                    )
                };
                // This bike's share of the surface's rolling resistance (§2.5).
                let rolling = self.profile.surface_factor(surface.rolling_factor());
                let target = self.sim.update_on(now_s, here, ahead, rolling)?;
                self.last_grade = Some(target.grade);
                Some(DeviceCommand::SetSimulation(SimulationParameters {
                    grade: target.grade,
                    wind_speed_mps: target.wind_speed_mps,
                    crr: target.crr,
                    cw_kg_per_m: target.cw_kg_per_m,
                    mass_kg: self.profile.rider_mass_kg + self.profile.bike_mass_kg,
                }))
            }
            TrainerMode::Erg { target_w } => self
                .erg
                .update(now_s, target_w, cadence_rpm)
                .map(DeviceCommand::SetTargetPower),
        }
    }

    /// One line for the HUD.
    pub fn status(&self) -> String {
        match (self.mode, self.last_grade) {
            (TrainerMode::Sim, Some(g)) => format!(
                "trainer SIM {:+.1} % (difficulty {:.0} %)",
                g * 100.0,
                self.difficulty() * 100.0
            ),
            (mode, _) => format!("trainer {mode}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    /// 100 m flat, then a 6 % climb (constant from 200 m on; the gradient
    /// is interpolated between route points).
    fn route() -> Route {
        Route::from_points(
            "c",
            "Climb",
            vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [200.0, 6.0, 0.0],
                [1100.0, 60.0, 0.0],
            ],
            false,
            6.0,
        )
        .unwrap()
    }

    fn at(distance: f64, speed: f64) -> RiderState {
        RiderState {
            distance,
            speed,
            lateral_offset: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn sim_mode_sends_the_scaled_road_gradient() {
        let mut t = TrainerControl::new(RiderProfile::default());
        assert_eq!(t.mode(), TrainerMode::Sim);
        let Some(DeviceCommand::SetSimulation(p)) =
            t.update(0.0, &route(), at(500.0, 8.0), Some(90.0))
        else {
            panic!("expected a SIM command");
        };
        // 6 % at 50 % difficulty.
        assert!((p.grade - 0.03).abs() < 1e-9, "{}", p.grade);
        assert_eq!(p.crr, RiderProfile::default().crr);
        assert!(t.status().contains("+3.0 %"));
        // Rate limited and at most 2 Hz: nothing new 0.1 s later.
        assert_eq!(t.update(0.1, &route(), at(500.8, 8.0), Some(90.0)), None);
    }

    /// The active rider's weight plus the bike is the SIM mass (spec §3.2.1).
    #[test]
    fn sim_mass_is_the_rider_profile_weight() {
        let mut t = TrainerControl::new(RiderProfile::for_rider_weight(64.0));
        let Some(DeviceCommand::SetSimulation(p)) = t.update(0.0, &route(), at(10.0, 8.0), None)
        else {
            panic!("expected a SIM command");
        };
        assert_eq!(p.mass_kg, 72.0);
    }

    /// The trainer gets the bike's own rolling resistance on gravel (§2.5):
    /// the mountain bike's wide tyres feel less of it than the road bike.
    #[test]
    fn sim_crr_follows_the_bike_profile_on_gravel() {
        let gravel = route().with_surfaces(vec![(0.0, 1000.0, ocw_world::Surface::Gravel)]);
        let crr = |profile| {
            let mut t = TrainerControl::new(profile);
            match t.update(0.0, &gravel, at(10.0, 8.0), None) {
                Some(DeviceCommand::SetSimulation(p)) => p.crr,
                other => panic!("expected a SIM command, got {other:?}"),
            }
        };
        let road = crr(RiderProfile::default());
        assert!((road - 0.004 * 2.5).abs() < 1e-9, "{road}");
        let mtb = crr(RiderProfile::curated(ocw_core::ProfileId::Mountain, 75.0));
        assert!(mtb < road, "{mtb} vs {road}");
    }

    #[test]
    fn sim_looks_ahead_before_the_climb() {
        let mut t = TrainerControl::new(RiderProfile::default());
        t.set_difficulty(1.0);
        // 95 m: still flat here, the climb starts 5 m ahead (at 10 m/s, 10 m look-ahead).
        let Some(DeviceCommand::SetSimulation(p)) = t.update(0.0, &route(), at(95.0, 10.0), None)
        else {
            panic!("expected a SIM command");
        };
        assert!(p.grade > 0.0);
    }

    /// God mode (spec §14 Phase 13E): SIM rides a flat road while flying,
    /// whatever the road below; ERG keeps its target.
    #[test]
    fn flying_simulates_a_flat_road_and_erg_stays() {
        let mut t = TrainerControl::new(RiderProfile::default());
        t.set_flying(true);
        let Some(DeviceCommand::SetSimulation(p)) = t.update(0.0, &route(), at(500.0, 8.0), None)
        else {
            panic!("a SIM command")
        };
        assert_eq!(p.grade, 0.0);
        t.set_flying(false);
        let Some(DeviceCommand::SetSimulation(p)) = t.update(10.0, &route(), at(500.0, 8.0), None)
        else {
            panic!("a SIM command")
        };
        assert!(p.grade > 0.0, "the climb again: {}", p.grade);
        t.set_mode(TrainerMode::Erg { target_w: 180.0 });
        t.set_flying(true);
        assert_eq!(
            t.update(20.0, &route(), at(500.0, 8.0), Some(90.0)),
            Some(DeviceCommand::SetTargetPower(180.0))
        );
    }

    #[test]
    fn erg_and_off_modes() {
        let mut t = TrainerControl::new(RiderProfile::default());
        t.set_mode(TrainerMode::Erg { target_w: 220.0 });
        assert_eq!(
            t.update(0.0, &route(), at(10.0, 8.0), Some(90.0)),
            Some(DeviceCommand::SetTargetPower(220.0))
        );
        assert_eq!(t.status(), "trainer ERG 220 W");
        // A new target within ERG ramps (does not reset the controller).
        t.set_mode(TrainerMode::Erg { target_w: 300.0 });
        assert_eq!(
            t.update(1.0, &route(), at(18.0, 8.0), Some(90.0)),
            Some(DeviceCommand::SetTargetPower(270.0))
        );
        t.set_mode(TrainerMode::Off);
        assert_eq!(t.update(2.0, &route(), at(26.0, 8.0), Some(90.0)), None);
        assert_eq!(t.status(), "trainer off");
        // Back to SIM: sent immediately.
        t.set_mode(TrainerMode::Sim);
        assert!(t.update(2.1, &route(), at(27.0, 8.0), None).is_some());
    }

    #[test]
    fn difficulty_is_clamped() {
        let mut t = TrainerControl::new(RiderProfile::default());
        t.set_difficulty(2.0);
        assert_eq!(t.difficulty(), 1.0);
        t.set_difficulty(-1.0);
        assert_eq!(t.difficulty(), 0.0);
    }
}
