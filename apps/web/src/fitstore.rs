//! The browser's FIT files of the training history (spec 14 Phase 7, 7A:
//! FIT files in the browser): each saved ride's FIT file is kept in the
//! Origin Private File System (folder `ocw-rides`, `<ride id>.fit`), so a
//! ride's *Export the FIT file* works after a reload too; the history
//! entry (in `localStorage`) names the file once it is written
//! (`RideEntry::fit`).
//!
//! OPFS is asynchronous: a [`FitBackend`] starts each operation and hands
//! back what finished ([`FitDone`]), which [`apply_fits`] applies once a
//! frame – the file named in the entry, a read for an export downloaded,
//! a file the browser has cleared forgotten. **Fallback** where OPFS is
//! missing (an older browser, a private window, a page that is not a
//! secure context) or fails: the FIT files stay in the page's memory as
//! before ([`Keeping::Memory`]), exported only while the page is open; the
//! history's top page says so. The browser's backend is
//! `crate::browser::OpfsFits` (wasm32); the tests use a mock.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ocw_client::history::RideEntry;
use ocw_render::console::ConsoleState;
use ocw_ui::history::TrainingHistory;

/// The OPFS folder of the rides' FIT files.
pub const FITS_DIR: &str = "ocw-rides";

/// A ride's FIT file in [`FITS_DIR`].
pub fn file_name(id: &str) -> String {
    format!("{id}.fit")
}

/// The history's note where the browser keeps no FIT files.
pub const MEMORY_NOTE: &str =
    "FIT files are kept only while this page is open (this browser has no private file storage)";

/// An operation that finished.
#[derive(Debug, Clone, PartialEq)]
pub enum FitDone {
    /// The storage can be used (or why not).
    Probed(Result<(), String>),
    Saved {
        id: String,
        result: Result<(), String>,
    },
    /// A read for an export: the bytes, or `None` when the file is gone.
    Read {
        id: String,
        result: Result<Option<Vec<u8>>, String>,
    },
    Removed {
        id: String,
        result: Result<(), String>,
    },
}

/// Where FIT files go: starts operations, hands back what finished.
pub trait FitBackend: Send + Sync + 'static {
    /// Checks that the storage can be used ([`FitDone::Probed`]).
    fn probe(&self);
    fn save(&self, id: &str, bytes: Vec<u8>);
    fn read(&self, id: &str);
    fn remove(&self, id: &str);
    /// What finished since the last call.
    fn take_done(&self) -> Vec<FitDone>;
}

/// How the FIT files are kept now.
#[derive(Debug, Clone, PartialEq)]
pub enum Keeping {
    /// The storage is being checked (operations already go to it).
    Probing,
    /// In the Origin Private File System: they survive a reload.
    Opfs,
    /// Only in the page's memory (the fallback), and why.
    Memory(String),
}

/// The browser's FIT store.
#[derive(Resource)]
pub struct FitKeeper {
    backend: Option<Box<dyn FitBackend>>,
    pub keeping: Keeping,
}

impl FitKeeper {
    /// Over `backend` (probed at once); `None`: the fallback.
    pub fn new(backend: Option<Box<dyn FitBackend>>) -> Self {
        let keeping = match &backend {
            Some(b) => {
                b.probe();
                Keeping::Probing
            }
            None => Keeping::Memory("no private file storage (OPFS) in this browser".into()),
        };
        Self { backend, keeping }
    }

    /// The backend, unless the fallback is in use.
    fn usable(&self) -> Option<&dyn FitBackend> {
        match self.keeping {
            Keeping::Memory(_) => None,
            _ => self.backend.as_deref(),
        }
    }

    /// Whether files kept on earlier pages can be read.
    pub fn persistent(&self) -> bool {
        self.usable().is_some()
    }

    /// Keeps a ride's FIT file (nothing in the fallback: the memory copy
    /// is the history's own).
    pub fn keep(&self, id: &str, bytes: Vec<u8>) {
        if let Some(b) = self.usable() {
            b.save(id, bytes);
        }
    }

    /// Reads a ride's FIT file for an export; false in the fallback.
    pub fn fetch(&self, id: &str) -> bool {
        self.usable().map(|b| b.read(id)).is_some()
    }

    /// Deletes a ride's FIT file; false in the fallback.
    pub fn forget(&self, id: &str) -> bool {
        self.usable().map(|b| b.remove(id)).is_some()
    }

    fn take_done(&self) -> Vec<FitDone> {
        self.backend
            .as_ref()
            .map(|b| b.take_done())
            .unwrap_or_default()
    }
}

/// The note on the history's top page for `keeping`.
pub fn note(keeping: &Keeping) -> Option<String> {
    match keeping {
        Keeping::Memory(_) => Some(MEMORY_NOTE.into()),
        _ => None,
    }
}

