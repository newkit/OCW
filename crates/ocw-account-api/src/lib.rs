//! The account API of Open Cycling World (spec 14 Phase 4, 4O): what the
//! accounts backend and the apps say to each other about an account's
//! workshop – its bikes, parts and kit, the hearts balance and its ledger –
//! and about rides that earn hearts.
//!
//! - [`workshop`]: the garage document (the same shape as the desktop's
//!   local garage, so the workshop shows either), the ledger, the actions
//!   (`POST /workshop/purchase`, `/sell`, `/garage`) and their answers,
//! - [`rides`]: the signed credit of a game server whose key the
//!   project's root key certified (Q19 (a), `POST /rides/credit`), the
//!   ride summary the app uploads for any other ride (`POST /rides/summary`)
//!   and the answer,
//! - [`plausibility`]: the checks of spec 6.4 a summary must pass before
//!   its hearts are credited (power curve, spikes, cadence, constant power,
//!   heart rate, device trust),
//! - [`offline`]: the apps' side of Q20 (a) – the garage last synced for
//!   read-only riding offline and the queue of ride summaries uploaded once
//!   the backend answers.
//!
//! Pure and wasm-safe: no clock, no I/O; callers pass the time.

pub mod offline;
pub mod plausibility;
pub mod rider;
pub mod rides;
pub mod workshop;

/// Hearts every account starts with (4O, Q21 (a): everybody starts fresh,
/// nothing imported).
pub const START_HEARTS: i64 = 5_000;
