# Open Cycling World

**Cross-Platform Indoor Cycling Simulation**

Project Description and Technical Architecture

Version 1.1 – November 2026

| Version | Change |
|---|---|
| 1.0 | Original specification (translated from German) |
| 1.1 | Decisions from the specification review merged in (former PROPOSALS.md, P-1 … P-20; see Appendix A) |

---

## 1. Product Vision

Open Cycling World is a cross-platform indoor cycling simulation for Linux, Windows and macOS.

The application connects real bicycle hardware with an interactive virtual 3D world. Users can ride alone or together with other participants on real geographic routes, train, take part in races, and individually customize their bicycles and their virtual characters.

The geographic foundation is OpenStreetMap data. From this data, locally usable 3D roads, landscapes and routes are generated.

Functionally, the simulation is modeled on Zwift and MyWhoosh, but it shall have an independent, modular, Rust-based architecture.

### 1.1 Main Goals

- Native desktop application for Linux, Windows and macOS (rollout in this order, §11.1).
- Complete implementation in Rust, wherever technically reasonable.
- 3D rendering via Bevy and wgpu.
- Support for Bluetooth smart trainers and bicycle sensors.
- Realistic bicycle and route physics.
- Real-time multiplayer with many participants riding simultaneously.
- Integrated voice communication.
- Bicycle and character editor.
- Generation of geographic 3D worlds from OpenStreetMap.
- Local rendering and local caching of route data.
- Offline-capable single-player rides on downloaded routes.
- A web companion for everything except riding.
- Modular extensibility for future features.

### 1.2 Clients

| Client | Purpose | Status |
|---|---|---|
| **Desktop app** (`apps/desktop`) | The simulation client: riding, rendering, sensors, editors | Core product |
| **Web companion** (`apps/web-companion`) | Account, history, Strava, routes, events in any browser – no riding | From Phase 5 (§2.10) |
| Full web client (riding in the browser via Web Bluetooth) | – | Future development, not scheduled |
| Native mobile apps (iPad / iOS, Android) | – | Future development, not scheduled |

### 1.3 Deferred and Future Items

- **ANT+** support is deferred. The device abstraction (§3.4) allows adding an ANT+ backend later without changes to other crates.
- **Full web client** and **native mobile apps** are future development. The architecture must not block them (§3.5).

---

## 2. Feature Scope

### 2.1 User Management and Start Screen

The application has a central dashboard.

Features:

- User account and profile.
- Rider statistics.
- Training history.
- Personal bests.
- Available routes.
- Current multiplayer sessions.
- Friends and groups.
- Scheduled events.
- Device management.
- Personal settings.

The application shall also provide a local training mode without requiring sign-in.

Most account, history and social functions are also available in the web companion (§2.10).

### 2.2 Virtual Cycling World

The 3D world is based on real geographic data.

Examples:

- City streets.
- Country roads.
- Forest trails.
- Cycle paths.
- Mountain routes.
- Alpine passes.
- Coastal roads.
- Custom-defined circuits.

The virtual world is reconstructed from geographic vector data (§4).

The presentation includes:

- 3D terrain.
- Road surfaces.
- Road markings.
- Curves and gradients.
- Forests and vegetation.
- Buildings and bridges.
- Rivers and lakes.
- Lighting and shadows.
- Weather and times of day.

The graphical presentation does not need to match reality exactly. However, route layout and elevation profiles shall match the geographic data as closely as possible.

#### 2.2.1 Steering at Junctions (Free Ride)

In free ride, the rider chooses turns at junctions with the **Left / Right arrow keys**. **On-screen overlay arrows** show the options. If no key is pressed, the rider **goes straight on / follows the road**.

**Junction detection**

- A **junction** is a road-graph node with more than two rideable edges (§4.4).
- A junction becomes **active** when it comes within the **look-ahead distance**: the larger of 150 m or 10 s at current speed. Only the next junction is active.
- **Rideable exits** exclude:
  - the edge the rider arrived on,
  - roads not allowed for bicycles (`highway=motorway|trunk` without bicycle access, `bicycle=no`),
  - `oneway=yes` against the direction of travel, unless `oneway:bicycle=no`. This mainly matters for dual carriageways, which OSM maps as two separate one-way roads.
- Depending on bike type, unpaved tracks / paths are shown with a warning icon for road bikes (allowed, not hidden).

**Classifying exits**

Each exit gets a **turn angle** relative to the current heading, measured over the first ~30 m of the exit (not just the first OSM segment, which is often very short):

| Angle | Class |
|---|---|
| −30° … +30° | straight |
| < −30° | left (sharp left below −120°) |
| > +30° | right (sharp right above +120°) |

Exits are **ordered from leftmost to rightmost**. Keys move the selection through this ordered list.

**Default when no key is pressed ("follow the road")** – the first match wins:

1. The exit continuing the **same road** (same OSM way, `name` or `ref`) whose angle is within ±60°.
2. Otherwise, the **most straight** exit within ±30°.
3. Otherwise (e.g. T-junction): the exit with the **higher road category** (`primary` > `secondary` > … > `track`).
4. Otherwise (both equal): **right** (matches right-hand traffic). Configurable to "left" for left-hand-traffic regions.
5. **Dead-end exits** (§2.2.2) are never chosen as the default if any other exit exists.

**Key behaviour**

- **Left / Right** moves the selection **one step** left or right in the ordered exit list. From "straight", Left selects the nearest left exit; pressing Left again selects the next one further left (e.g. sharp left).
- Pressing the **opposite key** moves back step by step; returning to the default exit is "no choice".
- A pre-selection is allowed **before** the junction becomes active ("next junction: turn left") and is kept until the junction is reached.
- **Lock point:** ~20 m before the junction the choice is locked; key presses after that apply to the following junction.
- **Down arrow:** U-turn (§2.2.2). Only in free ride; rate-limited (e.g. once per 10 s).
- All bindings are remappable. Later, gamepad and handlebar controller buttons map to the same actions (`turn_left`, `turn_right`, `u_turn`).

**On-screen overlay**

- When a junction is active, a **fan of arrows** appears at the lower centre of the HUD: one arrow per rideable exit, drawn at its real turn angle.
- The **selected** arrow is highlighted. The **default** arrow is marked while nothing else is chosen; once locked, the arrow pulses or turns solid.
- Labels: **distance to junction** (counting down), the **street name / ref** of the selected exit, and icons for **dead end**, **unpaved** and **steep climb** (> 8 % in the next 500 m).
- Optional: the minimap highlights the selected exit.
- The overlay fades out after the junction is passed.

**Modes**

| Mode | Steering |
|---|---|
| **Free ride** (solo, or shared free ride) | Keys and overlay active |
| **Route / workout / event / race** | The path is fixed by the route. Arrows show only the route direction ahead (informational); turn keys are disabled. Optional "leave route" via a console/menu action in solo rides |
| **Group ride "follow leader"** | Turns follow the ride leader's choices; keys disabled for followers |

Simulation and network aspects of steering are described in §5.2 and §6.5.

#### 2.2.2 Dead-End Roads

**At build time** (world builder, §4.4): data artefacts are removed, real dead ends are kept and flagged.

**At runtime:**

