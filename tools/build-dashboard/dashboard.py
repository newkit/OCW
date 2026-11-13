#!/usr/bin/env python3
"""Build server dashboard for Open Cycling World.

Polls the machines in config/machines.toml over SSH (read-only: one SSH call
per machine every few seconds) and shows their load, the build/test tasks
running on them and the queue of runs waiting for a build slot.

    python3 tools/build-dashboard/dashboard.py [--host 0.0.0.0] [--port 8765]
    python3 tools/build-dashboard/dashboard.py --once          # text, once

Standard library only. The server listens on all addresses by default
(--host 127.0.0.1 for local only) and only reads; it has no write actions.
Host names, addresses and user names never leave this
process: machines are identified by their config `name`, and every string
taken from a machine or from ssh is masked before it is stored.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
POLL_INTERVAL_S = 5.0
# Build slots per machine, as in scripts/remote.sh: the machine's `slots`
# (or [defaults] slots) in machines.toml, else these per-OS defaults.
OS_DEFAULT_SLOTS = {"linux": 2, "windows": 1, "macos": 1}
DEFAULT_SLOTS = 2  # other OSes (remote.sh treats them like Linux)


def default_slots(os_name: str) -> int:
    return OS_DEFAULT_SLOTS.get(str(os_name).lower(), DEFAULT_SLOTS)


def slots_setting(value, os_name: str) -> int:
    """Build slot count from a config value (int or numeric string >= 0),
    else the OS default."""
    try:
        n = int(value)
    except (TypeError, ValueError):
        return default_slots(os_name)
    return n if n >= 0 else default_slots(os_name)

# --------------------------------------------------------------------------
# config/machines.toml
# --------------------------------------------------------------------------

_KEY_RE = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*=\s*(.*)$")


def _strip_comment(value: str) -> str:
    """Removes a trailing `# comment` that is not inside a string."""
    out, in_str, esc = [], None, False
    for ch in value:
        if in_str:
            out.append(ch)
            if esc:
                esc = False
            elif ch == "\\" and in_str == '"':
                esc = True
            elif ch == in_str:
                in_str = None
        elif ch in "\"'":
            in_str = ch
            out.append(ch)
        elif ch == "#":
            break
        else:
            out.append(ch)
    return "".join(out).strip()


def _unescape_basic(s: str) -> str:
    table = {"\\": "\\", '"': '"', "n": "\n", "t": "\t", "r": "\r"}
    return re.sub(r"\\(.)", lambda m: table.get(m.group(1), m.group(0)), s)


def _parse_value(raw: str):
    raw = _strip_comment(raw)
    if raw.startswith('"') and raw.endswith('"') and len(raw) >= 2:
        return _unescape_basic(raw[1:-1])
    if raw.startswith("'") and raw.endswith("'") and len(raw) >= 2:
        return raw[1:-1]
    if raw.startswith("["):
        # Tolerant: take every string in the array, whatever separates them.
        return [_unescape_basic(a or "") if a is not None else b
                for a, b in re.findall(r'"((?:[^"\\]|\\.)*)"|\'([^\']*)\'', raw)]
    if re.fullmatch(r"[+-]?\d+", raw):
        return int(raw)
    if raw in ("true", "false"):
        return raw == "true"
    return None  # unknown or broken value (e.g. a bare `?`): ignored


def parse_machines_tolerant(text: str):
    """Line parser for the TOML subset machines.toml uses ([defaults],
    [[machine]], key = value). Broken lines are skipped instead of failing."""
    defaults: dict = {}
    machines: list = []
    cur = None
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("[[") and "machine" in s:
            cur = {}
            machines.append(cur)
            continue
        if s.startswith("["):
            cur = defaults if s.strip("[] \t") == "defaults" else None
            continue
        m = _KEY_RE.match(line)
        if m and cur is not None:
            v = _parse_value(m.group(2))
            if v is not None:
                cur[m.group(1)] = v
    return defaults, machines


def parse_machines(text: str):
    """Returns (defaults, machines). Uses tomllib (Python 3.11+) when the file
    is valid TOML, otherwise the tolerant line parser."""
    try:
        import tomllib  # type: ignore

        data = tomllib.loads(text)
        return dict(data.get("defaults", {})), [dict(m) for m in data.get("machine", [])]
    except Exception:  # ImportError or a syntax error in the file
        return parse_machines_tolerant(text)


def find_config_dir(repo: Path) -> Path:
    """config/ of this checkout, or of the main checkout for a git worktree
    (config/ is git-ignored), like scripts/remote.sh."""
    cfg = repo / "config"
    if (cfg / "machines.toml").is_file():
        return cfg
    try:
        common = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True, text=True, timeout=10).stdout.strip()
        if common:
            alt = Path(common).parent / "config"
            if (alt / "machines.toml").is_file():
                return alt
    except Exception:
        pass
    return cfg


class Machine:
    def __init__(self, cfg: dict, defaults: dict, config_dir: Path):
        g = lambda k, d=None: cfg.get(k, defaults.get(k, d))  # noqa: E731
        self.name = str(cfg.get("name", "?"))
        self.os = str(g("os", "linux")).lower()
        self.host = g("host")
        self.port = int(g("port", 22) or 22)
        self.user = g("user")
        ssh_dir = config_dir / str(g("ssh_dir", "ssh"))
        key = g("identity_file")
        self.identity = (key if key and os.path.isabs(key) else str(ssh_dir / key)) if key else None
        self.known_hosts = str(ssh_dir / "known_hosts")
        self.timeout = int(g("connect_timeout_s", 10) or 10)
        self.repo_path = str(g("repo_path", "~/src/OCW"))
        self.roles = g("roles", []) or []
        self.slots = slots_setting(g("slots"), self.os)

    def secrets(self) -> list:
        return [s for s in (self.host, self.user) if isinstance(s, str) and s]


def load_machines(config_dir: Path) -> list:
    text = (config_dir / "machines.toml").read_text(encoding="utf-8")
    defaults, machines = parse_machines(text)
    return [Machine(m, defaults, config_dir) for m in machines if m.get("name")]


# --------------------------------------------------------------------------
# Masking
# --------------------------------------------------------------------------

_IPV4 = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b")
# Full (8 groups) or `::`-compressed IPv6; never times like 21:32:41.
_IPV6 = re.compile(r"(?<![\w:])(?:(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}"
                   r"|(?:[0-9A-Fa-f]{1,4}:){1,6}:(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?"
                   r"|::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?)(?:%\w+)?(?![\w:])")
_MAC = re.compile(r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b")
_USERHOST = re.compile(r"\b[\w.+-]+@[\w.-]+\b")
_PORT = re.compile(r"\bport \d+\b")
_HOMES = [
    (re.compile(r"/home/[^/\s'\"]+"), "/home/*"),
    (re.compile(r"/Users/[^/\s'\"]+"), "/Users/*"),
    (re.compile(r"(?i)\b([A-Z]:\\Users\\)[^\\\s'\"]+"), r"\1*"),
]


class Masker:
    """Removes host names, addresses, user names and MAC addresses."""

    def __init__(self, secrets=(), homes=()):
        self.secrets = sorted({s for s in secrets if s}, key=len, reverse=True)
        self.homes = sorted({h.rstrip("/\\") for h in homes if h and len(h) > 1}, key=len, reverse=True)

    def __call__(self, text):
        if not isinstance(text, str) or not text:
            return text
        for h in self.homes:
            text = text.replace(h, "~")
        for s in self.secrets:
            text = re.sub(r"(?<![\w.-])" + re.escape(s) + r"(?![\w-])", "<masked>", text)
        text = _MAC.sub("<mac>", text)
        text = _USERHOST.sub("<user>@<host>", text)
        text = _IPV4.sub("<address>", text)
        text = _IPV6.sub("<address>", text)
        text = _PORT.sub("port <n>", text)
        for rx, rep in _HOMES:
            text = rx.sub(rep, text)
        return text

    def deep(self, obj):
        if isinstance(obj, str):
            return self(obj)
        if isinstance(obj, list):
            return [self.deep(x) for x in obj]
        if isinstance(obj, dict):
            return {k: self.deep(v) for k, v in obj.items()}
        return obj


_SSH_ERRORS = [
    ("connection refused", "connection refused"),
    ("connection timed out", "timeout"),
    ("operation timed out", "timeout"),
    ("no route to host", "no route to host"),
    ("network is unreachable", "network unreachable"),
    ("could not resolve hostname", "unknown host"),
    ("name or service not known", "unknown host"),
    ("host key verification failed", "host key mismatch"),
    ("remote host identification has changed", "host key mismatch"),
    ("permission denied", "permission denied"),
    ("connection closed", "connection closed"),
    ("connection reset", "connection reset"),
    ("no such file or directory", "missing key file"),
]


def classify_ssh_error(stderr: str, returncode, timed_out: bool = False) -> str:
    low = (stderr or "").lower()
    for needle, kind in _SSH_ERRORS:
        if needle in low:
            return kind
    if timed_out:
        return "timeout"
    if returncode == 255:
        return "ssh error"
    return "remote command failed (exit %s)" % returncode


# --------------------------------------------------------------------------
# Remote snippets (read-only)
# --------------------------------------------------------------------------

LINUX_SCRIPT = r"""
export LC_ALL=C
echo @@self; echo $$
echo @@home; echo "$HOME"; id -u
echo @@nproc; nproc 2>/dev/null || getconf _NPROCESSORS_ONLN
echo @@loadavg; cat /proc/loadavg
echo @@uptime; cat /proc/uptime
echo @@meminfo; grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo
echo @@df; df -Pk "$HOME" | tail -n 1
echo @@now; date +%s
echo @@clk; getconf CLK_TCK
ticks() { cat /proc/uptime; cat /proc/[0-9]*/stat 2>/dev/null | awk '{ n = split(substr($0, index($0, ") ") + 2), a, " "); print $1, a[12] + a[13] }'; }
echo @@stat; head -n 1 /proc/stat
echo @@ticks1; ticks
sleep 0.5
echo @@ticks2; ticks
echo @@stat2; head -n 1 /proc/stat
echo @@ps; ps -eo pid=,ppid=,uid=,etimes=,rss=,args= | cut -c1-2000
echo @@cwd; find /proc/[0-9]*/cwd -maxdepth 0 -printf '%h %l\n' 2>/dev/null
echo @@tasks; find /tmp/ocw-tasks/ -name '*.json' -exec cat {} \; -exec echo \; 2>/dev/null
echo @@slots
for f in /tmp/ocw-build-slot-*.lock; do
  [ -e "$f" ] || continue
  printf '%s %s :' "${f##*/}" "$(stat -c %i "$f")"
  command -v fuser >/dev/null && fuser "$f" 2>/dev/null
  echo
