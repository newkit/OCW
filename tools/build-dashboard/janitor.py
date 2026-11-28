"""The janitor of the build machines' disks (spec 11.2).

A thread of the build dashboard. It applies the cleanup rules of
cleanup_rules.py (the same rule set `scripts/remote-gc.sh` uses: every
checkout class, leftovers of failed deletions, the web tests' browser
profiles, old release output) on a schedule, and frees more when a disk
fills up:

* routine pass: once a day (`OCW_JANITOR_ROUTINE_H`, default 24; 0 = never)
  on every enabled, reachable machine – everything the rules allow;
* when the checkouts' disk is above 75 % or has less than 40 GB free
  (`OCW_JANITOR_FREE_GB`): a pass at once (at most one an hour per machine,
  also on machines disabled for building), which besides the rules sweeps
  the main checkout's `target/`: build units (cargo's `<crate>-<hash>`
  artefacts: fingerprint, deps files, build-script output) unused for 3 days
  that a newer unit of the same crate has replaced, plus incremental session
  directories unused for 3 days (like cargo-sweep);
* above 85 %: also the `target/` of the least recently used agent, merge and
  other checkouts idle for more than 24 h (sources kept), oldest first,
  until the disk is estimated below 85 % again;
* above 92 %: an alert (red banner in the dashboard, a log line).

Never touched: the main checkout's sources, `<repo>-HEAD`, checkouts with a
running or waiting remote.sh task (checked again on the machine right
before each deletion), the sccache cache, world data, a checkout's
test-output/ and data/ (a checkout holding files there is never removed as
a whole). Every deletion (and, in dry-run mode, every planned one) is
logged as a JSON line to test-output/build-log/DAY/janitor.jsonl: machine,
path (relative to the checkouts' directory), size, reason.

Modes: on, dry-run, off (`--janitor`, or the environment's `OCW_JANITOR` –
the kill switch). One machine per pass, at most one pass every 10 minutes;
the last pass per machine is kept in build-log/janitor-state.json, so a
restarted dashboard does not start over.

The decisions are pure functions (cleanup_rules.py, `plan_machine`,
`sweep_units`), unit-tested in test_cleanup_rules.py and test_janitor.py.
"""

from __future__ import annotations

import inspect
import json
import os
import threading
import time
from pathlib import Path

import cleanup_rules as rules

LEVEL_MERGED = 75.0   # % used: a pass at once, with the main checkout's sweep
LEVEL_LRU = 85.0      # % used: also least recently used idle checkouts
LEVEL_ALERT = 92.0    # % used: red banner
IDLE_LRU_S = 24 * 3600
SWEEP_AGE_S = 3 * 86400
PASS_INTERVAL_S = 600.0
PRESSURE_INTERVAL_S = 3600.0   # a full disk: at most one pass an hour per machine
ROUTINE_H = 24.0               # the routine pass: once a day per machine
FREE_MIN_GB = 40.0             # less free space than this counts like > 75 %
MODES = ("off", "dry-run", "on")
STATE_FILE = "janitor-state.json"
KEEP_ACTIONS = 40              # actions kept per machine for the page


def _env_float(name: str, default: float, env=None) -> float:
    try:
        v = float((os.environ if env is None else env).get(name, ""))
        return v if v >= 0 else default
    except (TypeError, ValueError):
        return default


def default_mode(env=None) -> str:
    """The mode without --janitor: OCW_JANITOR (off | dry-run | on), else on."""
    v = str((os.environ if env is None else env).get("OCW_JANITOR", "")).strip().lower()
    return {"0": "off", "false": "off", "dry": "dry-run", "1": "on"}.get(v, v if v in MODES else "on")


# --------------------------------------------------------------------------
# Pure decisions
# --------------------------------------------------------------------------


def disk_size(d: dict):
    """The usable size: used + free (like df's Use%: ext4's reserved blocks
    and, on APFS, the container's other volumes do not count)."""
    used, free = d.get("used"), d.get("free")
    if used is not None and free is not None and used + free > 0:
        return used + free
    return d.get("total") or None


