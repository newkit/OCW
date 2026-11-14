# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Open Cycling World (OCW) is a cross-platform (Linux/Windows/macOS) indoor cycling simulation in Rust. Phases 1–3 of spec §14 are done, Phases 4–7 partly, Phases 8 (textures) and 9 (sky, time, weather) are in progress, and Phases 10–13 (browser, VR, collisions, god mode) are planned. The spec's version table and the phase sections say what is done and what is open.

## Working mode: keep building (decided by the user, 2026-11-13)

- **When all build jobs are done** (no agent, remote build, render or deploy still running), do not wait idle. **Start implementing the next features from SPECIFICATION.md that are not implemented yet and for which all information is there**: decisions made, no open question blocking them. Prefer the order of the phases in progress (currently Phase 8, then 9), then the planned phases whose questions are answered. Spread the work over agents in worktrees and over all build machines, as usual.
- **Answers from the build dashboard:** the user answers open questions in the dashboard; they land in `test-output/answers/inbox.log` (and `answers.jsonl`). At the start of a session read the inbox for answers newer than the spec's decisions, and keep a watcher on it while working (Monitor: `tail -n 0 -F test-output/answers/inbox.log`, re-armed when it expires); record every answer in the spec right away (mark the question decided, version row) and continue with the work it unblocks.
- **Questions:** when a feature needs an answer from the user before it can go on, **ping the user with a message** (a push notification if available, else a short message in the session) with the concrete questions. Meanwhile continue with other features that are not blocked, never guess a decision the user has to make, and record the answers in the spec when they come.
- Each feature still follows the usual rules: tests (spec §11.4), full checks on the remote machines, merge into `main`, push, deploy for visible changes, test output and art-project documentation.

## Commands

Rust comes from rustup; `rust-toolchain.toml` pins the version (Bevy 0.20 needs a newer rustc than Debian ships). If `cargo` resolves to `/usr/bin/cargo`, run `source ~/.cargo/env` first.

`make help` lists the Makefile targets (`deps`, `build`, `check` = fmt/clippy/tests/wasm, and every test as `test-*`: `test` = `test-unit`, `test-wasm`, `test-kiosk`, `test-smoke` = headless screenshot runs on X11 and Wayland (`test-screenshot`, `test-screenshot-wayland`), `test-video`/`test-video-wayland` = smooth recordings via `ocw-desktop --capture`, a fixed game-time step per frame, `test-multiplayer` = server + kiosk on marge with `ocw-bot` riders on the other build machines over QUIC (spec §11.4.1); the old names `smoke`, `video`, … are deprecated aliases; all screenshots and videos are archived in the git-ignored `test-output/` with an `index.md`); they work without `~/.cargo/bin` on PATH.

```bash
cargo test --workspace                                   # all unit tests
cargo test -p ocw-sim prediction                         # one crate / test name filter
cargo clippy --workspace --all-targets -- -D warnings    # CI treats warnings as errors
cargo fmt --all
cargo check -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world -p ocw-voice -p ocw-sky -p ocw-weather -p ocw-devices --target wasm32-unknown-unknown
make test-web-build                                      # the browser app for wasm32 (part of make check)
make web && make test-web                                # build target/web/, headless Chromium on WebGPU → test-output/web/
cargo run -p ocw-server -- --bots 3 --seconds 10 --fast  # headless server
```

To run and screenshot the desktop app headlessly (software Vulkan), use the `run-ocw` skill: `.claude/skills/run-ocw/drive.sh shot /tmp/ocw.jpg 8 -- --cmd "teleport 1.2"` on X11 (Xvfb), or `shot-wayland` on a headless weston. Linux builds support both window systems; keep both smoke runs green. The first build compiles Bevy (several minutes).

## Remote machines

Build/test/benchmark machines are described in `config/machines.toml`, a git-ignored copy of `config/machines.example.toml`; SSH keys and `known_hosts` live in the git-ignored `config/ssh/`. Connect with `ssh -i config/ssh/<identity_file> -o IdentitiesOnly=yes -o UserKnownHostsFile=config/ssh/known_hosts <user>@<host>`. Never print, copy or commit their contents (host names, addresses, users, keys) outside these files. Setup steps: `config/README.md`.

**All building and testing runs on the remote build machines, never locally on the coordinator (deb).** Roles (decided by the user, 2026-11-13):

