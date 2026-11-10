# Open Cycling World – Proposals for Open Items

Companion to [SPECIFICATION.md](SPECIFICATION.md). Each item names an open question from the spec review and proposes a decision. Everything here is a proposal until accepted.

> **Deferred:** ANT+ support is out of scope for now. The device abstraction (P-11) is designed so that an ANT+ backend can be added later without changes to other crates.

---

## P-1 Simulation Authority and Shared Physics

**Problem:** §6.1 says the server "computes *or* validates" rider state; client prediction requires the same physics on both sides.

**Roles:**

| Role | What it is | Where it runs | Physics responsibility |
|---|---|---|---|
| **Client** | The desktop app (`apps/desktop`): rendering, sensors, UI, audio | The user's PC (Linux / macOS / Windows) | Runs `ocw-sim` for its **own rider only** as a *prediction*, so the bike reacts instantly to pedaling |
| **Remote server** | Our game server (`services/server`) | Our Linux hosts (Docker) | Runs `ocw-sim` for **all riders in a session**; its result is the truth (positions, drafting, race results) |
| **Local server** | The same server logic, linked into the desktop app | Inside the client process | Used for single-player / offline rides: it is the authority, no network involved |

The trainer and sensors are always connected to the **client** only. The server never talks to hardware; it receives the measured values from the client.

**Proposal:**

- The **server is authoritative** and runs the simulation. The client sends only *inputs*: timestamped power, cadence and heart-rate samples with sequence numbers.
- A single crate `ocw-sim` (no Bevy dependency) contains physics, rider state and drafting. It is used by the client (prediction), the server (authority) and the offline mode.
- **Riders move in 1D along the route:** state = `distance_along_route: f64`, `lateral_offset: f32`, `speed: f64`. 3D position is derived from the road geometry only for rendering. This makes physics cheap, robust and easy to synchronize.
- **Fixed timestep:** physics at 50 Hz (20 ms) on client and server, independent of frame rate.
- **No strict bit-level determinism:** use `f64`. The client predicts, receives authoritative state, and corrects small errors by smoothing (blending over ~0.5 s); it snaps only above a threshold (e.g. > 5 m). This avoids the cost of fixed-point math.
- **Drafting** (reduced CdA behind other riders) is computed server-side because it depends on other riders. The client predicts it from the interpolated positions of nearby riders.
- **Offline / single-player** runs the same server logic **in-process** ("local server"). There is only one code path.

---

## P-2 Message Format

**Proposal:** **Protobuf** only, via `prost`.

- Schemas live in `proto/` and are versioned (`ocw.v1`). Changes follow protobuf compatibility rules: add fields, never reuse field numbers.
- Position datagrams use compact quantized fields: distance in cm (`uint32`), speed in mm/s (`uint32`), lateral offset in cm (`sint32`), animation state (`uint32` enum).
- A protocol version is exchanged during the handshake; incompatible clients are rejected with an update prompt.

---

## P-3 Revised Phase Plan

**Problem:** Multiplayer is called core to the architecture but only arrives in Phase 5.

**Proposal:**

| Phase | Change |
|---|---|
| 1 – Foundation | Add `ocw-sim`, `ocw-protocol` and a **headless server skeleton**. The desktop app talks to the in-process local server through the protocol from day one. |
| 2 – Bike & Sensors | Unchanged; physics lands in `ocw-sim`. |
| 3 – OSM World | Add the route graph and route format (P-13). |
| 4 – Avatar & Editor | Unchanged. |
| 5 – Multiplayer | Becomes "networked server": the existing local-server logic runs remotely, plus accounts, sessions and leaderboards. |
| 6 – Voice | Unchanged. |
| 7 – Production | Unchanged. |

Phase 5 then means running existing code over a network rather than retrofitting it.

---

## P-4 Data Integrity / Anti-Cheat

**Problem:** Power comes from user hardware and cannot be fully verified.

**Proposal:** Reword the goal from "protection against manipulated power data" to "**detection and flagging of implausible data**", with these layers:

1. **Transport integrity:** authenticated QUIC (TLS 1.3), per-session tokens, sequence numbers and server timestamps (prevents replay and injection).
2. **Plausibility checks (server):**
   - Power-duration curve compared with the rider's history and with absolute W/kg limits.
   - Unrealistic spikes and slopes (power changes faster than physically possible).
   - Power without cadence, or perfectly constant power over long periods.
   - Heart rate not responding to load changes (when heart rate is present).
3. **Device trust levels:** smart trainer only < trainer + independent power meter ("dual recording") < verified setup. Organized races can require a minimum trust level.
4. **Weight:** the biggest real-world manipulation vector. Weight changes are logged; races may lock weight some time before the start.
5. **Consequences:** flagging, results marked "unverified", and manual review. No automatic bans in v1.

---

## P-5 Cosmetics vs. Performance

