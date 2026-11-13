# Build server dashboard

A small web page that shows how busy the build machines from
`config/machines.toml` are and which build/test tasks run or wait on them.
Python 3 standard library only (3.8+), no installs.

```bash
python3 tools/build-dashboard/dashboard.py                    # http://<this machine>:8765/
python3 tools/build-dashboard/dashboard.py --host 127.0.0.1   # local only
python3 tools/build-dashboard/dashboard.py --port 9000
python3 tools/build-dashboard/dashboard.py --once             # print once as text (e.g. over SSH)
python3 tools/build-dashboard/dashboard.py --once --json      # the same as JSON
python3 tools/build-dashboard/dashboard.py --janitor dry-run  # log what the janitor would delete, delete nothing
python3 tools/build-dashboard/dashboard.py --janitor-plan     # dry run on every machine now, then exit (make janitor-plan)
curl -s localhost:8765/api/status                             # JSON for scripts
python3 -m unittest discover tools/build-dashboard            # tests (no network)
```

In a git worktree it reads the main checkout's `config/` (like
`scripts/remote.sh`); `--config DIR` overrides.

## What it shows

Every machine is polled every 5 s in its own thread, with one SSH call that
runs a read-only snippet (same SSH options as `scripts/remote.sh`; connections
are reused through an SSH control socket in `$XDG_RUNTIME_DIR` or the temp
directory, never in the repository).

- **Overview:** machines online, running tasks, runs waiting for a build slot,
  build slots used of the total (the sum of all machines' slots: 2 + 2 + 1 + 1
  = 6 with the default counts).
- **Per machine:** online/unreachable (with the error type: timeout,
  connection refused, no route to host, host key mismatch, …), CPU busy %,
  load per core (Windows: threads running or ready to run per core, plus the
  processor queue length when counters are allowed), memory and swap, free
  disk space, uptime.
- **Running tasks:** one row per running `remote.sh run` from its task marker
  (below): checkout – `main` (`~/src/OCW`), `agent <id>`
  (`~/src/OCW-worktree-agent-<id>`), a branch name (`~/src/OCW-<branch>`) –,
  the command, running time (since it got its slot), its slot, and the CPU
  (100 % = one core) and memory of the checkout's processes, per process kind (cargo, build script, rustc, link, test binary,
  ocw-desktop/ocw-server, ffmpeg, weston, Xvfb). Processes of a checkout
  without a marker (older remote.sh, manual runs) are listed as "no marker".
  The checkout of a process comes from its working directory, its arguments
  or its parent.
- **What each job is for:** above its command, every running and queued
  job gets one sentence on what the build is trying to achieve (below).
- **Build slots:** used of total and which checkout holds each one (markers;
  for runs without one, Linux: the `/tmp/ocw-build-slot-N.lock` flocks,
  macOS: the `/tmp/ocw-build-slot-N.d` lock directories).
- **Queue:** the runs waiting for a build slot, oldest first: checkout,
  command, how long and since when they wait. Waiters of an older remote.sh
  (blocked in `flock 9`, no marker) are listed with "(no marker)".
- **Kiosk** (machines with `/var/tmp/ocw`): whether the kiosk app runs and
  since when, the deployed build (`VERSION`), connected Bluetooth devices
  (names only).