/// Applies what the store finished (once a frame).
pub fn apply_fits(world: &mut World) {
    let Some(done) = world.get_resource::<FitKeeper>().map(|k| k.take_done()) else {
        return;
    };
    for d in done {
        apply(world, d);
    }
}

fn say(world: &mut World, line: String) {
    info!("{line}");
    if let Some(mut c) = world.get_resource_mut::<ConsoleState>() {
        c.print(line);
    }
}

fn apply(world: &mut World, done: FitDone) {
    match done {
        FitDone::Probed(Ok(())) => {
            world.resource_mut::<FitKeeper>().keeping = Keeping::Opfs;
            crate::ui::web_refresh(world);
        }
        FitDone::Probed(Err(e)) => {
            warn!("no OPFS for the rides' FIT files ({e}): kept for this page only");
            world.resource_mut::<FitKeeper>().keeping = Keeping::Memory(e);
            crate::ui::web_refresh(world);
        }
        FitDone::Saved { id, result: Ok(()) } => {
            let saved = world
                .get_resource_mut::<TrainingHistory>()
                .and_then(|mut h| {
                    let r = h.history.ride_mut(&id)?;
                    r.fit = Some(file_name(&id));
                    Some(h.save())
                });
            if let Some(Err(e)) = saved {
                warn!("training history: {e}");
            }
            crate::ui::web_refresh(world);
        }
        FitDone::Saved { id, result: Err(e) } => {
            // Kept in memory for this page (the history has it).
            warn!("FIT file of ride {id} not kept in this browser: {e}");
        }
        FitDone::Read {
            id,
            result: Ok(Some(bytes)),
        } => {
            let ride = world
                .get_resource::<TrainingHistory>()
                .and_then(|h| h.history.rides.iter().find(|r| r.id == id).cloned());
            let Some(ride) = ride else { return };
            let name = ocw_client::recording::download_name(ride.start_unix_s);
            let downloads = world.resource::<crate::ui::Downloads>().clone();
            (downloads.0)(&name, "application/vnd.ant.fit", bytes);
            say(world, format!("downloading {name}"));
        }
        FitDone::Read {
            id,
            result: Ok(None),
        } => {
            // The browser cleared its storage: the entry stops naming it.
            let saved = world
                .get_resource_mut::<TrainingHistory>()
                .and_then(|mut h| {
                    let r = h.history.ride_mut(&id)?;
                    r.fit = None;
                    Some(h.save())
                });
            if let Some(Err(e)) = saved {
                warn!("training history: {e}");
            }
            crate::ui::web_refresh(world);
            say(
                world,
                "this ride's FIT file is no longer kept (the browser cleared its storage)".into(),
            );
        }
        FitDone::Read { id, result: Err(e) } => say(
            world,
            format!("the FIT file of ride {id} cannot be read: {e}"),
        ),
        FitDone::Removed { id, result } => match result {
            Ok(()) => info!("FIT file of ride {id} deleted"),
            Err(e) => warn!("FIT file of ride {id} not deleted: {e}"),
        },
    }
}

/// The export of a ride's FIT file: the page's copy at once, else a read
/// of the kept file (downloaded when it arrives).
pub fn export(world: &mut World, ride: &RideEntry) -> Result<String, String> {
    let name = ocw_client::recording::download_name(ride.start_unix_s);
    if let Some(bytes) = world
        .get_resource::<TrainingHistory>()
        .and_then(|h| h.fits.get(&ride.id).cloned())
    {
        let downloads = world.resource::<crate::ui::Downloads>().clone();
        (downloads.0)(&name, "application/vnd.ant.fit", bytes);
        return Ok(format!("downloading {name}"));
    }
    let kept = ride.fit.is_some()
        && world
            .get_resource::<FitKeeper>()
            .is_some_and(|k| k.fetch(&ride.id));
    if kept {
        Ok(format!(
            "reading {name} from this browser's storage; it downloads in a moment"
        ))
    } else {
        Err("this ride's FIT file is not kept in this browser".into())
    }
}

/// Deleting a ride: its kept FIT file goes too (the page's copy goes with
/// the entry).
pub fn delete(world: &mut World, ride: &RideEntry) -> Result<String, String> {
    let gone = ride.fit.is_some()
        && world
            .get_resource::<FitKeeper>()
            .is_some_and(|k| k.forget(&ride.id));
    Ok(if gone {
        "its FIT file is deleted from this browser's storage".into()
    } else {
        String::new()
    })
}