done
echo @@locks; cat /proc/locks 2>/dev/null
echo @@kiosk; head -n 3 /var/tmp/ocw/VERSION 2>/dev/null
echo @@bt; command -v bluetoothctl >/dev/null && timeout 3 bluetoothctl devices Connected 2>/dev/null
echo @@end
"""

MACOS_SCRIPT = r"""
export LC_ALL=C
echo @@self; echo $$
echo @@home; echo "$HOME"; id -u
echo @@sysctl; sysctl -n hw.ncpu hw.memsize vm.loadavg kern.boottime vm.swapusage
echo @@now; date +%s
echo @@vmstat; vm_stat
echo @@df; df -Pk "$HOME" | tail -n 1
echo @@cpu; top -l 2 -n 0 -s 1 | grep 'CPU usage' | tail -n 1
echo @@ps; ps -axo pid=,ppid=,uid=,etime=,rss=,args= | cut -c1-2000
echo @@pcpu; ps -axo pid=,%cpu=
echo @@tasks; find /tmp/ocw-tasks/ -name '*.json' -exec cat {} \; -exec echo \; 2>/dev/null
echo @@slots
for d in $(find /tmp/ -maxdepth 1 -name 'ocw-build-slot-*.d' 2>/dev/null); do echo "${d##*/} $(cat $d/pid 2>/dev/null)"; done
echo @@cwd
pids=$(ps -axo pid=,comm= | awk '/cargo|rustc|clippy|ld|cc|clang|ocw|OCW|build-script|deps|zsh|bash|sh$/ {print $1}' | paste -sd, -)
[ -n "$pids" ] && lsof -b -w -a -d cwd -Fn -p "$pids" 2>/dev/null
echo @@end
"""

WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
# CIM/WMI, the Perflib registry key and performance counters are usually
# denied to a non-admin SSH user (and a denied CIM query takes ~5 s): use .NET,
# Get-Process and kernel32 GetSystemTimes (CPU busy %, no rights needed).
# The processor queue length comes from PDH with the English counter name
# (Get-Counter wants localized names); it needs "Performance Monitor Users".
$r = [ordered]@{}
$r.ncpu = [Environment]::ProcessorCount
$r.uptime = [System.Diagnostics.Stopwatch]::GetTimestamp() / [System.Diagnostics.Stopwatch]::Frequency
$r.home = $env:USERPROFILE
$r.self = $PID
$r.now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
try {
  Add-Type -AssemblyName Microsoft.VisualBasic
  $ci = New-Object Microsoft.VisualBasic.Devices.ComputerInfo
  $r.mem_total = [int64]$ci.TotalPhysicalMemory
  $r.mem_avail = [int64]$ci.AvailablePhysicalMemory
} catch {}
$r.perfmon_member = [bool]([Security.Principal.WindowsIdentity]::GetCurrent().Groups | Where-Object { $_.Value -eq 'S-1-5-32-558' })
# P/Invoke through Reflection.Emit: in memory, no compiler run, no files.
$N = $null
try {
  $tb = [AppDomain]::CurrentDomain.DefineDynamicAssembly((New-Object Reflection.AssemblyName 'OcwNative'), 'Run').DefineDynamicModule('OcwNative').DefineType('OcwNative', 'Public,Abstract,Sealed')
  $L = [Int64].MakeByRefType(); $P = [IntPtr].MakeByRefType()
  foreach ($d in @(@('kernel32.dll', 'GetSystemTimes', [bool], @($L, $L, $L)),
                   @('pdh.dll', 'PdhOpenQueryW', [uint32], @([IntPtr], [IntPtr], $P)),
                   @('pdh.dll', 'PdhAddEnglishCounterW', [uint32], @([IntPtr], [string], [IntPtr], $P)),
                   @('pdh.dll', 'PdhCollectQueryData', [uint32], @([IntPtr])),
                   @('pdh.dll', 'PdhGetFormattedCounterValue', [uint32], @([IntPtr], [uint32], [IntPtr], [byte[]])))) {
    $m = $tb.DefinePInvokeMethod($d[1], $d[0], 'Public,Static,PinvokeImpl', 'Standard', $d[2], [Type[]]$d[3], 'Winapi', 'Unicode')
    $m.SetImplementationFlags('PreserveSig')
  }
  $N = $tb.CreateType()
} catch {}
$i1 = 0L; $k1 = 0L; $u1 = 0L; $times = $N -and $N::GetSystemTimes([ref]$i1, [ref]$k1, [ref]$u1)
$pq = [IntPtr]::Zero; $pc = [IntPtr]::Zero
if ($N) {
  $st = $N::PdhOpenQueryW([IntPtr]::Zero, [IntPtr]::Zero, [ref]$pq)
  if ($st -eq 0) { $st = $N::PdhAddEnglishCounterW($pq, '\System\Processor Queue Length', [IntPtr]::Zero, [ref]$pc) }
  if ($st -eq 0) { $st = $N::PdhCollectQueryData($pq) }
  $r.pdh_status = $st
}
$rx = '^(cargo|rustc|clippy-driver|link|lld-link|rust-lld|cl|build-script-build.*|ocw-.*|world-builder|ffmpeg|make)$'
$sel = @(Get-Process | Where-Object { $_.ProcessName -match $rx -or $_.Path -like '*\target\*' })
$cpu0 = @{}
foreach ($p in $sel) { if ($null -ne $p.TotalProcessorTime) { $cpu0[$p.Id] = $p.TotalProcessorTime.TotalMilliseconds } }
$w = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Milliseconds 500
$i2 = 0L; $k2 = 0L; $u2 = 0L
if ($times -and $N::GetSystemTimes([ref]$i2, [ref]$k2, [ref]$u2)) {
  $tot = ($k2 - $k1) + ($u2 - $u1)  # kernel time includes idle time
  if ($tot -gt 0) { $r.cpu_busy = 100.0 * ($tot - ($i2 - $i1)) / $tot }
}
if ($r.pdh_status -eq 0) {
  $st = $N::PdhCollectQueryData($pq); $buf = New-Object byte[] 16
  if ($st -eq 0) { $st = $N::PdhGetFormattedCounterValue($pc, 0x200, [IntPtr]::Zero, $buf) }  # PDH_FMT_DOUBLE
  if ($st -eq 0) { $r.proc_queue = [BitConverter]::ToDouble($buf, 8) }
  $r.pdh_status = $st
}
$ms = $w.Elapsed.TotalMilliseconds
# Load-like value without counters: threads running or ready to run (the
# idle process and this poll's own thread excluded).
$run = 0
foreach ($p in [Diagnostics.Process]::GetProcesses()) {
  if ($p.Id -eq 0) { continue }
  foreach ($t in $p.Threads) { $ts = $t.ThreadState; if ($ts -eq 'Running' -or $ts -eq 'Ready') { $run++ } }
}
$r.runnable = [Math]::Max(0, $run - 1)
# Get-PSDrive can take ~40 s (it waits for optical drives) and reports 0 here.
$r.disks = @([System.IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq 'Fixed' -and $_.IsReady } |
  ForEach-Object { [ordered]@{ name = $_.Name.Substring(0, 1); used = [int64]($_.TotalSize - $_.TotalFreeSpace); free = [int64]$_.AvailableFreeSpace } })
$now = Get-Date
$r.procs = @($sel | ForEach-Object { [ordered]@{ pid = $_.Id; ppid = $null; name = $_.ProcessName;
    args = if ($_.Path) { $_.Path } else { $_.ProcessName + '.exe' };
    elapsed = if ($_.StartTime) { ($now - $_.StartTime).TotalSeconds } else { $null };
    cpu = if ($cpu0.ContainsKey($_.Id) -and $ms -gt 0) { 100.0 * ($_.TotalProcessorTime.TotalMilliseconds - $cpu0[$_.Id]) / $ms } else { $null };
    rss = [int64]$_.WorkingSet64 } })
# Task markers of scripts/remote.sh runs; liveness checked here (read-only).
$r.markers = @(Get-ChildItem -LiteralPath (Join-Path $env:TEMP 'ocw-tasks') -Filter '*.json' | ForEach-Object {
  $mp = Get-Process -Id ($_.BaseName -split '-')[0]
  [ordered]@{ text = [IO.File]::ReadAllText($_.FullName); alive = [bool]$mp;
    pstart = if ($mp -and $mp.StartTime) { ([DateTimeOffset]$mp.StartTime).ToUnixTimeSeconds() } else { $null } } })
$r | ConvertTo-Json -Depth 4 -Compress
"""


