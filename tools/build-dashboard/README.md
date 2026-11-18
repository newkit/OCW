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
python3 tools/build-dashboard/dashboard.py --no-coordinator   # without the coordinator card and the Claude tasks
python3 tools/build-dashboard/coordinator.py                  # one sample of this machine as JSON
scripts/agent-slots.sh                                        # agent limit, running agents, free slots (exit 0 / 3)
curl -s localhost:8765/api/status                             # JSON for scripts
python3 -m unittest discover tools/build-dashboard            # tests (no network)
```

In a git worktree it reads the main checkout's `config/` (like
`scripts/remote.sh`); `--config DIR` overrides.

## Versions

The dashboard has its own version, `VERSION` in `dashboard.py` (also
`dashboard.py --version`, the page's header with the checkout's commit, and
`dashboard` in `/api/status`). **Every change of the dashboard – server, page
or systemd unit – bumps it and adds a line here** (minor for features, patch
for fixes; a test checks that the newest line here matches `VERSION`). An
open page reloads itself when the server runs another version (after a pull
and a restart); while something is typed it shows a banner instead.

When a change touches `ocw-build-dashboard.service`, the running unit changes
only after `make dashboard-install` (it copies the unit, reloads systemd and
restarts); every other change needs only a restart (the page's *Restart*
button, or `systemctl --user restart ocw-build-dashboard.service`).

| Version | Date | Change | Unit changed |
|---|---|---|---|
| 1.8.0 | 2026-11-18 | **Battery guard** (`battery_guard.py`): per machine a stop and a start threshold (`battery_stop` / `battery_start` in `config/build-machines.toml`, `build_machines.py set NAME --battery-guard 20:80\|off`, *Battery guard…* on the card, `battery_guard` in `POST /api/machines`); below the stop value (two readings in a row) the machine is disabled with "battery guard: N % (below S %) – building again above R %" and `disabled_by = "battery-guard"`, above the start value it is enabled again – only when the guard disabled it; unreachable or missing readings change nothing; "battery guard 20 % / 80 %" and its last action on the card; actions in `machines.jsonl` (`kind` `battery-guard`) and the dashboard's log. leela: 20 % / 80 % | no |
| 1.7.0 | 2026-11-17 | **Battery on the machine cards** (`battery.py`): laptops (Windows `GetSystemPowerStatus` + `CallNtPowerInformation`, Linux `/sys/class/power_supply`, macOS `pmset -g batt`) with percent, charging / on mains but discharging / on battery, rate and time left, red below 20 %, amber below 40 % or when draining on mains (with a warning), a 4 h sparkline; the **Meta Quest** on a Windows machine with the Meta runtime (level, charging / discharging, USB / Link, Touch controllers from the runtime's `DeviceCache.json` and service log); every 2 min per machine with a 25 s timeout, unreachable machines keep the last value with its age; `--no-battery` | no |
| 1.6.0 | 2026-11-17 | **Open questions kept current**: "Q30 (open) – …" bullets are read (a status in brackets before the dash), `spec_questions.py refresh` fast-forwards the main checkout (only on `main` without tracked changes) and lints the questions (bullets the dashboard cannot read, answers written outside the bold label), every 15 minutes by the new `ocw-spec-questions.timer` (report `test-output/answers/questions-refresh.json`); `spec_questions.py lint SPEC` | yes (new timer; `make dashboard-install`) |
| 1.5.0 | 2026-11-16 | **Agents on the coordinator selectable**: `[coordinator] max_agents` in `config/build-machines.toml` (default 5, 1–16; − N + on the Coordinator card with the token, `POST /api/coordinator`, `build_machines.py agents N`), `agents` in `/api/status`, "agents N of M" on the card and in the Claude tasks header (amber at the limit, red above it) with deb's available memory, `scripts/agent-slots.sh` for the sessions | no |
| 1.4.1 | 2026-11-16 | The cutover line is gone from the Claude tasks card; the worktree list stays open (kept in the browser across refreshes and updates) | no |
| 1.4.0 | 2026-11-16 | Versioning: `VERSION`, `--version`, version and commit in the header and `/api/status`, the page reloads itself on a new version | no |
| 1.3.0 | 2026-11-16 | Temp files on disk (`TMPDIR=%h/tmp` in the unit; deb's `/tmp` is a tmpfs); the Claude tasks section also reads `~/tmp/claude-*` | **yes** – `make dashboard-install` |
| 1.2.0 | 2026-11-16 | *Restart* button (`POST /api/restart`, token), and a changed `config/machines.toml` restarts it with the new machines | no |
| 1.1.0 | 2026-11-16 | Favicon: a bicycle wheel with a live status dot | no |
| 1.0.0 | 2026-11-15 | Everything before versioning: machines, tasks and queue, open questions and answers, enable/disable and slots, janitor, art status, coordinator (deb) card, Claude tasks | – |

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
- **Art project:** the last run of the hourly art collector
  (`collect.sh` of the art project's own repository
  [OCW-ART](https://github.com/newkit/OCW-ART), its `ART-PROJECT.md` §5.1;
  installed there with `make install-timer`), from the newest
  `test-output/build-log/DAY/art.json` of the main checkout, where the
  collector still writes its status: time, status
  (ok / failed / locked), new files, media of today, the mask check and
  any errors; "late" when no run ended for 2.5 h (or a run never ended).
  Also in `--once` and `/api/status` (`art`).
- **Coordinator (deb)** and **Claude tasks:** the machine the dashboard
  runs on and the agents working there, at the top of the page (below).
- **Open questions:** every question for the user in `SPECIFICATION.md`
  (below), grouped by phase, with its id, title, text and proposal; the
  count is in the overview. Decided ones are hidden behind "show decided".
  Milestones that name an open question are listed as blocked work.

## Battery (laptops, Meta Quest)

`battery.py`: a card shows a **Battery** line when the machine has a system
battery (a laptop build machine; desktops, a mouse's battery and a Mac mini
show nothing), and a **Quest** line when a Windows machine has the Meta
Quest runtime (`%LOCALAPPDATA%\Oculus`, e.g. Weela with the Quest on Link).
Each machine is probed in its own thread every **2 min** (machines without
a battery every 10 min) over the same SSH control socket, with a **25 s**
timeout, so a slow or unreachable machine never holds up the page. Also in
`/api/status` (`battery` of a machine), `--once` and the build log
(`servers.jsonl`, without the sparkline). `--no-battery` turns it off.

| Platform | Source (read-only, no admin rights) |
|---|---|
| Windows | kernel32 `GetSystemPowerStatus` (percent, mains, charging) and powrprof `CallNtPowerInformation(SystemBatteryState)` (capacity, rate in mW, Windows' estimate), bound in memory like the main poll's Win32 calls; CIM (`Win32_Battery`) is denied over SSH, `powercfg` and Windows Forms are not needed |
| Linux | `/sys/class/power_supply/*`: the first `type=Battery` without `scope=Device` (mice and keyboards report `Device`); `Mains`/`USB` `online` for the mains |
| macOS | `pmset -g batt` |

- **Shown:** battery icon, percent, state (charging, on mains but
  discharging, on battery, full, on mains not charging), the rate in W
  (+ charging, − discharging) and the time left or until full when the
  system gives it; a sparkline of the last 4 h (5-min buckets, the lowest
  value of each; kept in memory for 6 h, empty after a restart).
- **Colours** (`battery.level`): red below 20 %, amber below 40 % **or
  when the battery drains on mains** ("On mains but discharging (−12 W): the
  power supply is too weak for the load" – Weela's power supply cannot carry
  a full build plus VR); green otherwise. The same thresholds for the Quest.
- **Unreachable / stale:** a failed probe shows "unreachable – last value
  N min ago" (and the reason) with the last known values; no successful
  probe for 7 min shows "no fresh value". A machine that never showed a
  battery shows nothing.
- **Meta Quest** (read-only; nothing is started on the machine – no
  OculusDebugTool, no adb, no OpenXR session, which another app may hold –
  so a running VR app or the kiosk is never disturbed): the headset entry of
  the runtime's `DeviceCache.json` (its last battery level, `connectionState`,
  `isUsingAirLink`, `lastSeenAt`) and the last 1 MB of the newest
  `Service_*.txt` log (opened shared; the runtime keeps writing it). Its
  `[RemoteHeadset] HMD battery level update: from A to B` lines give the
  level while the headset is on Link and, from A → B, whether it is charging
  or discharging; `[winusb detector] … added/removed`, `Connected participant
  on transport winusb`, `Streaming session got invited` / `HMD Startup time
  first displayed frame` and `Received request to stop Link` /
  `didDisconnect` give USB and Link ("Link streaming (USB)", "on USB, Link
  not running", "not connected"); `Battery level for ovrControllerType_…`
  the Touch controllers. The newer of log and cache wins; when the headset
  is not on Link the level is shown with its age ("last report 20 min
  ago"). Whether the headset is **on the face is not available**: neither
  file has a proximity state, and reading it would need the headset (adb in
  developer mode) or an OpenXR session. Only derived values leave the
  parser, never raw log lines (they can hold device serials).

### Battery guard

`battery_guard.py` (1.8.0; user, 2026-11-18: "stop building on leela when
battery <20% and only start when >80%"): a laptop build machine stops taking
builds when its battery runs low and takes them again once it has
recharged. Per machine a **stop** and a **start** threshold (hysteresis):

- **Set:** `battery_stop` / `battery_start` in `config/build-machines.toml`;
  `python3 tools/build-dashboard/build_machines.py set leela
  --battery-guard 20:80` (`off` removes it); the card's *Battery guard…*
  button (with the token; `POST /api/machines {"machine", "battery_guard":
  "20:80" | "off"}`). 1 ≤ stop < start ≤ 99.
- **Below stop** in **two battery readings in a row** (about 4 min), and
  the machine is enabled: it is disabled for building with the reason
  "battery guard: 18 % (below 20 %) – building again above 80 %" and
  `disabled_by = "battery-guard"`. Running jobs finish (remote.sh refuses
  only new `run` / `sync`, as with any disable).
- **Above start** in two readings in a row: enabled again – **only when the
  guard disabled it**. Any other disable, or a new reason on a
  guard-disabled machine (the page, the CLI, the coordinator), sets
  `disabled_by` to who did it, and the guard never enables that machine;
  enabling by hand clears it. Between the thresholds nothing changes.
- **No flapping:** an unreachable probe or a reading without a percent
  changes nothing and breaks the streak; the same reading seen twice
  counts once; after an action the next one needs two fresh readings.
  The streaks live in memory (a restart starts them afresh); with
  `--no-battery` or `--no-machine-settings` the guard does nothing.
- **Shown:** "battery guard 20 % / 80 % · last: disabled building at
  18 %, 2026-11-18 21:04" on the card (amber edge while the guard holds the
  machine off, "1 of 2 readings below 20 %" while a streak runs), the
  thresholds in `--once` and `build_machines.py list`, `battery_guard` and
  `disabled_by` per machine in `/api/status`.
- **Logged:** each action is a line in `build-log/DAY/machines.jsonl`
  (`source` and `kind` `battery-guard`, `guard: {action, percent}`; the
  art project reads the build log) and in the dashboard's own log (stderr,
  the unit's journal).

## Coordinator (deb)

`coordinator.py`: a card for the machine the dashboard runs on (the
coordinator: Claude Code sessions and their agents, the art collector,
ffmpeg renders, git worktrees), added after deb ran out of memory on
2026-11-15 at 00:39. Read locally, without root and cheaply: `/proc` and
`/sys` every 10 s (one `/proc/PID/stat` read per process: name, parent and
RSS – never `cmdline` or `environ`, which can hold secrets and paths); the
few commands it starts are throttled and time-limited. Also in
`/api/status` (`coordinator`) and `--once`.

| Part | What | Source |
|---|---|---|
| Memory | total, available, used, swap; top 5 processes by RSS (name and RSS only); group sums: claude (also Claude Code's versioned binary, e.g. `2.1.325`), cargo/rustc, ffmpeg, python, qemu (VMs; `qemu-ga` is deb's own guest agent), browsers, other | `/proc/meminfo`, `/proc/PID/stat` |
| OOM kills | time and the killed unit or process, the count since boot | user journal (systemd's unit OOM message, `MESSAGE_ID=fe6faa94…`; every 5 min and at once when the counter grows), `/proc/vmstat` `oom_kill`; the kernel log (`journalctl -k`) and `dmesg` only when readable (group `adm` / `systemd-journal`, `kernel.dmesg_restrict=0`), else "not readable" |
| CPU | load against the cores; PSI some/full avg10/60/300 of cpu, memory, io | `/proc/loadavg`, `/proc/pressure/*` |
| Disks | `/`, `/home`, `/home/claude`, `/tmp`, each only when it is its own mount, deduplicated by device (tmpfs: "in RAM") | `/proc/mounts`, `statvfs` |
| Big directories | OCW's `target/`, `test-output/`, `.claude/worktrees/`; OCW-ART's `build-history/` and `.git` (with its Git LFS objects; next to the main checkout) | `nice ionice -c3 du -sx`, every 10 min, at most 60 s each, in its own thread |
| Activity | Claude Code sessions (claude processes without a claude parent) and their RSS, OCW's worktrees (`.git/worktrees/*/gitdir`, live and stale), ffmpeg, QEMU VMs, the art timer (state, next run), the dashboard's memory (its cgroup when it runs as the unit, else RSS) against `MemoryMax` | `/proc`, `systemctl --user show ocw-art-collect.timer` every 5 min, `/proc/self/cgroup`, the unit file |

**Alerts** (`coordinator.THRESHOLDS`, the one place for them; amber / red):

| Alert | Amber | Red |
|---|---|---|
| memory available | < 25 % or < 4 GiB | < 15 % or < 2 GiB |
| swap used | > 50 % | > 80 % |
| a disk (df's Use%) | > 85 % | > 90 % |
| OOM kill | in the last 7 days | in the last 24 h |
| memory pressure (some, avg60) | > 10 % | > 30 % |
| load per core (1 min) | > 1.5 | > 3 |
| the dashboard / its MemoryMax | > 75 % | > 90 % |
| cargo/rustc on the coordinator | any | – |

Red ones join the disks' red banner over the page, amber ones get an
amber banner; `--once` prints them as `ALERT` / `WARN` lines.

**History:** one sample a minute, as one JSON line in
`test-output/build-log/DAY/deb.jsonl` (the same build-log root as
`servers.jsonl`: `--record DIR`, none with `--no-record`; `servers.jsonl`
leaves the card out):

```json
{"time": "2026-11-15T14:35:00+01:00", "t": 1794749700.0,
 "memory": {"total": …, "available": …, "used": …, "free": …, "shmem": …, "swap_total": …, "swap_used": …,
            "available_pct": 82.3, "swap_pct": 95.1},
 "load": [0.5, 0.89, 0.75], "ncpu": 4, "psi": {"cpu": 0.4, "memory": 0.0, "io": 16.7},
 "disks": [{"mount": "/", "free": …, "pct": 94.0}, …],
 "groups": {"claude": …, "python": …}, "top": [["claude", 723456789], …],
 "claude_sessions": 1, "ffmpeg": 0, "qemu": 0, "oom_kill_since_boot": 2,
 "dashboard_rss": …, "alerts": ["bad swap", "bad disk", "bad oom"]}