**Proposal:**

- Each component has a `visual` part (mesh, materials, paint slots) and an optional reference to a **physics profile**.
- Only a **small, curated set of physics profiles** exists per bike type (e.g. "road – aero", "road – climbing", "road – standard"). Cosmetic variants share profiles.
- Unlocks and paint schemes are **always cosmetic**.
- Differences between profiles are capped (e.g. at most ±3 % speed at 250 W on flat road).
- **Race modes:**
  - *Open* – the chosen profile applies.
  - *Equal* – standardized physics for everyone, chosen equipment shown only visually.

---

## P-6 Elevation Quality

**Proposal:** In the world builder, **the road is the source of truth and the terrain adapts to it.**

1. **DEM sources:** prefer regional high-resolution open data (e.g. 1 m DGM/LiDAR where available) and fall back to Copernicus GLO-30.
2. **Sampling:** sample the DEM along each road centerline every 5 m with bilinear interpolation.
3. **Smoothing:** apply a median filter (removes spikes), then a constrained spline fit with:
   - a maximum gradient per road category (e.g. 25 % for paths, 15 % for main roads),
   - a maximum change in gradient per meter.
4. **Use OSM tags:** `bridge`, `tunnel`, `layer`, `incline`, `embankment`, `cutting`. Bridges and tunnels are interpolated linearly or with a smooth curve between their abutments or portals, ignoring the terrain.
5. **Terrain conformance:** the terrain mesh is flattened and blended along a road corridor (road width + shoulder) so roads never float or sink.
6. **Validation:** the world builder reports suspicious segments (gradient jumps, extreme climbs) for review.

Smoothing toward the trainer is covered in P-8.

---

## P-7 Coordinate System and Tiling

**Proposal:**

- **World builder:** all computation in `f64`, projected per region into a **local transverse Mercator projection** (or the UTM zone) with metres as units.
- **Tiles:** 1 km × 1 km world tiles. Geometry is stored in `f32` **relative to the tile origin**, so precision stays well below 1 mm.
- **Runtime:** a **floating origin**. When the camera is more than ~2 km from the render origin, the origin is shifted and all entities are re-offset. Evaluate the `big_space` crate as an alternative.
- **Physics** is unaffected because it runs in route distance (`f64`, P-1).
- **Streaming:** keep a 3 × 3 tile ring at full detail, with lower LOD rings beyond. Load and generate tiles on background tasks.

---

## P-8 Trainer Control

**Proposal:**

- **FTMS control point operations:**
  - *Set Indoor Bike Simulation Parameters* (grade, wind speed, Crr, Cw) for SIM mode.
  - *Set Target Power* for ERG mode (workouts).
  - *Set Target Resistance Level* as a fallback.
  - *Spin-down calibration* where supported.
- **Grade sent to the trainer:**
  - **Trainer difficulty** slider from 0–100 %, default 50 %, which scales the grade.
  - Separate downhill handling (e.g. negative grades capped or scaled further).
  - Rate limit (e.g. at most 1 %/s change) and a short lookahead to avoid jolts.
  - Updates sent at 1–2 Hz; most trainers do not handle faster updates well.
- **Workouts:**
  - Our own open workout format (TOML/JSON: intervals, targets as % FTP, cadence hints).
  - Import of the open formats `.erg`/`.mrc` and FIT workout files.
  - ERG with smoothing and a "catch-up" on sudden cadence drops.
- Workouts and ERG control are added to Phase 2 (basic) and Phase 7 (full).

---

## P-9 Licensing

**Proposal:**

| Item | Classification | Licence / Obligation |
|---|---|---|
| Source code | Own work | **Dual MIT / Apache-2.0** (Rust ecosystem convention) |
| Own 3D assets, sounds | Own work | Decide separately; suggestion **CC BY 4.0**, or proprietary if monetized |
| Third-party assets | External | Tracked in each asset's manifest (`license`, `source`, `author`); the `asset-validator` rejects assets without a licence entry |
| Road graph + attributes in world packages | **Derivative Database** (ODbL) | Published under ODbL; extracted data offered for download alongside each package |
| Meshes, textures, rendered views | **Produced Work** (ODbL) | Attribution "© OpenStreetMap contributors" |
| DEM-derived terrain | Depends on source | Include each source's required attribution notice (Copernicus, regional providers) |

- **Attribution in the app:** loading screen, about / credits screen, and the in-ride minimap.
- **No use of `tile.openstreetmap.org`** or other public OSM services at runtime. Data comes from Geofabrik extracts at build time; packages are served from our own world-distribution service.
- Have the classification reviewed by a lawyer before commercial release (as already stated in the spec).

---

## P-10 Voice Chat Integration

**Proposal:**

