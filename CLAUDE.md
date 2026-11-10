# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Open Cycling World (OCW) is a cross-platform (Linux/Windows/macOS) indoor cycling simulation in Rust. The repository is currently in the **specification phase**: there is no code, Cargo workspace or build tooling yet, so there are no build/lint/test commands. Add them here once the workspace exists.

## Documents

- `SPECIFICATION.md` (v1.1) – the single source of requirements and architecture decisions. The review proposals P-1 … P-20 were merged into it; Appendix A maps each former P-number to its section.

Reference the spec by section (e.g. "spec §6.1") in discussions and commits. New decisions go directly into the spec (bump the version table at the top) rather than into a separate document.

## Architecture essentials

These cut across many sections and shape all future code:

- **Server-authoritative simulation (§6.1, §5.2).** The desktop client sends only timestamped sensor inputs (power, cadence, heart rate) and steering intents. The game server runs physics for all riders and is the truth. The client predicts only its own rider and reconciles. Single-player/offline runs the same server logic in-process ("local server"), so there is one code path.
- **Riders move in 1D along their path** (distance along route + lateral offset); in free ride the path is extended at each junction. 3D positions are derived from road geometry only for rendering. Fixed 50 Hz physics, 10 Hz state broadcast.
- **Engine independence (§10).** Only `ocw-render` and `apps/desktop` depend on Bevy. `ocw-core`, `ocw-sim`, `ocw-protocol` and `ocw-world` must also build for `wasm32-unknown-unknown` (§3.5) – no direct threads, file system or sockets there.
- **Hardware talks only to the client.** BLE (btleplug, FTMS) runs on its own runtime behind a `DeviceBackend` trait. ANT+ is **deferred**.
- **World data (§4).** OSM PBF vector data + DEM are converted by `world-builder` on our build infrastructure into signed world packages. The client downloads them once and renders locally; no map images, no downloads during a ride. The road is the elevation reference and the terrain adapts to it. 1 km tiles with tile-local `f32` coordinates and a floating origin.
- **Voice is separate** from game state: LiveKit/WebRTC, never over the QUIC game connection.
- **Cosmetics are separate from physics (§2.5).** Physics comes from a small curated set of profiles.
- **Backend:** modular monolith (`services/server`): Axum API, Quinn/QUIC game server, Protobuf via `prost`, PostgreSQL, OIDC; the web companion (Leptos SSR) is a route group in it.

## Platform rollout

Linux first, then Windows, then macOS (§11.1). Linux is the primary dev/test platform; keep platform-specific code behind traits and keep the Windows/macOS build-only CI check and the wasm build green. The web companion comes with Phase 5. A full web client and native mobile apps are future development.

## Licensing constraints

- Code: dual MIT/Apache-2.0.
- OSM data is ODbL: the derived road graph in world packages is a Derivative Database (must be offered under ODbL); meshes/renders need "© OpenStreetMap contributors" attribution. Never use public OSM tile servers at runtime.
- Only openly licensed imagery (§4.6), never Google/Bing/Esri/Mapbox imagery. Every third-party asset needs a licence entry in its manifest.

## Repository conventions

- The main branch is `main`.
