//! The trust dialog and the trusted world keys in both apps (spec §4.7
//! *User keys*, §12; §14 Phase 16, 16C *Signing and trust*, Q2).
//!
//! - [`TrustedKeys`]: the user keys trusted here and their store – the
//!   app inserts it with a [`TextStore`]: the desktop
//!   `<data dir>/ocw/trusted-keys.json` (the same file the desktop's and
//!   the server's [`ocw_worldstore::trust::TrustConfig`] read), the
//!   browser `localStorage`. A corrupt store is shown on the page and not
//!   overwritten until the list changes.
//! - [`TrustDialog`]: a pending question. An app that is about to load a
//!   world signed by a user key nobody here trusts calls
//!   [`TrustDialog::ask`] with the line to run once the user agreed (the
//!   ride switch, following the host, streaming on) and the line to run
//!   when they decline (leaving a shared ride). The menu opens at the
//!   dialog (`ocw_client::menu::model::Page::ConfirmTrust`, Cancel
//!   focused); leaving it any other way than *Trust this key* or *Load
//!   once* – Cancel, Back, Esc, B, closing the menu – declines.
//! - `world trust list | add KEY | remove FINGERPRINT | show | accept |
//!   once | decline` ([`command`], [`run`]): each app's `world` command
//!   has it as a subcommand.

use bevy::prelude::*;
use clap::{Arg, ArgMatches, Command};
use ocw_client::menu::model::{Action, Page};
use ocw_client::menu::trust as model;
use ocw_render::console::ConsoleState;
use ocw_world::manifest::{RootKey, TrustStore};
use ocw_worldstore::userkeys::{self, TrustRequest, TrustedKey, UserKeys};

use crate::menu::MenuState;
use crate::settings::TextStore;

/// The trusted user keys and where they are kept.
#[derive(Resource, Default)]
pub struct TrustedKeys {
    pub keys: UserKeys,
    /// `None`: not saved (tests, a page without storage).
    pub store: Option<Box<dyn TextStore>>,
    /// Why the store could not be read (none are trusted then).
    pub warning: Option<String>,
    /// The roots that are not user keys: they cannot be added.
    pub builtin: TrustStore,
}

impl TrustedKeys {
    /// Reads `store` (a corrupt one: empty, with a warning).
    pub fn load(store: Option<Box<dyn TextStore>>, builtin: TrustStore) -> Self {
        let (keys, warning) = match &store {
            Some(s) => UserKeys::load(s.load()),
            None => (UserKeys::default(), None),
        };
        Self {
            keys,
            store,
            warning,
            builtin,
        }
    }

    fn save(&mut self) -> Result<(), String> {
        if let Some(s) = self.store.as_mut() {
            s.save(&self.keys.to_json())?;
        }
        // The list was written as it is now: the old text is gone.
        self.warning = None;
        Ok(())
    }

    /// Trusts `root` and saves the list.
    pub fn trust(
        &mut self,
        root: &RootKey,
        name: Option<&str>,
        note: Option<&str>,
        now_unix_s: i64,
    ) -> Result<TrustedKey, String> {
        let k = self.keys.add(root, name, note, now_unix_s, &self.builtin)?;
        if let Err(e) = self.save() {
            self.keys.keys.retain(|o| o.key_id != k.key_id);
            return Err(format!("not saved: {e}"));
        }
        Ok(k)
    }

    /// Removes the key `fingerprint` names and saves the list; a *Load
    /// once* acceptance of it ends too.
    pub fn remove(&mut self, fingerprint: &str) -> Result<TrustedKey, String> {
        let k = self.keys.remove(fingerprint)?;
        userkeys::forget_once(k.key_id);
        if let Err(e) = self.save() {
            self.keys.keys.push(k);
            return Err(format!("not saved: {e}"));
        }
        Ok(k)
    }

    /// The trusted keys as roots (the browser's trust store).
    pub fn roots(&self) -> Vec<RootKey> {
        self.keys.roots()
    }
}

