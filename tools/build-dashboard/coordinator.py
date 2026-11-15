"""The coordinator machine (deb) on the build dashboard (spec §11.2.1).

The dashboard runs on the coordinator, where the Claude Code sessions and
their agents, the art collector, ffmpeg renders and the git worktrees live
(deb ran out of memory on 2026-11-15 at 00:39). This module watches it
locally: memory, swap and the biggest processes, OOM kills, load and
pressure (PSI), disks and the big directories, activity, alerts and a
history of samples (test-output/build-log/DAY/deb.jsonl).

Cheap by design (deb is memory-tight): standard library only, /proc and
/sys reads, one file per process (/proc/PID/stat: name and RSS; never the
command line or the environment), and the few spawned commands (du,
journalctl, systemctl) throttled and time-limited.

    python3 tools/build-dashboard/coordinator.py      # one sample as JSON
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import threading
import time
from collections import deque
from pathlib import Path

NAME = "deb"  # what the page calls the coordinator (never a host name from machines.toml)
GIB = 1024 ** 3

# All alert thresholds in one place: (amber, red). "below" ones alert when
# the value is smaller, the others when it is larger.
THRESHOLDS = {
    "mem_avail_pct_below": (25.0, 15.0),      # % of MemTotal available
    "mem_avail_bytes_below": (4 * GIB, 2 * GIB),
    "swap_used_pct": (50.0, 80.0),            # % of SwapTotal used
    "disk_pct": (85.0, 90.0),                 # df's Use% of /, /home, /home/claude, /tmp
    "oom_age_s": (7 * 86400, 86400),          # an OOM kill newer than this
    "psi_memory_some_avg60": (10.0, 30.0),    # % of time some task stalled on memory
    "load_per_core": (1.5, 3.0),              # 1-min load average / cores
    "dashboard_of_max": (0.75, 0.9),          # the dashboard's memory / its MemoryMax
}

POLL_S = 10.0               # memory, load, PSI, processes, disks
SAMPLE_S = 60.0             # one history sample (and deb.jsonl line)
DU_INTERVAL_S = 600.0       # big directory sizes, at most every 10 min
DU_TIMEOUT_S = 60.0         # per directory
TASKS_S = 30.0              # the Claude tasks (agents.jsonl, task files; git throttled itself)
SLOW_INTERVAL_S = 300.0     # journalctl (OOM events), systemctl (art timer)
CMD_TIMEOUT_S = 10.0
HISTORY_S = 24 * 3600       # kept in memory (and read back from deb.jsonl on start)
PAGE_HISTORY_S = 6 * 3600   # what the page draws
PAGE_POINTS = 180
TOP_N = 5
DISK_CANDIDATES = ("/", "/home", "/home/claude", "/tmp")
VIRTUAL_FS = {"tmpfs", "devtmpfs", "ramfs", "overlay", "squashfs"}
# systemd's "A process of this unit has been killed by the OOM killer." (SD_MESSAGE_UNIT_OOMD... UNIT_OUT_OF_MEMORY)
OOM_MESSAGE_ID = "fe6faa94e7774663a0da52717891d8ef"
ART_TIMER = "ocw-art-collect.timer"
UNIT_FILE = Path(__file__).resolve().parent / "ocw-build-dashboard.service"

# --------------------------------------------------------------------------
# Parsers (pure; fixture text in the tests)
# --------------------------------------------------------------------------


def parse_meminfo(text: str) -> dict:
    """/proc/meminfo -> bytes: total, available, used (= total - available),
    free, shmem (tmpfs such as /tmp lives there), swap_total, swap_used."""
    kv = {}
    for line in text.splitlines():
        m = re.match(r"(\w+(?:\(\w+\))?):\s+(\d+)(?:\s+kB)?", line)
        if m:
            kv[m.group(1)] = int(m.group(2)) * 1024
    total = kv.get("MemTotal")
    if not total:
        return {}
    avail = kv.get("MemAvailable", kv.get("MemFree", 0))
    out = {"total": total, "available": avail, "used": total - avail, "free": kv.get("MemFree"),
           "shmem": kv.get("Shmem"), "swap_total": kv.get("SwapTotal", 0),
           "swap_used": kv.get("SwapTotal", 0) - kv.get("SwapFree", 0)}
    out["available_pct"] = round(100.0 * avail / total, 1)
    out["swap_pct"] = round(100.0 * out["swap_used"] / out["swap_total"], 1) if out["swap_total"] else 0.0
    return out


def parse_psi(text: str) -> dict:
    """/proc/pressure/{cpu,memory,io}: {"some": {"avg10", "avg60", "avg300"}, "full": {...}}."""
    out = {}
    for line in text.splitlines():
        f = line.split()
        if not f or f[0] not in ("some", "full"):
            continue
        vals = {}
        for kv in f[1:]:
            k, _, v = kv.partition("=")
            if k.startswith("avg"):
                try:
                    vals[k] = float(v)
                except ValueError:
                    pass
        out[f[0]] = vals
    return out


def parse_loadavg(text: str):
    try:
        out = [float(x) for x in text.split()[:3]]
    except ValueError:
        return None
    return out if len(out) == 3 else None


def _unoctal(s: str) -> str:
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), s)


def parse_mounts(text: str, wanted=DISK_CANDIDATES) -> list:
    """The mounts among `wanted` that are their own mount point (the last
    entry of a mount point wins, as the kernel stacks them), in the order of
    `wanted`, deduplicated by device (/home on the root's device is left
    out; a doubled /home/claude counts once). Virtual file systems (tmpfs)
    are told apart by their mount point. [{"mount", "device", "fstype"}]."""
    last = {}
    for line in text.splitlines():
        f = line.split()
        if len(f) < 3:
            continue
        mp = _unoctal(f[1])
        if mp in wanted:
            last[mp] = {"mount": mp, "device": _unoctal(f[0]), "fstype": f[2]}
    out, seen = [], set()
    for mp in wanted:
        e = last.get(mp)
        if not e:
            continue
        key = (e["device"], mp) if e["fstype"] in VIRTUAL_FS else e["device"]
        if key in seen:
            continue
        seen.add(key)
        out.append(e)
    return out


def disk_usage(mount: dict, statvfs=os.statvfs):
    """df's numbers for a mount: total, used, free (for users), pct = used / (used + free)."""
    try:
        s = statvfs(mount["mount"])
    except OSError:
        return None
    total = s.f_blocks * s.f_frsize
    free = s.f_bavail * s.f_frsize
    used = (s.f_blocks - s.f_bfree) * s.f_frsize
    pct = round(100.0 * used / (used + free), 1) if used + free else None
    return {"mount": mount["mount"], "fstype": mount["fstype"], "total": total, "used": used, "free": free,
            "pct": pct, "in_ram": mount["fstype"] in ("tmpfs", "ramfs")}


