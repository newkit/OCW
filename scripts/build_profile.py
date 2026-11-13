#!/usr/bin/env python3
"""Build profiling helpers (spec §11.2).

    build_profile.py summary DIR            markdown summary of a
                                            scripts/build-profile.sh run
    build_profile.py compare BEFORE AFTER   before/after table of two runs
    build_profile.py cargo-log LOG          JSON summary of the cargo output
                                            in a remote.sh run log (for the
                                            build log's runs.jsonl)

The cargo --timings reports embed their units as `const UNIT_DATA = [...]`;
`units()` reads them for the slowest crates of a build.
Tests: python3 scripts/test_build_profile.py
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

# `Finished `dev` profile [...] target(s) in 1m 23s` / `in 4.56s`.
FINISHED = re.compile(r"^\s*Finished .* in (?:(\d+)m )?(\d+(?:\.\d+)?)s\s*$")
COMPILING = re.compile(r"^\s*(Compiling|Checking) \S+ v")
# remote.sh prints this when a run had to wait for a build slot.
WAITED = re.compile(r"^remote\.sh: got build slot \d+ after (\d+) s\s*$")


def parse_duration(m: re.Match) -> float:
    return int(m.group(1) or 0) * 60 + float(m.group(2))


def cargo_log(text: str) -> dict:
    """Cargo's own numbers in a run's output: the summed `Finished` times,
    the number of `Finished` lines (cargo invocations), the crates compiled
    or checked, and the build-slot wait. Missing lines (cargo -q, filtered
    output) give None / 0."""
    cargo_s, builds, compiled, wait = 0.0, 0, 0, None
    for line in text.splitlines():
        if (m := FINISHED.match(line)) is not None:
            cargo_s += parse_duration(m)
            builds += 1
        elif COMPILING.match(line):
            compiled += 1
        elif (m := WAITED.match(line)) is not None:
            wait = (wait or 0) + int(m.group(1))
    return {
        "cargo_s": round(cargo_s, 1) if builds else None,
        "cargo_builds": builds,
        "compiled": compiled,
        "wait_s": wait,
    }


def units(html: str) -> list[dict]:
    """The UNIT_DATA array of a cargo --timings report ([] if absent)."""
    m = re.search(r"const UNIT_DATA = (\[.*?\]);\s*$", html, re.S | re.M)
    if not m:
        return []
    try:
        return json.loads(m.group(1))
    except ValueError:
        return []


def slowest(us: list[dict], n: int = 10) -> list[tuple[str, float]]:
    """The n units with the longest duration, as (name target, seconds); the
    same crate in several units (lib, build script run) is summed."""
    total: dict[str, float] = {}
    for u in us:
        key = u.get("name", "?") + (u.get("target") or "")
        total[key] = total.get(key, 0.0) + float(u.get("duration") or 0)
    return sorted(total.items(), key=lambda kv: -kv[1])[:n]


def load_results(d: Path) -> list[dict]:
    p = d / "results.jsonl"
    if not p.exists():
        return []
    rows = []
    for line in p.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            pass
    return rows


def fmt_s(s: float | None) -> str:
    if s is None:
        return "–"
    return f"{int(s // 60)}m {s % 60:04.1f}s" if s >= 60 else f"{s:.1f}s"


def summary(d: Path) -> str:
    rows = load_results(d)
    label = rows[0]["label"] if rows else d.name
    md = [f"# Build profile: {label}", "",
          "| Command | Step | Time | Crates | Exit | Load | Target |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        mb = r.get("target_mb")
        md.append(f"| {r['command']} | {r['step']} | {fmt_s(r['seconds'])} | {r['compiled']} "
                  f"| {r['exit']} | {r.get('load', '')} | {f'{mb / 1024:.1f} GB' if mb else '–'} |")
    for html in sorted(d.glob("*-clean.html")):
        top = slowest(units(html.read_text(errors="replace")))
        if top:
            md += ["", f"Slowest units of `{html.stem}`:", ""]
            md += [f"- {name}: {fmt_s(s)}" for name, s in top]
    return "\n".join(md) + "\n"


def compare(a: Path, b: Path) -> str:
    """Before/after table: one row per (command, step) present in both."""
    ra = {(r["command"], r["step"]): r for r in load_results(a)}
    rb = {(r["command"], r["step"]): r for r in load_results(b)}
    md = [f"| Command | Step | {a.name} | {b.name} | Change |", "|---|---|---|---|---|"]
    for key, x in ra.items():
        y = rb.get(key)
        if y is None:
            continue
        before, after = float(x["seconds"]), float(y["seconds"])
        change = f"{(after - before) / before:+.0%}" if before > 0 else "–"
        md.append(f"| {key[0]} | {key[1]} | {fmt_s(before)} | {fmt_s(after)} | {change} |")
    return "\n".join(md) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) >= 2 and argv[0] == "summary":
        sys.stdout.write(summary(Path(argv[1])))
    elif len(argv) >= 3 and argv[0] == "compare":
        sys.stdout.write(compare(Path(argv[1]), Path(argv[2])))
    elif len(argv) >= 2 and argv[0] == "cargo-log":
        text = Path(argv[1]).read_text(errors="replace") if Path(argv[1]).exists() else ""
        print(json.dumps(cargo_log(text)))
    else:
        sys.stderr.write(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
