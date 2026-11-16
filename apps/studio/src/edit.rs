//! The studio's side of the document core (spec §14 Phase 16, 16A): the
//! open documents live in an `ocw_edit::Session`; the panels, the menus,
//! the keys and the console only **send commands** ([`StudioCommand`],
//! queued in [`Pending`]) that [`apply`] carries out – the way the game's
//! UI drives `ClientCore`. Undo / redo act on the active document, the
//! autosave writes every changed document to
//! `<data dir>/ocw/studio/autosave/` once it has been left alone for a few
//! seconds (and at quit), and opening a world offers the autosave an
//! earlier session left for it.

use std::path::PathBuf;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_edit::world::{PackageRef, View, WorldDoc, WorldOp, check_views};
use ocw_edit::{AutosaveStore, DocId, Document, Session, autosave_name};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_sim::god::FlightModel;
use ocw_worldstore::details::WorldDetails;

use crate::flight::StudioFlight;

/// `<data dir>/ocw/studio/autosave` (`OCW_DATA_DIR` overrides
/// `<data dir>/ocw`, as in the game).
pub fn autosave_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("ocw"),
    };
    Some(base.join("studio").join("autosave"))
}

/// An autosave an earlier session left for the open world.
#[derive(Debug, Clone, PartialEq)]
pub struct Restorable {
    pub name: String,
    pub json: String,
    /// "3 views, 12 steps of history".
    pub summary: String,
}

/// The studio's documents and their autosave.
#[derive(Resource)]
pub struct Studio {
    pub session: Session,
    /// `None`: no data directory (nothing is autosaved).
    pub store: Option<Box<dyn AutosaveStore>>,
    /// Where the store writes, for the status line.
    pub store_dir: Option<PathBuf>,
    /// The World workspace's document.
    pub world_doc: Option<DocId>,
    pub restorable: Option<Restorable>,
    /// The last message (status line).
    pub status: String,
}

impl Studio {
    pub fn new(store: Option<Box<dyn AutosaveStore>>, store_dir: Option<PathBuf>) -> Self {
        Self {
            session: Session::default(),
            store,
            store_dir,
            world_doc: None,
            restorable: None,
            status: String::new(),
        }
    }

    pub fn world(&self) -> Option<&Document<WorldDoc>> {
        self.session.get(self.world_doc?)
    }
}

/// What the UI asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum StudioCommand {
    Undo,
    Redo,
    /// Saves the camera's view (a name, else "View N").
    AddView(Option<String>),
    RenameView(u32, String),
    /// The camera's view replaces the saved one.
    UpdateView(u32),
    DeleteView(u32),
    GoToView(u32),
    /// Ends a merging edit (a name field lost its focus).
    Seal,
    Open(String),
    RestoreAutosave,
    DiscardAutosave,
    /// Flies the camera above a package point (x, z), at its height above
    /// the ground or this one.
    FlyTo([f64; 2], Option<f64>),
    FlightModel(FlightModel),
}

/// The commands sent this frame, carried out by [`run_pending`].
#[derive(Resource, Debug, Default)]
pub struct Pending(pub Vec<StudioCommand>);

impl Pending {
    pub fn send(&mut self, c: StudioCommand) {
        self.0.push(c);
    }
}

fn now(world: &World) -> f64 {
    world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64())
}

/// The World document's validators.
fn world_validators() -> Vec<ocw_edit::Validator<WorldDoc>> {
    vec![Box::new(check_views)]
}

/// Opens the World workspace's document for a package just shown (the
/// previous one is closed – its changes are in its autosave) and looks
/// for an autosave of it.
pub fn open_world_doc(world: &mut World, spec: &str, details: &WorldDetails, now_s: f64) {
    let source = std::fs::canonicalize(spec)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| spec.to_string());
    let Some(mut studio) = world.get_resource_mut::<Studio>() else {
        return;
    };
    if let Some(old) = studio.world_doc.take() {
        // Its last changes go to its autosave first.
        let studio = &mut *studio;
        if let Some(store) = studio.store.as_deref_mut() {
            studio.session.autosave(store, now_s, 0.0);
        }
        studio.session.close(old);
    }
    let package = PackageRef {
        spec: spec.into(),
        id: details.id.clone(),
        version: details.version,
    };
    let name = details.region.clone();
    let id = studio.session.open(|id| {
        let mut d = Document::new(id, name, WorldDoc::new(package)).with_source(source.clone());
        for v in world_validators() {
            d = d.with_validator(v);
        }
        d
    });
    studio.world_doc = Some(id);
    let record = autosave_name("world", &source);
    studio.restorable = studio
        .store
        .as_ref()
        .and_then(|s| s.read(&record).ok().flatten())
        .and_then(|json| {
            let r: ocw_edit::AutosaveRecord<WorldDoc, WorldOp> =
                ocw_edit::AutosaveRecord::from_json(&json).ok()?;
            let steps = r.history.undo.len();
            Some(Restorable {
                name: record.clone(),
                summary: format!("{} views, {steps} steps of history", r.content.views.len()),
                json,
            })
        });
}

