//! Stage profile of a world build (spec §4.7, "Build profile"): wall time,
//! CPU time (and from both the cores in use) and peak memory per stage.
//!
//! [`time`] runs a closure as a named stage (recorded per thread: the
//! thread that drives the build); stages nest (a stage inside
//! another is indented in the report). [`report`] formats what was
//! recorded, [`json`] writes it for scripts (the art journal, comparisons).
//!
//! Memory and CPU come from Linux's `/proc/self`: a sampler thread reads
//! the resident set (`VmRSS`) every 10 ms and keeps the highest since the
//! running stage started (resetting the kernel's `VmHWM` instead would
//! also reset the peak that `scripts/measure.py` reports for the whole
//! process); CPU time is `utime + stime` of `/proc/self/stat` (all
//! threads, also finished ones). Elsewhere (macOS, Windows) only wall
//! times are recorded. Costs: a small file read per stage and per 10 ms.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// One finished stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub name: String,
    /// Nesting depth (0 = top level).
    pub depth: usize,
    pub wall_s: f64,
    /// CPU time of all threads during the stage.
    pub cpu_s: Option<f64>,
    /// Peak resident set during the stage, bytes.
    pub peak_bytes: Option<u64>,
    /// Resident set at the end of the stage, bytes.
    pub end_bytes: Option<u64>,
}

impl Stage {
    /// Cores busy on average (CPU time / wall time).
    pub fn cores(&self) -> Option<f64> {
        self.cpu_s
            .filter(|_| self.wall_s > 0.0)
            .map(|c| c / self.wall_s)
    }
}

struct Frame {
    name: String,
    t0: Instant,
    cpu0: Option<f64>,
    /// The highest resident set seen inside this stage so far (before the
    /// restarts by nested stages).
    peak: Option<u64>,
}

struct Profiler {
    stack: Vec<Frame>,
    done: Vec<Stage>,
}

thread_local! {
    /// Per thread: a build records its stages on the thread that drives it
    /// (tests running builds in parallel do not mix).
    static PROFILER: RefCell<Profiler> = const {
        RefCell::new(Profiler {
            stack: Vec::new(),
            done: Vec::new(),
        })
    };
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// `VmHWM` and `VmRSS` of a `/proc/PID/status` text, bytes.
pub fn parse_status(text: &str) -> (Option<u64>, Option<u64>) {
    let kb = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|k| k * 1024)
    };
    (kb("VmHWM:"), kb("VmRSS:"))
}

/// CPU seconds (`utime + stime`) of a `/proc/PID/stat` text with `hz`
/// clock ticks per second. The command name (field 2) may hold spaces and
/// parentheses, so the fields are counted after the last `)`.
pub fn parse_stat_cpu(text: &str, hz: f64) -> Option<f64> {
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state (3), ppid (4), … utime (14), stime (15).
    let utime: f64 = f.get(11)?.parse().ok()?;
    let stime: f64 = f.get(12)?.parse().ok()?;
    Some((utime + stime) / hz)
}

/// Clock ticks per second of `/proc/self/stat` (USER_HZ: 100 on every
/// mainstream Linux architecture).
const USER_HZ: f64 = 100.0;

fn cpu_now() -> Option<f64> {
    parse_stat_cpu(&read("/proc/self/stat")?, USER_HZ)
}

fn rss_now() -> Option<u64> {
    read("/proc/self/status").and_then(|t| parse_status(&t).1)
}

/// The highest resident set the sampler saw since the last [`restart_peak`]
/// (0: none yet).
static PEAK: AtomicU64 = AtomicU64::new(0);
static SAMPLER: std::sync::Once = std::sync::Once::new();

