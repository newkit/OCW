//! One open document: its content, the edit history (undo / redo), the
//! dirty flags, the selection and the validators' findings (spec §14
//! Phase 16, 16A).
//!
//! **Commands with undo / redo.** Every change is an operation of the
//! document's kind ([`DocumentKind::Op`]). Applying it returns its exact
//! inverse, which is kept with it: undo applies the inverses, redo the
//! operations again. The history is plain data (serialisable), unlimited
//! and per document. Consecutive operations that [`DocumentKind::merges`]
//! accepts (a drag, typing a name) become one undo step while they come
//! within [`Document::merge_window_s`] of each other and the step is not
//! [`Document::seal`]ed (the app seals on mouse release or focus loss).
//!
//! **Dirty flags.** Every undo step has an id that is never reused. The
//! document's *state* is the id of the step on top of the undo stack (0:
//! none), so undoing back to the saved state makes the document clean
//! again, and redoing to it too. The same id marks what the last autosave
//! holds.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::selection::Selection;
use crate::validate::{Finding, Validator};

/// A document's id within a [`crate::Session`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocId(pub u64);

/// Why an operation could not be applied (the document is unchanged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

impl EditError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// What a document holds, and how it is edited.
pub trait DocumentKind:
    Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// The edit operations (commands).
    type Op: Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync;
    /// What can be selected.
    type Item: Clone + Ord + std::fmt::Debug + Serialize + DeserializeOwned + Send + Sync;

    /// The kind's name in autosaves and the UI ("world", "route", "model").
    const KIND: &'static str;

    /// Applies `op` and returns the operation that undoes it exactly. On
    /// an error the content must be unchanged.
    fn apply(&mut self, op: &Self::Op) -> Result<Self::Op, EditError>;

    /// The undo menu's name of `op` ("Rename view").
    fn label(op: &Self::Op) -> String;

    /// Whether `next`, coming right after `prev`, continues it as one undo
    /// step (the same thing dragged on, the same name typed on).
    fn merges(_prev: &Self::Op, _next: &Self::Op) -> bool {
        false
    }

    /// Whether `item` is still in the document (the selection drops the
    /// rest after a change).
    fn contains(&self, _item: &Self::Item) -> bool {
        true
    }
}

/// An operation and its inverse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step<O> {
    pub op: O,
    pub inverse: O,
}

/// One undo step: one or more merged operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry<O> {
    /// Never reused within the document (the dirty flags compare it).
    pub id: u64,
    pub label: String,
    pub steps: Vec<Step<O>>,
    /// When its last operation was applied (the caller's clock, s).
    pub at_s: f64,
    /// Further merging operations may still join it.
    pub open: bool,
}

/// The undo and redo stacks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct History<O> {
    pub undo: Vec<Entry<O>>,
    /// The last undone step is at the end.
    pub redo: Vec<Entry<O>>,
    /// The next entry id (ids start at 1; 0 is the empty history).
    pub next_id: u64,
}

impl<O> Default for History<O> {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
        }
    }
}

impl<O> History<O> {
    /// The id of the state the document is in (0: no step applied).
    pub fn state(&self) -> u64 {
        self.undo.last().map_or(0, |e| e.id)
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// How long a merging operation may follow the one before it (s).
pub const MERGE_WINDOW_S: f64 = 1.0;

/// An open document.
pub struct Document<D: DocumentKind> {
    pub id: DocId,
    /// The name in the tabs and the title.
    pub name: String,
    /// Where it was opened from (a file, a package path), if anywhere.
    pub source: Option<String>,
    content: D,
    history: History<D::Op>,
    saved_state: u64,
    autosaved_state: u64,
    /// When the content last changed (the caller's clock, s).
    changed_at_s: Option<f64>,
    pub selection: Selection<D::Item>,
    validators: Vec<Validator<D>>,
    findings: Vec<Finding>,
    pub merge_window_s: f64,
}

impl<D: DocumentKind> std::fmt::Debug for Document<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("source", &self.source)
            .field("state", &self.state())
            .field("saved_state", &self.saved_state)
            .field("validators", &self.validators.len())
            .finish_non_exhaustive()
    }
}

