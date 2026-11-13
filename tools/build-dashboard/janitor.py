"""The janitor of the build machines' disks (spec §11.2).

The dashboard polls every machine's disks; when the disk holding the
checkouts fills up, the janitor frees space in this order:

* above 75 %: the `target/` of agent checkouts (`<repo>-worktree-agent-…`)
  whose branch is merged into main (worktree removed) or gone, unused for
  2 h; and in the main checkout's `target/` the build units (cargo's
  `<crate>-<hash>` artefacts: fingerprint, deps files, build-script output)
  unused for 3 days that a newer unit of the same crate has replaced, plus
  incremental session directories unused for 3 days (like cargo-sweep);
* above 85 %: also the `target/` of the least recently used agent checkouts
  idle for more than 24 h (sources kept), oldest first, until the disk is
  estimated below 85 % again;
* above 92 %: an alert (red banner in the dashboard, a log line).

Never touched: the sources of any checkout, checkouts with a running or
waiting remote.sh task (checked again on the machine right before each
deletion), the sccache cache, data/ and test-output/ – only paths ending in
`/target` of a checkout (or units inside the main checkout's target/) are
ever deleted. Every deletion (and, in dry-run mode, every planned one) is
logged as a JSON line to test-output/build-log/DAY/janitor.jsonl: machine,
path (relative to the checkouts' directory), size, reason.

The decisions are pure functions (`plan_machine`, `branch_verdict`,
`sweep_units`), unit-tested in test_janitor.py; `Janitor` runs them, one
machine per pass, at most one pass every 10 minutes.
"""

from __future__ import annotations

import inspect
import json
import os
import re
import subprocess
import threading
import time
from pathlib import Path

LEVEL_MERGED = 75.0   # % used: delete target/ of merged or gone agent branches
LEVEL_LRU = 85.0      # % used: also least recently used idle agent checkouts
LEVEL_ALERT = 92.0    # % used: red banner
IDLE_MERGED_S = 2 * 3600
IDLE_LRU_S = 24 * 3600
SWEEP_AGE_S = 3 * 86400
PASS_INTERVAL_S = 600.0
MODES = ("off", "dry-run", "on")
AGENT_PREFIX = "worktree-agent-"
_SAFE_NAME = re.compile(r"^[A-Za-z0-9._+-]+$")


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


def level_for(pct) -> int:
    """0, 75, 85 or 92: the highest threshold the disk is above."""
    if pct is None:
        return 0
    for lv in (LEVEL_ALERT, LEVEL_LRU, LEVEL_MERGED):
        if pct > lv:
            return int(lv)
    return 0


def checkout_kind(name: str, base: str):
    """('main' | 'agent' | 'other' | None, branch) of a checkout directory
    name next to the main checkout `base` (e.g. "OCW")."""
    if name == base:
        return "main", "main"
    if not name.startswith(base + "-"):
        return None, None
    branch = name[len(base) + 1:]
    return ("agent" if branch.startswith(AGENT_PREFIX) else "other"), branch


def branch_verdict(branch: str, branches, worktree_branches, merged):
    """Why an agent checkout's target/ may go (like scripts/remote-gc.sh):
    "branch gone", "merged into main, worktree removed", or None (keep: an
    open branch, a live worktree – a new agent's branch looks merged since
    it has no commits yet – or not an agent checkout)."""
    if not branch.startswith(AGENT_PREFIX):
        return None
    if branch not in branches:
        return "branch gone"
    if branch in worktree_branches:
        return None
    if branch in merged:
        return "merged into main, worktree removed"
    return None


