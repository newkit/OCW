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
  load per core, memory and swap, free disk space, uptime.
- **Tasks**, grouped by checkout: `main` (`~/src/OCW`), `agent <id>`
  (`~/src/OCW-worktree-agent-<id>`), a branch name (`~/src/OCW-<branch>`),
  with the command given to `remote.sh run`, and per process kind (cargo,
  build script, rustc, link, test binary, ocw-desktop/ocw-server, ffmpeg,
  weston, Xvfb) the count, elapsed time and memory. The checkout comes from
  the process's working directory, its arguments or its parent.
- **Queue** (Linux): which `/tmp/ocw-build-slot-N.lock` slots are held and by
  which checkout, and how many `remote.sh run` calls wait in `flock`.
- **Kiosk** (machines with `/var/tmp/ocw`): whether the kiosk app runs and
  since when, the deployed build (`VERSION`), connected Bluetooth devices
  (names only).

Platform notes: macOS has no build slots (remote.sh uses none there).
Windows is read with `Get-Process` and .NET only, because CIM/WMI, the
registry's Perflib and performance counters are denied to a non-admin build
user: CPU busy % is shown only if the user is in "Performance Monitor Users",
and processes cannot be assigned to a checkout unless they run from its
`target\` directory (process command lines and parents need CIM).

## Security

- The server listens on all addresses by default, so it is reachable from
  the LAN: anyone there can see machine load and the build processes. Use
  `--host 127.0.0.1` to keep it local.
- It never shows host names, IP addresses, user names or keys: machines are
  identified by their config `name`, and all text from the machines or from
  ssh is masked (addresses, `user@host`, home directories, MAC addresses)
  before it is stored or served.
- It only reads: there are no write actions, and the remote snippets change
  nothing on the machines (no kills, no files written).
- It reads `config/machines.toml` and the keys in `config/ssh/`, which stay
  git-ignored.
