//! The World workspace's document (spec §14 Phase 16, 16C): which package
//! is open, and the views saved in it – named camera positions to fly back
//! to (studio bookmarks: they belong to the studio session, not to the
//! package).
//!
//! **Where the world patch plugs in (16C (b), stage E5):** the patch
//! operations (road attributes, profile points, objects, areas, …) become
//! further variants of [`WorldOp`] – or a document kind of their own next
//! to this one – each returning its exact inverse from
//! [`DocumentKind::apply`], with the patch checks as a validator
//! ([`crate::Validator`]). Undo / redo, dirty flags and autosave then work
//! for them unchanged. Nothing of the patch format is assumed here.

use serde::{Deserialize, Serialize};

use crate::document::{DocumentKind, EditError};
use crate::validate::Finding;

/// The package a world document is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageRef {
    /// The `--world` path or id it was opened with.
    pub spec: String,
    /// Manifest id and version (`local/<name>`, 0 without a manifest).
    pub id: String,
    pub version: u64,
}

/// A saved view: where the camera was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Unique within the document, never reused (0: not given yet).
    pub id: u32,
    pub name: String,
    /// The eye (home package metres; x, y up, z).
    pub pos: [f64; 3],
    /// Yaw (rad, counter-clockwise from north) and pitch (rad, up).
    pub yaw: f64,
    pub pitch: f64,
}

impl View {
    /// A view at the origin (tests).
    pub fn named(name: &str) -> Self {
        Self {
            id: 0,
            name: name.into(),
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// The World workspace's document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldDoc {
    pub package: PackageRef,
    pub views: Vec<View>,
    /// The next view id.
    pub next_view: u32,
}

/// The World document's commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorldOp {
    /// Inserts a view at `index` (the end if past it); a view with id 0
    /// gets the next free id.
    Add { view: View, index: usize },
    /// Removes the view with this id.
    Remove(u32),
    /// Renames a view (typing merges into one undo step).
    Rename { id: u32, name: String },
    /// Moves a view to another camera position.
    Move {
        id: u32,
        pos: [f64; 3],
        yaw: f64,
        pitch: f64,
    },
}

impl WorldOp {
    /// Adds `view` at the end.
    pub fn add(view: View) -> Self {
        Self::Add {
            view,
            index: usize::MAX,
        }
    }
}

impl WorldDoc {
    pub fn new(package: PackageRef) -> Self {
        Self {
            package,
            views: Vec::new(),
            next_view: 1,
        }
    }

    pub fn view(&self, id: u32) -> Option<&View> {
        self.views.iter().find(|v| v.id == id)
    }

    fn index(&self, id: u32) -> Result<usize, EditError> {
        self.views
            .iter()
            .position(|v| v.id == id)
            .ok_or_else(|| EditError::new(format!("no view {id}")))
    }

    /// A name for a new view: "View 1", "View 2", … not taken yet.
    pub fn new_view_name(&self) -> String {
        (1..)
            .map(|n| format!("View {n}"))
            .find(|n| !self.views.iter().any(|v| &v.name == n))
            .expect("some number is free")
    }
}

/// What can be selected in the World document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorldItem {
    View(u32),
}

impl DocumentKind for WorldDoc {
    type Op = WorldOp;
    type Item = WorldItem;
    const KIND: &'static str = "world";

    fn apply(&mut self, op: &WorldOp) -> Result<WorldOp, EditError> {
        Ok(match op {
            WorldOp::Add { view, index } => {
                let mut view = view.clone();
                if view.id == 0 {
                    view.id = self.next_view.max(1);
                }
                if self.view(view.id).is_some() {
                    return Err(EditError::new(format!("view {} exists", view.id)));
                }
                self.next_view = self.next_view.max(view.id + 1);
                let id = view.id;
                let at = (*index).min(self.views.len());
                self.views.insert(at, view);
                WorldOp::Remove(id)
            }
            WorldOp::Remove(id) => {
                let i = self.index(*id)?;
                let view = self.views.remove(i);
                WorldOp::Add { view, index: i }
            }
            WorldOp::Rename { id, name } => {
                let i = self.index(*id)?;
                let old = std::mem::replace(&mut self.views[i].name, name.clone());
                WorldOp::Rename { id: *id, name: old }
            }
            WorldOp::Move {
                id,
                pos,
                yaw,
                pitch,
            } => {
                let i = self.index(*id)?;
                let v = &mut self.views[i];
                let undo = WorldOp::Move {
                    id: *id,
                    pos: v.pos,
                    yaw: v.yaw,
                    pitch: v.pitch,
                };
                (v.pos, v.yaw, v.pitch) = (*pos, *yaw, *pitch);
                undo
            }
        })
    }