def disk_pct(d: dict):
    size, used = disk_size(d), d.get("used")
    if not size or used is None:
        return None
    return round(100.0 * used / size, 1)


def level_for(pct, free=None, free_min=None) -> int:
    """0, 75, 85 or 92: the highest threshold the disk is above; a disk
    with less than `free_min` bytes free counts as above 75 %."""
    lv = 0
    if pct is not None:
        lv = next((int(x) for x in (LEVEL_ALERT, LEVEL_LRU, LEVEL_MERGED) if pct > x), 0)
    if not lv and free is not None and free_min and free < free_min:
        lv = int(LEVEL_MERGED)
    return lv


fmt_age = rules.fmt_age


def plan_machine(inv: dict, facts: dict, now: float | None = None, lim: dict | None = None, sweep=None,
                 free_min: float | None = None) -> dict:
    """What the janitor does on one machine now: {"pct", "level", "actions":
    [{"kind", "op", "path", "bytes", "reason", "cls", "level"}], "alert",
    "plan"}: everything the cleanup rules allow (level 0); above 75 % the
    main checkout's sweep (`sweep`: its dry run {"bytes", "units"} or None);
    above 85 % the least recently used idle checkouts until below."""
    disk = inv.get("disk") or {}
    pct = disk_pct(disk)
    level = level_for(pct, disk.get("free"), free_min)
    p = rules.plan(inv, facts, lim, now)
    actions = [dict(_action(i), level=0) for i in p["items"] if i["delete"]]
    if level >= LEVEL_MERGED:
        main = next((c for c in inv.get("checkouts", []) if c["cls"] == "main"), None)
        if sweep and sweep.get("units") and main and not main.get("busy"):
            actions.append({"kind": "sweep", "op": None, "cls": "main", "path": main["name"] + "/target",
                            "bytes": sweep.get("bytes") or 0, "units": sweep.get("units"),
                            "reason": "build units unused for 3 days and replaced by newer ones",
                            "level": int(LEVEL_MERGED)})
    size = disk_size(disk)
    lru = rules.lru_candidates(p, IDLE_LRU_S)
    if level >= LEVEL_LRU and size:
        used = (disk.get("used") or 0) - sum(a["bytes"] for a in actions)
        for i in lru:
            if 100.0 * used / size <= LEVEL_LRU:
                break
            actions.append(dict(_action(i), level=int(LEVEL_LRU),
                                reason="least recently used, idle for %s (%s)" % (fmt_age(i["idle"]), i["reason"])))
            used -= i["bytes"]
    return {"pct": pct, "level": level, "actions": actions, "alert": level >= LEVEL_ALERT, "plan": p, "lru": lru}


def _action(i: dict) -> dict:
    return {"kind": i["kind"], "op": i["op"], "cls": i["cls"], "path": i["path"], "bytes": i["bytes"],
            "reason": i["reason"] + ((" – " + i["note"]) if i.get("note") else "")}


def disk_alerts(machines: list) -> list:
    """One alert per machine disk above 92 %."""
    out = []
    for m in machines:
        for d in m.get("disks") or []:
            pct = d.get("pct")
            if pct is not None and pct > LEVEL_ALERT:
                out.append({"machine": m.get("name"), "disk": d.get("name"), "pct": pct,
                            "free": d.get("free"),
                            "text": "%s: disk %s %.0f %% full (%s free)" % (
                                m.get("name"), d.get("name"), pct, _gb(d.get("free")))})
    return out


def _gb(n) -> str:
    return "?" if n is None else "%.1f GB" % (n / 1e9)


# --------------------------------------------------------------------------
# The main checkout's target/: old build units (runs on the machine)
# --------------------------------------------------------------------------


