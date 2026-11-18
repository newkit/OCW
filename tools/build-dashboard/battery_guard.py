"""Battery guard: a laptop build machine stops building when its battery
runs low and builds again when it has recharged (dashboard 1.8.0).

Per machine a stop and a start threshold (hysteresis; `battery_stop` /
`battery_start` in config/build-machines.toml, `build_machines.py set NAME
--battery-guard 20:80|off`). The battery monitor (battery.py) hands every
probe to `BatteryGuard.observe`:

- below `stop` % in two readings in a row, and the machine is enabled:
  disabled for building with the reason "battery guard: N % (below S %) –
  building again above R %" and `disabled_by = "battery-guard"`; runs
  already going finish (remote.sh only refuses new ones);
- above `start` % in two readings in a row, and the guard was what disabled
  it: enabled again. A machine disabled by the user or the coordinator (any
  other `disabled_by`) is never enabled by the guard;
- between the thresholds: nothing changes;
- a failed probe (unreachable) or a reading without a percent changes
  nothing and breaks the streak, so a flaky connection never flaps the
  machine; a reading seen before (same time) is not counted twice.

Each action is a line in build-log/DAY/machines.jsonl (`source` and `kind`
"battery-guard", read by the art project) and in the dashboard's own log
(stderr, the journal of its unit).
"""

from __future__ import annotations

import sys
import threading
import time

import build_machines

CONFIRM = 2  # readings in a row beyond a threshold before the guard acts
GUARD = build_machines.GUARD


def reason_text(pct: float, stop: int, start: int) -> str:
    return "battery guard: %d %% (below %d %%) – building again above %d %%" % (round(pct), stop, start)


def _stderr(msg: str):
    print(msg, file=sys.stderr, flush=True)


class BatteryGuard:
    def __init__(self, bm, machines: list, log=_stderr, confirm: int = CONFIRM):
        self.bm = bm
        self.machines = machines
        self.log = log
        self.confirm = confirm
        self.lock = threading.Lock()
        self.streak: dict = {}  # name -> {"dir": "low"|"high", "n": int, "at": reading time, "pct": float}

    def _order(self):
        return [m.name for m in self.machines]

    def pending(self, name: str) -> dict | None:
        """The running streak of a machine (for the card), None without one."""
        with self.lock:
            s = self.streak.get(name)
            return dict(s, of=self.confirm) if s else None

    def observe(self, m, reading: dict | None, ok: bool = True, at: float | None = None) -> str | None:
        """One battery probe of Machine m: `reading` is battery.py's reading
        (None without a battery), `ok` False when the probe failed, `at` the
        reading's time. Returns "disabled" / "enabled" when the guard acted."""
        with self.lock:
            if self.bm is None or getattr(self.bm, "read_only", False):
                return None
            e = self.bm.effective(m)
            g = e.get("battery_guard")
            pct = reading.get("percent") if isinstance(reading, dict) else None
            if not g or not ok or not isinstance(pct, (int, float)) or isinstance(pct, bool):
                self.streak.pop(m.name, None)  # unreachable / no value: nothing changes
                return None
            prev = self.streak.get(m.name)
            if prev is not None and at is not None and prev.get("at") == at:
                return None  # the same reading again (stale): not a second one
            if pct < g["stop"]:
                d = "low"
            elif pct > g["start"]:
                d = "high"
            else:
                self.streak.pop(m.name, None)
                return None
            n = prev["n"] + 1 if prev is not None and prev["dir"] == d else 1
            self.streak[m.name] = {"dir": d, "n": n, "at": at, "pct": pct}
            if n < self.confirm:
                return None
            acts = (d == "low" and e["enabled"]) or (d == "high" and not e["enabled"] and e.get("disabled_by") == GUARD)
            if acts:  # after an action, the next one needs fresh readings (e.g. after a manual override)
                self.streak.pop(m.name, None)
            if d == "low" and e["enabled"]:
                self.bm.update(m, enabled=False, reason=reason_text(pct, g["stop"], g["start"]), by=GUARD,
                               source=GUARD, order=self._order(), guard_event=("disabled", pct))
                self.log("battery guard: %s disabled for building at %.0f %% (below %d %%; building again above %d %%)"
                         % (m.name, pct, g["stop"], g["start"]))
                return "disabled"
            if d == "high" and not e["enabled"] and e.get("disabled_by") == GUARD:
                self.bm.update(m, enabled=True, by=GUARD, source=GUARD, order=self._order(),
                               guard_event=("enabled", pct))
                self.log("battery guard: %s enabled for building again at %.0f %% (above %d %%)"
                         % (m.name, pct, g["start"]))
                return "enabled"
            return None

    def safe_observe(self, m, reading, ok=True, at=None):
        """observe() for the battery threads: an error is logged, never raised."""
        try:
            return self.observe(m, reading, ok, at)
        except Exception as ex:  # the guard must never stop the battery poll
            self.log("battery guard: %s: %s: %s" % (m.name, type(ex).__name__, ex))
            return None


def card_info(e: dict, pending: dict | None = None) -> dict | None:
    """`battery_guard` of a machine state for the page: thresholds, the
    card text, its last action and a running streak; None without a guard."""
    g = e.get("battery_guard")
    if not g:
        return None
    last = e.get("guard_last")
    out = {"stop": g["stop"], "start": g["start"], "text": "battery guard %d %% / %d %%" % (g["stop"], g["start"]),
           "active": (not e.get("enabled", True)) and e.get("disabled_by") == GUARD, "last": last, "last_text": "",
           "pending_text": ""}
    if last:
        pct = " at %d %%" % last["percent"] if last.get("percent") is not None else ""
        out["last_text"] = "%s building%s, %s" % (last["action"], pct, (last.get("at") or "").replace("T", " ")[:16])
    if pending and pending.get("n", 0) < pending.get("of", CONFIRM):
        out["pending_text"] = "%d of %d readings %s %d %%" % (
            pending["n"], pending["of"], "below" if pending["dir"] == "low" else "above",
            g["stop"] if pending["dir"] == "low" else g["start"])
    return out
