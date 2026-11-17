"""Battery of the build machines (laptops) and of a Meta Quest on Link.

Every machine is asked every ~2 min, in its own thread and with a short
timeout, for its battery (read-only, standard library only):

- Windows: kernel32 `GetSystemPowerStatus` (percent, mains, charging) and
  powrprof `CallNtPowerInformation(SystemBatteryState)` (capacity, charge
  or discharge rate in mW, estimated time) – both bound in memory like the
  main poll's Win32 calls; no admin rights, no CIM (denied over SSH), no GUI.
- Linux: `/sys/class/power_supply` (system batteries only: `scope=Device`
  batteries of mice and keyboards are left out).
- macOS: `pmset -g batt`.

On a Windows machine with the Meta Quest runtime (`%LOCALAPPDATA%\\Oculus`)
it also reads the headset: the runtime's `DeviceCache.json` (the headset's
last battery level and connection state) and the last 1 MB of the newest
`Service_*.txt` log (opened shared, read-only), whose `[RemoteHeadset] HMD
battery level update: from A to B` lines give the level while the headset
is on Link (the direction gives charging / discharging) and whose Link and
USB lines give the connection. Nothing is started on the machine (no
OculusDebugTool, no adb, no OpenXR session): the VR app and the kiosk are
never disturbed. Whether the headset is on the face is not in these files.

Only derived values leave the parsers (never raw log lines: they can hold
device serials).
"""

from __future__ import annotations

import base64
import json
import re
import struct
import threading
import time
from collections import deque
from datetime import datetime, timedelta

POLL_S = 120.0  # a machine with a battery (or a headset)
NO_BATTERY_POLL_S = 600.0  # a machine without one: asked again now and then
TIMEOUT_S = 25.0  # one probe, ssh included
STALE_S = 3.5 * POLL_S  # no successful probe for that long: "stale"
HISTORY_S = 6 * 3600.0  # kept in memory
PAGE_SPAN_S = 4 * 3600.0  # the page's sparkline
PAGE_POINTS = 48  # 5-min buckets
BAD_PCT = 20.0  # red below
WARN_PCT = 40.0  # amber below
QUEST_REPORT_STALE_S = 30 * 60.0  # a headset level older than that is "last report"

# --------------------------------------------------------------------------
# Remote snippets (read-only)
# --------------------------------------------------------------------------

POSIX_SCRIPT = r"""
export LC_ALL=C
echo @@now; date +%s
if [ -d /sys/class/power_supply ]; then
  echo @@sysfs
  for d in /sys/class/power_supply/*; do
    [ -d "$d" ] || continue
    n=${d##*/}
    for f in type scope status capacity online energy_now energy_full power_now charge_now charge_full current_now; do
      [ -r "$d/$f" ] && printf '%s %s %s\n' "$n" "$f" "$(cat "$d/$f" 2>/dev/null)"
    done
  done
fi
if command -v pmset >/dev/null 2>&1; then echo @@pmset; pmset -g batt 2>/dev/null; fi
echo @@end
"""

QUEST_LOG_RX = (r"HMD battery level update|Battery level for ovrControllerType|winusb detector\] device with"
                r"|Connected participant on transport winusb|HMD Startup time first displayed frame"
                r"|Streaming session got invited|RemoteHeadsetPlugin::didDisconnect|Received request to stop Link")

WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
$r = [ordered]@{}
$r.now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$r.local = [DateTime]::Now.ToString('yyyy-MM-ddTHH:mm:ss')
try {
  $tb = [AppDomain]::CurrentDomain.DefineDynamicAssembly((New-Object Reflection.AssemblyName 'OcwPower'), 'Run').DefineDynamicModule('OcwPower').DefineType('OcwPower', 'Public,Abstract,Sealed')
  foreach ($d in @(@('kernel32.dll', 'GetSystemPowerStatus', [bool], @([byte[]])),
                   @('powrprof.dll', 'CallNtPowerInformation', [uint32], @([int], [IntPtr], [uint32], [byte[]], [uint32])))) {
    $m = $tb.DefinePInvokeMethod($d[1], $d[0], 'Public,Static,PinvokeImpl', 'Standard', $d[2], [Type[]]$d[3], 'Winapi', 'Unicode')
    $m.SetImplementationFlags('PreserveSig')
  }
  $W = $tb.CreateType()
  $sp = New-Object byte[] 12
  if ($W::GetSystemPowerStatus($sp)) { $r.sps = [Convert]::ToBase64String($sp) }
  $sb = New-Object byte[] 32
  $r.nt = $W::CallNtPowerInformation(5, [IntPtr]::Zero, [uint32]0, $sb, [uint32]32)
  if ($r.nt -eq 0) { $r.sbs = [Convert]::ToBase64String($sb) }
} catch { $r.power_error = $_.Exception.GetType().Name }
$od = Join-Path $env:LOCALAPPDATA 'Oculus'
$dc = Join-Path $od 'DeviceCache.json'
if (Test-Path -LiteralPath $dc) {
  $q = [ordered]@{}
  try {
    $q.cache_mtime = ([DateTimeOffset](Get-Item -LiteralPath $dc).LastWriteTimeUtc).ToUnixTimeSeconds()
    $js = [IO.File]::ReadAllText($dc) | ConvertFrom-Json
    $q.headsets = @($js.devices | Where-Object { $_.type -eq 'headset' } | ForEach-Object {
      [ordered]@{ subtype = $_.subtype; battery = $_.batteryState; connection = $_.connectionState; power = $_.powerState;
                  airlink = $_.isUsingAirLink; last_seen = $_.lastSeenAt; operational = $_.operationalState } })
  } catch {}
  $lf = Get-ChildItem -LiteralPath $od -Filter 'Service_*.txt' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($lf) {
    $q.log_mtime = ([DateTimeOffset]$lf.LastWriteTimeUtc).ToUnixTimeSeconds()
    try {
      $fs = [IO.File]::Open($lf.FullName, 'Open', 'Read', 'ReadWrite,Delete')
      try {
        $n = [int][Math]::Min($fs.Length, 1MB); [void]$fs.Seek(-$n, 'End')
        $buf = New-Object byte[] $n; $got = $fs.Read($buf, 0, $n)
      } finally { $fs.Close() }
      $rx = '@RX@'
      $q.lines = @(([Text.Encoding]::UTF8.GetString($buf, 0, $got) -split "`n") |
        Where-Object { $_.Length -lt 400 -and $_ -match $rx } | Select-Object -Last 80 | ForEach-Object { $_.Trim() })
    } catch {}
  }
  $r.quest = $q
}
$r | ConvertTo-Json -Depth 4 -Compress
""".replace("@RX@", QUEST_LOG_RX)


def script_for(os_name: str) -> str:
    return WINDOWS_SCRIPT if os_name == "windows" else POSIX_SCRIPT


# --------------------------------------------------------------------------
# Parsers (pure, unit-tested)
# --------------------------------------------------------------------------

UNKNOWN_RATE = -0x80000000  # BATTERY_UNKNOWN_RATE as a signed int32
UNKNOWN_TIME = 0xFFFFFFFF


def _reading(pct, ac, state, rate_w=None, left=None, full_in=None, source="") -> dict:
    return {"percent": None if pct is None else round(float(pct), 1), "ac": ac, "state": state,
            "rate_w": None if rate_w is None else round(rate_w, 2), "time_left_s": left,
            "time_to_full_s": full_in, "source": source}


def parse_windows_power(sps: bytes | None, sbs: bytes | None) -> dict | None:
    """GetSystemPowerStatus (SYSTEM_POWER_STATUS, 12 bytes) and
    SYSTEM_BATTERY_STATE (32 bytes) → a reading, None without a battery."""
    if not sps or len(sps) < 12:
        return None
    ac_line, flag, pct, _saver, life, _full = struct.unpack("<BBBBii", sps[:12])
    if flag & 128 or (flag == 255 and pct == 255):  # no system battery / unknown
        return None
    ac = {0: False, 1: True}.get(ac_line)
    charging_flag = bool(flag & 8)
    discharging_flag = None
    rate_mw = mx = rem = est = None
    if sbs and len(sbs) >= 24:
        b_ac, present, charging, discharging = sbs[0], sbs[1], sbs[2], sbs[3]
        if not present:
            return None
        mx, rem, rate, est = struct.unpack("<IIiI", sbs[8:24])
        rate_mw = None if rate == UNKNOWN_RATE else rate
        est = None if est == UNKNOWN_TIME else est
        if ac is None:
            ac = bool(b_ac)
        charging_flag = charging_flag or bool(charging and not discharging) or bool(rate_mw and rate_mw > 0)
        discharging_flag = bool(discharging and not charging)
    if pct == 255:
        pct = 100.0 * rem / mx if mx else None
    if rate_mw is not None and rate_mw > 0:
        state = "charging"
    elif rate_mw is not None and rate_mw < 0:
        state = "mains_discharging" if ac else "battery"
    elif rate_mw == 0:
        state = ("full" if pct is not None and pct >= 99 else "not_charging") if ac else "battery"
    elif charging_flag:
        state = "charging"
    elif ac and discharging_flag:
        state = "mains_discharging"
    elif ac:
        state = "full" if pct is not None and pct >= 99 else "not_charging"
    elif ac is False:
        state = "battery"
    else:
        state = "unknown"
    left = full_in = None
    if state in ("battery", "mains_discharging"):
        if est:
            left = int(est)
        elif rate_mw and rate_mw < 0 and rem is not None:
            left = int(rem / -rate_mw * 3600)
        elif life >= 0:
            left = life
    elif state == "charging" and rate_mw and rate_mw > 0 and mx and rem is not None and mx > rem:
        full_in = int((mx - rem) / rate_mw * 3600)
    return _reading(pct, ac, state, None if rate_mw is None else rate_mw / 1000.0, left, full_in, "windows")


def parse_sysfs(lines: list) -> dict | None:
    """`NAME KEY VALUE` lines of /sys/class/power_supply → a reading of the
    first system battery (sorted by name), None without one."""
    sup: dict = {}
    for line in lines:
        parts = line.strip().split(" ", 2)
        if len(parts) == 3:
            sup.setdefault(parts[0], {})[parts[1]] = parts[2].strip()
    ac = None
    for s in sup.values():
        if s.get("type") in ("Mains", "USB", "USB_C", "USB_PD") and s.get("online") in ("0", "1"):
            ac = bool(ac) or s["online"] == "1"
    bats = [n for n in sorted(sup) if sup[n].get("type") == "Battery" and sup[n].get("scope", "System") != "Device"]
    if not bats:
        return None
    b = sup[bats[0]]

    def num(k):
        try:
            return float(b[k])
        except (KeyError, ValueError):
            return None

    status = b.get("status", "Unknown")
    now_e, full_e, power = num("energy_now"), num("energy_full"), num("power_now")
    if now_e is None:  # charge (µAh) and current (µA) instead of energy and power
        now_e, full_e, power = num("charge_now"), num("charge_full"), num("current_now")
        rate_w = None
    else:
        rate_w = abs(power) / 1e6 if power else None
    pct = num("capacity")
    if pct is None and now_e is not None and full_e:
        pct = 100.0 * now_e / full_e
    if status == "Charging":
        state, ac = "charging", True if ac is None else ac
    elif status == "Discharging":
        state = "mains_discharging" if ac else "battery"
    elif status == "Full":
        state = "full"
    elif status == "Not charging":
        state, ac = "not_charging", True if ac is None else ac
    else:
        state = "unknown"
    left = full_in = None
    p = abs(power) if power else None
    if p and now_e is not None:
        if state in ("battery", "mains_discharging"):
            left = int(now_e / p * 3600)
        elif state == "charging" and full_e and full_e > now_e:
            full_in = int((full_e - now_e) / p * 3600)
    if rate_w is not None and state in ("battery", "mains_discharging"):
        rate_w = -rate_w
    return _reading(pct, ac, state, rate_w, left, full_in, "sysfs")


_PMSET_BATT = re.compile(r"InternalBattery[^\t]*\t\s*(\d+)%;\s*([^;]+);\s*(?:(\d+):(\d+)\s+remaining|\(no estimate\))?")


def parse_pmset(lines: list) -> dict | None:
    """`pmset -g batt` → a reading, None on a Mac without a battery."""
    text = "\n".join(lines)
    m = _PMSET_BATT.search(text)
    if not m:
        return None
    ac = "'AC Power'" in text if "Now drawing from" in text else None
    pct, what = float(m.group(1)), m.group(2).strip().lower()
    t = int(m.group(3)) * 3600 + int(m.group(4)) * 60 if m.group(3) else None
    if what in ("charging", "finishing charge"):
        state = "charging"
    elif what == "discharging":
        state = "mains_discharging" if ac else "battery"
    elif what == "charged":
        state = "full"
    elif "not charging" in what or what == "ac attached":
        state = "not_charging"
    else:
        state = "unknown"
    left = t if state in ("battery", "mains_discharging") else None
    full_in = t if state == "charging" else None
    return _reading(pct, ac, state, None, left, full_in, "pmset")


_LOG_TIME = re.compile(r"^(\d{2})/(\d{2}) (\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?")
_HMD_BATT = re.compile(r"HMD battery level update: from (-?\d+) to (-?\d+)")
_CTRL_BATT = re.compile(r"Battery level for ovrControllerType_([LR])Touch changed to (-?\d+)")


def log_time(line: str, local_now: datetime, utc_now: float):
    """The epoch of a runtime log line ("17/11 23:44:31.922", day first, the
    machine's local time without a year), from the machine's local and UTC
    now; None without a time."""
    m = _LOG_TIME.match(line)
    if not m:
        return None
    day, mon, hh, mi, ss = (int(x) for x in m.groups()[:5])
    try:
        t = datetime(local_now.year, mon, day, hh, mi, ss)
    except ValueError:
        return None
    if t - local_now > timedelta(days=1):  # a line from last December, read in January
        t = t.replace(year=t.year - 1)
    return utc_now + (t - local_now).total_seconds()


def parse_quest(q: dict | None, utc_now: float, local_now: str | None) -> dict | None:
    """The `quest` object of the Windows probe → the headset's state: battery
    (log line or DeviceCache), its direction, USB / Link, controllers."""
    if not isinstance(q, dict):
        return None
    try:
        lnow = datetime.strptime(local_now or "", "%Y-%m-%dT%H:%M:%S")
    except ValueError:
        lnow = datetime.fromtimestamp(utc_now)
    out = {"battery": None, "battery_at": None, "battery_source": None, "trend": None, "usb": None,
           "link": None, "link_at": None, "connection": None, "airlink": None, "power": None,
           "last_seen": None, "controllers": {}, "mounted": None}
    hs = q.get("headsets") or []
    h = hs[0] if isinstance(hs, list) and hs and isinstance(hs[0], dict) else (hs if isinstance(hs, dict) else None)
    if h:
        out["connection"] = h.get("connection") if isinstance(h.get("connection"), str) else None
        out["power"] = h.get("power") if isinstance(h.get("power"), str) else None
        out["airlink"] = h.get("airlink") if isinstance(h.get("airlink"), bool) else None
        ls = h.get("last_seen")
        out["last_seen"] = float(ls) if isinstance(ls, (int, float)) and ls > 0 else None
        bat = h.get("battery")
        if isinstance(bat, (int, float)) and not isinstance(bat, bool) and 0 <= bat <= 100:
            out.update(battery=float(bat), battery_source="cache",
                       battery_at=out["last_seen"] or q.get("cache_mtime"))
    lines = q.get("lines") or []
    if isinstance(lines, str):
        lines = [lines]
    log_batt = None  # (level, time, trend) of the newest log line
    for line in lines:
        if not isinstance(line, str):
            continue
        t = log_time(line, lnow, utc_now)
        m = _HMD_BATT.search(line)
        if m:
            a, b = int(m.group(1)), int(m.group(2))
            if 0 <= b <= 100:
                log_batt = (float(b), t, ("charging" if b > a else "discharging" if b < a else None) if a >= 0 else None)
            continue
        m = _CTRL_BATT.search(line)
        if m:
            v = int(m.group(2))
            out["controllers"]["left" if m.group(1) == "L" else "right"] = v if 0 <= v <= 100 else None
            continue
        if "winusb detector] device with" in line:
            out["usb"] = "added]" in line
            if out["usb"] is False:
                out.update(link="stopped", link_at=t)
        elif "Connected participant on transport winusb" in line:
            out.update(usb=True, link="connected", link_at=t)
        elif "HMD Startup time first displayed frame" in line or "Streaming session got invited" in line:
            out.update(link="streaming", link_at=t)
        elif "didDisconnect" in line or "request to stop Link" in line:
            out.update(link="stopped", link_at=t)
    if log_batt is not None and (out["battery_at"] is None or log_batt[1] is None or log_batt[1] >= out["battery_at"] - 60):
        out.update(battery=log_batt[0], battery_at=log_batt[1], battery_source="log", trend=log_batt[2])
    if out["connection"] == "connected" and out["link"] in (None, "stopped"):
        out["link"] = "connected"
    out["controllers"] = {k: v for k, v in out["controllers"].items() if v is not None}
    return out


def parse_output(os_name: str, text: str) -> dict:
    """A probe's stdout → {"battery": reading|None, "quest": …|None, "now": t}."""
    if os_name == "windows":
        i = text.find("{")
        data = json.loads(text[i:].strip().splitlines()[0]) if i >= 0 else {}
        if not isinstance(data, dict):
            data = {}

        def b64(k):
            v = data.get(k)
            try:
                return base64.b64decode(v) if isinstance(v, str) else None
            except ValueError:
                return None
        now = data.get("now") if isinstance(data.get("now"), (int, float)) else time.time()
        return {"battery": parse_windows_power(b64("sps"), b64("sbs")),
                "quest": parse_quest(data.get("quest"), float(now), data.get("local")), "now": now}
    sec: dict = {}
    cur = None
    for line in text.splitlines():
        if re.fullmatch(r"@@\w+", line.strip()):
            cur = line.strip()[2:]
            sec[cur] = []
        elif cur:
            sec[cur].append(line)
    if "end" not in sec:
        raise ValueError("incomplete output")
    bat = parse_sysfs(sec.get("sysfs", [])) if "sysfs" in sec else None
    if bat is None and "pmset" in sec:
        bat = parse_pmset(sec["pmset"])
    return {"battery": bat, "quest": None, "now": time.time()}


# --------------------------------------------------------------------------
# Levels and text
# --------------------------------------------------------------------------

STATE_TEXT = {"charging": "charging", "mains_discharging": "on mains but discharging", "battery": "on battery",
              "full": "full, on mains", "not_charging": "on mains, not charging", "unknown": "state unknown"}


def level(pct, state=None) -> str:
    """ok / warn / bad: red below 20 %, amber below 40 % or when the battery
    drains on mains (a power supply too weak for the load)."""
    if pct is not None and pct < BAD_PCT:
        return "bad"
    if (pct is not None and pct < WARN_PCT) or state == "mains_discharging":
        return "warn"
    return "ok" if pct is not None else "unknown"


def fmt_dur(s) -> str:
    if s is None:
        return ""
    s = int(s)
    if s < 3600:
        return "%d min" % max(1, round(s / 60))
    return "%d h %02d min" % (s // 3600, (s % 3600) // 60)


def battery_text(r: dict) -> str:
    parts = ["%.0f %%" % r["percent"] if r.get("percent") is not None else "? %", STATE_TEXT.get(r.get("state"), "")]
    if r.get("rate_w"):
        parts.append("%+.1f W" % r["rate_w"])
    if r.get("time_left_s"):
        parts.append("%s left" % fmt_dur(r["time_left_s"]))
    if r.get("time_to_full_s"):
        parts.append("full in %s" % fmt_dur(r["time_to_full_s"]))
    return " · ".join(p for p in parts if p)


def warning_text(r: dict) -> str | None:
    if r.get("state") == "mains_discharging":
        w = "On mains but discharging"
        if r.get("rate_w"):
            w += " (%.1f W)" % r["rate_w"]
        return w + ": the power supply is too weak for the load"
    if r.get("percent") is not None and r["percent"] < BAD_PCT and r.get("state") != "charging":
        return "Battery low"
    return None


def quest_link_text(q: dict) -> str:
    link = q.get("link")
    via = "Air Link" if q.get("airlink") else "USB"
    if link == "streaming":
        return "Link streaming (%s)" % via
    if link == "connected":
        return "Link connected (%s)" % via
    if q.get("usb"):
        return "on USB, Link not running"
    if link == "stopped" or q.get("usb") is False or q.get("connection") == "disconnected":
        return "not connected"
    return "connection unknown"


def quest_text(q: dict, now: float) -> str:
    parts = []
    if q.get("battery") is not None:
        b = "%.0f %%" % q["battery"]
        age = now - q["battery_at"] if q.get("battery_at") else None
        linked = q.get("link") in ("streaming", "connected")
        old = age is not None and (age > QUEST_REPORT_STALE_S or (not linked and age > POLL_S))
        if q.get("trend") and not old:
            b += " " + q["trend"]
        parts.append(b)
        if old:
            parts.append("last report %s ago" % fmt_dur(now - q["battery_at"]))
    else:
        parts.append("battery not reported")
    parts.append(quest_link_text(q))
    c = q.get("controllers") or {}
    if c:
        parts.append("Touch " + " ".join("%s %d %%" % (k[0].upper(), v) for k, v in sorted(c.items())))
    return " · ".join(parts)


# --------------------------------------------------------------------------
# The monitor: one thread per machine, history, unreachable / stale
# --------------------------------------------------------------------------


def _series(hist, now: float, span: float = PAGE_SPAN_S, points: int = PAGE_POINTS) -> list:
    """The lowest value per bucket of the last `span` s (None = no sample)."""
    step = span / points
    out = [None] * points
    for t, v in hist:
        i = int((t - (now - span)) // step)
        if 0 <= i < points and v is not None:
            out[i] = v if out[i] is None else min(out[i], v)
    return out


class _State:
    def __init__(self):
        self.checked_at = None  # last probe (ok or not)
        self.ok_at = None  # last successful probe
        self.error = None  # the last probe's error, None when it worked
        self.reading = None
        self.reading_at = None
        self.has_battery = None  # None = not known yet
        self.quest = None
        self.quest_at = None
        self.hist = deque()
        self.qhist = deque()


class BatteryMonitor:
    """Polls every machine's battery in its own thread. `run(machine, script,
    timeout)` returns the probe's stdout or raises (dashboard.run_ssh)."""

    def __init__(self, machines: list, run, interval: float = POLL_S, timeout: float = TIMEOUT_S, clock=time.time):
        self.machines = machines
        self.run = run
        self.interval = interval
        self.timeout = timeout
        self.clock = clock
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.st = {m.name: _State() for m in machines}

    # -- recording (pure, tested) --
    def record(self, name: str, parsed: dict, now: float | None = None):
        now = self.clock() if now is None else now
        with self.lock:
            s = self.st.setdefault(name, _State())
            s.checked_at = s.ok_at = now
            s.error = None
            r = parsed.get("battery")
            s.has_battery = r is not None
            if r is not None:
                s.reading, s.reading_at = r, now
                s.hist.append((now, r.get("percent")))
            q = parsed.get("quest")
            if q is not None:
                s.quest, s.quest_at = q, now
                if q.get("battery") is not None and q.get("battery_at"):
                    if not s.qhist or s.qhist[-1][0] < q["battery_at"]:
                        s.qhist.append((min(now, q["battery_at"]), q["battery"]))
            for h in (s.hist, s.qhist):
                while h and h[0][0] < now - HISTORY_S:
                    h.popleft()

    def fail(self, name: str, error: str, now: float | None = None):
        now = self.clock() if now is None else now
        with self.lock:
            s = self.st.setdefault(name, _State())
            s.checked_at = now
            s.error = error or "unreachable"

    def poll(self, m):
        try:
            out = self.run(m, script_for(m.os), self.timeout)
            self.record(m.name, parse_output(m.os, out))
        except Exception as e:  # never let a probe kill the thread
            self.fail(m.name, str(e) or type(e).__name__)

    def wait_s(self, name: str) -> float:
        s = self.st.get(name)
        if s is not None and s.has_battery is False and s.quest is None and s.error is None:
            return NO_BATTERY_POLL_S
        return self.interval

    def _loop(self, m):
        while not self.stop.is_set():
            self.poll(m)
            self.stop.wait(self.wait_s(m.name))

    def start(self):
        for m in self.machines:
            threading.Thread(target=self._loop, args=(m,), daemon=True, name="battery-" + m.name).start()

    # -- the page's view --
    def view(self, name: str, now: float | None = None) -> dict | None:
        """The machine's battery for /api/status, None when it has none (or
        none was ever seen)."""
        now = self.clock() if now is None else now
        with self.lock:
            s = self.st.get(name)
            if s is None or (s.reading is None and s.quest is None):
                return None
            if s.error:
                status = "unreachable"
            elif s.ok_at is None or now - s.ok_at > STALE_S:
                status = "stale"
            else:
                status = "ok"
            v = {"status": status, "error": s.error, "checked_at": s.checked_at, "updated_at": s.reading_at,
                 "age": None if s.reading_at is None else now - s.reading_at, "span": PAGE_SPAN_S}
            if s.reading is not None and s.has_battery is not False:
                r = dict(s.reading)
                r["level"] = level(r.get("percent"), r.get("state"))
                r["text"] = battery_text(r)
                r["warning"] = warning_text(r)
                v["battery"] = r
                v["history"] = _series(s.hist, now)
            if s.quest is not None:
                q = dict(s.quest)
                q["level"] = level(q.get("battery"))
                q["text"] = quest_text(q, now)
                q["link_text"] = quest_link_text(q)
                q["updated_at"] = s.quest_at
                q["history"] = _series(s.qhist, now)
                v["quest"] = q
            if "battery" not in v and "quest" not in v:
                return None
            return v


def text_line(v: dict | None, now: float) -> str | None:
    """--once: one line per machine with a battery or headset."""
    if not v:
        return None
    parts = []
    if v.get("battery"):
        b = v["battery"]
        parts.append("battery %s" % b["text"] + (" [%s]" % b["warning"] if b.get("warning") else ""))
    if v.get("quest"):
        parts.append("Quest %s" % v["quest"]["text"])
    if v["status"] != "ok":
        parts.append("%s, last value %s ago" % (v["status"].upper(), fmt_dur(v.get("age"))))
    return "  " + "; ".join(parts)
