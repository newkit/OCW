#!/usr/bin/env bash
# Frees disk on the build machines (spec 11.2) by the cleanup rules of
# tools/build-dashboard/cleanup_rules.py – the rule set the dashboard's
# janitor applies by itself once a day; this is the same by hand.
#
#   scripts/remote-gc.sh [-v] [MACHINE...]     dry run: what would be freed
#   GC=1 scripts/remote-gc.sh [MACHINE...]     delete     (make remote-gc [GC=1])
#   scripts/remote-gc.sh --branch BRANCH [--dry-run] [MACHINE...]
#                                         cleanup on merge: delete the target/
#                                         of BRANCH's checkout on every
#                                         reachable machine (make remote-gc-branch
#                                         BRANCH=…); the coordinator runs it
#                                         after merging or abandoning a branch
#   scripts/remote-gc.sh --help           the rules and their thresholds
#                                         (OCW_GC_* environment variables)
#
# Default: every machine of config/machines.toml; unreachable ones are
# skipped. Only machine names are printed, never hosts. Every deletion is
# logged to test-output/build-log/DAY/janitor.jsonl of the main checkout.
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHONDONTWRITEBYTECODE=1 exec python3 tools/build-dashboard/remote_gc.py "$@"