def windows_command(script: str = WINDOWS_SCRIPT) -> str:
    enc = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
    return "powershell -NoProfile -NonInteractive -EncodedCommand " + enc


# --------------------------------------------------------------------------
# Parsers (pure, unit-tested)
# --------------------------------------------------------------------------


def parse_sections(text: str) -> dict:
    sections: dict = {}
    cur = None
    for line in text.splitlines():
        if line.startswith("@@") and re.fullmatch(r"@@\w+", line.strip()):
            cur = line.strip()[2:]
            sections[cur] = []
        elif cur is not None:
            sections[cur].append(line)
    return sections


def parse_cpu_busy(stat_lines: list):
    """Busy % between two `cpu ...` lines of /proc/stat."""
    rows = [list(map(int, l.split()[1:])) for l in stat_lines if l.startswith("cpu ")]
    if len(rows) < 2:
        return None
    a, b = rows[0], rows[1]
    idle = lambda r: r[3] + (r[4] if len(r) > 4 else 0)  # noqa: E731 idle + iowait
    total = sum(b) - sum(a)
    if total <= 0:
        return None
    return round(100.0 * (total - (idle(b) - idle(a))) / total, 1)


def parse_meminfo(lines: list) -> dict:
    kv = {}
    for l in lines:
        m = re.match(r"(\w+):\s+(\d+)", l)
        if m:
            kv[m.group(1)] = int(m.group(2)) * 1024
    total = kv.get("MemTotal")
    avail = kv.get("MemAvailable")
    out = {}
    if total:
        out["mem_total"] = total
        out["mem_used"] = total - (avail or 0)
    if kv.get("SwapTotal"):
        out["swap_total"] = kv["SwapTotal"]
        out["swap_used"] = kv["SwapTotal"] - kv.get("SwapFree", 0)
    return out


def parse_df(lines: list) -> dict:
    for l in lines:
        f = l.split()
        if len(f) >= 6 and f[1].isdigit():
            return {"disk_total": int(f[1]) * 1024, "disk_used": int(f[2]) * 1024,
                    "disk_free": int(f[3]) * 1024}
    return {}


def parse_etime(s: str):
    """ps etime `[[dd-]hh:]mm:ss` to seconds."""
    m = re.fullmatch(r"(?:(\d+)-)?(?:(\d+):)?(\d+):(\d+)", s.strip())
    if not m:
        return None
    d, h, mi, se = (int(x) if x else 0 for x in m.groups())
    return ((d * 24 + h) * 60 + mi) * 60 + se


def parse_ps(lines: list, etime: bool = False) -> list:
    procs = []
    for l in lines:
        f = l.split(None, 5)
        if len(f) < 6 or not f[0].isdigit():
            continue
        el = parse_etime(f[3]) if etime else (int(f[3]) if f[3].isdigit() else None)
        procs.append({"pid": int(f[0]), "ppid": int(f[1]), "uid": int(f[2]) if f[2].isdigit() else None,
                      "elapsed": el, "rss": int(f[4]) * 1024 if f[4].isdigit() else None,
                      "args": f[5].strip(), "cwd": None})
    return procs


def parse_cwd_linux(lines: list) -> dict:
    out = {}
    for l in lines:
        m = re.match(r"/proc/(\d+)\s+(.+)$", l)
        if m:
            out[int(m.group(1))] = m.group(2)
    return out


def parse_cwd_lsof(lines: list) -> dict:
    out, pid = {}, None
    for l in lines:
        if l.startswith("p") and l[1:].isdigit():
            pid = int(l[1:])
        elif l.startswith("n") and pid is not None:
            out[pid] = l[1:]
    return out


def parse_proc_locks(lines: list) -> dict:
    """inode -> {"held": bool, "waiting": n} for FLOCK entries."""
    out: dict = {}
    for l in lines:
        m = re.match(r"\s*\d+:\s+(->\s+)?FLOCK\s+\S+\s+\S+\s+(\d+)\s+[0-9a-f]+:[0-9a-f]+:(\d+)", l)
        if not m:
            continue
        e = out.setdefault(int(m.group(3)), {"held": False, "waiting": 0})
        if m.group(1):
            e["waiting"] += 1
        else:
            e["held"] = True
    return out


def parse_slot_lines(lines: list) -> list:
    out = []
    for l in lines:
        m = re.match(r"(ocw-build-slot-(\d+)\.lock)\s+(\d+)\s*:(.*)$", l)
        if m:
            pids = [int(p) for p in re.findall(r"\d+", m.group(4))]
            out.append({"slot": int(m.group(2)), "inode": int(m.group(3)), "pids": pids})
    return sorted(out, key=lambda s: s["slot"])


def parse_bluetooth(lines: list) -> list:
    names = []
    for l in lines:
        l = re.sub(r"\x1b\[[0-9;]*m", "", l)
        m = re.match(r"\s*Device\s+(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}\s*(.*)$", l)
        if m:
            names.append(m.group(1).strip() or "(unnamed)")
    return names


def parse_ticks(lines: list):
    """(uptime_s, {pid: cpu ticks}) from a `ticks` section (/proc/uptime, then
    `pid utime+stime` lines)."""
    up, ticks = None, {}
    for l in lines:
        f = l.split()
        if len(f) == 2 and up is None and "." in f[0]:
            try:
                up = float(f[0])
            except ValueError:
                pass
            continue
        if len(f) == 2 and f[0].isdigit() and f[1].isdigit():
            ticks[int(f[0])] = int(f[1])
    return up, ticks


def cpu_per_pid(a: list, b: list, clk: int = 100) -> dict:
    """pid -> CPU % of one core between two `ticks` sections."""
    up1, t1 = parse_ticks(a)
    up2, t2 = parse_ticks(b)
    if up1 is None or up2 is None or up2 <= up1 or clk <= 0:
        return {}
    dt = up2 - up1
    return {pid: round(100.0 * max(0, v - t1[pid]) / clk / dt, 1) for pid, v in t2.items() if pid in t1}


def parse_pcpu(lines: list) -> dict:
    out = {}
    for l in lines:
        f = l.split()
        if len(f) == 2 and f[0].isdigit():
            try:
                out[int(f[0])] = float(f[1].replace(",", "."))
            except ValueError:
                pass
    return out


# ---- task markers (scripts/remote.sh) ----------------------------------------

# A marker whose PID belongs to a process started this much later than the
# marker is a reused PID: the run is gone.
PID_REUSE_SLACK_S = 120


def parse_marker(text):
    """One task marker (JSON written by scripts/remote.sh), normalized, or
    None if it is not one."""
    try:
        m = json.loads(text)
    except (ValueError, TypeError):
        return None
    if not isinstance(m, dict) or not isinstance(m.get("pid"), int) or not isinstance(m.get("start"), (int, float)):
        return None
    state = m.get("state") if m.get("state") in ("running", "waiting") else "running"
    slot = m.get("slot") if isinstance(m.get("slot"), int) else None
    slots = m.get("slots") if isinstance(m.get("slots"), int) and m["slots"] >= 0 else None
    s = lambda k: m.get(k) if isinstance(m.get(k), str) else None  # noqa: E731
    since = int(m["since"]) if isinstance(m.get("since"), (int, float)) and m["since"] >= m["start"] else int(m["start"])
    return {"pid": m["pid"], "start": int(m["start"]), "since": since, "state": state, "slot": slot, "slots": slots,
            "dir": s("checkout"), "branch": s("branch"), "command": s("command")}


def parse_marker_lines(lines: list) -> list:
    out = []
    for l in lines:
        l = l.strip().lstrip("\ufeff")
        if l.startswith("{"):
            m = parse_marker(l)
            if m:
                out.append(m)
    return out


def marker_stale(m: dict, alive: bool, proc_start, slack: float = PID_REUSE_SLACK_S) -> bool:
    """A marker is stale when its PID is gone or now belongs to a process
    started well after the marker (PID reused)."""
    if not alive:
        return True
    return proc_start is not None and proc_start > m["start"] + slack