/// A pending trust question.
#[derive(Resource, Default, Debug)]
pub struct TrustDialog {
    pub request: Option<TrustRequest>,
    /// The console line to run after *Trust this key* or *Load once*.
    pub then: Option<String>,
    /// The console line to run when the user declines.
    pub declined: Option<String>,
    /// A world the host of a shared ride switched to.
    pub shared: bool,
    /// The dialog has been on screen (so leaving it declines).
    shown: bool,
}

impl TrustDialog {
    /// Asks about `request`; `then` runs once the user agreed, `declined`
    /// when they did not. A newer question replaces an open one.
    pub fn ask(
        &mut self,
        request: TrustRequest,
        then: Option<String>,
        declined: Option<String>,
        shared: bool,
    ) {
        *self = Self {
            request: Some(request),
            then,
            declined,
            shared,
            shown: false,
        };
    }

    pub fn is_pending(&self) -> bool {
        self.request.is_some()
    }
}

/// Asks `request` in `world` (see [`TrustDialog::ask`]); the menu opens
/// at the dialog next frame. Returns a line for the console.
pub fn ask(
    world: &mut World,
    request: TrustRequest,
    then: Option<String>,
    declined: Option<String>,
    shared: bool,
) -> String {
    let line = format!(
        "{} ({}) is {}; key fingerprint {}. Check it with its owner, then choose in the dialog (or: world trust accept | once | decline)",
        request.world,
        request.id,
        request.claimed(),
        request.fingerprint()
    );
    world.init_resource::<TrustDialog>();
    world
        .resource_mut::<TrustDialog>()
        .ask(request, then, declined, shared);
    line
}

pub struct TrustPlugin;

impl Plugin for TrustPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrustedKeys>()
            .init_resource::<TrustDialog>()
            .add_systems(Update, show_dialog.after(crate::ViewExtrasSet));
    }
}

/// Opens the menu at a new question; a question whose dialog was left
/// without an answer is declined.
fn show_dialog(world: &mut World) {
    let Some(dialog) = world.get_resource::<TrustDialog>() else {
        return;
    };
    if !dialog.is_pending() {
        return;
    }
    let shown = dialog.shown;
    let Some(page) = world.get_resource::<MenuState>().map(|s| s.menu.page()) else {
        return;
    };
    let on_dialog = page == Some(Page::ConfirmTrust);
    if on_dialog {
        world.resource_mut::<TrustDialog>().shown = true;
    } else if shown {
        let msg = decline(world);
        world.resource_mut::<ConsoleState>().print(msg);
    } else {
        world.resource_mut::<MenuState>().queue(Action::TrustDialog);
    }
}

/// Declines the pending question: its decline line runs.
fn decline(world: &mut World) -> String {
    let mut d = world.resource_mut::<TrustDialog>();
    let Some(r) = d.request.take() else {
        return "no world is waiting for a decision".into();
    };
    let line = d.declined.take();
    d.then = None;
    let shared = d.shared;
    if let Some(line) = line {
        world.resource_mut::<ConsoleState>().queue(line);
    }
    format!(
        "not loaded: you did not trust the key of {} ({}){}",
        r.world,
        r.fingerprint().short(),
        if shared {
            "; leaving the shared ride"
        } else {
            ""
        }
    )
}