/// Carries out one command; the status line's message or an error.
pub fn apply(world: &mut World, cmd: StudioCommand) -> Result<String, String> {
    let t = now(world);
    let camera = {
        let f = &world.resource::<StudioFlight>().flight;
        (f.pos, f.view_yaw(), f.view_pitch())
    };
    match cmd {
        StudioCommand::Open(spec) => crate::world::open(world, spec.trim()),
        StudioCommand::FlyTo(p, height) => {
            let ground = world
                .get_resource::<ocw_render::Ground>()
                .and_then(|g| (g.0)(p[0], p[1]))
                .unwrap_or(0.0);
            let mut f = world.resource_mut::<StudioFlight>();
            let above = height.unwrap_or(f.height_m.max(20.0));
            let (yaw, pitch) = (f.flight.view_yaw(), f.flight.view_pitch());
            f.jump([p[0], ground + above, p[1]], yaw, pitch);
            Ok(format!("flying at {:.0}, {:.0}", p[0], p[1]))
        }
        StudioCommand::FlightModel(m) => {
            let mut f = world.resource_mut::<StudioFlight>();
            f.flight.model = m;
            f.flight.stop();
            Ok(format!("flight: {m:?}"))
        }
        StudioCommand::GoToView(id) => {
            let view = world
                .resource::<Studio>()
                .world()
                .and_then(|d| d.content().view(id).cloned())
                .ok_or_else(|| format!("no view {id}"))?;
            world
                .resource_mut::<StudioFlight>()
                .jump(view.pos, view.yaw, view.pitch);
            Ok(format!("at view '{}'", view.name))
        }
        other => {
            let mut studio = world.resource_mut::<Studio>();
            edit(&mut studio, other, camera, t)
        }
    }
}