- **Prevention:** dead-end exits show a dead-end icon in the overlay and are never the default. While riding into a dead end, the HUD shows "Dead end in 800 m" once it is within 1 km.
- **Automatic U-turn at the end:** the rider slows to ~10 km/h, the avatar turns around in a short loop, and the ride continues on the same road in the opposite direction. Power and physics simply continue, so no workout or ride is interrupted.
- **Manual U-turn** at any time with the Down arrow key (free ride only), using the same animation.
- **Region border:** if the neighbouring package is installed, the ride continues across the border without interruption. Otherwise there is an automatic U-turn with the message "End of region – download <neighbour>". The download happens only after the ride; nothing is downloaded during a ride (§4.2).
- **Routes and GPX imports** (§2.7) are checked when they are created: a route that runs into a dead end must contain an explicit turnaround point (out-and-back routes are valid).

### 2.3 Real-Time Multiplayer

Multiplayer is a core part of the application and is taken into account in the architecture from the start: from Phase 1, the desktop app runs against an in-process server through the real protocol (§6.1).

Features:

- Shared rides.
- Group rides.
- Races.
- Time trials.
- Training groups.
- Private sessions.
- Public events.
- Friends lists.
- Invitations.
- Live leaderboards.
- Real-time positions of other riders.
- Power and speed comparisons.

Each participant is represented by their own avatar.

The position, speed, heading and animation of other participants are continuously synchronized.

#### Multiplayer Requirements

- Server-side authority over the simulation and race state.
- Client-side prediction for the local rider.
- Interpolation of other participants.
- Latency compensation.
- Reconnection after network interruptions.
- Detection and flagging of implausible power data (§6.4). Power comes from user hardware and cannot be fully verified.
- Separation of training data and visual presentation.

The simulation must not depend on the client's frame rate.

### 2.4 Voice Communication

The application includes integrated real-time voice communication.

Features:

- Group voice channels.
- Private voice rooms.
- Voice communication during shared rides.
- Push-to-talk.
- Permanently enableable (open) microphone.
- Microphone and speaker selection.
- Muting of individual participants.
- Global mute.
- Volume control.
- Automatic echo cancellation.
- Noise suppression.
- Connection status.

Additionally, spatial audio is provided: voices of riders who are virtually behind the local rider sound quieter than voices of immediately adjacent riders.

Voice communication is technically separated from the actual game state (§7).

### 2.5 Bicycle Editor

Each user can configure their virtual bicycle.

Features:

- Selection of bicycle type:
  - Road bike.
  - Gravel bike.
  - Mountain bike.
  - Time trial bike.
- Selection of frame and color.
- Wheels.
- Tires.
- Handlebars.
- Saddle.
- Attachments / accessories.
- Custom paint schemes.
- Component unlocking.
- 3D preview.

The bicycle configuration consists of reusable components. Each component has its own properties, such as weight, aerodynamics, rolling resistance, geometry and visual appearance.

#### Cosmetics vs. Performance

Visual properties are separated from physical properties:

- Each component has a `visual` part (mesh, materials, paint slots) and an optional reference to a **physics profile**.
- Only a **small, curated set of physics profiles** exists per bike type (e.g. "road – aero", "road – climbing", "road – standard"). Cosmetic variants share profiles.
- Unlocks and paint schemes are **always cosmetic**.
- Differences between profiles are capped (e.g. at most ±3 % speed at 250 W on flat road).
- **Race modes:**
  - *Open* – the chosen profile applies.
  - *Equal* – standardized physics for everyone; chosen equipment is shown only visually.

### 2.6 Character Editor

The user can individually design their rider.

Features:

- Body model.
- Gender or body representation.
- Skin color.
- Facial features.
- Hairstyles.
- Clothing.
- Shoes.
- Helmet.
- Glasses.
- Gloves.
- Jerseys.
- Shorts / pants.
- Animations.

Characters use a shared skeleton and animation system.

Animation is driven by cadence, speed and riding state.

Planned animations:

- Normal pedaling.
- Sprint.
- Climbing.
- Out-of-the-saddle (standing) pedaling.
- Coasting.
- Cornering.
- U-turn.
- Finish-line arrival.
- Victory animation.

Bicycle and character must be correctly attached to each other in the 3D presentation.

### 2.7 Routes and GPX Import

**Routes** are defined on the road graph (§4.4): an ordered list of edges with direction, start/finish markers, lap count and optional segments (sprints, KOMs). Routes are stored as TOML/JSON in the world package. A route editor in `world-builder` lets authors click edges together and set laps and segments.

**GPX import:** the user imports a GPX file (e.g. a real ride or a planned route) and rides it as a route.

1. **Parse** the GPX track (`gpx` crate); accept tracks and routes, and join multiple segments.
2. **Region check:** compute the bounding box. If no installed world package covers it, offer to download the matching package(s).
3. **Map matching:** snap the GPS points onto the road graph using an HMM-based matcher (the standard Viterbi approach used by OSRM / Valhalla), implemented in `ocw-world`. The result is the ordinary route format (edge list).
4. **Elevation from our DEM, not from the GPX:** GPS elevation is noisy, so the route uses the already smoothed road profile (§4.3). The GPX elevation is only shown for comparison.
5. **Problems are reported, not hidden:** sections more than ~30 m from any road, or on roads missing from the package, are highlighted. The user can accept a detour or cancel.
6. **Result:** a personal route stored locally. It can optionally be shared with friends or used for a group ride; the server receives only the edge list, not the GPX.

GPX upload is also available in the web companion, where map matching runs on the server with the same `ocw-world` code (§2.10).

### 2.8 Training, Export and Strava

- **Structured workouts:** our own open workout format (TOML/JSON: intervals, targets as % FTP, cadence hints). Import of the open formats `.erg` / `.mrc` and FIT workout files. Workouts use ERG mode (§5.3).
- **Export:** every ride is saved as a **FIT** activity file (power, cadence, heart rate, speed, distance, elevation, virtual GPS track). Ridden routes can also be exported as **GPX**. A FIT writer is needed; existing Rust crates mainly read FIT.
- **Strava upload for finished rides:**
  - **Account linking via our backend:** Strava's OAuth flow requires a client secret, which must not ship in the desktop app. The backend handles the OAuth exchange and stores the refresh tokens encrypted. Tokens never reach the desktop app or the browser.
  - **Upload flow:** when a ride ends, the client writes the FIT file and sends it to our API. The API uploads it via the Strava `uploads` endpoint as `VirtualRide`, polls Strava for the upload status and reports the result to the client.
  - **Options:** automatic upload after each ride, or a manual "Upload" button in the training history. Title and description can be edited; a default title is generated (route name, world).
  - **Offline:** uploads are queued and sent later.
  - **Constraints:** Strava API terms and rate limits apply. Beyond a small number of athletes, Strava requires an app review – apply early. The virtual GPS track uses real coordinates of the route; this is common practice for virtual rides, but it must be labelled as virtual.
  - Other services (intervals.icu, TrainingPeaks) may follow later.

### 2.9 In-Game Console

A developer and power-user console inside the running desktop app.

