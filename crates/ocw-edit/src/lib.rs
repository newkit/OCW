//! The document core of OCW Studio (spec §14 Phase 16, 16A): Bevy-free,
//! wasm-safe and tested. The studio app (`apps/studio`) only shows the
//! documents and sends commands to them, the way `ClientCore` drives the
//! game (§10).
//!
//! - [`document`]: a [`Document`] holds its content (any
//!   [`DocumentKind`]), its edit history – every change is an operation
//!   with its exact inverse, so **undo / redo** is unlimited and the
//!   history can be saved with the session – the dirty flags, the
//!   [`Selection`] and the validators' findings.
//! - [`session`]: the open documents of all kinds ([`Session`]), the
//!   active one, undo / redo of the active document, autosave of all.
//! - [`autosave`]: the autosave records and where they go
//!   ([`AutosaveStore`]: a directory on the desktop,
//!   `<data dir>/ocw/studio/autosave/`; memory in tests and, later, the
//!   browser).
//! - [`validate`]: the findings validators report after every change
//!   (the model contract of Phase 15, the world patch checks of 16C, route
//!   continuity plug in here).
//! - [`world`]: the World workspace's document (the package opened, the
//!   saved views). The world patch operations of 16C / E5 plug in as more
//!   [`world::WorldOp`]s (or a document kind of their own).
//!
//! - [`model`]: the Models workspace's document (16B, E4): one model
//!   file with its manifest entry, node / point / proxy / material /
//!   metadata edits with their inverses, the contract's validator, the
//!   fixes, the save with `model diff` and the re-import.
//!
//! - [`route`]: the Routes workspace's document (via-points and legs on a
//!   package's road graph, the imported track and its problem sections),
//!   the [`route::Editor`] that turns route edits into commands with the
//!   built-in planner, imports (GPX, TCX, GeoJSON, FIT, matched onto the
//!   graph) and exports ([`route::formats`]).
//!
//! **Adding a document kind:** implement [`DocumentKind`] for the content
//! type – `apply` changes it and returns the inverse operation, `merges`
//! says which consecutive operations are one undo step (a drag, typing) –
//! and open it with [`Session::open`].

pub mod autosave;
pub mod document;
pub mod model;
pub mod route;
pub mod selection;
pub mod session;
pub mod validate;
pub mod world;

#[cfg(not(target_arch = "wasm32"))]
pub use autosave::DirStore;
pub use autosave::{AutosaveRecord, AutosaveStore, MemoryStore, autosave_name};
pub use document::{DocId, Document, DocumentKind, EditError, Entry, History, Step};
pub use selection::Selection;
pub use session::{AnyDocument, Session};
pub use validate::{Finding, Severity, Validator};
