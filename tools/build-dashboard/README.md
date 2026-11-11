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
  build slots in use.
- **Per machine:** online/unreachable (with the error type: timeout,
  connection refused, no route to host, host key mismatch, …), CPU busy %,
  load per core (Windows: threads running or ready to run per core, plus the
  processor queue length when counters are allowed), memory and swap, free
  disk space, uptime.
- **Background tasks:** one row per `remote.sh run` from its task marker
  (below): checkout – `main` (`~/src/OCW`), `agent <id>`
  (`~/src/OCW-worktree-agent-<id>`), a branch name (`~/src/OCW-<branch>`) –,
  the command, running time, state (running in slot N / waiting for a build
  slot), and the CPU (100 % = one core) and memory of the checkout's
  processes, per process kind (cargo, build script, rustc, link, test binary,
  ocw-desktop/ocw-server, ffmpeg, weston, Xvfb). Processes of a checkout
  without a marker (older remote.sh, manual runs) are listed as "no marker".
  The checkout of a process comes from its working directory, its arguments
  or its parent.
- **Queue** (Linux, Windows): which build slots are held and by which
  checkout, and how many `remote.sh run` calls wait for one (Linux: the
  `/tmp/ocw-build-slot-N.lock` flocks; Windows: the markers).

## Task markers

`scripts/remote.sh run` writes a small JSON file on the machine for as long as
the command runs: `/tmp/ocw-tasks/PID-START.json` on Linux and macOS,
`%TEMP%\ocw-tasks\PID-START.json` on Windows (no admin rights needed). It
holds the checkout directory name, branch, command (one line, at most 200
characters), start time (UTC epoch, from the machine's clock), the PID of the
remote shell and the state (`waiting` for a build slot, `running` with the
slot number). The run deletes it on exit (EXIT trap / `finally`). A marker
whose PID is gone, or belongs to a process started after the marker (PID
reused), is stale: the dashboard ignores it (and only counts it), the next
`remote.sh run` on that machine deletes it.

Windows build slots: like Linux, at most `OCW_REMOTE_SLOTS` (default 2) runs
at once; each run opens `%TEMP%\ocw-tasks\slot-N.lock` with `FileShare.None`
and polls every second while all are taken. The OS closes the file when the
run's process dies, so a killed run frees its slot.
- **Kiosk** (machines with `/var/tmp/ocw`): whether the kiosk app runs and
  since when, the deployed build (`VERSION`), connected Bluetooth devices
  (names only).

Platform notes: macOS has no build slots (remote.sh uses none there); its
per-process CPU is ps's decaying `%cpu`. Linux samples per-process CPU from
`/proc/*/stat` over the 0.5 s CPU interval.

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
- It only reads: there are no write actions, and the remote snippets change
  nothing on the machines (no kills, no files written or deleted; stale task
  markers are cleaned up by `remote.sh`, not by the dashboard).
- Task markers hold only the checkout directory name, branch, command, start
  time, PID and state, never hosts, users or keys; the dashboard masks them
  like all other text.
- It reads `config/machines.toml` and the keys in `config/ssh/`, which stay
  git-ignored.