- **UI:** toggled with `` ` `` (backtick) or `F12`. Rendered with egui (`bevy_egui`) as an overlay. Includes command history, auto-completion, scrollback and log output (connected to `tracing`).
- **Command registry:** each crate/plugin registers commands with a name, argument definition (parsed with `clap`), help text and a **permission level**. The existing `bevy_console` crate follows this pattern; evaluate it or build a small equivalent.
- **Permission levels:**

| Level | Available | Examples |
|---|---|---|
| `user` | Always | `help`, `fps`, `net.stats`, `devices.list`, `trainer.difficulty 50`, `camera.mode chase`, `screenshot` |
| `dev` | Single-player / local server, or debug builds | `sim.power 250` (virtual power without a trainer – useful for testing), `teleport km 12.3`, `time 18:30`, `weather rain`, `world.reload_tile`, `render.wireframe`, `render.lod_bias` |
| `admin` | Remote server, authenticated organizers / staff only; executed **on the server** | `event.start`, `event.pause`, `rider.kick <id>`, `rider.flag <id>` |

- **Multiplayer safety:** `dev` commands that affect the simulation are rejected when connected to a remote server. The server is authoritative (§6.1), so a manipulated client cannot cheat with them anyway.
- **Scripts:** an `autoexec.cfg` file runs commands at startup. `exec <file>` runs command files, e.g. for automated performance tests (`tools/performance-test`).
- **Settings:** settings are also reachable as variables (`set render.shadows high`), so the settings UI and the console share one source of truth.

### 2.10 Web Companion

A browser app for everything **except riding**. It runs in any modern browser, including iOS / iPadOS. It uses no Bluetooth and no 3D rendering.

| Area | Features | Phase |
|---|---|---|
| Account | Sign-up / sign-in (same OIDC provider as the desktop app), profile, weight / FTP / height, privacy settings, data export and account deletion (§12) | 5 |
| Training history | List of rides, ride detail (power, heart rate, cadence, speed, elevation charts; power-duration curve), personal bests, FIT / GPX download | 5 |
| Strava | Link / unlink account, auto-upload setting, re-upload of single rides (§2.8) | 5 |
| Devices | Read-only list of the user's known trainers and sensors (pairing stays in the desktop app) | 5 |
| Social | Friends, groups, invitations | 5 |
| Events | Calendar, event pages, sign-up, start lists, results and live leaderboards | 7 |
| Routes | Browse routes per world package with map + elevation profile; GPX upload with server-side map matching (§2.7); simple route planner (click junctions on the map) | 7 |
| World packages | List of available regions, sizes and versions; mark regions for download on the desktop app | 7 |

Technology: see §3.6.

---

## 3. Overall Technical Stack

### 3.1 Programming Language and Engine

- **Primary programming language:** Rust
- **3D engine:** Bevy
- **Graphics backend:** wgpu

Bevy handles 3D scenes, rendering, cameras, lighting, shadows, materials, animations, the Entity Component System, asset management, game states and the user interface.

wgpu provides the cross-platform GPU abstraction with the backends Vulkan, Direct3D 12 and Metal.

**Bevy versioning:** Bevy has breaking changes in every release until 1.0. An exact Bevy version is pinned in the workspace and upgraded in dedicated milestones, typically one release behind current, once the ecosystem crates we use have caught up. Only `ocw-render` and `apps/desktop` depend on Bevy (§10), so upgrades stay local.

#### 3.1.1 Engine Evaluation

**Requirements:** Rust; native on Linux, Windows and macOS; Vulkan, D3D12 and Metal (no single-API dependency); large streamed outdoor worlds; many instanced objects; skeletal animation; modifiable and open licence.

| Option | Language / Licence | Graphics APIs | Strengths | Weaknesses |
|---|---|---|---|---|
| **A. Bevy** (latest: 0.20, Oct 2026) | Rust, MIT/Apache-2.0 | wgpu → Vulkan, Metal, D3D12 | Pure Rust; ECS fits our modular crates; PBR, cascaded shadows, procedural atmosphere, GPU-driven rendering, glTF, skeletal animation; experimental path tracing (Solari); very active community | Breaking changes in every release until 1.0 (no date); no built-in large-terrain / streaming system; editor still in progress; UI still maturing |
| **B. Fyrox** (1.0, Mar 2026) | Rust, MIT | **OpenGL only** (fyrox-graphics-gl); modern backends planned | Stable 1.0 API; full scene editor; built-in terrain | OpenGL is deprecated on macOS (no Metal) → violates the "no single GPU API" goal; smaller ecosystem; no global illumination yet |
| **C. Own renderer on wgpu** (+ ECS such as `hecs`, plus `winit`) | Rust, MIT/Apache-2.0 | Vulkan, Metal, D3D12 | Full control, tailored to terrain + roads; stable API under our control | Very high effort: shadows, PBR, animation, asset handling and UI built ourselves; delays all phases |
| **D. Godot 4 + godot-rust (gdext)** | C++ engine (MIT) + Rust game logic | Vulkan, Metal, D3D12 | Mature editor and tooling; terrain plugins (e.g. Terrain3D); stable releases; good UI system | Not Rust-native: engine in C++, logic through bindings; two build systems; ECS approach does not match; ties us to Godot's scene model |
| **E. Unreal Engine 5** | C++, proprietary (royalty above revenue threshold) | Vulkan, Metal, D3D12 | Best visuals; World Partition; landscape tools | Not open source; no practical Rust integration; licence costs; very large toolchain |
| **F. Unity** | C#, proprietary | all | Mature; large asset store | Proprietary, licence uncertainty in recent years, not Rust |
| *Other Rust renderers* (`rend3`, `kajiya`, `three-d`) | Rust | wgpu / Vulkan | – | `rend3` and `kajiya` are no longer maintained; `three-d` is too limited for large worlds |

**Decision:** **Bevy (A)**, with **Godot + gdext (D)** as the documented fallback.

- E and F contradict the core goals (Rust, open, modular) and are excluded.
- Fyrox has the stable API Bevy lacks, but the OpenGL-only renderer blocks macOS and the "no single GPU API" goal. Revisit if a modern backend ships.
- An own renderer would consume a large part of the project's capacity before any cycling feature exists.
- Bevy's missing terrain/streaming system is acceptable: we need a custom solution anyway (road-conformed terrain §4.3, tiles §4.5, splat materials §4.6). It becomes our own Bevy plugin.

**Technical spike in Phase 1** (about 2–3 weeks) to confirm the decision:

1. Render a 10 km × 10 km area: streamed 1 km tiles, terrain with splat materials, road meshes, a floating origin.
2. 100,000+ instanced trees/bushes with LOD, plus 100 animated rider avatars.
3. Run on Linux (Vulkan) on minimum-spec hardware (§15). Following the rollout order (§11.1), repeat the spike on Windows (D3D12) and macOS (Metal) at the start of each port.
4. Measure FPS, frame-time spikes during tile loading, RAM/VRAM and load times.

**Go criterion:** stable 60 FPS on the reference hardware with no visible hitches during streaming. If Bevy clearly fails and cannot be fixed in reasonable time, run the same spike on Godot.

**Supporting crates to evaluate:** `bevy_egui` (console, editor panels), `big_space` (floating origin), KTX2/Basis texture support (built into Bevy), `bevy_mod_billboard` or impostors for distant trees.

### 3.2 User Interface

- **Bevy UI** for the in-ride HUD (including the junction overlay, §2.2.1) and the main menus. The user interface is connected as directly as possible to the engine state.
- **egui** (`bevy_egui`) for the editor parameter panels, the in-game console (§2.9) and development/diagnostic interfaces, to keep development fast. The editors are reviewed after Phase 4.

The interface is divided into modules:

- Dashboard.
- Training view.
- Multiplayer lobby.
- Device management.
- Bicycle editor.
- Character editor.
- Settings.
- Route selection.

### 3.3 Audio

Two separate audio systems:

**Game audio** (Bevy Audio): background music, ambient sounds, bicycle sounds, wind, tires, freewheel, gear shifting.

**Voice communication** (§7): WebRTC, Opus audio codec, LiveKit SFU, microphone and speaker selection, echo cancellation, noise suppression. Voice is provided via a separate service.

### 3.4 Hardware Communication

Bluetooth Low Energy profiles:

- Fitness Machine Service (FTMS).
- Cycling Power Service.
- Cycling Speed and Cadence.
- Heart Rate Service.

Library: **btleplug**.

- The device layer (`ocw-devices`) has **no dependency on the 3D engine**. It runs on its own Tokio runtime/thread and talks to the app over channels.
- It defines a `DeviceBackend` trait. BLE is the first implementation. **ANT+ is deferred** and will be a second backend later.
- Paired devices are remembered and reconnected automatically.
- Hardware is always connected to the **client** only; the server never talks to hardware (§6.1).
- Per-platform requirements, implemented in the rollout order (§11.1):
  - Linux: BlueZ via D-Bus; document the minimum BlueZ version.
  - Windows: WinRT BLE APIs (Windows 10 or later).
  - macOS: CoreBluetooth; `NSBluetoothAlwaysUsageDescription` in `Info.plist` plus the matching entitlement.

Trainer control is described in §5.3.

### 3.5 Keeping Future Clients Possible

A full web client and native mobile apps are future development (§1.3). These rules keep them possible without redesign:

- `ocw-sim`, `ocw-protocol`, `ocw-world` and `ocw-core` must **compile to `wasm32-unknown-unknown`**. They may not use threads, the file system or sockets directly; those go through traits. A CI check builds them for wasm from Phase 1.
- The device layer stays behind `DeviceBackend` (§3.4), so Web Bluetooth (web) or CoreBluetooth (iOS) can be added as further backends.
- The game protocol is transport-independent (§6.2). A WebTransport endpoint can later be added next to native QUIC on the server.
- World packages keep a format that can be streamed over HTTP in tile-sized chunks, not only read as one large archive from disk.

Background: Web Bluetooth (needed for riding in a browser) is available only in Chromium-based browsers on Windows, macOS, ChromeOS and Android, and not on iOS / iPadOS. iOS therefore requires a native app.

### 3.6 Web Companion Technology

- **Frontend: Leptos** (Rust → WebAssembly, with server-side rendering).
  - Keeps the stack in Rust and reuses types from `ocw-core` and `ocw-protocol`.
  - **Server-side rendering on Axum** gives fast first loads, and public event and result pages that work as normal, shareable links.
  - Alternative if Leptos falls short: a TypeScript frontend (e.g. SvelteKit) against the same API.
- **Backend:** no new service. The pages are served by the modular monolith (`services/server`, Axum) under a separate route group, using the same database and permission checks as the desktop API.
- **Maps:** **MapLibre GL JS** with our **own vector tiles**, generated by `world-builder` from the same OSM extracts and served as **PMTiles** files from our CDN.
  - No public OSM tile servers (§13). Attribution "© OpenStreetMap contributors" stays visible on the map.
  - Routes and elevation profiles come from the world package data, so web and desktop show the same road geometry.
- **GPX map matching** runs on the server with the same `ocw-world` code as the desktop app.
- **Live data** (event leaderboards): server-sent events or WebSocket. They are read-only, so no WebTransport is needed.
- **Security:** short-lived session cookies (`HttpOnly`, `Secure`, `SameSite=Lax`), a strict Content Security Policy, CSRF protection for state-changing requests. Strava tokens never reach the browser.

---

## 4. OpenStreetMap and Local 3D World

### 4.1 Data Source

OpenStreetMap is the primary source of geographic information.

Data used: roads, cycle paths, paths / tracks, road categories, intersections, bridges, tunnels, buildings, water bodies, forests, land use, settlement areas, place information.

Data format: **OSM PBF**.

Data sources for regional extracts:

- Geofabrik.
- Custom OSM data extracts.
- Optionally, other OSM data providers.

Supplementary sources: elevation models (§4.3), land cover and imagery (§4.6).

### 4.2 Data Flow: Vector Data, Rendered Locally

- **No map images.** We never download or show rendered map tiles as the basis of the 3D world. We use OSM **vector data** (roads, buildings, land use as points, lines and polygons) plus DEM elevation data. From these we build 3D geometry, and the user's GPU renders it in real time.
- **Where the pipeline runs:**

| Step | Who | Where |
|---|---|---|
| Download OSM PBF (Geofabrik) + DEM + land cover / imagery | `world-builder` | Our build infrastructure (CI / build server) |
| Extract, project, smooth, build road graph, meshes and texture maps | `world-builder` | Our build infrastructure |
| Sign and publish world packages | `world-distribution` | Our servers / CDN |
| Download package once, cache on disk | Client | User's PC |
| Stream tiles from disk, render with Bevy | Client | User's PC (GPU) |

- **Why build centrally:** world generation is heavy (CPU, RAM, many GB of source data). Prebuilt packages are identical for all users, so multiplayer riders see the same road geometry and edge IDs, and packages can be signed.
- **Power users** can run `world-builder` locally to build a custom region. These packages are unsigned and limited to single-player and private sessions.
- **During a ride nothing is downloaded.** Everything comes from the local package.

**World-builder pipeline:**

1. Select region.
2. Download OSM PBF data and supplementary data.
3. Extract relevant geographic objects.
4. Transform coordinates (§4.5).
5. Reconstruct road courses and build the road graph (§4.4).
6. Determine or estimate road widths.
7. Compute smoothed road elevation profiles (§4.3).
8. Generate terrain, conformed to the roads.
9. Generate road surfaces.
10. Place buildings and vegetation.
11. Generate meshes, splat maps and textures (§4.6).
12. Generate vector tiles for the web companion map (§3.6).
13. Package, sign and publish (§4.7).

The client then loads the package into Bevy.

### 4.3 Terrain and Elevation

OpenStreetMap does not contain precise elevation information with full coverage, so an additional digital elevation model (DEM) is required.

Sources, in order of preference:

1. Regional high-resolution open elevation models (e.g. 1 m DGM / LiDAR where available).
2. Copernicus DEM (GLO-30).
3. SRTM.

From the elevation data the following are generated: elevation profiles, terrain meshes, climbs, descents, mountain passes, valleys and elevation differences.

**The road is the source of truth and the terrain adapts to it:**

1. **Sampling:** sample the DEM along each road centerline every 5 m with bilinear interpolation.
2. **Smoothing:** apply a median filter (removes spikes), then a constrained spline fit with:
   - a maximum gradient per road category (e.g. 25 % for paths, 15 % for main roads),
   - a maximum change in gradient per metre.
3. **Use OSM tags:** `bridge`, `tunnel`, `layer`, `incline`, `embankment`, `cutting`. Bridges and tunnels are interpolated linearly or with a smooth curve between their abutments or portals, ignoring the terrain height.
4. **Terrain conformance:** the terrain mesh is flattened and blended along a road corridor (road width + shoulder), so roads never float or sink.
5. **Validation:** the world builder reports suspicious segments (gradient jumps, extreme climbs) for review.

Smoothing of the gradient sent to the trainer is covered in §5.3.

### 4.4 Road Reconstruction and Road Graph

Each road is converted into a 3D road segment with these properties: geographic centerline, road width, surface material, gradient, curve radius, road marking, connectivity information and road category.

The road surface is generated from the line geometries.

The road connections form the **road graph**, which is the basis for routes (§2.7), junction steering (§2.2.1) and navigation.

- **Stable edge IDs:** derived from OSM way ID + node range plus a geometry hash. When a package is rebuilt from newer OSM data, routes are re-matched automatically and the world builder flags any that break.
- **Graph cleanup and dead ends:**
  1. Keep only the **largest connected component** of the rideable graph per region (drops disconnected fragments).
  2. **Prune short spurs** shorter than ~50 m, and leave out driveways / parking aisles (`service=driveway|parking_aisle`) entirely.
  3. **Flag remaining dead ends:** every edge leading only into a dead-end subtree is marked `dead_end` with the distance to its end. This includes cul-de-sacs, valley roads ending at a village, and pass roads ending at a summit.
  4. **Region borders:** edges cut by the package boundary are marked `boundary`. Neighbouring packages share stable edge IDs at the border, so an installed neighbour package continues the road seamlessly.

### 4.5 Coordinates, Tiling and Streaming

The application uses its own Bevy-based world rendering system.

- **World builder:** all computation in `f64`, projected per region into a **local transverse Mercator projection** (or the UTM zone) with metres as units.
- **Tiles:** 1 km × 1 km world tiles. Geometry is stored in `f32` **relative to the tile origin**, so precision stays well below 1 mm.
- **Floating origin at runtime:** when the camera is more than ~2 km from the render origin, the origin is shifted and all entities are re-offset. The `big_space` crate is evaluated as an alternative.
- **Physics** is unaffected because it runs in route distance (`f64`, §5.2).

Hierarchy: region → area → world tile → road sections, terrain mesh, building groups, vegetation groups.

**Streaming:**

- Only the required areas are loaded into memory. A 3 × 3 tile ring is kept at full detail, with lower-LOD rings beyond.
- While riding, neighbouring tiles are loaded on background tasks and tiles no longer needed are unloaded.

Goals:

- No downloads during a ride.
- Offline reuse of downloaded regions.
- Bounded memory consumption (§15).
- GPU-accelerated rendering.
- LOD for distant objects.
- Frustum culling.
- Instancing for vegetation and recurring objects.

### 4.6 Landscape Textures and Imagery

Ground textures are needed for different landscapes (meadow, field, forest floor, rock, sand, snow, urban, wetland, water edges), plus road surfaces (asphalt, gravel, cobbles, dirt).

**Approach (hybrid):**

1. **Close-up: tiling PBR materials.** Albedo, normal and roughness textures per surface type, blended by a per-tile **splat map**.
   - Sources: **CC0 libraries** (ambientCG, Poly Haven) plus our own materials.
   - Compressed as KTX2 / Basis Universal for the GPU.
2. **Splat map generation** (world builder): the surface type per terrain cell is chosen from:
   - OSM `landuse` / `natural` / `surface` tags (forest, meadow, farmland, residential, scree, …),
   - **ESA WorldCover** to fill gaps where OSM has no land-use polygons,
   - slope (steep → rock) and altitude (high → alpine grass / rock / snow),
   - noise for natural variation.
3. **Mid and far distance: satellite color tint.** A low-resolution color map (Sentinel-2 at 10 m) modulates the tiling materials, so fields, forests and villages get their real patchwork look from a distance without blur up close. 1 km² at 10 m is only 100 × 100 pixels (about 30 KB uncompressed), so even a 1,000 km² region adds only tens of MB.
4. **Optional orthophotos:** high-resolution orthophotos only as an optional download for selected regions, used for far-LOD color and for better land-cover classification at build time.
5. **Vegetation:** instanced tree, bush and grass models placed by land cover and density, so the 3D look comes from geometry rather than textures.
6. **Roads:** material from the OSM `surface` tag (asphalt, gravel, paving stones, …), with a fallback per road category. Markings are generated from `lanes` and the road category.

**Why satellite imagery is not the main ground texture:**

- It is a top-down photo with baked-in shadows, cars, tree crowns and seasonal colors. Seen from a bike at 1.7 m height it looks blurry and flat.
- At 20 cm resolution, 1 km² is 25 megapixels (roughly 10–25 MB compressed). A 1,000 km² region would be 10–25 GB, far above the package size target (§15).

**Permitted imagery and land-cover sources:**

| Source | Resolution | Licence | Usable? |
|---|---|---|---|
| **Copernicus Sentinel-2** (cloud-free mosaics, e.g. EOX Sentinel-2 cloudless) | 10 m | Copernicus data: free, attribution. Third-party mosaics have their own terms (EOX: CC BY-NC-SA for some years, commercial licence needed) | Yes; check the mosaic's licence |
| **ESA WorldCover** (land-cover classification, not imagery) | 10 m | CC BY 4.0 | Yes |
| **Official orthophotos** (e.g. German states' DOP20/DOP40, many under dl-de/by-2-0 or dl-de/zero-2-0; other EU countries similar) | 0.2–0.4 m | Open, varies by country/state | Yes, region by region |
| **USGS NAIP** (USA) | 0.6–1 m | Public domain | Yes |
| Google, Bing, Apple, Esri, Mapbox satellite layers | high | Proprietary; terms forbid extraction / offline use like this | **No** |

Every imagery and land-cover source used in a package is listed in its metadata and shown on the credits screen (§13).

### 4.7 Data Packages

Regions are stored as self-contained data packages.

Example:

```
worlds/europe/germany/berlin/
```

Contents:

- Geographic source data (the extracted road graph and attributes, §13).
- Terrain data.
- Road meshes.
- Building meshes.
- Vegetation.
- Materials, splat maps and color maps.
- Routes.
- Metadata, including source attributions.
- Version information.

Packages are **signed, versioned and updatable**:

- **Ed25519** signatures. An offline root key signs rotating online signing keys. The client ships the root public key.
- Each package manifest lists SHA-256 hashes of all files.
- **TUF** (The Update Framework) is evaluated for package and app updates.
- Target size: 200–500 MB for a mid-sized region.
- The format allows streaming over HTTP in tile-sized chunks (§3.5).

---

## 5. Bicycle Physics and Training Simulation

### 5.1 Physics Module

Bicycle physics is a standalone Rust crate, `ocw-sim`, with no Bevy dependency. It contains physics, rider state, drafting and session logic, and is used by the client (prediction), the server (authority) and offline mode (§6.1).

**Inputs:** power in watts, cadence, rider weight, bicycle weight (from the physics profile, §2.5), gradient, speed, wind, rolling resistance, air resistance, drafting position.

**Outputs:** speed, acceleration, position, energy expenditure, resistance, riding state.

Fundamental resistive forces:

- Gravitational force.
- Rolling resistance.
- Aerodynamic drag.
- Inertial (acceleration) resistance.

### 5.2 Simulation Model

- **Riders move in 1D along their path:** state = `distance_along_route: f64`, `lateral_offset: f32`, `speed: f64`. The 3D position is derived from the road geometry only for rendering. This makes the physics cheap, robust and easy to synchronize.
- In free ride the path is an **edge list that is extended on the fly** by each junction decision (§2.2.1). The lateral offset is blended across junctions so the avatar follows a smooth curve, not the raw graph corner.
- **Fixed timestep:** physics at **50 Hz** (20 ms) on client and server, independent of frame rate.
- **No strict bit-level determinism:** `f64` is used. The client predicts, receives authoritative state, and corrects small errors by smoothing (blending over ~0.5 s); it snaps only above a threshold (e.g. > 5 m). This avoids the cost of fixed-point math.
- **Drafting** (reduced CdA behind other riders) is computed server-side because it depends on other riders. The client predicts it from the interpolated positions of nearby riders.
- **Sharp turns** (> 120°): an automatic short slow-down is applied (the rider can't brake on a trainer), so the avatar does not take a hairpin at 40 km/h.
- **U-turns** (§2.2.2): slow-down to ~10 km/h, turn, continue in the opposite direction.

Training data is stored independently of rendering.

### 5.3 Trainer Control

Trainer control translates the virtual road into resistance for compatible smart trainers via the FTMS control point:

- *Set Indoor Bike Simulation Parameters* (grade, wind speed, Crr, Cw) for **SIM mode**.
- *Set Target Power* for **ERG mode** (workouts, §2.8).
- *Set Target Resistance Level* as a fallback.
- *Spin-down calibration* where supported.

**Grade sent to the trainer:**

- **Trainer difficulty** slider from 0–100 %, default 50 %, which scales the grade.
- Separate downhill handling (e.g. negative grades capped or scaled further).
- Rate limit (e.g. at most 1 %/s change) and a short look-ahead to avoid jolts.
- Updates sent at 1–2 Hz; most trainers do not handle faster updates well.

**ERG mode:** smoothing, and a "catch-up" on sudden cadence drops.

---

## 6. Real-Time Multiplayer Architecture

### 6.1 Roles: Client, Remote Server, Local Server

| Role | What it is | Where it runs | Physics responsibility |
|---|---|---|---|
| **Client** | The desktop app (`apps/desktop`): rendering, sensors, UI, audio | The user's PC | Runs `ocw-sim` for its **own rider only** as a *prediction*, so the bike reacts instantly to pedaling |
| **Remote server** | Our game server (`services/server`) | Our Linux hosts (Docker) | Runs `ocw-sim` for **all riders in a session**; its result is the truth (positions, drafting, race results) |
| **Local server** | The same server logic, linked into the desktop app | Inside the client process | Single-player / offline rides: it is the authority, no network involved |

The application uses a **server-authoritative** model. There is only one code path: single-player runs the same server logic in-process, and the desktop app talks to it through the protocol from Phase 1.

The trainer and sensors are always connected to the client only. The server never talks to hardware; it receives the measured values from the client.

**The client transmits** only inputs: timestamped power, cadence and heart-rate samples with sequence numbers, device information, steering inputs (§6.5) and riding state requests.

**The server computes:** rider state, position, speed, drafting, race state, rankings, group membership and events. It then distributes updated states to the participants.

### 6.2 Network Protocol

- **Tokio** as the asynchronous runtime.
- **Quinn** for QUIC-based communication (TLS 1.3).
- **Protobuf** via `prost` for all structured messages.
  - Schemas live in `proto/` and are versioned (`ocw.v1`). Changes follow protobuf compatibility rules: add fields, never reuse field numbers.
  - Position datagrams use compact quantized fields: distance in cm (`uint32`), speed in mm/s (`uint32`), lateral offset in cm (`sint32`), animation state (`uint32` enum).
  - A protocol version is exchanged during the handshake; incompatible clients are rejected with an update prompt.
- **Unreliable datagrams** for position updates; **reliable streams** for login, session management, steering intents and important events.
- The protocol is transport-independent, so a WebTransport endpoint can be added later (§3.5).

### 6.3 Synchronization

The simulation accounts for differing frame rates, differing network latencies, packet loss, connection drops, reconnection, server-side timestamps, interpolation of other riders and client-side prediction.

| Topic | Target |
|---|---|
| Riders per session (v1) | 100 |
| Design target | 1,000 per event through **interest management**: full-rate updates for riders within ±500 m, reduced rate beyond, aggregated (count / gap) for the rest |
| Server physics tick | 50 Hz |
| State broadcast | 10 Hz (enough for cycling with interpolation) |
| Interpolation delay | ~100–150 ms |
| Playable latency | up to 250 ms RTT |
| Downstream bandwidth | < 64 kbit/s per client in a 100-rider session |
| Reconnect | Session state kept on the server for 2 min; the rider resumes at the server-side position |

The multiplayer logic is developed independently of the graphics engine.

### 6.4 Data Integrity and Anti-Cheat

Goal: **detection and flagging of implausible data**. Power comes from user hardware and cannot be fully verified.

1. **Transport integrity:** authenticated QUIC (TLS 1.3), per-session tokens, sequence numbers and server timestamps (prevents replay and injection).
2. **Plausibility checks (server):**
   - Power-duration curve compared with the rider's history and with absolute W/kg limits.
   - Unrealistic spikes and slopes (power changes faster than physically possible).
   - Power without cadence, or perfectly constant power over long periods.
   - Heart rate not responding to load changes (when heart rate is present).
3. **Device trust levels:** smart trainer only < trainer + independent power meter ("dual recording") < verified setup. Organized races can require a minimum trust level.
4. **Weight:** the biggest real-world manipulation vector. Weight changes are logged; races may lock weight some time before the start.
5. **Consequences:** flagging, results marked "unverified", and manual review. No automatic bans in v1.

### 6.5 Steering in Multiplayer

- The client sends a reliable `TurnIntent { junction_id, exit_edge_id }` as soon as the choice changes (§2.2.1).
- The **server decides** at the lock point and broadcasts the chosen edge.
- The client predicts with its own choice and is corrected only in the rare case of disagreement (e.g. the intent arrived too late → the server default applies).
- In group rides with "follow leader", the server applies the leader's choice to all followers.

---

## 7. Voice Chat

Infrastructure: **LiveKit** (WebRTC SFU).

Features: voice rooms, participant management, audio transmission, dynamic subscription to audio tracks, push-to-talk, group communication.

- The client uses the official **LiveKit Rust SDK**. It wraps libwebrtc, but LiveKit provides prebuilt binaries for all target platforms.
- It is isolated in the `ocw-voice` crate behind a trait, with a cargo feature `voice`, so the app also builds and runs without it (CI, minimal builds).
- The **LiveKit server is self-hosted** (Apache 2.0) as a separate service. Our API issues short-lived LiveKit access tokens (JWT) per room.
- Voice is **not** transmitted over the QUIC game-state connection. Voice and game server are separate services with separate scaling.
- **Spatial audio** on the client:
  - Subscribe only to the nearest **N riders (e.g. 8–12)** plus explicitly chosen group members.
  - Attenuate volume by distance along the route (optionally pan by lateral offset).
  - Mix on the client.
- Echo cancellation and noise suppression come from the WebRTC audio processing module included in libwebrtc.
- Voice is **never recorded or stored** on our side.

---

## 8. Bicycle and Character Assets

### 8.1 Asset Pipeline

3D format: **glTF 2.0**. Models are created, for example, with Blender.

Import process:

1. Create model.
2. Define materials.
3. Create skeleton and animations.
4. Export glTF.
5. Validate assets (`tools/asset-validator`, including licence metadata, §13).
6. Import into Bevy.
7. Register components.

### 8.2 Bicycle Components

Bicycles consist of modular parts: frame, fork, handlebars, wheels, tires, saddle, pedals, drivetrain, accessories.

Each component has a unique ID, a configuration, a visual part and an optional physics profile reference (§2.5).

### 8.3 Characters

Characters use a shared rig. Body, clothing and accessories are loaded modularly. Animations are driven by the riding state.

---

## 9. Backend and Infrastructure

The desktop application is the actual simulation client. Server services handle only tasks that must be centrally managed or synchronized between participants.

| Area | Technology |
|---|---|
| Language | Rust |
| Async runtime | Tokio |
| HTTP API | Axum |
| Web companion | Leptos (SSR on Axum) |
| Real-time networking | Quinn / QUIC |
| Data format | Protobuf (`prost`) |
| Database | PostgreSQL |
| Cache | Redis (optional) |
| Voice | LiveKit (self-hosted) |
| Authentication | OIDC |
| Telemetry | OpenTelemetry |
| Containers | Docker |
| Deployment | Linux servers, hosted in the EU |

Backend modules:

- Authentication Service.
- User Service.
- Profile Service.
- Device Service.
- Multiplayer Session Service (game server).
- Race Management.
- Friends and Groups.
- World Metadata Service.
- Asset and World Distribution.
- Training History.
- Strava Integration.
- Voice Room Management (LiveKit tokens).
- Web Companion (route group).

The first version is a **modular backend monolith** (`services/server`). A split into microservices only makes sense once there is an actual scaling need. LiveKit runs as a separate service.

---

## 10. Cargo Workspace and Repository Structure

The workspace starts with about 8 crates; crates are split only when they grow.

**Dependency rule:** only `ocw-render` and `apps/desktop` depend on Bevy. `ocw-core`, `ocw-sim`, `ocw-protocol` and `ocw-world` must also build for `wasm32-unknown-unknown` (§3.5).

```
open-cycling-world/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── apps/
│   ├── desktop/          # Bevy app, wires all plugins together
│   ├── world-builder/    # CLI: OSM + DEM → world packages, route editor
│   └── web-companion/    # browser app without riding (Leptos)
├── services/
│   └── server/           # modular monolith: Axum API + QUIC game server
├── crates/
│   ├── ocw-core/         # shared types, units, IDs, config
│   ├── ocw-sim/          # physics, rider state, drafting, session logic (no Bevy)
│   ├── ocw-protocol/     # prost-generated messages + framing (no Bevy)
│   ├── ocw-devices/      # DeviceBackend trait, BLE/FTMS backend (no Bevy)
│   ├── ocw-world/        # package format, tiles, road graph, routes, map matching (no Bevy)
│   ├── ocw-worldgen/     # OSM/DEM import, smoothing, meshing (used by world-builder)
│   ├── ocw-render/       # Bevy plugins: terrain, roads, avatars, bikes, HUD, console
│   └── ocw-voice/        # LiveKit integration behind a trait (feature "voice")
├── proto/                # .proto schemas
├── assets/
│   ├── bicycles/
│   ├── characters/
│   ├── animations/
│   ├── materials/
│   ├── sounds/
│   └── shaders/
├── worlds/
├── tools/
│   ├── asset-validator/
│   ├── osm-converter/
│   └── performance-test/
├── infrastructure/
│   ├── docker/
│   └── deployment/
└── .github/
    └── workflows/