def sweep_units(target: str, now: float, age_s: float, delete: bool) -> dict:
    """cargo-sweep-like cleanup of one target/ directory. A build unit is
    cargo's `<name>-<hash>` (16 hex digits): its `.fingerprint/` and
    `build/` directories and the `deps/` files with that hash. A unit goes
    when every one of its files is older than `age_s` (by the later of
    access and modification time: atime alone is useless on noatime mounts)
    and a newer unit of the same crate and kind (lib, bin, test, build
    script, its run: from the fingerprint's file names) exists in the same
    profile directory, so the crate's current build stays. Incremental session directories
    (`incremental/<crate>-<id>`) go when all their files are older.
    Self-contained (runs on the machine with python3 via ssh); returns
    {"bytes", "units", "incremental", "deleted"}."""
    import os as _os
    import re as _re
    import shutil as _shutil
    hexre = _re.compile(r"-([0-9a-f]{16})(?=\.|$)")
    stats = {"bytes": 0, "units": 0, "incremental": 0, "deleted": delete}

    def walk(p):
        """(newest of atime/mtime, bytes) of a file or directory tree."""
        try:
            st = _os.lstat(p)
        except OSError:
            return 0, 0
        if not _os.path.isdir(p) or _os.path.islink(p):
            return max(st.st_atime, st.st_mtime), st.st_size
        newest, size = 0, 0  # files only: a directory's own mtime says little
        for root, dirs, files in _os.walk(p):
            for f in files:
                try:
                    s = _os.lstat(_os.path.join(root, f))
                except OSError:
                    continue
                newest = max(newest, s.st_atime, s.st_mtime)
                size += s.st_size
        return newest, size

    def remove(p):
        if _os.path.isdir(p) and not _os.path.islink(p):
            _shutil.rmtree(p, ignore_errors=True)
        else:
            try:
                _os.remove(p)
            except OSError:
                pass

    profiles = []
    for root, dirs, files in _os.walk(target):
        if ".fingerprint" in dirs:
            profiles.append(root)
        # profile dirs are at most two levels down (target/debug,
        # target/wasm32-unknown-unknown/debug)
        if root.count(_os.sep) - target.rstrip(_os.sep).count(_os.sep) >= 2:
            dirs[:] = []
        dirs[:] = [d for d in dirs if d not in (".fingerprint", "deps", "build", "incremental", "examples")]
    for prof in profiles:
        units = {}  # hash -> {"name", "paths": [...]}
        fp = _os.path.join(prof, ".fingerprint")
        for d in _os.listdir(fp):
            m = hexre.search(d)
            if m and m.end() == len(d):
                # The unit's kind (lib, bin, test, build script, its run)
                # from the fingerprint file names: same crate and kind =
                # siblings that replace each other.
                try:
                    kind = ",".join(sorted(f for f in _os.listdir(_os.path.join(fp, d))
                                           if not f.startswith(("invoked", "output", "stderr", "root-output"))))
                except OSError:
                    kind = ""
                units.setdefault(m.group(1), {"name": d[:m.start()] + "|" + kind, "paths": []})["paths"].append(
                    _os.path.join(fp, d))
        for sub in ("build", "deps", "examples"):
            sd = _os.path.join(prof, sub)
            if not _os.path.isdir(sd):
                continue
            for f in _os.listdir(sd):
                m = hexre.search(f)
                if m and m.group(1) in units:
                    units[m.group(1)]["paths"].append(_os.path.join(sd, f))
        info = []
        for h, u in units.items():
            newest, size = 0, 0
            for p in u["paths"]:
                n, s = walk(p)
                newest, size = max(newest, n), size + s
            info.append((u["name"], newest, size, u["paths"]))
        latest = {}
        for name, newest, size, paths in info:
            latest[name] = max(latest.get(name, 0), newest)
        for name, newest, size, paths in info:
            if now - newest >= age_s and latest[name] > newest:
                stats["units"] += 1
                stats["bytes"] += size
                if delete:
                    for p in paths:
                        remove(p)
        inc = _os.path.join(prof, "incremental")
        if _os.path.isdir(inc):
            for d in _os.listdir(inc):
                p = _os.path.join(inc, d)
                newest, size = walk(p)
                if now - newest >= age_s:
                    stats["incremental"] += 1
                    stats["bytes"] += size
                    if delete:
                        remove(p)
    return stats