- Use the official **LiveKit Rust SDK**. It wraps libwebrtc, but LiveKit provides prebuilt binaries for all target platforms, which removes the hardest build problem.
- Isolate it in the `ocw-voice` crate behind a trait, with a cargo feature `voice`, so the app also builds and runs without it (CI, minimal builds).
- **Self-host the LiveKit server** (Apache 2.0) as a separate service. Our API issues short-lived LiveKit access tokens (JWT) per room.
- **Spatial audio:**
  - Subscribe only to the nearest **N riders (e.g. 8–12)** plus explicitly chosen group members.
  - Attenuate volume by distance along the route (and optionally pan by lateral offset).
  - Mix on the client.
- Echo cancellation and noise suppression come from the WebRTC audio processing module included in libwebrtc.
- Voice is **never recorded or stored** on our side (see P-12).

---

## P-11 Platform Topics

**Bevy versioning:**

- Pin an exact Bevy version in the workspace.
- Upgrade in dedicated milestones, typically one release behind current, once the ecosystem crates we use have caught up.

**UI:**

- **Bevy UI** for the in-ride HUD and main menus (as specified).
- **egui** (`bevy_egui`) for the editor parameter panels and diagnostics from the start, to keep development fast.
- Review the editors after Phase 4.

**Bluetooth (btleplug):**

- `ocw-devices` runs on its own Tokio runtime/thread and talks to the app over channels. It has no Bevy dependency (spec §3.4).
- It defines a `DeviceBackend` trait. BLE is the first implementation; ANT+ (deferred) will be the second.
- Per-platform requirements (implemented in the rollout order Linux → Windows → macOS, spec §11):
  - macOS: `NSBluetoothAlwaysUsageDescription` in `Info.plist` plus the matching entitlement.
  - Linux: BlueZ via D-Bus; document the minimum BlueZ version.
  - Windows: WinRT BLE APIs (Windows 10 or later).
- Remember paired devices and reconnect automatically.

---

## P-12 Non-Functional Requirements

### Multiplayer capacity and timing

| Topic | Proposal |
|---|---|
| Riders per session (v1) | 100 |
| Design target | 1,000 per event through **interest management**: full-rate updates for riders within ±500 m, reduced rate beyond, aggregated (count / gap) for the rest |
| Server physics tick | 50 Hz |
| State broadcast | 10 Hz (enough for cycling with interpolation) |
| Interpolation delay | ~100–150 ms |
| Playable latency | up to 250 ms RTT |
| Downstream bandwidth | < 64 kbit/s per client in a 100-rider session |
| Reconnect | Session state kept on the server for 2 min; the rider resumes at the server-side position |

### Client hardware and resources

| Topic | Proposal |
|---|---|
| Minimum hardware | 4-core CPU, 8 GB RAM, GPU with Vulkan 1.2 / Metal / DX12 and 2 GB VRAM |
| Memory budget | ≤ 2 GB RAM and ≤ 1.5 GB VRAM at default settings |
| Disk per region | Target 200–500 MB for a mid-sized region |

### Privacy and data handling

| Topic | Proposal |
|---|---|
| Data separation | Identity (account schema), training data (separate schema / service module), voice (only relayed, never stored), telemetry (opt-in, pseudonymized) |
| GDPR | Data export (FIT + JSON), account deletion including training data, hosting in the EU |

### Package signing

- **Ed25519** signatures.
- An offline root key signs rotating online signing keys.
- Each package manifest lists SHA-256 hashes of all files.
- The client ships the root public key.
- Evaluate **TUF** (The Update Framework) for package and app updates.

---

## P-13 Missing Features

| Feature | Proposal | Phase |
|---|---|---|
| **Route format** | A route is an ordered list of road-graph edges with direction, start/finish markers, lap count and optional segments (sprints, KOMs). Stored as TOML/JSON in the world package. | 3 |
| **Stable edge IDs** | Derived from OSM way ID + node range plus a geometry hash. When a package is rebuilt from newer OSM data, routes are re-matched automatically and the world builder flags any that break. | 3 |
| **GPX import** | See P-13a. | 3 |
| **Route editor** | In `world-builder`: click edges together, set laps and segments. | 3 |
| **FIT export** | Write FIT activity files (a FIT writer is needed; existing Rust crates mainly read). Basis for Strava upload. | 2 |
| **Strava upload** | See P-13b. Other services (intervals.icu, TrainingPeaks) later. | 5 |
| **Drafting** | Part of `ocw-sim` (see P-1). | 2 / 5 |
| **Structured workouts / ERG** | See P-8. | 2 / 7 |
| **In-game console** | See P-17. | 1 |

### P-13a GPX Import for Tracks

The user imports a GPX file (e.g. a real ride or a planned route) and rides it as a route.