```

Bytes everywhere; `psi` is "some" avg60 in %. The last 24 h are kept in
memory (read back from the day files on start, bad lines skipped); the
page draws sparklines of the last 6 h (memory available, 1-min load, root
disk free; inline SVG, the worst value of each bucket).

## Claude tasks

`claude_tasks.py`: the agents of the coordinator, next to its card (also
`claude_tasks` in `/api/status` and in `--once`). Refreshed every 30 s by
the coordinator thread; git at most every 2 min.

- **Tasks:** `test-output/build-log/DAY/agents.jsonl` of the main checkout
  (today and the two days before; written, masked, by OCW-ART's `diary.py
  agent-event`): one line per event (`time`, `agent_id_short`, `task`,
  `branch`, `event`, `commits`, `note`). The last event decides: `started`
  / `resumed` = running (with branch, start time, running time and last
  event); `interrupted` is listed with them; `reported`, `merged`,
  `abandoned` in the last 24 h behind the "finished" toggle. Lines whose
  `task` holds the agent id are understood too (the branch then names the
  task).
- **Liveness:** Claude Code keeps each background task's transcript as
  `/tmp/claude-<uid>/<project>/<session>/tasks/<task id>.output` (found
  with the glob `/tmp/claude-*/*/*/tasks/`, so also for the `claude` user).
  Only the file's **mtime and size** are used (`os.stat`): "growing" /
  "active", "idle / waiting" when unchanged for over 20 min, "unknown"
  without a file. The files are full transcripts (prompts, tool output) and
  are **never opened**; session ids (directory names) are never shown.
- **Worktrees:** `git worktree list --porcelain` of OCW's main checkout and
  of OCW-ART, `for-each-ref` and `branch --merged main`: per branch the last
  commit (short hash, subject, age), the commits not in `main` (cached per
  branch and main commit), merged into `main`, pushed (`refs/remotes/
  origin/<branch>`, never fetched). Linked to a task by its branch or by
  `worktree-agent-<id>`. The page lists main, unmerged branches with
  commits and the tasks' worktrees; the rest are counts. Paths are shown as
  `~/…`, Claude Code's session directories as `/tmp/claude-…` (on deb
  `~/tmp/claude-…`). The list is open unless the viewer closes it; the choice
  is kept in the browser (refreshes, reloads, new versions).
- The cutover check of the coordinator's move (`docs/MIGRATE-DEB-USER.md`) is
  still in `/api/status` (`claude_tasks.cutover`) but no longer on the page
  (1.4.1; the move is done).

## Agents on the coordinator

How many agents (Claude Code subagents in worktrees) may run on the
coordinator at once – each holds memory on deb, which ran out of memory on
2026-11-15 with about ten agents plus VMs. The user sets the number on the
Coordinator card; the sessions read it before they start an agent.

- **Setting:** `[coordinator] max_agents` in `config/build-machines.toml`
  (the file of the machine settings below; the table comes before the
  machines, and `remote.sh`'s lookup skips it). Default **5** without the
  table or with a broken value; the page and the CLI set **1–16** (a hand
  edit may hold up to 64). `changed_by` / `changed_at` like a machine's.

  ```toml
  [coordinator]
  max_agents = 5
  changed_by = "dashboard"
  changed_at = "2026-11-16T15:00:00+01:00"
  ```
- **Page:** a line on the Coordinator card, "agents N of M" with the free
  slots and deb's available memory, and a − N + stepper (same token as the
  other controls; without it only the numbers); amber at the limit, red
  above it. The Claude tasks header has the same "agents N of M" badge.
- **Counting:** the running tasks of the Claude tasks section – the tasks of
  `agents.jsonl` whose last event is started / resumed (interrupted ones
  hold no slot). So an agent counts only when its session logs its
  `started` event (and frees its slot with `reported` / `merged` /
  `abandoned`) with OCW-ART's `diary.py agent-event`.
- **API:** `agents` in `/api/status`: `max_agents`, `source` (`dashboard` /
  `default`), `default`, `min`, `max`, `running` (null without the Claude
  tasks section), `free`, `level` (`ok`, `warn` at the limit, `bad` above
  it), `text` ("agents N of M"), `mem_available`, `mem_total`,
  `changed_by`, `changed_at`. `POST /api/coordinator {"max_agents", "by"?}`
  with the `X-OCW-Token` header (403 without it, 400 for anything but a
  JSON integer 1–16). Every change is a line in `machines.jsonl` with
  `"machine": "coordinator"`. `--once` prints an `Agents on the
  coordinator:` line.
- **For the sessions:** `scripts/agent-slots.sh` prints `limit M
  (dashboard|default)`, `running N`, `free F` and exits 0 when a slot is
  free, 3 when not (`-q`: no output; python3's standard library only,
  reads the two files, no network, works without the dashboard running).
  Wait for a slot with Monitor, e.g. `until scripts/agent-slots.sh -q; do
  sleep 60; done`. `OCW_AGENT_CONFIG` / `OCW_AGENT_LOG` point it at other
  files (tests).
- **CLI:** `python3 tools/build-dashboard/build_machines.py agents [N]`
  shows or sets the limit (logged with source `cli`).

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

### Answering on the page

Every open question has an answer box and quick buttons: "Accept proposal"
when the spec names one, and the options the question lists ("A: …",
"B: …", from its "…, or …" alternatives); a button fills the box, "Send
answer" stores it. Large touch targets, real buttons (keyboard: Tab, Enter),
16 px text fields (no zoom on phones).

- **Storage:** append-only `test-output/answers/answers.jsonl` of the main
  checkout (git-ignored, collected by the art project): one JSON line per
  answer with the ISO time, the question id, title and **exact question
  text from the spec** (taken on the server, not from the browser), the
  answer, the chosen option, the spec version, and which earlier answer it
  supersedes. A new answer to the same question supersedes the old one (the
  page shows the latest; "Send new answer" edits).
- **The coordinator:** each answer is also one line in
  `test-output/answers/inbox.log` (`tail -f` it), and `GET
  /api/answers?since=<unix time>` lists the answers after that time. A
  question answered here shows "answered – waiting for the coordinator to
  record it in the spec" until the spec marks it decided; then it is
  decided (the answer stays visible with the decided ones).
- **Token:** writing needs the token in `config/dashboard-token` of the
  main checkout (git-ignored; created on the first start with a random
  value, mode 0600). The page asks for it once and keeps it in the
  browser's localStorage; it is sent as the `X-OCW-Token` header. A POST
  without it, or with a wrong one, gets 403 and nothing is stored. Reading
  (the page, `/api/status`, `/api/answers`) needs no token. The token is
  never logged, served or written to the answer files. `--no-answers`
  turns answering off.

## Build machines: enabled/disabled and build slots

Each machine card has an **Enabled/Disabled** toggle, a **slots stepper**
(− N +, 1–8) and a **Note…** button (a short free-text note shown on the
card, e.g. "Windows-related builds only"). Disabling asks for an optional
reason; a disabled card is greyed with a dashed border, names the reason
and since when, and its chip and the overview count it. Real buttons
(Tab, Enter, Space), 40 px touch targets, wraps at phone width. The
controls need the same token as the answers (asked once, kept in the
browser); without it they ask for it, and a POST without it gets 403.

- **Storage:** `config/build-machines.toml` of the main checkout
  (git-ignored, next to `machines.toml`, which stays untouched), in the
  same `[[machine]]` format, so `scripts/remote.sh` reads it with its
  `field` lookup:

  ```toml
  [[machine]]
  name = "leela"
  enabled = false
  slots = 2
  reason = "excluded by the user, 2026-11-13"
  note = "kiosk only (Tenerife)"
  changed_by = "dashboard"
  changed_at = "2026-11-14T00:08:32+01:00"
  ```

  No file, no entry or no `enabled` key: enabled. `#` in text is written
  as the TOML escape `\u0023` (remote.sh's lookup takes `#` for a
  comment). The file is re-read when it changes, so hand edits and the CLI
  apply at once.