    fn label(op: &WorldOp) -> String {
        match op {
            WorldOp::Add { .. } => "Add view".into(),
            WorldOp::Remove(_) => "Delete view".into(),
            WorldOp::Rename { .. } => "Rename view".into(),
            WorldOp::Move { .. } => "Move view".into(),
        }
    }

    fn merges(prev: &WorldOp, next: &WorldOp) -> bool {
        match (prev, next) {
            (WorldOp::Rename { id: a, .. }, WorldOp::Rename { id: b, .. }) => a == b,
            (WorldOp::Move { id: a, .. }, WorldOp::Move { id: b, .. }) => a == b,
            _ => false,
        }
    }

    fn contains(&self, item: &WorldItem) -> bool {
        match item {
            WorldItem::View(id) => self.view(*id).is_some(),
        }
    }
}

/// The World document's validator: views have names, and different ones.
pub fn check_views(doc: &WorldDoc) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, v) in doc.views.iter().enumerate() {
        let name = v.name.trim();
        if name.is_empty() {
            out.push(Finding::warning("a view without a name").about(format!("view {}", v.id)));
        } else if doc.views[..i].iter().any(|o| o.name.trim() == name) {
            out.push(Finding::warning(format!("two views are called '{name}'")).about(name));
        }
        if v.pos.iter().any(|c| !c.is_finite()) {
            out.push(Finding::error("a view without a valid position").about(name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocId, Document};

    fn doc() -> Document<WorldDoc> {
        Document::new(
            DocId(1),
            "Testland",
            WorldDoc::new(PackageRef {
                spec: "data/testland.ocwpkg".into(),
                id: "local/testland".into(),
                version: 0,
            }),
        )
        .with_validator(Box::new(check_views))
    }

    #[test]
    fn views_are_added_renamed_moved_and_removed_with_undo() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("Bridge")), 0.0).unwrap();
        d.execute(WorldOp::add(View::named("Pass")), 5.0).unwrap();
        let ids: Vec<u32> = d.content().views.iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(d.content().new_view_name(), "View 1");
        d.execute(
            WorldOp::Move {
                id: 2,
                pos: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: -0.1,
            },
            10.0,
        )
        .unwrap();
        d.execute(WorldOp::Remove(1), 15.0).unwrap();
        assert_eq!(d.content().views.len(), 1);
        d.undo(16.0).unwrap();
        assert_eq!(d.content().views[0].name, "Bridge", "back in its place");
        d.undo(17.0).unwrap();
        assert_eq!(d.content().view(2).unwrap().pos, [0.0; 3]);
        // A removed view's id is not given again.
        d.undo(18.0).unwrap();
        d.execute(WorldOp::add(View::named("Lake")), 30.0).unwrap();
        assert_eq!(d.content().view(3).unwrap().name, "Lake");
        assert!(d.execute(WorldOp::Remove(9), 31.0).is_err());
        let mut taken = View::named("x");
        taken.id = 3;
        assert!(d.execute(WorldOp::add(taken), 32.0).is_err());
    }

    #[test]
    fn typing_a_name_is_one_undo_step() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("V")), 0.0).unwrap();
        d.seal();
        for (k, name) in ["S", "St", "Sta", "Star", "Start"].iter().enumerate() {
            d.execute(
                WorldOp::Rename {
                    id: 1,
                    name: (*name).into(),
                },
                1.0 + 0.2 * k as f64,
            )
            .unwrap();
        }
        assert_eq!(d.history().undo.len(), 2);
        assert_eq!(d.undo_label(), Some("Rename view"));
        d.undo(3.0).unwrap();
        assert_eq!(d.content().views[0].name, "V");
    }

    #[test]
    fn the_validator_reports_unnamed_and_doubled_views() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("A")), 0.0).unwrap();
        assert!(d.findings().is_empty());
        d.execute(WorldOp::add(View::named("A")), 5.0).unwrap();
        assert_eq!(d.findings().len(), 1);
        assert!(d.findings()[0].message.contains("'A'"));
        d.execute(
            WorldOp::Rename {
                id: 2,
                name: " ".into(),
            },
            10.0,
        )
        .unwrap();
        assert!(d.findings()[0].message.contains("without a name"));
        let mut bad = View::named("B");
        bad.pos[0] = f64::NAN;
        d.execute(WorldOp::add(bad), 15.0).unwrap();
        assert_eq!(
            crate::validate::worst(d.findings()),
            Some(crate::Severity::Error)
        );
        d.undo(16.0).unwrap();
        d.undo(17.0).unwrap();
        d.undo(18.0).unwrap();
        assert!(d.findings().is_empty());
    }

    #[test]
    fn a_selected_view_is_dropped_when_it_goes() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("A")), 0.0).unwrap();
        d.selection.select_only(WorldItem::View(1));
        d.execute(WorldOp::Remove(1), 5.0).unwrap();
        assert!(d.selection.is_empty());
    }
}