def fmt_age(s) -> str:
    s = int(max(0, s or 0))
    if s >= 86400:
        return "%dd %dh" % (s // 86400, s % 86400 // 3600)
    if s >= 3600:
        return "%dh %02dm" % (s // 3600, s % 3600 // 60)
    return "%dm" % (s // 60)


def candidates(checkouts: list, now: float, verdicts: dict) -> dict:
    """The agent checkouts each rule could delete, ignoring the disk level:
    {"merged": [...], "lru": [...] (oldest first), "kept": [...]}; each a
    checkout dict with "reason" added."""
    merged, lru, kept = [], [], []
    for c in checkouts:
        c = dict(c)
        idle = now - (c.get("last_used") or 0)
        if c.get("kind") == "main":
            c["reason"] = "main checkout (only its old build units are swept)"
            kept.append(c)
            continue
        if c.get("kind") != "agent":
            c["reason"] = "not an agent checkout"
            kept.append(c)
            continue
        if c.get("busy"):
            c["reason"] = "in use (running or waiting task)"
            kept.append(c)
            continue
        why = verdicts.get(c.get("branch"))
        if why and idle >= IDLE_MERGED_S:
            c["reason"] = "%s, unused for %s" % (why, fmt_age(idle))
            merged.append(c)
        elif idle >= IDLE_LRU_S:
            c["reason"] = "least recently used, idle for %s" % fmt_age(idle)
            lru.append(c)
        else:
            c["reason"] = ("%s but used %s ago" % (why, fmt_age(idle))) if why else \
                "branch open, used %s ago" % fmt_age(idle)
            kept.append(c)
    lru.sort(key=lambda c: c.get("last_used") or 0)
    return {"merged": merged, "lru": lru, "kept": kept}


def plan_machine(checkouts: list, disk: dict, now: float, verdicts: dict, sweep=None) -> dict:
    """What the janitor does on one machine now: {"pct", "level",
    "actions": [{"kind": "target"|"sweep", "checkout", "path", "bytes",
    "reason", "level"}], "alert"}. `disk` is the checkouts' disk
    ({"total", "used"} in bytes); `sweep` the main checkout's dry-run sweep
    ({"bytes", "units"}) or None."""
    pct = disk_pct(disk)
    level = level_for(pct)
    cand = candidates(checkouts, now, verdicts)
    actions = []
    if level >= LEVEL_MERGED:
        for c in cand["merged"]:
            actions.append({"kind": "target", "checkout": c["name"], "path": c["name"] + "/target",
                            "bytes": c.get("target_bytes") or 0, "reason": c["reason"], "level": int(LEVEL_MERGED)})
        main = next((c for c in checkouts if c.get("kind") == "main"), None)
        if sweep and sweep.get("units") and main and not main.get("busy"):
            actions.append({"kind": "sweep", "checkout": main["name"], "path": main["name"] + "/target",
                            "bytes": sweep.get("bytes") or 0, "units": sweep.get("units"),
                            "reason": "build units unused for 3 days and replaced by newer ones",
                            "level": int(LEVEL_MERGED)})
    size = disk_size(disk)
    if level >= LEVEL_LRU and size:
        used = (disk.get("used") or 0) - sum(a["bytes"] for a in actions)
        for c in cand["lru"]:
            if 100.0 * used / size <= LEVEL_LRU:
                break
            actions.append({"kind": "target", "checkout": c["name"], "path": c["name"] + "/target",
                            "bytes": c.get("target_bytes") or 0, "reason": c["reason"], "level": int(LEVEL_LRU)})
            used -= c.get("target_bytes") or 0
    return {"pct": pct, "level": level, "actions": actions, "alert": level >= LEVEL_ALERT,
            "candidates": cand}


def safe_target(path: str) -> bool:
    """A janitor path is "<checkout>/target" with a plain directory name."""
    parts = path.split("/")
    return len(parts) == 2 and parts[1] == "target" and bool(_SAFE_NAME.match(parts[0])) \
        and parts[0] not in (".", "..")


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
# Remote snippets: inventory and deletion
# --------------------------------------------------------------------------


def _sh_path(p: str) -> str:
    """A POSIX shell word for a path from machines.toml ("~/src/OCW")."""
    if p == "~" or p.startswith("~/"):
        return '"$HOME"' + _shq(p[1:]) if len(p) > 1 else '"$HOME"'
    return _shq(p)


def _shq(s: str) -> str:
    return "'" + s.replace("'", "'\\''") + "'"


def _psq(s: str) -> str:
    return "'" + s.replace("'", "''") + "'"


def posix_inventory(repo_path: str) -> str:
    """Read-only: the disks, every checkout with a target/ (size, last use),
    the checkouts of live task markers."""
    return r"""
export LC_ALL=C
r=@REPO@
mt() { stat -c %Y "$@" 2>/dev/null || stat -f %m "$@" 2>/dev/null; }
echo @@disks; df -Pk "$HOME" "${r%/*}" 2>/dev/null | tail -n +2
echo @@checkouts
for d in "$r" "$r"-*; do
  [ -d "$d/target" ] || continue
  t=$(for f in "$d/.ocw-last-run" "$d/target" "$d"/target/*/deps "$d"/target/*/.fingerprint "$d"/target/*/incremental; do [ -e "$f" ] && mt "$f"; done | sort -n | tail -n 1)
  kb=$(du -sk "$d/target" 2>/dev/null | cut -f1)
  echo "${kb:-0} ${t:-0} ${d##*/}"
done
echo @@busy
for g in /tmp/ocw-tasks/*.json; do [ -e "$g" ] || continue; p=${g##*/}; p=${p%%-*}; ps -p "$p" >/dev/null 2>&1 && sed -n 's/.*"checkout":"\([^"]*\)".*/\1/p' "$g"; done
echo @@end
""".replace("@REPO@", _sh_path(repo_path))


def posix_delete(repo_path: str, names: list) -> str:
    """Deletes <checkouts dir>/NAME/target for each name, unless a live task
    marker names the checkout (checked right before): renamed first, so a
    run starting meanwhile sees no half-deleted target/."""
    for n in names:
        if not _SAFE_NAME.match(n):
            raise ValueError("unsafe checkout name %r" % n)
    return r"""
export LC_ALL=C
r=@REPO@; p=${r%/*}
busy() { for g in /tmp/ocw-tasks/*.json; do [ -e "$g" ] || continue; q=${g##*/}; q=${q%%-*}; grep -q "\"checkout\":\"$1\"" "$g" && ps -p "$q" >/dev/null 2>&1 && return 0; done; return 1; }
for n in @NAMES@; do
  t="$p/$n/target"
  [ -d "$t" ] || { echo "@@gone $n"; continue; }
  if busy "$n"; then echo "@@busy $n"; continue; fi
  kb=$(du -sk "$t" 2>/dev/null | cut -f1)
  g="$p/$n/target.ocw-janitor.$$"
  mv "$t" "$g" && rm -rf "$g" && echo "@@deleted $n ${kb:-0}" || echo "@@failed $n"
done
echo @@end
""".replace("@REPO@", _sh_path(repo_path)).replace("@NAMES@", " ".join(_shq(n) for n in names))


def windows_inventory(repo_path: str) -> str:
    return r"""
$ErrorActionPreference = 'SilentlyContinue'; $ProgressPreference = 'SilentlyContinue'
$r = @REPO@; $parent = Split-Path $r; $leaf = Split-Path -Leaf $r
$o = [ordered]@{}
$want = @($r.Substring(0, 1).ToUpper(), 'C')
$o.disks = @([System.IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq 'Fixed' -and $_.IsReady -and $want -contains $_.Name.Substring(0, 1).ToUpper() } |
  ForEach-Object { [ordered]@{ name = $_.Name.Substring(0, 2); total = [int64]$_.TotalSize; used = [int64]($_.TotalSize - $_.TotalFreeSpace); free = [int64]$_.AvailableFreeSpace } })
$o.checkouts = @(Get-ChildItem -LiteralPath $parent -Directory -Force | Where-Object { $_.Name -eq $leaf -or $_.Name -like ($leaf + '-*') } | ForEach-Object {
  $t = Join-Path $_.FullName 'target'
  if (Test-Path -LiteralPath $t) {
    $ts = @((Join-Path $_.FullName '.ocw-last-run'), $t) + @(Get-ChildItem -LiteralPath $t -Directory -Force | ForEach-Object { Join-Path $_.FullName 'deps'; Join-Path $_.FullName '.fingerprint'; Join-Path $_.FullName 'incremental' })
    $last = 0; foreach ($f in $ts) { $i = Get-Item -LiteralPath $f -Force; if ($i) { $u = ([DateTimeOffset]$i.LastWriteTimeUtc).ToUnixTimeSeconds(); if ($u -gt $last) { $last = $u } } }
    $sz = (Get-ChildItem -LiteralPath $t -Recurse -File -Force | Measure-Object Length -Sum).Sum
    [ordered]@{ name = $_.Name; bytes = [int64]$sz; last_used = $last } } })
$o.busy = @(Get-ChildItem -LiteralPath (Join-Path $env:TEMP 'ocw-tasks') -Filter '*.json' | ForEach-Object {
  if (Get-Process -Id ($_.BaseName -split '-')[0]) { try { ([IO.File]::ReadAllText($_.FullName).TrimStart([char]0xFEFF) | ConvertFrom-Json).checkout } catch {} } })
$o | ConvertTo-Json -Depth 4 -Compress
""".replace("@REPO@", _psq(repo_path))


def windows_delete(repo_path: str, names: list) -> str:
    for n in names:
        if not _SAFE_NAME.match(n):
            raise ValueError("unsafe checkout name %r" % n)
    return r"""
$ErrorActionPreference = 'SilentlyContinue'; $ProgressPreference = 'SilentlyContinue'
$parent = Split-Path @REPO@
$busy = @(Get-ChildItem -LiteralPath (Join-Path $env:TEMP 'ocw-tasks') -Filter '*.json' | ForEach-Object {
  if (Get-Process -Id ($_.BaseName -split '-')[0]) { try { ([IO.File]::ReadAllText($_.FullName).TrimStart([char]0xFEFF) | ConvertFrom-Json).checkout } catch {} } })
foreach ($n in @(@NAMES@)) {
  $t = Join-Path (Join-Path $parent $n) 'target'
  if (-not (Test-Path -LiteralPath $t)) { '@@gone ' + $n; continue }
  if ($busy -contains $n) { '@@busy ' + $n; continue }
  $kb = [int64](((Get-ChildItem -LiteralPath $t -Recurse -File -Force | Measure-Object Length -Sum).Sum) / 1024)
  $g = $t + '.ocw-janitor.' + $PID
  Rename-Item -LiteralPath $t -NewName (Split-Path -Leaf $g)
  if (Test-Path -LiteralPath $g) { Remove-Item -LiteralPath $g -Recurse -Force; if (Test-Path -LiteralPath $g) { '@@failed ' + $n } else { '@@deleted ' + $n + ' ' + $kb } } else { '@@failed ' + $n }
}
'@@end'
""".replace("@REPO@", _psq(repo_path)).replace("@NAMES@", ", ".join(_psq(n) for n in names))


def parse_posix_inventory(text: str, base: str) -> dict:
    sec, cur = {}, None
    for line in text.splitlines():
        if re.fullmatch(r"@@\w+", line.strip()):
            cur = line.strip()[2:]
            sec[cur] = []
        elif cur:
            sec[cur].append(line)
    disks, seen = [], set()
    for i, l in enumerate(sec.get("disks", [])):
        f = l.split()
        if len(f) >= 6 and f[1].isdigit() and f[0] not in seen:
            seen.add(f[0])
            disks.append({"name": "home" if i == 0 else "src", "total": int(f[1]) * 1024,
                          "used": int(f[2]) * 1024, "free": int(f[3]) * 1024})
    busy = {l.strip() for l in sec.get("busy", []) if l.strip()}
    checkouts = []
    for l in sec.get("checkouts", []):
        f = l.split(None, 2)
        if len(f) == 3 and f[0].isdigit() and f[1].isdigit():
            kind, branch = checkout_kind(f[2], base)
            if kind:
                checkouts.append({"name": f[2], "kind": kind, "branch": branch, "target_bytes": int(f[0]) * 1024,
                                  "last_used": int(f[1]), "busy": f[2] in busy})
    # The checkouts live on the disk of the repo's directory (the last df line).
    return {"disks": disks, "disk": disks[-1] if disks else {}, "checkouts": checkouts}


def parse_windows_inventory(text: str, base: str) -> dict:
    start = text.find("{")
    data = json.loads(text[start:]) if start >= 0 else {}
    lst = lambda v: v if isinstance(v, list) else ([v] if v else [])  # noqa: E731
    disks = [{"name": d.get("name"), "total": d.get("total"), "used": d.get("used"), "free": d.get("free")}
             for d in lst(data.get("disks"))]
    busy = {str(b) for b in lst(data.get("busy"))}
    checkouts = []
    for c in lst(data.get("checkouts")):
        kind, branch = checkout_kind(str(c.get("name")), base)
        if kind:
            checkouts.append({"name": c["name"], "kind": kind, "branch": branch,
                              "target_bytes": int(c.get("bytes") or 0), "last_used": int(c.get("last_used") or 0),
                              "busy": c["name"] in busy})
    drive = None
    return {"disks": disks, "disk": next((d for d in disks if d["name"] != "C:"), disks[0] if disks else {}),
            "checkouts": checkouts, "drive": drive}


def parse_results(text: str) -> list:
    """@@deleted NAME KB / @@busy NAME / @@gone NAME / @@failed NAME lines."""
    out = []
    for l in text.splitlines():
        m = re.match(r"@@(deleted|busy|gone|failed)\s+(\S+)(?:\s+(\d+))?", l.strip())
        if m:
            out.append({"result": m.group(1), "checkout": m.group(2),
                        "bytes": int(m.group(3)) * 1024 if m.group(3) else None})
    return out


def parse_sweep(text: str):
    for l in text.splitlines():
        if l.startswith("@@sweep "):
            try:
                return json.loads(l[8:])
            except ValueError:
                return None
    return None


# --------------------------------------------------------------------------
# Git: which agent branches are merged or gone (on the coordinator)
# --------------------------------------------------------------------------


def git_verdicts(repo: Path, names: list) -> dict:
    """branch -> reason for the agent branches among `names` (like
    scripts/remote-gc.sh's verdict)."""
    def git(*a):
        return subprocess.run(["git", "-C", str(repo)] + list(a), capture_output=True, text=True,
                              timeout=30).stdout
    branches = set(git("for-each-ref", "--format=%(refname:short)", "refs/heads").split())
    wt = {l[len("branch refs/heads/"):] for l in git("worktree", "list", "--porcelain").splitlines()
          if l.startswith("branch refs/heads/")}
    merged = set(git("for-each-ref", "--merged", "main", "--format=%(refname:short)", "refs/heads").split())
    return {b: branch_verdict(b, branches, wt, merged) for b in names}


# --------------------------------------------------------------------------
# The janitor loop
# --------------------------------------------------------------------------


class Janitor:
    """Runs a pass over one machine at a time, at most every PASS_INTERVAL_S,
    on the machine whose checkouts' disk is fullest above 75 %."""

    def __init__(self, machines: list, repo: Path, log_root: Path | None, mode: str = "on",
                 ssh=None, interval: float = PASS_INTERVAL_S, clock=time.time):
        assert mode in MODES
        self.machines = {m.name: m for m in machines}
        self.repo, self.log_root, self.mode = repo, log_root, mode
        self.ssh = ssh            # ssh(machine, script, timeout) -> stdout text (or raises)
        self.interval, self.clock = interval, clock
        self.last_pass = None     # epoch of the last pass
        self.visited: dict = {}   # machine -> epoch of its last pass
        self.results: dict = {}   # machine -> result of its last pass
        self.recent: list = []    # recent log entries (newest last)
        self.lock = threading.Lock()
        self.test_output_bytes = None
        self._last_dry: dict = {}

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

    def pick(self, states: list):
        """The online machine whose checkout disk is fullest above 75 %
        (ties: the one visited longest ago)."""
        best = None
        for st in states:
            if not st.get("online") or st.get("name") not in self.machines:
                continue
            pct = st.get("disk_pct")
            if pct is None or pct <= LEVEL_MERGED:
                continue
            key = (pct, -(self.visited.get(st["name"]) or 0))
            if best is None or key > best[0]:
                best = (key, st["name"])
        return best[1] if best else None

    # ---- a pass
    def inventory(self, m) -> dict:
        base = re.split(r"[\\/]", m.repo_path.rstrip("\\/"))[-1]
        if m.os == "windows":
            return parse_windows_inventory(self.ssh(m, windows_inventory(m.repo_path), 1800), base)
        return parse_posix_inventory(self.ssh(m, posix_inventory(m.repo_path), 900), base)

    def sweep(self, m, delete: bool):
        target = m.repo_path.rstrip("\\/") + ("\\target" if m.os == "windows" else "/target")
        if m.os == "windows":
            ps = SWEEP_PS.replace("@TARGET@", _psq(target)).replace("@AGE@", str(int(SWEEP_AGE_S))) \
                .replace("@DELETE@", "$true" if delete else "$false")
            return parse_sweep(self.ssh(m, ps, 1800))
        cmd = "python3 - %s %d %s" % (_sh_path(target), SWEEP_AGE_S, "delete" if delete else "dry")
        return parse_sweep(self.ssh(m, cmd, 1800, stdin=sweep_script()))

    def plan(self, m, inv: dict, now: float, with_sweep: bool = True) -> dict:
        verdicts = git_verdicts(self.repo, [c["branch"] for c in inv["checkouts"] if c["kind"] == "agent"])
        sweep = None
        if with_sweep and level_for(disk_pct(inv["disk"])) >= LEVEL_MERGED:
            main = next((c for c in inv["checkouts"] if c["kind"] == "main"), None)
            if main and not main["busy"]:
                sweep = self.sweep(m, False)
        return plan_machine(inv["checkouts"], inv["disk"], now, verdicts, sweep)

    def run_pass(self, name: str) -> dict:
        m = self.machines[name]
        now = self.clock()
        self.last_pass = now
        self.visited[name] = now
        res = {"at": now, "mode": self.mode, "actions": [], "freed": 0}
        try:
            inv = self.inventory(m)
            p = self.plan(m, inv, now)
        except Exception as e:  # unreachable machine, timeout, parse error
            res["error"] = "%s: %s" % (type(e).__name__, e)
            self.results[name] = res
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
            res["actions"] = [dict(a, result="planned") for a in p["actions"]]
        else:
            res["actions"] = self.execute(m, p, name)
            res["freed"] = sum(a.get("freed") or 0 for a in res["actions"])
        self.results[name] = res
        return res

    def execute(self, m, p: dict, name: str) -> list:
        out = []
        targets = [a for a in p["actions"] if a["kind"] == "target" and safe_target(a["path"])]
        if targets:
            names = [a["checkout"] for a in targets]
            script = windows_delete(m.repo_path, names) if m.os == "windows" else posix_delete(m.repo_path, names)
            try:
                results = {r["checkout"]: r for r in parse_results(self.ssh(m, script, 3600))}
            except Exception as e:
                results = {}
                self.log({"machine": name, "action": "error", "reason": "%s: %s" % (type(e).__name__, e)})
            for a in targets:
                r = results.get(a["checkout"], {"result": "failed"})
                a = dict(a, result=r["result"], freed=r.get("bytes") if r["result"] == "deleted" else 0)
                if r["result"] == "deleted":
                    a["bytes"] = r.get("bytes") or a["bytes"]
                self.log(dict(_entry(name, a, p["pct"]), action="delete" if r["result"] == "deleted" else r["result"],
                              mode="on"))
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
        name = self.pick(states)
        if name is None:
            return None
        return self.run_pass(name)

    def status(self, machines: list) -> dict:
        with self.lock:
            recent = list(self.recent)
        nxt = None if self.last_pass is None else self.last_pass + self.interval
        return {"mode": self.mode, "levels": {"merged": LEVEL_MERGED, "lru": LEVEL_LRU, "alert": LEVEL_ALERT},
                "last_pass": self.last_pass, "next_pass": nxt, "machines": dict(self.results),
                "recent": recent[-12:], "alerts": disk_alerts(machines),
                "test_output_bytes": self.test_output_bytes}


def _entry(machine: str, a: dict, pct) -> dict:
    e = {"machine": machine, "path": a["path"], "bytes": a.get("bytes"), "reason": a["reason"],
         "kind": a["kind"], "level": a.get("level"), "disk_pct": pct}
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
