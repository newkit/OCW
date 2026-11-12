//! Ghost storage (spec §2.3): the best time-trial effort per course and
//! length, as a distance-over-time trace in `<data dir>/ocw/ghosts/`
//! (`OCW_DATA_DIR` overrides `<data dir>/ocw`), one small text file each:
//!
//! ```text
//! ocw-ghost 1
//! course test-hills
//! length_m 4120
//! time_s 612.340
//! 0.000 0.00
//! 0.500 0.41
//! ...
//! ```
//!
//! The header names the course and length (the file name is only a
//! sanitized copy); then one `seconds metres` pair per line, every 0.5 s
//! (a 20 min effort is about 30 KB). Only a faster effort replaces the file
//! (written through a temporary name, so a crash never leaves half a ghost).

use std::path::PathBuf;

use ocw_sim::GhostTrace;

use crate::event::{GhostKey, GhostOutcome};

const MAGIC: &str = "ocw-ghost 1";

/// `<data dir>/ocw/ghosts`; `OCW_DATA_DIR` overrides `<data dir>/ocw`.
pub fn default_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("ocw"),
    };
    Some(dir.join("ghosts"))
}

/// The ghost file format.
pub fn encode(key: &GhostKey, trace: &GhostTrace) -> String {
    let mut s = format!(
        "{MAGIC}\ncourse {}\nlength_m {}\ntime_s {:.3}\n",
        key.course,
        key.length_m,
        trace.duration_s()
    );
    for (t, d) in trace.samples() {
        s.push_str(&format!("{t:.3} {d:.2}\n"));
    }
    s
}

/// Parses a ghost file: its key and trace.
pub fn decode(text: &str) -> Result<(GhostKey, GhostTrace), String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a ghost file".into());
    }
    let mut field = |name: &str| -> Result<String, String> {
        let line = lines.next().ok_or(format!("missing {name}"))?;
        line.strip_prefix(name)
            .and_then(|r| r.strip_prefix(' '))
            .map(str::to_owned)
            .ok_or(format!("expected {name}, got {line:?}"))
    };
    let course = field("course")?;
    let length_m = field("length_m")?
        .trim()
        .parse()
        .map_err(|e| format!("length_m: {e}"))?;
    field("time_s")?;
    let mut samples = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace().map(str::parse::<f64>);
        match (it.next(), it.next()) {
            (Some(Ok(t)), Some(Ok(d))) => samples.push((t, d)),
            _ => return Err(format!("bad sample {line:?}")),
        }
    }
    let trace = GhostTrace::from_samples(samples);
    if trace.is_empty() {
        return Err("no samples".into());
    }
    Ok((GhostKey { course, length_m }, trace))
}

/// The ghosts on disk.
#[derive(Debug, Clone)]
pub struct GhostStore {
    dir: PathBuf,
}

impl GhostStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &GhostKey) -> PathBuf {
        self.dir.join(format!("{}.ghost", key.file_stem()))
    }

    /// The best effort for `key`, if any (an unreadable file counts as none).
    pub fn load(&self, key: &GhostKey) -> Option<GhostTrace> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        match decode(&text) {
            Ok((k, trace)) if k == *key => Some(trace),
            Ok((k, _)) => {
                tracing::warn!(?k, ?key, "ghost file for another course, ignored");
                None
            }
            Err(e) => {
                tracing::warn!(%e, path = %self.path(key).display(), "unreadable ghost file");
                None
            }
        }
    }

    /// Keeps `trace` if it is the first or a faster effort.
    pub fn save_if_better(&self, key: &GhostKey, trace: &GhostTrace) -> GhostOutcome {
        let time = trace.duration_s();
        let previous = self.load(key).map(|g| g.duration_s());
        if let Some(best_s) = previous
            && best_s <= time
        {
            return GhostOutcome::Slower { best_s };
        }
        let path = self.path(key);
        let tmp = path.with_extension("ghost.tmp");
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&tmp, encode(key, trace)))
            .and_then(|()| std::fs::rename(&tmp, &path));
        match (written, previous) {
            (Err(e), _) => {
                tracing::warn!(%e, path = %path.display(), "could not save the ghost");
                GhostOutcome::Failed
            }
            (Ok(()), None) => GhostOutcome::First,
            (Ok(()), Some(previous_s)) => GhostOutcome::NewBest { previous_s },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> GhostKey {
        GhostKey {
            course: "Vaduz - Malbun".into(),
            length_m: 4120,
        }
    }

    fn trace(seconds: f64) -> GhostTrace {
        GhostTrace::from_samples(
            (0..=10).map(|i| (seconds * f64::from(i) / 10.0, 412.0 * f64::from(i))),
        )
    }

    #[test]
    fn file_format_round_trips() {
        let t = trace(612.34);
        let text = encode(&key(), &t);
        assert!(
            text.starts_with("ocw-ghost 1\ncourse Vaduz - Malbun\nlength_m 4120\ntime_s 612.340\n")
        );
        let (k, back) = decode(&text).unwrap();
        assert_eq!(k, key());
        assert_eq!(back.samples().len(), 11);
        assert!((back.duration_s() - 612.34).abs() < 1e-3);
        assert!((back.length_m() - 4120.0).abs() < 1e-6);
        assert!(decode("hello").is_err());
        assert!(decode("ocw-ghost 1\ncourse x\nlength_m 1\ntime_s 1\n").is_err());
        assert!(decode("ocw-ghost 1\ncourse x\nlength_m 1\ntime_s 1\n0 a\n").is_err());
        assert!(decode("ocw-ghost 1\nlength_m 1\n").is_err());
    }

    #[test]
    fn only_a_faster_effort_replaces_the_ghost() {
        let dir = std::env::temp_dir().join(format!("ocw-ghosts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = GhostStore::new(&dir);
        assert!(store.load(&key()).is_none());
        assert_eq!(
            store.save_if_better(&key(), &trace(600.0)),
            GhostOutcome::First
        );
        assert_eq!(
            store.save_if_better(&key(), &trace(610.0)),
            GhostOutcome::Slower { best_s: 600.0 }
        );
        assert_eq!(store.load(&key()).unwrap().duration_s(), 600.0);
        assert_eq!(
            store.save_if_better(&key(), &trace(590.5)),
            GhostOutcome::NewBest { previous_s: 600.0 }
        );
        assert_eq!(store.load(&key()).unwrap().duration_s(), 590.5);
        // Another length is another ghost; the file names stay plain.
        let other = GhostKey {
            length_m: 5000,
            ..key()
        };
        assert!(store.load(&other).is_none());
        let files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, ["Vaduz_-_Malbun_4120m.ghost"]);
        // A broken file counts as no ghost.
        std::fs::write(dir.join("Vaduz_-_Malbun_4120m.ghost"), "garbage").unwrap();
        assert!(store.load(&key()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
