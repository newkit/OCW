//! The question before a package's world patch is loaded (spec 14 Phase
//! 16, 16C *Files*, 16H Q16 (b)): when the studio opens a package with a
//! patch file next to it – `<package>.ocwpatch`, or a stack
//! `<package>.<NNN>-<topic>.ocwpatch` / `<package>.2.ocwpatch` as
//! `world-builder` and `worlds/<region>/patches/` name them (16H Q9, Q10)
//! – it asks first, in the shared menu's dialog style (as the trust
//! dialog, 16H Q14): "This world has a patch: N changes (by …, date)" –
//! *Load it* (focused) / *Open without it*.
//!
//! - *Load it* is focused: the patch next to a package is most often the
//!   user's own earlier work, and Enter does what the studio did before
//!   the question. Back, Esc and B open without it; nothing on disk
//!   changes either way.
//! - The answer is **not remembered**: every open asks again. Scripts
//!   answer before with `--patch load|skip` ([`PatchChoice`], every open
//!   of the run), the console with `studio patch load | skip`.
//! - The order with the trust dialog: a package by a user key not trusted
//!   here asks about the key first (it is not opened before); the patch
//!   question comes once it is open.
//! - Opened without it, the first *Save patch* to the default file keeps
//!   the patch that was there as `<package>.ocwpatch.bak` (as
//!   `world-builder` keeps a replaced one); a stack loaded as one patch is
//!   saved with *Save patch as* (saving it over the first file would apply
//!   the later files twice).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use ocw_client::menu::model::{Action, Page};
use ocw_world::manifest::{PatchStatus, SignedManifest};
use ocw_world::patch::{EXTENSION, WorldPatch};

use crate::edit::{Studio, shown_path};

/// How the studio answers the patch question (`--patch`).
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PatchChoice {
    /// Ask in a dialog (the default).
    #[default]
    Ask,
    /// Load the patch without asking (scripts).
    Load,
    /// Open the package without its patch (scripts).
    Skip,
}

/// The patch files next to `package`: `<stem>.ocwpatch` first, then the
/// stack `<stem>.<part>.ocwpatch` in order (a numbered part by its number,
/// `<stem>.2.ocwpatch` before `<stem>.10.ocwpatch`, `<stem>.020-bridge`
/// as 20; the rest by name), as `scripts/world-patches.sh` stacks them.
pub fn patch_files(package: &Path) -> Vec<PathBuf> {
    let dir = package
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Some(stem) = package
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    let suffix = format!(".{EXTENSION}");
    let mut stack: Vec<(u64, String, PathBuf)> = Vec::new();
    let mut main = None;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(base) = name.strip_suffix(&suffix) else {
            continue;
        };
        if !e.path().is_file() {
            continue;
        }
        if base == stem {
            main = Some(e.path());
        } else if let Some(part) = base.strip_prefix(&format!("{stem}.")) {
            // `<stem>.<part>`, not another package's `<stem>-x`.
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            let n = digits.parse().unwrap_or(u64::MAX);
            stack.push((n, part.to_owned(), e.path()));
        }
    }
    stack.sort();
    main.into_iter()
        .chain(stack.into_iter().map(|(_, _, p)| p))
        .collect()
}

/// One patch file of the question.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    /// The patch, or why it cannot be loaded (broken, another package's).
    pub patch: Result<WorldPatch, String>,
    /// The file's date (UTC, `2026-11-20`), if the file system has one.
    pub date: Option<String>,
    /// The package's manifest records this very file: it was built with it.
    pub built_in: bool,
}

/// The patch question of a package just opened.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchQuestion {
    pub files: Vec<Found>,
    /// The dialog has been on screen (leaving it opens without the patch).
    pub shown: bool,
}

/// "2026-11-20" (UTC) of a file's modification time.
fn file_date(path: &Path) -> Option<String> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let s = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(ocw_client::menu::trust::iso_date(s))
}

/// The patch file names the package's manifest records with the content
/// found next to it (16H Q9).
fn built_in(manifest: Option<&Path>) -> Vec<String> {
    let Some(m) = manifest
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| SignedManifest::parse(&t).ok())
    else {
        return Vec::new();
    };
    let dir = manifest.and_then(Path::parent).unwrap_or(Path::new("."));
    m.manifest
        .patches
        .iter()
        .filter(|p| {
            let found = std::fs::read(dir.join(&p.path)).ok();
            p.status(found.as_deref()) == PatchStatus::Same
        })
        .map(|p| p.path.clone())
        .collect()
}

