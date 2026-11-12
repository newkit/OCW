# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Open Cycling World (OCW) is a cross-platform (Linux/Windows/macOS) indoor cycling simulation in Rust. Development is in **Phase 1** (spec §14): Cargo workspace, local client/server architecture, test route, console.

## Commands

Rust comes from rustup; `rust-toolchain.toml` pins the version (Bevy 0.20 needs a newer rustc than Debian ships). If `cargo` resolves to `/usr/bin/cargo`, run `source ~/.cargo/env` first.

`make help` lists the Makefile targets (`deps`, `build`, `test`, `check` = fmt/clippy/tests/wasm, `smoke` = headless screenshot runs on X11 and Wayland, `video`/`video-wayland` = smooth recordings via `ocw-desktop --capture`, a fixed game-time step per frame; all screenshots and videos are archived in the git-ignored `test-output/` with an `index.md`); they work without `~/.cargo/bin` on PATH.

```bash
cargo test --workspace                                   # all unit tests
cargo test -p ocw-sim prediction                         # one crate / test name filter
cargo clippy --workspace --all-targets -- -D warnings    # CI treats warnings as errors
cargo fmt --all
cargo check -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world -p ocw-voice --target wasm32-unknown-unknown
cargo run -p ocw-server -- --bots 3 --seconds 10 --fast  # headless server
```

To run and screenshot the desktop app headlessly (software Vulkan), use the `run-ocw` skill: `.claude/skills/run-ocw/drive.sh shot /tmp/ocw.png 8 -- --cmd "teleport 1.2"` on X11 (Xvfb), or `shot-wayland` on a headless weston. Linux builds support both window systems; keep both smoke runs green. The first build compiles Bevy (several minutes).

## Remote machines

Build/test/benchmark machines (Windows, macOS, Linux with a real GPU) are described in `config/machines.toml`, a git-ignored copy of `config/machines.example.toml`; SSH keys and `known_hosts` live in the git-ignored `config/ssh/`. Connect with `ssh -i config/ssh/<identity_file> -o IdentitiesOnly=yes -o UserKnownHostsFile=config/ssh/known_hosts <user>@<host>`. Never print, copy or commit their contents (host names, keys) outside these files. Setup steps: `config/README.md`.

Test build on marge's screen: `scripts/deploy-test.sh marge` (or `make deploy-test`) installs a release build to `/var/tmp/ocw`; the kiosk (`scripts/ocw-kiosk.sh`, running in the desktop user's session) restarts the app on each deployment. Deploy after user-visible changes so the user can try them.