1. **Parse** the GPX track (`gpx` crate); accept tracks and routes, and join multiple segments.
2. **Region check:** compute the bounding box. If no installed world package covers it, offer to download the matching package(s).
3. **Map matching:** snap the GPS points onto the road graph using an HMM-based matcher (the standard Viterbi approach used by OSRM / Valhalla), implemented in `ocw-world`. The result is the ordinary route format (edge list).
4. **Elevation from our DEM, not from the GPX:** GPS elevation is noisy, so the route uses the already smoothed road profile (P-6). The GPX elevation is only shown for comparison.
5. **Problems are reported, not hidden:** sections more than ~30 m from any road, or on roads missing from the package, are highlighted. The user can accept a detour or cancel.
6. **Result:** a personal route stored locally. It can optionally be shared with friends or used for a group ride; the server receives only the edge list, not the GPX.

GPX **export** of ridden routes is also offered (in addition to FIT).

### P-13b Strava Upload for Finished Rides

- **Account linking via our backend:** Strava's OAuth flow requires a client secret, which must not ship in the desktop app. The backend handles the OAuth exchange and stores the refresh tokens encrypted.
- **Upload flow:**
  1. When a ride ends, the client writes a FIT file (power, cadence, heart rate, speed, distance, elevation, virtual GPS track).
  2. The client sends it to our API, which uploads it via the Strava `uploads` endpoint as `VirtualRide`.
  3. The API polls Strava for the upload status and reports the result to the client.
- **Options:** automatic upload after each ride, or a manual "Upload" button in the training history. Title and description can be edited, and a default title is generated (route name, world).
- **Offline:** if offline, uploads are queued and sent later.
- **Constraints:**
  - Strava API terms and rate limits apply.
  - Beyond a small number of athletes, Strava requires an app review. Apply early.
  - The virtual GPS track uses real coordinates of the route. This is common practice for virtual rides, but it must be labelled as virtual.

---

## P-14 Lean Initial Repository Layout

**Proposal:** Start with about 8 crates and split them only when they grow. The **dependency rule** is that only `ocw-render` and `apps/desktop` depend on Bevy.

```
open-cycling-world/
├── apps/
│   ├── desktop/          # Bevy app, wires all plugins together
│   ├── world-builder/    # CLI: OSM + DEM → world packages, route editor
│   └── web-companion/    # browser app without riding (P-20)
├── services/
│   └── server/           # modular monolith: Axum API + QUIC game server
├── crates/
│   ├── ocw-core/         # shared types, units, IDs, config
│   ├── ocw-sim/          # physics, rider state, drafting, session logic (no Bevy)
│   ├── ocw-protocol/     # prost-generated messages + framing (no Bevy)
│   ├── ocw-devices/      # DeviceBackend trait, BLE/FTMS backend (no Bevy)
│   ├── ocw-world/        # package format, tiles, road graph, routes (no Bevy)
│   ├── ocw-worldgen/     # OSM/DEM import, smoothing, meshing (used by world-builder)
│   ├── ocw-render/       # Bevy plugins: terrain, roads, avatars, bikes, HUD
│   └── ocw-voice/        # LiveKit integration behind a trait (feature "voice")
├── proto/                # .proto schemas
├── assets/
├── tools/
├── infrastructure/
└── .github/workflows/
```

Mapping from the original layout:

| Original crate(s) | New crate |
|---|---|
| `cycling-protocol`, `multiplayer` (protocol part) | `ocw-protocol` |
| `cycling-physics`, `simulation`, `training` (sim part) | `ocw-sim` |
| `osm-import`, `terrain`, `road-generator` | `ocw-worldgen` |
| `avatar`, `bicycle`, `*-editor` | modules within `ocw-render` and `apps/desktop` |
| `ant-plus` | deferred; later a second backend in `ocw-devices` |


---

## P-15 World Data Flow: OSM Vector Data, Rendered Locally

**Problem:** The spec (§4.2) describes the pipeline but does not say *where* it runs, and "local rendering" can be misread as rendering map images.

**Proposal:** make it explicit.

- **No map images.** We never download or show rendered map tiles. We use OSM **vector data** (roads, buildings, land use as points, lines and polygons) plus DEM elevation data. From these we build 3D geometry, and the user's GPU renders it in real time.
- **Where the pipeline runs:**

| Step | Who | Where |
|---|---|---|
| Download OSM PBF (Geofabrik) + DEM + imagery (P-16) | `world-builder` | Our build infrastructure (CI / build server) |
| Extract, project, smooth, generate road graph, meshes and texture maps | `world-builder` | Our build infrastructure |
| Sign and publish world packages | `world-distribution` service | Our servers / CDN |
| Download package once, cache on disk | Client | User's PC |
| Stream tiles from disk, render with Bevy | Client | User's PC (GPU) |

- **Why build centrally:** world generation is heavy (CPU, RAM, many GB of source data). Prebuilt packages are identical for all users, so multiplayer riders see the same road geometry and edge IDs, and packages can be signed.
- **Power users** can still run `world-builder` locally to build a custom region. These packages are unsigned and limited to single-player and private sessions.
- **During a ride** nothing is downloaded (spec §4.5). Everything comes from the local package.

