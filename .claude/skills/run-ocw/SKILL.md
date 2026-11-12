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

Every successful `shot`/`video` run is also archived in the git-ignored
`test-output/YYYY-MM-DD/HHMMSS-<commit>[-dirty]-<mode>-<name>.{png,mp4,log}`,
with one row per run in `test-output/index.md` (date, commit, mode, window
system, app arguments, links). `OCW_TEST_OUTPUT=DIR` changes the folder,
`OCW_TEST_OUTPUT=off` disables it. CI uploads `test-output/` as an artifact.
`OCW_FEATURES=voice` builds the app with extra cargo features (voice needs
`CC=clang-21 CXX=clang++-21` on Linux, spec §7).
Every run shows the FPS counter (top right: the real, wall-clock render rate,
also during `--capture`) and logs `render rate fps=…` every 5 s; `OCW_FPS=0`
turns both off. `OCW_PROFILE=release` runs the release build.

## Prerequisites (Debian/Ubuntu)

```bash
sudo apt-get install -y libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-0 libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev xvfb xauth weston ffmpeg mesa-vulkan-drivers
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

# Record a smooth 25 fps video of 6 s game time (after 3 s riding; also writes
# ride.png and ride.log). Takes ~25 s with software rendering:
.claude/skills/run-ocw/drive.sh video /tmp/ride.mp4 6 -- --power 300 --cmd "teleport 0.9"
# The same on Wayland (headless weston):
.claude/skills/run-ocw/drive.sh video-wayland /tmp/ride-wl.mp4 4 -- --cmd "camera.mode side" --cmd "teleport 2.2"

# Show the console overlay in the screenshot:
.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png 6 -- --cmd "help" --cmd "console open"

# Headless game server with test riders, simulated as fast as possible:
.claude/skills/run-ocw/drive.sh server --bots 3 --seconds 10 --fast
```

Useful console commands (`help` lists all): `sim.power <W>`, `teleport <km>`,
`camera.mode chase|side|top`, `fps on`, `net.stats`, `rider.state`,
`screenshot <path>`, `console open`, `exec <file>`, `quit`.
Other desktop flags: `--bots N` (default 4), `--power W` (default 200),
`--window 1920x1080`, `--fullscreen`, `--exec FILE`, `--capture DIR` (with
`--capture-fps`, `--capture-seconds`, `--capture-warmup`).

Reproducible scenes (camera, teleport, events, menus, tunnel, world package):
`scripts/render-scenes.sh --list | --only NAME | --remote marge` renders
`scripts/scenes.toml` with this driver (`make test-scenes`; README, "Scene harness").

`drive.sh` prints the window system the app used and the console output
(`INFO console: …` lines) from the log. `make test-smoke` runs both X11 and Wayland
(`make test-screenshot`, `test-screenshot-wayland`, `test-video`, `test-video-wayland`
run one mode each).

## Test

`make check` runs all of the following; `make help` lists the other targets.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world --target wasm32-unknown-unknown
```

Most logic (physics, session, prediction, protocol, server, console parsing) is
unit-tested without Bevy windows; prefer adding tests there over GUI runs.
Every test has a `test-*` make target (`test-unit`, `test-wasm`, `test-kiosk`,
`test-smoke`, …). `make test-multiplayer` (`scripts/test-multiplayer.sh`, spec
§11.4.1) runs a real multiplayer session: `ocw-server` and the kiosk app on
marge, headless `ocw-bot` riders on the other build machines, a verdict and
the kiosk's screenshots and video in `test-output/`.

## Run (human path)

`cargo run -p ocw-desktop` opens a window. PgUp/PgDn change the virtual power,
`` ` `` or F12 opens the console. Useless headless.

## Gotchas

- On a machine with a real GPU (marge), the X11 modes (`shot`, `video`) under
  Xvfb panic with "Fallback system failed to choose present mode ... Options: []";
  use `shot-wayland` / `video-wayland` there (via `scripts/remote.sh marge run`).
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
- Videos use frame capture, not screen recording: software rendering runs at
  ~8 FPS, so a screen recorder repeats frames unevenly (judder). With
  `--capture DIR` the app advances game time exactly 1/`--capture-fps` per
  rendered frame and saves every frame; the local server then runs stepped
  in-process (`SteppedServer`) so client and server share that clock. A 6 s
  video takes ~25 s. Check that every frame is distinct:
  `ffmpeg -i ride.mp4 -vf mpdecimate -loglevel debug -f null - 2>&1 | grep -c "keep pts"`
  (should equal the frame count). To look at a video quickly, tile frames:
  `ffmpeg -i ride.mp4 -vf "select='eq(n\,0)+eq(n\,75)+eq(n\,149)',scale=640:-1,tile=3x1" -frames:v 1 sheet.png`.
- Route direction and gradient are interpolated between route points; if
  riders or the camera jerk on curves again, check `Route::sample` first
  (test `direction_and_grade_are_continuous`).
- Under weston, `ERROR sctk_adwaita::config: XDG Settings Portal did not return
  response` is harmless (no desktop portal for the window decoration theme).
- Road meshes are built in code: triangles must be counter-clockwise seen from
  above, otherwise the road is culled and invisible (looks like plain grass).
- Vertex colours (`Mesh::ATTRIBUTE_COLOR`) are linear RGB: convert sRGB values
  with `Color::srgb(..).to_linear()`, otherwise surfaces look washed out.
- The FPS counter shows the real render rate; with software Vulkan expect ~8-10 FPS at
  1280x720, on a headless weston with a GPU (marge) about 40 (weston paces
  vsync; `--no-vsync` shows what the app could draw). Compare against a run of
  the previous commit before blaming a change.
- The default Bevy font has no `·`, `±`, `…`: they render as boxes. Use ASCII in HUD text.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `requires rustc 1.97.1` building Bevy | Use the rustup toolchain (`source ~/.cargo/env`), not `/usr/bin/cargo` |
| `Library libxkbcommon-x11.so could not be loaded` | `sudo apt-get install libxkbcommon-x11-0` |
| `app failed, see …log` from `drive.sh` | Read the log tail it prints; look for `panicked` |
