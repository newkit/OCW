# Open Cycling World

**Cross-Platform Indoor Cycling Simulation**

Project Description and Technical Architecture

Version 2.3 – November 2026

| Version | Change |
|---|---|
| 1.0 | Original specification (translated from German) |
| 1.1 | Decisions from the specification review merged in (former PROPOSALS.md, P-1 … P-20; see Appendix A) |
| 1.2 | Testing strategy (§11.4); unit tests required for every development step (§14) |
| 1.3 | Graphics engine spike results (§3.1.1): Bevy confirmed on Linux; trees merged per tile |
| 1.4 | Wahoo's proprietary trainer control as a fallback for trainers without FTMS (§3.4, §5.3) |
| 1.5 | Lateral steering: autodrive, manual lean steering, road edge, wild mode; surfaces with their own rolling resistance (§5.2.1) |
| 1.6 | FIT export: own writer crate `ocw-fit`, rides saved in the user data directory; interim direct Strava upload from the desktop with user-supplied API credentials until the backend exists (§2.8) |
| 1.7 | Free ride on the road graph: junction keys next to lean steering, U-turn key and loop (§2.2.1, §2.2.2); road graph in the world package, format 2 (§4.7); free-ride path and slow-downs (§5.2); `TurnIntent`, `UTurn`, `PathUpdate` (§6.5) |
| 1.8 | Steering fixes (§5.2.1): only pushing into the road edge scrapes; a held opposite lean key pauses at upright, then leans on to the other side |
| 1.9 | Catch-up of implemented decisions: elevation filter along through-roads and terrain cut under roads (§4.3), road surfaces (§4.4), route file format (§4.7), console scripting (§2.9), input devices incl. gamepad, camera and avatar on the terrain (§3.2), start point and bots (§6.1), repository layout (§10), build machines, desktop integration, AppImage, release and build-history folders, CI resilience (§11), own artwork licence (§13) |
| 2.0 | Restart key R, camera reset on C, video recording on V; gamepad ↑ / ↓ also on the D-pad and the left stick, analog triggers, U-turn on East (§3.2) |
| 2.1 | In-game menu plan (§3.2.1): structure (Ride, Workshop, Navi, Settings), gamepad navigation, settings file, increments M1–M9 mapped to phases; audio output device plan (§3.3) |
| 2.2 | Menu decisions (§3.2.1): the ride never pauses (trainer unchanged, time and recording continue, autodrive while the menu is open); Esc never quits; rider weight affects physics, rider profiles in Settings, chosen in the Workshop |
| 2.3 | macOS app bundle (§11.3): `scripts/build-macos-app.sh` (Info.plist with Bluetooth usage text, icon, ad-hoc signature, DMG and zip), logs to `~/Library/Logs/OCW/` when started from Finder, CI job building the app on pushes to main |

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
- **Lock point:** ~20 m before the junction the choice is locked; key presses after that apply to the following junction. (v1.6: earlier at speed – 20 m plus the distance needed to slow down from the current speed to 15 km/h at 2 m/s², so the slow-down for the chosen turn (§5.2) fits; e.g. ~100 m at 65 km/h.)
- **Down arrow:** U-turn (§2.2.2). Only in free ride; rate-limited (e.g. once per 10 s).
- All bindings are remappable. Later, gamepad and handlebar controller buttons map to the same actions (`turn_left`, `turn_right`, `u_turn`).

**Decided key bindings (v1.6).** The arrow keys also steer (§5.2.1) and ↑ / ↓ set the keyboard power and brake, so the defaults are:

- With **autodrive**, ← / → choose the exit while a junction is **active** (within the look-ahead and before the lock point): one exit per press, and they do not move the line meanwhile. Otherwise they move the line as usual.
- With **manual** (lean) steering the arrows keep leaning – the rider has to steer the turn anyway. **Q / E** choose the exit in every mode, also before the junction is active (pre-selection).
- **U** is the U-turn key (↓ stays the brake). Gamepad: D-pad left / right / down = `turn_left` / `turn_right` / `u_turn`.
- Console: `ride.turn left|right|straight|default|+|-|<n>`, `ride.uturn`, `ride.junction` (for scripted rides).
- The overlay's arrow turns solid green once the choice is locked; the junction is identified by its graph vertex, the exit by its stable edge ID (§6.5).

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

**Route rides on the road graph (decided).** A route of the package (§2.7) is ridden with the free-ride path machinery (§5.2): the path is extended with the route's next step instead of the default exit, so the same junctions, rounded corners, slow-downs and U-turns apply.

- **Arrows:** at each junction the route's exit is the default and drawn in blue; it turns green when locked like any choice. The label says `[route]`.
- **Leave route:** in solo rides the turn keys stay active. Selecting another exit marks it `[leaves the route]` in the overlay (the confirmation); once it locks, the rider is off the route and free rides. The HUD shows "Off the route – press J (`ride.rejoin`) to return".
- **Rejoin:** **J** (gamepad north) or `ride.rejoin` keeps the path up to the first vertex beyond the lock point and adds the shortest way (one-way roads respected, no U-turn) to a route step from where the rider left on, weighing skipped route at 0.3 per metre against the detour. The rider also rejoins by riding onto a later route edge in the route's direction. The client sends `RouteRejoin { distance, speed }` (like `UTurn`), so the server finds the same way.
- **Bots** always take the route's exit.
- **Route state is derived from the path:** a step is on the route when it rides the route's next step (same edge, same direction), or – off the route – a later route step. Server and clients agree on it from the step list alone; no extra state is sent.
- After the finish of a route that is not a loop, the ride continues as a free ride. Loops are ridden in laps.
- The HUD shows the route name, distance done and length (and the lap); `ride.route` prints it.

Simulation and network aspects of steering are described in §5.2 and §6.5.

#### 2.2.2 Dead-End Roads

**At build time** (world builder, §4.4): data artefacts are removed, real dead ends are kept and flagged.

**At runtime:**