/// The document commands (pure on the [`Studio`]; `camera`: position,
/// yaw, pitch of the 3D view).
pub fn edit(
    studio: &mut Studio,
    cmd: StudioCommand,
    camera: ([f64; 3], f64, f64),
    now_s: f64,
) -> Result<String, String> {
    let doc = studio.world_doc;
    let world_op = |studio: &mut Studio, op: WorldOp| -> Result<(), String> {
        let id = doc.ok_or("no world is open")?;
        studio
            .session
            .execute::<WorldDoc>(id, op, now_s)
            .map_err(|e| e.to_string())
    };
    match cmd {
        StudioCommand::Undo => {
            let label = studio
                .session
                .active()
                .and_then(|d| d.undo_label().map(String::from));
            match studio.session.undo(now_s).map_err(|e| e.to_string())? {
                true => Ok(format!("undone: {}", label.unwrap_or_default())),
                false => Ok("nothing to undo".into()),
            }
        }
        StudioCommand::Redo => {
            let label = studio
                .session
                .active()
                .and_then(|d| d.redo_label().map(String::from));
            match studio.session.redo(now_s).map_err(|e| e.to_string())? {
                true => Ok(format!("redone: {}", label.unwrap_or_default())),
                false => Ok("nothing to redo".into()),
            }
        }
        StudioCommand::AddView(name) => {
            let name = match name.filter(|n| !n.trim().is_empty()) {
                Some(n) => n,
                None => studio
                    .world()
                    .map_or_else(|| "View 1".into(), |d| d.content().new_view_name()),
            };
            let (pos, yaw, pitch) = camera;
            world_op(
                studio,
                WorldOp::add(View {
                    id: 0,
                    name: name.clone(),
                    pos,
                    yaw,
                    pitch,
                }),
            )?;
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok(format!("view '{name}' saved"))
        }
        StudioCommand::RenameView(id, name) => {
            world_op(studio, WorldOp::Rename { id, name })?;
            Ok(String::new())
        }
        StudioCommand::UpdateView(id) => {
            let (pos, yaw, pitch) = camera;
            world_op(
                studio,
                WorldOp::Move {
                    id,
                    pos,
                    yaw,
                    pitch,
                },
            )?;
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok("view moved to the camera".into())
        }
        StudioCommand::DeleteView(id) => {
            world_op(studio, WorldOp::Remove(id))?;
            Ok("view deleted".into())
        }
        StudioCommand::Seal => {
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok(String::new())
        }
        StudioCommand::RestoreAutosave => {
            let r = studio.restorable.take().ok_or("no autosave to restore")?;
            let id = studio
                .session
                .restore::<WorldDoc>(&r.json, world_validators())
                .map_err(|e| e.to_string())?;
            let same = match (
                doc.and_then(|d| studio.session.get::<WorldDoc>(d)),
                studio.session.get::<WorldDoc>(id),
            ) {
                (Some(a), Some(b)) => a.content().package.id == b.content().package.id,
                _ => true,
            };
            if let Some(old) = doc {
                studio.session.close(old);
            }
            studio.world_doc = Some(id);
            Ok(if same {
                format!("restored: {}", r.summary)
            } else {
                format!(
                    "restored: {} (saved for another package version)",
                    r.summary
                )
            })
        }
        StudioCommand::DiscardAutosave => {
            let r = studio.restorable.take().ok_or("no autosave to discard")?;
            if let Some(s) = studio.store.as_mut() {
                s.remove(&r.name)?;
            }
            Ok("autosave discarded".into())
        }
        StudioCommand::Open(_)
        | StudioCommand::FlyTo(..)
        | StudioCommand::FlightModel(_)
        | StudioCommand::GoToView(_) => Err("not a document command".into()),
    }
}

/// Carries out the commands sent this frame.
fn run_pending(world: &mut World) {
    let cmds = std::mem::take(&mut world.resource_mut::<Pending>().0);
    for c in cmds {
        let msg = match apply(world, c) {
            Ok(m) => m,
            Err(e) => format!("error: {e}"),
        };
        if !msg.is_empty() {
            info!("{msg}");
            world.resource_mut::<Studio>().status = msg;
        }
    }
}

/// Writes the autosaves that are due; at quit all changed documents.
fn autosave(mut studio: ResMut<Studio>, time: Res<Time<Real>>, mut exits: MessageReader<AppExit>) {
    let quitting = exits.read().count() > 0;
    let delay = if quitting {
        0.0
    } else {
        ocw_edit::autosave::DEFAULT_DELAY_S
    };
    let studio = &mut *studio;
    let Some(store) = studio.store.as_deref_mut() else {
        return;
    };
    let (written, errors) = studio
        .session
        .autosave(store, time.elapsed_secs_f64(), delay);
    for e in &errors {
        warn!("autosave: {e}");
    }
    if let Some(e) = errors.first() {
        studio.status = format!("autosave failed: {e}");
    } else if !written.is_empty() {
        info!("autosaved {}", written.join(", "));
    }
}

/// Ctrl + Z, Ctrl + Shift + Z and Ctrl + Y (not while a text field has
/// the keyboard: it undoes its own typing).
fn shortcut_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    focus: Res<crate::flight::UiFocus>,
    mut pending: ResMut<Pending>,
) {
    let Some(k) = keys else {
        return;
    };
    if focus.keyboard {
        return;
    }
    let ctrl = k.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if ctrl && k.just_pressed(KeyCode::KeyZ) {
        pending.send(if shift {
            StudioCommand::Redo
        } else {
            StudioCommand::Undo
        });
    } else if ctrl && k.just_pressed(KeyCode::KeyY) {
        pending.send(StudioCommand::Redo);
    }
}

pub struct EditPlugin;