- **Disks:** the home volume and, if it is another file system, the one
  holding the checkouts (`repo_path`'s directory); Windows: the checkouts'
  drive and C:. Use is df's (used / (used + free)); the bars turn amber
  above 75 % and red above 85 %, and a red banner over the page names every
  disk above 92 %.
- **Janitor:** its mode, the last pass per machine (what it deleted or, in
  dry-run mode, would delete, and why) and the recent log entries; the size
  of `test-output/` on this machine (never deleted).
- **Open questions:** every question for the user in `SPECIFICATION.md`
  (below), grouped by phase, with its id, title, text and proposal; the
  count is in the overview. Decided ones are hidden behind "show decided".
  Milestones that name an open question are listed as blocked work.

## Open questions

`spec_questions.py` parses the bullets `- **Q<n> – Title:** text` of the
spec (mostly the "Open questions (for the user)" subsections of §14). The
phase comes from the nearest `### Phase N – …` heading, the section from
the nearest heading (`4L`), the status from the bold label (`decided` or
`postponed` = decided; everything else, also "research", is open), the
proposal from "Proposal: …", "(proposal: …)" or the option marked
"(proposal)" / "(recommended)". The dashboard re-reads the main checkout's
`SPECIFICATION.md` when its mtime changes (`--spec FILE` overrides); the
spec stays the only place where questions are written and decided.

## Janitor (disk cleanup)

A thread of the dashboard; one machine per pass, at most one pass every
10 minutes, on the machine whose checkouts' disk is fullest above 75 %:

| Disk use | What goes |
|---|---|
| > 75 % | `target/` of agent checkouts (`<repo>-worktree-agent-…`) whose branch is merged into main (worktree removed) or gone, unused for 2 h; in the main checkout's `target/`, build units unused for 3 days that a newer unit of the same crate and kind replaced, and incremental session directories unused for 3 days |
| > 85 % | also `target/` of the least recently used agent checkouts idle > 24 h, oldest first, until the disk is estimated below 85 % |
| > 92 % | red banner and a log line |

"Unused" is the newest of `.ocw-last-run` (touched by every `remote.sh
run`), `target/`, `target/*/deps`, `.fingerprint` and `incremental`. Branch
states come from this checkout's git (like `scripts/remote-gc.sh`). Kept
always: the sources of every checkout, checkouts with a running or waiting
task (also checked on the machine right before each deletion; `target/` is
renamed first), the sccache cache, `data/`, `test-output/`, other
checkouts (`OCW-bisect-…`). Every deletion is logged as a JSON line in
`test-output/build-log/DAY/janitor.jsonl` (time, machine, path relative to
the checkouts' directory, bytes, reason, threshold, mode); dry-run logs
each plan once as `would-delete`. `--janitor off|dry-run|on` (default on).
`scripts/remote-gc.sh --branch <branch>` and `remote.sh`'s dev-profile
cleanup write to the same log.

The slot count of a machine is found like `scripts/remote.sh` does: its
`slots` key in `machines.toml`, else `[defaults] slots`, else the OS default
(linux 2, windows 1, macos 1). A run started with another `OCW_REMOTE_SLOTS`
writes its count into its marker, and the dashboard then shows the larger
one.

## Job descriptions

Each job's description (`description` in the JSON, at most 140 characters;
`description_detail` is the tooltip) is built by rules, without any model
calls, as "<purpose> – <what> on <machine>":

- **What** comes from the command: it is split into its shell steps
  (pipes into grep/tail, echo, redirections, `cd`, env variables and the
  like are left out; PowerShell's `cmd /c "…"` is unwrapped; a command cut at
  200 characters drops its cut word). cargo fmt / clippy / test / check
  become one "Checking formatting, clippy and the tests of ocw-render" (crates
  from `-p`, `--workspace` = "all tests", test name filters, `--test`,
  `--no-run`, the voice feature), `--target wasm32…` and `make test-wasm`
  "Browser (wasm) build check"; release builds; `scripts/deploy-test.sh`
  (also its remote script) "Release build for the kiosk deployment"; world
  builds (`scripts/build-<region>.sh`, `world-builder package|graph|…`,
  `fetch-pilot.sh` / `fetch-imagery.sh <region>`) with the region name; scene
  renders (`render-scenes.sh --only`, `--scenes`, `--world`);
  `build-profile.sh` (label, `--commands`, `--steps clean`, sccache) and
  `world-profile.sh`; `measure.py --label`; run-ocw `drive.sh shot|video`
  (X11/Wayland); `make test-*`, `make art-*`, `collect-test-output.sh`, the
  dashboard's tests, `rm -rf` of old checkouts. A plain build before the
  real step (e.g. `cargo build -p world-builder; scripts/build-tenerife.sh`)
  and sccache statistics only count when nothing else is there.
- **Purpose:** in an agent checkout the agent's task description (from the
  Claude Code transcripts: the Agent call or the agent's
  `subagents/agent-<id>.meta.json`); in the main checkout, for checks,
  deployments and app builds, the subject of the newest commit (or merge) of
  the main checkout made before the job started (remote.sh copies that
  working tree). Other jobs have no purpose part.
- **Machine:** its config name (the OS name when the machine is named after
  it, e.g. "macOS").

Examples: "Merge wheels on the road – checking clippy and all tests on
macOS", "Profile and speed up builds – measuring a clean build with a cold
sccache (build profiling: macos-sccache-cold, build, test) on macOS",
"Building the Tenerife world package on marge".

The user's prompts are no longer shown. `--show-prompts` still adds the
prompt a job belongs to (`prompt`, `agent`) to the JSON (and so to the build
log), off by default; `--no-context` describes jobs from their commands only
(no transcripts, no git). Descriptions are masked like everything else.

## Task markers