- **API:** `POST /api/machines {"machine", "enabled"?, "slots"?, "reason"?,
  "note"?, "by"?}` with the `X-OCW-Token` header. The machine must be a
  name from `machines.toml` (nothing else reaches a path), `enabled` a
  JSON boolean, `slots` a JSON integer 1–8 (0 = no limit is only possible
  by hand in the file or with `OCW_REMOTE_SLOTS=0`, because an unlimited
  Bevy machine runs out of memory); text loses control characters and is
  cut at 200 characters, and is HTML-escaped on the page. Enabling clears
  the reason. A new entry starts with the machine's current slot count.
- **Log:** every change is a JSON line in
  `test-output/build-log/DAY/machines.jsonl` (time, machine, source
  `dashboard`/`cli`, by, the changed fields as `[old, new]`, the new
  state) for the art project.
- **Status:** each machine in `/api/status` has `enabled`,
  `disabled_reason`, `note`, the effective `slots`, `slots_source`
  (`dashboard`, `machines.toml`, `default`), `slots_config` (what
  machines.toml or the OS default says), `changed_by`, `changed_at`;
  `overview.disabled` counts disabled machines, and `slots_total` leaves
  their slots out. `--once` prints a `build:` line per machine.
- **remote.sh:** `run` and `sync` on a disabled machine fail at once with
  "MACHINE is disabled for building (reason) – enable it in the build
  dashboard", exit code 3, before anything is copied; `ssh`, `field`,
  `fetch`, `put` stay allowed, and so do deploys (`scripts/deploy-test.sh`
  sets `OCW_REMOTE_DEPLOY=1`, so kiosks on disabled machines still get new
  builds). `remote.sh MACHINE enabled` (exit 0 or 3) and `remote.sh
  MACHINE slots` (the effective count) need no ssh; the multiplayer test
  skips disabled bot machines. Slot precedence: `OCW_REMOTE_SLOTS` (one
  call) > `build-machines.toml` > `machines.toml` (machine, then
  `[defaults]`) > OS default. A changed count applies to runs started
  afterwards; a running run keeps its slot, also one above a lowered count
  (it finishes normally; the dashboard shows that slot until it ends).