def parse_stat(text: str):
    """/proc/PID/stat -> (pid, comm, ppid, rss pages); the name may hold
    spaces and parentheses, so it ends at the last ')'."""
    try:
        a, b = text.index("("), text.rindex(")")
        pid = int(text[:a])
        comm = text[a + 1:b]
        rest = text[b + 2:].split()
        return pid, comm, int(rest[1]), int(rest[21])
    except (ValueError, IndexError):
        return None


_VERSION_COMM = re.compile(r"^\d+\.\d+\.\d+$")  # Claude Code's binary is named after its version
BROWSERS = {"chrome", "chromium", "chromium-browse", "headless_shell", "chrome_crashpad", "firefox",
            "firefox-esr", "firefox-bin", "Web Content", "WebExtensions", "Isolated Web Co", "Privileged Cont",
            "RDD Process", "Socket Process", "GPU Process", "Utility Process"}
GROUPS = ("claude", "cargo/rustc", "ffmpeg", "python", "qemu", "browsers")


def process_kind(comm: str, exe: str = "") -> str:
    """The group of a process name: claude, cargo/rustc, ffmpeg, python,
    qemu, browsers or other."""
    if comm == "claude" or (_VERSION_COMM.match(comm) and "/claude/" in exe):
        return "claude"
    if comm in ("cargo", "rustc", "sccache", "rust-analyzer", "rustup", "clippy-driver") or comm.startswith("cargo-"):
        return "cargo/rustc"
    if comm in ("ffmpeg", "ffprobe"):
        return "ffmpeg"
    if re.match(r"^python(\d(\.\d+)?)?$", comm):
        return "python"
    if comm.startswith("qemu") and comm != "qemu-ga":  # qemu-ga: the guest agent of deb itself, no VM
        return "qemu"
    if comm in BROWSERS or comm.startswith("chrom"):
        return "browsers"
    return "other"


def summarize_procs(procs: list, page: int = 4096) -> dict:
    """procs: [(pid, comm, ppid, rss_pages, kind)] -> the top processes by
    RSS (name and RSS only), group sums, Claude Code sessions (claude
    processes whose parent is no claude process), ffmpeg and QEMU counts."""
    kind_of = {p[0]: p[4] for p in procs}
    groups = {g: {"count": 0, "rss": 0} for g in GROUPS + ("other",)}
    total = 0
    for pid, comm, ppid, rss, kind in procs:
        b = rss * page
        total += b
        groups[kind]["count"] += 1
        groups[kind]["rss"] += b
    top = sorted((p for p in procs if p[3] > 0), key=lambda p: -p[3])[:TOP_N]
    sessions = [p for p in procs if p[4] == "claude" and p[1] == "claude" and kind_of.get(p[2]) != "claude"]
    return {
        "top": [{"name": ("claude " + p[1]) if p[4] == "claude" and p[1] != "claude" else p[1],
                 "rss": p[3] * page, "kind": p[4]} for p in top],
        "groups": groups,
        "rss_total": total,
        "claude_sessions": len(sessions),
        "claude_session_rss": [p[3] * page for p in sessions],
        "ffmpeg": sum(1 for p in procs if p[1] == "ffmpeg"),
        "qemu": sum(1 for p in procs if p[4] == "qemu"),
        "processes": len(procs),
    }


