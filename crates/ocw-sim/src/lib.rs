//! Bicycle physics, rider state and session logic (spec §5).
//!
//! The same code runs in three places (spec §6.1):
//! - on the **server** (remote or in-process local server) for all riders, as the authority,
//! - on the **client** for its own rider only, as a prediction ([`Predictor`]),
//! - on the client to smooth other riders between snapshots ([`SnapshotBuffer`]).
//!
//! No engine, OS or network dependencies; must build for `wasm32-unknown-unknown`.

pub mod interpolation;
pub mod physics;
pub mod prediction;
pub mod session;
pub mod trainer;

pub use interpolation::SnapshotBuffer;
pub use physics::{Environment, RiderState};
pub use prediction::Predictor;
pub use session::{RiderInput, RiderSnapshot, Session};
pub use trainer::{ErgController, ErgSettings, SimController, SimSettings, SimTarget};
