//! The open documents of a studio session (spec §14 Phase 16, 16A):
//! documents of every kind side by side (a world, its routes, a model),
//! one of them active – the Edit menu's undo / redo act on it – and the
//! autosave of all of them.

use std::any::Any;

use crate::autosave::{AutosaveRecord, AutosaveStore, autosave_name};
use crate::document::{DocId, Document, DocumentKind, EditError};
use crate::validate::{Finding, Validator};

/// What the session (and the UI) needs of a document of any kind.
pub trait AnyDocument: Any + Send + Sync {
    fn id(&self) -> DocId;
    fn name(&self) -> &str;
    fn kind(&self) -> &'static str;
    fn source(&self) -> Option<&str>;
    fn is_dirty(&self) -> bool;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;
    fn undo_label(&self) -> Option<&str>;
    fn redo_label(&self) -> Option<&str>;
    fn undo(&mut self, now_s: f64) -> Result<bool, EditError>;
    fn redo(&mut self, now_s: f64) -> Result<bool, EditError>;
    fn seal(&mut self);
    fn findings(&self) -> &[Finding];
    fn autosave_due(&self, now_s: f64, delay_s: f64) -> bool;
    /// The record name of its autosave.
    fn autosave_name(&self) -> String;
    /// Its autosave record as JSON.
    fn autosave_json(&self) -> Result<String, String>;
    fn mark_autosaved(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: DocumentKind> AnyDocument for Document<D> {
    fn id(&self) -> DocId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &'static str {
        D::KIND
    }
    fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    fn is_dirty(&self) -> bool {
        Document::is_dirty(self)
    }
    fn can_undo(&self) -> bool {
        Document::can_undo(self)
    }
    fn can_redo(&self) -> bool {
        Document::can_redo(self)
    }
    fn undo_label(&self) -> Option<&str> {
        Document::undo_label(self)
    }
    fn redo_label(&self) -> Option<&str> {
        Document::redo_label(self)
    }
    fn undo(&mut self, now_s: f64) -> Result<bool, EditError> {
        Document::undo(self, now_s)
    }
    fn redo(&mut self, now_s: f64) -> Result<bool, EditError> {
        Document::redo(self, now_s)
    }
    fn seal(&mut self) {
        Document::seal(self);
    }
    fn findings(&self) -> &[Finding] {
        Document::findings(self)
    }
    fn autosave_due(&self, now_s: f64, delay_s: f64) -> bool {
        Document::autosave_due(self, now_s, delay_s)
    }
    fn autosave_name(&self) -> String {
        autosave_name(D::KIND, self.source.as_deref().unwrap_or(&self.name))
    }
    fn autosave_json(&self) -> Result<String, String> {
        self.autosave_record().to_json()
    }
    fn mark_autosaved(&mut self) {
        Document::mark_autosaved(self);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The open documents.
#[derive(Default)]
pub struct Session {
    docs: Vec<Box<dyn AnyDocument>>,
    active: Option<DocId>,
    next_id: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field(
                "docs",
                &self.docs.iter().map(|d| d.name()).collect::<Vec<_>>(),
            )
            .field("active", &self.active)
            .finish()
    }
}

impl Session {
    /// Opens a document made by `make` (given its new id) and makes it the
    /// active one.
    pub fn open<D: DocumentKind>(&mut self, make: impl FnOnce(DocId) -> Document<D>) -> DocId {
        self.next_id += 1;
        let id = DocId(self.next_id);
        let mut doc = make(id);
        doc.id = id;
        self.docs.push(Box::new(doc));
        self.active = Some(id);
        id
    }

    /// Restores a document from an autosave record's JSON and makes it the
    /// active one.
    pub fn restore<D: DocumentKind>(
        &mut self,
        json: &str,
        validators: Vec<Validator<D>>,
    ) -> Result<DocId, EditError> {
        let record: AutosaveRecord<D, D::Op> =
            AutosaveRecord::from_json(json).map_err(EditError)?;
        let id = DocId(self.next_id + 1);
        let doc = Document::restore(id, record, validators)?;
        self.next_id += 1;
        self.docs.push(Box::new(doc));
        self.active = Some(id);
        Ok(id)
    }

    /// Closes a document (the next one becomes active).
    pub fn close(&mut self, id: DocId) -> Option<Box<dyn AnyDocument>> {
        let i = self.docs.iter().position(|d| d.id() == id)?;
        let doc = self.docs.remove(i);
        if self.active == Some(id) {
            self.active = self
                .docs
                .get(i)
                .or_else(|| self.docs.last())
                .map(|d| d.id());
        }
        Some(doc)
    }

    pub fn docs(&self) -> impl Iterator<Item = &dyn AnyDocument> {
        self.docs.iter().map(|d| d.as_ref())
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn any(&self, id: DocId) -> Option<&dyn AnyDocument> {
        self.docs.iter().find(|d| d.id() == id).map(|d| d.as_ref())
    }

    fn any_mut(&mut self, id: DocId) -> Option<&mut Box<dyn AnyDocument>> {
        self.docs.iter_mut().find(|d| d.id() == id)
    }

    /// The document `id` as its kind (`None`: no such document, or
    /// another kind).
    pub fn get<D: DocumentKind>(&self, id: DocId) -> Option<&Document<D>> {
        self.any(id)?.as_any().downcast_ref()
    }

    pub fn get_mut<D: DocumentKind>(&mut self, id: DocId) -> Option<&mut Document<D>> {
        self.any_mut(id)?.as_any_mut().downcast_mut()
    }

    /// The first open document of kind `D`.
    pub fn first<D: DocumentKind>(&self) -> Option<&Document<D>> {
        self.docs.iter().find_map(|d| d.as_any().downcast_ref())
    }

    pub fn active_id(&self) -> Option<DocId> {
        self.active
    }

    pub fn active(&self) -> Option<&dyn AnyDocument> {
        self.any(self.active?)
    }

    /// Makes `id` the active document; `false` if it is not open.
    pub fn set_active(&mut self, id: DocId) -> bool {
        let open = self.any(id).is_some();
        if open {
            self.active = Some(id);
        }
        open
    }

    /// Executes a command on document `id`.
    pub fn execute<D: DocumentKind>(
        &mut self,
        id: DocId,
        op: D::Op,
        now_s: f64,
    ) -> Result<(), EditError> {
        self.get_mut::<D>(id)
            .ok_or_else(|| EditError::new(format!("no open {} document {}", D::KIND, id.0)))?
            .execute(op, now_s)
    }

    /// Undoes the active document's last step (`Ok(false)`: nothing).
    pub fn undo(&mut self, now_s: f64) -> Result<bool, EditError> {
        match self.active.and_then(|id| self.any_mut(id)) {
            Some(d) => d.undo(now_s),
            None => Ok(false),
        }
    }

    pub fn redo(&mut self, now_s: f64) -> Result<bool, EditError> {
        match self.active.and_then(|id| self.any_mut(id)) {
            Some(d) => d.redo(now_s),
            None => Ok(false),
        }
    }

    /// Any document with unsaved changes (asked before quitting).
    pub fn any_dirty(&self) -> bool {
        self.docs.iter().any(|d| d.is_dirty())
    }

    /// Writes the autosave of every document that is due (changed and
    /// left alone for `delay_s`). The names written, and the errors.
    pub fn autosave(
        &mut self,
        store: &mut dyn AutosaveStore,
        now_s: f64,
        delay_s: f64,
    ) -> (Vec<String>, Vec<String>) {
        let mut written = Vec::new();
        let mut errors = Vec::new();
        for d in &mut self.docs {
            if !d.autosave_due(now_s, delay_s) {
                continue;
            }
            let name = d.autosave_name();
            match d.autosave_json().and_then(|j| store.write(&name, &j)) {
                Ok(()) => {
                    d.mark_autosaved();
                    written.push(name);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        (written, errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autosave::MemoryStore;
    use crate::document::tests::{NumOp, Numbers};
    use crate::world::{View, WorldDoc, WorldOp};

    fn numbers(id: DocId) -> Document<Numbers> {
        Document::new(id, "numbers", Numbers::default()).with_source("/data/n.txt")
    }

    #[test]
    fn documents_of_two_kinds_undo_on_their_own() {
        let mut s = Session::default();
        let n = s.open(numbers);
        let w = s.open(|id| Document::new(id, "Vaduz", WorldDoc::default()));
        assert_eq!(s.active_id(), Some(w));
        s.execute::<Numbers>(n, NumOp::Push(4), 0.0).unwrap();
        s.execute::<WorldDoc>(w, WorldOp::add(View::named("Start")), 1.0)
            .unwrap();
        assert!(
            s.execute::<WorldDoc>(n, WorldOp::Remove(1), 2.0).is_err(),
            "wrong kind"
        );
        // Undo acts on the active document only.
        assert!(s.undo(3.0).unwrap());
        assert!(s.get::<WorldDoc>(w).unwrap().content().views.is_empty());
        assert_eq!(s.get::<Numbers>(n).unwrap().content().0, vec![4]);
        assert!(!s.undo(4.0).unwrap(), "nothing left in the world");
        assert!(s.set_active(n));
        assert!(s.any_dirty());
        assert_eq!(s.active().unwrap().undo_label(), Some("Add number"));
        assert!(s.undo(5.0).unwrap());
        assert!(!s.any_dirty());
        assert!(s.redo(6.0).unwrap());
        assert_eq!(s.first::<Numbers>().unwrap().content().0, vec![4]);
        // Closing the active document activates the next one.
        assert!(s.close(n).is_some());
        assert_eq!(s.active_id(), Some(w));
        assert!(s.close(n).is_none());
        assert!(!s.set_active(n));
        s.close(w);
        assert!(s.is_empty());
        assert_eq!(s.active_id(), None);
        assert!(!s.undo(7.0).unwrap());
    }

    #[test]
    fn autosave_writes_what_is_due_and_restores_it() {
        let mut s = Session::default();
        let n = s.open(numbers);
        let mut store = MemoryStore::default();
        assert_eq!(s.autosave(&mut store, 100.0, 5.0).0.len(), 0, "unchanged");
        s.execute::<Numbers>(n, NumOp::Push(1), 10.0).unwrap();
        assert_eq!(s.autosave(&mut store, 12.0, 5.0).0.len(), 0, "too soon");
        let (written, errors) = s.autosave(&mut store, 15.0, 5.0);
        assert_eq!((written.len(), errors.len()), (1, 0));
        assert_eq!(s.autosave(&mut store, 30.0, 5.0).0.len(), 0, "done");
        // A failing store reports and tries again next time.
        s.execute::<Numbers>(n, NumOp::Push(2), 40.0).unwrap();
        store.fail = Some("disk full".into());
        let (written, errors) = s.autosave(&mut store, 50.0, 5.0);
        assert!(written.is_empty());
        assert!(errors[0].contains("disk full"), "{errors:?}");
        store.fail = None;
        assert_eq!(s.autosave(&mut store, 51.0, 5.0).0.len(), 1);

        // After a crash: a new session restores it, history included.
        let json = store.read(&written_name(&s, n)).unwrap().unwrap();
        let mut t = Session::default();
        let r = t.restore::<Numbers>(&json, Vec::new()).unwrap();
        assert_eq!(t.get::<Numbers>(r).unwrap().content().0, vec![1, 2]);
        assert!(t.any_dirty());
        assert!(t.undo(60.0).unwrap());
        assert!(t.undo(61.0).unwrap());
        assert!(!t.any_dirty(), "undone to the file as opened");
        assert!(
            t.restore::<WorldDoc>(&json, Vec::new()).is_err(),
            "wrong kind"
        );
    }

    fn written_name(s: &Session, id: DocId) -> String {
        s.any(id).unwrap().autosave_name()
    }
}