def marker_label(m: dict, repo_path: str, home=None) -> str:
    """Checkout label of a marker from the remote directory name."""
    lab = checkout_label("/" + m["dir"], repo_path, home) if m.get("dir") else None
    return lab or m.get("dir") or m.get("branch") or "other"


def live_markers(markers: list, now, procs_by_pid=None, alive=None, repo_path="~/src/OCW", home=None):
    """(live markers with label/elapsed, number of stale ones). Liveness comes
    from the process list (Linux/macOS: procs_by_pid) or from per-marker
    `alive`/`pstart` values checked on the machine (Windows: alive dict
    pid -> (alive, pstart))."""
    live, stale = [], 0
    for m in markers:
        if alive is not None:
            ok, pstart = alive.get(m["pid"], (False, None))
        else:
            p = (procs_by_pid or {}).get(m["pid"])
            ok = p is not None
            pstart = (now - p["elapsed"]) if ok and now is not None and p.get("elapsed") is not None else None
        if marker_stale(m, ok, pstart):
            stale += 1
            continue
        live.append(dict(m, label=marker_label(m, repo_path, home),
                         # running: since it got its slot; waiting: since it started
                         elapsed=max(0, now - (m["since"] if m["state"] == "running" else m["start"]))
                         if now is not None else None))
    live.sort(key=lambda m: m["start"])
    return live, stale


def adopt_labels(procs: list, markers: list, slack: float = 5) -> dict:
    """pid -> checkout label for processes that cannot be assigned otherwise
    (Windows: no command lines or parents), when exactly one run is active:
    the processes it started, i.e. those not older than the run."""
    running = [m for m in markers if m["state"] == "running"]
    if len(running) != 1 or running[0].get("elapsed") is None:
        return {}
    m = running[0]
    return {p["pid"]: m["label"] for p in procs
            if p.get("elapsed") is not None and p["elapsed"] <= m["elapsed"] + slack}


def merge_tasks(markers: list, groups: list) -> list:
    """Background tasks: one per live marker (checkout, command, state,
    running time), with the CPU/memory of the processes of its checkout;
    process groups without a marker (runs not started by remote.sh) follow."""
    by_label = {g["checkout"]: g for g in groups}
    used, tasks = set(), []
    for m in markers:
        t = {"checkout": m["label"], "command": m["command"], "state": m["state"], "slot": m["slot"],
             "elapsed": m["elapsed"], "started": m["start"], "rss": None, "cpu": None, "kinds": [],
             "source": "marker"}
        if m["state"] == "running":
            g = by_label.get(m["label"]) if m["label"] not in used else None
            if g is not None:
                used.add(m["label"])
                t.update(rss=g["rss"], cpu=g.get("cpu"), kinds=g["kinds"])
                t["command"] = t["command"] or g.get("command")
        tasks.append(t)
    for g in groups:
        if g["checkout"] not in used:
            tasks.append(dict(g, state="running", slot=None, started=None, source="process"))
    return tasks


def queue_from_markers(markers: list, total: int = DEFAULT_SLOTS, held=None, extra_waiting=()) -> dict:
    """Build slots and queue of a machine.

    markers: live markers (with label/elapsed); total: the configured slot
    count (machines.toml or OS default); a run started with another
    OCW_REMOTE_SLOTS reports its count in its marker, and the larger one
    counts. held: slot -> holder label (or None) seen on the machine itself
    (Linux flocks, macOS lock dirs), for runs without a marker; Windows'
    exclusively opened lock files cannot be inspected, there the markers
    are all. extra_waiting: labels of waiting runs without a marker (older
    remote.sh blocked in `flock 9`), or dicts with label and elapsed)."""
    holders = {m["slot"]: m["label"] for m in markers if m["state"] == "running" and m["slot"]}
    for sl, lab in (held or {}).items():
        if not holders.get(sl):
            holders[sl] = lab
    total = max([total] + [m["slots"] for m in markers if m.get("slots") is not None] + list(holders))
    queued = [{"checkout": m["label"], "command": m.get("command"), "since": m["start"],
               "elapsed": m.get("elapsed")} for m in markers if m["state"] == "waiting"]
    for w in extra_waiting:
        w = w if isinstance(w, dict) else {"label": w}
        queued.append({"checkout": w["label"], "command": None, "since": None, "elapsed": w.get("elapsed")})
    return {"total": total, "used": len(holders), "waiting": len(queued),
            "waiting_checkouts": [q["checkout"] for q in queued], "queued": queued,
            "slots": [{"slot": i, "held": i in holders, "holder": holders.get(i)} for i in range(1, total + 1)]}


# ---- Windows performance counters ---------------------------------------------

PERFMON_HINT = "n/a (add build user to Performance Monitor Users)"
PDH_CANNOT_READ_NAME_STRINGS = 0xC0000BC8  # Perflib registry denied


def windows_counters(data: dict) -> dict:
    """State of the PDH counter query: {"ok", "hint", "status"}."""
    st = data.get("pdh_status")
    if st == 0 and data.get("proc_queue") is not None:
        return {"ok": True, "hint": None, "status": 0}
    code = ("0x%08X" % (st & 0xFFFFFFFF)) if isinstance(st, int) else None
    if data.get("perfmon_member"):
        hint = "n/a (counters denied%s although in Performance Monitor Users: restart sshd)" % (
            " " + code if code else "")
    else:
        hint = PERFMON_HINT
    return {"ok": False, "hint": hint, "status": code}


# ---- processes -------------------------------------------------------------

LINKERS = {"cc", "c++", "gcc", "g++", "clang", "clang++", "ld", "ld.lld", "ld.bfd", "ld.gold",
           "ld64.lld", "ld64", "mold", "collect2", "lld", "rust-lld", "link", "lld-link"}
_TEST_BIN = re.compile(r"[/\\]target[/\\](?:[^/\\]+[/\\])?(?:debug|release)[/\\]deps[/\\][^/\\]+-[0-9a-f]{16}(?:\.exe)?$")
_TARGET_BIN = re.compile(r"[/\\]target[/\\](?:[^/\\]+[/\\])?(?:debug|release)[/\\]")


def _split_args(args: str) -> list:
    args = args.strip()
    if args.startswith('"'):  # Windows: "C:\path with space\x.exe" rest
        end = args.find('"', 1)
        if end > 0:
            return [args[1:end]] + args[end + 1:].split()
    return args.split()


def _base(exe: str) -> str:
    b = re.split(r"[/\\]", exe)[-1]
    return b[:-4] if b.lower().endswith(".exe") else b


def run_command(args: str):
    """The user command of a remote.sh session (`... cd REPO && . ~/.cargo/env
    && export X && CMD`), or of its PowerShell equivalent."""
    m = re.search(r"\.\s+~/\.cargo/env\s*&&\s*export\s+\S+\s*&&\s*(.+)$", args)
    if m:
        return m.group(1).strip()
    m = re.search(r"\$env:Path\s*=.*?;\s*(?:\$env:CARGO_BUILD_JOBS='\d+';\s*)?(.+?);\s*exit \$LASTEXITCODE", args)
    if m:
        return m.group(1).strip()
    return None


def classify_process(args: str):
    """Process kind for the task table, or None for unrelated processes."""
    toks = _split_args(args)
    if not toks:
        return None
    exe = toks[0]
    base = _base(exe)
    low = base.lower()
    rest = toks[1:]
    if low == "cargo":
        sub = next((t for t in rest if not t.startswith(("-", "+"))), "")
        return ("cargo " + sub).strip()
    if low in ("rustc", "clippy-driver"):
        return "rustc"
    if low.startswith("build-script-"):
        return "build script"
    if low in LINKERS:
        return "link"
    if _TEST_BIN.search(exe):
        return "test"
    if low in ("ocw-desktop", "ocw-server", "world-builder"):
        return base
    if low == "ffmpeg":
        return "ffmpeg"
    if low == "weston":
        return "weston"
    if low == "xvfb":
        return "Xvfb"
    if low == "flock" and rest[-1:] == ["9"]:
        return "waiting for slot"
    if low in ("bash", "sh", "zsh", "powershell", "pwsh") and run_command(args):
        return "run"
    if any(t.endswith("ocw-kiosk.sh") for t in toks[:2]):
        return "kiosk loop"
    if _TARGET_BIN.search(exe):
        return "binary"
    return None


def _expand(path: str, home) -> str:
    if home and (path == "~" or path.startswith("~/") or path.startswith("~\\")):
        return home.rstrip("/\\") + path[1:]
    return path


def checkout_label(path, repo_path: str = "~/src/OCW", home=None):
    """Checkout a path belongs to: "main", "agent <id8>", a branch name,
    "kiosk" (/var/tmp/ocw), or None."""
    if not path:
        return None
    p = path.replace("\\", "/")
    if p.startswith("/var/tmp/ocw"):
        return "kiosk"
    repo = _expand(repo_path, home).replace("\\", "/").rstrip("/")
    ci = re.match(r"^[A-Za-z]:/", repo) is not None  # Windows: case-insensitive
    pc, rc = (p.lower(), repo.lower()) if ci else (p, repo)
    branch = None
    if pc == rc or pc.startswith(rc + "/"):
        return "main"
    if pc.startswith(rc + "-"):
        branch = p[len(repo) + 1:].split("/")[0]
    else:
        base = repo.split("/")[-1] or "OCW"
        m = re.search(r"/" + re.escape(base) + r"(?:-([^/]+))?(?:/|$)", p, re.I if ci else 0)
        if not m:
            return None
        branch = m.group(1)
        if branch is None:
            return "main"
    m = re.match(r"worktree-agent-([0-9a-f]+)$", branch)
    if m:
        return "agent " + m.group(1)[:8]
    return branch