def scan_procs(proc_root="/proc") -> list:
    """Every process's name, parent and RSS from /proc/PID/stat (one small
    read each). Never reads cmdline or environ."""
    out = []
    try:
        names = os.listdir(proc_root)
    except OSError:
        return out
    for n in names:
        if not n.isdigit():
            continue
        try:
            with open(os.path.join(proc_root, n, "stat"), "r", errors="replace") as f:
                st = parse_stat(f.read())
        except OSError:
            continue
        if not st:
            continue
        pid, comm, ppid, rss = st
        exe = ""
        if _VERSION_COMM.match(comm):  # only for "2.1.325"-like names: is it Claude Code?
            try:
                exe = os.readlink(os.path.join(proc_root, n, "exe"))
            except OSError:
                pass
        out.append((pid, comm, ppid, rss, process_kind(comm, exe)))
    return out


# OOM events -----------------------------------------------------------------

_KILLED = re.compile(r"Killed process (\d+) \(([^)]*)\)")
_UNIT_OOM = re.compile(r"^(\S+): A process of this unit has been killed by the OOM killer")
_SCOPE_UUID = re.compile(r"-([0-9a-f]{8})-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


def short_unit(unit: str) -> str:
    """tmux-spawn-75bb77d1-0461-….scope -> tmux-spawn-75bb77d1….scope."""
    return _SCOPE_UUID.sub(lambda m: "-" + m.group(1) + "…", unit)


def parse_journal_oom(lines, source="user journal") -> list:
    """journalctl -o json lines -> [{"t", "name", "source"}]: systemd's
    unit OOM messages (the unit) and kernel "Killed process N (name)"."""
    out = []
    for line in lines:
        try:
            e = json.loads(line)
        except ValueError:
            continue
        msg = e.get("MESSAGE")
        if not isinstance(msg, str):
            continue
        try:
            t = int(e.get("__REALTIME_TIMESTAMP")) / 1e6
        except (TypeError, ValueError):
            continue
        m = _KILLED.search(msg)
        if m:
            out.append({"t": t, "name": m.group(2), "source": source})
            continue
        m = _UNIT_OOM.match(msg)
        if m:
            out.append({"t": t, "name": short_unit(m.group(1)), "source": source})
    return out


def parse_dmesg_oom(text: str, boot_time: float) -> list:
    """`dmesg` lines "[12345.678] Out of memory: Killed process 1 (x) …"."""
    out = []
    for line in text.splitlines():
        m = _KILLED.search(line)
        if not m:
            continue
        ts = re.match(r"\s*\[\s*(\d+\.\d+)\]", line)
        out.append({"t": boot_time + float(ts.group(1)) if ts else None, "name": m.group(2), "source": "dmesg"})
    return out


def merge_oom(events: list, window: float = 90.0) -> list:
    """Newest first; the same name within `window` s from two sources once."""
    out = []
    for e in sorted(events, key=lambda e: -(e.get("t") or 0)):
        if any(o["name"] == e["name"] and abs((o.get("t") or 0) - (e.get("t") or 0)) < window for o in out):
            continue
        out.append(e)
    return out


def parse_vmstat_oom(text: str):
    m = re.search(r"^oom_kill (\d+)$", text, re.M)
    return int(m.group(1)) if m else None


def parse_systemctl_show(text: str) -> dict:
    return dict(line.split("=", 1) for line in text.splitlines() if "=" in line)


def parse_memory_size(s: str):
    """systemd/cgroup sizes: "1073741824", "1G", "768M", "max"/"infinity" -> None."""
    s = (s or "").strip()
    if not s or s in ("max", "infinity"):
        return None
    m = re.match(r"^(\d+(?:\.\d+)?)([KMGT]?)$", s)
    if not m:
        return None
    return int(float(m.group(1)) * 1024 ** " KMGT".index(m.group(2) or " "))


def unit_memory_limits(text: str) -> dict:
    """MemoryMax / MemoryHigh of a unit file."""
    out = {}
    for line in text.splitlines():
        m = re.match(r"^\s*(MemoryMax|MemoryHigh)\s*=\s*(\S+)", line)
        if m:
            out[m.group(1)] = parse_memory_size(m.group(2))
    return out


# Alerts ---------------------------------------------------------------------


def _lvl(value, warn, bad, below=False):
    if value is None:
        return None
    if below:
        return "bad" if value < bad else "warn" if value < warn else None
    return "bad" if value > bad else "warn" if value > warn else None


def _g(n) -> str:
    return "?" if n is None else "%.1f GiB" % (n / GIB)


def evaluate_alerts(st: dict, now: float, th=THRESHOLDS) -> list:
    """[{"level": "bad"|"warn", "kind", "text"}] for a coordinator status."""
    out = []

    def add(level, kind, text):
        if level:
            out.append({"machine": NAME, "level": level, "kind": kind, "text": "%s: %s" % (NAME, text)})

    mem = st.get("memory") or {}
    if mem.get("total"):
        a, pct = mem["available"], mem["available_pct"]
        lv = [_lvl(pct, *th["mem_avail_pct_below"], below=True), _lvl(a, *th["mem_avail_bytes_below"], below=True)]
        level = "bad" if "bad" in lv else "warn" if "warn" in lv else None
        add(level, "memory", "memory available %s (%.0f %% of %s)" % (_g(a), pct, _g(mem["total"])))
        if mem.get("swap_total"):
            add(_lvl(mem["swap_pct"], *th["swap_used_pct"]), "swap",
                "swap %.0f %% used (%s of %s)" % (mem["swap_pct"], _g(mem["swap_used"]), _g(mem["swap_total"])))
    for d in st.get("disks") or []:
        add(_lvl(d.get("pct"), *th["disk_pct"]), "disk",
            "disk %s %.0f %% full (%s free)" % (d["mount"], d["pct"] or 0, _g(d.get("free"))))
    newest = next((e for e in (st.get("oom") or {}).get("events") or [] if e.get("t")), None)
    if newest:
        age = now - newest["t"]
        warn_s, bad_s = th["oom_age_s"]
        level = "bad" if age < bad_s else "warn" if age < warn_s else None
        add(level, "oom", "OOM killer stopped %s %s ago" % (newest["name"], _ago(age)))
    psi = ((st.get("pressure") or {}).get("memory") or {}).get("some") or {}
    add(_lvl(psi.get("avg60"), *th["psi_memory_some_avg60"]), "pressure",
        "memory pressure: tasks stalled %.0f %% of the last minute" % (psi.get("avg60") or 0))
    load, ncpu = st.get("load"), st.get("ncpu")
    if load and ncpu:
        add(_lvl(load[0] / ncpu, *th["load_per_core"]), "load", "load %.2f on %d cores" % (load[0], ncpu))
    dm = st.get("dashboard") or {}
    if dm.get("used") and dm.get("max"):
        add(_lvl(dm["used"] / dm["max"], *th["dashboard_of_max"]), "dashboard",
            "the dashboard uses %s of its MemoryMax %s" % (_g(dm["used"]), _g(dm["max"])))
    cr = ((st.get("processes") or {}).get("groups") or {}).get("cargo/rustc") or {}
    if cr.get("count"):
        add("warn", "cargo", "%d cargo/rustc process(es) running on the coordinator (builds belong on the build "
            "machines)" % cr["count"])
    return out


def _ago(s: float) -> str:
    s = int(max(0, s))
    if s >= 86400:
        return "%dd %dh" % (s // 86400, s % 86400 // 3600)
    if s >= 3600:
        return "%dh %02dm" % (s // 3600, s % 3600 // 60)
    return "%dm" % (s // 60)


# History ----------------------------------------------------------------------


class History:
    """A ring of compact samples (24 h at one per minute), written as one
    JSON line per sample to ROOT/DAY/deb.jsonl and read back on start."""

    FIELDS = ("t", "mem_avail", "mem_total", "swap_used", "load1", "root_free", "root_pct")

    def __init__(self, root: Path | None = None, keep_s: float = HISTORY_S):
        self.root = root
        self.keep_s = keep_s
        self.ring = deque(maxlen=int(keep_s / SAMPLE_S) + 60)
        self.lock = threading.Lock()

    @staticmethod
    def point(line: dict):
        """The ring's tuple of a deb.jsonl line (None: not one)."""
        try:
            t = float(line["t"])
        except (KeyError, TypeError, ValueError):
            return None
        mem = line.get("memory") or {}
        root = next((d for d in line.get("disks") or [] if d.get("mount") == "/"), {})
        load = line.get("load") or [None]
        return (t, mem.get("available"), mem.get("total"), mem.get("swap_used"), load[0], root.get("free"),
                root.get("pct"))

    def add(self, line: dict, write: bool = True):
        p = self.point(line)
        if p is None:
            return
        with self.lock:
            self.ring.append(p)
            self._trim(p[0])
        if write and self.root is not None:
            path = self.path(p[0])
            path.parent.mkdir(parents=True, exist_ok=True)
            with open(path, "a", encoding="utf-8") as f:
                f.write(json.dumps(line, separators=(",", ":"), sort_keys=True) + "\n")

    def _trim(self, now):
        while self.ring and self.ring[0][0] < now - self.keep_s:
            self.ring.popleft()

    def path(self, t: float) -> Path:
        return self.root / time.strftime("%Y-%m-%d", time.localtime(t)) / "deb.jsonl"

    def load(self, now: float | None = None) -> int:
        """Reads the days of the last `keep_s` back (bad lines are skipped)."""
        if self.root is None:
            return 0
        now = time.time() if now is None else now
        days = sorted({time.strftime("%Y-%m-%d", time.localtime(now - k * 3600))
                       for k in range(0, int(self.keep_s // 3600) + 2)})
        pts = []
        for day in days:
            f = self.root / day / "deb.jsonl"
            try:
                text = f.read_text(encoding="utf-8")
            except OSError:
                continue
            for line in text.splitlines():
                try:
                    p = self.point(json.loads(line))
                except ValueError:
                    continue
                if p and now - self.keep_s <= p[0] <= now + 60:
                    pts.append(p)
        with self.lock:
            for p in sorted(pts):
                self.ring.append(p)
        return len(pts)

    def series(self, now: float, span: float = PAGE_HISTORY_S, points: int = PAGE_POINTS) -> dict:
        """The page's sparklines: the last `span` s, at most `points` per
        series (the minimum of memory available and disk free, the maximum of
        load in each bucket: the worst moment stays visible)."""
        with self.lock:
            pts = [p for p in self.ring if p[0] >= now - span]
        if not pts:
            return {"t": [], "mem_avail": [], "load1": [], "root_free": [], "span": span}
        step = span / points
        buckets = {}
        for p in pts:
            buckets.setdefault(int((p[0] - (now - span)) // step), []).append(p)
        t, mem, load, root = [], [], [], []
        pick = lambda xs, f: f(x for x in xs if x is not None) if any(x is not None for x in xs) else None  # noqa: E731
        for k in sorted(buckets):
            b = buckets[k]
            t.append(round(b[-1][0]))
            mem.append(pick([x[1] for x in b], min))
            load.append(pick([x[4] for x in b], max))
            root.append(pick([x[5] for x in b], min))
        return {"t": t, "mem_avail": mem, "load1": load, "root_free": root, "span": span,
                "mem_total": pts[-1][2]}


# --------------------------------------------------------------------------
# The collector
# --------------------------------------------------------------------------


def home_path(p, home=None) -> str:
    """A path under the home directory as ~/…"""
    home = str(home or Path.home()).rstrip("/")
    s = str(p)
    return "~" + s[len(home):] if s == home or s.startswith(home + "/") else s


def _read(path, default=""):
    try:
        with open(path, "r", errors="replace") as f:
            return f.read()
    except OSError:
        return default


def _run(argv, timeout=CMD_TIMEOUT_S):
    """(returncode, stdout) of a short read-only command; None if it could not run."""
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, errors="replace",
                           stdin=subprocess.DEVNULL)
        return r.returncode, r.stdout
    except (OSError, subprocess.SubprocessError):
        return None


def _can_read_system_journal() -> bool:
    if os.geteuid() == 0:
        return True
    try:
        import grp
        names = {grp.getgrgid(g).gr_name for g in os.getgroups()}
    except (ImportError, KeyError):
        return False
    return bool(names & {"adm", "systemd-journal", "wheel"})


def big_dirs(main_repo: Path, art_repo: Path | None) -> list:
    """The directories whose size the card shows: (label, path)."""
    out = [("OCW target/", main_repo / "target"), ("test-output/", main_repo / "test-output"),
           ("worktrees (.claude/worktrees/)", main_repo / ".claude" / "worktrees")]
    if art_repo is not None:
        out += [("OCW-ART build-history/", art_repo / "build-history"), ("OCW-ART .git (with Git LFS)", art_repo / ".git")]
    return out


def du_bytes(path: Path, timeout: float = DU_TIMEOUT_S):
    """Disk use of a tree with du, niced and time-limited: (bytes, error)."""
    if not path.exists():
        return None, "not there"
    argv = ["du", "-sx", "-B1", str(path)]
    if shutil.which("ionice"):
        argv = ["ionice", "-c3"] + argv
    if shutil.which("nice"):
        argv = ["nice", "-n", "19"] + argv
    r = _run(argv, timeout)
    if r is None:
        return None, "timed out after %d s" % timeout
    try:
        return int(r[1].split()[0]), None
    except (IndexError, ValueError):
        return None, "du failed"


def count_worktrees(main_repo: Path) -> dict:
    """Linked worktrees of a repository, from .git/worktrees/*/gitdir (no
    git process): the live ones (their directory exists) and the stale ones."""
    live = stale = claude = 0
    wt = main_repo / ".git" / "worktrees"
    try:
        entries = list(os.scandir(wt))
    except OSError:
        entries = []
    for e in entries:
        target = _read(os.path.join(e.path, "gitdir")).strip()
        if target and os.path.exists(target):
            live += 1
            claude += "/.claude/worktrees/" in target
        else:
            stale += 1
    return {"linked": live, "stale": stale, "claude": claude}


class Coordinator:
    """Collects the coordinator's state in its own thread; `status()` is
    cheap (called by every page refresh)."""

    def __init__(self, main_repo: Path, log_root: Path | None = None, art_repo: Path | None = None,
                 proc_root: str = "/proc", home=None):
        self.main_repo = main_repo
        self.art_repo = art_repo
        self.proc_root = proc_root
        self.home = home or Path.home()
        self.history = History(log_root)
        self.lock = threading.Lock()
        self.latest: dict = {}
        self.dirs: dict = {"at": None, "items": []}
        self.slow: dict = {"at": None}
        self.oom_count = None
        self._du_running = False
        self.stop = threading.Event()
        self.unit_limits = unit_memory_limits(_read(UNIT_FILE))
        self.tasks = None  # a claude_tasks.ClaudeTasks, refreshed in this thread
        self.tasks_at = 0.0

    # -- the parts -------------------------------------------------------
    def read_fast(self) -> dict:
        p = self.proc_root
        st = {"name": NAME, "at": time.time(), "available": os.path.exists(os.path.join(p, "meminfo"))}
        st["memory"] = parse_meminfo(_read(os.path.join(p, "meminfo")))
        st["load"] = parse_loadavg(_read(os.path.join(p, "loadavg")))
        st["ncpu"] = os.cpu_count()
        st["pressure"] = {k: parse_psi(_read(os.path.join(p, "pressure", k))) for k in ("cpu", "memory", "io")}
        if not any(st["pressure"].values()):
            st["pressure"] = None
        st["disks"] = [d for d in (disk_usage(m) for m in parse_mounts(_read(os.path.join(p, "mounts")))) if d]
        for d in st["disks"]:
            d["label"] = home_path(d["mount"], self.home)
        st["processes"] = summarize_procs(scan_procs(p), os.sysconf("SC_PAGE_SIZE") if hasattr(os, "sysconf") else 4096)
        st["oom_kill_since_boot"] = parse_vmstat_oom(_read(os.path.join(p, "vmstat")))
        bt = re.search(r"^btime (\d+)", _read(os.path.join(p, "stat")), re.M)
        st["boot_time"] = int(bt.group(1)) if bt else None
        st["worktrees"] = count_worktrees(self.main_repo)
        st["dashboard"] = self.self_memory()
        return st

    def self_memory(self) -> dict:
        """The dashboard's RSS, its cgroup's memory.current and the limits
        (the cgroup's memory.max when it runs as the unit, else the unit file's)."""
        rss = re.search(r"^VmRSS:\s+(\d+) kB", _read(os.path.join(self.proc_root, "self", "status")), re.M)
        out = {"rss": int(rss.group(1)) * 1024 if rss else None, "used": None,
               "max": self.unit_limits.get("MemoryMax"), "high": self.unit_limits.get("MemoryHigh"),
               "in_unit": False}
        cg = _read(os.path.join(self.proc_root, "self", "cgroup")).strip()
        rel = cg.split("::", 1)[1] if "::" in cg else ""
        if rel.endswith("/ocw-build-dashboard.service"):
            base = "/sys/fs/cgroup" + rel
            cur = parse_memory_size(_read(base + "/memory.current"))
            mx, hi = parse_memory_size(_read(base + "/memory.max")), parse_memory_size(_read(base + "/memory.high"))
            out.update(in_unit=True, used=cur, max=mx or out["max"], high=hi or out["high"])
        if out["used"] is None:
            out["used"] = out["rss"]
        return out

    def read_slow(self) -> dict:
        """OOM events and the art timer (journalctl / systemctl, time-limited)."""
        out = {"at": time.time(), "oom_sources": {}}
        events = []
        r = _run(["journalctl", "--user", "-q", "--no-pager", "-o", "json", "--since=-7d",
                  "MESSAGE_ID=" + OOM_MESSAGE_ID]) if shutil.which("journalctl") else None
        if r is None or r[0] not in (0, 1):
            out["oom_sources"]["user journal"] = "not readable"
        else:
            out["oom_sources"]["user journal"] = "ok"
            events += parse_journal_oom(r[1].splitlines())
        if shutil.which("journalctl") and _can_read_system_journal():
            r = _run(["journalctl", "-k", "-q", "--no-pager", "-o", "json", "--since=-7d", "-g", "Killed process"])
            ok = r is not None and r[0] in (0, 1)
            out["oom_sources"]["kernel log"] = "ok" if ok else "not readable"
            if ok:
                events += parse_journal_oom(r[1].splitlines(), "kernel log")
        else:
            out["oom_sources"]["kernel log"] = "not readable (needs group adm or systemd-journal)"
        if _read("/proc/sys/kernel/dmesg_restrict").strip() == "1" and os.geteuid() != 0:
            out["oom_sources"]["dmesg"] = "not readable (kernel.dmesg_restrict)"
        else:
            r = _run(["dmesg"]) if shutil.which("dmesg") else None
            ok = r is not None and r[0] == 0
            out["oom_sources"]["dmesg"] = "ok" if ok else "not readable"
            if ok:
                bt = (self.latest or {}).get("boot_time") or 0
                events += parse_dmesg_oom(r[1], bt)
        out["oom_events"] = merge_oom(events)[:10]
        if shutil.which("systemctl"):
            r = _run(["systemctl", "--user", "show", ART_TIMER, "-p", "ActiveState", "-p", "NextElapseUSecRealtime",
                      "-p", "LastTriggerUSec", "-p", "LoadState"], 5)
            if r is not None and r[0] == 0:
                kv = parse_systemctl_show(r[1])
                out["art_timer"] = {"state": kv.get("ActiveState") if kv.get("LoadState") != "not-found" else "not installed",
                                    "next": kv.get("NextElapseUSecRealtime") or None,
                                    "last": kv.get("LastTriggerUSec") or None}
        return out

    def read_dirs(self):
        items = []
        for label, path in big_dirs(self.main_repo, self.art_repo):
            b, err = du_bytes(path)
            items.append({"label": label, "path": home_path(path, self.home), "bytes": b, "error": err})
        with self.lock:
            self.dirs = {"at": time.time(), "items": items}
        self._du_running = False

    # -- loop ------------------------------------------------------------
    def tick(self, now: float | None = None, sample: bool = False, block: bool = False):
        now = time.time() if now is None else now
        fast = self.read_fast()
        oom = fast.get("oom_kill_since_boot")
        new_oom = self.oom_count is not None and oom is not None and oom > self.oom_count
        self.oom_count = oom
        with self.lock:
            self.latest = fast
        if new_oom or not self.slow.get("at") or now - self.slow["at"] >= SLOW_INTERVAL_S:
            slow = self.read_slow()
            with self.lock:
                self.slow = slow
        if not self._du_running and (not self.dirs["at"] or now - self.dirs["at"] >= DU_INTERVAL_S):
            self._du_running = True
            if block:
                self.read_dirs()
            else:
                threading.Thread(target=self.read_dirs, daemon=True, name="coordinator-du").start()
        if self.tasks is not None and (sample or now - self.tasks_at >= TASKS_S):
            try:
                self.tasks.refresh(now)
            finally:
                self.tasks_at = now
        if sample:
            self.history.add(self.sample_line(now))

    def sample_line(self, now: float) -> dict:
        """One deb.jsonl line (see the README): no command lines, no hosts."""
        st = self.status(now, with_history=False)
        lt = time.localtime(now)
        off = time.strftime("%z", lt)
        p = st.get("processes") or {}
        return {"time": time.strftime("%Y-%m-%dT%H:%M:%S", lt) + (off[:3] + ":" + off[3:] if off else ""),
                "t": round(now, 1), "memory": st.get("memory"), "load": st.get("load"), "ncpu": st.get("ncpu"),
                "psi": {k: ((v or {}).get("some") or {}).get("avg60") for k, v in (st.get("pressure") or {}).items()},
                "disks": [{k: d.get(k) for k in ("mount", "free", "pct")} for d in st.get("disks") or []],
                "groups": {k: v["rss"] for k, v in (p.get("groups") or {}).items() if v["count"]},
                "top": [[x["name"], x["rss"]] for x in p.get("top") or []],
                "claude_sessions": p.get("claude_sessions"), "ffmpeg": p.get("ffmpeg"), "qemu": p.get("qemu"),
                "oom_kill_since_boot": st.get("oom_kill_since_boot"),
                "dashboard_rss": (st.get("dashboard") or {}).get("used"),
                "alerts": ["%s %s" % (a["level"], a["kind"]) for a in st.get("alerts") or []]}

    def run(self, poll: float = POLL_S, sample: float = SAMPLE_S):
        self.history.load()
        last = 0.0
        while not self.stop.is_set():
            now = time.time()
            due = now - last >= sample
            try:
                self.tick(now, sample=due)
            except Exception as e:  # never stop the dashboard
                import sys
                print("coordinator: %s: %s" % (type(e).__name__, e), file=sys.stderr)
            if due:
                last = now
            self.stop.wait(poll)

    def start(self):
        threading.Thread(target=self.run, daemon=True, name="coordinator").start()

    # -- status ----------------------------------------------------------
    def status(self, now: float | None = None, with_history: bool = True) -> dict:
        now = time.time() if now is None else now
        with self.lock:
            st = dict(self.latest)
            slow, dirs = dict(self.slow), dict(self.dirs)
        if not st:
            return {"name": NAME, "available": False, "pending": True}
        st["oom"] = {"events": slow.get("oom_events") or [], "sources": slow.get("oom_sources") or {},
                     "since_boot": st.get("oom_kill_since_boot"), "checked_at": slow.get("at")}
        st["art_timer"] = slow.get("art_timer")
        st["dirs"] = dirs
        st["thresholds"] = THRESHOLDS
        st["alerts"] = evaluate_alerts(st, now)
        if with_history:
            st["history"] = self.history.series(now)
        return st


# --------------------------------------------------------------------------
# Text (--once)
# --------------------------------------------------------------------------


def _b(n) -> str:
    if n is None:
        return "?"
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(n) < 1024 or unit == "TiB":
            return ("%.0f %s" if unit in ("B", "KiB") else "%.1f %s") % (n, unit)
        n /= 1024
    return "?"


def text_lines(st: dict, now: float | None = None) -> list:
    now = time.time() if now is None else now
    if not st or not st.get("available"):
        return ["Coordinator (%s): not available on this system" % NAME, ""]
    mem = st.get("memory") or {}
    out = ["Coordinator (%s): memory %s used of %s (%s available, %.0f %%), swap %s of %s" % (
        NAME, _b(mem.get("used")), _b(mem.get("total")), _b(mem.get("available")), mem.get("available_pct") or 0,
        _b(mem.get("swap_used")), _b(mem.get("swap_total")))]
    load = st.get("load") or [0, 0, 0]
    psi = st.get("pressure") or {}
    ps = ", ".join("%s %.1f %%" % (k, ((psi.get(k) or {}).get("some") or {}).get("avg60") or 0) for k in ("cpu", "memory", "io")) \
        if psi else "n/a"
    out.append("  cpu: load %.2f %.2f %.2f on %s cores; pressure (some, avg60): %s" % (
        load[0], load[1], load[2], st.get("ncpu"), ps))
    out.append("  disks: " + ", ".join("%s %s%s %% (%s free)" % (
        d["label"], "tmpfs in RAM " if d.get("in_ram") else "", "?" if d.get("pct") is None else "%.0f" % d["pct"],
        _b(d.get("free"))) for d in st.get("disks") or []))
    p = st.get("processes") or {}
    out.append("  top: " + ", ".join("%s %s" % (x["name"], _b(x["rss"])) for x in p.get("top") or []))
    out.append("  groups: " + ", ".join("%s %s (%d)" % (k, _b(v["rss"]), v["count"])
                                        for k, v in (p.get("groups") or {}).items() if v["count"] and k != "other"))
    wt = st.get("worktrees") or {}
    art = st.get("art_timer") or {}
    dm = st.get("dashboard") or {}
    out.append("  activity: %s Claude Code session(s), %s worktree(s) of OCW (+ main)%s, %s ffmpeg, %s QEMU VM(s), "
               "art timer %s%s, dashboard %s of %s" % (
                   p.get("claude_sessions"), wt.get("linked"), (" + %d stale" % wt["stale"]) if wt.get("stale") else "",
                   p.get("ffmpeg"), p.get("qemu"), art.get("state") or "?",
                   (" (next %s)" % art["next"]) if art.get("next") else "", _b(dm.get("used")), _b(dm.get("max"))))
    oom = st.get("oom") or {}
    ev = oom.get("events") or []
    unread = [k for k, v in (oom.get("sources") or {}).items() if v != "ok"]
    out.append("  OOM kills: %s since boot; %s%s" % (
        oom.get("since_boot"), "; ".join("%s %s" % (time.strftime("%m-%d %H:%M", time.localtime(e["t"])), e["name"])
                                         for e in ev[:5] if e.get("t")) or "none in the last 7 days",
        ("; not readable: " + ", ".join(unread)) if unread else ""))
    items = (st.get("dirs") or {}).get("items") or []
    if items:
        out.append("  dirs: " + ", ".join("%s %s" % (i["path"], _b(i["bytes"]) if i["bytes"] is not None else i["error"])
                                          for i in items))
    for a in st.get("alerts") or []:
        out.insert(0, ("ALERT " if a["level"] == "bad" else "WARN ") + a["text"])
    return out + [""]


if __name__ == "__main__":
    c = Coordinator(Path(__file__).resolve().parents[2])
    c.tick(block=True)
    print(json.dumps(c.status(), indent=2, default=str))