/// The rides whose FIT file can be exported: the page's, and those kept in
/// the storage (not in the fallback).
pub fn exportable(world: &World) -> std::collections::BTreeSet<String> {
    let persistent = world
        .get_resource::<FitKeeper>()
        .is_some_and(FitKeeper::persistent);
    world
        .get_resource::<TrainingHistory>()
        .map(|h| {
            let mut ids: std::collections::BTreeSet<String> = h.fits.keys().cloned().collect();
            if persistent {
                ids.extend(
                    h.history
                        .rides
                        .iter()
                        .filter(|r| r.fit.is_some())
                        .map(|r| r.id.clone()),
                );
            }
            ids
        })
        .unwrap_or_default()
}

/// A backend in memory with the browser's behaviour, answering at once:
/// the tests' (and a model of OPFS's: files by name, a probe that may
/// fail, writes that may fail).
#[derive(Clone, Default)]
pub struct MockFits(pub Arc<Mutex<MockState>>);

#[derive(Default)]
pub struct MockState {
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
    /// The probe's error (no OPFS).
    pub probe_error: Option<String>,
    /// Writes fail with this (the quota).
    pub write_error: Option<String>,
    pub done: Vec<FitDone>,
}

impl FitBackend for MockFits {
    fn probe(&self) {
        let mut s = self.0.lock().expect("not poisoned");
        let r = s.probe_error.clone().map_or(Ok(()), Err);
        s.done.push(FitDone::Probed(r));
    }
    fn save(&self, id: &str, bytes: Vec<u8>) {
        let mut s = self.0.lock().expect("not poisoned");
        let result = match s.write_error.clone() {
            Some(e) => Err(e),
            None => {
                s.files.insert(file_name(id), bytes);
                Ok(())
            }
        };
        s.done.push(FitDone::Saved {
            id: id.into(),
            result,
        });
    }
    fn read(&self, id: &str) {
        let mut s = self.0.lock().expect("not poisoned");
        let result = Ok(s.files.get(&file_name(id)).cloned());
        s.done.push(FitDone::Read {
            id: id.into(),
            result,
        });
    }
    fn remove(&self, id: &str) {
        let mut s = self.0.lock().expect("not poisoned");
        s.files.remove(&file_name(id));
        s.done.push(FitDone::Removed {
            id: id.into(),
            result: Ok(()),
        });
    }
    fn take_done(&self) -> Vec<FitDone> {
        std::mem::take(&mut self.0.lock().expect("not poisoned").done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_ui::settings::memory::MemoryStore;

    fn ride(id: &str, start: i64) -> RideEntry {
        RideEntry {
            id: id.into(),
            start_unix_s: start,
            distance_m: 5000.0,
            elapsed_s: 600.0,
            ..RideEntry::default()
        }
    }

    /// A page of the browser app's history part: the shared history with
    /// the browser's hooks over `backend`, the history in `store` (the
    /// page's localStorage), downloads into `log`.
    fn page(
        backend: Option<Box<dyn FitBackend>>,
        store: &MemoryStore,
        log: crate::ui::DownloadLog,
    ) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins((
                ocw_ui::settings::SettingsPlugin,
                ocw_ui::history::HistoryPlugin,
            ))
            .insert_resource(crate::ui::Downloads::logged(log))
            .insert_resource(crate::ui::history_hooks())
            .insert_resource(FitKeeper::new(backend))
            .add_systems(Update, apply_fits);
        app.update();
        let rider = ocw_client::riders::Rider::default().name;
        app.insert_resource(TrainingHistory::load(&rider, Some(Box::new(store.clone()))));
        crate::ui::web_refresh(app.world_mut());
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        let n = app.world().resource::<ConsoleState>().log().len();
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.update();
        app.world().resource::<ConsoleState>().log()[n..].join("\n")
    }

    fn save(app: &mut App, id: &str, start: i64, bytes: Vec<u8>) {
        ocw_ui::history::add_ride(app.world_mut(), ride(id, start), Some(bytes.clone())).unwrap();
        app.world().resource::<FitKeeper>().keep(id, bytes);
        app.update();
    }

    /// OPFS (the mock): a saved ride's FIT file is written, the entry
    /// names it, and after a reload – a new page, the same storage – the
    /// ride still exports it (read, then downloaded); deleting the ride
    /// deletes the file.
    #[test]
    fn fit_files_survive_a_reload_in_opfs() {
        let mock = MockFits::default();
        let store = MemoryStore::default();
        let log = crate::ui::DownloadLog::default();
        let mut app = page(Some(Box::new(mock.clone())), &store, log.clone());
        assert_eq!(app.world().resource::<FitKeeper>().keeping, Keeping::Opfs);
        save(&mut app, "2026-11-19_10-00-00", 1_794_211_200, vec![7; 40]);
        save(&mut app, "2026-11-19_12-00-00", 1_794_218_400, vec![8; 50]);
        assert_eq!(mock.0.lock().unwrap().files.len(), 2);
        let h = app.world().resource::<TrainingHistory>();
        assert_eq!(
            h.history.rides[0].fit.as_deref(),
            Some("2026-11-19_10-00-00.fit")
        );
        assert!(h.kept_note.is_none());
        // The reload: the page's memory is gone, the storage is not.
        let mut app = page(Some(Box::new(mock.clone())), &store, log.clone());
        {
            let h = app.world().resource::<TrainingHistory>();
            assert!(h.fits.is_empty());
            assert_eq!(h.history.rides.len(), 2);
        }
        let i = ocw_ui::history::info(app.world());
        assert_eq!(i.exportable.len(), 2, "both kept");
        let out = run(&mut app, "history.export 2");
        assert!(out.contains("from this browser's storage"), "{out}");
        let got = log.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert!(got[0].0.starts_with("ocw-ride-") && got[0].0.ends_with(".fit"));
        assert_eq!(got[0].2, vec![7; 40]);
        // Deleting the ride deletes its file.
        let out = run(&mut app, "history.delete 1");
        assert!(out.contains("deleted from this browser's storage"), "{out}");
        assert_eq!(
            mock.0.lock().unwrap().files.keys().collect::<Vec<_>>(),
            ["2026-11-19_10-00-00.fit"]
        );
        let again = TrainingHistory::load("x", Some(Box::new(store.clone())));
        assert_eq!(again.history.rides.len(), 1);
    }

    /// A file the browser has cleared: the export says so and the entry
    /// stops naming it.
    #[test]
    fn a_cleared_file_is_forgotten() {
        let mock = MockFits::default();
        let store = MemoryStore::default();
        let log = crate::ui::DownloadLog::default();
        let mut app = page(Some(Box::new(mock.clone())), &store, log.clone());
        save(&mut app, "a", 1_794_211_200, vec![1; 10]);
        let mut app = page(Some(Box::new(mock.clone())), &store, log.clone());
        mock.0.lock().unwrap().files.clear();
        let out = run(&mut app, "history.export 1");
        assert!(out.contains("no longer kept"), "{out}");
        assert!(log.lock().unwrap().is_empty());
        assert!(
            app.world().resource::<TrainingHistory>().history.rides[0]
                .fit
                .is_none()
        );
        assert!(ocw_ui::history::info(app.world()).exportable.is_empty());
        assert!(run(&mut app, "history.export 1").contains("not kept"));
    }

    /// The fallback: no OPFS (or a probe that fails) keeps the FIT files
    /// in the page's memory – exported while the page is open, not after
    /// a reload – and the history's top page says so; a failed write
    /// keeps the page's copy.
    #[test]
    fn without_opfs_the_page_keeps_them_until_it_closes() {
        for backend in [
            None,
            Some(Box::new(MockFits(Arc::new(Mutex::new(MockState {
                probe_error: Some("SecurityError: private window".into()),
                ..MockState::default()
            })))) as Box<dyn FitBackend>),
        ] {
            let store = MemoryStore::default();
            let log = crate::ui::DownloadLog::default();
            let mut app = page(backend, &store, log.clone());
            assert!(matches!(
                app.world().resource::<FitKeeper>().keeping,
                Keeping::Memory(_)
            ));
            save(&mut app, "a", 1_794_211_200, vec![3; 12]);
            let i = ocw_ui::history::info(app.world());
            assert_eq!(i.kept_note.as_deref(), Some(MEMORY_NOTE));
            let status =
                ocw_client::menu::history::status(ocw_client::menu::pages::Page::History, &i);
            assert_eq!(status, MEMORY_NOTE);
            assert!(run(&mut app, "history.export 1").contains("downloading"));
            assert_eq!(log.lock().unwrap().len(), 1);
            assert!(
                app.world().resource::<TrainingHistory>().history.rides[0]
                    .fit
                    .is_none()
            );
            let mut app = page(None, &store, log.clone());
            assert!(ocw_ui::history::info(app.world()).exportable.is_empty());
            assert!(run(&mut app, "history.export 1").contains("not kept"));
        }
        // OPFS there, the write refused (the quota): this page's copy.
        let mock = MockFits(Arc::new(Mutex::new(MockState {
            write_error: Some("QuotaExceededError".into()),
            ..MockState::default()
        })));
        let store = MemoryStore::default();
        let log = crate::ui::DownloadLog::default();
        let mut app = page(Some(Box::new(mock)), &store, log.clone());
        save(&mut app, "a", 1_794_211_200, vec![3; 12]);
        assert!(
            app.world().resource::<TrainingHistory>().history.rides[0]
                .fit
                .is_none()
        );
        assert!(run(&mut app, "history.export 1").contains("downloading"));
    }
}