impl Plugin for EditPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pending>()
            .add_systems(Update, (shortcut_keys, run_pending).chain())
            .add_systems(Last, autosave);
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("studio")
                .about("OCW Studio: undo, redo, views, open, fly")
                .subcommand_required(true)
                .subcommand(Command::new("undo").about("Undo the last edit of the active document"))
                .subcommand(Command::new("redo").about("Redo the last undone edit"))
                .subcommand(
                    Command::new("status").about("The documents, their history and autosave"),
                )
                .subcommand(
                    Command::new("open")
                        .about("Open a world package")
                        .arg(Arg::new("path").required(true)),
                )
                .subcommand(
                    Command::new("fly")
                        .about("Fly the camera above a package point")
                        .arg(
                            Arg::new("x")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(
                            Arg::new("z")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(Arg::new("height").value_parser(value_parser!(f64))),
                )
                .subcommand(
                    Command::new("view")
                        .about("Saved views of the World workspace")
                        .subcommand_required(true)
                        .subcommand(Command::new("add").arg(Arg::new("name").num_args(0..)))
                        .subcommand(Command::new("list"))
                        .subcommand(
                            Command::new("go").arg(
                                Arg::new("id")
                                    .required(true)
                                    .value_parser(value_parser!(u32)),
                            ),
                        )
                        .subcommand(
                            Command::new("delete").arg(
                                Arg::new("id")
                                    .required(true)
                                    .value_parser(value_parser!(u32)),
                            ),
                        ),
                ),
            studio_command,
        ));
    }
}

