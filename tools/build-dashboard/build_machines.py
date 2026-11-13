#!/usr/bin/env python3
"""Build machines switched on and off, and their build slot counts
(spec §11.2, v2.66).

The settings live in config/build-machines.toml of the main checkout
(git-ignored, next to machines.toml, which stays the user's hand-edited
file). It uses the same TOML subset as machines.toml, so scripts/remote.sh
reads it with its `field` lookup:

    [[machine]]
    name = "leela"
    enabled = false
    slots = 2
    reason = "excluded by the user, 2026-11-13"
    note = "kiosk only"
    changed_by = "dashboard"
    changed_at = "2026-11-13T21:04:05+01:00"

- `enabled = false`: `remote.sh MACHINE run|sync` refuses the machine (exit
  3); ssh, field, fetch, put and deploys (scripts/deploy-test.sh) stay
  allowed. No entry, or no `enabled` key: enabled.
- `slots`: the machine's build slot count; wins over machines.toml and the
  OS default; OCW_REMOTE_SLOTS still wins for a single call. A new count
  applies to runs started after the change.
- `note`: a short free-text note shown on the dashboard card (any state).

The build dashboard changes it (POST /api/machines with the token of
config/dashboard-token); every change is one JSON line in
test-output/build-log/DAY/machines.jsonl (who, when, what) for the art
project. From a shell:

    python3 tools/build-dashboard/build_machines.py list
    python3 tools/build-dashboard/build_machines.py set leela --disable --reason "kiosk only"
    python3 tools/build-dashboard/build_machines.py set marge --enable --slots 3
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import tempfile
import threading
import time
from pathlib import Path

FILE_NAME = "build-machines.toml"
LOG_NAME = "machines.jsonl"
MIN_SLOTS, MAX_SLOTS = 1, 8          # what the dashboard and the CLI may set
MAX_FILE_SLOTS = 64                  # what a hand-edited file may hold (0 = no limit)
MAX_REASON = 200
MAX_BY = 40
_CTRL = re.compile(r"[\x00-\x1f\x7f]")

HEADER = """\
# Build machines: enabled for building, and their build slot counts.
# Written by the build dashboard (tools/build-dashboard/build_machines.py);
# git-ignored. scripts/remote.sh refuses `run` and `sync` on a machine with
# enabled = false (exit 3; ssh, fetch, put and deploys stay allowed). `slots`
# wins over machines.toml and the OS default; OCW_REMOTE_SLOTS wins for a call.
"""


def clean_text(s, n: int) -> str:
    """One line of text: control characters removed, at most n characters."""
    if s is None:
        return ""
    if not isinstance(s, str):
        raise ValueError("text expected")
    return _CTRL.sub(" ", s).strip()[:n]


def toml_str(s: str) -> str:
    """A TOML basic string that remote.sh's line lookup reads back: `#`
    (which it takes for a comment) as \\u0023, no control characters."""
    s = _CTRL.sub(" ", str(s))
    s = s.replace("\\", "\\\\").replace('"', '\\"').replace("#", "\\u0023")
    return '"%s"' % s


def iso(t: float) -> str:
    lt = time.localtime(t)
    off = time.strftime("%z", lt)
    return time.strftime("%Y-%m-%dT%H:%M:%S", lt) + (off[:3] + ":" + off[3:] if off else "")


def parse(text: str) -> dict:
    """name -> normalized entry {enabled, slots, reason, note, changed_by,
    changed_at}; unknown keys and broken values are dropped."""
    from dashboard import parse_machines  # same TOML subset as machines.toml

    _, items = parse_machines(text)
    out = {}
    for it in items:
        name = it.get("name")
        if not isinstance(name, str) or not name:
            continue
        slots = it.get("slots")
        if isinstance(slots, bool) or not isinstance(slots, int) or not 0 <= slots <= MAX_FILE_SLOTS:
            slots = None
        e = {"enabled": it.get("enabled") is not False, "slots": slots}
        for k, n in (("reason", MAX_REASON), ("note", MAX_REASON), ("changed_by", MAX_BY), ("changed_at", 40)):
            v = it.get(k)
            e[k] = clean_text(v, n) if isinstance(v, str) else ""
        out[name] = e
    return out


def render(entries: dict, order=()) -> str:
    """The file for `entries` (name -> entry), machines in `order` first."""
    names = [n for n in order if n in entries] + sorted(n for n in entries if n not in order)
    out = [HEADER]
    for n in names:
        e = entries[n]
        out.append("[[machine]]")
        out.append("name = %s" % toml_str(n))
        out.append("enabled = %s" % ("true" if e.get("enabled", True) else "false"))
        if e.get("slots") is not None:
            out.append("slots = %d" % e["slots"])
        for k in ("reason", "note", "changed_by", "changed_at"):
            if e.get(k):
                out.append("%s = %s" % (k, toml_str(e[k])))
        out.append("")
    return "\n".join(out)


class BuildMachines:
    """config/build-machines.toml, re-read when it changes (also hand
    edits or the CLI), and the change log in the build log."""

    def __init__(self, path: Path, log_root: Path | None = None, clock=time.time):
        self.path = Path(path)
        self.log_root = Path(log_root) if log_root else None
        self.clock = clock
        self.lock = threading.Lock()
        self._key = None
        self._entries: dict = {}
        self.error = None
        self.read_only = False  # dashboard --no-machine-settings

    def entries(self) -> dict:
        try:
            st = self.path.stat()
            key = (st.st_mtime_ns, st.st_size)
        except OSError:
            self._key, self._entries, self.error = None, {}, None
            return {}
        if key != self._key:
            try:
                self._entries = parse(self.path.read_text(encoding="utf-8"))
                self.error = None
            except (OSError, UnicodeDecodeError) as e:
                self._entries, self.error = {}, "%s: %s" % (self.path.name, type(e).__name__)
            self._key = key
        return self._entries

    def effective(self, m) -> dict:
        """The settings of Machine m that remote.sh applies: enabled, slots
        (override > machines.toml > OS default) and where the count is from."""
        e = self.entries().get(m.name) or {}
        if e.get("slots") is not None:
            slots, src = e["slots"], "dashboard"
        else:
            slots, src = m.slots, ("machines.toml" if getattr(m, "slots_configured", False) else "default")
        return {"enabled": e.get("enabled", True), "reason": e.get("reason") or "", "note": e.get("note") or "", "slots": slots,
                "slots_source": src, "slots_config": m.slots, "changed_by": e.get("changed_by") or "",
                "changed_at": e.get("changed_at") or ""}

    def update(self, m, enabled=None, slots=None, reason=None, note=None, by: str = "dashboard", source: str = "dashboard",
               order=()) -> dict:
        """Changes Machine m's entry (validated), writes the file atomically
        and logs the change; returns the new effective settings. A new entry
        starts with the machine's current slot count."""
        if enabled is not None and not isinstance(enabled, bool):
            raise ValueError("enabled must be true or false")
        if slots is not None and (isinstance(slots, bool) or not isinstance(slots, int)
                                  or not MIN_SLOTS <= slots <= MAX_SLOTS):
            raise ValueError("slots must be a whole number from %d to %d" % (MIN_SLOTS, MAX_SLOTS))
        if enabled is None and slots is None and note is None and reason is None:
            raise ValueError("nothing to change (enabled, slots, reason or note)")
        reason = clean_text(reason, MAX_REASON) if reason is not None else None
        note = clean_text(note, MAX_REASON) if note is not None else None
        by = clean_text(by, MAX_BY) or source
        with self.lock:
            entries = {k: dict(v) for k, v in self.entries().items()}
            before = self.effective(m)
            e = entries.get(m.name) or {"enabled": True, "slots": None}
            if e.get("slots") is None:
                e["slots"] = before["slots"]
            if enabled is not None:
                e["enabled"] = enabled
                # A reason belongs to the disabled state; enabling clears it.
                e["reason"] = (reason or e.get("reason") or "") if not enabled else (reason or "")
            elif reason is not None:
                e["reason"] = reason
            if slots is not None:
                e["slots"] = slots
            if note is not None:  # "" clears it
                e["note"] = note
            now = self.clock()
            e["changed_by"], e["changed_at"] = by, iso(now)
            entries[m.name] = e
            self._write(render(entries, order))
            after = self.effective(m)
            self._log(now, m.name, source, by, before, after)
            return after

    def _write(self, text: str):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd, tmp = tempfile.mkstemp(prefix=".build-machines.", dir=str(self.path.parent))
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(text)
            os.chmod(tmp, 0o644)
            os.replace(tmp, self.path)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise

    def _log(self, now, name, source, by, before, after):
        if self.log_root is None:
            return
        changes = {k: [before[k], after[k]] for k in ("enabled", "slots", "reason", "note") if before[k] != after[k]}
        line = {"time": iso(now), "machine": name, "source": source, "by": by, "changes": changes,
                "enabled": after["enabled"], "slots": after["slots"], "reason": after["reason"], "note": after["note"]}
        try:
            d = self.log_root / time.strftime("%Y-%m-%d", time.localtime(now))
            d.mkdir(parents=True, exist_ok=True)
            with open(d / LOG_NAME, "a", encoding="utf-8") as f:
                f.write(json.dumps(line, ensure_ascii=False, sort_keys=True) + "\n")
        except OSError as e:  # the log must never block a change
            print("build log: %s" % e, file=sys.stderr)