- **Prevention:** dead-end exits show a dead-end icon in the overlay and are never the default. While riding into a dead end, the HUD shows "Dead end in 800 m" once it is within 1 km.
- **Automatic U-turn at the end:** the rider slows to ~10 km/h, the avatar turns around in a short loop, and the ride continues on the same road in the opposite direction. Power and physics simply continue, so no workout or ride is interrupted.
- **Manual U-turn** at any time with the Down arrow key (free ride only), using the same animation.
- **The loop (v1.6):** a U-turn is a "bulb" of radius 4 m (60° right, 300° left, 60° right – it ends where it began, heading the other way, about 30 m long). The rider slows down automatically to ~10 km/h before it and is steered through it (also with manual lean steering). At a dead end the loop starts ~12 m before the road's end, so it stays on the road; a manual U-turn starts after the slow-down distance. The path then continues on the same edge in the other direction.
- **The loop stays on the road (decided):** the bulb is `2 r` wide and centred on the centreline, so its radius comes from the road width: `r = clamp(width / 2 − 0.4 m, 1.5 m, 4 m)` (5 m road: 2.1 m). It is ridden at `min(10 km/h, √(2.5 m/s² · r))` (~8 km/h on a 5 m road). On the last 20 m before it and in it the rider is steered to the centreline (autodrive with offset 0, also for lean steering), because a rider keeping a lane offset would ride a loop of radius `r + offset`. A manual U-turn starts at least these 20 m ahead. On paths narrower than ~3.8 m the 1.5 m loop still overhangs slightly (walking pace). Regression test: `u_turns_stay_on_the_road` (`ocw-sim`).
- **Region border:** if the neighbouring package is installed, the ride continues across the border without interruption. Otherwise there is an automatic U-turn with the message "End of region – download <neighbour>". The download happens only after the ride; nothing is downloaded during a ride (§4.2).
- **Neighbouring packages (data model decided, runtime pending):** the builder cuts a road at the region boundary and marks the cut end as a boundary vertex in both packages. `ocw_world::border::match_border` pairs the boundary vertices of two packages that are the same road (same OSM way or stable edge ID) at the same geographic point (within 8 m; each package keeps its own projection, points are compared through latitude / longitude). `join_graphs` builds one road graph in the first package's metres with the linked vertices merged and no longer boundary, so a free-ride path continues across (unit-tested); the neighbour's own outer border stays a border. Still to do: the client loads both packages (graph joined at start; tiles of the second package streamed with an offset into the first one's frame), and the server rides the joined graph – the Welcome then names both packages (§6.5). Until then a border always turns the rider round.
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

**Implementation (decided):**

- **Route format:** `GraphRoute` = name, loop flag, steps: `edge <stable ID> <from m> <to m>` (metres from the edge's first vertex, `from > to` = against it) or `turn` (an explicit U-turn, required for out-and-back routes, §2.2.2). The first step may start and the last end part-way along an edge. Stored in the package as the tagged section `ROUT` (§4.7), or as a standalone text file `ocw-graph-route 1` that names its package and graph fingerprint (`package NAME HASH`). Lap count, start/finish markers and segments (sprints, KOMs) come later as more lines / fields. Binary in the package and plain text outside instead of TOML/JSON: no new dependencies in `ocw-world`, and the file stays readable.
- **Resolving** a stored route against a graph checks every stable ID and the continuity of the steps (each starts where the previous one ends; a U-turn reverses the same edge) and reports the first break.
- **GPX parsing:** a small hand-written scanner in `ocw-world` (`gpx.rs`) instead of the `gpx` crate, so `ocw-world` stays dependency-free and wasm-safe. GPX 1.0 / 1.1, tracks (segments joined) and routes, namespace prefixes, comments, CDATA, entities.
- **Map matching** (`matching.rs`): points thinned to ≥ 10 m; up to 6 edges within 50 m per point as states, in each direction a one-way road allows; emission Gaussian (σ 10 m); transition `exp(−|road distance − straight distance| / 25 m)` with the road distance from a bounded Dijkstra (3 × straight distance + 300 m) over riding directions, no turning back at vertices; a U-turn on an edge costs 60 m extra; backwards jitter of up to 8 m on one edge costs 3 × its length. Points without a road within 50 m, or not connected to the previous point, are skipped and reported; more than 1.5 km of skipped track is an error ("the roads of this package do not connect the track near point N"). A track ending within 50 m of its start on the same edge becomes a loop starting at that edge's vertex. The report lists stretches more than 30 m from the matched road and skipped points.
- **Tools:** `world-builder gpx --world PKG --gpx FILE [--name N] [--route-file F] [--dry-run]` matches and stores a route in the package; `world-builder route --world PKG --from LAT,LON --to LAT,LON --name N` stores the shortest main-road route; `world-builder routes --world PKG [--from OLD] [--remove N] [--export-gpx]` lists and checks the routes (and carries them over from an older build, §4.4). The desktop app rides one with `--world PKG --route NAME` (or `--route FILE` for an `ocw-graph-route` file) or matches a GPX file at the start with `--route-gpx FILE`; the headless server takes `--world PKG --route NAME`.
- **Elevation** comes from the package's smoothed road profiles (step 4); GPX elevation is read but not used yet.

### 2.8 Training, Export and Strava

- **Structured workouts:** our own open workout format (TOML/JSON: intervals, targets as % FTP, cadence hints). Import of the open formats `.erg` / `.mrc` and FIT workout files. Workouts use ERG mode (§5.3).
- **Export:** every ride is saved as a **FIT** activity file (power, cadence, heart rate, speed, distance, elevation, virtual GPS track). Ridden routes can also be exported as **GPX**. A FIT writer is needed; existing Rust crates mainly read FIT.
  - **Implementation (v1.6):** our own dependency-free, wasm-safe writer crate `ocw-fit` (file_id, device_info, timer events, 1 Hz records, lap, session, activity; sport cycling, sub-sport virtual activity). The desktop client records the local rider once per second and saves the ride on exit (rides under one minute are dropped) or with `ride.save` to `<user data dir>/ocw/rides/<UTC start>.fit`, with a `.toml` sidecar (title, description). The virtual GPS track is the road position unprojected with the route's geographic origin (`ocw_world::geo`); procedural test routes have none.
- **Strava upload for finished rides:**
  - **Account linking via our backend:** Strava's OAuth flow requires a client secret, which must not ship in the desktop app. The backend handles the OAuth exchange and stores the refresh tokens encrypted. Tokens never reach the desktop app or the browser.
  - **Upload flow:** when a ride ends, the client writes the FIT file and sends it to our API. The API uploads it via the Strava `uploads` endpoint as `VirtualRide`, polls Strava for the upload status and reports the result to the client.
  - **Options:** automatic upload after each ride, or a manual "Upload" button in the training history. Title and description can be edited; a default title is generated (route name, world).
  - **Offline:** uploads are queued and sent later.
  - **Constraints:** Strava API terms and rate limits apply. Beyond a small number of athletes, Strava requires an app review – apply early. The virtual GPS track uses real coordinates of the route; this is common practice for virtual rides, but it must be labelled as virtual.
  - Other services (intervals.icu, TrainingPeaks) may follow later.
  - **Interim desktop upload (v1.6, until the backend's Strava integration in Phase 5):** a user may register their *own* Strava API application and give the desktop app its client id and secret (`OCW_STRAVA_CLIENT_ID` / `OCW_STRAVA_CLIENT_SECRET` or `<user config dir>/ocw/strava.toml`). No credentials ship with the app; without them the feature is off. The client then runs the OAuth authorization-code flow itself (browser, redirect to `http://127.0.0.1:<port>/strava/callback`), stores the tokens in `<user config dir>/ocw/strava-token.toml` (mode 0600) and refreshes them. A background thread uploads every saved ride not yet on Strava (`VirtualRide`, trainer), polls the upload status and keeps a sync state (`rides/strava-sync.toml`: uploaded / processing / failed with exponential backoff / rejected). Console: `strava.connect`, `strava.sync`, `strava.status`, `strava.disconnect`. Once the backend handles Strava, this path is replaced by the upload to our API described above.

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
- **Implemented (v1.9):** the commands listed in the README (rider, camera, trainer and devices, rides and Strava, `teleport`, `screenshot`, …). Scripted test rides use `--cmd COMMAND` (startup), `--exec FILE` and `--at SECONDS=COMMAND` (run once the ride has lasted that long, e.g. `--at "4=rider.lean 15"`).
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

**Spike results, Linux (November 2026).** Implementation: `apps/spike` (`ocw-spike`), run with `drive.sh spike` / `spike-video`.

- **Setup:** generated 10 × 10 km world (noise terrain standing in for the DEM, a 27.8 km winding road loop, road-conformed terrain), 1 km tiles in a 5 × 5 ring with three terrain LODs (10/25/50 m grids), built on background threads with one tile upload per frame, floating origin (rebasing per 1 km), about 140,000 trees in view, 100 riders, splat terrain material (four texture layers), cascaded shadows, fog.
- **Machine:** AMD Radeon RX Vega 56/64 (8 GB, RADV / Vulkan), 12-core EPYC, Debian 13, 1920 × 1080, vsync off, headless Wayland compositor. This GPU is clearly above the minimum spec of §15.

| Variant (40 km/h unless noted) | Mean FPS | p99 frame | Max frame | Frames > 16.7 ms | Frames > 33 ms |
|---|---|---|---|---|---|
| One entity per tree (`VisibilityRange` LOD), 10 km | 220 | 8.9 ms | 43 ms | 225 of 198k | 4 |
| **Trees merged per tile (per-tile LOD), 10 km** | **447** | **5.5 ms** | **11.4 ms** | **0** | **0** |
| One entity per tree, 150 km/h | 203 | 10.5 ms | 45 ms | 160 | 5 |
| **Trees merged per tile, 150 km/h** | **298** | **7.1 ms** | **15.6 ms** | **0** | **0** |
| Merged + splat material, 2 km | 387 | 6.6 ms | 11.7 ms | 0 | 0 |

Tile builds take up to 23 ms on worker threads; uploads cost at most 4.6 ms on the main thread; initial load of the 25 tiles 0.3–0.4 s; process RAM 330–470 MB; VRAM about +360 MB. Single runs vary by ±30 % (the GPU is shared with a desktop session), so only differences larger than that count.

**Decisions from the spike:**

- **Go for Bevy on Linux:** the go criterion (stable 60 FPS, no visible hitches while streaming) is met with headroom.
- **Vegetation is merged per tile and LOD**, not spawned as one entity per tree: spawning ~8,000 tree entities with a tile causes 40–45 ms hitches. Per-tile merged meshes (full trees near, impostors far) have no frame above 16.7 ms even at 150 km/h.
- **Splat terrain material** (spec §4.6) as a Bevy `ExtendedMaterial` with a WESL shader: no measurable cost.
- **Floating origin** with per-tile rebasing works; `big_space` is not needed for this world size.
- Tiles are built off the main thread and uploaded with a per-frame budget; this pattern carries over to the world streaming of Phase 3.

**Still open:** a run on minimum-spec hardware (§15, GTX 1050 / RX 560 class); skeletal animation (needs a rigged rider model, Phase 4); Windows (D3D12) and macOS (Metal) at the start of each port (§11.1); textures from real CC0 sources instead of procedural ones.

### 3.2 User Interface

- **Bevy UI** for the in-ride HUD (including the junction overlay, §2.2.1) and the main menus. The user interface is connected as directly as possible to the engine state.
- **egui** (`bevy_egui`) for the editor parameter panels, the in-game console (§2.9) and development/diagnostic interfaces, to keep development fast. The editors are reviewed after Phase 4.

**Input devices (v1.9):** keyboard and gamepads (Bevy's gamepad support, gilrs: Xbox, PlayStation, Switch Pro and generic pads, hot-plugged). Keyboard: ↑ adds power (25 W per press, more while held, on top of any device), ↓ stops (power 0 and brake), ← / → steer (§5.2.1) or choose exits (§2.2.1), A autodrive, W wild mode, U U-turn, Q / E exits, PgUp / PgDn ±10 W, F11 next screen, C camera reset, R restart (v2.0), V video (v2.0). Gamepad: left stick left / right steers (sets the lean directly, centred = upright); ↑ = D-pad up, left stick up or right trigger, ↓ = D-pad down, left stick down or left trigger (triggers and stick read as analog values, v2.0); South = autodrive, West = wild, D-pad left / right = exits, East = U-turn (v2.0). Several pads: the largest stick deflection steers.

**Restart and video (v2.0):** R (console `reset`) starts the whole app again as at program start, with the same command line: the app exits normally (ride saved, devices shut down) and replaces its process with a fresh one (Unix `exec`, same process id for the kiosk). V (console `video`) records the window: screenshots at up to 25 fps streamed as raw pixels into `ffmpeg` (H.264 MP4 in the user's video directory); slow frames are repeated to keep real time, a running recording is finished on exit.

**Camera and avatar (v1.9):** the chase camera follows the avatar itself (on the terrain when off the road) along the rider's own heading; it can be orbited with the mouse. It stays above the ground, never inside a tree crown, and lifts until the line of sight to the rider clears terrain and trees; its look-at point is smoothed. The avatar stands on the surface under both wheel contacts (front wheel first over a bump), leans by atan(v² κ / g) for the curve it rides, and is blended from road height onto the ground over 2 m beside the road.

The interface is divided into modules:

- Dashboard.
- Training view.
- Multiplayer lobby.
- Device management.
- Bicycle editor.
- Character editor.
- Settings.
- Route selection.

These modules are reached through the in-game menu (§3.2.1); the dashboard of §2.1 is its start page once accounts exist (Phase 5).

#### 3.2.1 In-Game Menu

One menu, reachable during a ride and (later) as the start screen, replaces command-line options and console commands for everyday use. The console (§2.9) stays the power-user path; both drive the same functions.

**Opening and pausing**

| | Keyboard / mouse | Gamepad |
|---|---|---|
| Open / close | Esc (closes the console first if it is open) | Start / Menu (☰, Options) |
| Move focus | ↑ ↓ ← →, Tab / Shift+Tab, mouse hover | D-pad, left stick (with repeat) |
| Confirm | Enter, Space, click | South (A / ✕) |
| Back (one page up; closes at the top) | Esc, Backspace, right click | East (B / ○) |
| Change a value (slider, choice) | ← / → on the focused row, mouse drag / wheel | D-pad / stick left / right, shoulder buttons for big steps |
| Switch tab | Q / E, PgUp / PgDn | LB / RB (L1 / R1) |

- While the menu is open, ride inputs (↑ power, ← → steering, A, W, U, R, V, …) are **not** passed to the ride; one input-context resource (`Ride`, `Menu`, `Console`) replaces today's per-system `console.is_open()` checks. The power from the trainer keeps flowing.
- **The ride never pauses (decided, v2.2):** the menu is an overlay over the running ride, in single-player exactly as in multiplayer (in a shared ride the others keep riding, so one rider's menu cannot stop time). The trainer is left as it is (SIM keeps following the road, ERG keeps its target), the ride time and the FIT recording continue. While the menu is open, steering falls back to autodrive and junctions take the default exit (§2.2.1), so the rider stays on the road; the previous steering mode is restored on closing. Pages that would end the ride (Ride, Restart) ask for confirmation.
- **Esc never quits** (decided, v2.2): only *Quit* in the menu ends the app (kiosk-safe).
- The top page shows a status strip: power, speed, connected trainer, Strava sync state.

**Menu tree**

| Top level | Page | Contents | Phase |
|---|---|---|---|
| **Resume** | | Close the menu, continue the ride | now |
| **Ride** | World mode | Installed world packages (region, size, version, © OSM attribution); mode: free ride (start: random, named place, or last position) or a route of the package (laps); built-in test routes (hills, loop); number of bots. *Start ride* | now (picker); map-based start point: Phase 7 |
| | Workouts | Structured workouts (§2.8): list, preview of the profile, start in ERG mode | with workouts (§2.8: basics Phase 2, full Phase 7) |
| | Events | Scheduled group rides, races, time trials (§2.3): calendar, sign-up, join a running session, private sessions and invitations | Phase 5 (sessions), Phase 7 (calendar) |
| **Workshop** | Bike | Bike type, frame, wheels, tyres, handlebars, saddle, accessories, paint; 3D preview; the physics profile shown separately ("road – aero", caps per §2.5), cosmetic choices never change it | Phase 4 |
| | Character | Body, skin, face, hair, clothing, shoes, helmet, glasses, gloves (§2.6); 3D preview with pedalling animation | Phase 4 |
| **Navi** | Routes | Route library: the package routes and the user's own routes with distance, elevation profile, region; ride, rename, delete | now (list) |
| | GPX import | Pick a GPX file, region check, map matching with the problem sections highlighted, accept / cancel (§2.7); offers the missing package's download (after the ride, §4.2) | Phase 3 (with GPX import) |
| | Rides and export | Saved rides (`rides/*.fit`): date, route, distance, time, Strava state; export as FIT / GPX, edit title and description, delete | now (list, FIT); GPX export with §2.8 |
| | Strava sync | Per-ride sync state (uploaded / processing / failed / rejected), *Sync now*, *Retry failed* | now (interim upload, §2.8); backend upload Phase 5 |
| **Settings** | Controls | Key and gamepad bindings (view now, rebinding later), autodrive on at start, steering sensitivity (lean rate, stick dead zone), power step (↑, PgUp/PgDn), invert camera, mouse orbit speed | now (view + options), rebinding Phase 7 |
| | Trainer | Devices: scan, connect, forget, the running devices with battery and signal; virtual trainer and its power; control mode SIM / ERG / off; trainer difficulty (§5.3); downhill handling; spin-down calibration where supported | now; calibration with §5.3 |
| | Strava | Link / unlink, automatic upload on / off, account name, last result; without client credentials (§2.8 interim) a short explanation | now (interim), Phase 5 (via backend) |
| | Audio | Mix: master, music, effects (bike, wind, ambience), voice; output device; later voice: microphone, push-to-talk key / open mic, spatial voice on / off | output + mix with game audio (see §3.3); voice Phase 6 |
| | Display | Monitor, fullscreen / window, resolution, graphics quality (shadows, view distance, tree density, LOD bias), FPS counter, HUD size | now |
| | Riders | Rider profiles (decided, v2.2): add / edit / delete riders with display name, weight, height, FTP, units (metric / imperial); the active rider is chosen in the Workshop; later account sign-in, privacy | now (local); account Phase 5 |
| **Restart** | | Same as R: start everything again, ride saved first | now |
| **Quit** | | Save the ride and exit | now |

**Interaction design**

- **Ten-foot UI:** the app is used on a trainer, often from 2–3 m away and with a gamepad. Text at least 3 % of the window height (scales with it), high contrast, a clearly visible focus frame, at most about seven rows per page, no hover-only functions, no text input where a choice works (names and titles get an on-screen keyboard on the gamepad).
- **Focus navigation** works identically for keyboard and gamepad (spatial navigation between focusable items, a page stack for Back); the mouse is optional. Every item is reachable without a mouse, in the kiosk's borderless fullscreen (§11.2).
- **Values change in place** (slider, ← / → choice) and apply immediately; there is no "Apply" button. Settings are saved when the menu closes. Device scans and Strava actions show their progress inline.
- **Rendering: Bevy UI** for the whole menu, as for the HUD (§3.2): it shares the HUD's theme and scaling, renders in the same window and screenshot / capture paths, and Bevy's input-focus and directional-navigation support fits gamepad control; egui's gamepad navigation is weak and it looks like a tool, not a game. egui stays for the console, dev panels and, in Phase 4, the advanced parameter panels of the editors (§3.2); the Workshop's main selection is Bevy UI.

**Architecture**

- **State:** a Bevy state `UiMode { Ride, Menu }` (the console stays an overlay flag); ride systems run only in `Ride` or check the input context. The menu itself is a **Bevy-free model** in `apps/desktop` (like `ClientCore`): the page tree, page stack, focused item, item values. Input is translated into abstract actions (`Up`, `Down`, `Left`, `Right`, `Confirm`, `Back`, `TabNext`, `TabPrev`); `ocw-render::menu` only draws a `MenuView` resource, as the HUD draws `RideView`.
- **Actions are console commands.** Each menu item either runs a command line (`trainer.difficulty 60`, `devices.connect 2`, `strava.sync`, `reset`) through `ConsoleState::queue`, or reads / writes a setting variable. Menu, console, `--cmd` and `autoexec.cfg` therefore share one code path, and every menu function is scriptable and testable headless.
- **Settings: one registry, one file.** Settings are variables with a name, type, range, default and an apply function (`set controls.autodrive_default on`, `get trainer.difficulty`, §2.9). They persist in `<user config dir>/ocw/settings.toml` (sections like the menu: `[controls]`, `[trainer]`, `[audio]`, `[display]`, `[profile]`, `[ride]` for the last world / route). Precedence: defaults < `settings.toml` < `autoexec.cfg` < command-line options; command-line options and `set --temp` apply only to the session and are not saved. Unknown keys are kept and reported, invalid values fall back to the default with a warning. Device memory (`devices.toml`), Strava credentials and tokens (`strava.toml`, `strava-token.toml`) stay in their own files – tokens never go into the settings file.
- **Starting another ride** (Ride page) first reuses the restart path (`restart.rs`): the choice is written to `[ride]` in the settings and the app re-executes itself, which starts the new world / route exactly as at program start. An in-app switch (tear down the local server and world, start the new ones without restarting the process) can replace it later.
- **Rider weight affects the physics (decided, v2.2):** the active rider's weight (and later the bike's physics profile, §2.5) goes to the server in the join message (new field) – the server stays the truth (§6.1) – and to the trainer (SIM total mass); today's fixed 75 kg + 8 kg profile becomes the default rider. Riders are created in *Settings → Riders* and selected in the *Workshop* (the Workshop's rider choice comes before Phase 4's editors).
- **Local now vs. backend later:** settings, devices, routes, GPX import, rides, the interim Strava upload and the Workshop configuration are local. Accounts and profile sync (§2.1), Events and sessions (§2.3), the backend Strava upload (§2.8), sharing routes and friends need the Phase 5 backend; those pages appear disabled with a short hint until it exists.

**Increments** (each with unit tests, §11.4; the menu model, settings and actions are tested without Bevy or with `MinimalPlugins` + `ConsoleCorePlugin`)

| # | Increment | Phase | Size | Tests |
|---|---|---|---|---|
| M1 | Input contexts, menu shell (Resume, Restart, Quit), Esc / Start, focus navigation for keyboard, mouse and gamepad, ten-foot theme; the ride continues under the menu (autodrive and default exits while it is open) | 3 (now) | M | navigation model (focus moves, wrap, page stack, Back at the top closes); key / button → action mapping; ride inputs ignored in `Menu`; Esc closes the console first and never quits; steering falls back to autodrive while the menu is open and is restored after; ride time, trainer and recording unaffected; headless app opens the menu and selects Resume; X11 + Wayland smoke screenshot of the menu |
| M2 | Settings registry, `settings.toml`, `set` / `get` commands; Settings pages Controls (options), Trainer, Strava, Display, Riders (profiles: name, weight via the join message and SIM mass, FTP, units), rider choice in the Workshop | 3 (now) | L | registry validation and ranges, TOML round-trip, precedence (file < autoexec < CLI, temp values not saved), unknown keys kept; each page item → expected command line; weight reaches the server's physics profile |
| M3 | Ride → World mode picker (installed packages, free ride / package routes / test routes, bots) via the restart path; Navi → Routes list, Rides list with Strava state and FIT export, Strava sync page | 3 (end) | M | package discovery on a fixture directory, `[ride]` choice → command line of the restarted process, ride list from fixture FIT + sidecar + `strava-sync.toml` |
| M4 | Navi → GPX import with map-matching report; GPX export of rides | 3 (with §2.7) | M | GPX fixtures through import → route; problem sections listed; GPX export round-trip |
| M5 | Game audio and Settings → Audio (mix, output device) | with game audio (see §3.3) | M | bus volume maths (master × bus), device selection fallback to the default device when the saved one is missing, settings round-trip |
| M6 | Workshop: bike and character pages with 3D preview | 4 | L | configuration model and persistence (§14 Phase 4); cosmetic choices never change the physics profile; menu items → configuration |
| M7 | Accounts / profile sync, Events (sessions, join), backend Strava, route sharing; start-screen dashboard (§2.1) | 5 (calendar 7) | L | API client against a mocked backend, disabled-page states offline, event list and join flow |
| M8 | Voice settings (microphone, push-to-talk, open mic, per-participant mute) | 6 | S | device selection and push-to-talk state (§14 Phase 6) |
| M9 | Rebinding of keys and buttons, accessibility (text size, colour-blind safe HUD) | 7 | M | binding conflicts, persistence, defaults restore |

**Open questions**

Decided (v2.2): no pause (trainer unchanged, ride time and recording continue); Esc never quits; rider weight affects physics, riders managed in Settings and chosen in the Workshop. Still open: switching rides by restart vs. in-app session switch (M3), and local Events before the backend (M7).

### 3.3 Audio

Two separate audio systems:

**Game audio** (Bevy Audio): background music, ambient sounds, bicycle sounds, wind, tires, freewheel, gear shifting.

**Status and output device (plan):** the app has no audio yet. Game audio comes as its own increment (M5 in §3.2.1), at the latest with Phase 4's riding animations. `bevy_audio` (rodio) always plays on the system's default output device and has no device list or bus mixer, so the user cannot pick the speakers (e.g. headphones vs. the TV on a trainer setup). Plan: a small audio module in `apps/desktop` behind a trait, on its own thread, that enumerates output devices with `cpal`, opens the chosen one with `rodio` and mixes four buses (master × music / effects / voice); evaluate `kira` (with its cpal backend) as the alternative before building it. The chosen device is saved by name (`audio.output`) and falls back to the default device when it is missing; hot-plugging is followed. Voice (Phase 6) plays through the same output setting via its own LiveKit audio path, with its own microphone selection.

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

**Trainers without FTMS.** Older Wahoo KICKRs (e.g. the KICKR on the Linux test machine) offer Cycling Power for data but no FTMS service; they are controlled through Wahoo's proprietary trainer control point (`a026e005-0a7d-4ab3-97fa-f1500f9feb8b` in the Cycling Power service). OCW supports it as a second control path; FTMS is used whenever a trainer offers both.

- Commands: unlock (`0x20`), ERG target (`0x42`), SIM mode with rider + bike mass, Crr and wind coefficient (`0x43`), gradient (`0x46`), wind speed (`0x47`). Answers are indications `[0x01, op code, result, …]` with result `0x01` = success.
- The layout is not officially documented; it was verified against the real KICKR with the hardware-in-the-loop tool (`cargo run -p ocw-devices --example ble -- ride <id>`), and the recorded answers are unit-test fixtures.
- **One control procedure at a time** (FTMS and Wahoo): a second write before the answer is rejected (observed: ATT error `0x80`). The client queues control writes, sends the next after the answer or a 1.5 s timeout, and replaces a queued gradient or target with the newest one.

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

Every pipeline step is a separately testable function with its own unit tests on small, hand-made input data (see §11.4).

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

**Implementation (v1.9):**

- **Filtering along whole roads:** OSM splits one road into many ways (tag changes, bridges, side roads). The pieces are joined into **through-roads** wherever the road continues (straightest continuation, same name, same class; main roads first) and each through-road is sampled, median- and Gaussian-filtered as one profile, so the gradient does not jump at the splits. Main roads fix the heights of their junctions first; later roads are split into runs at junctions already fixed.
- **Gradient limit without drift:** the class limit is applied *locally* (each interval that is too steep is flattened by moving its two samples towards each other, repeated until it holds), then light smoothing (σ 20 m) rounds the gradient changes and the local limit is applied again. Clipping the gradient and integrating along a long road would move the whole road metres away from the terrain.
- **Bridges and tunnels inside a through-road:** the DEM along them (valley below, mountain above) is replaced by a straight line between the ground at their ends before filtering. Junctions touched only by bridges or tunnels (a bridge mapped as several ways) are interpolated along the structure instead of taking the DEM of the valley floor.
- **Distances along the road** are the true arc length of the centreline (also for road strips and route files), not the chords between 5 m samples, which cut every bend.
- **Terrain under the road:** within the road width plus 1.5 grid cells the terrain is cut down to just below the road surface (0.3 m), so the 10 m grid cannot interpolate it through the road on side slopes; further out it blends into the DEM over 60 m.
- **Checks:** `world-builder check` measures where the terrain rises above a road strip (sampled every metre along and 0.25 m across, with the same triangle split as the mesh); `world-builder graph --probe X,Z` lists the roads near a point with lengths, end heights and gradients. The pilot package's test requires < 0.1 % of road samples under terrain.

Smoothing of the gradient sent to the trainer is covered in §5.3.

### 4.4 Road Reconstruction and Road Graph

Each road is converted into a 3D road segment with these properties: geographic centerline, road width, surface material, gradient, curve radius, road marking, connectivity information and road category.

The road surface is generated from the line geometries.

**Surfaces (v1.9):** asphalt, gravel and grass (off the road), with a rolling-resistance factor each (§5.2.1). Without an OSM `surface` tag, tracks and paths are gravel and all other classes asphalt; the `surface` tag refines this later.

The road connections form the **road graph**, which is the basis for routes (§2.7), junction steering (§2.2.1) and navigation.

- **Stable edge IDs:** derived from OSM way ID + node range plus a geometry hash. When a package is rebuilt from newer OSM data, routes are re-matched automatically and the world builder flags any that break. (Decided: `world-builder routes --world NEW --from OLD` keeps a route whose stable IDs all resolve and join up; otherwise it reports the break and re-matches the route's old geometry – points every 10 m along the old graph, converted through latitude / longitude – onto the new graph with the GPX matcher (σ 4 m). Routes that cannot be re-matched are reported and dropped.)
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

**Package format (v1.6):** the binary package (`.ocwpkg`) holds the 1 km tiles (terrain heights, road strips) and, from format version 2 (`OCWPKG02`), tagged sections after the tiles that readers skip when they do not know them. The first section is the **road graph** (`GRPH`: vertices, edges with stable ID, class, name, width, one-way, bridge / tunnel / boundary / dead-end flags, centreline and smoothed elevation profile), so the client and the server can ride free on it (§2.2.1). Packages without a graph are still written as version 1, and clients read both.

**Routes section (decided):** `ROUT` holds the package's routes on the road graph (§2.7): per route its name, a loop flag and the steps (stable edge ID with from / to metres as `f64`, or a U-turn). A package with only routes and no graph is still version 2. `world-builder gpx` and `route --world` add routes; readers that do not know the section skip it.

**Route files (v1.9, interim):** a plain-text `ocw-route 1` file holds one route with its elevation and the terrain grid around it (`name`, `origin` lat/lon of the local metres, `width`, `points` x y z every 5 m, optional `surface FROM TO NAME` stretches that are not asphalt, `grid`). It is written by `world-builder route` and ridden with `--route-file`, optionally together with a world package (`--world`) for the surroundings. Routes as part of packages (above) replace it; it keeps working until the last users are moved to package routes.

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
- **Free-ride path (v1.6):** a list of steps – (part of) an edge with its riding direction, or a U-turn loop (§2.2.2). It is extended automatically where the road just continues (up to 2 km ahead), with a U-turn at dead ends and at the region border, and at the **lock point** (20 m before the next junction) with the chosen exit. Corners at junctions are rounded (a curve starting at most 10 m before the junction, so it is never under the rider at the lock point). The geometry is a pure function of the step list, so the server, the client's prediction and the client's copy of other riders' paths agree; steps more than 600 m behind the rider are dropped, step indices keep counting. Route files and test loops keep their fixed route; physics and rendering use either through the same interface (`Track`).
- **Routes on the graph** use the same path: it may follow a route plan, which decides the steps the free ride would otherwise take by default (§2.2.1 route rides). Whether each step is on the route is a function of the steps, so the path stays the one source of truth for server and clients.
- Riders on different paths are compared where their paths share a road in the same direction (drafting, §5.2).
- **Sharp turns** and U-turns: the slow-down is a speed limit along the path – 15 km/h through turns sharper than 120°, 10 km/h through the U-turn loop – reached with ~2 m/s² before it. Junction turns sharper than 60° are taken with at most 4.5 m/s² sideways in the rounded corner (~22 km/h), so nobody turns off a descent at 60 km/h.
- **Fixed timestep:** physics at **50 Hz** (20 ms) on client and server, independent of frame rate.
- **No strict bit-level determinism:** `f64` is used. The client predicts, receives authoritative state, and corrects small errors by smoothing (blending over ~0.5 s); it snaps only above a threshold (e.g. > 5 m). This avoids the cost of fixed-point math.
- **Drafting** (reduced CdA behind other riders) is computed server-side because it depends on other riders. The client predicts it from the interpolated positions of nearby riders.
- **Sharp turns** (> 120°): an automatic short slow-down is applied (the rider can't brake on a trainer), so the avatar does not take a hairpin at 40 km/h.

#### 5.2.1 Lateral Steering and Surfaces

- Besides `lateral_offset`, a rider has a **heading relative to the road** and rides a curve of its own; the road's curves turn the road under the rider. Lateral offset changes with `v · sin(heading)`; the distance along the road with `v · cos(heading)`. The avatar leans by `atan(v² κ / g)` for the curvature κ it rides.
- **Autodrive** (default; bots always): an autopilot leans the rider to hold a line (the middle of the road, or where the rider moved it), through the road's curves. **Manual:** the rider leans (keys ramp the lean and it stays; held through upright, the opposite key pauses there for ~0.3 s, then leans on to the other side; a gamepad stick sets it), κ = g·tan(lean)/v²; the road's bends have to be steered, or the rider drifts to the edge. Near the largest angle to the road (~17°, wild ~70°) the turn fades out, so the rider settles at it smoothly.
- **Road edge:** the rider stops at the edge (0.6 m inside); while still steering (or carried by a bend) into it, the rider scrapes along it with an extra deceleration. Rolling straight along the edge or steering away from it does not scrape. **Wild mode:** no edge; the rider may leave the road (up to 60 m either side, at up to ~70° to it) and rides on the terrain. Off-road gradients are still the road's (a simplification until riders move freely in 2D).
- **Surfaces:** asphalt, gravel (OSM tracks and paths; the `surface` tag later), grass (anything beside the road). Each has a factor on the rolling resistance coefficient (1, 2.5, 6), applied in the physics and sent to the trainer in SIM mode (Crr), so gravel and meadows are harder to ride.
- Steering is an intent like the junction choice (§6.1): the server applies it, the client predicts it with the same rule; heading and ridden curvature are part of the rider state in snapshots.
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

**Start point and test riders (v1.9):** a session starts all riders together at a configured distance (`ServerConfig::start_m`); the desktop app picks a random point by default (on a route that ends, at least 1 km before the end; off for screenshot and capture runs, which must be reproducible). Server-side test riders (bots) ride at different strengths (85–250 W) so they spread out, push harder uphill (at least the power for 10 km/h on the current gradient, so easy bots do not crawl) and steer with autodrive.

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

- The client sends a reliable `TurnIntent { junction_id, exit_edge_id }` as soon as the choice changes (§2.2.1). (v1.6: plus a sequence number and the exit's riding direction; no `exit_edge_id` = no choice, the default; `junction_id` is the vertex index in the world package's graph – client and server use the same package.)
- A manual U-turn is a reliable `UTurn { distance, speed }`: where the client turned, so the server builds the same loop when it is close to its own state.
- **Same package on both sides (decided):** because junctions are vertex indices, the client sends `Hello.world = WorldPackageId { name, graph_hash }` and the server sends its own in `Welcome.world`. `graph_hash` is the FNV-1a 64 fingerprint of the road graph's package encoding (the `GRPH` payload), so any rebuild that changes the graph changes it. The server rejects a Hello with another hash ("world package mismatch … install the same package version"); a client refuses a Welcome with another hash (for servers that do not check). Sessions without a road graph (route files, test routes) skip the check. Package signing and manifests (§4.7) are separate.
- Route rides: `Welcome.graph_route` names the route; leaving it is an ordinary `TurnIntent`, rejoining a reliable `RouteRejoin { distance, speed }` (§2.2.1).
- The server sends each rider's path as reliable `PathUpdate`s (the steps from an index on, or a new path after joining or a teleport); clients use them for the other riders and to correct their own prediction.
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
│   ├── ocw-fit/          # FIT activity writer (no dependencies, wasm-safe)
│   └── ocw-voice/        # LiveKit integration behind a trait (feature "voice")
├── proto/                # .proto schemas
├── artwork/              # original artwork: logos/ (app icon), build-history/ (screenshots, videos)
├── packaging/            # desktop entry (Linux), later installers
├── release/              # release builds per version and platform (+ SHA256SUMS)
├── scripts/              # build machines, deploy, kiosk, AppImage, Windows build server setup
├── config/               # build machine config (only examples in git)
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

**Build machines (v1.9):** all building and testing runs on remote machines described in `config/machines.toml` (git-ignored; keys in `config/ssh/`): a Linux machine with a real GPU and Bluetooth (development, tests, smoke screenshots and videos on X11 and Wayland, trainer tests, a kiosk that shows the latest build on its screen), a macOS Apple Silicon machine, and a Windows machine set up with `scripts/setup-windows-buildserver.ps1` (OpenSSH server and a local, non-admin build user). `scripts/remote.sh MACHINE run CMD` copies the working tree (git-tracked and untracked, not ignored) and runs the command; other branches get their own checkout and fewer parallel build jobs, so parallel work does not collide. `scripts/deploy-test.sh` installs a release build for the kiosk.

**CI resilience (v1.9):** the Linux job's package installation retries with short timeouts and a step timeout, so an unreachable distribution mirror fails within minutes instead of holding a runner for hours.

### 11.3 Packaging

**Linux:** AppImage, Flatpak, optionally native packages.

**Windows:** EXE/MSI, code signing, installer.

**macOS:** app bundle, code signing, Apple notarization, DMG.

Build and release pipelines are automated for all three operating systems, introduced in the rollout order (§11.1).

**Implemented so far (v1.9):**

- **App icon and desktop integration:** the logo (`artwork/logos/`, a unicorn riding a bike over a rainbow) is the window icon (X11, Windows); the window's app id `ocw-desktop` matches the desktop entry (`packaging/linux/ocw-desktop.desktop`), which Wayland compositors, docks and menus use. On Linux the app installs its desktop entry and hicolor icons into the user's data directory at start (only changed files).
- **AppImage:** `scripts/build-appimage.sh` assembles the AppDir (binary, desktop entry, icons, `.DirIcon`) and packs it with `appimagetool` when installed.
- **macOS app bundle:** `scripts/build-macos-app.sh` (`make macos-app`, runs on macOS) builds `Open Cycling World.app` (bundle id `org.opencyclingworld.desktop`; stripped binary; `Info.plist` with version + commit, minimum macOS read from the binary, `NSBluetoothAlwaysUsageDescription`, without which macOS ends the app when it uses CoreBluetooth; `ocw.icns` from `artwork/logos/`), signs it ad hoc and packs a DMG (with an Applications link) and a zip. Started from Finder, the app logs to `~/Library/Logs/OCW/ocw-desktop.log` (stderr has no terminal there). CI builds and uploads both on pushes to `main`. Still open: Developer ID signing with the hardened runtime and notarisation (`CODESIGN_IDENTITY` already signs with a given identity), a 1024 px icon render for the 512@2x slot.
- **Release folder:** `release/<version>-<commit>/<platform>/` holds stripped release binaries (`ocw-desktop`, `ocw-server`, `world-builder`) with SHA-256 sums; binaries above GitHub's file size limits are stored xz-compressed. Map data is not included (ODbL, §13). `artwork/build-history/` keeps the screenshots and videos made during development with an index.

---

### 11.4 Testing Strategy

**Rule: no step is done without tests.** Every development step in §14, every feature and every pipeline step comes with unit tests that are merged together with the code. CI runs them on every change; a failing test blocks the merge.

| Level | What | Where | When |
|---|---|---|---|
| **Unit tests** | Functions and modules in isolation: physics, route sampling, protocol encoding, session logic, parsers, pipeline steps | `#[cfg(test)]` modules next to the code in every crate | Every commit, all platforms in CI |
| **Integration tests** | Several crates together: client ↔ local server over the protocol, world-builder on a small OSM extract, device backend ↔ app channel | `tests/` directory of the crate | Every commit (Linux); per platform once ported |
| **End-to-end smoke test** | The real desktop app under Xvfb with software rendering (Mesa lavapipe): launch, ride on the virtual trainer, run console commands, take a screenshot, exit | `tools/` / run skill driver | Every commit on Linux |
| **Hardware-in-the-loop** | Real trainers and sensors (FTMS, power meter, heart rate) | Manual test protocol, later a test bench | Before each release |
| **Performance tests** | Frame time, streaming hitches, memory (§15) | `tools/performance-test` (console `exec` scripts, §2.9) | Nightly / before releases |

**Principles:**

- **Tests run without hardware, network or GPU.** Devices are replaced by the virtual trainer or mock backends (`DeviceBackend`, §3.4); network transports by in-process channels; rendering-free logic lives outside Bevy (§10) and is tested headless.
- **Core crates are tested for wasm too:** `ocw-core`, `ocw-sim`, `ocw-protocol` and `ocw-world` must build for `wasm32-unknown-unknown` (§3.5).
- **Physics tests check physical plausibility**, not only code paths: steady-state speed for given power and gradient, energy balance, no rolling backwards, stability at the 50 Hz tick.
- **Protocol compatibility tests:** encoded messages of earlier protocol versions are kept as fixtures and must still decode (§6.2).
- **World-builder tests** use small, checked-in OSM / DEM fixtures with known expected results (golden files), e.g. a junction, a bridge, a dead end.
- **Regression rule:** every fixed bug gets a test that failed before the fix.
- Commands: `cargo test --workspace` runs all unit and integration tests; `cargo clippy --workspace --all-targets -- -D warnings` must pass.

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
| Own artwork (logo, icons, `artwork/`) | Own work | Like the code: **MIT / Apache-2.0** |
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
- Graphics engine spike (§3.1.1) – done on Linux (results and decisions in §3.1.1); minimum-spec run still open.
- Linux build; build-only CI checks for Windows and macOS; wasm build of the core crates.

- **Unit tests:** physics basics, route sampling, protocol encode/decode and quantization, session logic (inputs, sequence numbers, bots), client prediction and interpolation, virtual trainer, local server, console command parsing and permissions.
- **Integration / smoke tests:** desktop app ↔ in-process local server; end-to-end launch under Xvfb with screenshot; CI pipeline with tests on Linux, build checks for Windows, macOS and wasm.

**Result:** A launchable Linux application on the final client/server architecture.

### Phase 2 – Bicycle and Sensors

- Bluetooth scanning (BlueZ).
- FTMS.
- Power, cadence and heart rate.
- Bicycle physics and drafting model in `ocw-sim`.
- Virtual resistance: SIM mode with trainer difficulty (§5.3).
- Basic workouts and ERG mode.
- FIT export.

- **Unit tests:** FTMS / Cycling Power / Heart Rate data parsing against recorded byte sequences, control point encoding (SIM, ERG, resistance), trainer difficulty and gradient rate limiting, drafting model, workout parsing (own format, `.erg` / `.mrc`, FIT), ERG target calculation, FIT writer (round-trip through a FIT reader).
- **Integration tests:** device backend with a mock BLE peripheral ↔ app; hardware-in-the-loop protocol with at least one real FTMS trainer.

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
- In-game menu, first increments (§3.2.1, M1–M4): pause menu, settings file and Settings pages (controls, trainer, Strava, display, profile), Ride picker, Navi (routes, rides, Strava sync, GPX import).

- **Unit tests:** every world-builder step on fixtures — OSM extraction, projection, road graph and stable edge IDs, graph cleanup and dead-end flags, DEM sampling and smoothing (gradient limits, bridges, tunnels), terrain conformance, splat-map classification, tile cutting, package manifest and signature verification; route format, GPX parsing and map matching; junction detection, exit classification, default-exit rules, key selection and lock point, U-turn.
- **Integration tests:** build a small region end-to-end and load it in the client headless; rebuild with changed OSM data and check route re-matching.

**Result:** Ride a geographically realistic route locally in 3D. **Windows port starts.**

### Phase 4 – Avatar and Bicycle Editor

- Bicycle models.
- Character models.
- Components and physics profiles (§2.5).
- Animations.
- Configuration and persistence.
- Workshop pages of the in-game menu (§3.2.1, M6); game audio and audio settings at the latest here (M5, §3.3).

- **Unit tests:** component and configuration model (IDs, physics profile references, cosmetic parts never change physics), profile caps, persistence round-trip, animation state selection from cadence / speed / riding state, asset validator rules (including licence metadata).
- **Integration tests:** load all shipped glTF assets through the validator and into a headless Bevy app.

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
- Menu: accounts and profile sync, Events (sessions), backend Strava, start-screen dashboard (§3.2.1, M7).

- **Unit tests:** QUIC transport framing, handshake and version check, reconnect logic, interest management, steering intents and lock-point decisions, plausibility checks and trust levels, leaderboard ranking, API handlers (Axum) and database access, Strava upload flow against a mocked Strava API, web companion components.
- **Integration tests:** server with many simulated clients (latency, packet loss, disconnects); load test toward 100 riders per session; web companion end-to-end tests in a headless browser.

**Result:** Shared virtual rides.

### Phase 6 – Voice Chat

- LiveKit (self-hosted) and the LiveKit Rust SDK.
- WebRTC.
- Microphone management.
- Push-to-talk.
- Voice rooms.
- Spatial audio.
- Voice settings in the menu (§3.2.1, M8).

- **Unit tests:** voice room and token management (backend), spatial audio attenuation and nearest-N selection, push-to-talk and mute state, device selection logic; app builds and runs with the `voice` feature disabled.
- **Integration tests:** client ↔ self-hosted LiveKit test server in CI (two simulated participants).

**Result:** Shared rides with voice communication.

### Phase 7 – Production Readiness

- Performance optimization.
- Large regions.
- Weather and times of day.
- Training history.
- Events (menu: calendar, §3.2.1).
- Key and button rebinding, accessibility (§3.2.1, M9).
- Full workout and ERG support.
- Web companion: events, routes, world packages.
- Error monitoring.
- Installers and updates (signed, TUF evaluated).
- Security audits.
- macOS port.

- **Unit tests:** weather and time-of-day logic, event scheduling, update / package verification (TUF), all macOS-specific code paths (CoreBluetooth backend, permissions); the full unit and integration test suite runs on all three operating systems.
- **Release tests:** performance tests against §15, security tests, installer and update tests per platform, hardware-in-the-loop protocol.

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
| Tests | Unit tests for every crate, feature and pipeline step; all tests pass in CI before merge (§11.4) |

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
