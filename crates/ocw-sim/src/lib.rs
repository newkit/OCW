//! Bicycle physics, rider state and session logic (spec §5).
//!
//! The same code runs in three places (spec §6.1):
//! - on the **server** (remote or in-process local server) for all riders, as the authority,
//! - on the **client** for its own rider only, as a prediction ([`Predictor`]),
//! - on the client to smooth other riders between snapshots ([`SnapshotBuffer`]).
//!
//! No engine, OS or network dependencies; must build for `wasm32-unknown-unknown`.

pub mod avoid;
pub mod collide;
pub mod drafting;
pub mod event;
pub mod freeride;
pub mod god;
pub mod interpolation;
pub mod physics;
pub mod prediction;
pub mod ride;
pub mod session;
pub mod trainer;
pub mod workout;

pub use event::{Event, EventKind, EventSpec, GhostTrace, Phase as EventPhase, Standing};
pub use freeride::TurnChoice;
pub use interpolation::SnapshotBuffer;
pub use physics::{Environment, RiderState, Steering};
pub use prediction::Predictor;
pub use session::{
    FreeRideStart, GodLeave, GodRider, PathChange, RiderInput, RiderSnapshot, Session,
};
pub use trainer::{ErgController, ErgSettings, SimController, SimSettings, SimTarget};
