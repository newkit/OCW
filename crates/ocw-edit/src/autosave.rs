//! Autosave (spec §14 Phase 16, 16A): every document with unsaved changes
//! is written, after a quiet moment, as one JSON record – its content and
//! its whole edit history, so undo and redo work on after a restore – into
//! an [`AutosaveStore`]. On the desktop that is the directory
//! `<data dir>/ocw/studio/autosave/` ([`DirStore`], the app picks the
//! directory); tests and, later, the browser studio use a
//! [`MemoryStore`] or a store of their own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::document::History;

/// The record format; a restore refuses others.
pub const FORMAT: u32 = 1;

/// How long a document must be left alone before it is autosaved (s).
pub const DEFAULT_DELAY_S: f64 = 5.0;

/// What an autosave holds of a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutosaveRecord<D, O> {
    pub format: u32,
    /// The document kind ([`crate::DocumentKind::KIND`]).
    pub kind: String,
    pub name: String,
    pub source: Option<String>,
    pub content: D,
    pub history: History<O>,
    /// The history state that was last saved (the dirty flag).
    pub saved_state: u64,
}

impl<D: Serialize, O: Serialize> AutosaveRecord<D, O> {
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

impl<D: serde::de::DeserializeOwned, O: serde::de::DeserializeOwned> AutosaveRecord<D, O> {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("not an autosave: {e}"))
    }
}

/// Where autosaves go: named text records.
pub trait AutosaveStore: Send + Sync {
    fn write(&mut self, name: &str, text: &str) -> Result<(), String>;
    /// `Ok(None)`: no record of that name.
    fn read(&self, name: &str) -> Result<Option<String>, String>;
    /// Removing a missing record is no error.
    fn remove(&mut self, name: &str) -> Result<(), String>;
    /// The names of all records.
    fn list(&self) -> Result<Vec<String>, String>;
}

/// Records in memory (tests; a page's storage later).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryStore {
    pub records: BTreeMap<String, String>,
    /// Every write fails with this (tests of a full disk).
    pub fail: Option<String>,
}

impl AutosaveStore for MemoryStore {
    fn write(&mut self, name: &str, text: &str) -> Result<(), String> {
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        self.records.insert(name.into(), text.into());
        Ok(())
    }

    fn read(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.records.get(name).cloned())
    }

    fn remove(&mut self, name: &str) -> Result<(), String> {
        self.records.remove(name);
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, String> {
        Ok(self.records.keys().cloned().collect())
    }
}