def main(argv=None) -> int:
    import dashboard as d

    ap = argparse.ArgumentParser(description="Enable/disable build machines and set their build slots")
    ap.add_argument("--config", type=Path, help="config directory (default: config/ of the main checkout)")
    ap.add_argument("--log", type=Path, help="build log directory (default: test-output/build-log of the main checkout)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list", help="the machines with their effective settings")
    s = sub.add_parser("set", help="change a machine")
    s.add_argument("machine")
    g = s.add_mutually_exclusive_group()
    g.add_argument("--enable", dest="enabled", action="store_true", default=None)
    g.add_argument("--disable", dest="enabled", action="store_false")
    s.add_argument("--slots", type=int)
    s.add_argument("--reason")
    s.add_argument("--note", help="free-text note shown on the card (\"\" clears it)")
    s.add_argument("--by", default="cli")
    a = ap.parse_args(argv)
    repo = d.HERE.parent.parent
    cfg = a.config or d.find_config_dir(repo)
    machines = d.load_machines(cfg)
    bm = BuildMachines(cfg / FILE_NAME, a.log or (d.main_checkout(repo) / "test-output" / "build-log"))
    if a.cmd == "set":
        m = next((x for x in machines if x.name == a.machine), None)
        if m is None:
            print("no machine %r in machines.toml" % a.machine, file=sys.stderr)
            return 2
        try:
            bm.update(m, a.enabled, a.slots, a.reason, a.note, a.by, "cli", [x.name for x in machines])
        except ValueError as e:
            print(e, file=sys.stderr)
            return 2
    for m in machines:
        e = bm.effective(m)
        print("%-14s %-8s slots %d (%s)%s" % (m.name, "enabled" if e["enabled"] else "DISABLED", e["slots"],
                                             e["slots_source"], "".join(" – " + x for x in (e["reason"], e["note"]) if x)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