- **Probes stay:** the dashboard's poll and the janitor still read
  disabled machines (read-only probes, and the janitor's cleanup of
  `target/` when their disk fills up); only builds are refused.
- **CLI:** `python3 tools/build-dashboard/build_machines.py list`, `set
  MACHINE [--enable|--disable] [--slots N] [--reason R] [--note N]
  [--battery-guard STOP:START|off] [--by WHO]` (logged with source `cli`). `--no-machine-settings` turns the page's
  controls off (read-only).

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
`slots` in `build-machines.toml` (set on the page, above), else its `slots` key
in `machines.toml`, else `[defaults] slots`, else the OS default
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
free build slot" and tries all slots again every second, taking whichever
slot frees first in order of arrival: while an older run (by its marker's
start, then PID) still waits, a waiting run leaves the free slot to it
(Linux and macOS, since v2.59; Windows takes the first free slot). A waiting run whose ssh
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
- The only write actions of the page are answering an open question and
  the machine settings (enabled, slots, note; above). Both need
  the token of `config/dashboard-token`; question, answer, reason and note text
  are always HTML-escaped. The poll snippets change nothing on
  the machines (no kills, no files written; stale task markers are cleaned
  up by `remote.sh`, not by the dashboard). The only changes it makes are
  the janitor's deletions of build artefacts (`<checkout>/target` and old
  units in the main checkout's `target/`), off with `--janitor off`, only
  logged with `--janitor dry-run`.
