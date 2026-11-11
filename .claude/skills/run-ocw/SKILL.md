---
name: run-ocw
description: Build, run, start, test, screenshot or drive the Open Cycling World apps (ocw-desktop Bevy client, ocw-server headless game server) headlessly on Linux via Xvfb (X11) or headless weston (Wayland) and console commands.
---

# Run Open Cycling World

Paths are relative to the repository root. The desktop app is driven without a
real display: `drive.sh` builds it, runs it under Xvfb (X11) or a headless
weston compositor (Wayland) with software Vulkan (Mesa lavapipe), passes console
commands with `--cmd`, rides for a few seconds, saves a screenshot and exits.
It fails if the app reports a different window system than requested.
**Always look at the screenshot**, and send screenshots and videos to the user
(SendUserFile) – they want to follow visual test output.

## Prerequisites (Debian/Ubuntu)

```bash
sudo apt-get install -y libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-0 libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev xvfb xauth weston cage wf-recorder ffmpeg mesa-vulkan-drivers
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

# The same on Wayland (headless weston) instead of X11:
.claude/skills/run-ocw/drive.sh shot-wayland /tmp/ocw-wl.png 8 -- --cmd "teleport 2.5" --cmd "camera.mode side"

# Record a short video (6 s after a 4 s warm-up; also writes ride.png and ride.log):
.claude/skills/run-ocw/drive.sh video /tmp/ride.mp4 6 -- --power 300 --cmd "teleport 1.0"
# The same on Wayland (headless cage + wf-recorder; app runs --fullscreen):
.claude/skills/run-ocw/drive.sh video-wayland /tmp/ride-wl.mp4 6 -- --power 300 --cmd "teleport 1.0"

# Show the console overlay in the screenshot:
.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png 6 -- --cmd "help" --cmd "console open"

# Headless game server with test riders, simulated as fast as possible:
.claude/skills/run-ocw/drive.sh server --bots 3 --seconds 10 --fast
```

Useful console commands (`help` lists all): `sim.power <W>`, `teleport <km>`,
`camera.mode chase|side|top`, `fps on`, `net.stats`, `rider.state`,
`screenshot <path>`, `console open`, `exec <file>`, `quit`.
Other desktop flags: `--bots N` (default 4), `--power W` (default 200),
`--window 1920x1080`, `--fullscreen`, `--exec FILE`.

`drive.sh` prints the window system the app used and the console output
(`INFO console: …` lines) from the log. `make smoke` runs both X11 and Wayland.

## Test

`make check` runs all of the following; `make help` lists the other targets.

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
- On Linux, winit picks Wayland if `WAYLAND_DISPLAY` is set, else X11; the app
  logs `window created window_system="wayland"|"x11"`. `drive.sh shot` unsets
  `WAYLAND_DISPLAY`, `shot-wayland` unsets `DISPLAY` so neither can fall back.
- Videos: record only after the window exists plus a few seconds; the first
  second shows shader compilation (white ground, brown trees, objects missing).
  `drive.sh video` waits for the `window created` log line. Software rendering
  gives roughly 20 fps; `-draw_mouse 0` keeps the X cursor out of the video.
  To check a video quickly, tile frames:
  `ffmpeg -i ride.mp4 -vf "select='eq(n\,0)+eq(n\,60)+eq(n\,119)',scale=640:-1,tile=3x1" -frames:v 1 sheet.png`.
- Wayland screenshots use weston, Wayland videos use cage: weston has no
  screen-capture protocol for external recorders, wf-recorder needs wlroots
  (`wlr-screencopy`). Two compositor implementations also widen coverage.
- In cage, a windowed app leaves a black strip at the bottom (winit reserves
  room for client-side title-bar decorations); `video-wayland` therefore passes
  `--fullscreen`. cage's `[ERROR] ... configure is scheduled for an
  uninitialized xdg_surface` lines at startup are harmless.
- Under weston, `ERROR sctk_adwaita::config: XDG Settings Portal did not return
  response` is harmless (no desktop portal for the window decoration theme).
- Road meshes are built in code: triangles must be counter-clockwise seen from
  above, otherwise the road is culled and invisible (looks like plain grass).
- Vertex colours (`Mesh::ATTRIBUTE_COLOR`) are linear RGB: convert sRGB values
  with `Color::srgb(..).to_linear()`, otherwise surfaces look washed out.
- `fps on` shows the frame rate; with software Vulkan expect ~8-10 FPS at
  1280x720. Compare against a run of the previous commit before blaming a change.
- The default Bevy font has no `·`, `±`, `…`: they render as boxes. Use ASCII in HUD text.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `requires rustc 1.97.1` building Bevy | Use the rustup toolchain (`source ~/.cargo/env`), not `/usr/bin/cargo` |
| `Library libxkbcommon-x11.so could not be loaded` | `sudo apt-get install libxkbcommon-x11-0` |
| `app failed, see …log` from `drive.sh` | Read the log tail it prints; look for `panicked` |