def _label_from_args(args: str, repo_path: str, home):
    # remote.sh's `cd REPO && . ~/.cargo/env` first (its task marker prelude
    # may quote a user command with another `cd`), then any cd.
    m = (re.search(r"\bcd\s+'?([^\s']+)'?\s*&&\s*\.\s+~/\.cargo/env", args)
         or re.search(r"(?:\bcd|Set-Location -LiteralPath)\s+'?([^\s']+)'?", args))
    if m:
        lab = checkout_label(m.group(1), repo_path, home)
        if lab:
            return lab
    for t in _split_args(args):
        if "/" in t or "\\" in t:
            lab = checkout_label(t.split("=", 1)[-1], repo_path, home)
            if lab:
                return lab
    return None


def _shorten(cmd: str, n: int = 90) -> str:
    cmd = " ".join(cmd.split())
    return cmd if len(cmd) <= n else cmd[: n - 1] + "…"


def build_tasks(procs: list, repo_path: str, home=None, self_pid=None, fallback=None, queued_pids=()) -> dict:
    """Groups classified processes by checkout. Returns
    {"groups": [...], "kiosk_proc": proc|None, "waiting": [labels]}."""
    by_pid = {p["pid"]: p for p in procs}
    # Drop the poll's own shell and its children.
    skip = set()
    if self_pid is not None:
        skip.add(self_pid)
        changed = True
        while changed:
            changed = False
            for p in procs:
                if p["ppid"] in skip and p["pid"] not in skip:
                    skip.add(p["pid"])
                    changed = True

    own: dict = {}

    def label(p, depth=0):
        if p["pid"] in own:
            return own[p["pid"]]
        lab = (checkout_label(p.get("cwd"), repo_path, home)
               or _label_from_args(p["args"], repo_path, home))
        if lab is None and depth < 30:
            parent = by_pid.get(p["ppid"])
            if parent is not None and parent["pid"] != p["pid"]:
                lab = label(parent, depth + 1)
        own[p["pid"]] = lab
        return lab

    # Sessions blocked in `flock 9` (older remote.sh) or whose task marker
    # says "waiting" (queued_pids) are queued, not running.
    queued = {p["ppid"] for p in procs if classify_process(p["args"]) == "waiting for slot"} | set(queued_pids)
    groups: dict = {}
    kiosk = None
    waiting = []
    for p in procs:
        if p["pid"] in skip:
            continue
        kind = classify_process(p["args"])
        if kind is None:
            continue
        parent = by_pid.get(p["ppid"])
        # rustup proxies: `cargo` → `cargo`, `rustc` → `rustc`; count once.
        if parent is not None and kind in ("rustc",) and classify_process(parent["args"]) == kind:
            continue
        if parent is not None and kind.startswith("cargo") and classify_process(parent["args"]) == kind:
            continue
        lab = label(p) or (fallback or {}).get(p["pid"]) or "other"
        if kind == "ocw-desktop" and lab == "kiosk":
            kiosk = p
            continue
        if kind == "kiosk loop":
            continue
        if kind == "waiting for slot":
            wl = label(parent) if parent is not None else lab
            waiting.append({"pid": p["ppid"], "label": wl or "other", "elapsed": p.get("elapsed")})
            continue
        if kind == "run" and (p["pid"] in queued or (
                parent is not None and classify_process(parent["args"]) == "run")):
            continue
        g = groups.setdefault(lab, {"checkout": lab, "command": None, "elapsed": 0, "rss": 0, "cpu": None,
                                    "kinds": {}})
        if kind == "run":
            g["command"] = _shorten(run_command(p["args"]) or "")
            g["elapsed"] = max(g["elapsed"], p["elapsed"] or 0)
            continue
        k = g["kinds"].setdefault(kind, {"kind": kind, "count": 0, "elapsed": 0, "rss": 0, "cpu": None,
                                         "example": None})
        k["count"] += 1
        k["elapsed"] = max(k["elapsed"], p["elapsed"] or 0)
        k["rss"] += p["rss"] or 0
        if p.get("cpu") is not None:
            k["cpu"] = round((k["cpu"] or 0) + p["cpu"], 1)
            g["cpu"] = round((g["cpu"] or 0) + p["cpu"], 1)
        if k["example"] is None:
            k["example"] = _shorten(p["args"])
        g["elapsed"] = max(g["elapsed"], p["elapsed"] or 0)
        g["rss"] += p["rss"] or 0
    order = ["cargo", "build script", "rustc", "link", "test", "binary", "ocw-desktop",
             "ocw-server", "world-builder", "ffmpeg", "weston", "Xvfb"]

    def rank(kind):
        for i, o in enumerate(order):
            if kind.startswith(o):
                return i
        return len(order)

    out = []
    for g in groups.values():
        g["kinds"] = sorted(g["kinds"].values(), key=lambda k: rank(k["kind"]))
        out.append(g)
    out.sort(key=lambda g: (g["checkout"] != "main", g["checkout"]))
    return {"groups": out, "kiosk_proc": kiosk, "waiting": waiting}


def held_flock_slots(slot_lines: list, locks: dict, procs: list, repo_path, home, skip_pids=()) -> dict:
    """Linux: slot -> holder label (or None) of the held
    /tmp/ocw-build-slot-N.lock flocks. Waiting runs (blocked in `flock 9`,
    or polling with the file briefly open: skip_pids) are not holders."""
    by_pid = {p["pid"]: p for p in procs}
    waiter_pids = set(skip_pids)
    for p in procs:
        if classify_process(p["args"]) == "waiting for slot":
            waiter_pids.add(p["pid"])
            waiter_pids.add(p["ppid"])
    held = {}
    for s in slot_lines:
        if not locks.get(s["inode"], {}).get("held"):
            continue
        holder = None
        for pid in s["pids"]:
            if pid in waiter_pids or pid not in by_pid:
                continue
            p = by_pid[pid]
            holder = (checkout_label(p.get("cwd"), repo_path, home)
                      or _label_from_args(p["args"], repo_path, home))
            if holder:
                break
        held[s["slot"]] = holder
    return held


def parse_slot_dirs(lines: list, procs_by_pid: dict, procs=(), repo_path="~/src/OCW", home=None) -> dict:
    """macOS: slot -> holder label (or None) from `ocw-build-slot-N.d PID`
    lines; a directory whose PID is gone is free (the next run takes it)."""
    held = {}
    for l in lines:
        m = re.match(r"ocw-build-slot-(\d+)\.d(?:\s+(\d+))?\s*$", l.strip())
        if not m:
            continue
        pid = int(m.group(2)) if m.group(2) else None
        if pid is None or pid not in procs_by_pid:
            continue
        p = procs_by_pid[pid]
        held[int(m.group(1))] = (checkout_label(p.get("cwd"), repo_path, home)
                                 or _label_from_args(p["args"], repo_path, home))
    return held


def _int0(lines):
    """First line of a section as int, or None."""
    try:
        return int(str((lines or [""])[0]).strip())
    except ValueError:
        return None


def parse_linux(text: str, machine_repo: str = "~/src/OCW", slots: int = OS_DEFAULT_SLOTS["linux"]) -> dict:
    s = parse_sections(text)
    home = (s.get("home") or [None])[0]
    uid = int(s["home"][1]) if len(s.get("home", [])) > 1 and s["home"][1].strip().isdigit() else None
    self_pid = int(s["self"][0]) if s.get("self") and s["self"][0].strip().isdigit() else None
    out: dict = {"home": home}
    try:
        out["ncpu"] = int(s["nproc"][0])
    except Exception:
        out["ncpu"] = None
    la = (s.get("loadavg") or [""])[0].split()
    out["load"] = [float(x) for x in la[:3]] if len(la) >= 3 else None
    up = (s.get("uptime") or [""])[0].split()
    out["uptime"] = float(up[0]) if up else None
    out.update(parse_meminfo(s.get("meminfo", [])))
    out.update(parse_df(s.get("df", [])))
    out["cpu_busy"] = parse_cpu_busy(s.get("stat", []) + s.get("stat2", []))
    procs = parse_ps(s.get("ps", []))
    cwd = parse_cwd_linux(s.get("cwd", []))
    clk = _int0(s.get("clk")) or 100
    cpu = cpu_per_pid(s.get("ticks1", []), s.get("ticks2", []), clk)
    for p in procs:
        p["cwd"] = cwd.get(p["pid"])
        p["cpu"] = cpu.get(p["pid"])
    markers, out["stale_markers"] = live_markers(
        parse_marker_lines(s.get("tasks", [])), _int0(s.get("now")),
        {p["pid"]: p for p in procs}, repo_path=machine_repo, home=home)
    tasks = build_tasks(procs, machine_repo, home, self_pid,
                        queued_pids={m["pid"] for m in markers if m["state"] == "waiting"})
    out["tasks"] = merge_tasks(markers, tasks["groups"])
    waiting_pids = {m["pid"] for m in markers if m["state"] == "waiting"}
    held = held_flock_slots(parse_slot_lines(s.get("slots", [])), parse_proc_locks(s.get("locks", [])),
                            procs, machine_repo, home, waiting_pids)
    q = queue_from_markers(markers, slots, held,
                           [w for w in tasks["waiting"] if w["pid"] not in waiting_pids])
    out["queue"] = q
    version = [l for l in s.get("kiosk", []) if l.strip()]
    kp = tasks["kiosk_proc"]
    if version or kp:
        out["kiosk"] = {
            "running": kp is not None,
            "elapsed": kp["elapsed"] if kp else None,
            "rss": kp["rss"] if kp else None,
            "other_user": (kp["uid"] != uid) if kp and uid is not None else None,
            "version": version[0].strip() if version else None,
            "bluetooth": parse_bluetooth(s.get("bt", [])),
        }
    return out