- The coordinator card reads only process names, parents and RSS
  (`/proc/PID/stat`), never command lines or environments; the Claude
  tasks only the task files' mtime and size, never their content. Neither
  shows a token, a host from `machines.toml` or a session id.
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

- **Restart and memory cap:** `Restart=on-failure` (after 10 s), and
  `MemoryHigh=768M` / `MemoryMax=1G` (it needs under 400 MB). This is the
  follow-up of the 2026-11-15 incident, when the OOM killer stopped the
  dashboard while deb ran out of memory (OCW-ART's `build-history/2026-11-15/incidents.md`).
  The OOM kill then took it down, and the systemd restart brought it back.
- **Restart button:** the page's header has a *Restart* button (with the
  token of `config/dashboard-token`, `POST /api/restart`): the dashboard
  replaces its process with a fresh start of the same command line
  (`os.execv`, the same PID, so the unit stays active) – e.g. after pulling
  a new version of the dashboard. Its tooltip says since when it runs; the
  page shows "restarting…" until the new process answers.
- **`machines.toml` is watched:** a change of `config/machines.toml` (checked
  every 2 s, acted on once the content stayed the same for one more check)
  restarts the dashboard with the new machines. A file that lists no
  machines, or is missing, keeps the machines it had and shows an amber
  alert until it is fixed. `build-machines.toml` (the dashboard's own
  settings) is not watched.
- **Changing the unit:** the running unit changes only after a reinstall:
  `make dashboard-install` (it copies the unit again, then daemon-reload and
  restart).