def sweep_script() -> str:
    """A python3 program for the machine: sweep_units() on argv[1]."""
    return ("import json, sys, time\n" + inspect.getsource(sweep_units) +
            "\nprint('@@sweep ' + json.dumps(sweep_units(sys.argv[1], time.time(), float(sys.argv[2]),"
            " sys.argv[3] == 'delete')))\n")


# The Windows version of sweep_units (no python there).
SWEEP_PS = r"""
$ErrorActionPreference = 'SilentlyContinue'
$t = @TARGET@; $age = @AGE@; $del = @DELETE@; $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$st = [ordered]@{ bytes = [int64]0; units = 0; incremental = 0; deleted = $del }
function Newest($p) { $n = 0; $s = [int64]0; $items = @(Get-Item -LiteralPath $p -Force); if ($items[0].PSIsContainer) { $items += @(Get-ChildItem -LiteralPath $p -Recurse -File -Force) }
  foreach ($i in $items) { $a = [Math]::Max(([DateTimeOffset]$i.LastWriteTimeUtc).ToUnixTimeSeconds(), ([DateTimeOffset]$i.LastAccessTimeUtc).ToUnixTimeSeconds()); if (-not $i.PSIsContainer) { if ($a -gt $n) { $n = $a }; $s += $i.Length } }; return @($n, $s) }
foreach ($fp in @(Get-ChildItem -LiteralPath $t -Directory -Recurse -Depth 2 -Force -Filter '.fingerprint')) {
  $prof = $fp.Parent.FullName; $units = @{}
  foreach ($d in Get-ChildItem -LiteralPath $fp.FullName -Directory -Force) { if ($d.Name -match '^(.*)-([0-9a-f]{16})$') { $h = $Matches[2]; $nm = $Matches[1]
    $kind = (@(Get-ChildItem -LiteralPath $d.FullName -Force | Where-Object { $_.Name -notmatch '^(invoked|output|stderr|root-output)' } | ForEach-Object { $_.Name } | Sort-Object) -join ',')
    $units[$h] = @{ name = $nm + '|' + $kind; paths = @($d.FullName) } } }
  foreach ($sub in 'build', 'deps', 'examples') { $sd = Join-Path $prof $sub; if (Test-Path -LiteralPath $sd) { foreach ($f in Get-ChildItem -LiteralPath $sd -Force) { if ($f.Name -match '-([0-9a-f]{16})(\.|$)' -and $units.ContainsKey($Matches[1])) { $units[$Matches[1]].paths += $f.FullName } } } }
  $info = @(); $latest = @{}
  foreach ($u in $units.Values) { $n = 0; $s = [int64]0; foreach ($p in $u.paths) { $r = Newest $p; if ($r[0] -gt $n) { $n = $r[0] }; $s += $r[1] }; $info += ,@($u.name, $n, $s, $u.paths); if (-not $latest.ContainsKey($u.name) -or $latest[$u.name] -lt $n) { $latest[$u.name] = $n } }
  foreach ($i in $info) { if (($now - $i[1]) -ge $age -and $latest[$i[0]] -gt $i[1]) { $st.units++; $st.bytes += $i[2]; if ($del) { foreach ($p in $i[3]) { Remove-Item -LiteralPath $p -Recurse -Force } } } }
  $inc = Join-Path $prof 'incremental'
  if (Test-Path -LiteralPath $inc) { foreach ($d in Get-ChildItem -LiteralPath $inc -Directory -Force) { $r = Newest $d.FullName; if (($now - $r[0]) -ge $age) { $st.incremental++; $st.bytes += $r[1]; if ($del) { Remove-Item -LiteralPath $d.FullName -Recurse -Force } } } }
}
'@@sweep ' + ($st | ConvertTo-Json -Compress)
"""


# --------------------------------------------------------------------------
# Running the snippets of cleanup_rules.py on a machine
# --------------------------------------------------------------------------

_sh_path, _shq, _psq = rules._sh_path, rules._shq, rules._psq


def parse_sweep(text: str):
    for l in text.splitlines():
        if l.startswith("@@sweep "):
            try:
                return json.loads(l[8:])
            except ValueError:
                return None
    return None