def parse_macos(text: str, machine_repo: str = "~/src/OCW", slots: int = OS_DEFAULT_SLOTS["macos"]) -> dict:
    s = parse_sections(text)
    home = (s.get("home") or [None])[0]
    self_pid = int(s["self"][0]) if s.get("self") and s["self"][0].strip().isdigit() else None
    out: dict = {"home": home}
    sc = s.get("sysctl", [])
    try:
        out["ncpu"] = int(sc[0])
        out["mem_total"] = int(sc[1])
    except Exception:
        out["ncpu"] = None
    for l in sc:
        m = re.match(r"\{\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\}", l.strip())
        if m:
            out["load"] = [float(x) for x in m.groups()]
        m = re.search(r"sec = (\d+)", l)
        if m and s.get("now"):
            try:
                out["uptime"] = float(int(s["now"][0]) - int(m.group(1)))
            except ValueError:
                pass
        m = re.search(r"total = ([\d.]+)M\s+used = ([\d.]+)M", l)
        if m:
            out["swap_total"] = int(float(m.group(1)) * 1024 * 1024)
            out["swap_used"] = int(float(m.group(2)) * 1024 * 1024)
    page = 4096
    pages = {}
    for l in s.get("vmstat", []):
        m = re.search(r"page size of (\d+) bytes", l)
        if m:
            page = int(m.group(1))
        m = re.match(r'"?([^:"]+)"?:\s+(\d+)\.?$', l.strip())
        if m:
            pages[m.group(1).strip()] = int(m.group(2))
    if out.get("mem_total") and pages:
        avail = (pages.get("Pages free", 0) + pages.get("Pages inactive", 0)
                 + pages.get("Pages speculative", 0)) * page
        out["mem_used"] = max(0, out["mem_total"] - avail)
    out.update(parse_df(s.get("df", [])))
    cpu = " ".join(s.get("cpu", []))
    m = re.search(r"([\d.]+)% idle", cpu)
    out["cpu_busy"] = round(100.0 - float(m.group(1)), 1) if m else None
    procs = parse_ps(s.get("ps", []), etime=True)
    cwd = parse_cwd_lsof(s.get("cwd", []))
    pcpu = parse_pcpu(s.get("pcpu", []))  # macOS %cpu: a recent (decaying) average
    for p in procs:
        p["cwd"] = cwd.get(p["pid"])
        p["cpu"] = pcpu.get(p["pid"])
    markers, out["stale_markers"] = live_markers(
        parse_marker_lines(s.get("tasks", [])), _int0(s.get("now")),
        {p["pid"]: p for p in procs}, repo_path=machine_repo, home=home)
    tasks = build_tasks(procs, machine_repo, home, self_pid,
                        queued_pids={m["pid"] for m in markers if m["state"] == "waiting"})
    out["tasks"] = merge_tasks(markers, tasks["groups"])
    by_pid = {p["pid"]: p for p in procs}
    held = parse_slot_dirs(s.get("slots", []), by_pid, procs, machine_repo, home)
    out["queue"] = queue_from_markers(markers, slots, held)
    return out


def parse_windows(text: str, machine_repo: str = "B:\\src\\OCW", slots: int = OS_DEFAULT_SLOTS["windows"]) -> dict:
    start = text.find("{")
    data = json.loads(text[start:]) if start >= 0 else {}
    home = data.get("home")
    cpu_busy = data.get("cpu_busy")
    out: dict = {"home": home, "ncpu": data.get("ncpu"), "load": None, "uptime": data.get("uptime"),
                 "cpu_busy": round(float(cpu_busy), 1) if isinstance(cpu_busy, (int, float)) else None,
                 "runnable": data.get("runnable"), "proc_queue": data.get("proc_queue"),
                 "counters": windows_counters(data)}
    if data.get("mem_total"):
        out["mem_total"] = data["mem_total"]
        if data.get("mem_avail") is not None:
            out["mem_used"] = data["mem_total"] - data["mem_avail"]
    disks = data.get("disks") or []
    if isinstance(disks, dict):
        disks = [disks]
    want = (machine_repo[:1] or "C").upper()
    disk = next((d for d in disks if str(d.get("name", "")).upper() == want), disks[0] if disks else None)
    if disk:
        out["disk_used"] = disk.get("used")
        out["disk_free"] = disk.get("free")
        out["disk_total"] = (disk.get("used") or 0) + (disk.get("free") or 0)
        out["disk_name"] = str(disk.get("name")) + ":"
    raw = data.get("procs") or []
    if isinstance(raw, dict):
        raw = [raw]
    procs = []
    for p in raw:
        el = p.get("elapsed")
        cpu = p.get("cpu")
        procs.append({"pid": p.get("pid"), "ppid": p.get("ppid"), "uid": None,
                      "elapsed": int(el) if isinstance(el, (int, float)) else None,
                      "cpu": round(max(0.0, cpu), 1) if isinstance(cpu, (int, float)) else None,
                      "rss": p.get("rss"), "args": p.get("args") or p.get("name") or "", "cwd": None})
    raw_markers = data.get("markers") or []
    if isinstance(raw_markers, dict):
        raw_markers = [raw_markers]
    markers, alive = [], {}
    for rm in raw_markers:
        m = parse_marker(str(rm.get("text") or "").strip().lstrip("\ufeff")) if isinstance(rm, dict) else None
        if m:
            markers.append(m)
            alive[m["pid"]] = (bool(rm.get("alive")), rm.get("pstart"))
    now = data.get("now")
    markers, out["stale_markers"] = live_markers(markers, now if isinstance(now, (int, float)) else None,
                                                 alive=alive, repo_path=machine_repo, home=home)
    # No command lines or parents without CIM: when a single run is active,
    # the processes started since it began are its own.
    tasks = build_tasks(procs, machine_repo, home, data.get("self"), adopt_labels(procs, markers))
    out["tasks"] = merge_tasks(markers, tasks["groups"])
    out["queue"] = queue_from_markers(markers, slots)
    return out


# --------------------------------------------------------------------------
# Polling
# --------------------------------------------------------------------------


def control_dir() -> str:
    """Private directory for SSH control sockets, outside the repository."""
    base = os.environ.get("XDG_RUNTIME_DIR")
    if not base or not os.path.isdir(base):
        base = tempfile.gettempdir()
    d = os.path.join(base, "ocw-build-dashboard-%d" % os.getuid() if hasattr(os, "getuid") else "ocw-build-dashboard")
    os.makedirs(d, mode=0o700, exist_ok=True)
    try:
        os.chmod(d, 0o700)
    except OSError:
        pass
    return d


def ssh_argv(m: Machine, ctl: str | None) -> list:
    argv = ["ssh", "-p", str(m.port), "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
            "-o", "UserKnownHostsFile=" + m.known_hosts, "-o", "StrictHostKeyChecking=yes",
            "-o", "ConnectTimeout=%d" % m.timeout, "-o", "LogLevel=ERROR",
            "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"]
    if m.identity:
        argv[1:1] = ["-i", m.identity]
    if ctl:
        argv += ["-o", "ControlMaster=auto", "-o", "ControlPath=" + os.path.join(ctl, "%C"),
                 "-o", "ControlPersist=60"]
    argv.append("%s@%s" % (m.user, m.host) if m.user else str(m.host))
    return argv