| Machine | System | Use it for |
|---|---|---|
| `marge` | Debian, 12 cores, AMD GPU, Bluetooth with the Wahoo KICKR, kiosk screen | The main Linux machine: full checks (fmt, clippy plain and with voice, `cargo test --workspace`, `make test-wasm`), X11/Wayland screenshots and videos, world packages, the kiosk, BLE hardware tests, release AppImages, the multiplayer test's server |
| `leela` | Fedora, 16 cores, 30 GB, AMD GPUs (Radeon 780M + RX 7700S), second kiosk screen | **Excluded from building (user, 2026-11-13): no builds, tests or renders there until the user says otherwise; it only runs its kiosk (deploys stay allowed).** Before that: heavy Linux work in parallel with marge: agent checks, workspace tests, release builds, world builds, renders; its own kiosk (Tenerife). Was often unreachable before 2026-11-13 – skip it when it does not answer |
| `macos` | Mac M1, 8 cores, 16 GB | macOS builds and checks (app bundle, CoreBluetooth/CoreAudio paths) and smaller dev tasks: a crate's tests, clippy, the wasm check |
| `Win11Builder` | Windows 11 | **Only Windows-related builds** (resumed by the user, 2026-11-14): Windows checks of merged code (clippy, the tests of touched crates), Windows-specific fixes (WASAPI, Windows paths, the voice backend's MSVC build), Windows release zips. No general dev tasks, no measurements or long experiments there – its single build slot is slow (`OCW_REMOTE_JOBS=2`, the linker runs out of memory otherwise) and other work queues behind it |

Spread parallel work across marge and the Mac (leela is excluded for now; Win11Builder only for Windows-related builds); a merge is checked fully on marge, the touched crates on the Mac, and the Windows check runs on Win11Builder. `scripts/remote.sh MACHINE run 'CMD'` copies the working tree there and runs the command (Windows: PowerShell syntax); long builds run in the background. `remote.sh` lets only a few commands run at once per machine (build slots: 2 per Linux machine, 1 on the Mac, 1 on Windows; `slots` in `machines.toml`, `OCW_REMOTE_SLOTS`), because many parallel Bevy builds run a machine out of memory; the others wait for a free slot (see the build dashboard's queue). Machines are enabled/disabled for building and their slots set in the build dashboard – the dashboard's state is the source of truth for which machines may build (`config/build-machines.toml`); `remote.sh` refuses `run`/`sync` on a disabled machine (exit 3; deploys stay allowed) – check with `scripts/remote.sh MACHINE enabled`. `remote.sh` snapshots the tree when it is called (Linux/macOS; Windows still copies at once, `OCW_WIN_HANDSHAKE=1` opts in) and syncs by content, so changed files always get a fresh mtime and cargo never keeps a stale build.

`remote.sh` uses `sccache` on every machine (shared by all checkouts there, `make deps-sccache` / `deps-sccache-windows`; `OCW_SCCACHE=0` turns it off); `OCW_SEED=1` (copy-on-write seeding of a new checkout's `target/`) and `OCW_CARGO_SPLIT=1` (own build directory for voice builds) exist but are off – measured without gain. `make remote-gc` lists (`GC=1` deletes) the `target/` of merged agent checkouts. Build profiling: `make build-profile MACHINE=…` (spec §11.2); world builds print a stage profile, `scripts/world-profile.sh` compares packages byte for byte with a reference build (spec §4.7).

**Kiosks:** `scripts/deploy-test.sh marge` (or `make deploy-test`) and `scripts/deploy-test.sh leela` install a release build to `/var/tmp/ocw`; the kiosk (`scripts/ocw-kiosk.sh`, running in the desktop user's session) restarts the app on each deployment, in show mode (virtual trainer at 500 W, `scripts/kiosk-show.args`). Each kiosk keeps its world across deploys (marge: Black Forest north, leela: Tenerife, Puerto de la Cruz – Teide; `OCW_KIOSK_WORLD` changes it). Deploy after user-visible changes so the user can try them; world packages built on marge are copied to leela (identical checksums).

After merging or abandoning an agent branch, run `scripts/remote-gc.sh --branch <branch>` (`make remote-gc-branch BRANCH=…`): it deletes that checkout's `target/` on every reachable build machine (the dashboard's janitor cleans up the rest when a disk fills up). Commit only the files you changed (`git commit FILES`, never `-a`) while agents work in the main checkout.

## Code map

- `apps/bot` (`ocw-bot`): Bevy-free headless bot clients (`NetClient`, `Predictor`, pacing with `--follow`) and `ocw-bot verdict` for the multiplayer test (`scripts/test-multiplayer.sh`).
- `services/server` (`ocw-server`): `GameServer` is transport-independent (encoded protobuf in/out). `local::LocalServer` runs it on a thread at 50 Hz inside the desktop app; the binary runs it headless. `net` is the QUIC transport (spec §6.2): `NetServer` (pumped by the game loop, `ocw-server --listen`) and `NetClient` (`ocw-desktop --server`), loopback tests in `services/server/tests/`.
- `crates/ocw-client` (shared by desktop and web, wasm-safe; uses only `ocw-render`'s view types): `client.rs` (`ClientCore`), events, `god_state`, the pure key logic (`input`: `KeyThrottle`, lean / line steering), `gps`, `sky`. `client.rs` (`ClientCore`) is the Bevy-free client loop: device events → `SensorSample` to the server; snapshots → `Predictor::reconcile` (own rider) and `SnapshotBuffer` (others); `view()` for the renderer. Its tests drive it against an in-memory `GameServer`. The desktop's `apps/desktop/src/connection.rs` is only Bevy glue (channels, `FixedUpdate` prediction at 50 Hz, console commands).
- `apps/web` (`ocw-web`, spec §14 Phase 10): the same client in the browser (wasm32, Bevy on WebGPU): `session.rs` (`WebSession`: `ocw_server::local::FrameServer` – the `GameServer` stepped from frame times, no threads – plus `ClientCore` and `ocw_devices::FrameVirtualTrainer`), `plugin.rs` the Bevy glue; `web/index.html`. Desktop-only parts (BLE, cpal, QUIC, files, voice, captures) stay in `apps/desktop` and move into shared crates behind web backends (B2–B6); keep both apps one client (full parity).
- `crates/ocw-render`: renders only what is in the `RideView` resource; console commands are registered via `ConsoleAppExt::console_command` with a clap definition and a `Permission` level (`Dev` commands need a local server). `ConsoleCorePlugin` runs commands without egui, for headless tests with `MinimalPlugins`.
- `ocw-world::Terrain`: road-conformed height field (spec §4.3 step 4), pure and unit-tested; Phase 1 uses a smoothed road-elevation base, Phase 3 swaps in the DEM. `ocw-render::terrain` turns it into near/middle/far meshes; anything placed in the world (trees, later buildings) takes its height from `Terrain::height`, never from the road.
- Free ride (spec §2.2.1, §5.2): `ocw-world::Track` is what riders ride (a `Route`, or a `RidePath` through the `RoadNetwork` built from the package's road graph – format `OCWPKG02`); `network.rs` has the junction/exit/default rules, `path.rs` the path (lock point, rounded corners, U-turn loop); `ocw-sim::freeride` the shared exit decisions. `ocw-desktop --world data/pilot/<pkg> --free-ride [--start X,Z]`.
- `ocw-devices`: `gatt` parses/encodes the BLE profiles (FTMS, Cycling Power, CSC, Heart Rate, Wahoo control); `ble::protocol` is the pure trainer state machine (data sources, FTMS/Wahoo control, one control write at a time); `ble::runner` drives it over a `GattLink` (btleplug or test mock). Real hardware: `cargo run -p ocw-devices --example ble -- scan | services <id> | monitor <id> | ride <id>` on marge (Wahoo KICKR in range).
- `ocw-sky` (Phase 9, pure, wasm): time zones (jiff, bundled tzdb), NOAA sun position and events, `WorldClock` (owned by `GameServer`, in `Welcome`/snapshots, `ClockPredictor` on the client), time-machine commands/presets, the light model. `ocw-render::sky` draws it (sun, atmosphere, exposure, bike lights); `apps/desktop/src/timemachine.rs` has the zone of the place, the `time` command and World → Time (the time machine).
- `ocw-weather` (Phase 9 S4, pure, wasm): `WeatherState` and transitions, presets, `SessionWeather` (owned by `GameServer` like the clock: `Welcome.weather` / `Snapshot.weather`, `WeatherRequest`, `weather_access`), the seeded simulation, road wetness / puddles / snow cover, gusts, lightning and thunder, particle and visibility maths, cloud light; `WeatherFollower` / `ocw_client::weather` on the client. `ocw-render::weather` draws it (cloud dome, precipitation, fog, wet roads, snow, tree sway, spray, bolts; the light in `sky.rs`); `apps/desktop/src/weather.rs` has the `weather` command and World → Weather, `audio/weather.rs` the rain, hail, wind and thunder.
- World menu (spec §3.2.1 *World menu*): `ocw_worldstore::discover` finds the worlds (cache, `--world` and its siblings, data dirs; `ocw_world::summary` reads a package without loading it), `apps/desktop/src/ride/world.rs` has the World pages and `world list|info|switch`; in shared sessions the switch is a `WorldRequest` the server checks (host only) and answers with `WorldChange` (`GameServer::switch_world`, `ocw_server::worlds`).
- Protocol schema: `proto/ocw/v1/messages.proto`, compiled by `ocw-protocol/build.rs` with protox (no `protoc` needed). Only add fields; never reuse numbers.

## Documents

- `SPECIFICATION.md` (v1.3) – the single source of requirements and architecture decisions. The review proposals P-1 … P-20 were merged into it; Appendix A maps each former P-number to its section.

- `art/ART-PROJECT.md` – the art project: the **OCW Build Diary**, a video per build day plus a growing diary video, made from *every* screenshot, video, sound, log, commit, merge and test of the day (`art/diary.py`, `make art-*`; day scripts in `art/days/`). Keep all media of test runs in `test-output/`; never delete build-history material. At the end of a build day: gather, write the day script, render, publish.

Reference the spec by section (e.g. "spec §6.1") in discussions and commits. New decisions go directly into the spec (bump the version table at the top) rather than into a separate document.

## Architecture essentials

These cut across many sections and shape all future code:

- **Server-authoritative simulation (§6.1, §5.2).** The desktop client sends only timestamped sensor inputs (power, cadence, heart rate) and steering intents. The game server runs physics for all riders and is the truth. The client predicts only its own rider and reconciles. Single-player/offline runs the same server logic in-process ("local server"), so there is one code path.
- **Riders move in 1D along their path** (distance along route + lateral offset); in free ride the path is extended at each junction. 3D positions are derived from road geometry only for rendering. Fixed 50 Hz physics, 10 Hz state broadcast.
- **Engine independence (§10).** Only `ocw-render`, `ocw-client` (view types only) and the apps `apps/desktop` / `apps/web` depend on Bevy. `ocw-core`, `ocw-sim`, `ocw-protocol`, `ocw-world`, `ocw-sky`, `ocw-weather`, `ocw-voice` (without its `livekit` feature), `ocw-devices` (BLE and the threaded trainer are native only), the `ocw-server` lib (QUIC, `worlds` and the server thread native only) and `ocw-client` must also build for `wasm32-unknown-unknown` (§3.5) – no direct threads, file system or sockets there, and no `std::time::{Instant, SystemTime}::now()` (it panics: use `web-time` / `bevy::platform::time`).
- **Hardware talks only to the client.** BLE (btleplug, FTMS) runs on its own runtime behind a `DeviceBackend` trait. ANT+ is **deferred**.
- **World data (§4).** OSM PBF vector data + DEM are converted by `world-builder` on our build infrastructure into signed world packages. The client downloads them once and renders locally; no map images, no downloads during a ride. The road is the elevation reference and the terrain adapts to it. 1 km tiles with tile-local `f32` coordinates and a floating origin.
- **Voice is separate** from game state: LiveKit/WebRTC, never over the QUIC game connection.
- **Physics comes from curated profiles (§2.5), changed only a little by parts and kit within fixed caps (§14 Phase 4, 4G / 4C).** Looks (paint, colours, body, hair, glasses) never change the physics; in *Profile* and *Equal* race modes parts and kit are cosmetic too.
- **Backend:** modular monolith (`services/server`): Axum API, Quinn/QUIC game server, Protobuf via `prost`, PostgreSQL, OIDC; the web companion (Leptos SSR) is a route group in it.

## Testing

No step is done without unit tests (spec §11.4, per-phase test lists in §14). Tests run without hardware, network or GPU: use the virtual trainer / mock `DeviceBackend`, in-process channels instead of sockets, and keep logic outside Bevy so it can be tested headless. Every bug fix gets a regression test.

## Platform rollout

Linux first, then Windows, then macOS (§11.1). Linux is the primary dev/test platform; keep platform-specific code behind traits and keep the Windows/macOS build-only CI check and the wasm build green. The web companion comes with Phase 5. A full web client and native mobile apps are future development.

## Licensing constraints

- Code: dual MIT/Apache-2.0.
- OSM data is ODbL: the derived road graph in world packages is a Derivative Database (must be offered under ODbL); meshes/renders need "© OpenStreetMap contributors" attribution. Never use public OSM tile servers at runtime.
- Only openly licensed imagery (§4.6), never Google/Bing/Esri/Mapbox imagery. Every third-party asset needs a licence entry in its manifest.

## Repository conventions

- The main branch is `main`.