def inventory(ssh, m) -> dict:
    """The parsed inventory of machine `m` (ssh(m, script, timeout, stdin))."""
    base = rules.base_name(m.repo_path)
    if m.os == "windows":
        return rules.parse_windows_inventory(ssh(m, rules.windows_inventory(m.repo_path), 2400), base)
    return rules.parse_posix_inventory(ssh(m, rules.posix_inventory(m.repo_path), 1800), base)


def run_ops(ssh, m, ops: list, dry: bool = False) -> dict:
    """Deletes (or with `dry` only measures) the ops on machine `m`:
    op -> {"result", "bytes"}; an op without an answer counts as failed."""
    base = rules.base_name(m.repo_path)
    ops = [op for op in ops if rules.safe_op(op, base)]
    if not ops:
        return {}
    if m.os == "windows":
        res = {}
        for i in range(0, len(ops), 40):  # an encoded command line has a length limit
            res.update(rules.parse_results(ssh(m, rules.windows_delete(m.repo_path, ops[i:i + 40], dry), 3600)))
    else:
        res = rules.parse_results(ssh(m, rules.posix_delete(m.repo_path, dry), 3600, stdin=rules.ops_stdin(ops, base)))
    return {op: res.get(op, {"result": "failed", "bytes": None}) for op in ops}


# --------------------------------------------------------------------------
# The janitor loop
# --------------------------------------------------------------------------