fn studio_command(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let cmd = match m.subcommand() {
        Some(("undo", _)) => StudioCommand::Undo,
        Some(("redo", _)) => StudioCommand::Redo,
        Some(("status", _)) => return Ok(status_lines(world.resource::<Studio>()).join("\n")),
        Some(("open", a)) => {
            StudioCommand::Open(a.get_one::<String>("path").cloned().unwrap_or_default())
        }
        Some(("fly", a)) => StudioCommand::FlyTo(
            [
                *a.get_one::<f64>("x").expect("required"),
                *a.get_one::<f64>("z").expect("required"),
            ],
            a.get_one::<f64>("height").copied(),
        ),
        Some(("view", v)) => match v.subcommand() {
            Some(("add", a)) => {
                let name: Vec<String> = a
                    .get_many::<String>("name")
                    .map(|n| n.cloned().collect())
                    .unwrap_or_default();
                StudioCommand::AddView((!name.is_empty()).then(|| name.join(" ")))
            }
            Some(("list", _)) => {
                let s = world.resource::<Studio>();
                let doc = s.world().ok_or("no world is open")?;
                return Ok(doc
                    .content()
                    .views
                    .iter()
                    .map(|v| {
                        format!(
                            "{} '{}' at {:.0}, {:.0}, {:.0}",
                            v.id, v.name, v.pos[0], v.pos[1], v.pos[2]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            Some(("go", a)) => StudioCommand::GoToView(*a.get_one::<u32>("id").expect("required")),
            Some(("delete", a)) => {
                StudioCommand::DeleteView(*a.get_one::<u32>("id").expect("required"))
            }
            _ => return Err("studio view add|list|go|delete".into()),
        },
        _ => return Err("studio undo|redo|status|open|fly|view".into()),
    };
    let msg = apply(world, cmd)?;
    if !msg.is_empty() {
        world.resource_mut::<Studio>().status = msg.clone();
    }
    Ok(msg)
}

/// `studio status`: one line per document.
pub fn status_lines(s: &Studio) -> Vec<String> {
    let mut out: Vec<String> = s
        .session
        .docs()
        .map(|d| {
            format!(
                "{}{} '{}'{}: undo {}, redo {}, {} findings",
                if s.session.active_id() == Some(d.id()) {
                    "* "
                } else {
                    "  "
                },
                d.kind(),
                d.name(),
                if d.is_dirty() { " (changed)" } else { "" },
                d.undo_label().unwrap_or("-"),
                d.redo_label().unwrap_or("-"),
                d.findings().len()
            )
        })
        .collect();
    out.push(match &s.store_dir {
        Some(d) => format!("autosave: {}", d.display()),
        None => "autosave: off (no data directory)".into(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_edit::MemoryStore;

    fn studio() -> Studio {
        let mut s = Studio::new(Some(Box::new(MemoryStore::default())), None);
        let id = s.session.open(|id| {
            Document::new(id, "Testland", WorldDoc::new(PackageRef::default()))
                .with_source("/data/testland.ocwpkg")
                .with_validator(Box::new(check_views))
        });
        s.world_doc = Some(id);
        s
    }

    const CAM: ([f64; 3], f64, f64) = ([10.0, 500.0, 20.0], 0.3, -0.4);

    #[test]
    fn views_are_saved_from_the_camera_and_undone() {
        let mut s = studio();
        assert_eq!(
            edit(&mut s, StudioCommand::AddView(None), CAM, 0.0).unwrap(),
            "view 'View 1' saved"
        );
        edit(
            &mut s,
            StudioCommand::AddView(Some("Pass".into())),
            CAM,
            0.5,
        )
        .unwrap();
        let views = &s.world().unwrap().content().views;
        assert_eq!(
            views.len(),
            2,
            "the second add did not merge into the first"
        );
        assert_eq!(views[0].pos, CAM.0);
        assert_eq!(views[0].yaw, CAM.1);
        for (k, n) in ["P", "Pa", "Par"].iter().enumerate() {
            edit(
                &mut s,
                StudioCommand::RenameView(1, (*n).into()),
                CAM,
                1.0 + 0.1 * k as f64,
            )
            .unwrap();
        }
        edit(&mut s, StudioCommand::Seal, CAM, 1.5).unwrap();
        assert_eq!(s.world().unwrap().undo_label(), Some("Rename view"));
        assert_eq!(
            edit(&mut s, StudioCommand::Undo, CAM, 2.0).unwrap(),
            "undone: Rename view"
        );
        assert_eq!(s.world().unwrap().content().views[0].name, "View 1");
        edit(&mut s, StudioCommand::Undo, CAM, 3.0).unwrap();
        edit(&mut s, StudioCommand::Undo, CAM, 4.0).unwrap();
        assert_eq!(
            edit(&mut s, StudioCommand::Undo, CAM, 5.0).unwrap(),
            "nothing to undo"
        );
        assert!(!s.session.any_dirty());
        assert_eq!(
            edit(&mut s, StudioCommand::Redo, CAM, 6.0).unwrap(),
            "redone: Add view"
        );
        assert!(edit(&mut s, StudioCommand::DeleteView(9), CAM, 7.0).is_err());
        assert!(
            edit(&mut s, StudioCommand::GoToView(1), CAM, 7.0).is_err(),
            "a Bevy command"
        );
        let lines = status_lines(&s);
        assert!(
            lines[0].starts_with("* world 'Testland' (changed): undo Add view"),
            "{lines:?}"
        );
    }

    #[test]
    fn an_autosave_is_offered_restored_or_discarded() {
        let mut s = studio();
        edit(
            &mut s,
            StudioCommand::AddView(Some("Lake".into())),
            CAM,
            0.0,
        )
        .unwrap();
        let store = s.store.as_deref_mut().unwrap();
        let (written, _) = s.session.autosave(store, 10.0, 5.0);
        let json = s
            .store
            .as_ref()
            .unwrap()
            .read(&written[0])
            .unwrap()
            .unwrap();
        // A new session of the same package: the autosave is offered.
        let mut t = studio();
        t.restorable = Some(Restorable {
            name: written[0].clone(),
            json,
            summary: "1 views".into(),
        });
        assert_eq!(
            edit(&mut t, StudioCommand::RestoreAutosave, CAM, 0.0).unwrap(),
            "restored: 1 views"
        );
        assert_eq!(t.session.len(), 1, "the fresh document was replaced");
        assert_eq!(t.world().unwrap().content().views[0].name, "Lake");
        edit(&mut t, StudioCommand::Undo, CAM, 1.0).unwrap();
        assert!(
            t.world().unwrap().content().views.is_empty(),
            "with its history"
        );
        assert!(edit(&mut t, StudioCommand::RestoreAutosave, CAM, 2.0).is_err());
        // Discarding removes it from the store.
        let mut u = studio();
        u.store
            .as_deref_mut()
            .unwrap()
            .write("world-x.json", "{}")
            .unwrap();
        u.restorable = Some(Restorable {
            name: "world-x.json".into(),
            json: "{}".into(),
            summary: String::new(),
        });
        edit(&mut u, StudioCommand::DiscardAutosave, CAM, 0.0).unwrap();
        assert_eq!(
            u.store.as_ref().unwrap().list().unwrap(),
            Vec::<String>::new()
        );
    }
}
