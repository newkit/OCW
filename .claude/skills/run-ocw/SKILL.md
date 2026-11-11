---
name: run-ocw
description: Build, run, start, test, screenshot or drive the Open Cycling World apps (ocw-desktop Bevy client, ocw-server headless game server) headlessly on Linux via Xvfb and console commands.
---

# Run Open Cycling World

Paths are relative to the repository root. The desktop app is driven without a
real display: `drive.sh` builds it, runs it under Xvfb with software Vulkan
(Mesa lavapipe), passes console commands with `--cmd`, rides for a few seconds,
saves a screenshot and exits. **Always look at the screenshot.**

## Prerequisites (Debian/Ubuntu)

```bash
sudo apt-get install -y libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-0 libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev xvfb xauth mesa-vulkan-drivers
```

Rust comes from rustup; `rust-toolchain.toml` pins the version (Bevy 0.20 needs
≥ 1.97, Debian's system Rust is too old). If `~/.cargo/bin` is not on `PATH`,
run `source ~/.cargo/env` first (`drive.sh` does this itself).

## Run (agent path)

```bash
# Desktop: ride 8 s, screenshot, exit. Log goes to /tmp/ocw.log.
.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png

# With console commands (run once at startup, in order) and a custom delay:
.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png 10 -- --cmd "teleport 1.2" --cmd "sim.power 350"

# Show the console overlay in the screenshot:
.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png 6 -- --cmd "help" --cmd "console open"

# Headless game server with test riders, simulated as fast as possible:
.claude/skills/run-ocw/drive.sh server --bots 3 --seconds 10 --fast
```

Useful console commands (`help` lists all): `sim.power <W>`, `teleport <km>`,
`camera.mode chase|side|top`, `fps on`, `net.stats`, `rider.state`,
`screenshot <path>`, `console open`, `exec <file>`, `quit`.
Other desktop flags: `--bots N` (default 4), `--power W` (default 200),
`--window 1920x1080`, `--exec FILE`.

`drive.sh shot` prints the console output (`INFO console: …` lines) from the log.

## Test

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world --target wasm32-unknown-unknown
```

Most logic (physics, session, prediction, protocol, server, console parsing) is
unit-tested without Bevy windows; prefer adding tests there over GUI runs.

## Run (human path)

`cargo run -p ocw-desktop` opens a window. PgUp/PgDn change the virtual power,
`` ` `` or F12 opens the console. Useless headless.

## Gotchas

- `--cmd` commands run at **startup** (t ≈ 0), so `rider.state` printed via
  `--cmd` shows distance 0. Read later state from the screenshot's HUD.
- The rider starts from standstill; with constant power the speed builds up
  slowly (≈ 20 km/h after 6 s at 350 W). Use a delay ≥ 8 s for a "riding" shot.
- The debug binary is ~2 GB (debug info); the first build compiles Bevy and
  takes several minutes.
- Without `libxkbcommon-x11-0` the app panics at startup with
  `Library libxkbcommon-x11.so could not be loaded` (only the runtime package is
  needed, not `-dev`).
- ALSA errors and "No audio device found" in the log are harmless under Xvfb.
- Road meshes are built in code: triangles must be counter-clockwise seen from
  above, otherwise the road is culled and invisible (looks like plain grass).
- The default Bevy font has no `·`, `±`, `…`: they render as boxes. Use ASCII in HUD text.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `requires rustc 1.97.1` building Bevy | Use the rustup toolchain (`source ~/.cargo/env`), not `/usr/bin/cargo` |
| `Library libxkbcommon-x11.so could not be loaded` | `sudo apt-get install libxkbcommon-x11-0` |
| `app failed, see …log` from `drive.sh` | Read the log tail it prints; look for `panicked` |
