#!/usr/bin/env bash
# Is an agent slot free on the coordinator (deb)? (spec §11.2.1, dashboard 1.5.0)
#
#   scripts/agent-slots.sh        prints the limit, the running agents and the
#                                 free slots; exit 0 when a slot is free, 3 when not
#   scripts/agent-slots.sh -q     the same without output (for until-loops)
#
# A coordinator session runs it before it starts an agent and waits while it
# exits 3 (CLAUDE.md, Working mode). The limit is set by the user in the build
# dashboard (Coordinator card, "Agents on the coordinator"): `[coordinator]
# max_agents` in config/build-machines.toml, default 5. The running agents are
# counted like the dashboard's Claude tasks: the tasks of
# test-output/build-log/DAY/agents.jsonl whose last event is started / resumed.
# Both files are the main checkout's (config/ is git-ignored; a worktree uses
# the main checkout's, like scripts/remote.sh). Overrides for tests:
# OCW_AGENT_CONFIG (config directory), OCW_AGENT_LOG (build log directory).
# Needs only python3's standard library; reads two files, no network.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
main=$here
common=$(git -C "$here" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
[ -n "$common" ] && main=$(dirname "$common")
config=${OCW_AGENT_CONFIG:-}
if [ -z "$config" ]; then
  config=$here/config
  [ -f "$config/machines.toml" ] || [ -f "$config/build-machines.toml" ] || config=$main/config
fi
log=${OCW_AGENT_LOG:-$main/test-output/build-log}
exec python3 "$here/tools/build-dashboard/agent_slots.py" --config "$config" --log "$log" "$@"