/// Agrees to the pending question (`remember`: trust the key, else load
/// once): its line runs, the menu closes.
fn agree(world: &mut World, remember: bool) -> Result<String, String> {
    let (request, then) = {
        let mut d = world.resource_mut::<TrustDialog>();
        let r = d
            .request
            .take()
            .ok_or("no world is waiting for a decision")?;
        d.declined = None;
        (r, d.then.take())
    };
    let fp = request.fingerprint();
    let what = if remember {
        world.init_resource::<TrustedKeys>();
        let now = ocw_render::stamp::wall_unix_s() as i64;
        let trusted = world
            .resource_mut::<TrustedKeys>()
            .trust(&request.root, None, None, now);
        match trusted {
            Ok(k) => format!(
                "trusted the key '{}' (unverified name), fingerprint {fp}",
                k.name
            ),
            // Trusted meanwhile, or a built-in root: load anyway.
            Err(e) if e.contains("already") || e.contains("built-in") => e,
            Err(e) => {
                world.resource_mut::<TrustDialog>().request = Some(request);
                return Err(format!("cannot trust the key: {e}"));
            }
        }
    } else {
        userkeys::accept_once(&request.root);
        format!(
            "loading {} once (the key {fp} is not remembered)",
            request.world
        )
    };
    if let Some(mut s) = world.get_resource_mut::<MenuState>() {
        s.queue(Action::Close);
    }
    if let Some(line) = then {
        world.resource_mut::<ConsoleState>().queue(line);
    }
    Ok(what)
}

/// The `trust` subcommand of the apps' `world` command.
pub fn command() -> Command {
    Command::new("trust")
        .about("World keys you trust (spec §4.7): list, add or remove user keys; answer the trust dialog")
        .subcommand_required(true)
        .subcommand(Command::new("list").about("The trusted user keys: name (as claimed), fingerprint, date added"))
        .subcommand(
            Command::new("add")
                .about("Trust a user key: its 64 hex digits, a .pub.json key file or a package / manifest that carries it")
                .arg(Arg::new("key").required(true).value_name("PUBKEY|FILE"))
                .arg(Arg::new("name").long("name").value_name("NAME").help("A name for it (default: the name it claims)"))
                .arg(Arg::new("note").long("note").value_name("NOTE")),
        )
        .subcommand(
            Command::new("remove")
                .about("Stop trusting a user key (by its fingerprint or 8+ of its first hex digits)")
                .arg(Arg::new("fingerprint").required(true).num_args(1..).value_name("FINGERPRINT")),
        )
        .subcommand(Command::new("show").about("The world waiting for a decision, with its key's fingerprint"))
        .subcommand(Command::new("accept").about("Trust the waiting world's key and load it"))
        .subcommand(Command::new("once").about("Load the waiting world once, without remembering its key"))
        .subcommand(Command::new("decline").about("Do not load the waiting world"))
}

/// The key a typed argument names: the key itself, or on the desktop a
/// file (a key file, a manifest, or a package's manifest).
fn typed_key(arg: &str) -> Result<RootKey, String> {
    #[cfg(not(target_arch = "wasm32"))]
    return userkeys::read_key(arg);
    #[cfg(target_arch = "wasm32")]
    return userkeys::parse_key(arg);
}

fn list_text(keys: &TrustedKeys) -> String {
    let mut out = String::new();
    if let Some(w) = &keys.warning {
        out.push_str(&format!("warning: {w}\n"));
    }
    if keys.keys.keys.is_empty() {
        out.push_str("no trusted user keys (official worlds need none)");
        return out;
    }
    out.push_str("trusted user keys (names as the keys claim them):");
    for (i, k) in keys.keys.keys.iter().enumerate() {
        out.push_str(&format!(
            "\n  {}. '{}'  {}  added {}{}",
            i + 1,
            k.name,
            k.fingerprint(),
            model::iso_date(k.added_unix_s),
            k.note
                .as_ref()
                .map_or(String::new(), |n| format!("  ({n})"))
        ));
    }
    out
}