impl PatchQuestion {
    /// The question for `package` (id `package_id`): `None` when no patch
    /// file lies next to it.
    pub fn find(package: &Path, manifest: Option<&Path>, package_id: &str) -> Option<Self> {
        let files = patch_files(package);
        if files.is_empty() {
            return None;
        }
        let recorded = built_in(manifest);
        let files = files
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Found {
                    patch: crate::patch::read_patch(&path, package_id),
                    date: file_date(&path),
                    built_in: recorded.contains(&name),
                    path,
                }
            })
            .collect();
        Some(Self {
            files,
            shown: false,
        })
    }

    /// The files that can be loaded.
    pub fn loadable(&self) -> impl Iterator<Item = (&Found, &WorldPatch)> {
        self.files
            .iter()
            .filter_map(|f| f.patch.as_ref().ok().map(|p| (f, p)))
    }

    /// Whether anything can be loaded (else there is nothing to ask).
    pub fn any(&self) -> bool {
        self.loadable().next().is_some()
    }

    /// The number of changes (operations) of the loadable files.
    pub fn changes(&self) -> usize {
        self.loadable().map(|(_, p)| p.ops.len()).sum()
    }

    /// A stack of more than one loadable file.
    pub fn is_stack(&self) -> bool {
        self.loadable().count() > 1
    }

    /// "by Ada Rider, 2026-11-20": the authors the patches name and the
    /// newest file date, as far as known.
    pub fn by_line(&self) -> String {
        let mut authors: Vec<&str> = Vec::new();
        for (_, p) in self.loadable() {
            if let Some(a) = p.author.as_deref().map(str::trim).filter(|a| !a.is_empty())
                && !authors.contains(&a)
            {
                authors.push(a);
            }
        }
        let date = self.loadable().filter_map(|(f, _)| f.date.clone()).max();
        let who = if authors.is_empty() {
            "author not named".to_owned()
        } else {
            format!(
                "by {}",
                authors
                    .iter()
                    .map(|a| ocw_client::menu::pages::short_label(a, 30))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        match date {
            Some(d) => format!("{who}, {d}"),
            None => who,
        }
    }

    /// The question's first line: "This world has a patch: 3 changes (by
    /// Ada Rider, 2026-11-20)."
    pub fn summary(&self) -> String {
        let n = self.changes();
        let files = self.loadable().count();
        format!(
            "This world has a patch: {n} change{}{} ({}).",
            if n == 1 { "" } else { "s" },
            if files > 1 {
                format!(" in {files} files")
            } else {
                String::new()
            },
            self.by_line()
        )
    }

    /// The dialog's paragraphs (the most important first: a small window
    /// drops them from the end).
    pub fn text(&self) -> Vec<String> {
        let mut out = vec![self.summary()];
        let names: Vec<String> = self
            .loadable()
            .map(|(f, p)| {
                let name = f
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let title = p
                    .title
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| format!(" '{}'", ocw_client::menu::pages::short_label(t, 40)))
                    .unwrap_or_default();
                let built = if f.built_in {
                    ", built into this package"
                } else {
                    ""
                };
                format!("{name}{title} ({} changes{built})", p.ops.len())
            })
            .collect();
        out.push(format!(
            "{}: {}.",
            if names.len() > 1 { "Files" } else { "File" },
            names.join("; ")
        ));
        let all_built = self.loadable().all(|(f, _)| f.built_in);
        out.push(if all_built {
            "The package was built with it (its manifest records it), so its roads already look that way. Load it to see and change those edits; Open without it to start a new patch."
                .into()
        } else {
            "Load it to see its changes as edits you can change, undo and save. Open without it to see the package as it was built; File > Open patch loads it later."
                .into()
        });
        if self.is_stack() {
            out.push(
                "The files are loaded as one patch, in this order; save it with Save patch as."
                    .into(),
            );
        }
        for f in &self.files {
            if let Err(e) = &f.patch {
                out.push(format!("Not loadable: {e}"));
            }
        }
        out
    }

    /// The console's line while it waits.
    pub fn console_line(&self) -> String {
        format!(
            "{} Load it or open without it in the dialog (or: studio patch load | skip)",
            self.summary()
        )
    }

    /// The loadable files as one patch: one file as it is; a stack's
    /// operations in order under the first file's header, with every
    /// file's sources.
    pub fn merged(&self) -> Option<WorldPatch> {
        let mut it = self.loadable();
        let (_, first) = it.next()?;
        let mut out = first.clone();
        for (_, p) in it {
            for s in &p.sources {
                if !out.sources.contains(s) {
                    out.sources.push(s.clone());
                }
            }
            out.ops.extend(p.ops.iter().cloned());
            // The saved views of every file of the stack (16H Q13 (b)).
            for v in &p.views {
                if !out.views.contains(v) {
                    out.views.push(v.clone());
                }
            }
        }
        Some(out)
    }

    /// The file Save writes to after loading: the single file loaded, none
    /// for a stack (Save as).
    pub fn save_file(&self) -> Option<PathBuf> {
        let mut it = self.loadable();
        let (f, _) = it.next()?;
        it.next().is_none().then(|| f.path.clone())
    }

    /// The files' names for messages ("a.ocwpatch, a.2.ocwpatch").
    pub fn names(&self) -> String {
        self.loadable()
            .map(|(f, _)| shown_path(&f.path))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What *Save patch* to the default file must mind after the question.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveGuard {
    /// Opened without the patch: the file there is kept as `….bak` first.
    Skipped(PathBuf),
    /// A stack was loaded as one patch: Save as only.
    Stack(String),
}

/// The text of the dialog page (`MenuHooks::text`).
pub fn menu_text(world: &World, page: Page) -> Vec<String> {
    if page != Page::ConfirmPatch {
        return Vec::new();
    }
    match world
        .get_resource::<Studio>()
        .and_then(|s| s.patch_question.as_ref())
    {
        Some(q) => q.text(),
        None => vec!["No patch is waiting for a decision.".into()],
    }
}

/// Opens the menu at the question while one waits – after a pending trust
/// question (that one first) –, and opens without the patch when the
/// dialog was left without an answer (Back, Esc, B).
pub fn show_dialog(world: &mut World) {
    let waiting = world
        .get_resource::<Studio>()
        .and_then(|s| s.patch_question.as_ref())
        .map(|q| q.shown);
    let Some(shown) = waiting else {
        return;
    };
    let trust_first = world
        .get_resource::<ocw_ui::trust::TrustDialog>()
        .is_some_and(ocw_ui::trust::TrustDialog::is_pending);
    if trust_first {
        return;
    }
    let Some(page) = world
        .get_resource::<ocw_ui::menu::MenuState>()
        .map(|s| s.menu.page())
    else {
        return;
    };
    if page == Some(Page::ConfirmTrust) {
        return;
    }
    if page == Some(Page::ConfirmPatch) {
        if let Some(q) = world.resource_mut::<Studio>().patch_question.as_mut() {
            q.shown = true;
        }
    } else if shown {
        // Left with Back: the menu it fell back to closes too (the studio
        // has no game menu).
        world
            .resource_mut::<ocw_ui::menu::MenuState>()
            .queue(Action::Close);
        let msg = answer(world, false).unwrap_or_else(|e| e);
        info!("{msg}");
        world.resource_mut::<Studio>().status = msg.clone();
        if let Some(mut c) = world.get_resource_mut::<ocw_render::console::ConsoleState>() {
            c.print(msg);
        }
    } else {
        world
            .resource_mut::<ocw_ui::menu::MenuState>()
            .queue(Action::PatchDialog);
    }
}

/// Answers the question (`load`: *Load it*, else *Open without it*) and
/// closes the dialog; with no question waiting, *load* loads the patch
/// next to the package now (as one undoable step if the document has
/// changes) and *skip* says there is nothing to answer.
pub fn answer(world: &mut World, load: bool) -> Result<String, String> {
    let now = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    let on_dialog = world
        .get_resource::<ocw_ui::menu::MenuState>()
        .is_some_and(|s| s.menu.page() == Some(Page::ConfirmPatch));
    let r = world.resource_mut::<Studio>().answer_patch(load, now);
    if on_dialog && let Some(mut s) = world.get_resource_mut::<ocw_ui::menu::MenuState>() {
        s.queue(Action::Close);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ocw-studio-prompt-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn patch(author: Option<&str>, ops: usize) -> WorldPatch {
        let mut p = WorldPatch::new("test/land");
        p.author = author.map(Into::into);
        p.title = Some("Vaduz fixes".into());
        for i in 0..ops {
            p.ops.push(ocw_world::patch::Operation::RemoveRoad(
                ocw_world::patch::RemoveRoad {
                    way: Some(100 + i as i64),
                    edge: None,
                    note: None,
                },
            ));
        }
        p
    }

    fn write(path: &Path, p: &WorldPatch) {
        std::fs::write(path, p.to_toml()).unwrap();
    }

    /// The main file first, then the stack by its numbers; other
    /// packages' patches and backups are not taken.
    #[test]
    fn the_patch_files_next_to_a_package_and_their_order() {
        let d = tmp("files");
        let pkg = d.join("land.ocwpkg");
        assert!(patch_files(&pkg).is_empty());
        for f in [
            "land.10.ocwpatch",
            "land.2.ocwpatch",
            "land.020-bridge.ocwpatch",
            "land.ocwpatch",
            "land.ocwpatch.bak",
            "land-north.ocwpatch",
            "other.ocwpatch",
            "land.zz-notes.ocwpatch",
        ] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        let names: Vec<String> = patch_files(&pkg)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "land.ocwpatch",
                "land.2.ocwpatch",
                "land.10.ocwpatch",
                "land.020-bridge.ocwpatch",
                "land.zz-notes.ocwpatch"
            ]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The dialog's text: the changes, the author and date, the files; a
    /// stack merged in order; a foreign file named but not counted.
    #[test]
    fn the_question_says_what_the_patch_is() {
        let d = tmp("question");
        let pkg = d.join("land.ocwpkg");
        assert_eq!(PatchQuestion::find(&pkg, None, "test/land"), None);
        write(&d.join("land.ocwpatch"), &patch(Some("Ada Rider"), 3));
        let q = PatchQuestion::find(&pkg, None, "test/land").unwrap();
        let today = q.files[0].date.clone().expect("a file date");
        assert_eq!(
            q.summary(),
            format!("This world has a patch: 3 changes (by Ada Rider, {today}).")
        );
        let t = q.text();
        assert!(
            t[1].contains("land.ocwpatch 'Vaduz fixes' (3 changes)"),
            "{t:?}"
        );
        assert!(t[2].starts_with("Load it to see"), "{t:?}");
        assert_eq!(q.save_file(), Some(d.join("land.ocwpatch")));
        assert_eq!(q.merged().unwrap().ops.len(), 3);
        assert!(!q.is_stack());
        // A stack: merged in order, saved with Save as.
        write(&d.join("land.2.ocwpatch"), &patch(Some("Bea Climber"), 1));
        let mut foreign = patch(None, 1);
        foreign.package = "test/other".into();
        write(&d.join("land.3.ocwpatch"), &foreign);
        let q = PatchQuestion::find(&pkg, None, "test/land").unwrap();
        assert_eq!(q.changes(), 4);
        assert!(q.is_stack());
        assert!(
            q.summary().starts_with(
                "This world has a patch: 4 changes in 2 files (by Ada Rider, Bea Climber, "
            ),
            "{}",
            q.summary()
        );
        let m = q.merged().unwrap();
        assert_eq!(m.author.as_deref(), Some("Ada Rider"));
        assert_eq!(m.ops.len(), 4);
        assert_eq!(m.ops[3], patch(None, 1).ops[0], "the second file's last");
        assert_eq!(q.save_file(), None);
        let t = q.text().join("\n");
        assert!(t.contains("Save patch as"), "{t}");
        assert!(
            t.contains("Not loadable") && t.contains("test/other"),
            "{t}"
        );
        // Only a foreign file: nothing to ask.
        let d2 = tmp("foreign");
        write(&d2.join("land.ocwpatch"), &foreign);
        let q = PatchQuestion::find(&d2.join("land.ocwpkg"), None, "test/land").unwrap();
        assert!(!q.any());
        // No author: said so.
        let d3 = tmp("anon");
        write(&d3.join("land.ocwpatch"), &patch(None, 1));
        let q = PatchQuestion::find(&d3.join("land.ocwpkg"), None, "test/land").unwrap();
        assert!(
            q.summary().contains("1 change (author not named"),
            "{}",
            q.summary()
        );
        for d in [d, d2, d3] {
            std::fs::remove_dir_all(&d).unwrap();
        }
    }

    /// A patch the manifest records with the same content is "built into
    /// this package" (16H Q9); an edited one is not.
    #[test]
    fn a_patch_the_package_was_built_with_is_named_so() {
        use ocw_world::manifest::{Attribution, Manifest, PatchFile};
        let d = tmp("built");
        let pkg = d.join("land.ocwpkg");
        let text = patch(Some("Ada Rider"), 2).to_toml();
        std::fs::write(d.join("land.ocwpatch"), &text).unwrap();
        let mut m = Manifest::new(
            "test/land",
            "Land",
            1,
            0,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("land.ocwpkg", b"pkg").unwrap();
        m.patches
            .push(PatchFile::of("land.ocwpatch", text.as_bytes()).unwrap());
        let mp = d.join("land.ocwpkg.manifest.json");
        std::fs::write(&mp, SignedManifest::unsigned(&m).to_json()).unwrap();
        let q = PatchQuestion::find(&pkg, Some(&mp), "test/land").unwrap();
        assert!(q.files[0].built_in);
        let t = q.text();
        assert!(t[1].contains("built into this package"), "{t:?}");
        assert!(t[2].starts_with("The package was built with it"), "{t:?}");
        // Edited after the build: no longer the one it was built with.
        write(&d.join("land.ocwpatch"), &patch(Some("Ada Rider"), 3));
        let q = PatchQuestion::find(&pkg, Some(&mp), "test/land").unwrap();
        assert!(!q.files[0].built_in);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