/// Records as `<name>` files in a directory (created on the first write).
/// A record is written to a temporary file first and then renamed, so a
/// crash during the write never leaves half a record.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq)]
pub struct DirStore {
    pub dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirStore {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> Result<std::path::PathBuf, String> {
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(format!("not a record name: {name:?}"));
        }
        Ok(self.dir.join(name))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AutosaveStore for DirStore {
    fn write(&mut self, name: &str, text: &str) -> Result<(), String> {
        let path = self.path(name)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn read(&self, name: &str) -> Result<Option<String>, String> {
        let path = self.path(name)?;
        match std::fs::read_to_string(&path) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn remove(&mut self, name: &str) -> Result<(), String> {
        let path = self.path(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn list(&self) -> Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", self.dir.display())),
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        Ok(names)
    }
}

/// The record name of a document: its kind, a readable part of its source
/// (or name) and a hash of the whole, so two packages with the same file
/// name in different folders do not share one autosave
/// (`world-liechtenstein.ocwpkg-3f2a9c11d4e5b6a7.json`).
pub fn autosave_name(kind: &str, source: &str) -> String {
    let base = source.rsplit(['/', '\\']).next().unwrap_or(source);
    let mut readable: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    readable = readable.trim_start_matches('.').to_string();
    if readable.is_empty() {
        readable.push_str("untitled");
    }
    format!("{kind}-{readable}-{:016x}.json", fnv1a(source.as_bytes()))
}

/// FNV-1a, 64 bit: a stable hash (the std hasher is not stable across
/// releases, and the name must stay the same).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{NumOp, Numbers, doc};
    use crate::{DocId, Document};

    #[test]
    fn names_are_safe_stable_and_distinct() {
        let a = autosave_name("world", "/home/ada/worlds/liechtenstein.ocwpkg");
        let b = autosave_name("world", "/home/bea/other/liechtenstein.ocwpkg");
        assert!(a.starts_with("world-liechtenstein.ocwpkg-"), "{a}");
        assert!(a.ends_with(".json"));
        assert_ne!(a, b);
        assert_eq!(
            a,
            autosave_name("world", "/home/ada/worlds/liechtenstein.ocwpkg")
        );
        // The hash is fixed (FNV-1a), not the release's std hasher.
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        let odd = autosave_name("route", "C:\\Users\\x\\..\\a b?.gpx");
        assert!(odd.starts_with("route-a_b_.gpx-"), "{odd}");
        assert!(autosave_name("model", "").starts_with("model-untitled-"));
        assert!(autosave_name("model", "...").starts_with("model-untitled-"));
    }

    #[test]
    fn a_record_round_trips_through_a_store_with_its_history() {
        let mut d = doc().with_source("/data/numbers.txt");
        d.execute(NumOp::Push(1), 0.0).unwrap();
        d.mark_saved();
        d.execute(NumOp::Push(2), 5.0).unwrap();
        d.execute(NumOp::Push(3), 10.0).unwrap();
        d.undo(11.0).unwrap();
        let mut store = MemoryStore::default();
        let name = autosave_name(
            <Numbers as crate::DocumentKind>::KIND,
            d.source.as_deref().unwrap(),
        );
        store
            .write(&name, &d.autosave_record().to_json().unwrap())
            .unwrap();
        d.mark_autosaved();
        assert!(!d.needs_autosave());

        let text = store.read(&name).unwrap().unwrap();
        let mut r: Document<Numbers> = Document::restore(
            DocId(7),
            AutosaveRecord::from_json(&text).unwrap(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(r.content(), d.content());
        assert_eq!(r.source.as_deref(), Some("/data/numbers.txt"));
        assert!(r.is_dirty(), "the saved state was before");
        assert!(!r.needs_autosave(), "it is what the autosave holds");
        // Undo and redo work on.
        assert!(r.redo(20.0).unwrap());
        assert_eq!(r.content().0, vec![1, 2, 3]);
        r.undo(21.0).unwrap();
        r.undo(22.0).unwrap();
        assert!(!r.is_dirty(), "back at the saved state");
        assert_eq!(store.list().unwrap(), vec![name.clone()]);
        store.remove(&name).unwrap();
        store.remove(&name).unwrap();
        assert_eq!(store.read(&name).unwrap(), None);
    }

    #[test]
    fn a_record_of_another_kind_or_format_is_refused() {
        let d = doc();
        let mut rec = d.autosave_record();
        rec.kind = "route".into();
        assert!(Document::<Numbers>::restore(DocId(1), rec, Vec::new()).is_err());
        let mut rec = d.autosave_record();
        rec.format = FORMAT + 1;
        assert!(Document::<Numbers>::restore(DocId(1), rec, Vec::new()).is_err());
        assert!(AutosaveRecord::<Numbers, NumOp>::from_json("{").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_directory_store_writes_reads_lists_and_removes() {
        let dir = std::env::temp_dir().join(format!(
            "ocw-edit-test-{}-{}",
            std::process::id(),
            fnv1a(b"dirstore")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = DirStore::new(dir.join("studio/autosave"));
        assert_eq!(s.list().unwrap(), Vec::<String>::new(), "no directory yet");
        assert_eq!(s.read("a.json").unwrap(), None);
        s.write("a.json", "one").unwrap();
        s.write("a.json", "two").unwrap();
        s.write("b.json", "three").unwrap();
        assert_eq!(s.read("a.json").unwrap().as_deref(), Some("two"));
        assert_eq!(s.list().unwrap(), vec!["a.json", "b.json"]);
        assert!(s.write("../x", "no").is_err());
        assert!(s.write(".hidden", "no").is_err());
        s.remove("a.json").unwrap();
        s.remove("a.json").unwrap();
        assert_eq!(s.list().unwrap(), vec!["b.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
