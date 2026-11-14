//! Weather for Open Cycling World (spec §14 Phase 9, milestone S4: weather
//! simulation and rendering; 9E, 9G): the weather state and its smooth
//! transitions ([`state`]), named states with the extremes (decision Q9,
//! [`presets`]), the server-owned session weather with its seeded
//! simulation ([`session`]), the ground's memory – road wetness, puddles,
//! snow cover ([`ground`]) –, gusts ([`wind`]), lightning and thunder
//! ([`lightning`]), what falls and how far one sees ([`particles`]), the
//! light under clouds ([`light`]), the client's follower ([`follow`]), the
//! console command ([`command`]) and real weather from the providers
//! ([`real`], S7: parsing, mapping, the server's feed with its cache and
//! fallback) and the weather's physics coefficients ([`physics`], S6: wind
//! at the rider, air density, wet-road rolling resistance and grip).
//!
//! Pure Rust without Bevy, threads, files, clock reads or OS randomness
//! (session time and seeds are passed in), so it builds for
//! `wasm32-unknown-unknown` (spec §3.5) and the server, the clients and the
//! tests agree.

pub mod command;
pub mod follow;
pub mod ground;
pub mod icon;
pub mod light;
pub mod lightning;
pub mod machine;
pub mod particles;
pub mod physics;
pub mod presets;
pub mod real;
pub mod session;
pub mod state;
pub mod wind;

pub use follow::WeatherFollower;
pub use ground::Ground;
pub use light::CloudLight;
pub use lightning::Strike;
pub use physics::{RelativeWind, WeatherPhysics};
pub use session::{Mode, SessionWeather};
pub use state::{CloudType, Condition, Precip, WeatherState};
