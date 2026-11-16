#!/usr/bin/env python3
"""Free agent slots on the coordinator (deb), for scripts/agent-slots.sh
(spec §11.2.1, dashboard 1.5.0).

The limit is `[coordinator] max_agents` of config/build-machines.toml (set
in the build dashboard; default 5); the running agents come from
test-output/build-log/DAY/agents.jsonl, counted like the dashboard's Claude
tasks (last event started / resumed). Prints

    limit 5 (dashboard)
    running 3
    free 2

and exits 0 when a slot is free, 3 when not (2: usage). Standard library
only; reads two files, starts nothing.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import build_machines
import claude_tasks

EXIT_FULL = 3


def check(config_file: Path, build_log: Path, now: float | None = None) -> dict:
    """The setting and the agent slots (claude_tasks.agent_slots)."""
    setting = build_machines.BuildMachines(config_file).coordinator()
    return dict(setting, **claude_tasks.read_agent_slots(build_log, setting["max_agents"], now))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Is an agent slot free on the coordinator? (exit 0 yes, 3 no)")
    ap.add_argument("--config", type=Path, required=True, help="config directory (holds build-machines.toml)")
    ap.add_argument("--log", type=Path, required=True, help="build log directory (holds DAY/agents.jsonl)")
    ap.add_argument("-q", "--quiet", action="store_true", help="print nothing, only the exit code")
    a = ap.parse_args(argv)
    s = check(a.config / build_machines.FILE_NAME, a.log)
    if not a.quiet:
        print("limit %d (%s)" % (s["max_agents"], s["source"]))
        print("running %d" % s["running"])
        print("free %d" % s["free"])
    return 0 if s["free"] > 0 else EXIT_FULL


if __name__ == "__main__":
    sys.exit(main())
