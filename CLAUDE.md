# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Open Cycling World (OCW) is a cross-platform (Linux/macOS/Windows) indoor cycling simulation in Rust. The repository is currently in the **specification phase**: there is no code, Cargo workspace or build tooling yet, so there are no build/lint/test commands. Add them here once the workspace exists.

## Documents

- `SPECIFICATION.md` – the product and architecture specification (English translation of the original German spec, v1.0, November 2026). It is the source of the requirements.
- `PROPOSALS.md` – proposed decisions for open items found in the spec review, numbered `P-1` … `P-18`. Proposals are **not yet accepted** unless the user says so. When a decision is made, update the proposal (or the spec) rather than leaving the two in conflict.

Reference items by their number (e.g. "spec §6.1", "P-7") in discussions and commits.

## Architecture essentials (from spec + proposals)

These cut across many documents and shape all future code:

- **Server-authoritative simulation (P-1).** The desktop client sends only timestamped sensor inputs (power, cadence, heart rate). The game server runs physics for all riders and is the truth. The client predicts only its own rider and reconciles. Single-player/offline runs the same server logic in-process ("local server"), so there is one code path.
- **Riders move in 1D along the route** (distance along route + lateral offset). 3D positions are derived from road geometry only for rendering.
- **Fixed physics timestep** (proposed 50 Hz), independent of frame rate. State broadcast is proposed at 10 Hz.
- **Engine independence.** Physics/simulation, network protocol, device handling and world data crates must not depend on Bevy. Only the renderer crate and the desktop app do (P-14). This keeps the Godot fallback (P-18) and Bevy API churn manageable.
- **Hardware talks only to the client.** BLE (btleplug, FTMS/Cycling Power/CSC/Heart Rate) runs on its own runtime and talks to the app over channels, behind a `DeviceBackend` trait. ANT+ is **deferred**.
- **World data:** OSM PBF vector data + DEM elevation are converted by `world-builder` on our build infrastructure into signed, versioned world packages (`worlds/<continent>/<country>/<region>/`). The client downloads them once and renders locally. No map tiles/images are used, and nothing is downloaded during a ride (P-15). The road is the elevation reference and the terrain adapts to it (P-6). Geometry uses 1 km tiles with tile-local `f32` coordinates and a floating origin (P-7).
- **Voice is separate** from game state: LiveKit/WebRTC, never over the QUIC game connection. Spatial audio is mixed on the client.
- **Cosmetics are separate from physics.** Visual component data never implies a performance advantage; physics comes from a small curated set of profiles (P-5).
- **Networking/backend stack:** Tokio, Quinn (QUIC; unreliable datagrams for positions), Protobuf via `prost` (P-2), Axum, PostgreSQL, OIDC. The backend starts as a modular monolith.

## Licensing constraints

- Code: dual MIT/Apache-2.0 (proposed).
- OSM data is ODbL: the derived road graph in world packages is a Derivative Database (must be offered under ODbL); meshes/renders need "© OpenStreetMap contributors" attribution. Never use public OSM tile servers at runtime.
- Only openly licensed imagery (Sentinel-2, official open orthophotos, NAIP), never Google/Bing/Esri/Mapbox imagery (P-16). Every third-party asset needs a licence entry in its manifest.

## Repository conventions

- The main branch is `main`.