/// Runs `world trust …`.
pub fn run(world: &mut World, m: &ArgMatches) -> Result<String, String> {
    world.init_resource::<TrustedKeys>();
    world.init_resource::<TrustDialog>();
    match m.subcommand() {
        Some(("list", _)) => Ok(list_text(world.resource::<TrustedKeys>())),
        Some(("add", a)) => {
            let arg = a.get_one::<String>("key").expect("required");
            let key = typed_key(arg)?;
            let now = ocw_render::stamp::wall_unix_s() as i64;
            let k = world.resource_mut::<TrustedKeys>().trust(
                &key,
                a.get_one::<String>("name").map(String::as_str),
                a.get_one::<String>("note").map(String::as_str),
                now,
            )?;
            Ok(format!(
                "trusted '{}' (unverified name), fingerprint {}",
                k.name,
                k.fingerprint()
            ))
        }
        Some(("remove", a)) => {
            let fp: Vec<String> = a
                .get_many::<String>("fingerprint")
                .expect("required")
                .cloned()
                .collect();
            let k = world.resource_mut::<TrustedKeys>().remove(&fp.join(" "))?;
            Ok(format!(
                "no longer trusted: '{}' ({}); its worlds are asked about again",
                k.name,
                k.fingerprint()
            ))
        }
        Some(("show", _)) => Ok(world
            .resource::<TrustDialog>()
            .request
            .as_ref()
            .map_or_else(
                || "no world is waiting for a decision".into(),
                model::request_text,
            )),
        Some(("accept", _)) => agree(world, true),
        Some(("once", _)) => agree(world, false),
        Some(("decline", _)) => {
            let shown = world.resource::<TrustDialog>().shown;
            let msg = decline(world);
            if shown
                && let Some(mut s) = world.get_resource_mut::<MenuState>()
                && s.menu.page() == Some(Page::ConfirmTrust)
            {
                s.queue(Action::Back);
            }
            Ok(msg)
        }
        _ => Err(
            "world trust list | add KEY | remove FINGERPRINT | show | accept | once | decline"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleCorePlugin, Permission};
    use ocw_render::menu::MenuView;
    use ocw_world::manifest::{Attribution, RootKeyPair};

    struct Memory(std::sync::Arc<std::sync::Mutex<Option<String>>>);
    impl TextStore for Memory {
        fn load(&self) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&mut self, text: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(text.into());
            Ok(())
        }
        fn describe(&self) -> String {
            "memory".into()
        }
    }

    fn request(seed: u8) -> TrustRequest {
        TrustRequest {
            world: "Ada's Alps".into(),
            id: "user/ada/alps".into(),
            version: 3,
            created_unix_s: 1_800_000_000,
            claimed_signer: "Ada Rider".into(),
            root: RootKeyPair::from_seed("Ada Rider", [seed; 32]).public(),
            // As many sources as a real package has: they must fit too.
            attributions: vec![Attribution::openstreetmap(), Attribution::srtm(), {
                let mut a = Attribution::srtm();
                a.source = "swisstopo SWISSIMAGE 10 cm orthophoto".into();
                a
            }],
        }
    }

    fn app(saved: &std::sync::Arc<std::sync::Mutex<Option<String>>>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(crate::UiPlatform(crate::Platform::Browser))
            .insert_resource(ocw_render::RideView::new(ocw_world::Route::test_loop()))
            .add_plugins((
                crate::settings::SettingsPlugin,
                crate::weather::WeatherPlugin,
                crate::timemachine::TimeMachinePlugin,
                crate::riders_page::RidersPagePlugin,
                crate::menu::MenuPlugin,
            ));
        app.insert_resource(TrustedKeys::load(
            Some(Box::new(Memory(saved.clone()))),
            TrustStore::default(),
        ));
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("world").subcommand(command()),
            |w, m| match m.subcommand() {
                Some(("trust", t)) => run(w, t),
                _ => Err("world trust".into()),
            },
        ));
        app.update();
        app
    }

    fn frames(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    #[test]
    fn the_dialog_opens_shows_the_key_and_trusts_it_on_purpose() {
        let saved = Default::default();
        let mut app = app(&saved);
        let r = request(91);
        let fp = r.fingerprint();
        ask(
            app.world_mut(),
            r,
            Some("echo-then".into()),
            Some("echo-declined".into()),
            false,
        );
        frames(&mut app, 3);
        let v = app.world().resource::<MenuView>().clone();
        assert!(v.open);
        assert_eq!(v.title, "Trust this world's key?");
        assert_eq!(v.items, ["Trust this key", "Load once", "Cancel"]);
        assert_eq!(v.focused, 2, "Cancel");
        assert_eq!(v.emphasis.join(" "), fp.to_string());
        assert_eq!(v.text[0], model::WARNING);
        assert_eq!(v.status, "Signed by 'Ada Rider' (unverified name):");
        // The ten-foot panel holds it all at 16:9 and 4:3.
        for aspect in [16.0 / 9.0, 4.0 / 3.0] {
            assert_eq!(
                ocw_render::menu::fitted_paragraphs(&v, aspect),
                v.text.len(),
                "{aspect}"
            );
        }
        // Trust: through the menu (up twice, Enter).
        for a in [Action::Up, Action::Up, Action::Confirm] {
            app.world_mut().resource_mut::<MenuState>().queue(a);
            frames(&mut app, 1);
        }
        frames(&mut app, 3);
        assert!(!app.world().resource::<TrustDialog>().is_pending());
        let keys = &app.world().resource::<TrustedKeys>().keys;
        assert_eq!(keys.keys.len(), 1);
        assert_eq!(keys.keys[0].fingerprint(), fp);
        // Saved to the store.
        assert!(
            saved
                .lock()
                .unwrap()
                .as_deref()
                .unwrap()
                .contains(&fp.to_string())
        );
        assert!(
            !app.world().resource::<MenuView>().open,
            "closed for the loading"
        );
        // The menu row lists it on Settings → Account → Trusted world keys.
        let rows = crate::menu::sub_rows(app.world(), ocw_client::menu::pages::Page::Account);
        assert!(rows.iter().any(|r| r.label == "Trusted world keys"));
    }

    #[test]
    fn leaving_the_dialog_declines_and_remove_forgets() {
        let saved = Default::default();
        let mut app = app(&saved);
        ask(
            app.world_mut(),
            request(92),
            None,
            Some("echo-declined".into()),
            true,
        );
        frames(&mut app, 3);
        assert!(app.world().resource::<MenuView>().open);
        assert!(
            app.world()
                .resource::<MenuView>()
                .text
                .iter()
                .any(|t| t.contains("shared ride"))
        );
        // Enter on the focused Cancel declines.
        app.world_mut()
            .resource_mut::<MenuState>()
            .queue(Action::Confirm);
        frames(&mut app, 3);
        assert!(!app.world().resource::<TrustDialog>().is_pending());
        assert!(app.world().resource::<TrustedKeys>().keys.keys.is_empty());
        // Load once: not remembered.
        let r = request(93);
        let id = r.root.key_id;
        ask(app.world_mut(), r, None, None, false);
        frames(&mut app, 3);
        app.world_mut()
            .resource_mut::<MenuState>()
            .queue(Action::Click(1));
        frames(&mut app, 3);
        assert!(userkeys::once_roots().iter().any(|k| k.key_id == id));
        assert!(app.world().resource::<TrustedKeys>().keys.keys.is_empty());
        userkeys::forget_once(id);
        // Console: add, list, remove.
        let pk = RootKeyPair::from_seed("Bo Builder", [94; 32]).public();
        let mut keys = app.world_mut().resource_mut::<TrustedKeys>();
        keys.trust(&pk, None, Some("club"), 0).unwrap();
        assert!(list_text(&keys).contains("'Bo Builder'"));
        keys.remove(&pk.fingerprint().to_string()).unwrap();
        assert!(keys.keys.keys.is_empty());
        // A corrupt store: a warning, none trusted, not overwritten.
        let bad: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Some("{ broken".into())));
        let k = TrustedKeys::load(Some(Box::new(Memory(bad.clone()))), TrustStore::default());
        assert!(k.warning.is_some() && k.keys.keys.is_empty());
        assert_eq!(bad.lock().unwrap().as_deref(), Some("{ broken"));
    }
}