class Janitor:
    """Runs a pass over one machine at a time, at most every PASS_INTERVAL_S:
    first the machine whose checkouts' disk is fullest above 75 % (each at
    most once an hour), then the enabled machine whose routine pass is due
    (once a day)."""

    def __init__(self, machines: list, repo: Path, log_root: Path | None, mode: str = "on",
                 ssh=None, interval: float = PASS_INTERVAL_S, clock=time.time, routine_s: float | None = None,
                 free_min: float | None = None, limits: dict | None = None):
        assert mode in MODES
        self.machines = {m.name: m for m in machines}
        self.repo, self.log_root, self.mode = repo, log_root, mode
        self.ssh = ssh            # ssh(machine, script, timeout, stdin=None) -> stdout text (or raises)
        self.interval, self.clock = interval, clock
        self.routine_s = _env_float("OCW_JANITOR_ROUTINE_H", ROUTINE_H) * 3600 if routine_s is None else routine_s
        self.free_min = _env_float("OCW_JANITOR_FREE_GB", FREE_MIN_GB) * 1e9 if free_min is None else free_min
        self.limits = rules.limits() if limits is None else limits
        self.last_pass = None     # epoch of the last pass
        self.visited: dict = {}   # machine -> epoch of its last pass
        self.results: dict = {}   # machine -> result of its last pass
        self.recent: list = []    # recent log entries (newest last)
        self.lock = threading.Lock()
        self.test_output_bytes = None
        self._last_dry: dict = {}
        self.load_state()

    # ---- the last pass per machine, kept across restarts
    def state_path(self):
        return None if self.log_root is None else self.log_root / STATE_FILE

    def load_state(self):
        p = self.state_path()
        try:
            data = json.loads(p.read_text(encoding="utf-8")) if p and p.is_file() else {}
        except (OSError, ValueError):
            data = {}
        for name, r in (data.get("machines") or {}).items():
            if name in self.machines and isinstance(r, dict) and r.get("at"):
                self.results[name] = r
                self.visited[name] = r["at"]

    def save_state(self):
        p = self.state_path()
        if p is None:
            return
        try:
            p.parent.mkdir(parents=True, exist_ok=True)
            tmp = p.with_suffix(".tmp")
            tmp.write_text(json.dumps({"machines": self.results}, sort_keys=True), encoding="utf-8")
            os.replace(tmp, p)
        except OSError:
            pass

    # ---- logging
    def log(self, entry: dict):
        t = self.clock()
        lt = time.localtime(t)
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S", lt)
        off = time.strftime("%z", lt)
        entry = dict({"time": stamp + (off[:3] + ":" + off[3:] if off else "")}, **entry)
        with self.lock:
            self.recent = (self.recent + [entry])[-30:]
        if self.log_root is None:
            return
        try:
            p = self.log_root / time.strftime("%Y-%m-%d", lt) / "janitor.jsonl"
            p.parent.mkdir(parents=True, exist_ok=True)
            with open(p, "a", encoding="utf-8") as f:
                f.write(json.dumps(entry, sort_keys=True) + "\n")
        except OSError:
            pass

    # ---- choosing a machine
    def due(self) -> bool:
        return self.mode != "off" and (self.last_pass is None or self.clock() - self.last_pass >= self.interval)

    def pressure(self, st: dict) -> bool:
        """The checkouts' disk is above 75 %, or short of free space."""
        free = next((d.get("free") for d in st.get("disks") or [] if "checkouts" in str(d.get("role"))), None)
        return level_for(st.get("disk_pct"), free, self.free_min) > 0

    def routine_due(self, name: str, now: float) -> bool:
        return self.routine_s > 0 and now - (self.visited.get(name) or 0) >= self.routine_s

    def pick(self, states: list):
        """(machine, why) of the next pass or (None, None): the online
        machine whose checkout disk is fullest above 75 % and that had no
        pass for an hour ("disk"); else the enabled online machine whose
        daily pass is due, the longest-waiting first ("routine")."""
        now = self.clock()
        full, routine = [], []
        for st in states:
            name = st.get("name")
            if not st.get("online") or name not in self.machines:
                continue
            seen = self.visited.get(name) or 0
            if self.pressure(st) and now - seen >= min(PRESSURE_INTERVAL_S, self.routine_s or PRESSURE_INTERVAL_S):
                full.append(((st.get("disk_pct") or 0, -seen), name))
            elif st.get("enabled", True) and self.routine_due(name, now):
                routine.append((seen, name))
        if full:
            return max(full)[1], "disk"
        if routine:
            return min(routine)[1], "routine"
        return None, None

    # ---- a pass
    def inventory(self, m) -> dict:
        return inventory(self.ssh, m)

    def sweep(self, m, delete: bool):
        target = m.repo_path.rstrip("\\/") + ("\\target" if m.os == "windows" else "/target")
        if m.os == "windows":
            ps = SWEEP_PS.replace("@TARGET@", _psq(target)).replace("@AGE@", str(int(SWEEP_AGE_S))) \
                .replace("@DELETE@", "$true" if delete else "$false")
            return parse_sweep(self.ssh(m, ps, 1800))
        cmd = "python3 - %s %d %s" % (_sh_path(target), SWEEP_AGE_S, "delete" if delete else "dry")
        return parse_sweep(self.ssh(m, cmd, 1800, stdin=sweep_script()))

    def facts(self) -> dict:
        return rules.git_facts(self.repo)

    def plan(self, m, inv: dict, now: float, with_sweep: bool = True) -> dict:
        sweep = None
        disk = inv.get("disk") or {}
        if with_sweep and level_for(disk_pct(disk), disk.get("free"), self.free_min) >= LEVEL_MERGED:
            main = next((c for c in inv["checkouts"] if c["cls"] == "main"), None)
            if main and not main["busy"]:
                sweep = self.sweep(m, False)
        return plan_machine(inv, self.facts(), now, self.limits, sweep, self.free_min)

    def run_pass(self, name: str, why: str = "disk") -> dict:
        m = self.machines[name]
        now = self.clock()
        self.last_pass = now
        self.visited[name] = now
        res = {"at": now, "mode": self.mode, "why": why, "actions": [], "freed": 0, "count": 0, "bytes": 0}
        try:
            inv = self.inventory(m)
            p = self.plan(m, inv, now)
        except Exception as e:  # unreachable machine, timeout, parse error
            res["error"] = "%s: %s" % (type(e).__name__, e)
            self.results[name] = res
            self.save_state()
            return res
        res.update(pct=p["pct"], level=p["level"])
        if p["alert"]:
            self.log({"machine": name, "action": "alert", "disk_pct": p["pct"], "mode": self.mode,
                      "reason": "disk above %d %%" % LEVEL_ALERT})
        if self.mode == "dry-run":
            sig = [(a["path"], a["kind"]) for a in p["actions"]]
            if sig != self._last_dry.get(name):
                for a in p["actions"]:
                    self.log(dict(_entry(name, a, p["pct"]), action="would-delete", mode="dry-run"))
                self._last_dry[name] = sig
            acts = [dict(a, result="planned") for a in p["actions"]]
        else:
            acts = self.execute(m, p, name)
            res["freed"] = sum(a.get("freed") or 0 for a in acts)
        done = [a for a in acts if a["result"] in ("planned", "deleted")]
        res.update(count=len(done), bytes=sum(a.get("bytes") or 0 for a in done),
                   actions=[{k: a.get(k) for k in ("kind", "path", "bytes", "reason", "result")}
                            for a in acts[:KEEP_ACTIONS]])
        self.results[name] = res
        self.save_state()
        return res

    def execute(self, m, p: dict, name: str) -> list:
        out = []
        ops = [a for a in p["actions"] if a["kind"] != "sweep" and a.get("op")]
        if ops:
            try:
                results = run_ops(self.ssh, m, [a["op"] for a in ops])
            except Exception as e:
                results = {}
                self.log({"machine": name, "action": "error", "reason": "%s: %s" % (type(e).__name__, e)})
            for a in ops:
                r = results.get(a["op"], {"result": "failed"})
                a = dict(a, result=r["result"], freed=(r.get("bytes") or 0) if r["result"] == "deleted" else 0)
                if r["result"] == "deleted":
                    a["bytes"] = r.get("bytes") or a["bytes"]
                if r["result"] != "gone":
                    self.log(dict(_entry(name, a, p["pct"]),
                                  action="delete" if r["result"] == "deleted" else r["result"], mode="on"))
                out.append(a)
        for a in p["actions"]:
            if a["kind"] != "sweep":
                continue
            try:
                s = self.sweep(m, True)
            except Exception as e:
                s = None
                self.log({"machine": name, "action": "error", "reason": "sweep: %s" % e})
            a = dict(a, result="deleted" if s else "failed", freed=(s or {}).get("bytes") or 0,
                     units=(s or {}).get("units"))
            if s:
                a["bytes"] = s.get("bytes") or 0
            self.log(dict(_entry(name, a, p["pct"]), action="delete" if s else "failed", mode="on"))
            out.append(a)
        return out

    def tick(self, states: list):
        """Called by the loop: one pass if due and a machine needs it."""
        if not self.due():
            return None
        name, why = self.pick(states)
        if name is None:
            return None
        return self.run_pass(name, why)

    def status(self, machines: list) -> dict:
        with self.lock:
            recent = list(self.recent)
        nxt = None if self.last_pass is None else self.last_pass + self.interval
        routine = {n: (self.visited.get(n) or 0) + self.routine_s for n in self.machines} if self.routine_s > 0 else {}
        return {"mode": self.mode, "levels": {"merged": LEVEL_MERGED, "lru": LEVEL_LRU, "alert": LEVEL_ALERT},
                "routine_h": self.routine_s / 3600.0, "free_min_gb": self.free_min / 1e9, "next_routine": routine,
                "last_pass": self.last_pass, "next_pass": nxt, "machines": dict(self.results),
                "recent": recent[-12:], "alerts": disk_alerts(machines),
                "test_output_bytes": self.test_output_bytes}


def _entry(machine: str, a: dict, pct) -> dict:
    e = {"machine": machine, "path": a["path"], "bytes": a.get("bytes"), "reason": a["reason"],
         "kind": a["kind"], "level": a.get("level"), "disk_pct": pct}
    if a.get("cls"):
        e["class"] = a["cls"]
    if a.get("units") is not None:
        e["units"] = a["units"]
    return e


def dir_bytes(path: Path) -> int:
    """Size of a local directory tree (test-output/: reported, never deleted)."""
    total = 0
    for root, dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_size
            except OSError:
                pass
    return total