**All building and testing runs on remote Linux machines, not locally:** `marge` (Debian, AMD GPU, Bluetooth with the Wahoo KICKR, kiosk screen) and `leela` (Fedora, 16 cores, AMD GPU); spread parallel work over both. `remote.sh` lets only a few commands run at once per machine (build slots: 2 per Linux machine, 1 on the Mac, 1 on Windows by default; `slots` in `machines.toml`, `OCW_REMOTE_SLOTS`), because many parallel Bevy builds run a machine out of memory; the others wait for a free slot (see the build dashboard's queue). Platform-independent checks (workspace build, clippy, tests) may also go to the Mac build machine `macos` (M1, 8 cores, 16 GB); Wayland screenshots and the kiosk stay on Linux. `scripts/remote.sh marge run 'cargo test --workspace'` copies the working tree there and runs the command. Long builds: run in the background.

## Code map

- `services/server` (`ocw-server`): `GameServer` is transport-independent (encoded protobuf in/out). `local::LocalServer` runs it on a thread at 50 Hz inside the desktop app; the binary runs it headless. `net` is the QUIC transport (spec §6.2): `NetServer` (pumped by the game loop, `ocw-server --listen`) and `NetClient` (`ocw-desktop --server`), loopback tests in `services/server/tests/`.
- `apps/desktop/src/client.rs` (`ClientCore`): the Bevy-free client loop: device events → `SensorSample` to the server; snapshots → `Predictor::reconcile` (own rider) and `SnapshotBuffer` (others); `view()` for the renderer. Its tests drive it against an in-memory `GameServer`. `connection.rs` is only Bevy glue (channels, `FixedUpdate` prediction at 50 Hz, console commands).
- `crates/ocw-render`: renders only what is in the `RideView` resource; console commands are registered via `ConsoleAppExt::console_command` with a clap definition and a `Permission` level (`Dev` commands need a local server). `ConsoleCorePlugin` runs commands without egui, for headless tests with `MinimalPlugins`.
- `ocw-world::Terrain`: road-conformed height field (spec §4.3 step 4), pure and unit-tested; Phase 1 uses a smoothed road-elevation base, Phase 3 swaps in the DEM. `ocw-render::terrain` turns it into near/middle/far meshes; anything placed in the world (trees, later buildings) takes its height from `Terrain::height`, never from the road.
- Free ride (spec §2.2.1, §5.2): `ocw-world::Track` is what riders ride (a `Route`, or a `RidePath` through the `RoadNetwork` built from the package's road graph – format `OCWPKG02`); `network.rs` has the junction/exit/default rules, `path.rs` the path (lock point, rounded corners, U-turn loop); `ocw-sim::freeride` the shared exit decisions. `ocw-desktop --world data/pilot/<pkg> --free-ride [--start X,Z]`.
- `ocw-devices`: `gatt` parses/encodes the BLE profiles (FTMS, Cycling Power, CSC, Heart Rate, Wahoo control); `ble::protocol` is the pure trainer state machine (data sources, FTMS/Wahoo control, one control write at a time); `ble::runner` drives it over a `GattLink` (btleplug or test mock). Real hardware: `cargo run -p ocw-devices --example ble -- scan | services <id> | monitor <id> | ride <id>` on marge (Wahoo KICKR in range).
- Protocol schema: `proto/ocw/v1/messages.proto`, compiled by `ocw-protocol/build.rs` with protox (no `protoc` needed). Only add fields; never reuse numbers.

## Documents

- `SPECIFICATION.md` (v1.3) – the single source of requirements and architecture decisions. The review proposals P-1 … P-20 were merged into it; Appendix A maps each former P-number to its section.

Reference the spec by section (e.g. "spec §6.1") in discussions and commits. New decisions go directly into the spec (bump the version table at the top) rather than into a separate document.

## Architecture essentials

These cut across many sections and shape all future code:

- **Server-authoritative simulation (§6.1, §5.2).** The desktop client sends only timestamped sensor inputs (power, cadence, heart rate) and steering intents. The game server runs physics for all riders and is the truth. The client predicts only its own rider and reconciles. Single-player/offline runs the same server logic in-process ("local server"), so there is one code path.
- **Riders move in 1D along their path** (distance along route + lateral offset); in free ride the path is extended at each junction. 3D positions are derived from road geometry only for rendering. Fixed 50 Hz physics, 10 Hz state broadcast.
- **Engine independence (§10).** Only `ocw-render` and `apps/desktop` depend on Bevy. `ocw-core`, `ocw-sim`, `ocw-protocol`, `ocw-world` and `ocw-voice` (without its `livekit` feature) must also build for `wasm32-unknown-unknown` (§3.5) – no direct threads, file system or sockets there.
- **Hardware talks only to the client.** BLE (btleplug, FTMS) runs on its own runtime behind a `DeviceBackend` trait. ANT+ is **deferred**.
- **World data (§4).** OSM PBF vector data + DEM are converted by `world-builder` on our build infrastructure into signed world packages. The client downloads them once and renders locally; no map images, no downloads during a ride. The road is the elevation reference and the terrain adapts to it. 1 km tiles with tile-local `f32` coordinates and a floating origin.
- **Voice is separate** from game state: LiveKit/WebRTC, never over the QUIC game connection.
- **Cosmetics are separate from physics (§2.5).** Physics comes from a small curated set of profiles.
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