`scripts/remote.sh run` writes a small JSON file on the machine for as long as
the command runs: `/tmp/ocw-tasks/PID-START.json` on Linux and macOS,
`%TEMP%\ocw-tasks\PID-START.json` on Windows (no admin rights needed). It
holds the checkout directory name, branch, command (one line, at most 200
characters), start time (UTC epoch, from the machine's clock), the PID of the
remote shell, the state (`waiting` for a build slot, `running` with the
slot number), the time of the last state change (`since`) and the machine's
slot count (`slots`). The run deletes it on exit (EXIT trap / `finally`). A
marker whose PID is gone, or belongs to a process started after the marker (PID
reused), is stale: the dashboard ignores it (and only counts it), the next
`remote.sh run` on that machine deletes it.

## Build slots

`scripts/remote.sh run` takes a build slot before it runs the command; when
all are taken it writes a `waiting` marker, prints "remote.sh: waiting for a
free build slot" and tries all slots again every second, so it gets whichever
slot frees first (no strict first-come order). A waiting run whose ssh
session ended (remote.sh interrupted) gives up on Linux and macOS.

- Linux: `flock -n` on `/tmp/ocw-build-slot-N.lock`, held on fd 9 by the run's
  shell; the kernel drops it when the shell dies.
- macOS (no flock): `mkdir /tmp/ocw-build-slot-N.d` (atomic) with the owner's
  PID in `pid`; the EXIT trap removes it (also after INT/TERM/HUP). A
  directory whose PID is gone, or that has no PID after a minute, is taken
  over by the next run (under a second `mkdir` lock, `N.d.take`).
- Windows: `%TEMP%\ocw-tasks\slot-N.lock` opened with `FileShare.None`; the
  OS closes it when the run's process dies, so a killed run frees its slot.
  A waiting run keeps waiting after its ssh session ended.

Platform notes: macOS per-process CPU is ps's decaying `%cpu`. Linux samples
per-process CPU from `/proc/*/stat` over the 0.5 s CPU interval.

## Windows permissions

Windows is read with `Get-Process`, .NET and two Win32 calls bound in memory
(no compiler, no files), because CIM/WMI, the registry's Perflib and
performance counters are denied to a non-admin build user over SSH:

- CPU busy % comes from `GetSystemTimes` and per-process CPU from
  `TotalProcessorTime`: no rights needed.
- The load-like value is the number of threads running or ready to run (from
  the process list), also without rights.
- The processor queue length (`\System\Processor Queue Length`, read with
  PDH's English counter name because `Get-Counter` only takes localized names)
  needs the group "Performance Monitor Users"; without it the page shows
  "n/a (add build user to Performance Monitor Users)". An admin adds the user
  once (in an elevated PowerShell), then restarts sshd so new logons get the
  group:

  ```powershell
  Add-LocalGroupMember -SID S-1-5-32-558 -Member <build user>
  Restart-Service sshd
  ```

- Processes cannot be assigned to a checkout unless they run from its
  `target\` directory (command lines and parents need CIM). When only one
  run is active, the unassigned processes started since it began are counted
  for it; the rest stay under "other".

## Security

- The server listens on all addresses by default, so it is reachable from
  the LAN: anyone there can see machine load and the build processes. Use
  `--host 127.0.0.1` to keep it local.
- It never shows host names, IP addresses, user names or keys: machines are
  identified by their config `name`, and all text from the machines or from
  ssh is masked (addresses, `user@host`, home directories, MAC addresses)
  before it is stored or served.
- The web page has no write actions. The poll snippets change nothing on
  the machines (no kills, no files written; stale task markers are cleaned
  up by `remote.sh`, not by the dashboard). The only changes it makes are
  the janitor's deletions of build artefacts (`<checkout>/target` and old
  units in the main checkout's `target/`), off with `--janitor off`, only
  logged with `--janitor dry-run`.
- Task markers hold only the checkout directory name, branch, command, start
  time, PID and state, never hosts, users or keys; the dashboard masks them
  like all other text.
- It reads `config/machines.toml` and the keys in `config/ssh/`, which stay
  git-ignored.

## Run as a service

`make dashboard-install` installs `ocw-build-dashboard.service` as a systemd
**user** service (no root): it starts now and at every login, restarts on
failure, and serves http://localhost:8765/ (all addresses, read-only).
Logs: `journalctl --user -u ocw-build-dashboard`. To start it at boot without
a login, once: `sudo loginctl enable-linger $USER`. Remove it with
`make dashboard-uninstall`.