/// Starts the sampler thread once (Linux only: elsewhere there is no RSS).
fn start_sampler() {
    if rss_now().is_none() {
        return;
    }
    SAMPLER.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("stage-rss".into())
            .spawn(|| {
                loop {
                    if let Some(r) = rss_now() {
                        PEAK.fetch_max(r, Ordering::Relaxed);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
    });
}

/// The peak since the last restart, and starts a new one at `rss`.
fn restart_peak(rss: Option<u64>) -> Option<u64> {
    let seen = PEAK.swap(rss.unwrap_or(0), Ordering::Relaxed);
    (seen > 0).then_some(seen)
}

fn peak_so_far() -> Option<u64> {
    let seen = PEAK.load(Ordering::Relaxed);
    (seen > 0).then_some(seen)
}

fn max_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Runs `f` as the stage `name` (nested inside the running stage, if any).
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    start(name);
    let out = f();
    end();
    out
}

/// Starts a stage; [`end`] finishes the innermost one.
pub fn start(name: &str) {
    PROFILER.with_borrow_mut(|p| start_on(p, name));
}

fn start_on(p: &mut Profiler, name: &str) {
    start_sampler();
    // The peak so far belongs to the enclosing stage: fold it in before
    // the restart.
    let rss = rss_now();
    let seen = max_opt(restart_peak(rss), rss);
    if let Some(parent) = p.stack.last_mut() {
        parent.peak = max_opt(parent.peak, seen);
    }
    p.stack.push(Frame {
        name: name.to_owned(),
        t0: Instant::now(),
        cpu0: cpu_now(),
        peak: None,
    });
}

/// Finishes the innermost running stage.
pub fn end() {
    PROFILER.with_borrow_mut(end_on);
}

fn end_on(p: &mut Profiler) {
    let Some(frame) = p.stack.pop() else {
        return;
    };
    let wall_s = frame.t0.elapsed().as_secs_f64();
    let cpu_s = match (frame.cpu0, cpu_now()) {
        (Some(a), Some(b)) => Some((b - a).max(0.0)),
        _ => None,
    };
    let rss = rss_now();
    let peak = max_opt(max_opt(frame.peak, peak_so_far()), rss);
    // The parent's peak includes this stage's.
    if let Some(parent) = p.stack.last_mut() {
        parent.peak = max_opt(parent.peak, peak);
    }
    let depth = p.stack.len();
    p.done.push(Stage {
        name: frame.name,
        depth,
        wall_s,
        cpu_s,
        peak_bytes: peak,
        end_bytes: rss,
    });
}

/// The stages finished so far, in the order they started (a parent before
/// its children).
pub fn stages() -> Vec<Stage> {
    PROFILER.with_borrow(|p| order(&p.done))
}

/// Forgets all recorded stages (tests, several builds in one process).
pub fn clear() {
    PROFILER.with_borrow_mut(|p| {
        p.done.clear();
        p.stack.clear();
    });
}

/// Stages are recorded when they end (children before their parent);
/// this puts each parent before its children, keeping the start order.
fn order(done: &[Stage]) -> Vec<Stage> {
    // pending[d]: finished subtrees at depth d waiting for their parent,
    // which ends after them.
    let mut pending: Vec<Vec<Vec<Stage>>> = Vec::new();
    for s in done {
        let d = s.depth;
        while pending.len() <= d + 1 {
            pending.push(Vec::new());
        }
        let mut tree = vec![s.clone()];
        for child in pending[d + 1].drain(..) {
            tree.extend(child);
        }
        pending[d].push(tree);
    }
    // Top-level trees, then children whose parent is still running.
    pending.into_iter().flatten().flatten().collect()
}

fn gb(b: Option<u64>) -> String {
    b.map_or_else(|| "–".to_owned(), |b| format!("{:.2}", b as f64 / 1e9))
}

/// The stage table: wall time, share of the total (top-level stages),
/// CPU time, cores in use, peak and end resident set; `threads` is the
/// machine's (for the utilisation of the last column).
pub fn report(stages: &[Stage], threads: usize) -> String {
    let total: f64 = stages
        .iter()
        .filter(|s| s.depth == 0)
        .map(|s| s.wall_s)
        .sum();
    let width = stages
        .iter()
        .map(|s| s.name.chars().count() + 2 * s.depth)
        .max()
        .unwrap_or(5)
        .max(5);
    let mut out = format!(
        "{:<width$}  {:>8}  {:>5}  {:>8}  {:>5}  {:>5}  {:>7}  {:>6}\n",
        "stage", "wall s", "%", "cpu s", "cores", "util", "peak GB", "end GB"
    );
    for s in stages {
        let name = format!("{}{}", "  ".repeat(s.depth), s.name);
        let share = if total > 0.0 {
            format!("{:.0}", 100.0 * s.wall_s / total)
        } else {
            "–".to_owned()
        };
        let cpu = s
            .cpu_s
            .map_or_else(|| "–".to_owned(), |c| format!("{c:.1}"));
        let (cores, util) = s.cores().map_or_else(
            || ("–".to_owned(), "–".to_owned()),
            |c| {
                (
                    format!("{c:.1}"),
                    format!("{:.0}%", 100.0 * c / threads.max(1) as f64),
                )
            },
        );
        out += &format!(
            "{name:<width$}  {:>8.2}  {share:>5}  {cpu:>8}  {cores:>5}  {util:>5}  {:>7}  {:>6}\n",
            s.wall_s,
            gb(s.peak_bytes),
            gb(s.end_bytes)
        );
    }
    let peak = stages.iter().filter_map(|s| s.peak_bytes).max();
    // The CPU time of the top-level stages that have one (None if none).
    let known: Vec<f64> = stages
        .iter()
        .filter(|s| s.depth == 0)
        .filter_map(|s| s.cpu_s)
        .collect();
    let cpu = (!known.is_empty()).then(|| known.iter().sum::<f64>());
    out += &format!(
        "{:<width$}  {total:>8.2}  {:>5}  {:>8}  {:>5}  {:>5}  {:>7}\n",
        "total",
        "100",
        cpu.map_or_else(|| "–".to_owned(), |c| format!("{c:.1}")),
        cpu.filter(|_| total > 0.0)
            .map_or_else(|| "–".to_owned(), |c| format!("{:.1}", c / total)),
        cpu.filter(|_| total > 0.0).map_or_else(
            || "–".to_owned(),
            |c| format!("{:.0}%", 100.0 * c / total / threads.max(1) as f64)
        ),
        gb(peak)
    );
    out
}

fn json_num(v: Option<f64>) -> String {
    v.map_or_else(|| "null".to_owned(), |v| format!("{v:.3}"))
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o += "\\\"",
            '\\' => o += "\\\\",
            c if (c as u32) < 0x20 => o += &format!("\\u{:04x}", c as u32),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The stages as JSON: `{"threads":N,"stages":[{"name","depth","wall_s",
/// "cpu_s","peak_bytes","end_bytes"}...]}` (absent values are null).
pub fn json(stages: &[Stage], threads: usize) -> String {
    let rows: Vec<String> = stages
        .iter()
        .map(|s| {
            format!(
                "{{\"name\":{},\"depth\":{},\"wall_s\":{:.3},\"cpu_s\":{},\"peak_bytes\":{},\"end_bytes\":{}}}",
                json_str(&s.name),
                s.depth,
                s.wall_s,
                json_num(s.cpu_s),
                s.peak_bytes.map_or_else(|| "null".to_owned(), |b| b.to_string()),
                s.end_bytes.map_or_else(|| "null".to_owned(), |b| b.to_string()),
            )
        })
        .collect();
    format!(
        "{{\"threads\":{threads},\"stages\":[{}]}}\n",
        rows.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(name: &str, depth: usize, wall: f64, cpu: Option<f64>, peak: Option<u64>) -> Stage {
        Stage {
            name: name.into(),
            depth,
            wall_s: wall,
            cpu_s: cpu,
            peak_bytes: peak,
            end_bytes: peak.map(|p| p / 2),
        }
    }

    #[test]
    fn proc_status_and_stat_are_parsed() {
        let status =
            "Name:\tworld-builder\nVmPeak:\t 9000 kB\nVmHWM:\t  7340032 kB\nVmRSS:\t  1048576 kB\n";
        assert_eq!(
            parse_status(status),
            (Some(7_340_032 * 1024), Some(1_048_576 * 1024))
        );
        assert_eq!(parse_status("Name: x\n"), (None, None));
        // The command name holds a space and a parenthesis.
        let stat = "4242 (world (b) x) R 1 4242 4242 0 -1 4194304 100 0 0 0 1234 266 0 0 20 0 17 0 100 0 0";
        assert_eq!(parse_stat_cpu(stat, 100.0), Some(15.0));
        assert_eq!(parse_stat_cpu("garbage", 100.0), None);
    }

    #[test]
    fn report_shows_nesting_shares_cores_and_peaks() {
        let s = vec![
            stage("read OSM", 0, 10.0, Some(10.0), Some(2_000_000_000)),
            stage("terrain", 0, 30.0, Some(120.0), Some(6_500_000_000)),
            stage("settle", 1, 5.0, Some(5.0), Some(6_000_000_000)),
            stage("write", 0, 0.0, None, None),
        ];
        let r = report(&s, 12);
        let lines: Vec<&str> = r.lines().collect();
        assert_eq!(lines.len(), 6, "{r}");
        assert!(lines[0].starts_with("stage"));
        // 10 of 40 s, one core of 12.
        assert!(lines[1].contains("read OSM"), "{r}");
        assert!(lines[1].contains(" 25 ") && lines[1].contains("1.0") && lines[1].contains("8%"));
        assert!(
            lines[1].contains("2.00") && lines[1].contains("1.00"),
            "{r}"
        );
        // 4 cores in the terrain stage; its child is indented.
        assert!(lines[2].contains("4.0") && lines[2].contains("33%"), "{r}");
        assert!(lines[3].starts_with("  settle"), "{r}");
        // Unknown values print as dashes.
        assert!(lines[4].contains('–'), "{r}");
        // Total: 40 s, 130 CPU s, peak 6.5 GB.
        assert!(
            lines[5].starts_with("total") && lines[5].contains("40.00"),
            "{r}"
        );
        assert!(
            lines[5].contains("130.0") && lines[5].contains("6.50"),
            "{r}"
        );
    }

    #[test]
    fn json_escapes_and_nulls() {
        let s = vec![stage("a \"b\"", 1, 1.5, None, Some(10))];
        assert_eq!(
            json(&s, 4),
            "{\"threads\":4,\"stages\":[{\"name\":\"a \\\"b\\\"\",\"depth\":1,\"wall_s\":1.500,\"cpu_s\":null,\"peak_bytes\":10,\"end_bytes\":5}]}\n"
        );
    }

    #[test]
    fn stages_nest_and_keep_their_start_order() {
        clear();
        let v = time("outer", || {
            time("first", || ());
            time("second", || time("inner", || 7))
        });
        time("after", || ());
        assert_eq!(v, 7);
        let names: Vec<(String, usize)> = stages().into_iter().map(|s| (s.name, s.depth)).collect();
        assert_eq!(
            names,
            [
                ("outer".to_owned(), 0),
                ("first".to_owned(), 1),
                ("second".to_owned(), 1),
                ("inner".to_owned(), 2),
                ("after".to_owned(), 0),
            ]
        );
        let all = stages();
        assert!(all[0].wall_s >= all[2].wall_s);
        if cfg!(target_os = "linux") {
            // A parent's peak is at least its children's.
            assert!(all[0].peak_bytes >= all[3].peak_bytes);
            assert!(all.iter().all(|s| s.cpu_s.is_some()));
        }
        clear();
    }
}