impl<D: DocumentKind> Document<D> {
    /// A clean document with `content` (as opened or created).
    pub fn new(id: DocId, name: impl Into<String>, content: D) -> Self {
        Self {
            id,
            name: name.into(),
            source: None,
            content,
            history: History::default(),
            saved_state: 0,
            autosaved_state: 0,
            changed_at_s: None,
            selection: Selection::default(),
            validators: Vec::new(),
            findings: Vec::new(),
            merge_window_s: MERGE_WINDOW_S,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Adds a validator and runs all of them on the content now.
    pub fn with_validator(mut self, v: Validator<D>) -> Self {
        self.validators.push(v);
        self.validate();
        self
    }

    pub fn content(&self) -> &D {
        &self.content
    }

    pub fn history(&self) -> &History<D::Op> {
        &self.history
    }

    /// The validators' findings for the content as it is now.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// The id of the state the content is in (see the module docs).
    pub fn state(&self) -> u64 {
        self.history.state()
    }

    /// Changed since it was opened or last saved.
    pub fn is_dirty(&self) -> bool {
        self.state() != self.saved_state
    }

    /// The content was saved (as it is now).
    pub fn mark_saved(&mut self) {
        self.saved_state = self.state();
    }

    /// Changed since the last autosave (or since opening).
    pub fn needs_autosave(&self) -> bool {
        self.state() != self.autosaved_state
    }

    /// An autosave is due: changed since the last one, and the last change
    /// is at least `delay_s` old (no autosave in the middle of a drag).
    pub fn autosave_due(&self, now_s: f64, delay_s: f64) -> bool {
        self.needs_autosave() && self.changed_at_s.is_none_or(|t| now_s - t >= delay_s)
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// The name of what undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo.last().map(|e| e.label.as_str())
    }

    /// Executes `op` (a new command) at `now_s`: applies it, keeps its
    /// inverse, drops the redo stack, runs the validators. Merges into the
    /// previous step when the kind says so (see the module docs).
    pub fn execute(&mut self, op: D::Op, now_s: f64) -> Result<(), EditError> {
        let inverse = self.content.apply(&op)?;
        self.history.redo.clear();
        let window = self.merge_window_s;
        let merge = self.history.undo.last().is_some_and(|last| {
            last.open
                && now_s - last.at_s <= window
                && last.steps.last().is_some_and(|s| D::merges(&s.op, &op))
        });
        if merge {
            // A new id: the merged step is another state than the one a
            // save or an autosave may have seen.
            let id = self.history.fresh_id();
            let last = self.history.undo.last_mut().expect("checked above");
            last.id = id;
            last.at_s = now_s;
            last.steps.push(Step { op, inverse });
        } else {
            let id = self.history.fresh_id();
            self.history.undo.push(Entry {
                id,
                label: D::label(&op),
                steps: vec![Step { op, inverse }],
                at_s: now_s,
                open: true,
            });
        }
        self.changed(now_s);
        Ok(())
    }

    /// Ends the open undo step: the next operation starts a new one.
    pub fn seal(&mut self) {
        if let Some(last) = self.history.undo.last_mut() {
            last.open = false;
        }
    }

    /// Undoes the last step. `Ok(false)`: nothing to undo.
    pub fn undo(&mut self, now_s: f64) -> Result<bool, EditError> {
        let Some(mut entry) = self.history.undo.pop() else {
            return Ok(false);
        };
        let ops: Vec<D::Op> = entry
            .steps
            .iter()
            .rev()
            .map(|s| s.inverse.clone())
            .collect();
        match self.run_all(&ops) {
            Ok(redos) => {
                // The inverses' inverses (the operations again), newest
                // last again.
                for (step, op) in entry.steps.iter_mut().rev().zip(redos) {
                    step.op = op;
                }
                entry.open = false;
                self.history.redo.push(entry);
                self.changed(now_s);
                Ok(true)
            }
            Err(e) => {
                self.history.undo.push(entry);
                Err(e)
            }
        }
    }

    /// Redoes the last undone step. `Ok(false)`: nothing to redo.
    pub fn redo(&mut self, now_s: f64) -> Result<bool, EditError> {
        let Some(mut entry) = self.history.redo.pop() else {
            return Ok(false);
        };
        let ops: Vec<D::Op> = entry.steps.iter().map(|s| s.op.clone()).collect();
        match self.run_all(&ops) {
            Ok(inverses) => {
                for (step, inv) in entry.steps.iter_mut().zip(inverses) {
                    step.inverse = inv;
                }
                entry.open = false;
                self.history.undo.push(entry);
                self.changed(now_s);
                Ok(true)
            }
            Err(e) => {
                self.history.redo.push(entry);
                Err(e)
            }
        }
    }

    /// Applies `ops` in order; on an error the ones already applied are
    /// undone again, so the content is as before.
    fn run_all(&mut self, ops: &[D::Op]) -> Result<Vec<D::Op>, EditError> {
        let mut inverses = Vec::with_capacity(ops.len());
        for op in ops {
            match self.content.apply(op) {
                Ok(inv) => inverses.push(inv),
                Err(e) => {
                    for inv in inverses.iter().rev() {
                        // Inverses of operations just applied apply.
                        let _ = self.content.apply(inv);
                    }
                    return Err(e);
                }
            }
        }
        Ok(inverses)
    }

    fn changed(&mut self, now_s: f64) {
        self.changed_at_s = Some(now_s);
        let content = &self.content;
        self.selection.retain(|i| content.contains(i));
        self.validate();
    }

    /// Runs the validators on the content now.
    pub fn validate(&mut self) {
        self.findings = self
            .validators
            .iter()
            .flat_map(|v| v(&self.content))
            .collect();
    }

    /// The autosave record of the document as it is now.
    pub fn autosave_record(&self) -> crate::AutosaveRecord<D, D::Op> {
        crate::AutosaveRecord {
            format: crate::autosave::FORMAT,
            kind: D::KIND.into(),
            name: self.name.clone(),
            source: self.source.clone(),
            content: self.content.clone(),
            history: self.history.clone(),
            saved_state: self.saved_state,
        }
    }

    /// The autosave was written for the state now.
    pub fn mark_autosaved(&mut self) {
        self.autosaved_state = self.state();
    }

    /// A document restored from an autosave record: its content, history
    /// (undo and redo work on) and dirty flag as autosaved; the
    /// validators run at once.
    pub fn restore(
        id: DocId,
        record: crate::AutosaveRecord<D, D::Op>,
        validators: Vec<Validator<D>>,
    ) -> Result<Self, EditError> {
        if record.kind != D::KIND {
            return Err(EditError(format!(
                "an autosave of a {} document, not of a {}",
                record.kind,
                D::KIND
            )));
        }
        if record.format != crate::autosave::FORMAT {
            return Err(EditError(format!(
                "autosave format {} (this studio reads {})",
                record.format,
                crate::autosave::FORMAT
            )));
        }
        let mut doc = Self::new(id, record.name, record.content);
        doc.source = record.source;
        doc.history = record.history;
        doc.saved_state = record.saved_state;
        doc.autosaved_state = doc.state();
        doc.validators = validators;
        doc.validate();
        Ok(doc)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A test document: a list of numbers.
    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct Numbers(pub Vec<i64>);

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum NumOp {
        Push(i64),
        Pop,
        /// Adds to the value at an index (a drag: merges with the same
        /// index).
        Add(usize, i64),
        Set(usize, i64),
    }

    impl DocumentKind for Numbers {
        type Op = NumOp;
        type Item = usize;
        const KIND: &'static str = "numbers";

        fn apply(&mut self, op: &NumOp) -> Result<NumOp, EditError> {
            Ok(match op {
                NumOp::Push(v) => {
                    self.0.push(*v);
                    NumOp::Pop
                }
                NumOp::Pop => NumOp::Push(self.0.pop().ok_or(EditError::new("empty"))?),
                NumOp::Add(i, d) => {
                    let v = self.0.get_mut(*i).ok_or(EditError::new("no such index"))?;
                    *v += d;
                    NumOp::Add(*i, -d)
                }
                NumOp::Set(i, x) => {
                    let v = self.0.get_mut(*i).ok_or(EditError::new("no such index"))?;
                    NumOp::Set(*i, std::mem::replace(v, *x))
                }
            })
        }

        fn label(op: &NumOp) -> String {
            match op {
                NumOp::Push(_) => "Add number".into(),
                NumOp::Pop => "Remove number".into(),
                NumOp::Add(..) => "Change number".into(),
                NumOp::Set(..) => "Set number".into(),
            }
        }

        fn merges(prev: &NumOp, next: &NumOp) -> bool {
            matches!((prev, next), (NumOp::Add(a, _), NumOp::Add(b, _)) if a == b)
        }

        fn contains(&self, item: &usize) -> bool {
            *item < self.0.len()
        }
    }

    pub fn doc() -> Document<Numbers> {
        Document::new(DocId(1), "numbers", Numbers::default())
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut d = doc();
        assert!(!d.can_undo() && !d.can_redo());
        d.execute(NumOp::Push(1), 0.0).unwrap();
        d.seal();
        d.execute(NumOp::Push(2), 5.0).unwrap();
        d.execute(NumOp::Set(0, 9), 10.0).unwrap();
        assert_eq!(d.content().0, vec![9, 2]);
        assert_eq!(d.undo_label(), Some("Set number"));
        assert!(d.undo(11.0).unwrap());
        assert_eq!(d.content().0, vec![1, 2]);
        assert!(d.undo(12.0).unwrap());
        assert!(d.undo(13.0).unwrap());
        assert_eq!(d.content().0, Vec::<i64>::new());
        assert!(!d.undo(14.0).unwrap(), "nothing left to undo");
        assert_eq!(d.redo_label(), Some("Add number"));
        assert!(d.redo(15.0).unwrap());
        assert!(d.redo(16.0).unwrap());
        assert!(d.redo(17.0).unwrap());
        assert_eq!(d.content().0, vec![9, 2]);
        assert!(!d.redo(18.0).unwrap());
        // A new command after an undo drops the redo stack.
        d.undo(19.0).unwrap();
        d.execute(NumOp::Push(3), 30.0).unwrap();
        assert!(!d.can_redo());
        assert_eq!(d.content().0, vec![1, 2, 3]);
    }

    #[test]
    fn a_failing_command_changes_nothing() {
        let mut d = doc();
        d.execute(NumOp::Push(1), 0.0).unwrap();
        let before = d.history().clone();
        assert!(d.execute(NumOp::Set(4, 1), 1.0).is_err());
        assert_eq!(d.content().0, vec![1]);
        assert_eq!(d.history(), &before);
    }

    #[test]
    fn consecutive_drags_merge_into_one_step() {
        let mut d = doc();
        d.execute(NumOp::Push(10), 0.0).unwrap();
        d.seal();
        for k in 0..5 {
            d.execute(NumOp::Add(0, 1), 1.0 + 0.1 * f64::from(k))
                .unwrap();
        }
        assert_eq!(d.content().0, vec![15]);
        assert_eq!(d.history().undo.len(), 2, "push + one drag");
        assert_eq!(d.history().undo[1].steps.len(), 5);
        d.undo(2.0).unwrap();
        assert_eq!(d.content().0, vec![10]);
        d.redo(3.0).unwrap();
        assert_eq!(d.content().0, vec![15]);
        // A seal, a pause or another target starts a new step.
        d.execute(NumOp::Add(0, 1), 3.1).unwrap();
        assert_eq!(d.history().undo.len(), 3, "redo sealed the step");
        d.execute(NumOp::Add(0, 1), 3.2).unwrap();
        assert_eq!(d.history().undo.len(), 3);
        d.execute(NumOp::Add(0, 1), 9.0).unwrap();
        assert_eq!(d.history().undo.len(), 4, "too late to merge");
        d.execute(NumOp::Push(0), 9.1).unwrap();
        d.execute(NumOp::Add(1, 1), 9.2).unwrap();
        assert_eq!(d.history().undo.len(), 6, "push and add do not merge");
    }

    #[test]
    fn dirty_follows_the_saved_state_through_undo_and_redo() {
        let mut d = doc();
        assert!(!d.is_dirty());
        d.execute(NumOp::Push(1), 0.0).unwrap();
        assert!(d.is_dirty());
        d.mark_saved();
        assert!(!d.is_dirty());
        d.execute(NumOp::Push(2), 5.0).unwrap();
        assert!(d.is_dirty());
        d.undo(6.0).unwrap();
        assert!(!d.is_dirty(), "back at the saved state");
        d.undo(7.0).unwrap();
        assert!(d.is_dirty(), "before the saved state");
        d.redo(8.0).unwrap();
        assert!(!d.is_dirty(), "redone to the saved state");
        // A merge into the saved step is a change.
        d.execute(NumOp::Add(0, 1), 10.0).unwrap();
        d.mark_saved();
        d.execute(NumOp::Add(0, 1), 10.5).unwrap();
        assert_eq!(d.history().undo.len(), 2, "merged");
        assert!(d.is_dirty());
        // The same state reached by another path is not clean: ids are
        // never reused.
        let mut e = doc();
        e.execute(NumOp::Push(1), 0.0).unwrap();
        e.mark_saved();
        e.undo(1.0).unwrap();
        e.execute(NumOp::Push(1), 2.0).unwrap();
        assert!(e.is_dirty());
    }

    #[test]
    fn autosave_is_due_after_a_quiet_change() {
        let mut d = doc();
        assert!(!d.autosave_due(100.0, 2.0), "nothing changed");
        d.execute(NumOp::Push(1), 10.0).unwrap();
        assert!(!d.autosave_due(11.0, 2.0), "still being edited");
        assert!(d.autosave_due(12.0, 2.0));
        d.mark_autosaved();
        assert!(!d.autosave_due(20.0, 2.0));
        d.undo(21.0).unwrap();
        assert!(d.autosave_due(30.0, 2.0), "an undo is a change too");
    }

    #[test]
    fn validators_run_on_every_change() {
        let mut d = doc().with_validator(Box::new(|n: &Numbers| {
            n.0.iter()
                .enumerate()
                .filter(|(_, v)| **v < 0)
                .map(|(i, _)| Finding::warning("negative").about(i.to_string()))
                .collect()
        }));
        assert!(d.findings().is_empty());
        d.execute(NumOp::Push(-1), 0.0).unwrap();
        assert_eq!(d.findings().len(), 1);
        assert_eq!(d.findings()[0].target.as_deref(), Some("0"));
        d.undo(1.0).unwrap();
        assert!(d.findings().is_empty(), "after undo");
        d.redo(2.0).unwrap();
        assert_eq!(d.findings().len(), 1, "after redo");
    }

    #[test]
    fn the_selection_drops_what_is_gone() {
        let mut d = doc();
        d.execute(NumOp::Push(1), 0.0).unwrap();
        d.execute(NumOp::Push(2), 5.0).unwrap();
        d.selection.select_only(1);
        d.selection.add(0);
        d.undo(6.0).unwrap();
        assert_eq!(d.selection.single(), Some(&0));
    }

    /// A small deterministic generator for the random sequences.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Spec 16G: every command applies and undoes to the identical
    /// document, over random sequences of commands, undos, redos, seals
    /// and saves – checked against a replayed model of the history.
    #[test]
    fn random_sequences_undo_to_the_identical_document() {
        for seed in 1..=200u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut d = doc();
            // The content after each undo step (index 0: the start).
            let mut states = vec![d.content().clone()];
            let mut redo_states: Vec<Numbers> = Vec::new();
            let mut now = 0.0;
            for _ in 0..80 {
                now += [0.1, 0.5, 2.0][rng.below(3) as usize];
                match rng.below(10) {
                    0..=4 => {
                        let len = d.content().0.len() as u64;
                        let op = match rng.below(4) {
                            0 => NumOp::Push(rng.below(100) as i64),
                            1 => NumOp::Pop,
                            2 => NumOp::Add(rng.below(len.max(1) + 1) as usize, 1),
                            _ => NumOp::Set(rng.below(len.max(1)) as usize, rng.below(9) as i64),
                        };
                        let steps = d.history().undo.len();
                        if d.execute(op, now).is_ok() {
                            if d.history().undo.len() == steps {
                                // Merged: the step's end state changed.
                                *states.last_mut().unwrap() = d.content().clone();
                            } else {
                                states.push(d.content().clone());
                            }
                            redo_states.clear();
                        }
                    }
                    5 | 6 => {
                        if d.undo(now).unwrap() {
                            redo_states.push(states.pop().unwrap());
                        }
                    }
                    7 => {
                        if d.redo(now).unwrap() {
                            states.push(redo_states.pop().unwrap());
                        }
                    }
                    8 => d.seal(),
                    _ => d.mark_saved(),
                }
                assert_eq!(d.content(), states.last().unwrap(), "seed {seed}");
                assert_eq!(d.history().undo.len() + 1, states.len());
            }
            // Redo what is left, then undo all the way: the empty
            // document; redo all the way: the newest state again.
            while d.redo(now).unwrap() {
                states.push(redo_states.pop().unwrap());
                assert_eq!(d.content(), states.last().unwrap(), "seed {seed}");
            }
            let end = d.content().clone();
            while d.undo(now).unwrap() {}
            assert_eq!(d.content(), &Numbers::default(), "seed {seed}");
            while d.redo(now).unwrap() {}
            assert_eq!(d.content(), &end, "seed {seed}");
            assert!(d.history().redo.is_empty());
        }
    }
}