```

Mapping from the original (v1.0) layout:

| Original crate(s) | New crate |
|---|---|
| `cycling-protocol`, `multiplayer` (protocol part) | `ocw-protocol` |
| `cycling-physics`, `simulation`, `training` (sim part) | `ocw-sim` |
| `device-manager`, `bluetooth-ftms` | `ocw-devices` |
| `osm-import`, `terrain`, `road-generator` | `ocw-worldgen` |
| `world` | `ocw-world` |
| `renderer`, `avatar`, `bicycle`, `*-editor` | `ocw-render` and `apps/desktop` |
| `voice` | `ocw-voice` |
| `core`, `platform`, `telemetry` | `ocw-core` (split out when needed) |
| `api`, `game-server`, `account-service`, `world-distribution` | `services/server` (modules) |
| `ant-plus` | deferred; later a second backend in `ocw-devices` |

---

## 11. Cross-Platform Build System

Target platforms:

- Linux x86_64.
- Windows x86_64.
- macOS Apple Silicon.
- Optional: Linux ARM64, macOS Intel.

Each platform gets a native CI runner.

### 11.1 Platform Rollout Order

Platforms are implemented one after another, not in parallel:

| Order | Platform | Graphics backend | Bluetooth stack | Packaging |
|---|---|---|---|---|
| 1 | **Linux** x86_64 | Vulkan | BlueZ (D-Bus) | AppImage, Flatpak |
| 2 | **Windows** x86_64 | Direct3D 12 (Vulkan as fallback) | WinRT BLE | EXE/MSI installer, code signing |
| 3 | **macOS** Apple Silicon | Metal | CoreBluetooth | App bundle, DMG, code signing, notarization |

- **Linux is the primary development and test platform** from Phase 1 on. All features are first implemented, tested and released on Linux.
- **Windows** follows once the single-player core runs on Linux (end of Phase 3: real trainer + OSM world).
- **macOS** follows after Windows, before the release-ready milestone (Phase 7).
- Optional targets (Linux ARM64, macOS Intel) come after their main platform.
- The **web companion** is developed alongside, starting in Phase 5 (§2.10).
- **Future development (not scheduled):** full web client and native mobile apps (§1.3).
- **Portability from day one:** platform-specific code stays behind traits (`ocw-devices`, `ocw-core`). From Phase 1, CI also runs a build-only check (`cargo check` / `cargo clippy`) for Windows and macOS, and a `wasm32-unknown-unknown` build of the core crates.

### 11.2 Build Process

1. Install Rust toolchain.
2. Resolve dependencies.
3. Format code.
4. Run Clippy.
5. Run unit tests.
6. Run integration tests.
7. Create release build.
8. Package assets.
9. Create installers.
10. Publish artifacts.

### 11.3 Packaging

**Linux:** AppImage, Flatpak, optionally native packages.

**Windows:** EXE/MSI, code signing, installer.

**macOS:** app bundle, code signing, Apple notarization, DMG.

Build and release pipelines are automated for all three operating systems, introduced in the rollout order (§11.1).

---

## 12. Security and Privacy

Particularly important for multiplayer and voice communication:

- Encrypted network connections (QUIC / TLS 1.3, HTTPS).
- Authentication (OIDC).
- Server-side authorization.
- Detection of manipulated training data (§6.4).
- Protection against replay attacks (sequence numbers, server timestamps, per-session tokens).
- Rate limiting.
- Secure token management (Strava and LiveKit tokens handled only by the backend).
- Microphone permissions.
- Visible microphone activity.
- Muting.
- Privacy-friendly telemetry.
- Signed world packages and updates (§4.7).

**Data separation and GDPR:**

| Data | Handling |
|---|---|
| Identity | Account schema |
| Training data | Separate schema / service module |
| Voice | Only relayed, never recorded or stored |
| Telemetry | Opt-in, pseudonymized |
| User rights | Data export (FIT + JSON), account deletion including training data |
| Hosting | EU |

Personal data, voice data and training data are handled separately.

---

## 13. Licensing and Open-Source Strategy

| Item | Classification | Licence / Obligation |
|---|---|---|
| Source code | Own work | **Dual MIT / Apache-2.0** |
| Own 3D assets, sounds | Own work | Decided separately; suggestion **CC BY 4.0**, or proprietary if monetized |
| Third-party assets | External | Tracked in each asset's manifest (`license`, `source`, `author`); the asset validator rejects assets without a licence entry |
| Road graph + attributes in world packages | **Derivative Database** (ODbL) | Published under ODbL; extracted data offered for download alongside each package |
| Meshes, textures, rendered views | **Produced Work** (ODbL) | Attribution "© OpenStreetMap contributors" |
| DEM-derived terrain | Depends on source | Each source's required attribution notice (Copernicus, regional providers) |
| Imagery and land cover | Depends on source | Only sources permitted in §4.6; attribution per source |

- **Attribution in the app:** loading screen, about / credits screen, the in-ride minimap and the web companion map.
- **No use of `tile.openstreetmap.org`** or other public OSM services at runtime. The use of public OSM tile servers must not be confused with the free use of the raw data. Data comes from Geofabrik extracts at build time; packages and vector tiles are served from our own infrastructure.
- Geographic data and rendering software are considered separately from a licensing perspective.
- Before commercial release, the data and asset licences must be reviewed by a lawyer.

---

## 14. Development Phases

### Phase 1 – Technical Foundation

- Cargo workspace (§10).
- Bevy application, rendering, camera, simple test route.
- `ocw-sim`, `ocw-protocol` and a headless server skeleton. The desktop app talks to the in-process local server through the protocol from day one.
- In-game console (§2.9).
- Graphics engine spike (§3.1.1).
- Linux build; build-only CI checks for Windows and macOS; wasm build of the core crates.

**Result:** A launchable Linux application on the final client/server architecture.

### Phase 2 – Bicycle and Sensors

- Bluetooth scanning (BlueZ).
- FTMS.
- Power, cadence and heart rate.
- Bicycle physics and drafting model in `ocw-sim`.
- Virtual resistance: SIM mode with trainer difficulty (§5.3).
- Basic workouts and ERG mode.
- FIT export.

**Result:** Ride with a real smart trainer.

### Phase 3 – OpenStreetMap World

- PBF import and road extraction.
- Road graph with stable edge IDs, graph cleanup and dead-end flags (§4.4).
- Elevation model and road profile smoothing (§4.3).
- Terrain generation, road-conformed.
- Road meshes, splat materials and color maps (§4.6).
- Coordinates, tiles, streaming, floating origin (§4.5).
- Local caching and regional data packages (§4.7).
- Route format and route editor; GPX import (§2.7).
- Junction steering and dead-end handling (§2.2.1, §2.2.2).

**Result:** Ride a geographically realistic route locally in 3D. **Windows port starts.**

### Phase 4 – Avatar and Bicycle Editor

- Bicycle models.
- Character models.
- Components and physics profiles (§2.5).
- Animations.
- Configuration and persistence.

**Result:** Individually customizable riders.

### Phase 5 – Multiplayer

- Accounts.
- Sessions.
- Networked server: the existing local-server logic runs remotely.
- Real-time synchronization and interest management (§6.3).
- Other riders.
- Group rides (including "follow leader").
- Races.
- Leaderboards.
- Steering intents (§6.5).
- Plausibility checks and trust levels (§6.4).
- Strava upload (§2.8).
- Web companion, first version (§2.10).

**Result:** Shared virtual rides.

### Phase 6 – Voice Chat

- LiveKit (self-hosted) and the LiveKit Rust SDK.
- WebRTC.
- Microphone management.
- Push-to-talk.
- Voice rooms.
- Spatial audio.

**Result:** Shared rides with voice communication.

### Phase 7 – Production Readiness

- Performance optimization.
- Large regions.
- Weather and times of day.
- Training history.
- Events.
- Full workout and ERG support.
- Web companion: events, routes, world packages.
- Error monitoring.
- Installers and updates (signed, TUF evaluated).
- Security audits.
- macOS port.

**Result:** A release-ready indoor cycling platform on Linux, Windows and macOS.

---

## 15. Quality Goals

| Topic | Target |
|---|---|
| Frame rate | 60 FPS on suitable desktop hardware |
| Minimum hardware | 4-core CPU, 8 GB RAM, GPU with Vulkan 1.2 / DX12 / Metal and 2 GB VRAM |
| Memory budget | ≤ 2 GB RAM and ≤ 1.5 GB VRAM at default settings |
| Disk per region | 200–500 MB for a mid-sized region |
| Time steps | Physics (50 Hz) decoupled from rendering |
| Multiplayer | Smooth rendering of other riders; robust reconnection; targets in §6.3 |
| Offline | Offline playback of downloaded routes |
| Streaming | Bounded memory consumption through world streaming; no hitches during tile loading |
| Graphics APIs | No dependency on a single GPU API |
| Platforms | Consistent behavior on Linux, Windows and macOS; automated tests on all three, added per platform in the rollout order (§11.1) |

These values are development targets, not guaranteed performance figures.

---

## 16. Architecture Summary

| Component | Selected Technology |
|---|---|
| Programming language | Rust |
| 3D engine | Bevy (pinned version); fallback Godot + gdext |
| Rendering | wgpu (Vulkan, D3D12, Metal) |
| GUI | Bevy UI |
| Supplementary GUI | egui (editors, console, diagnostics) |
| Game audio | Bevy Audio |
| Voice chat | WebRTC / LiveKit (Rust SDK, self-hosted server) |
| Bluetooth | btleplug |
| ANT+ | Deferred |
| Async | Tokio |
| Networking | QUIC / Quinn |
| API | Axum |
| Web companion | Leptos, MapLibre GL JS, PMTiles |
| Data format | Protobuf (`prost`) |
| Database | PostgreSQL |
| Cache | Redis (optional) |
| Geodata | OpenStreetMap PBF |
| Elevation model | Regional open DEM / Copernicus DEM / SRTM |
| Land cover / imagery | ESA WorldCover, Sentinel-2, open orthophotos |
| Ground materials | CC0 PBR libraries + own, KTX2 / Basis Universal |
| 3D asset format | glTF 2.0 |
| Modeling | Blender |
| Package signing | Ed25519, SHA-256 manifests |
| CI/CD | GitHub Actions |
| Deployment | Docker / Linux, EU hosting |

---

## Conclusion

Open Cycling World is conceived as an independent, Rust-based simulation platform.

The desktop client handles rendering, local world management, prediction of the local rider, device communication and user interaction. For offline rides it also runs the server logic in-process.

The backend handles identity, the authoritative multiplayer simulation, synchronization, groups, races, voice rooms, Strava integration, the web companion and data distribution.

The geographic world is generated centrally from OpenStreetMap vector data and supplementary elevation and land-cover data, distributed as signed packages, stored locally, and rendered on the respective hardware.

The architecture is deliberately modular, so that individual components can later be replaced or extended without redeveloping the entire application.

**Primary development goal:** A technically independent, cross-platform indoor cycling system with real routes, real bicycle hardware, individual customization and synchronized multiplayer.

---

## Appendix A – Decision Record

The specification review produced proposals P-1 … P-20 (formerly `PROPOSALS.md`). All were accepted and merged into version 1.1:

| ID | Topic | Section |
|---|---|---|
| P-1 | Simulation authority, roles, shared physics | §5.2, §6.1 |
| P-2 | Protobuf message format | §6.2 |
| P-3 | Revised phase plan | §14 |
| P-4 | Data integrity / anti-cheat | §6.4 |
| P-5 | Cosmetics vs. performance | §2.5 |
| P-6 | Elevation quality | §4.3 |
| P-7 | Coordinate system and tiling | §4.5 |
| P-8 | Trainer control | §5.3 |
| P-9 | Licensing | §13 |
| P-10 | Voice chat integration | §7 |
| P-11 | Platform topics (Bevy versioning, UI, Bluetooth) | §3.1, §3.2, §3.4 |
| P-12 | Non-functional requirements, privacy, package signing | §4.7, §6.3, §12, §15 |
| P-13 | Routes, stable edge IDs, FIT export, drafting | §2.7, §2.8, §4.4, §5.2 |
| P-13a | GPX import | §2.7 |
| P-13b | Strava upload | §2.8 |
| P-14 | Repository layout | §10 |
| P-15 | World data flow | §4.2 |
| P-16 | Landscape textures and imagery | §4.6 |
| P-17 | In-game console | §2.9 |
| P-18 | 3D graphics stack evaluation | §3.1.1 |
| P-19 | Junction steering and dead ends | §2.2.1, §2.2.2, §4.4, §6.5 |
| P-20 | Web companion and future clients | §1.2, §2.10, §3.5, §3.6 |
