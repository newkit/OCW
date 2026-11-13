#!/usr/bin/env python3
"""Build server dashboard for Open Cycling World.

Polls the machines in config/machines.toml over SSH (read-only: one SSH call
per machine every few seconds) and shows their load, the build/test tasks
running on them and the queue of runs waiting for a build slot.

    python3 tools/build-dashboard/dashboard.py [--host 0.0.0.0] [--port 8765]
    python3 tools/build-dashboard/dashboard.py --once          # text, once

Standard library only. The server listens on all addresses by default
(--host 127.0.0.1 for local only). Its only write actions need the token of
config/dashboard-token: answering the spec's open questions, and enabling or
disabling build machines and setting their build slots
(config/build-machines.toml, see build_machines.py).
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
import shlex
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import answers
import build_machines
import janitor
import spec_questions

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

    def one(m):
        if m.group(1):
            return chr(int(m.group(1), 16))
        return table.get(m.group(2), m.group(0))
    return re.sub(r"\\(?:u([0-9A-Fa-f]{4})|(.))", one, s)


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
        n = g("slots")
        self.slots_configured = isinstance(n, int) and not isinstance(n, bool) and n >= 0

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
echo @@dfsrc; df -Pk @SRC@ 2>/dev/null | tail -n 1
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
echo @@dfsrc; df -Pk @SRC@ 2>/dev/null | tail -n 1
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


def _disk(name: str, role: str, total, used, free) -> dict:
    d = {"name": name, "role": role, "total": total, "used": used, "free": free}
    d["pct"] = janitor.disk_pct(d)  # like df's Use%: used / (used + free)
    return d


def parse_disks_posix(df: list, dfsrc: list) -> dict:
    """The disk watcher's disks (spec §11.2): the home volume and, when it
    is another file system, the one holding the checkouts (repo_path's
    directory). disk_pct = use of the checkouts' disk (the janitor's)."""
    disks, devs = [], []
    for name, role, lines in (("home", "home", df), ("src", "checkouts", dfsrc)):
        for l in lines:
            f = l.split()
            if len(f) >= 6 and f[1].isdigit():
                if f[0] in devs:  # same file system as home: one disk
                    disks[devs.index(f[0])]["role"] = "home, checkouts"
                else:
                    devs.append(f[0])
                    disks.append(_disk(name, role, int(f[1]) * 1024, int(f[2]) * 1024, int(f[3]) * 1024))
                break
    if len(disks) == 1:
        disks[0]["role"] = "home, checkouts"
    build = next((d for d in disks if "checkouts" in d["role"]), disks[0] if disks else None)
    return {"disks": disks, "disk_pct": build["pct"] if build else None}


def posix_script(script: str, repo_path: str) -> str:
    """A poll script with @SRC@ = the directory holding the checkouts."""
    parent = repo_path.rstrip("/").rsplit("/", 1)[0] if "/" in repo_path.rstrip("/") else "~"
    return script.replace("@SRC@", janitor._sh_path(parent))


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
    out.update(parse_disks_posix(s.get("df", []), s.get("dfsrc", [])))
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
    out.update(parse_disks_posix(s.get("df", []), s.get("dfsrc", [])))
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
    # Disk watcher: the drive of the checkouts and C: (user profile: cargo,
    # rustup, sccache until moved, scripts/windows-move-toolchain.ps1).
    watched = []
    for d in disks:
        letter = str(d.get("name", "")).upper()[:1]
        if letter == want or letter == "C":
            used, free = d.get("used"), d.get("free")
            total = (used or 0) + (free or 0)
            role = "checkouts" if letter == want else "system"
            if letter == want and want == "C":
                role = "checkouts, system"
            watched.append(_disk(letter + ":", role, total, used, free))
    watched.sort(key=lambda d: "checkouts" not in d["role"])
    out["disks"] = watched
    out["disk_pct"] = watched[0]["pct"] if watched else None
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


def poll_machine(m: Machine, ctl: str | None = None, slots: int | None = None) -> dict:
    """One poll of m (read-only; also of machines disabled for building).
    `slots`: the effective slot count (build-machines.toml), else m.slots."""
    t0 = time.time()
    slots = m.slots if slots is None else slots
    base = {"name": m.name, "os": m.os, "roles": m.roles, "slots": slots, "polled_at": t0}
    if not m.host:
        return dict(base, online=False, error="no host configured")
    masker = Masker(m.secrets())
    if m.os == "windows":
        cmd, parser = windows_command(), parse_windows
    elif m.os == "macos":
        cmd, parser = posix_script(MACOS_SCRIPT, m.repo_path), parse_macos
    else:
        cmd, parser = posix_script(LINUX_SCRIPT, m.repo_path), parse_linux
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
        data = parser(r.stdout, m.repo_path, slots)
    except Exception as e:  # a parser bug must not kill the poller
        return dict(base, online=False, error="unexpected output", detail=masker("%s: %s" % (type(e).__name__, e)))
    home = data.pop("home", None)
    masker = Masker(m.secrets(), [home] if home else [])
    data = masker.deep(data)
    return dict(base, online=True, poll_ms=int((time.time() - t0) * 1000), **data)


def run_ssh(m: Machine, script: str, timeout: float, stdin: str | None = None, ctl: str | None = None) -> str:
    """Runs a janitor snippet on a machine (Windows: PowerShell, encoded) and
    returns its stdout; raises RuntimeError with a masked reason."""
    cmd = windows_command(script) if m.os == "windows" else script
    try:
        r = subprocess.run(ssh_argv(m, ctl) + [cmd], capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=timeout, input=stdin if stdin is not None else "")
    except subprocess.TimeoutExpired:
        raise RuntimeError("timeout after %d s" % timeout)
    if r.returncode != 0 and "@@" not in r.stdout and "{" not in r.stdout:
        raise RuntimeError(classify_ssh_error(r.stderr, r.returncode))
    return r.stdout


def main_checkout(repo: Path) -> Path:
    """The main checkout of a git worktree (or repo itself)."""
    try:
        common = subprocess.run(["git", "-C", str(repo), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                capture_output=True, text=True, timeout=10).stdout.strip()
        if common:
            return Path(common).parent
    except Exception:
        pass
    return repo


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
        self.build_machines = None  # a build_machines.BuildMachines

    def settings(self, m: Machine) -> dict | None:
        bm = getattr(self, "build_machines", None)
        return bm.effective(m) if bm is not None else None

    def _loop(self, m: Machine):
        while not self.stop.is_set():
            t0 = time.time()
            s = self.settings(m)
            st = poll_machine(m, self.ctl, s["slots"] if s else None)
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
        apply_settings(machines, self.machines, getattr(self, "build_machines", None))
        status = summarize(machines, getattr(self, "questions", None), getattr(self, "janitor", None),
                           getattr(self, "answers", None))
        bm = getattr(self, "build_machines", None)
        # The page shows the enable/disable and slot controls only then.
        status["machine_settings"] = bool(bm is not None and not bm.read_only and getattr(self, "token", None))
        return annotate_jobs(status, getattr(self, "prompts", None), getattr(self, "commits", None),
                             getattr(self, "prompt_mask", None), getattr(self, "show_prompts", False))

    def states(self) -> list:
        with self.lock:
            return [dict(self.state[m.name]) for m in self.machines]


def apply_settings(states: list, machines: list, bm) -> list:
    """Adds to each machine state what build-machines.toml says: `enabled`,
    `disabled_reason`, the effective `slots` (and `slots_source`,
    `slots_config` = machines.toml / OS default), `changed_by`/`changed_at`."""
    by_name = {m.name: m for m in machines}
    for st in states:
        m = by_name.get(st.get("name"))
        if m is None:
            continue
        e = bm.effective(m) if bm is not None else {
            "enabled": True, "reason": "", "note": "", "slots": m.slots, "slots_source": "machines.toml"
            if getattr(m, "slots_configured", False) else "default", "slots_config": m.slots,
            "changed_by": "", "changed_at": ""}
        st.update(enabled=e["enabled"], disabled_reason=e["reason"] if not e["enabled"] else "",
                  slots=e["slots"], slots_source=e["slots_source"], slots_config=e["slots_config"],
                  note=e["note"], changed_by=e["changed_by"], changed_at=e["changed_at"])
    return states


def janitor_loop(dash, jan, test_output: Path | None, every: float = 30.0):
    """The janitor's thread: a pass when due (one machine, at most every
    10 min), and the size of test-output/ (reported only) every 10 min."""
    sized = 0.0
    while not dash.stop.wait(every):
        try:
            if test_output is not None and time.time() - sized >= janitor.PASS_INTERVAL_S:
                sized = time.time()
                jan.test_output_bytes = janitor.dir_bytes(test_output) if test_output.is_dir() else None
            res = jan.tick(dash.states())
            if res:
                print("janitor: pass %s" % json.dumps({k: res.get(k) for k in ("mode", "pct", "level", "freed", "error")}),
                      file=sys.stderr)
        except Exception as e:  # the janitor must never stop the dashboard
            print("janitor: %s: %s" % (type(e).__name__, e), file=sys.stderr)


def summarize(machines: list, questions=None, jan=None, store=None) -> dict:
    """The status: machines, overview, disk alerts, the janitor, and the open
    questions of SPECIFICATION.md (a SpecQuestions; re-read when it changed)
    with the answers given on the dashboard (an AnswerStore)."""
    status = _summarize(machines)
    alerts = janitor.disk_alerts(machines)
    status["alerts"] = alerts
    status["janitor"] = jan.status(machines) if jan is not None else {"mode": "off", "alerts": alerts}
    if questions is not None:
        q = questions.get()
        if store is not None:
            q = answers.annotate(q, store.latest())
            q["answers_enabled"] = True
        status["questions"] = q
        status["overview"]["questions_open"] = q.get("open", 0)
    return status


def _summarize(machines: list) -> dict:
    online = [m for m in machines if m.get("online")]
    tasks = sum(1 for m in online for t in (m.get("tasks") or []) if t.get("state") != "waiting")
    waiting = sum((m.get("queue") or {}).get("waiting", 0) for m in online)
    used = sum((m.get("queue") or {}).get("used", 0) for m in online)
    # Slots of machines disabled for building are not counted (no new runs).
    total = sum((m.get("queue") or {}).get("total", 0) if m.get("online") else (m.get("slots") or 0)
                for m in machines if m.get("enabled", True))
    disabled = sum(1 for m in machines if not m.get("enabled", True))
    return {"generated_at": time.time(), "machines": machines,
            "overview": {"machines": len(machines), "online": len(online), "tasks": tasks,
                         "waiting": waiting, "slots_used": used, "slots_total": total, "disabled": disabled}}


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
    for a in status.get("alerts") or []:
        lines.insert(0, "ALERT " + a["text"])
    for m in status["machines"]:
        setting = machine_setting_text(m)
        if not m.get("online"):
            det = (" - " + m["detail"]) if m.get("detail") else ""
            lines.append("%-14s %-7s UNREACHABLE (%s)%s" % (m["name"], m["os"], m.get("error"), det))
            if setting:
                lines.append("  " + setting)
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
        if m.get("disks"):
            for d in m["disks"]:
                parts.append("disk %s %s%% (%s free)" % (d["name"], "?" if d.get("pct") is None else "%.0f" % d["pct"],
                                                        fmt_bytes(d.get("free"))))
        elif m.get("disk_free") is not None:
            parts.append("disk %s free" % fmt_bytes(m["disk_free"]))
        if m.get("uptime"):
            parts.append("up %s" % fmt_dur(m["uptime"]))
        c = m.get("counters")
        if c:
            parts.append("processor queue %s" % (
                ("%.0f" % m["proc_queue"]) if c.get("ok") and m.get("proc_queue") is not None else c.get("hint")))
        lines.append("%-14s %-7s online  %s" % (m["name"], m["os"], ", ".join(parts)))
        if setting:
            lines.append("  " + setting)
        q = m.get("queue")
        if q:
            held = ", ".join("slot %d: %s" % (s["slot"], s["holder"] or "held") for s in q["slots"] if s["held"])
            lines.append("  slots: %d used of %d%s, %d waiting" % (
                q["used"], q["total"], (" (" + held + ")") if held else "", q["waiting"]))
            for w in q.get("queued") or []:
                lines.append("  queue: [%s] %s  (waiting %s)" % (
                    w["checkout"], w.get("command") or "(command unknown)", fmt_dur(w.get("elapsed"))))
                if w.get("description"):
                    lines.append("         %s" % w["description"])
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
            if g.get("description"):
                lines.append("      %s" % g["description"])
            for kd in g["kinds"]:
                lines.append("      %-16s x%-3d %8s %7s %10s" % (
                    kd["kind"], kd["count"], fmt_dur(kd["elapsed"]),
                    fmt_cpu(kd.get("cpu")) if kd.get("cpu") is not None else "", fmt_bytes(kd["rss"])))
        if m.get("stale_markers"):
            lines.append("  (%d stale task marker(s) of ended runs ignored)" % m["stale_markers"])
        jr = ((status.get("janitor") or {}).get("machines") or {}).get(m["name"])
        if jr:
            lines.append("  janitor: %s" % janitor_line(jr, status.get("generated_at")))
        lines.append("")
    lines += janitor_text(status.get("janitor") or {})
    lines += questions_text(status.get("questions"))
    return "\n".join(lines)


def machine_setting_text(m: dict) -> str:
    """"build: enabled, 2 slots (machines.toml)" or "build: DISABLED (reason) ..."."""
    if "enabled" not in m:
        return ""
    n = m.get("slots")
    slots = "no slot limit" if n == 0 else "%s slot%s" % (n, "" if n == 1 else "s")
    src = {"dashboard": "set in the dashboard", "machines.toml": "machines.toml", "default": "OS default"}.get(
        m.get("slots_source"), m.get("slots_source") or "")
    when = (", changed %s by %s" % (m["changed_at"], m.get("changed_by") or "?")) if m.get("changed_at") else ""
    if m.get("note"):
        when = " - note: %s%s" % (m["note"], when)
    if m["enabled"]:
        return "build: enabled, %s (%s)%s" % (slots, src, when)
    reason = (" (%s)" % m["disabled_reason"]) if m.get("disabled_reason") else ""
    return "build: DISABLED%s - remote.sh refuses runs; %s (%s)%s" % (reason, slots, src, when)


def janitor_line(r: dict, now=None) -> str:
    ago = (" %s ago" % fmt_dur(now - r["at"])) if now and r.get("at") else ""
    if r.get("error"):
        return "last pass%s failed (%s)" % (ago, r["error"])
    acts = r.get("actions") or []
    verb = "would delete" if r.get("mode") == "dry-run" else "deleted"
    if not acts:
        return "last pass%s: disk %s%%, nothing to do" % (ago, r.get("pct"))
    size = sum(a.get("bytes") or 0 for a in acts)
    return "last pass%s: disk %s%%, %s %d item(s), %s" % (ago, r.get("pct"), verb, len(acts), fmt_bytes(size))


def janitor_text(j: dict) -> list:
    out = ["Janitor: %s (target/ of merged or gone agent branches > %.0f %%, idle agent checkouts > %.0f %%, "
           "alert > %.0f %%)" % (j.get("mode", "off"), janitor.LEVEL_MERGED, janitor.LEVEL_LRU, janitor.LEVEL_ALERT)]
    if j.get("test_output_bytes") is not None:
        out.append("  test-output on this machine: %s (kept: the art project)" % fmt_bytes(j["test_output_bytes"]))
    for e in (j.get("recent") or [])[-5:]:
        out.append("  %s %s %s %s %s" % (e.get("time", "")[:16], e.get("machine"), e.get("action"),
                                         e.get("path", ""), fmt_bytes(e.get("bytes")) if e.get("bytes") else ""))
    return out + [""]


def questions_text(q) -> list:
    if not q:
        return []
    out = ["Open questions (%s): %d open, %d decided" % (q.get("source", "SPECIFICATION.md"), q.get("open", 0),
                                                         q.get("decided", 0))]
    if q.get("error"):
        out.append("  (%s)" % q["error"])
    for g in q.get("groups") or []:
        if not g["open"]:
            continue
        out.append("  %s – %s (%d open, %d decided)" % (g["group"], g["title"], g["open"], g["decided"]))
        for it in g["items"]:
            if it["status"] != "open":
                continue
            out.append("    Q%d %s%s" % (it["n"], it["title"],
                                         (" [%s]" % it["status_note"]) if it.get("status_note") else ""))
            out.append("        %s" % _shorten(it["text"], 120))
            if it.get("answer_state") == "answered":
                out.append("        ANSWERED %s (waiting for the coordinator to record it in the spec): %s" % (
                    it["answer"]["time"], _shorten(it["answer"]["answer"].replace("\n", " / "), 100)))
            if it.get("proposal"):
                out.append("        proposal: %s" % _shorten(it["proposal"], 110))
    for b in q.get("blocked") or []:
        out.append("  blocked: Phase %s %s waits for %s" % (b["phase"], b["milestone"], ", ".join(b["waits_for"])))
    return out + [""]


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
            elif path == "/api/answers":
                store = getattr(dash, "answers", None)
                if store is None:
                    return self._json(404, {"error": "answers are off"})
                q = parse_qs(urlsplit(self.path).query)
                try:
                    since = float((q.get("since") or ["0"])[0])
                except ValueError:
                    return self._json(400, {"error": "since: a Unix time in seconds"})
                self._json(200, {"answers": store.since(since), "now": time.time()})
            else:
                self._send(404, b"not found\n", "text/plain")

        def _json(self, code, obj):
            self._send(code, json.dumps(obj).encode(), "application/json")

        def _body(self):
            """The JSON object of the request (None: the error was sent)."""
            try:
                n = int(self.headers.get("Content-Length") or 0)
            except ValueError:
                n = -1
            if not 0 < n <= 64 * 1024:
                self._json(400, {"error": "body missing or too large"})
                return None
            try:
                body = json.loads(self.rfile.read(n).decode("utf-8"))
            except (ValueError, UnicodeDecodeError):
                self._json(400, {"error": "body is not JSON"})
                return None
            if not isinstance(body, dict):
                self._json(400, {"error": "body must be a JSON object"})
                return None
            return body

        def do_POST(self):  # noqa: N802
            path = self.path.split("?", 1)[0]
            if path == "/api/machines":
                return self._post_machine()
            if path != "/api/answers":
                return self._send(404, b"not found\n", "text/plain")
            self._post_answer()

        def _post_machine(self):
            """POST /api/machines {"machine", "enabled"?, "slots"?, "reason"?, "note"?,
            "by"?} with the token header: enables or disables a build machine
            and sets its build slots (config/build-machines.toml)."""
            bm, token = getattr(dash, "build_machines", None), getattr(dash, "token", None)
            if bm is None or bm.read_only or not token:
                return self._json(404, {"error": "machine settings are off"})
            if not answers.token_ok(token, self.headers.get(answers.TOKEN_HEADER)):
                return self._json(403, {"error": "wrong or missing token (config/dashboard-token)"})
            body = self._body()
            if body is None:
                return
            name = body.get("machine")
            # Only the names of machines.toml: never a path or anything else.
            m = next((x for x in dash.machines if isinstance(name, str) and x.name == name), None)
            if m is None:
                return self._json(400, {"error": "no such machine in machines.toml"})
            try:
                e = bm.update(m, body.get("enabled"), body.get("slots"), body.get("reason"), body.get("note"),
                              body.get("by") or "dashboard", "dashboard", [x.name for x in dash.machines])
            except ValueError as ex:
                return self._json(400, {"error": str(ex)[:200]})
            self._json(200, {"ok": True, "machine": m.name, "settings": e})

        def _post_answer(self):
            """POST /api/answers {"id", "answer", "choice"?} with the token
            header: stores an answer to an open question of the spec."""
            store, token = getattr(dash, "answers", None), getattr(dash, "token", None)
            if store is None or not token:
                return self._json(404, {"error": "answers are off"})
            if not answers.token_ok(token, self.headers.get(answers.TOKEN_HEADER)):
                return self._json(403, {"error": "wrong or missing token (config/dashboard-token)"})
            try:
                n = int(self.headers.get("Content-Length") or 0)
            except ValueError:
                n = -1
            if not 0 < n <= 64 * 1024:
                return self._json(400, {"error": "body missing or too large"})
            try:
                body = json.loads(self.rfile.read(n).decode("utf-8"))
                qid = str(body.get("id") or "")
                qs = dash.questions.get()
                question = next((it for g in qs.get("groups") or [] for it in g["items"] if it["id"] == qid), None)
                if question is None:
                    return self._json(400, {"error": "no question %s in the spec" % qid[:40]})
                e = store.add(question, body.get("answer"), body.get("choice"), qs.get("spec_version"))
            except (ValueError, AttributeError, UnicodeDecodeError) as ex:
                return self._json(400, {"error": str(ex)[:200]})
            self._json(200, {"ok": True, "answer": e})

        def log_message(self, fmt, *args):  # quiet: no request logs
            pass

    return Handler


# ---------------------------------------------------------------- prompts
#
# Which task a job belongs to: read from the Claude Code session transcripts
# of this project (~/.claude/projects/*OCW*/*.jsonl, main sessions, and the
# agents' subagents/agent-<id>.meta.json). A job in an agent worktree
# (checkout ".../worktree-agent-<id>", "OCW-worktree-agent-<id>" or
# "agent <id8>") gets the agent's task description, which goes into the job
# description (below). Only with --show-prompts the user prompt is added too:
# the one that preceded the Agent call that started agent <id>, or for any
# other job the latest user prompt before the job started.

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

    def feed_meta(self, name: str, meta: dict):
        """An agent's meta.json (subagents/agent-<id>.meta.json): its task
        description, for the worktree it runs in. An agent that only
        inherited its parent's worktree leaves the parent's description."""
        desc = str(meta.get("description") or "").strip()
        if not desc:
            return
        wt = meta.get("worktreePath")
        if wt:
            m = re.search(r"agent-([0-9a-f]{6,})$", str(wt).rstrip("/\\"))
            aid = m.group(1) if m else None
        elif meta.get("inheritedWorktreePath"):
            return
        else:
            m = re.match(r"agent-([0-9a-f]{6,})\.meta\.json$", name)
            aid = m.group(1) if m else None
        if aid:
            a = self.agents.setdefault(aid, {"prompt": None, "agent": desc})
            if not a.get("agent"):
                a["agent"] = desc

    def agent_for(self, checkout: str):
        """The task description of the agent whose checkout this is, or None."""
        m = _CHECKOUT_AGENT.search(checkout or "")
        if not m:
            return None
        with self.lock:
            hits = [v for k, v in self.agents.items() if k.startswith(m.group(1))]
        return (hits[0].get("agent") or None) if len(hits) == 1 else None

    def refresh(self):
        files = sorted(self.root.glob("*OCW*/*.jsonl")) if self.root.is_dir() else []
        metas = sorted(self.root.glob("*OCW*/*/subagents/agent-*.meta.json")) if self.root.is_dir() else []
        with self.lock:
            for f in metas:
                if f in self.offsets:
                    continue
                try:
                    self.feed_meta(f.name, json.loads(f.read_text(encoding="utf-8")))
                    self.offsets[f] = -1   # read once: the description does not change
                except (OSError, ValueError):
                    continue
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


def prompt_loop(dash, index, commits=None, interval: float = 15.0):
    while True:
        for src in (index, commits):
            if src is not None:
                try:
                    src.refresh()
                except Exception as e:  # context must never stop the dashboard
                    print("job context: %s" % e, file=sys.stderr)
        if dash.stop.wait(interval):
            return


# ---------------------------------------------------------------- job descriptions
#
# What a job is trying to achieve, in plain words, built deterministically
# from its command (rules below, no model calls) and its context: the task
# description of the agent whose checkout it runs in, or, for a check of the
# coordinator's own checkout, the commit (or merge) being checked. One short
# sentence ("<purpose> – <what> on <machine>", at most DESC_MAX characters)
# plus a longer breakdown for the tooltip.

DESC_MAX = 140
OS_NAMES = {"linux": "Linux", "macos": "macOS", "windows": "Windows"}
REGIONS = {"liechtenstein": "Liechtenstein", "pilot": "Liechtenstein", "li": "Liechtenstein",
           "blackforest": "Black Forest", "bf": "Black Forest", "schwarzwald": "Black Forest",
           "tenerife": "Tenerife", "tf": "Tenerife"}
_REGION_PARTS = {"north": "north", "middle": "middle", "south": "south"}
# Words that only shape output or prepare a step: never a job's purpose.
_IGNORED = {"echo", "printf", "uname", "ls", "cat", "grep", "egrep", "tail", "head", "sed", "awk", "sort",
            "uniq", "wc", "cut", "tr", "tee", "mkdir", "touch", "find", "cp", "mv", "install", "ln", "chmod",
            "sleep", "wait", "true", "false", "exit", "cd", "pwd", "date", "test", "[", "[[", "which",
            "command", "type", "du", "df", "stat", "file", "xargs", "diff", "cmp", "jq", "basename", "dirname",
            "realpath", "readlink", "mktemp", "rustup", "rustc", "nproc", "free", "ps", "kill", "pkill", "trap",
            "read", "local", "return", "source", ".", "unset", "shift", "git", "sha256sum", "md5sum", "zcat",
            "gzip", "tar", "unzip", "zip", "ffprobe", "identify", "convert", "{", "}", "then", "else",
            "elif", "fi", "do", "done", "for", "while", "if", "in", "case", "esac", "function"}
_PS_IGNORED = ("write-output", "write-host", "select-string", "select-object", "get-", "set-", "foreach",
               "measure-object", "out-", "where-object", "remove-item", "new-item", "test-path", "copy-item")
_CARGO_VALUE_FLAGS = {"-p", "--package", "--features", "-F", "--target", "--test", "--bin", "--example",
                      "-j", "--jobs", "--profile", "--message-format", "--manifest-path", "--color",
                      "--exclude", "--bench", "-Z", "--config", "--target-dir"}


def region_name(word):
    """Plain region name of a package / data / script word, or None."""
    w = (word or "").lower().strip("'\"")
    w = re.sub(r"\.(ocwpkg|route|sh?)$", "", w)
    w = re.sub(r"-(before|after|old|new|tmp)\d*$", "", w)
    if w in REGIONS:
        return REGIONS[w]
    head, _, tail = w.partition("-")
    if head in REGIONS and tail in _REGION_PARTS:
        return REGIONS[head] + " " + _REGION_PARTS[tail]
    return None


def command_regions(cmd: str) -> list:
    """Regions a command works on, in order of appearance (world builds,
    downloads, data directories, packages, `--world name=`)."""
    found = []
    pats = [r"\bbuild-([a-z][\w-]*)\.sh", r"\bfetch-(?:pilot|imagery)\.sh\s+([a-z][\w-]*)",
            r"\bdata/([a-z][\w-]*)", r"([a-z][\w-]*)\.ocwpkg", r"--world\s+([a-z][\w-]*)=",
            r"world-(?:clean-compare|profile)\.sh(?:\s+--\S+(?:\s+\S+)?)*\s+([a-z][\w-]*)"]
    hits = []
    for p in pats:
        hits += [(m.start(1), m.group(1)) for m in re.finditer(p, cmd or "")]
    if re.search(r"\bfetch-pilot\.sh(?!\s+[a-z])", cmd or ""):
        hits.append((cmd.find("fetch-pilot.sh"), "liechtenstein"))
    for _, w in sorted(hits):
        r = region_name(w)
        if r and r not in found:
            found.append(r)
    return found


def _and(items) -> str:
    items = [i for i in items if i]
    if len(items) <= 1:
        return "".join(items)
    return ", ".join(items[:-1]) + " and " + items[-1]


def _crates(names, n: int = 3) -> str:
    names = list(dict.fromkeys(names))
    return _and(names) if len(names) <= n else ", ".join(names[:n]) + " +%d more" % (len(names) - n)


def _shell_steps(cmd: str) -> list:
    """Top-level simple commands of a shell (or PowerShell) line: split at
    ; && || & newlines and subshell parentheses, outside quotes and $( );
    a pipeline keeps only its first command (the rest filters output)."""
    steps, cur, i, quote, depth, piped = [], [], 0, None, 0, False
    s = cmd or ""

    def flush():
        nonlocal cur, piped
        t = "".join(cur).strip()
        if t:
            steps.append(t)
        cur, piped = [], False

    while i < len(s):
        c = s[i]
        nxt = s[i + 1] if i + 1 < len(s) else ""
        if quote:
            if c == "\\" and quote == '"' and nxt:
                if not piped:
                    cur.append(c + nxt)
                i += 2
                continue
            if c == quote:
                quote = None
        elif depth:
            if c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
        elif c in "'\"":
            quote = c
        elif c == "$" and nxt == "(":
            depth = 1
            if not piped:
                cur.append("$(")
            i += 2
            continue
        elif c in ";\n" or c in "()" or (c == "&" and nxt == "&") or (c == "|" and nxt == "|"):
            flush()
            i += 2 if (c in "&|" and nxt == c) else 1
            continue
        elif c == "&" and (not cur or cur[-1][-1:] not in "<>") and nxt != ">":
            flush()
            i += 1
            continue
        elif c == "|":
            piped = True
            i += 1
            continue
        if not piped:
            cur.append(c)
        i += 1
    flush()
    return steps


def _words(step: str) -> list:
    try:
        toks = shlex.split(step, posix=True)
    except ValueError:  # cut-off marker command: unbalanced quotes
        toks = [t.strip("'\"") for t in step.split()]
    out, skip = [], False
    for t in toks:
        if skip:
            skip = False
            continue
        if re.match(r"^(\d*|&)[<>]{1,2}$", t):        # `>` `2>` `>>`: the target follows
            skip = True
            continue
        if re.match(r"^(\d*|&)[<>]{1,2}&?\S+$", t):   # `2>&1` `>/dev/null`
            continue
        out.append(t)
    return out


def _cargo(words: list) -> dict:
    """cargo invocation → {"sub", "crates", "workspace", "features", "target",
    "release", "flags", "filters", "after"}."""
    i, sub = 0, None
    info = {"sub": None, "crates": [], "workspace": False, "features": [], "target": None, "release": False,
            "flags": set(), "filters": [], "after": [], "tests": []}
    rest = words[1:]
    while i < len(rest):
        t = rest[i]
        if t == "--":
            info["after"] = rest[i + 1:]
            break
        if t.startswith("+") and sub is None:
            i += 1
            continue
        flag, eq, val = t.partition("=")
        if t.startswith("-"):
            if flag in _CARGO_VALUE_FLAGS and not eq:
                val = rest[i + 1] if i + 1 < len(rest) else ""
                i += 1
            if flag in ("-p", "--package"):
                if val:
                    info["crates"].append(val)
            elif flag in ("--features", "-F"):
                info["features"] += [f for f in re.split(r"[ ,]", val) if f]
            elif flag == "--target":
                info["target"] = val
            elif flag == "--test":
                info["tests"].append(val)
            elif flag in ("--workspace", "--all"):
                info["workspace"] = True
            elif flag == "--release" or (flag == "--profile" and val == "release"):
                info["release"] = True
            info["flags"].add(flag)
        elif sub is None:
            sub = t
        else:
            info["filters"].append(t)
        i += 1
    info["sub"] = sub
    return info


def _voice(info: dict, env: dict) -> bool:
    return any("voice" in f for f in info.get("features", [])) or "voice" in env.get("OCW_FEATURES", "")


def _scope_tests(info: dict) -> str:
    if info["sub"] == "test" and "--no-run" in info["flags"]:
        return "the test build of " + ("the workspace" if info["workspace"] or not info["crates"]
                                       else _crates(info["crates"]))
    name = None
    if info["tests"]:
        name = _and(info["tests"]) + (" tests" if len(info["tests"]) > 1 else " test")
    elif info["filters"]:
        name = _and(info["filters"][:2]) + " tests"
    if info["workspace"] or not info["crates"]:
        return ("the " + name) if name else "all tests"
    crates = _crates(info["crates"])
    if name:
        return "the %s of %s" % (name, crates)
    return ("the %s tests" % crates) if len(info["crates"]) == 1 else "the tests of " + crates


def _act(kind, text, detail, check=False, prep=False, region=None) -> dict:
    return {"kind": kind, "text": text, "detail": detail, "check": check, "prep": prep, "region": region}


def _classify_cargo(words, env, regions) -> dict | None:
    info = _cargo(words)
    sub = info["sub"] or ""
    crates = info["crates"]
    scope = "workspace" if info["workspace"] or not crates else ", ".join(crates)
    voice = " with voice" if _voice(info, env) else ""
    feats = (" (features " + ", ".join(info["features"]) + ")") if info["features"] else ""
    if sub == "fmt":
        chk = "--check" in info["flags"] or "--check" in info["after"]
        return _act("fmt", "formatting", "cargo fmt" + (" --check: is the code formatted?" if chk else
                                                          ": format the code"), check=True)
    if sub == "clippy":
        text = "clippy" + ("" if scope == "workspace" else " on " + _crates(crates)) + voice
        return _act("clippy", text, "clippy (warnings are errors) on the %s%s" % (scope, feats), check=True)
    if sub in ("test", "nextest") or (sub == "build" and "--tests" in info["flags"]):
        if sub == "build":
            info = dict(info, sub="test", flags=info["flags"] | {"--no-run"})
        text = _scope_tests(info) + voice
        det = "cargo test on the %s%s%s%s" % (scope, feats, " (release)" if info["release"] else "",
                                             (", filter " + " ".join(info["filters"])) if info["filters"] else "")
        return _act("test", text, det, check=True)
    if sub == "check":
        if info["target"] and "wasm32" in info["target"]:
            return _act("wasm", "the browser (wasm) build", "cargo check for %s of %s" % (info["target"], scope),
                        check=True)
        text = "the build of " + ("the workspace" if scope == "workspace" else _crates(crates)) + voice
        return _act("check", text, "cargo check of the %s%s" % (scope, feats), check=True)
    if sub == "build":
        if info["target"] and "wasm32" in info["target"]:
            return _act("wasm", "the browser (wasm) build", "cargo build for " + info["target"], check=True)
        kind = "Release build" if info["release"] else "Debug build"
        what = "the workspace" if scope == "workspace" else _crates(crates)
        return _act("build", "%s of %s%s" % (kind, what, voice), "cargo build%s of the %s%s" % (
            " --release" if info["release"] else "", scope, feats), prep=True)
    if sub == "clean":
        return _act("clean", "Removing build artefacts", "cargo clean " + scope, prep=True)
    if sub == "run":
        pkg = crates[0] if crates else (info["filters"][0] if info["filters"] else "")
        args = info["after"]
        if pkg == "ocw-assets":
            if args[:2] == ["materials", "fetch"]:
                return _act("materials", "Fetching the material library", "ocw-assets materials fetch " +
                            " ".join(args[2:]))
            return _act("assets", "Generating the assets" if args[:1] == ["generate"] else
                        "Running the asset tool", "ocw-assets " + " ".join(args))
        if pkg == "world-builder":
            return _classify_world_builder(["world-builder"] + args, regions)
        if pkg == "ocw-server":
            return _act("server", "Headless server run", "ocw-server " + " ".join(args))
        if pkg == "ocw-desktop":
            return _act("desktop", "Desktop app run", "ocw-desktop " + " ".join(args))
        if pkg == "ocw-bot":
            return _act("bot", "Bot riders run", "ocw-bot " + " ".join(args))
        if pkg == "ocw-devices":
            return _act("ble", "Bluetooth trainer run", "ocw-devices " + " ".join(info["filters"] + args))
        return _act("run", "Running " + (pkg or "a binary"), "cargo run " + " ".join(words[2:]))
    if sub:
        return _act("cargo", "cargo " + sub, "cargo " + " ".join(words[1:]))
    return None


def _classify_world_builder(words, regions) -> dict:
    sub = next((w for w in words[1:] if not w.startswith("-")), "")
    where = (" for " + _and(regions)) if regions else ""
    texts = {"package": "Building a world package", "graph": "Inspecting the road graph",
             "dem-import": "Importing elevation data", "lint": "Checking a world package"}
    text = texts.get(sub, "world-builder " + sub if sub else "world-builder")
    return _act("world-builder", text + where, "world-builder " + " ".join(words[1:])[:160], region=regions)


def _flag(words, name):
    for i, w in enumerate(words):
        if w == name and i + 1 < len(words):
            return words[i + 1]
        if w.startswith(name + "="):
            return w.split("=", 1)[1]
    return None


def _scene_names(only: str) -> str:
    names = [n for n in re.split(r"[ ,]+", only or "") if n]
    if not names:
        return "all scenes"
    if len(names) <= 2:
        return "the %s scene%s" % (_and(names), "s" if len(names) > 1 or "*" in names[0] else "")
    return "%d scenes (%s, …)" % (len(names), ", ".join(names[:2]))


_MAKE_TARGETS = {
    "check": [("fmt", "formatting"), ("clippy", "clippy"), ("test", "all tests"), ("wasm", "the browser (wasm) build")],
    "lint": [("fmt", "formatting"), ("clippy", "clippy")],
    "clippy": [("clippy", "clippy")], "fmt-check": [("fmt", "formatting")], "fmt": [("fmt", "formatting")],
    "test": [("test", "all tests")], "test-unit": [("test", "all tests")],
    "test-wasm": [("wasm", "the browser (wasm) build")],
}
_MAKE_OTHER = {
    "build": ("build", "Debug build of the workspace", True),
    "release": ("build", "Release build of ocw-desktop and ocw-server", False),
    "deploy-test": ("deploy", "Release build for the kiosk deployment", False),
    "test-kiosk": ("kiosk", "Testing the kiosk script", False),
    "test-screenshot": ("shot", "Screenshot smoke test on X11", False),
    "test-screenshot-wayland": ("shot", "Screenshot smoke test on Wayland", False),
    "test-smoke": ("shot", "Screenshot smoke tests on X11 and Wayland", False),
    "test-video": ("video", "Recording a test ride video on X11", False),
    "test-video-wayland": ("video", "Recording a test ride video on Wayland", False),
    "test-scenes": ("render", "Rendering all scenes", False),
    "test-multiplayer": ("multiplayer", "Multiplayer test: server, kiosk and bot riders over QUIC", False),
    "test-multiplayer-local": ("multiplayer", "Local multiplayer test: server and 3 bot riders", False),
    "materials": ("materials", "Fetching the material library", False),
    "macos-app": ("app", "Building the macOS app", False),
    "test-release-version": ("version", "Testing the release version bump", False),
    "version-next": ("version", "Next release version", True),
    "version": ("version", "Workspace version", True),
    "clean": ("clean", "Removing build artefacts", True),
    "art-gather": ("art", "Art project: gathering the day's media", False),
    "art-sheets": ("art", "Art project: contact sheets of the day", False),
    "art-render": ("art", "Art project: rendering the day videos", False),
    "art-diary": ("art", "Art project: joining the diary video", False),
    "art-publish": ("art", "Art project: publishing the videos", False),
    "server": ("server", "Headless server run", False),
}


def _classify_make(words) -> list:
    targets, i = [], 1
    while i < len(words):
        w = words[i]
        if w in ("-C", "-j", "-f") and i + 1 < len(words):
            i += 2
            continue
        if not w.startswith("-") and "=" not in w:
            targets.append(w)
        i += 1
    out = []
    for t in targets or ["all"]:
        if t in _MAKE_TARGETS:
            out += [_act(k, txt, "make " + t, check=True) for k, txt in _MAKE_TARGETS[t]]
        elif t in _MAKE_OTHER:
            k, txt, prep = _MAKE_OTHER[t]
            out.append(_act(k, txt, "make " + t, prep=prep))
        elif t.startswith("art-"):
            out.append(_act("art", "Art project: " + t[4:], "make " + t))
        else:
            out.append(_act("make", "make " + t, "make " + t))
    return out


def _classify_script(name, words, env, regions, cmd) -> list:
    """Repository scripts (scripts/*.sh, the run-ocw skill, art/diary.py)."""
    args = words[1:]
    base = re.sub(r"\.(sh?|py?)$", "", name)
    if base.startswith("build-") and base not in ("build-appimage", "build-macos-app", "build-id", "build-profile"):
        r = region_name(base[6:]) or base[6:].replace("-", " ").title()
        return [_act("world", "Building the %s world package" % r, name + " " + " ".join(args), region=[r])]
    if base == "build-profile":
        label = _flag(args, "--label") or ""
        steps = _flag(args, "--steps") or ""
        cmds = _flag(args, "--commands") or ""
        what = "a clean build" if "clean" in steps.split(",") else "build times"
        if "sccache" in label or "sccache" in cmd:
            what += " with a %s sccache" % ("cold" if "cold" in label else "warm" if "warm" in label else "shared")
        extra = ", ".join(x for x in (label, cmds.replace(",", ", ")) if x)
        return [_act("profile", "Measuring %s (build profiling%s)" % (what, (": " + extra) if extra else ""),
                     "build-profile.sh " + " ".join(args))]
    if base == "world-profile":
        named = [region_name(a) for i, a in enumerate(args)
                 if not a.startswith("-") and not (i and args[i - 1].startswith("--"))]
        named = [r for r in dict.fromkeys(named) if r] or regions
        where = (" of " + _and(named)) if named else ""
        return [_act("profile", "Measuring world build times" + where, name + " " + " ".join(args))]
    if base == "world-clean-compare":
        where = (" (" + _and(regions) + ")") if regions else ""
        return [_act("compare", "Comparing world packages before and after the cleanup" + where,
                     name + " " + " ".join(args), region=regions)]
    if base == "fetch-pilot":
        r = region_name(args[0]) if args and not args[0].startswith("-") else "Liechtenstein"
        r = r or args[0]
        return [_act("fetch", "Downloading the %s source data (OSM, elevation)" % r, name + " " + " ".join(args),
                     region=[r])]
    if base == "fetch-imagery":
        r = (region_name(args[0]) if args else None) or (args[0] if args else "")
        return [_act("fetch", ("Downloading %s aerial imagery" % r).replace("  ", " "), name + " " + " ".join(args),
                     region=[r] if r else None)]
    if base in ("fetch-materials", "install-materials"):
        return [_act("materials", "Fetching the material library", name)]
    if base in ("render-scenes", "render_scenes"):
        if "--list" in args:
            return [_act("render", "Listing the scenes", name + " --list", prep=True)]
        only = _flag(args, "--only")
        scenes = _flag(args, "--scenes")
        if scenes and not only:
            stem = re.sub(r"^scenes-|-tmp$|\.toml$", "", Path(scenes).name.replace(".toml", ""))
            what = "the %s scenes" % stem.replace("-", " ") if stem and stem != "scenes" else "all scenes"
        else:
            what = _scene_names(only)
        worlds = [region_name(re.sub(r"=.*", "", w)) or region_name(Path(w.split("=")[-1]).name)
                  for i, w in enumerate(args) if i and args[i - 1] == "--world"]
        worlds = [w for w in dict.fromkeys(worlds) if w]
        where = (" in " + _and(worlds)) if worlds else ""
        return [_act("render", "Rendering %s%s" % (what, where), name + " " + " ".join(args),
                     region=worlds or None)]
    if base == "drive":
        mode = args[0] if args else ""
        ws = "Wayland" if "wayland" in mode else "X11"
        if mode.startswith("video"):
            return [_act("video", "Recording a test ride video on " + ws, "run-ocw drive.sh " + " ".join(args))]
        if mode.startswith("shot"):
            return [_act("shot", "Screenshot run on " + ws, "run-ocw drive.sh " + " ".join(args))]
        return [_act("drive", "Running the desktop app headlessly", "run-ocw drive.sh " + " ".join(args))]
    if base == "measure":
        label = _flag(args, "--label")
        return [_act("measure", "Measuring performance" + (": " + label if label else ""),
                     name + " " + " ".join(args))]
    if base == "deploy-test":
        return [_act("deploy", "Release build for the kiosk deployment", name + " " + " ".join(args))]
    if base == "test-multiplayer":
        return [_act("multiplayer", "Multiplayer test: server, kiosk and bot riders over QUIC", name)]
    if base == "test-kiosk":
        return [_act("kiosk", "Testing the kiosk script", name)]
    if base == "test-release-version":
        return [_act("version", "Testing the release version bump", name)]
    if base == "collect-test-output":
        return [_act("collect", "Collecting the test output into the build history", name)]
    if base == "build-appimage":
        return [_act("app", "Building the Linux AppImage" + (" with voice" if "voice" in cmd else ""), name)]
    if base == "build-macos-app":
        return [_act("app", "Building the macOS app" + (" with voice" if "voice" in cmd else ""), name)]
    if base == "prepare-world-release":
        return [_act("world", "Preparing a world release" + ((" of " + _and(regions)) if regions else ""), name)]
    if base == "demo-gpx-route":
        return [_act("route", "Building the demo GPX route", name)]
    if base == "diary":
        sub = args[0] if args else ""
        return [_act("art", "Art project: " + (sub or "diary"), "art/diary.py " + " ".join(args))]
    if base == "repro":
        return [_act("repro", "Reproduction run" + ((" (" + args[0] + ")") if args else ""),
                     name + " " + " ".join(args))]
    return [_act("script", "Running " + name, name + " " + " ".join(args))]


def _classify_python(words) -> list:
    args = [w for w in words[1:] if w not in ("-I", "-u", "-B")]
    joined = " ".join(args)
    if "build-dashboard" in joined:
        return [_act("dashboard", "Build dashboard tests" if ("unittest" in joined or "test_" in joined)
                     else "Build dashboard", "python3 " + joined)]
    if args[:2] == ["-m", "unittest"]:
        d = _flag(args, "-s")
        return [_act("pytest", "Python tests" + ((" of " + d) if d else ""), "python3 " + joined)]
    if args and args[0].endswith(".py"):
        return _classify_script(Path(args[0]).name, args, {}, command_regions(joined), joined)
    return [_act("script", "Python", "python3 " + joined)]


def _rm_checkouts(words) -> dict | None:
    paths = [w for w in words[1:] if not w.startswith("-")]
    checkouts = [p for p in paths if re.search(r"OCW-(?:worktree-)?[\w.-]+/?$", p)]
    if checkouts:
        n = len(checkouts)
        return _act("cleanup", "Cleaning up %s old checkout%s" % (n if n > 1 else "an", "s" if n > 1 else ""),
                    "rm -rf " + " ".join(checkouts))
    if any(re.search(r"(^|/)target/?$", p) for p in paths):
        return _act("clean", "Removing build artefacts", "rm -rf " + " ".join(paths), prep=True)
    return None


def classify_step(step: str, env: dict, regions: list, cmd: str, cut: bool = False) -> list:
    """Activities of one simple command; cut: its last word is partial."""
    words = _words(step)
    if cut and words and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", words[-1]):
        words = words[:-1]
    while words:
        w = words[0]
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", w):    # VAR=value prefix
            k, _, v = w.partition("=")
            env[k] = v
            words = words[1:]
        elif w in ("export", "env", "time", "nice", "nohup", "exec", "builtin", "set", "do", "then", "else",
                   "{", "!", "sudo"):
            words = words[1:]
            if w == "set":
                words = [x for x in words if not x.startswith(("-", "+"))]
        elif w.lower() in ("cmd", "cmd.exe") and len(words) > 2 and words[1].lower() == "/c":
            inner = " ".join(words[2:])
            out = []
            for s in _shell_steps(inner):
                out += classify_step(s, env, regions, cmd, cut)
            return out
        else:
            break
    if cut and len(words) > 1:
        last = words[-1].rsplit(",", 1)[0] if "," in words[-1] else None  # a cut list keeps its whole items
        words = words[:-1] + ([last] if last else [])
        cut = False
    if not words:
        return []
    w0 = words[0]
    base = re.split(r"[/\\]", w0)[-1]
    low = base.lower()
    if low.endswith(".exe"):
        low = low[:-4]
    if low == "cargo":
        a = _classify_cargo(words, env, regions)
        return [a] if a else []
    if low == "make":
        return _classify_make(words)
    if low == "world-builder":
        return [_classify_world_builder(words, regions)]
    if low in ("ocw-server", "ocw-desktop", "ocw-bot"):
        return [_classify_cargo(["cargo", "run", "-p", low, "--"] + words[1:], env, regions)]
    if low == "sccache":
        return [_act("sccache", "sccache statistics", "sccache " + " ".join(words[1:]), prep=True)]
    if low in ("python", "python3"):
        return _classify_python(words)
    if low in ("bash", "sh", "zsh") and len(words) > 1:
        return classify_step(" ".join(shlex.quote(x) for x in words[1:]), env, regions, cmd, cut)
    if low == "rm":
        a = _rm_checkouts(words)
        return [a] if a else []
    if low.endswith((".sh", ".py")) or (cut and "scripts/" in w0) or "/run-ocw/" in w0:
        return _classify_script(base, words, env, regions, cmd)
    if (low in _IGNORED or low.startswith(_PS_IGNORED) or w0.startswith(("$", "-", "+", "#")) or " " in w0
            or ")" in w0):
        return []
    if re.match(r"^[A-Za-z_]\w*\(\)$", w0):  # shell function definition
        return []
    return [_act("other", "Running " + base, " ".join(words)[:120])]


def _deploy_prelude(cmd: str):
    """scripts/deploy-test.sh sends a script, not a command: recognise it."""
    m = re.search(r"\bD=(\S+)", cmd or "")
    if m and "put()" in cmd:
        if m.group(1).startswith("/var/tmp/ocw"):
            return _act("deploy", "Release build for the kiosk deployment", "deploy-test.sh: build, then install "
                        "to the kiosk directory (the kiosk restarts the app)")
        return _act("deploy", "Release build for a test deployment", "deploy-test.sh into a scratch directory")
    return None


def classify_command(cmd: str) -> list:
    """The activities of a command line, in order (see _act)."""
    cmd = cmd or ""
    cut = cmd.rstrip().endswith(("...", "…"))
    if cut:
        # remote.sh cuts marker commands at 200 characters (the process view
        # at 90): the last word is partial. A cut argument is dropped, a cut
        # command name kept (scripts/build-blackforest.s still says enough).
        cmd = re.sub(r"(?:\.\.\.|…)\s*$", "", cmd.rstrip())
    regions = command_regions(cmd)
    pre = _deploy_prelude(cmd)
    if pre:
        return [pre]
    env, acts = {}, []
    steps = _shell_steps(cmd)
    for i, step in enumerate(steps):
        last = cut and i == len(steps) - 1
        got = classify_step(step, env, regions, cmd, cut=last)
        if last and len(_words(step)) == 1:
            # Only a cut command name: keep it when it still names something known.
            got = [a for a in got if a["kind"] not in ("script", "other")]
        acts += got
    for a in acts:
        if a["kind"] == "world-builder" and not a.get("region") and regions:
            a["region"] = regions
    return acts


def _cap(s: str) -> str:
    return s[:1].upper() + s[1:] if s else s


def _low(s: str) -> str:
    # lower-case a leading verb/noun, never a name ("Black Forest", "Tenerife").
    if not s or (len(s) > 1 and s[1].isupper()):
        return s
    first = s.split(" ", 1)[0]
    if region_name(first) or first in ("Linux", "Windows", "macOS", "Bluetooth"):
        return s
    return s[:1].lower() + s[1:]


def describe_activities(acts: list, cmd: str = "") -> tuple:
    """(what, check?) for a list of activities: one phrase, sentence case."""
    if not acts:
        if re.match(r"^\s*echo\b", cmd or ""):
            return "Connection test", False
        return "", False
    real = [a for a in acts if not a["prep"]] or acts
    checks = [a for a in real if a["check"]]
    others = [a for a in real if not a["check"]]
    parts = []
    if checks:
        texts = list(dict.fromkeys(a["text"] for a in checks))
        if texts == ["the browser (wasm) build"]:
            parts.append("Browser (wasm) build check")
        else:
            parts.append("Checking " + _and(texts))
    seen = set()
    for a in others:
        if a["text"] not in seen:
            seen.add(a["text"])
            parts.append(a["text"])
    # Several world builds: one phrase.
    worlds = [p for p in parts if re.match(r"^Building the (.+) world package$", p)]
    if len(worlds) > 1:
        names = [re.match(r"^Building the (.+) world package$", p).group(1) for p in worlds]
        i = parts.index(worlds[0])
        parts = [p for p in parts if p not in worlds]
        parts.insert(i, "Building the %s world packages" % _and(names))
    what = parts[0] + "".join("; " + _low(p) for p in parts[1:])
    return what, bool(checks) and not others


def machine_name(name: str, os_name: str = "") -> str:
    """The machine's config name; the OS name when the config names it after its OS."""
    if name and os_name and name.lower() == os_name.lower():
        return OS_NAMES.get(os_name.lower(), name)
    return name or OS_NAMES.get((os_name or "").lower(), os_name or "")


def _clip(s: str, n: int) -> str:
    s = " ".join((s or "").split())
    return s if len(s) <= n else s[: n - 1].rstrip() + "…"


def describe_job(command, machine: str = "", os_name: str = "", agent=None, commit=None) -> dict:
    """{"description": short sentence, "description_detail": tooltip} for a
    job. agent: the agent's task description (its checkout); commit: the
    subject of the commit or merge being checked (coordinator's checkout,
    used for checks, builds for deployment and release apps only)."""
    cmd = command or ""
    acts = classify_command(cmd) if cmd else []
    what, _ = describe_activities(acts, cmd)
    code_job = any(a["check"] or a["kind"] in ("deploy", "app") for a in acts if not a["prep"])
    purpose = agent or (commit if code_job else None)
    where = machine_name(machine, os_name)
    tail = (" on " + where) if where else ""
    if not what:
        what = "Running a command"
    if purpose:
        body = _low(what) + tail
        room = DESC_MAX - len(body) - 3
        desc = (_clip(purpose, max(room, 30)) + " – " + body) if room >= 20 else what + tail
    else:
        desc = _cap(what) + tail
    detail = []
    if agent:
        detail.append("Agent task: " + agent)
    elif purpose:
        detail.append("Checking the commit: " + commit)
    detail += ["• " + a["detail"] for a in acts]
    if where:
        detail.append("Machine: %s (%s)" % (machine, OS_NAMES.get((os_name or "").lower(), os_name or "?")))
    if cmd:
        detail.append("Command: " + cmd)
    return {"description": _clip(desc, DESC_MAX), "description_detail": "\n".join(detail)}


class CommitLog:
    """Subjects of the recent commits of the coordinator's checkout (the main
    checkout this dashboard belongs to; remote.sh copies its working tree to
    the machines' main checkout)."""

    def __init__(self, repo: Path | None = None, n: int = 100):
        self.repo, self.n = repo, n
        self.commits = []   # (epoch, subject), newest first
        self.lock = threading.Lock()

    def refresh(self):
        if self.repo is None:
            return
        try:
            common = subprocess.run(["git", "-C", str(self.repo), "rev-parse", "--path-format=absolute",
                                     "--git-common-dir"], capture_output=True, text=True, timeout=10).stdout.strip()
            main = str(Path(common).parent) if common else str(self.repo)
            out = subprocess.run(["git", "-C", main, "log", "-n", str(self.n), "--format=%ct%x1f%s"],
                                 capture_output=True, text=True, timeout=10).stdout
        except Exception:
            return
        commits = []
        for line in out.splitlines():
            ts, _, subj = line.partition("\x1f")
            if ts.isdigit():
                commits.append((int(ts), subj))
        with self.lock:
            self.commits = commits

    def at(self, started=None, slack: float = 120):
        """Subject of the newest commit made before the job started."""
        with self.lock:
            for ts, subj in self.commits:
                if started is None or ts <= float(started) + slack:
                    return subj
        return None


def annotate_jobs(status: dict, index=None, commits=None, mask=None, show_prompts: bool = False) -> dict:
    """Adds "description" and "description_detail" (what the job is trying
    to achieve) to every task and queued run of the status, and with
    show_prompts the prompt it belongs to ("prompt", "agent")."""
    for m in status.get("machines") or []:
        jobs = list(m.get("tasks") or []) + list((m.get("queue") or {}).get("queued") or [])
        for t in jobs:
            if not isinstance(t, dict) or "checkout" not in t:
                continue
            started = t.get("started") or t.get("since")
            checkout = t.get("checkout") or ""
            agent = index.agent_for(checkout) if index is not None else None
            commit = None
            if commits is not None and not _CHECKOUT_AGENT.search(checkout) and checkout == "main":
                commit = commits.at(started)
            # A process group without a command (no marker) has nothing to describe.
            info = describe_job(t.get("command"), m.get("name", ""), m.get("os", ""), agent, commit) \
                if t.get("command") else {}
            if show_prompts and index is not None:
                info.update(index.for_job(checkout, started))
            for k, v in info.items():
                t[k] = mask(v) if mask else v
    return status


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
            st = dash.snapshot()
            q = st.get("questions")
            if q:  # counts only: the questions themselves are in the spec
                st["questions"] = {k: q.get(k) for k in ("open", "decided", "spec_version")}
            with open(p, "a", encoding="utf-8") as f:
                f.write(record_line(st) + "\n")
        except Exception as e:  # recording must never stop the dashboard
            print("build log: %s" % e, file=sys.stderr)


def janitor_plan_text(jan, machines: list) -> str:
    """--janitor-plan: for every reachable machine the disks, every checkout
    with a target/ and what each threshold would delete (nothing is
    deleted; the main checkout's sweep runs as a dry run)."""
    now = time.time()

    def one(m):
        try:
            inv = jan.inventory(m)
            verdicts = janitor.git_verdicts(jan.repo, [c["branch"] for c in inv["checkouts"] if c["kind"] == "agent"])
            main = next((c for c in inv["checkouts"] if c["kind"] == "main"), None)
            sweep = jan.sweep(m, False) if main else None
            return m, inv, verdicts, sweep, None
        except Exception as e:
            return m, None, None, None, "%s: %s" % (type(e).__name__, e)

    with ThreadPoolExecutor(max_workers=max(1, len(machines))) as ex:
        results = list(ex.map(one, machines))
    out = ["Janitor dry run (%s; nothing deleted)" % time.strftime("%Y-%m-%d %H:%M"), ""]
    gb = lambda n: "%.1f GB" % ((n or 0) / 1e9)  # noqa: E731
    for m, inv, verdicts, sweep, err in results:
        if err:
            out += ["== %s: unreachable, skipped (%s)" % (m.name, err), ""]
            continue
        p = janitor.plan_machine(inv["checkouts"], inv["disk"], now, verdicts, sweep)
        disks = ", ".join("%s %s%% used, %s free" % (d["name"], janitor.disk_pct(d), gb(d.get("free")))
                          for d in inv["disks"])
        out.append("== %s: %s; level now: %s" % (m.name, disks, ("> %d %%" % p["level"]) if p["level"] else "below 75 %"))
        c = p["candidates"]
        tot = lambda xs: sum(x.get("target_bytes") or 0 for x in xs)  # noqa: E731
        out.append("  at > 75 %%: target/ of %d merged or gone agent checkouts, %s" % (len(c["merged"]), gb(tot(c["merged"]))))
        for x in c["merged"]:
            out.append("    %9s  %s/target  (%s)" % (gb(x.get("target_bytes")), x["name"], x["reason"]))
        if sweep:
            out.append("  at > 75 %%: main checkout sweep: %d old build units + %d incremental dirs, %s" % (
                sweep.get("units") or 0, sweep.get("incremental") or 0, gb(sweep.get("bytes"))))
        out.append("  at > 85 %%: also least recently used agent checkouts idle > 24 h, oldest first, until < 85 %%: "
                   "%d candidates, %s" % (len(c["lru"]), gb(tot(c["lru"]))))
        for x in c["lru"]:
            out.append("    %9s  %s/target  (%s)" % (gb(x.get("target_bytes")), x["name"], x["reason"]))
        out.append("  kept: %d checkouts, %s" % (len(c["kept"]), gb(tot(c["kept"]))))
        for x in c["kept"]:
            out.append("    %9s  %s/target  (%s)" % (gb(x.get("target_bytes")), x["name"], x["reason"]))
        if p["actions"]:
            out.append("  NOW (disk at %s%%) the janitor would delete: %s" % (
                p["pct"], ", ".join("%s (%s)" % (a["path"], gb(a["bytes"])) for a in p["actions"])))
        out.append("")
    return "\n".join(out)


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
    ap.add_argument("--no-context", action="store_true",
                    help="describe jobs from their command only (no agent tasks, no commits)")
    ap.add_argument("--show-prompts", action="store_true",
                    help="also put the user prompt each job belongs to into the JSON (off by default)")
    ap.add_argument("--no-prompts", action="store_true", help=argparse.SUPPRESS)  # old flag; now the default
    ap.add_argument("--record-interval", type=float, default=RECORD_INTERVAL_S, help="seconds between build-log lines")
    ap.add_argument("--janitor", choices=janitor.MODES, default="on",
                    help="disk janitor: on (default), dry-run (log what it would delete) or off")
    ap.add_argument("--janitor-plan", action="store_true",
                    help="dry run of the janitor on every reachable machine now (what each threshold would delete), then exit")
    ap.add_argument("--no-answers", action="store_true",
                    help="no answering of the open questions on the page (read-only)")
    ap.add_argument("--no-machine-settings", action="store_true",
                    help="no enabling/disabling of build machines and slot changes on the page")
    ap.add_argument("--spec", type=Path, default=None,
                    help="spec with the open questions (default: SPECIFICATION.md of the main checkout)")
    a = ap.parse_args(argv)

    repo = HERE.parent.parent
    cfg = a.config or find_config_dir(repo)
    if not (cfg / "machines.toml").is_file():
        print("%s/machines.toml missing (copy config/machines.example.toml)" % cfg, file=sys.stderr)
        return 1
    machines = load_machines(cfg)
    dash = Dashboard(machines, a.interval, not a.no_control_master)
    main_repo = main_checkout(repo)
    dash.questions = spec_questions.SpecQuestions(a.spec or (main_repo / "SPECIFICATION.md"))
    # Answers to the open questions: test-output/answers/ of the main
    # checkout; writing needs the token in config/dashboard-token (0600).
    dash.answers = None if a.no_answers else answers.AnswerStore(main_repo / "test-output" / "answers")
    log_root = None if a.no_record else (a.record or (repo / "test-output" / "build-log"))
    # Build machines enabled/disabled and their slots: config/build-machines.toml
    # (read by scripts/remote.sh); changes logged to build-log/DAY/machines.jsonl.
    dash.build_machines = build_machines.BuildMachines(
        cfg / build_machines.FILE_NAME, a.record or (main_repo / "test-output" / "build-log"))
    ssh = lambda m, script, timeout, stdin=None: run_ssh(m, script, timeout, stdin, dash.ctl)  # noqa: E731
    dash.janitor = janitor.Janitor(machines, main_repo, log_root, a.janitor, ssh)
    if a.janitor_plan:
        print(janitor_plan_text(dash.janitor, machines))
        return 0
    if a.once:
        with ThreadPoolExecutor(max_workers=max(1, len(machines))) as ex:
            states = list(ex.map(lambda m: poll_machine(m, dash.ctl, dash.settings(m)["slots"]), machines))
        apply_settings(states, machines, dash.build_machines)
        status = annotate_jobs(summarize(states, dash.questions, None, dash.answers), mask=Masker([s for m in machines for s in m.secrets()], ()))
        status["janitor"]["mode"] = a.janitor
        print(json.dumps(status, indent=2) if a.json else format_text(status))
        return 0
    if a.no_machine_settings:
        dash.build_machines.read_only = True
    if dash.answers is not None or not a.no_machine_settings:
        dash.token = answers.load_token(cfg / "dashboard-token")
        print("writing (answers, machine settings): token in %s (enter it once on the page)"
              % (cfg / "dashboard-token").name)
    dash.start()
    # Also with --janitor off: the thread then only measures test-output/.
    threading.Thread(target=janitor_loop, args=(dash, dash.janitor, repo / "test-output"), daemon=True,
                     name="janitor").start()
    dash.prompt_mask = Masker([s for m in machines for s in m.secrets()], ())
    dash.show_prompts = a.show_prompts and not a.no_prompts
    if not a.no_context:
        dash.prompts = PromptIndex()
        dash.commits = CommitLog(repo)
        threading.Thread(target=prompt_loop, args=(dash, dash.prompts, dash.commits), daemon=True).start()
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