def poll_machine(m: Machine, ctl: str | None = None) -> dict:
    t0 = time.time()
    base = {"name": m.name, "os": m.os, "roles": m.roles, "slots": m.slots, "polled_at": t0}
    if not m.host:
        return dict(base, online=False, error="no host configured")
    masker = Masker(m.secrets())
    if m.os == "windows":
        cmd, parser = windows_command(), parse_windows
    elif m.os == "macos":
        cmd, parser = MACOS_SCRIPT, parse_macos
    else:
        cmd, parser = LINUX_SCRIPT, parse_linux
    try:
        r = subprocess.run(ssh_argv(m, ctl) + [cmd], capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=m.timeout + 20, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return dict(base, online=False, error="timeout", detail="no answer within %d s" % (m.timeout + 20))
    except OSError as e:
        return dict(base, online=False, error="ssh not available", detail=masker(str(e)))
    if r.returncode != 0 and "@@end" not in r.stdout and "{" not in r.stdout:
        err = r.stderr.strip().splitlines()
        return dict(base, online=False, error=classify_ssh_error(r.stderr, r.returncode),
                    detail=masker(err[-1] if err else ""))
    try:
        data = parser(r.stdout, m.repo_path, m.slots)
    except Exception as e:  # a parser bug must not kill the poller
        return dict(base, online=False, error="unexpected output", detail=masker("%s: %s" % (type(e).__name__, e)))
    home = data.pop("home", None)
    masker = Masker(m.secrets(), [home] if home else [])
    data = masker.deep(data)
    return dict(base, online=True, poll_ms=int((time.time() - t0) * 1000), **data)


class Dashboard:
    def __init__(self, machines: list, interval: float = POLL_INTERVAL_S, use_control: bool = True):
        self.machines = machines
        self.interval = interval
        self.ctl = control_dir() if use_control else None
        self.lock = threading.Lock()
        self.state = {m.name: {"name": m.name, "os": m.os, "roles": m.roles, "slots": m.slots, "online": None,
                               "error": "connecting"} for m in machines}
        self.last_ok: dict = {}
        self.stop = threading.Event()

    def _loop(self, m: Machine):
        while not self.stop.is_set():
            t0 = time.time()
            st = poll_machine(m, self.ctl)
            if st.get("online"):
                self.last_ok[m.name] = st["polled_at"]
            elif m.name in self.last_ok:
                st["last_seen"] = self.last_ok[m.name]
            with self.lock:
                self.state[m.name] = st
            self.stop.wait(max(0.5, self.interval - (time.time() - t0)))

    def start(self):
        for m in self.machines:
            threading.Thread(target=self._loop, args=(m,), daemon=True, name="poll-" + m.name).start()

    def snapshot(self) -> dict:
        with self.lock:
            machines = [dict(self.state[m.name]) for m in self.machines]
        status = summarize(machines)
        if getattr(self, "prompts", None) is not None:
            status = annotate_prompts(status, self.prompts, getattr(self, "prompt_mask", None))
        return status


def summarize(machines: list) -> dict:
    online = [m for m in machines if m.get("online")]
    tasks = sum(1 for m in online for t in (m.get("tasks") or []) if t.get("state") != "waiting")
    waiting = sum((m.get("queue") or {}).get("waiting", 0) for m in online)
    used = sum((m.get("queue") or {}).get("used", 0) for m in online)
    total = sum((m.get("queue") or {}).get("total", 0) if m.get("online") else (m.get("slots") or 0)
                for m in machines)
    return {"generated_at": time.time(), "machines": machines,
            "overview": {"machines": len(machines), "online": len(online), "tasks": tasks,
                         "waiting": waiting, "slots_used": used, "slots_total": total}}


# --------------------------------------------------------------------------
# Text output (--once)
# --------------------------------------------------------------------------


def fmt_bytes(n) -> str:
    if n is None:
        return "?"
    n = float(n)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(n) < 1024 or unit == "TiB":
            return ("%.0f %s" if unit in ("B", "KiB") else "%.1f %s") % (n, unit)
        n /= 1024
    return "?"


def fmt_dur(s) -> str:
    if s is None:
        return "?"
    s = int(s)
    d, s = divmod(s, 86400)
    h, s = divmod(s, 3600)
    mi, s = divmod(s, 60)
    if d:
        return "%dd %dh" % (d, h)
    if h:
        return "%dh %02dm" % (h, mi)
    if mi:
        return "%dm %02ds" % (mi, s)
    return "%ds" % s


def fmt_cpu(c) -> str:
    """CPU % of one core (100 % = one core busy)."""
    return "?" if c is None else "%.0f%%" % c


def task_state(t: dict) -> str:
    if t.get("state") == "waiting":
        return "waiting for slot"
    return "running" + (" (slot %d)" % t["slot"] if t.get("slot") else "")


def task_usage(t: dict) -> list:
    out = [fmt_dur(t.get("elapsed"))]
    if t.get("cpu") is not None:
        out.append("cpu " + fmt_cpu(t["cpu"]))
    if t.get("rss"):
        out.append(fmt_bytes(t["rss"]))
    if t.get("source") == "process":
        out.append("no marker")
    return out


def format_text(status: dict) -> str:
    o = status["overview"]
    lines = ["%d/%d machines online: %d running, %d waiting, build slots %d used of %d"
             % (o["online"], o["machines"], o["tasks"], o["waiting"], o["slots_used"], o["slots_total"]), ""]
    for m in status["machines"]:
        if not m.get("online"):
            det = (" - " + m["detail"]) if m.get("detail") else ""
            lines.append("%-14s %-7s UNREACHABLE (%s)%s" % (m["name"], m["os"], m.get("error"), det))
            lines.append("")
            continue
        parts = []
        if m.get("cpu_busy") is not None:
            parts.append("cpu %.0f%%" % m["cpu_busy"])
        if m.get("load"):
            parts.append("load %.2f/%s" % (m["load"][0], m.get("ncpu") or "?"))
        elif m.get("runnable") is not None:
            parts.append("runnable threads %s/%s" % (m["runnable"], m.get("ncpu") or "?"))
        elif m.get("ncpu"):
            parts.append("%s cpus" % m["ncpu"])
        if m.get("mem_total"):
            parts.append("mem %s/%s" % (fmt_bytes(m.get("mem_used")), fmt_bytes(m["mem_total"])))
        if m.get("swap_total"):
            parts.append("swap %s" % fmt_bytes(m.get("swap_used")))
        if m.get("disk_free") is not None:
            parts.append("disk %s free" % fmt_bytes(m["disk_free"]))
        if m.get("uptime"):
            parts.append("up %s" % fmt_dur(m["uptime"]))
        c = m.get("counters")
        if c:
            parts.append("processor queue %s" % (
                ("%.0f" % m["proc_queue"]) if c.get("ok") and m.get("proc_queue") is not None else c.get("hint")))
        lines.append("%-14s %-7s online  %s" % (m["name"], m["os"], ", ".join(parts)))
        q = m.get("queue")
        if q:
            held = ", ".join("slot %d: %s" % (s["slot"], s["holder"] or "held") for s in q["slots"] if s["held"])
            lines.append("  slots: %d used of %d%s, %d waiting" % (
                q["used"], q["total"], (" (" + held + ")") if held else "", q["waiting"]))
            for w in q.get("queued") or []:
                lines.append("  queue: [%s] %s  (waiting %s)" % (
                    w["checkout"], w.get("command") or "(command unknown)", fmt_dur(w.get("elapsed"))))
        k = m.get("kiosk")
        if k:
            st = ("running %s" % fmt_dur(k["elapsed"])) if k["running"] else "NOT running"
            bt = ", ".join(k.get("bluetooth") or []) or "none"
            lines.append("  kiosk: app %s, build %s, bluetooth: %s" % (st, k.get("version") or "?", bt))
        tasks = [t for t in (m.get("tasks") or []) if t.get("state") != "waiting"]
        if not tasks:
            lines.append("  no build tasks running")
        for g in tasks:
            lines.append("  [%s] %s: %s  (%s)" % (g["checkout"], task_state(g), g["command"] or "(command unknown)",
                                                 ", ".join(task_usage(g))))
            for kd in g["kinds"]:
                lines.append("      %-16s x%-3d %8s %7s %10s" % (
                    kd["kind"], kd["count"], fmt_dur(kd["elapsed"]),
                    fmt_cpu(kd.get("cpu")) if kd.get("cpu") is not None else "", fmt_bytes(kd["rss"])))
        if m.get("stale_markers"):
            lines.append("  (%d stale task marker(s) of ended runs ignored)" % m["stale_markers"])
        lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# HTTP
# --------------------------------------------------------------------------


def make_handler(dash: Dashboard):
    page = (HERE / "index.html").read_bytes()

    class Handler(BaseHTTPRequestHandler):
        def _send(self, code, body: bytes, ctype: str):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):  # noqa: N802
            path = self.path.split("?", 1)[0]
            if path in ("/", "/index.html"):
                self._send(200, page, "text/html; charset=utf-8")
            elif path == "/api/status":
                self._send(200, json.dumps(dash.snapshot()).encode(), "application/json")
            else:
                self._send(404, b"not found\n", "text/plain")

        def log_message(self, fmt, *args):  # quiet: no request logs
            pass

    return Handler


# ---------------------------------------------------------------- prompts
#
# Which prompt a job belongs to: read from the Claude Code session
# transcripts of this project (~/.claude/projects/*OCW*/*.jsonl, main sessions
# only). A job in an agent worktree (checkout ".../worktree-agent-<id>" or
# "OCW-worktree-agent-<id>") gets the user prompt that preceded the Agent call
# that started agent <id> (plus the agent's short description); any other job
# gets the latest user prompt before the job started.