---

## P-16 Landscape Textures and Satellite Imagery

**Need:** ground textures for different landscapes (meadow, field, forest floor, rock, sand, snow, urban, wetland, water edges), plus road surfaces (asphalt, gravel, cobbles, dirt).

### Can we use satellite images?

Yes, but only from openly licensed sources, and not as the main close-up ground texture.

| Source | Resolution | Licence | Usable? |
|---|---|---|---|
| **Copernicus Sentinel-2** (cloud-free mosaics, e.g. EOX Sentinel-2 cloudless) | 10 m | Copernicus data: free, attribution. Third-party mosaics have their own terms (EOX: CC BY-NC-SA for some years, commercial licence needed) | Yes; check the mosaic's licence |
| **ESA WorldCover** (land-cover classification, not imagery) | 10 m | CC BY 4.0 | Yes; very useful (see below) |
| **Official orthophotos** (e.g. German states' DOP20/DOP40, many under dl-de/by-2-0 or dl-de/zero-2-0; other EU countries similar) | 0.2–0.4 m | Open, varies by country/state | Yes, region by region |
| **USGS NAIP** (USA) | 0.6–1 m | Public domain | Yes |
| Google, Bing, Apple, Esri, Mapbox satellite layers | high | Proprietary; terms forbid extraction / offline use like this | **No** |

**Limits of satellite imagery at eye level:**

- It is a top-down photo with baked-in shadows, cars, tree crowns and seasonal colors. Seen from a bike at 1.7 m height it looks blurry and flat.
- **Size:** at 20 cm resolution, 1 km² is 25 megapixels, roughly 10–25 MB compressed. A 1,000 km² region would be 10–25 GB, far above the 200–500 MB package target (P-12).

### Proposed approach (hybrid)

1. **Close-up: tiling PBR materials.** Albedo, normal and roughness textures per surface type, blended by a per-tile **splat map**.
   - Sources: **CC0 libraries** (ambientCG, Poly Haven) plus our own materials.
   - Compressed as KTX2 / Basis Universal for the GPU.
2. **Splat map generation (world-builder):** pick the surface type per terrain cell from:
   - OSM `landuse` / `natural` / `surface` tags (forest, meadow, farmland, residential, scree, …),
   - **ESA WorldCover** to fill gaps where OSM has no land-use polygons,
   - slope (steep → rock) and altitude (high → alpine grass / rock / snow),
   - noise for natural variation.
3. **Mid and far distance: satellite color tint.** A low-resolution color map (Sentinel-2 at 10 m) modulates the tiling materials. Fields, forests and villages then get their real patchwork look from a distance without the blur up close. This is cheap in size: 1 km² at 10 m is only 100 × 100 pixels (about 30 KB uncompressed), so even a 1,000 km² region adds only tens of MB.
4. **Optional orthophotos:** high-resolution orthophotos only as an optional download for selected regions, used for far-LOD color and for better land-cover classification at build time.
5. **Vegetation:** instanced tree, bush and grass models placed by land cover and density (spec §4.5), so the 3D look comes from geometry rather than textures.
6. **Roads:** material from the OSM `surface` tag (asphalt, gravel, paving stones, …), with a fallback per road category. Markings are generated from `lanes` and the road category.

**Attribution:** every imagery and land-cover source used in a package is listed in its metadata and shown on the credits screen (extends P-9).

---

## P-17 In-Game Console

A developer and power-user console inside the running app.

- **UI:** toggled with `` ` `` (backtick) or `F12`. Rendered with egui (`bevy_egui`) as an overlay. Includes command history, auto-completion, scrollback and log output (connected to `tracing`).
- **Command registry:** each crate/plugin registers commands with a name, argument definition (parsed with `clap`), help text and a **permission level**. The existing `bevy_console` crate follows this pattern; evaluate it or build a small equivalent.
- **Permission levels:**

| Level | Available | Examples |
|---|---|---|
| `user` | Always | `help`, `fps`, `net.stats`, `devices.list`, `trainer.difficulty 50`, `camera.mode chase`, `screenshot` |
| `dev` | Single-player / local server, or debug builds | `sim.power 250` (virtual power without a trainer – useful for testing), `teleport km 12.3`, `time 18:30`, `weather rain`, `world.reload_tile`, `render.wireframe`, `render.lod_bias` |
| `admin` | Remote server, authenticated organizers / staff only; executed **on the server** | `event.start`, `event.pause`, `rider.kick <id>`, `rider.flag <id>` |

- **Multiplayer safety:** `dev` commands that affect the simulation are rejected when connected to a remote server. The server is authoritative (P-1), so a manipulated client cannot cheat with them anyway.
- **Scripts:** an `autoexec.cfg` file runs commands at startup. `exec <file>` runs command files, e.g. for automated performance tests (`tools/performance-test`).
- **Settings:** settings are also reachable as variables (`set render.shadows high`), so settings UI and console share one source of truth.
- **Phase:** 1 – it pays off immediately for debugging.

---

## P-18 3D Graphics Stack – Options

**Requirements from the spec:** Rust; native on Linux, macOS and Windows; Vulkan, Metal and D3D12 (no single-API dependency); large streamed outdoor worlds; many instanced objects; skeletal animation; modifiable and open licence.

### Candidates

| Option | Language / Licence | Graphics APIs | Strengths | Weaknesses |
|---|---|---|---|---|
| **A. Bevy** (latest: 0.20, Oct 2026) | Rust, MIT/Apache-2.0 | wgpu → Vulkan, Metal, D3D12 | Pure Rust; ECS fits our modular crates; PBR, cascaded shadows, procedural atmosphere, GPU-driven rendering, glTF, skeletal animation; experimental path tracing (Solari); very active community | Breaking changes in every release until 1.0 (no date); no built-in large-terrain / streaming system; editor still in progress; UI still maturing |
| **B. Fyrox** (1.0, Mar 2026) | Rust, MIT | **OpenGL only** (fyrox-graphics-gl); modern backends planned | Stable 1.0 API; full scene editor; built-in terrain | OpenGL is deprecated on macOS (no Metal) → violates the "no single GPU API" goal; smaller ecosystem; no global illumination yet |
| **C. Own renderer on wgpu** (+ ECS such as `hecs`, plus `winit`) | Rust, MIT/Apache-2.0 | Vulkan, Metal, D3D12 | Full control, tailored to terrain + roads; stable API under our control | Very high effort: we must build shadows, PBR, animation, asset handling and UI ourselves; delays all phases |
| **D. Godot 4 + godot-rust (gdext)** | C++ engine (MIT) + Rust game logic | Vulkan, Metal, D3D12 | Mature editor and tooling; terrain plugins (e.g. Terrain3D); stable releases; good UI system | Not Rust-native: engine in C++, logic through bindings; two build systems; ECS approach does not match; ties us to Godot's scene model |
| **E. Unreal Engine 5** | C++, proprietary (royalty above revenue threshold) | Vulkan, Metal, D3D12 | Best visuals; World Partition; landscape tools | Not open source; no practical Rust integration; licence costs; very large toolchain. Rules out the spec's core goals |
| **F. Unity** | C#, proprietary | all | Mature; large asset store | Proprietary, licence uncertainty in recent years, not Rust. Rules out the spec's core goals |
| *Other Rust renderers* (`rend3`, `kajiya`, `three-d`) | Rust | wgpu / Vulkan | – | `rend3` and `kajiya` are no longer maintained; `three-d` is too limited for large worlds |

### Assessment

- **E and F** contradict the spec's core goals (Rust, open, modular). They are excluded.
- **B (Fyrox)** has the stable API Bevy lacks, but the OpenGL-only renderer is a blocker for macOS and for the "no single GPU API" goal. Revisit it if a wgpu/Vulkan backend ships.
- **C** gives control but would cost a large part of the project's capacity before any cycling feature exists.
- **D (Godot + Rust)** is the only realistic **fallback** if Bevy fails for large outdoor worlds: mature tooling, with our engine-independent crates (`ocw-sim`, `ocw-protocol`, `ocw-devices`, `ocw-world`) usable through gdext.
- **A (Bevy)** fits the spec best. Its weaknesses are manageable:
  - API churn: pin the version and upgrade in planned steps (P-11). Our core logic does not depend on Bevy (P-14), so churn only affects `ocw-render` and `apps/desktop`.
  - Missing terrain/streaming: we need a custom solution anyway (road-conformed terrain P-6, tiles P-7, splat materials P-16). It becomes our own Bevy plugin.

### Recommendation

**Keep Bevy (A)**, with **Godot + gdext (D)** as a documented fallback. Confirm with a **technical spike in Phase 1** (about 2–3 weeks):

1. Render a 10 km × 10 km area: streamed 1 km tiles, terrain with splat materials, road meshes, a floating origin.
2. 100,000+ instanced trees/bushes with LOD, plus 100 animated rider avatars.
3. Run on Linux (Vulkan) on minimum-spec hardware (P-12). Following the platform rollout order (spec §11), repeat the same spike on Windows (D3D12) and macOS (Metal) at the start of each port.
4. Measure FPS, frame-time spikes during tile loading, RAM/VRAM and load times.

**Go criterion:** stable 60 FPS on the reference hardware with no visible hitches during streaming. If Bevy clearly fails and cannot be fixed in reasonable time, run the same spike on Godot.

**Supporting crates to evaluate:** `bevy_egui` (console, editor panels), `big_space` (floating origin), KTX2/Basis texture support (built into Bevy), `bevy_mod_billboard` or impostors for distant trees.

---

## P-19 Steering at Junctions (Free Ride)

**Requirement:** In free ride, the rider chooses turns at junctions with the **Left / Right arrow keys**. **On-screen overlay arrows** show the options. If no key is pressed, the rider **goes straight on / follows the road**.

### Junction detection

- A **junction** is a road-graph node with more than two rideable edges (P-13: the route graph from the world package).
- A junction becomes **active** when it comes within the **look-ahead distance**: the larger of 150 m or 10 s at current speed. Only the next junction is active.
- **Rideable exits** exclude:
  - the edge the rider arrived on,
  - roads not allowed for bicycles (`highway=motorway|trunk` without bicycle access, `bicycle=no`),
  - `oneway=yes` against the direction of travel, unless `oneway:bicycle=no`. This mainly matters for dual carriageways, which OSM maps as two separate one-way roads.
  - Depending on bike type, unpaved tracks / paths are shown with a warning icon for road bikes (allowed, not hidden).

### Classifying exits

Each exit gets a **turn angle** relative to the current heading, measured over the first ~30 m of the exit (not just the first OSM segment, which is often very short):

| Angle | Class |
|---|---|
| −30° … +30° | straight |
| < −30° | left (sharp left below −120°) |
| > +30° | right (sharp right above +120°) |

Exits are **ordered from leftmost to rightmost**. Keys move the selection through this ordered list.

### Default when no key is pressed ("follow the road")

Pick the first match in this list:

1. The exit continuing the **same road** (same OSM way, `name` or `ref`) whose angle is within ±60°.
2. Otherwise, the **most straight** exit within ±30°.
3. Otherwise (e.g. T-junction): the exit with the **higher road category** (`primary` > `secondary` > … > `track`).
4. Otherwise (both equal): **right** (matches right-hand traffic). This is configurable to "left" in settings for left-hand-traffic regions.
5. **Dead-end exits** (see below) are never chosen as the default if any other exit exists.

### Key behaviour

- **Left / Right** moves the selection **one step** left or right in the ordered exit list. From "straight", Left selects the nearest left exit; pressing Left again selects the next one further left (e.g. sharp left).
- Pressing the **opposite key** moves back step by step; returning to the default exit is "no choice".
- A pre-selection is allowed **before** the junction becomes active ("next junction: turn left") and is kept until the junction is reached.
- **Lock point:** ~20 m before the junction the choice is locked; key presses after that apply to the following junction.
- **Down arrow:** U-turn (see dead ends). Only in free ride; rate-limited (e.g. once per 10 s).
- All bindings are remappable. Later, gamepad and handlebar controller buttons map to the same actions (`turn_left`, `turn_right`, `u_turn`).

### On-screen overlay

- When a junction is active, a **fan of arrows** appears at the lower centre of the HUD. There is one arrow per rideable exit, drawn at its real turn angle.
- **Selected** arrow highlighted. The **default** arrow is marked while nothing else is chosen; once locked, the arrow pulses or turns solid.
- Labels: **distance to junction** (counting down), the **street name / ref** of the selected exit, and icons for **dead end**, **unpaved** and **steep climb** (> 8 % in the next 500 m).
- Optional: the minimap highlights the selected exit.
- The overlay fades out after the junction is passed.

### Modes

| Mode | Steering |
|---|---|
| **Free ride** (solo, or shared free ride) | Keys and overlay active |
| **Route / workout / event / race** | The path is fixed by the route. Arrows show only the route direction ahead (informational); turn keys are disabled. Optional "leave route" via a console/menu action in solo rides |
| **Group ride "follow leader"** | Turns follow the ride leader's choices; keys disabled for followers |

### Simulation and multiplayer

- Riders move in 1D along a path (P-1). In free ride the path is an **edge list that is extended on the fly** by each junction decision.
- The client sends a reliable message `TurnIntent { junction_id, exit_edge_id }` (P-2) as soon as the choice changes. The **server decides** at the lock point and broadcasts the chosen edge. The client predicts with its own choice and is corrected only in the rare case of disagreement (e.g. intent arrived too late → server default applies).
- The lateral offset is blended across the junction so the avatar follows a smooth curve, not the raw graph corner.
- **Sharp turns** (> 120°): an automatic short slow-down is applied in the physics (the rider can't brake on a trainer), so the avatar does not take a hairpin at 40 km/h.

### Dead-end roads

**At build time (world-builder):** remove the data artefacts, keep the real dead ends.

1. Keep only the **largest connected component** of the rideable graph per region (drops disconnected fragments).
2. **Prune short spurs** shorter than ~50 m, and leave out driveways / parking aisles (`service=driveway|parking_aisle`) entirely.
3. **Flag remaining dead ends:** every edge leading only into a dead-end subtree is marked `dead_end` with the distance to its end. This includes cul-de-sacs, valley roads ending at a village, and pass roads ending at a summit.
4. **Region borders:** edges cut by the package boundary are marked `boundary`. Neighbouring packages share stable edge IDs at the border (P-13), so an installed neighbour package continues the road seamlessly.

**At runtime:**

- **Prevention:** dead-end exits show a dead-end icon in the overlay and are never the default. While riding into a dead end, the HUD shows "Dead end in 800 m" once it is within 1 km.
- **Automatic U-turn at the end:** the rider slows to ~10 km/h, the avatar turns around in a short loop, and the ride continues on the same road in the opposite direction. Power and physics simply continue, so no workout or ride is interrupted.
- **Manual U-turn** at any time with the Down arrow key (free ride only), using the same animation.
- **Region border:** if the neighbouring package is installed, the ride continues across the border without interruption. Otherwise there is an automatic U-turn with the message "End of region – download <neighbour>" (download only after the ride; nothing is downloaded during a ride, see P-15).
- **Routes and GPX imports** (P-13a) are checked when they are created: a route that runs into a dead end must contain an explicit turnaround point (out-and-back routes are valid).

**Phase:** junction steering and dead-end handling are part of Phase 3 (OSM world). The multiplayer `TurnIntent` is part of Phase 5.

---

## P-20 Web Companion

**Scope:** a browser app for everything **except riding**. It runs in any modern browser, including iOS / iPadOS. It uses no Bluetooth and no 3D rendering.

> **Future development (not scheduled):** a full web client for riding in the browser and native mobile apps (iPad / iOS, Android). See the platform rollout order in spec §11. Nothing in the current design may block them (see "Keeping the future options open" below).

### Features

| Area | Features | Phase |
|---|---|---|
| Account | Sign-up / sign-in (same OIDC provider as the desktop app), profile, weight / FTP / height, privacy settings, data export and account deletion (P-12) | 5 |
| Training history | List of rides, ride detail (power, heart rate, cadence, speed, elevation charts; power-duration curve), personal bests, FIT / GPX download | 5 |
| Strava | Link / unlink account, auto-upload setting, re-upload of single rides (P-13b) | 5 |
| Devices | Read-only list of the user's known trainers and sensors (pairing stays in the desktop app) | 5 |
| Social | Friends, groups, invitations | 5 |
| Events | Calendar, event pages, sign-up, start lists, results and live leaderboards | 7 |
| Routes | Browse routes per world package with map + elevation profile; **GPX upload** with server-side map matching (P-13a); simple route planner (click junctions on the map) | 7 |
| World packages | List of available regions, sizes and versions; mark regions for download on the desktop app | 7 |

### Technology

- **Frontend: Leptos** (Rust → WebAssembly, with server-side rendering).
  - It keeps the stack in Rust and reuses types from `ocw-core` and `ocw-protocol` (P-2).
  - **Server-side rendering on Axum** gives fast first loads, and public event and result pages that work as normal, shareable links.
  - Alternative if Leptos falls short: a TypeScript frontend (e.g. SvelteKit) against the same API.
- **Backend:** no new service. The pages are served by the existing modular monolith (`services/server`, Axum) under a separate route group, using the same database and permission checks as the desktop API.
- **Maps:** **MapLibre GL JS** with our **own vector tiles**, generated by `world-builder` from the same OSM extracts and served as **PMTiles** files from our CDN.
  - No public OSM tile servers (P-9). Attribution "© OpenStreetMap contributors" stays visible on the map.
  - Routes and elevation profiles come from the world package data, so the web and the desktop app show the same road geometry.
- **GPX map matching** runs on the server with the same `ocw-world` code as the desktop app (P-13a), so routes created on the web are identical in the desktop app.
- **Live data** (event leaderboards): server-sent events or WebSocket from the backend. They are read-only, so no WebTransport is needed.
- **Security:**
  - Short-lived session cookies (`HttpOnly`, `Secure`, `SameSite=Lax`) and a strict Content Security Policy.
  - CSRF protection for state-changing requests.
  - Strava tokens never reach the browser (P-13b).

### Keeping the future options open

These rules keep a later full web client and native mobile apps possible without redesign:

- `ocw-sim`, `ocw-protocol`, `ocw-world` and `ocw-core` must **compile to `wasm32-unknown-unknown`**. They may not use threads, the file system or sockets directly; those go through traits. A CI check builds them for wasm from Phase 1.
- The device layer stays behind `DeviceBackend` (P-11), so Web Bluetooth (web) or CoreBluetooth (iOS) can be added as further backends.
- The game protocol is transport-independent (P-2). A WebTransport endpoint can later be added next to native QUIC on the server.
- World packages keep a format that can be streamed over HTTP in tile-sized chunks, not only read as one large archive from disk.