PROMPT_SHORT = 90
_REMINDER = re.compile(r"<system-reminder>.*?</system-reminder>", re.S)
_AGENT_ID = re.compile(r"agentId:\s*([0-9a-f]{12,})")
_CHECKOUT_AGENT = re.compile(r"agent[- ]([0-9a-f]{6,})")
_NOT_PROMPT = ("<task-notification", "<agent-message", "[SYSTEM NOTIFICATION", "Another Claude session",
               "<local-command", "[Request interrupted", "Caveat:", "<command-message",
               "This session is being continued")


def prompt_from_text(raw: str):
    """The user's words in a transcript message, or None if it is not a human
    prompt (notifications, agent messages, tool output)."""
    t = _REMINDER.sub("", raw or "").strip()
    if not t:
        return None
    m = re.match(r"<command-name>(/?[^<]+)</command-name>", t)
    if m:
        return "/" + m.group(1).lstrip("/")
    if any(t.startswith(b) or b in t[:200] for b in _NOT_PROMPT):
        return None
    return t


def short_prompt(text: str, n: int = PROMPT_SHORT) -> str:
    """First line, whitespace collapsed, at most n characters."""
    t = " ".join((text or "").strip().splitlines()[0].split()) if text and text.strip() else ""
    return t if len(t) <= n else t[: n - 1].rstrip() + "…"


def _epoch(ts: str):
    try:
        from datetime import datetime
        return datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
    except (ValueError, AttributeError):
        return None


class PromptIndex:
    """Incremental reader of the transcripts: user prompts with their time and
    the prompt + description that started each agent."""

    def __init__(self, root: Path | None = None):
        self.root = root or Path(os.environ.get("OCW_TRANSCRIPTS", str(Path.home() / ".claude" / "projects")))
        self.offsets = {}
        self.prompts = []          # (epoch, text), in file order
        self.pending = {}          # Agent tool_use id -> (description, prompt)
        self.agents = {}           # agent id -> {"prompt": ..., "agent": description}
        self.last = None           # last prompt text seen
        self.lock = threading.Lock()

    def feed(self, d: dict):
        t = d.get("type")
        if t == "user" and not d.get("isMeta"):
            c = (d.get("message") or {}).get("content")
            if isinstance(c, str):
                text = prompt_from_text(c)
                if text:
                    self.last = text
                    self.prompts.append((_epoch(d.get("timestamp", "")), text))
            elif isinstance(c, list):
                for b in c:
                    if isinstance(b, dict) and b.get("type") == "tool_result" and b.get("tool_use_id") in self.pending:
                        body = b.get("content")
                        body = body if isinstance(body, str) else json.dumps(body)
                        m = _AGENT_ID.search(body or "")
                        desc, prompt = self.pending.pop(b["tool_use_id"])
                        if m:
                            self.agents[m.group(1)] = {"prompt": prompt, "agent": desc}
        elif t == "attachment":
            a = d.get("attachment") or {}
            if a.get("type") == "queued_command" and (a.get("origin") or {}).get("kind") == "human":
                text = prompt_from_text(str(a.get("prompt", "")))
                if text:
                    self.last = text
                    self.prompts.append((_epoch(d.get("timestamp", "")), text))
        elif t == "assistant":
            for b in (d.get("message") or {}).get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == "Agent":
                    desc = str((b.get("input") or {}).get("description", ""))
                    self.pending[b.get("id")] = (desc, self.last)

    def refresh(self):
        files = sorted(self.root.glob("*OCW*/*.jsonl")) if self.root.is_dir() else []
        with self.lock:
            for f in files:
                try:
                    size = f.stat().st_size
                    off = self.offsets.get(f, 0)
                    if size <= off:
                        continue
                    with open(f, "rb") as fh:
                        fh.seek(off)
                        data = fh.read()
                    end = data.rfind(b"\n") + 1   # only whole lines
                    for line in data[:end].splitlines():
                        if b'"user"' in line or b'"attachment"' in line or b'"Agent"' in line:
                            try:
                                self.feed(json.loads(line))
                            except ValueError:
                                pass
                    self.offsets[f] = off + end
                except OSError:
                    continue
            self.prompts.sort(key=lambda p: p[0] or 0)

    def for_job(self, checkout: str, started=None) -> dict:
        """{"prompt": short text, "agent": description} for a job, or {}."""
        with self.lock:
            m = _CHECKOUT_AGENT.search(checkout or "")
            hits = [v for k, v in self.agents.items() if m and k.startswith(m.group(1))]
            if len(hits) == 1:  # the label may carry a shortened agent id
                a = hits[0]
                return {"prompt": short_prompt(a["prompt"] or ""), "agent": a["agent"]}
            if m:
                return {}
            best = None
            for ts, text in self.prompts:
                if ts is not None and (started is None or ts <= float(started)):
                    best = text
            return {"prompt": short_prompt(best)} if best else {}


def annotate_prompts(status: dict, index, mask=None) -> dict:
    """Adds "prompt" (and "agent") to every task and queued run of the status."""
    def walk(o):
        if isinstance(o, dict):
            if "checkout" in o and ("command" in o or "state" in o):
                info = index.for_job(o.get("checkout"), o.get("started"))
                for k, v in info.items():
                    o[k] = mask(v) if mask else v
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)
    walk(status)
    return status


def prompt_loop(dash, index, interval: float = 15.0):
    index.refresh()
    while not dash.stop.wait(interval):
        index.refresh()


RECORD_INTERVAL_S = 60.0


def record_line(status: dict, now: float | None = None) -> str:
    """One JSON line of the build servers' state for the art project's build
    log (art/ART-PROJECT.md §2): the dashboard's status (already masked: no
    host names, addresses or user names) with the time it was taken."""
    t = time.localtime(now if now is not None else time.time())
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S", t)
    off = time.strftime("%z", t)
    stamp += off[:3] + ":" + off[3:] if off else ""
    return json.dumps({"time": stamp, "status": status}, separators=(",", ":"), sort_keys=True)


def record_path(root: Path, now: float | None = None) -> Path:
    """root/YYYY-MM-DD/servers.jsonl for the local day of `now`."""
    day = time.strftime("%Y-%m-%d", time.localtime(now if now is not None else time.time()))
    return root / day / "servers.jsonl"


def record_loop(dash, root: Path, interval: float = RECORD_INTERVAL_S):
    """Appends a status line every `interval` seconds until dash.stop is set."""
    while not dash.stop.wait(interval):
        try:
            p = record_path(root)
            p.parent.mkdir(parents=True, exist_ok=True)
            with open(p, "a", encoding="utf-8") as f:
                f.write(record_line(dash.snapshot()) + "\n")
        except Exception as e:  # recording must never stop the dashboard
            print("build log: %s" % e, file=sys.stderr)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="OCW build server dashboard (read-only)")
    ap.add_argument("--host", default="0.0.0.0", help="address to listen on (default: all; 127.0.0.1 = local only)")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--once", action="store_true", help="poll all machines once, print text, exit")
    ap.add_argument("--json", action="store_true", help="with --once: print the JSON of /api/status")
    ap.add_argument("--interval", type=float, default=POLL_INTERVAL_S, help="seconds between polls")
    ap.add_argument("--config", type=Path, help="config directory (default: config/ of the checkout)")
    ap.add_argument("--no-control-master", action="store_true", help="new SSH connection per poll")
    ap.add_argument("--record", type=Path, default=None,
                    help="build log directory for the art project (default: test-output/build-log of the checkout)")
    ap.add_argument("--no-record", action="store_true", help="don't write the build log")
    ap.add_argument("--no-prompts", action="store_true", help="don't show the prompt each job belongs to")
    ap.add_argument("--record-interval", type=float, default=RECORD_INTERVAL_S, help="seconds between build-log lines")
    a = ap.parse_args(argv)

    repo = HERE.parent.parent
    cfg = a.config or find_config_dir(repo)
    if not (cfg / "machines.toml").is_file():
        print("%s/machines.toml missing (copy config/machines.example.toml)" % cfg, file=sys.stderr)
        return 1
    machines = load_machines(cfg)
    dash = Dashboard(machines, a.interval, not a.no_control_master)
    if a.once:
        with ThreadPoolExecutor(max_workers=max(1, len(machines))) as ex:
            states = list(ex.map(lambda m: poll_machine(m, dash.ctl), machines))
        status = summarize(states)
        print(json.dumps(status, indent=2) if a.json else format_text(status))
        return 0
    dash.start()
    if not a.no_prompts:
        dash.prompts = PromptIndex()
        secrets = [s for m in machines for s in m.secrets()]
        dash.prompt_mask = Masker(secrets, ()).__call__ if secrets else None
        threading.Thread(target=prompt_loop, args=(dash, dash.prompts), daemon=True).start()
    if not a.no_record:
        root = a.record or (repo / "test-output" / "build-log")
        threading.Thread(target=record_loop, args=(dash, root, a.record_interval), daemon=True).start()
    srv = ThreadingHTTPServer((a.host, a.port), make_handler(dash))
    print("OCW build dashboard on http://%s:%d/ (%d machines, Ctrl+C to stop)" % (a.host, a.port, len(machines)))
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        dash.stop.set()
        srv.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
