# Open Cycling World

**Cross-Platform Indoor Cycling Simulation**

Project Description and Technical Architecture

Version 2.16 – November 2026

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
| 2.4 | Routes on the road graph (§2.2.1, §2.7, §4.4, §4.7): GPX import with HMM map matching, package routes (section `ROUT`, `ocw-graph-route` files), route arrows with leave / rejoin (J), re-matching after rebuilds; U-turn loop from the road width (§2.2.2); package check in Hello / Welcome (§6.5); border matching for neighbouring packages (runtime loading still open) |
| 2.5 | Signed, cached world packages (§4.7, §12): manifest with file and chunk hashes, Ed25519 root → signing key chain (dev root committed, public only), trust policy for unsigned packages, cache `~/.local/share/ocw/worlds/<id>/<version>`, verified download with resume; end-to-end test from an OSM fixture to a bot ride and a rebuild |
| 2.6 | Land use and buildings from OSM (§4.6, §4.7): 17 land classes on a 5 m grid per tile (`LUSE`), flat water with carved terrain, colour maps and trees by land use, extruded buildings with flat or gable roofs (`BLDG`) |
| 2.7 | Menu M2 (§3.2.1): settings registry (keys, scopes, layers) and `settings.toml`, `set` / `get`; rider profiles in `riders.toml`, weight 30–200 kg in `Hello.rider_weight_kg` (the client's physics parameters are ignored, §2.5, §6.5) and as Wahoo SIM mass; Settings / Workshop pages as rows of command lines; HUD on / off and imperial units |
| 2.8 | Autonomous night 2026-11-11/12 (Appendix B): menu M1–M3 (in-app ride switch, Navi, GPX import), workouts with ERG (§2.8), local events with ghost (§2.3.1), gradient aids and camera tilt, camera classes and tunnels (§3.2), physics profiles and bike looks (§2.5), neighbour packages (§4.7), QUIC transport (§6.2), fixes for one-way U-turns, steep starts and gradient artefacts; all assumptions listed in Appendix B |
| 2.9 | Priorities 2026-11-12: the route editor (§2.7) is postponed (Phase 3 is closed without it); Phase 4 models, animations and game audio (M5, §3.3) and Phase 6 voice chat (§7) start in parallel, ahead of the Phase 5 backend |
| 2.10 | Voice chat always on like a call (Teams / Meet): automatic join, open microphone, no push-to-talk; hotkeys on keyboard (`M`, `Shift+M`) and gamepad (Select, right-stick click) to mute the own microphone and the speaker (all voices); mute state on the HUD (§2.4, §7, M8, §14 Phase 6) |
| 2.11 | Releases on GitHub Releases with the world package (ODbL data licence, OSM source attached); binaries removed from the git history (`release/commit-map.txt`) (§11) |
| 2.12 | Voice platform plan: LiveKit on every platform – Rust SDK on the desktop (Linux build needs clang ≥ 21), `livekit-client` from wasm in the browser, native or Rust SDKs on iOS / Android; `ocw-voice` trait stays open for them (§7) |
| 2.13 | Build-history media out of git: one zip per day on the GitHub release `build-history`, day folders git-ignored, history rewritten again (`release/commit-map.txt`) (§11) |
| 2.14 | Phase 4 models and animation (§2.5, §2.6, §8.1, §14): generated glTF bikes per physics profile and a rigid-part rider (ocw-assets, MIT OR Apache-2.0), asset manifest and validator, rider-on-bike IK pose, riding states (seated, climbing, standing, sprint, coasting) and far LOD at 40 m |
| 2.15 | `artwork/build-history/` removed from the repository; the build history (zips per day and `index.md`) lives only on the GitHub release `build-history` (§11) |
| 2.16 | Game audio (M5, §3.3): own audio module (cpal behind an `OutputBackend` trait on its own thread; kira rejected, Bevy audio disabled), output device by name with default fallback and hot-plugging, buses master × music / effects / voice, ride sounds and cues synthesised in code; Settings → Audio, `audio.*` settings and console commands, `--no-audio` (§3.2.1); synthesised sounds are code (§13) |

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
  - Exception: a rider who arrives *against* a one-way road (after a U-turn on one: manual, or automatic at a dead end or the region border) and has no other rideable exit rides on against the one-way roads (not straight back), instead of turning again and again. Regression test: `against_a_one_way_road_without_a_legal_exit_ride_on` (`ocw-world`).
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
- **U-turn on a one-way road (decided):** on a one-way road (or in a roundabout) a manual U-turn moves ahead to the first stretch of the known path that may be ridden back, up to 300 m beyond the slow-down (the HUD shows "U-turn in N m"); without one the rider turns anyway and rides back against the one-way road to the first legal exit (§2.2.1). Before, the next vertex against the one-way road had no exit, an automatic U-turn followed a few metres on, and the rider turned back and forth in place. Regression test: `manual_u_turn_on_a_one_way_road_turns_once` (`ocw-world`).
- **Region border:** if the neighbouring package is installed, the ride continues across the border without interruption. Otherwise there is an automatic U-turn with the message "End of region – download <neighbour>". The download happens only after the ride; nothing is downloaded during a ride (§4.2).
- **Neighbouring packages (decided and implemented):** the builder cuts a road exactly where it crosses the region boundary: the crossing point (found by bisection on the region test, so a neighbour with the same border line finds the same point; a node within 0.5 m of the line is the cut end itself) becomes a boundary vertex in both packages. The cut halves have different stable edge IDs (the cut end counts as a marker in the ID). `ocw_world::border::match_border` pairs the boundary vertices of two packages that are the same road (same OSM way or stable edge ID) at the same geographic point (within 8 m; each package keeps its own projection, points are compared through latitude / longitude). `join_graphs` builds one road graph in the first package's metres with the linked vertices merged and no longer boundary (the neighbour's road starts exactly at the merged vertex), so a free-ride path continues across; the neighbour's own outer border stays a border.
  - **Adjacency is geographic** (no list of neighbour ids in a package, which would need every package rebuilt when a neighbour appears): each package stores its region (section `REGN`, §4.7), its manifest the region box (`bounds`). At the start the client (and the headless server) look for installed packages whose box touches the home package's (within 0.01°): the newest version of every other id in the cache, and the packages in the directory of a `--world` path. Each candidate is loaded and verified like the home package (same trust policy, §4.7) and joined only if at least one road crosses to it (`WorldSet::add_neighbour`); a package covering the same area (another build of the same region) or without a region is refused. Only direct neighbours of the home package are joined; their own outer borders stay borders. `--no-neighbours` rides the package alone.
  - **One frame:** everything at runtime is in the home package's metres. A neighbour's metres map into them with a `Frame` (x scaled by the ratio of the projection latitudes' cosines, then shifted) – exact for the local projection (§4.5), so the neighbour's tiles, its roads in the joined graph and its ground agree with the home package. Neighbours built for each other use one projection origin (`world-builder package --origin`), then the frame is the identity and the tile grids coincide.
  - **No step at the seam:** each package conforms its terrain only to its own roads and smooths the profiles of its own (cut) edges, so two independently built neighbours differ by up to a few metres along the border (measured on a split of the pilot: up to 3 m). A part cut with `--bbox` is therefore built with **context**: the roads up to 0.01° (about 1 km) beyond the box are built too; their profiles replace the package's own (sampled along the same OSM way, `profile::transfer_profiles`), and the terrain is conformed to them (`build_package_with_terrain`). Both neighbours then see the same roads around the border, and terrain and road heights agree there (end-to-end test: terrain within 0.3 m, road ends within 5 cm). Neighbours built from different extracts (each with its own `--poly`) get no context and may still show a small step; building region sets from one extract is the supported way.
  - **Runtime (`ocw_world::world_set::WorldSet`, pure):** the joined graph is built once at the start and fixed for the ride (junction ids are its vertex indices; the server rides it, the session name is `Home + Neighbour`, and the graph fingerprint covers the joined graph, §6.5). Tiles overlap at the border (every package has terrain-only margin tiles), so each point of the ground belongs to the first package with loaded tiles that has a tile there and whose region contains it, else to the first such package with a tile there (home first). The renderer draws a package's terrain cells, water, trees and buildings only on ground it owns; road strips always (they end at the border). The graph of each neighbour stays in memory; its tiles are loaded (from disk, verified again; a package changed since the start is refused) when the rider comes within 3 km of its region box and dropped beyond 5 km, at most two neighbours at a time (memory budget, §15). Seam tiles are re-meshed when a neighbour's tiles come or go.
  - Without an installed neighbour, a border turns the rider round as before.
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

#### 2.3.1 Events: Model and Local Events

Group rides, races and time trials are **events** of a session. The first increment runs them **locally** (single player with bots on the in-process server, started on demand); the event logic lives in the server's session (`ocw-sim::event`, `Session::start_event`) so the networked server (Phase 5) runs the same code, with events scheduled by organizers instead of started by a rider.

**Kinds**

| Kind | Field | Finish and ranking |
|---|---|---|
| Group ride | everyone in the session; bots hold a target power (default 150 W ≈ 2 W/kg; each bot within ±10 %) and regroup: a bot ahead of the group's anchor eases off, one behind pushes (2.5 % of the target per metre, 0.5–1.6 × the target; never below the climbing minimum, §6.1). The anchor is halfway between the bots' mean and the riders' mean, so the group waits for the riders | none: runs until cancelled |
| Race | everyone, from the start formation (§6.1) | a finish line `length` after the start line (laps of a loop or a distance; default one lap of a loop, 5 km of a route that ends); ranked by finish time, then by distance for riders still riding; DNF for riders who leave the session and for everyone still riding when the time limit after the first finisher is over (max(2 min, 25 % of the winner's time)) |
| Time trial | the rider alone (the bots are removed), **no drafting** | like a race; the comparison is the rider's own **ghost** |

**State machine (server-authoritative, §6.1).** `Scheduled → Countdown → Running → Finished`, in server ticks. A local event is scheduled at the current tick, so it starts with the countdown (5 s: "Get ready", 3-2-1, GO!). At the start every rider is placed in the start formation (§6.1: the client rider in the front row's centre, bots in joining order, standing still, autodrive in their lane) behind the start line: on a loop the start line is the start / finish (one lap in, so distances stay positive), on a route that ends it is the formation's length after the route's beginning (the last row at 0 m); on a route on the road graph (§2.7) everyone's path is rebuilt from the route's start (broadcast as path resets, §6.5). During the countdown the session holds every rider (no physics), and the client stops predicting its own rider (so the bike does not creep forward and snap back). A rider's finish time is the crossing interpolated within the 50 Hz tick. Group rides need no finish; races and time trials end when every entrant finished or is DNF. Riders ride on after the finish. Events need a fixed course: free ride (§2.2.1) refuses them. A new event replaces the old one; `cancel` ends it (everyone rides on where they are).

**Protocol** (only added messages, §6.2): `EventRequest` (client → server: `EventStart { kind, laps?, length_m?, bots?, target_w? }` or `cancel`), answered with `DevCommandResult`; only a server with `allow_local_events` (the local server) accepts it, a remote server answers that organizers schedule events. `EventState` (server → client, reliable): kind, phase, countdown / GO / finish ticks, start line, length, laps, target power and the standings (`EventStanding`: position, progress, finish time, DNF, gap to the rider ahead in ms); sent when the event starts, changes phase or a rider finishes, twice a second while a race or time trial runs, and to riders who join; phase `NONE` = no event (cancelled). The gap to the rider ahead is the difference of the finish times, or for riders still riding the distance gap at the own speed (at least 1 m/s).

**Ghost (decided: client-side).** The ghost is the rider's own best previous effort on the same course and length, replayed as a translucent rider (it neither casts nor receives shadows: a solid shadow under a see-through bike looks wrong); it is a rendering and a gap computation only, so the server never needs it (and nobody drafts it). The client records its authoritative distance past the start line every 0.5 s (from the snapshots) and the exact finish time; the best trace per course (route id; routes are ridden in their own direction) and length is kept as a small text file `<user data dir>/ocw/ghosts/<route>_<length>m.ghost` (header with course, length and time, then `seconds metres` lines; ~30 KB for 20 min; replaced only by a faster effort, written through a temporary name). Gap to the ghost = time now − the ghost's time at the rider's distance (positive = behind). Later options: ghosts of friends from the backend, ghosts in races.

**HUD** (§3.2): the big countdown in the middle; on the right the event's title, the position and gap ("P3/11  +4.2 s to Bot 4"), time and distance to go (with the lap), the ghost's gap, or the place relative to the group; at the own finish (or the event's) the results panel in the middle (position, name, time, gap to the winner; provisional while others ride; at most ten rows plus the own; the ghost comparison for time trials) until Esc / Enter / A / B closes it or after 30 s. The texts are built Bevy-free in the app (`apps/desktop/src/event.rs`), the HUD only lays them out. Starting: console `event race [--laps N|--km X] [--bots N]`, `event group [--watts W]`, `event tt`, `event cancel`, `event status`, `event dismiss`, and the menu's Ride page (§3.2.1; decided: an event start from the menu asks first – *Start the event?*, *Start now* / *Cancel* with Cancel focused, like Restart and Quit –, since it moves the rider to the start line and ends a running event; *End the event* is not asked).

Open: scheduled events and sign-up (Phase 5/7), "follow leader" steering in group rides (§6.5), weight lock and trust levels for races (§6.4), ghosts across devices.

### 2.4 Voice Communication

The application includes integrated real-time voice communication.

Features:

- Group voice channels.
- Private voice rooms.
- Voice communication during shared rides.
- **Always on, like a call** (Teams, Google Meet; decided 2026-11-12, see below).
- Microphone and speaker selection.
- Muting of individual participants.
- Global mute.
- Volume control.
- Automatic echo cancellation.
- Noise suppression.
- Connection status.

Additionally, spatial audio is provided: voices of riders who are virtually behind the local rider sound quieter than voices of immediately adjacent riders.

**Always-on behaviour (decided 2026-11-12):**

- Voice works like a video conference or phone call, not like a radio: as soon as the rider is in a shared ride (a session with other human riders, or a group / event), the client joins the voice room **automatically** with an **open microphone**. Solo rides (local server, bots only) do not connect. There is **no push-to-talk** mode (dropped from the earlier feature list; it may return later as an option if riders ask for it).
- **Two mute switches**, each a toggle with a hotkey on the keyboard and on the gamepad:

  | Action | Keyboard | Gamepad | Effect |
  |---|---|---|---|
  | Mute / unmute own microphone | `M` | Select (Back / View / Share) | Nobody hears the rider; the microphone track is muted (not unpublished), so unmuting is instant |
  | Mute / unmute speaker | `Shift+M` | Right-stick click (R3) | The rider hears no voices at all; game sounds stay (they have their own volume and mute in Settings → Audio, §3.3); the own microphone keeps its state |

  The same toggles exist in the menu (Settings → Voice) and as console commands (`voice.mute-mic`, `voice.mute-speaker`, with `on` / `off` / toggle). Hotkeys are fixed for now and become rebindable with key rebinding (M9).
- **State is remembered** across rides and restarts (`settings.toml`): a rider who left with the microphone muted joins the next ride muted. First start: microphone and speaker on.
- **HUD:** a small voice indicator while connected: microphone on / muted (crossed out), speaker on / muted, connection status (connecting, connected, reconnecting, failed), and a speaking marker at riders who are talking. A short sound confirms each toggle and someone joining / leaving.
- Muting individual participants (§2.4 list) stays available in the menu's participant list.
- **Assumptions / uncertainty:** "mute speaker" is taken to mean *all incoming voices*, not all app sound (the master mute in Settings → Audio covers that). Options for the gamepad were (a) Select = microphone and a long press of Select = speaker, (b) Select = microphone and R3 = speaker, (c) a button combination; (b) is chosen because two separate buttons are easier to learn and a long press delays feedback. R3 is otherwise unused; Select is free (Start opens the menu). `M` is not used by any other key.

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
- The client never sends physics parameters (v2.7): it sends only the rider's weight (`Hello.rider_weight_kg`, §3.2.1) and the id of the bike's physics profile (`Hello.bike_profile`); the server takes drag, rolling resistance, bike mass and drivetrain efficiency from its own copy of the curated table (an unknown or absent id is the default profile).
- Differences between profiles are capped (e.g. at most ±3 % speed at 250 W on flat road).

**Implemented (Phase 4 groundwork):**

- *Profiles* (`ocw_core::bike`, Bevy-free and wasm-safe, shared by server and client prediction): six profiles in four bike types – `road` (standard, the default), `road-aero`, `road-climbing` (bike type Road), `tt` (Time trial), `gravel`, `mtb` (Mountain bike). Each has bike mass, CdA, Crr on asphalt, drivetrain efficiency and an *off-road share* (how much of a rough surface's extra rolling resistance the bike feels, §5.2.1). The bike type is the profile's; there is no separate bike-type choice that could disagree with the physics. Handling (max lean) is the same for all profiles for now.

  | Profile | Bike mass | CdA | Crr | Efficiency | Off-road share |
  |---|---|---|---|---|---|
  | `road` | 8.0 kg | 0.320 m² | 0.0040 | 97.6 % | 1.0 |
  | `road-aero` | 8.6 kg | 0.305 m² | 0.0040 | 97.6 % | 1.0 |
  | `road-climbing` | 6.8 kg | 0.330 m² | 0.0040 | 97.6 % | 1.0 |
  | `tt` | 9.2 kg | 0.295 m² | 0.0039 | 97.6 % | 1.3 |
  | `gravel` | 9.0 kg | 0.325 m² | 0.0045 | 97.4 % | 0.5 |
  | `mtb` | 11.0 kg | 0.330 m² | 0.0048 | 97.2 % | 0.3 |

- *Caps (tested):* at 250 W, 75 kg, on flat asphalt every profile is within ±3 % of `road`'s steady speed (TT +2.7 %, aero +1.5 %, MTB −2.3 %); on a 6 % climb within ±5 % (climbing +1 %, MTB −4.4 %). Off the asphalt the differences are larger on purpose (grass: MTB about +17 %, gravel +12 %), since choosing the bike for the terrain is the point of the types. *Equal* race mode (one profile for all) comes with sessions (Phase 5).
- *Look* (`RiderLook`, cosmetic): frame, jersey and helmet colour (sRGB `0xRRGGBB`; the server keeps only the low 24 bits) and a kit pattern (plain, band, sash; unknown values are plain). Sent as `Hello.look`, stored per rider by the server and passed through in every `RiderUpdate` (`bike_profile`, `look`) together with the display name; a reliable roster message can replace these per-snapshot fields when bandwidth matters (Phase 5). Riders without a look (old servers) are drawn in the colours by id.
- *Persistence:* per rider in `riders.toml` (`bike`, `frame_color`, `jersey_color`, `helmet_color`, `kit`; all optional, so files from before are read unchanged; invalid values are dropped with a warning).
- *Models (§8, Phase 4):* glTF 2.0 files in `assets/models/`: one bike per profile (`bike-<profile>.glb`) and one rider (`rider.glb`), listed in `assets/models/manifest.toml`. They are made by a generator in `crates/ocw-assets` (`cargo run -p ocw-assets -- generate`) from real geometry numbers (stack, reach, seat and head angle, chainstay, bottom-bracket drop, fork rake, wheel and tyre size, crank length), so the silhouettes differ the way real bikes do:
  - `road`: compact frame with a slightly sloping top tube, 35 mm rims, 24 spokes, two bottles, rim brakes;
  - `road-aero`: deep aero tubes, dropped seat stays, aero seat post, 60 mm rims;
  - `road-climbing`: thin round tubes, level top tube, shallow 28-spoke wheels;
  - `tt`: 78° seat angle, low front, base bar with extensions and arm pads, three-spoke front wheel, disc rear wheel;
  - `gravel`: taller head tube, flared drops, 40 mm tan-wall tyres with tread, disc brakes, top-tube bag;
  - `mtb`: long and slack (66°), suspension fork, 760 mm flat bar, 2.3" knobby tyres, disc brakes.

  Nodes: `bike` (frame, fork, drivetrain), `wheel_front` / `wheel_rear` (spin about X), `crank` with `pedal_l` / `pedal_r`, `handlebar` with the grip anchors `grip_hoods_*`, `grip_tops_*`, `grip_drops_*`, `saddle` (where the sit bones rest) and `lod1` (far model). Materials are *paint slots* (`ocw_assets::slots`): `frame`, `jersey`, `helmet` and `accent` (kit pattern; white on dark jerseys, black on light) take the rider's look, the others (tyre, rim, metal, component, bar tape, decal, bottle, shorts, skin, shoe, sock, lens, hair, …) keep the colour in the file. An avatar is rebuilt when its bike or look changes; riders with the same colours share materials, and all avatars share the meshes.
- *Asset decisions (Phase 4):*
  - *Where the models come from.* Options: model them in Blender and export; download CC0 models; generate them in code. **Generated in code**: reproducible, reviewable as a diff, the licence is clear (own work), the six bikes follow from their geometry numbers, and the anchors the pose uses come from the same numbers. The test `shipped_assets_match_the_generator` fails when the files are stale. Medium – the look is "engineered low-poly"; an artist's models can replace any file later, since the validator and the loader accept any rigid-part glTF that has the required nodes and paint slots.
  - *Licence of the own 3D assets.* §13 suggested CC BY 4.0. **MIT OR Apache-2.0**, like the code and `artwork/`. The validator accepts `MIT OR Apache-2.0`, `MIT`, `Apache-2.0`, `CC0-1.0` and `CC-BY-4.0` (with an author) and rejects anything else (share-alike, non-commercial, no licence). Medium – review by a lawyer before a commercial release (§13).
  - *Loading.* Options: Bevy's glTF loader (asset server, scenes, asynchronous), or reading the files with the `gltf` crate (the one Bevy uses) into our own model and building Bevy meshes directly. **Own reader, files embedded in the binary** (`include_bytes!`): spawning is synchronous (no "scene not loaded yet" frames, one rebuild path), there are no asset paths to package on Linux / Windows / macOS / AppImage, and the validator and the app read the same way. The files are plain glTF facing +Z (glTF convention); the reader turns them to the app's −Z. Medium – textures, skins and morph targets are not supported yet (not needed by the generated models).
  - *Far LOD.* Each bike file has a `lod1` node: frame, wheels, bars and a seated rider (posed by the same code as the near model) in at most 8 meshes (≈ 1/5 of the triangles). Switch at **40 m** from the camera with Bevy's `VisibilityRange` (4 m dither cross-fade); beyond it the near model is hidden and not posed. Low – the distance is a first tuning.
- *Bike type is the profile's.* The model follows only from `ProfileId`; the manifest names the profile a bike shows and the validator rejects physics values (`mass`, `cda`, `crr`, …) in an asset entry: cosmetic assets never change physics.
- *Bots* ride the profiles in turn (road, TT, gravel, MTB, aero, climbing) with palette colours and patterns (never the local rider's #FF1472 jersey).
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

**Implemented (Phase 4: rider model and riding animation):**

- *Model:* one generated low-poly rider (`assets/models/rider.glb`, about 1.78 m) for everyone: jersey (look colour) with a zip and the kit pattern as separate nodes (`kit_band`, `kit_sash`), bib shorts, gloves, socks, shoes, glasses; three head wears as nodes (`helmet_road`, `helmet_aero`, `cap`), one shown: the local rider keeps the cotton cap (white crown in the helmet colour, band and brim in the jersey colour), TT riders the aero helmet, the others the road helmet. Body model, skin colour, hair, shoes etc. are not yet choices (open: needs `RiderLook` fields and the Character page).
- *Rig: rigid parts, no skinning.* Options: a skinned mesh with a skeleton, or rigid parts on a node hierarchy (`rider` → `pelvis` → `thigh_*` → `shin_*` → `foot_*` (+ `cleat_*` anchor); `pelvis` → `torso` → `head`, `upper_arm_*` → `forearm_*` → `hand_*` (+ `grip_*` anchor)). **Rigid parts**: no GPU skinning cost per rider, trivially cheap far LOD, the same nodes for every future body; joints are covered by small spheres. Medium – a skinned rider looks smoother up close; the pose code (below) would stay the same.
- *Pose (`ocw_assets::pose`, pure):* the rig is read from the rider model, the anchors from the bike model. The saddle height of each generated bike is fitted to the rider's legs (largest hip-to-ankle distance 95 % of the leg); seated, the hips are 9 cm above and 3 cm ahead of the saddle anchor; standing, they move forward over the cranks and up until the legs reach 98.5 %. The torso lean is solved so that the shoulders are 90 % of the arm length from the grips: TT riders lie low, MTB riders sit upright, without per-bike lean values. Legs and arms are two-bone IK (knees forward, elbows out; on the TT bike down onto the pads); the feet follow the pedals with a little ankling; the pedals turn with the feet.
- *Riding states (`ocw_assets::anim`, pure):* Stopped (< 0.3 m/s; moves again above 0.6 m/s), Coasting (measured cadence < 15 rpm, or for others without cadence < 10 W; hysteresis 25 rpm / 20 W), Seated, Climbing (seated, hands on the tops – on the TT bike on the extensions below 6 % –, from 4 % down to 3 %), Standing (from 8.5 % down to 6.5 %, only with ≥ 120 W), Sprinting (≥ 650 W and ≥ 7 m/s, down to 480 W; standing on the drops). Each rider's climbing thresholds move by up to ±1.5 % (from the id), so a group does not stand up all at once; pedalling states change at most every 0.8 s, stopping and coasting at once. Pose weights blend with a 0.35 s time constant. Uncertainty: medium – thresholds are tuned by eye.
- *Cranks and wheels:* the wheels turn at speed / radius. The cranks turn at the measured cadence (local rider), else at a typical cadence for the state (seated 88, climbing 80, standing 66, sprint 108 rpm), since others' cadence is not sent; freewheeling they ease forward to level (3 and 9 o'clock) and stay. Standing rocks the bike ±0.10 rad with the pedal strokes (sprint ±0.16), the rider keeps 80 % upright over the contact line; seated the hips rock a little. Fast descents (coasting > 12 m/s below −2 %): on the drops, tucked.
- *Not yet:* cornering beyond the lean, U-turn, finish and victory animations, foot down at a stop.

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
  - **Implementation (decided, Phase 2 basics):** module `ocw_sim::workout` (pure, wasm-safe, so the web companion can show and later edit workouts; `serde` + `toml` are its only new dependencies). A workout is a flat list of segments (steady or ramp target in watts or as a fraction of FTP, or *free*: no target), with optional cadence hint and text, plus timed text cues; intervals are expanded when parsing. Targets are resolved with the rider's FTP only when riding.
  - **Format** (`.toml`): `name`, `description`, `units = "ftp" | "watts"` (how bare numbers are read; `"250W"` / `"88%"` strings override it), `[[steps]]` with `type = "steady" | "ramp" | "intervals" | "free"`, `duration` (seconds, `"M:SS"`, `"H:MM:SS"`, `"90s"`, `"5min"`), `power` or `from`/`to`, optional `cadence`, `text`; intervals take `repeat`, `on`, `off` (each with its own duration and target); `[[cues]]` with `at`, `text`, optional `duration` (default 10 s). Unknown keys are errors (typos), messages name the step. Examples in `crates/ocw-sim/test-data/workouts/`.
  - **Import:** `.erg` (minutes + watts, absolute) and `.mrc` (minutes + % FTP): the `[COURSE HEADER]` / `[COURSE DATA]` / `[COURSE TEXT]` text format. Consecutive data points form a ramp or steady segment, a repeated minute is a step; the `MINUTES WATTS|PERCENT` header line decides the unit (else the extension). Lenient (any case, CRLF, comments, blank lines, extra columns), errors with line numbers; distance-based courses are refused. FIT workout import is still open.
  - **Workout clock:** pedalling time, not wall-clock time. It waits at 0:00 until the rider first pedals, runs while power > 0 and through up to 3 s without power, then pauses until power returns; `workout pause` / `resume` by hand. The ride itself never pauses (§3.2); the ERG target stays set while the clock pauses.
  - **Client-side only:** a workout is target control between the client and its trainer; the server only receives the rider's power, as in any ride (§6.1), so the protocol is unchanged. Shared workouts in group rides or events (everyone on the same step) need the server and come later.
  - **Desktop client:** `--workout FILE` (loaded and started); console `workout load FILE`, `workout start [FILE | --example NAME]`, `workout stop|skip|pause|resume|status|list`.
  - **FTP (decided):** one function (`workout::ftp_w`): `--ftp` on the command line wins (an explicit choice for this session), else the riding profile's `ftp_w` from `riders.toml` (§3.2.1), else 200 W.
  - **Menu (decided, Ride → Workouts):** lists the user's workout files in `<user data dir>/ocw/workouts/` (`.toml`, `.erg`, `.mrc`, by file name; files that do not parse are left out), then the four examples of `crates/ocw-sim/test-data/workouts/`, which are **built into the app** (`include_str!`, ~2 KB) so that installed builds without the source tree (kiosk) offer them too. Choosing one loads and starts it and closes the menu, so the workout panel shows; while one runs the page also shows its step, *Skip step* (the menu stays) and *Stop workout* (closes). No confirmation: starting a workout ends nothing (the trainer mode returns when it stops). The profile preview of §3.2.1 is still open. HUD panel: target, step and time left, next step, cue text, and the workout's profile coloured by training zone with the progress; its content is computed in `ocw_sim::workout::WorkoutPanel`.
- **Export:** every ride is saved as a **FIT** activity file (power, cadence, heart rate, speed, distance, elevation, virtual GPS track). Ridden routes can also be exported as **GPX**. A FIT writer is needed; existing Rust crates mainly read FIT.
  - **Implementation (v1.6):** our own dependency-free, wasm-safe writer crate `ocw-fit` (file_id, device_info, timer events, 1 Hz records, laps, session, activity; sport cycling, sub-sport virtual activity).
  - **Laps (decided):** the activity carries lap marks (elapsed seconds); each mark starts a new `lap` message with its own totals (time, distance, work, averages and maxima), neighbouring laps share the boundary record so their times and distances add up to the session's, and `session.num_laps` counts them. The desktop client marks a lap at each workout start, step change (also by skip), end and stop, and at an event's start (GO), each event lap of a multi-lap race or time trial (from the own progress and the lap length) and the own finish (or the event's end). Joining an event (the countdown) is no lap of its own. Marks within half a second of the ride's first or last record are dropped. The desktop client records the local rider once per second and saves the ride on exit (rides under one minute are dropped) or with `ride.save` to `<user data dir>/ocw/rides/<UTC start>.fit`, with a `.toml` sidecar (title, description). The virtual GPS track is the road position unprojected with the route's geographic origin (`ocw_world::geo`); procedural test routes have none.
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

**Restart and video (v2.0):** R (console `reset`) starts the whole app again as at program start, with the same command line: the app exits normally (ride saved, devices shut down) and replaces its process with a fresh one (Unix `exec`, same process id for the kiosk). V (console `video`) records the window: a screenshot is requested on every 1/25 s tick (several in flight, since the GPU readback takes a few frames), each image is placed at the video frame of its request time, and the frames are streamed as raw pixels into `ffmpeg` (H.264 MP4, `ultrafast`, at most 1920 px wide, in the user's video directory); a tick without an image repeats the previous one to keep real time, the game never waits for the encoder (an image it cannot take is dropped and counted), a screenshot requested by the user or a script in the same frame takes precedence, and a running recording is finished on exit. V again stops it (the key is ignored while the console is open). While recording, the HUD shows a small blinking red dot with "REC m:ss" (recording length) at the top right; after stopping, "Video saved: <file>" (or the error) for 4 s.

**GPS position:** bottom-left, the HUD shows the local rider's position as latitude / longitude (5 decimals, N/S, E/W) and altitude in metres, unprojected from the predicted position with the route's or world package's geographic origin (the same as the FIT track); hidden on routes without one (built-in test routes). Console: `rider.gps`. The HUD panel never extends past the window: long lines break at their " | " separators (then at commas) to fit the window width.

**Camera and avatar (v1.9):** the chase camera follows the avatar itself (on the terrain when off the road) along the rider's own heading; it can be orbited with the mouse. It stays above everything camera-solid and lifts until the line of sight to the rider clears it; its look-at point is smoothed. **Tunnels:** where the road runs under the terrain (the terrain more than 0.5 m above the road centre over at least 10 m, gaps under 20 m merged; found by the client from its track and the ground, so route files, package routes and free ride all work without extra package data), the client draws a simple tube (floor, walls, arch, 1 m beside the road edge, 6 m high, unlit and dark with a strip of lamps along the crown, since sunlight and tree shadows would reach through the hill) reaching 4 m out of the hill at each end as the portal. From 15 m before it until 12 m after it the camera (any mode) blends over 0.4 s into a tunnel camera on the road 4.5 m behind and 2.2 m above the rider, looking 10 m ahead; terrain and buildings above do not lift it, the mouse orbit is ignored there. At the entry portal the hillside slopes over the road (the 10 m terrain grid cannot form a vertical face) and a camera above it would see the hillside instead of the rider, so once the rider's wheels are under the terrain the eye moves closer (at most 1 m behind) and 0.3 m under the terrain, where the terrain is culled (seen from below); at the exit the eye is under the terrain anyway. A few frames can still show the hillside at the entry. Considered and not chosen: tunnel stretches written into the route file / taken from the graph's `tunnel` flag (route files have no flags; tunnels the terrain does not cover need no special camera), hiding the terrain above the road locally (needs terrain mesh holes, shows the inside of the hill), and lowering the camera only (rider still hidden by the hill in curves without a tube). The avatar stands on the surface under both wheel contacts (front wheel first over a bump), leans by atan(v² κ / g) for the curve it rides, and is blended from road height onto the ground over 2 m beside the road.

**Gradient feel:** the gradient at the rider alone does not tell how the road goes on, and a level chase camera makes climbs look flat (the terrain beside the road is a surface model and need not follow the road's gradient, §4.3). Three aids, computed Bevy-free in `ocw-world::gradient` on the rider's `Track`:

- **Average gradient ahead:** the main HUD line shows the gradient at the rider and, in brackets, the average over the next 100 m (height difference over horizontal distance), e.g. `+9.4 % (100 m +7.8 %)`. Where the track ends sooner (a route's finish), the known part counts (at least 10 m, else the gradient at the rider).
- **Mini elevation profile** (bottom left, above the GPS line; console `hud.profile on|off`, on by default): the road from 150 m behind to 1.5 km ahead, sampled every 15 m on a fixed grid of the track distance (the samples stay on the road while the rider moves), drawn as bars with a white rider marker and a caption with the climbing and descending ahead. Heights are scaled to their range, widened to at least 40 m so a flat road stays flat (≈ 8× vertical exaggeration on the 300 × 56 px box); the lowest point keeps 8 % of the height. Bars are coloured by gradient band: < −2 % blue (descent), −2 … 3 % green, 3 … 6 % yellow, 6 … 9 % orange, 9 … 12 % red, ≥ 12 % wine red; behind the rider dimmed. Nothing is drawn where the track is not known (before the start, after a route's finish).
- **Look-ahead in free ride:** the path is only known up to the next undecided junction (§5.2), so the HUD looks ahead on a display-only copy, `RidePath::preview`: extended as if the rider rode on, at the next junction with the exit currently selected (the default if none), at later ones with the default exit (route mode: the route's exit), with the automatic U-turns at dead ends and the region border. It is rebuilt when the path or the selection changes; a different choice later simply changes the preview.
- **Camera tilt** (console `camera.tilt on|off|FRACTION`): the chase orbit is turned around the rider's right axis by 0.7 of the road's slope angle (gradient averaged from 5 m behind to 20 m ahead, low-pass with a 1.2 s time constant, capped at ±8°; snapped after a teleport). Uphill the eye drops behind the rider and looks up the road, downhill it rises. The tilt is applied before the solid-object rules below, so the camera still stays above the slope and buildings; mouse orbit and C (reset) are relative to the tilted frame. Side and top views are not tilted.

**What moves the camera:** every world object is in one of two classes (`CameraInteraction` in `ocw-render::camera`):

| Class | Objects | Camera |
|---|---|---|
| **Solid** | Terrain, road, buildings | Avoids them: stays above them (and 0.5 m away from walls) and lifts until the line of sight to the rider clears them. Terrain comes from the ground height function; every other solid object registers the height of its top per tile as a camera obstacle layer, removed with the tile (buildings: footprint prisms up to the ridge, courtyards counted as solid, grid-indexed). |
| **See-through** | Trees; later bushes, signs, other riders | Never change the camera position. Where one would block the view – within ~3 m of the lens, or in a cone around the line of sight from the camera to the rider (≈0.8 m radius at the camera, 2 m at the rider, ending 1 m before the rider) – it is faded out with screen-door transparency (ordered dither in the fragment shader; no blending, so depth sorting stays opaque and the shadow stays). |

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
| | Audio | Mix: master, music, effects (bike, wind, ambience), voice; output device; later voice: microphone, microphone / speaker mute, spatial voice on / off | output + mix with game audio (see §3.3); voice Phase 6 |
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
- **Starting another ride** (Ride page; decided in M3): an **in-app switch**, not the restart path – the process, the window and the devices stay. (The earlier plan, writing the choice to `[ride]` and re-executing the app, was dropped: a re-exec reconnects the BLE trainer, which takes seconds and can fail.) Nothing is written to `[ride]` yet; the app still starts with its command line.
- **Implemented (M3):**
  - *Ride choice* (`apps/desktop/src/ride/choice.rs`, Bevy-free): a test route (hills, loop), a route file, or a world package in free ride (random start or `--start X,Z`), on a route of the package by name or an `ocw-graph-route` file, on a GPX file matched at the start, or with a route file. It is built from the command line, from the console command `ride.switch --route NAME | --route-file FILE | --world PKG [--free-ride] [--start X,Z] | --world PKG --route NAME|FILE | --world PKG --route-gpx FILE` (a world alone is a free ride) and turns back into the ride options of the arguments, so the start and the switch share one setup (`ride/setup.rs`: route, route-file terrain, package with neighbours and road network, free-ride start or route plan; errors are messages, not exits). `ride.current` prints the current ride as a `ride.switch` line; `ride.restart` starts it again.
  - *Switch sequence* (`ride/mod.rs`): the new ride is loaded on a background thread (package reading and verification, neighbours, road network, GPX matching) while the old one goes on; the HUD notice says "Loading … ". A failed load keeps the ride and says why. When it is loaded, in one frame: the ride so far is saved as a FIT file like on exit (under a minute: dropped) and handed to the Strava sync; the ride options, `WorldInfo`, `PackageWorld` and `FreeRideSetup` are replaced and the `RideView` gets the new route; a new session starts (`connection::start_session`: dropping the old local server's link ends its thread, a new local server – threaded, or stepped in frame capture – and a new `ClientCore` join, with the rider profile active *now*: the Workshop's rider and bike choice applies from the next ride, not only the next start); the recorder starts a new ride, the ride time starts at 0 (`SessionStart`), and `ocw_render::rebuild_scene` despawns the old world (light, terrain, road, trees, streamed tiles, camera obstacles, avatars) and spawns the new one from the new `RideView` / `PackageWorld`. Devices (BLE trainer, sensors, gamepads), the trainer mode (SIM / ERG, difficulty), a running workout, the camera, HUD, menu and console stay; a running local event (§2.3) ends with its session (server and client are new) and its HUD state is cleared. **A video recording continues** (decided): it records the window, and the switch is part of what the user sees; V stops it as before. With `--server` (a remote server) the switch is refused: the server chooses the course.
  - *R after a switch* restarts the current ride **in the app** (decided): the process' command line would start the first ride again. Without a switch, R still re-executes the app.
  - *Menu:* Ride → *Change ride* (a sub page next to the event rows) lists *World packages* (the newest version of each package in the cache and the `.ocwpkg` files next to the executable – the kiosk installs one there –, in `data/` and `data/pilot/` of the working directory and in the directory of the package ridden now; one package: *Free ride* from a random road, *Free ride from here* (the current position, in the package ridden now) and its *Routes*) and the test routes. Navi has *Routes* (the routes of the loaded package from section `ROUT` with their length, and the user's routes for this package), *Rides* (`rides/*.fit`, newest first, with the route from the sidecar and the Strava state from `strava-sync.toml`), *GPX import* and *Strava* (a page describing what comes with the backend; no network calls). Rows that start another ride ask first (*Change the ride?*, Cancel focused); confirmed, the menu closes and the switch runs. The lists are read from disk when the menu opens and after a switch or import, not every frame.
  - *Paging (ten-foot rule):* a list with more than six rows shows four rows per page with *Previous (n/m)* / *Next (n/m)* and Back (at most seven items); ← / → on a row without a value turn the page. This also covers the Workshop and Riders pages with many rider profiles.
  - *GPX import* (`ride.import-gpx FILE [--ride]`, §2.7): the file is matched onto the road graph of the loaded package (and its joined neighbours) with the HMM matcher of `ocw-world::matching` – the same as `--route-gpx` and `world-builder gpx` – and saved as an `ocw-graph-route` file with the package name and graph fingerprint in `<data dir>/ocw/routes/` (listed under Navi → Routes for that package; re-matched by stable edge IDs after a package rebuild); the matching report (length, off-road stretches, U-turns) is printed. `--ride` rides it at once. The menu offers the `.gpx` files in `<data dir>/ocw/gpx/` and the downloads folder (no file dialog on the gamepad). Without a loaded package with a road graph there is nothing to match onto, and the import is refused (a plain GPX-to-route-file conversion would need terrain data the client does not have). The matching runs on the main thread (a long track can stall a frame for a moment); the import report with highlighted problem sections is M4.
  - *Not yet:* the number of bots per ride (the `ride.bots` setting applies), the start point on a map (Phase 7), named start places, renaming / deleting routes, FIT / GPX export rows, package downloads from the menu.
- **Rider weight affects the physics (decided, v2.2):** the active rider's weight (and later the bike's physics profile, §2.5) goes to the server in the join message (new field) – the server stays the truth (§6.1) – and to the trainer (SIM total mass); today's fixed 75 kg + 8 kg profile becomes the default rider. Riders are created in *Settings → Riders* and selected in the *Workshop* (the Workshop's rider choice comes before Phase 4's editors).
- **Implemented (M2, v2.7):**
  - *Registry* (`apps/desktop/src/settings/registry.rs`): each setting has a key `section.name`, a type (switch, whole number with range and step, choice), a default, a description and a scope – *live* (an apply command runs on every change and at the start, e.g. `display.fps` → `fps on`) or *next start* (read when the app starts: `ride.bots`, `trainer.virtual_power`, `ride.record`, `ride.random_start`, `display.fullscreen` fill in the command-line option of the same meaning; `controls.autodrive`). Initial set: `controls.autodrive`, `trainer.difficulty`, `trainer.virtual_power`, `display.hud`, `display.units` (metric / imperial), `display.fps`, `display.camera`, `display.fullscreen`, `ride.bots`, `ride.random_start`, `ride.record`. The key is `controls.autodrive`, not `controls.autodrive_default`.
  - *Layers:* a command-line option the user gave (clap's value source, the args file included) becomes the setting's session value; a setting only replaces an option's default when it is in the file, so screenshot and capture runs keep their reproducible defaults. `set` saves at once (not only when the menu closes) and replaces a session value; `set --temp` is session-only. `autoexec.cfg` runs after the settings' startup commands, so its commands win; a `set` in it is saved like one typed in the console (use `set --temp` there for session-only values). The settings' apply commands are queued before `autoexec.cfg`, `--exec` and `--cmd`.
  - *Files:* a broken `settings.toml` (not TOML) is left untouched; the app uses the defaults and does not save. Invalid entries stay in the file until set again. `--settings-file` names another file; `riders.toml` is read next to it.
  - *Rider profiles* are not in `settings.toml` but in `riders.toml` (`active = NAME` and `[[rider]]` tables: `name`, `weight_kg`, optional `height_cm`, `ftp_w`) next to it, since they are a list rather than variables. Weight 30–200 kg (`ocw_core::MIN/MAX_RIDER_WEIGHT_KG`; entered values outside are refused, values in the file and from the network are clamped), height 100–250 cm, FTP 30–700 W, names up to 32 characters without quotes. Without profiles the rider is the default (75 kg). `--rider NAME` rides another profile for the session, `--name` still sets only the display name. Units are one global setting (`display.units`), not per rider.
  - *Join message:* `Hello.rider_weight_kg` (field 5). The server builds the physics profile from its curated default (§2.5) with that weight (clamped); the client's `Hello.profile` (drag, rolling resistance, bike mass) is deprecated and ignored except for its rider mass when field 5 is absent. The client predicts with the same profile.
  - *Trainer:* the SIM mass is rider weight + 8 kg bike; Wahoo's SIM mode carries it (sent again when it changes); FTMS has no mass field (its Indoor Bike Simulation parameters are wind, grade, crr, cw), so FTMS trainers get only the gradient. The FTMS User Data Service (weight) is not used.
  - *When it applies:* a changed or newly selected rider applies from the next start (R restarts); the server has no message to change a rider's mass mid-session.
- **Workshop → Bike (Phase 4 groundwork, M6 in part):** the Workshop lists the riders, then a *Bike* row with the chosen rider's bike and colours that opens the *Bike* page of that rider (`Page::Bike`, Back goes to the Workshop): *Bike* (← / → step through the six profiles, named by bike type and setup: Road – standard / aero / climbing, Time trial, Gravel, Mountain bike; one row instead of separate type and setup rows keeps the page at six rows), *Physics* (the profile's bike mass, CdA and Crr, read only – shown separately as planned), *Frame / Jersey / Helmet colour* (← / → step through a 12-colour palette; any `#RRGGBB` from the console) and *Kit* (plain, band, sash). Each row runs `riders.edit "NAME" --bike ID | --frame-color #RRGGBB | --jersey-color … | --helmet-color … | --kit …`, the same options as on the console (`riders.add` takes them too). Like the weight, bike and look apply **from the next start** (decided): the server has no message to change a rider's profile mid-session, and a live change of only the own avatar would show the others something else; a live preview comes with the 3D preview. Without rider profiles the Bike row only shows the default bike (add a rider to change it). Still open in M6: the 3D preview, components (wheels, tyres, saddle, …) and the character page.
  - *Pages:* the Settings sub pages and the Workshop are a Bevy-free model (`settings/pages.rs`): each row has a label, a value and the command lines for ← / → / Confirm (or a page to open); the menu (`menu::model`, `Page::Sub`) shows value rows as "Label:  < value >", ← / → run the row's command, Q / E, PgUp / PgDn and LB / RB switch between the tabs Workshop, Riders, Controls, Trainer, Display, Strava (replacing the page, so Back still goes to the parent); the rows are read again every frame, so values changed from the console show at once. A rider editor whose rider was deleted goes back to the list. Renaming is a console command for now (`riders.edit NAME --rename NEW`; the on-screen keyboard comes later). The Workshop lists the riders (Confirm = `riders.select`); *Riders* lists them for editing (weight ±1 kg, height ±1 cm, FTP ±5 W, rename, delete, add). Console: `riders.list / add / edit / remove / select`.
- **Implemented (M5):** *Settings → Audio* (between Trainer and Display, also a tab): *Master volume*, *Effects (bike, wind)*, *Music*, *Voice* (← / → in 5 % steps; Enter plays a test sound on that bus), *Mute*, *Output* (← / → step through `default`, a saved but missing device, and the devices the engine lists; the value shows "default", "default (none)", "DEVICE" or "DEVICE (gone)", names cut to 16 characters – the row has no room for more in the ten-foot layout; `audio.status` and `audio.devices` show the full names). Six rows and Back, so no paging. Each row runs `set audio.KEY VALUE`; the device list is read again when the page opens. The menu plays the UI cues (move, value, page, back) itself, from the change of what it shows. Game audio and the engine: §3.3.
- **Local now vs. backend later:** settings, devices, routes, GPX import, rides, the interim Strava upload and the Workshop configuration are local. Accounts and profile sync (§2.1), Events and sessions (§2.3), the backend Strava upload (§2.8), sharing routes and friends need the Phase 5 backend; those pages appear disabled with a short hint until it exists.

**Increments** (each with unit tests, §11.4; the menu model, settings and actions are tested without Bevy or with `MinimalPlugins` + `ConsoleCorePlugin`)

| # | Increment | Phase | Size | Tests |
|---|---|---|---|---|
| M1 | Input contexts, menu shell (Resume, Restart, Quit), Esc / Start, focus navigation for keyboard, mouse and gamepad, ten-foot theme; the ride continues under the menu (autodrive and default exits while it is open) | 3 (now) | M | navigation model (focus moves, wrap, page stack, Back at the top closes); key / button → action mapping; ride inputs ignored in `Menu`; Esc closes the console first and never quits; steering falls back to autodrive while the menu is open and is restored after; ride time, trainer and recording unaffected; headless app opens the menu and selects Resume; X11 + Wayland smoke screenshot of the menu |
| M2 | Settings registry, `settings.toml`, `set` / `get` commands; Settings pages Controls (options), Trainer, Strava, Display, Riders (profiles: name, weight via the join message and SIM mass, FTP, units), rider choice in the Workshop | 3 (now) | L | registry validation and ranges, TOML round-trip, precedence (file < autoexec < CLI, temp values not saved), unknown keys kept; each page item → expected command line; weight reaches the server's physics profile |
| M3 | Ride → World mode picker (installed packages, free ride / package routes / test routes, bots) as an in-app switch (done: see *Implemented (M3)*; bots per ride still open); Navi → Routes list, Rides list with Strava state and FIT export, Strava sync page | 3 (end) | M | package discovery on a fixture directory, `[ride]` choice → command line of the restarted process, ride list from fixture FIT + sidecar + `strava-sync.toml` |
| M4 | Navi → GPX import with map-matching report; GPX export of rides | 3 (with §2.7) | M | GPX fixtures through import → route; problem sections listed; GPX export round-trip |
| M5 | Game audio and Settings → Audio (mix, output device) (done: see §3.3 *Implemented (M5)*) | with game audio (see §3.3) | M | bus volume maths (master × bus), device selection fallback to the default device when the saved one is missing, settings round-trip |
| M6 | Workshop: bike and character pages with 3D preview | 4 | L | configuration model and persistence (§14 Phase 4); cosmetic choices never change the physics profile; menu items → configuration |
| M7 | Accounts / profile sync, Events (sessions, join), backend Strava, route sharing; start-screen dashboard (§2.1) | 5 (calendar 7) | L | API client against a mocked backend, disabled-page states offline, event list and join flow |
| M8 | Voice settings (microphone device, microphone mute, speaker mute, voice volume, per-participant mute, spatial voice on / off) | 6 | S | device selection and mute state (§14 Phase 6) |
| M9 | Rebinding of keys and buttons, accessibility (text size, colour-blind safe HUD) | 7 | M | binding conflicts, persistence, defaults restore |

**M1 implemented:** Esc / Start opens the menu over the running ride: Resume, Ride, Workshop, Navi, Settings, Restart, Quit; Ride, Workshop, Navi and Settings are placeholder pages that say what their increment brings (Settings already toggles the FPS counter). Restart and Quit ask first, with *Cancel* focused, and run the console commands `reset` / `quit`; the console command `menu open|close|toggle|up|down|select|back` drives the menu from scripts and `--cmd`. Code: the Bevy-free model (`apps/desktop/src/menu/model.rs`: pages, page stack, focus with wrap-around), the key / button → action mapping with hold repeat (`menu/controls.rs`), the drawing of a `MenuView` in `ocw-render::menu` (text sizes in % of the window height). Deviations from the plan above:

- No Bevy state `UiMode`: one resource `InputContext { Ride, Menu, Console }` (console over menu over ride) is decided once per frame in `PreUpdate`, and the ride's input systems (power, steering, A / W / U / J / Q / E, R, V, F11, the mouse camera and C) run only in `Ride` (`run_if(ride_input)`); a key that closes the menu is therefore not also seen by the ride in that frame. The console keeps its own open flag.
- Steering while the menu is open (`ClientCore::hold_steering` / `release_steering`, so server and prediction agree): wild mode off, autodrive on – from manual steering at the rider's current lateral position (no swerve), an autodrive line stays where it was; the next junction's choice is reset to its default exit and not restored on closing. Closing restores manual steering upright (the lean is not carried over) and wild mode; autodrive keeps its line. The HUD shows "autodrive (menu)".
- The mouse hover moves the focus only while the mouse moves (rows of a new page appearing under a resting pointer do not take the focus); right click = Back.
- ← / → and tab switching (Q / E, PgUp / PgDn, LB / RB) had no rows or tabs to act on in M1; M2 (v2.7) uses them on the Settings sub pages and the Workshop (see *Implemented (M2)* above). The FPS toggle moved to *Settings → Display*.
- The status strip shows power, speed and the device / trainer / steering status line; the Strava sync state joins it with the Navi pages (M3).
- Smoke screenshots of the open menu are taken on Wayland (1280×720 and 800×600); the build machines with a GPU have no X11 server.

**Open questions**

Decided (v2.2): no pause (trainer unchanged, ride time and recording continue); Esc never quits; rider weight affects physics, riders managed in Settings and chosen in the Workshop. Decided: local events before the backend (§2.3.1) - the Ride page starts a race, a group ride or a time trial on the current route (console command lines `event race --bots 5`, `event group --watts 150`, `event tt`, `event cancel`; the starts ask first, then the menu closes for the countdown). Still open: switching rides by restart vs. in-app session switch (M3).

### 3.3 Audio

Two separate audio systems:

**Game audio** (own audio module, decided in M5): ambient sounds, bicycle sounds, wind, tyres, freewheel, gear shifting, UI and event cues; background music later.

**Implemented (M5):** `apps/desktop/src/audio/`, Bevy-free except its plugin (`mod.rs`).

- *Options evaluated:* (a) `bevy_audio` (rodio) – plays only on the default device, no device list, no bus mixer: rejected (it is now disabled in the app). (b) `kira` 0.12 – a good game mixer (tracks, tweens, spatial), but it links **cpal 0.18** while Bevy 0.20 links cpal 0.17, so the app would carry two cpal copies (two ALSA bindings), and it is built around sample files we do not have; its device choice is no better than cpal's. (c) **cpal directly with an own small mixer** (chosen): cpal 0.17 is already in the lock file through Bevy (Apache-2.0, no new system library: `libasound` is linked anyway), it lists and opens devices by name on ALSA / WASAPI / CoreAudio, and since every sound is synthesised in code a sample-playing layer (rodio) is not needed – the mixer renders straight into the device buffer at the device's rate, channel count and sample format (f32, i16, u16, i32). If music files come later, rodio's decoders (or symphonia) can feed the music bus.
- *Engine:* an `OutputBackend` trait (list devices, default device, open by name, close, stream failed) with the cpal backend, a `NullBackend` (no sound) and a mock for tests. The engine runs on its own thread (cpal streams are not `Send` on every platform) and talks to the game through channels: commands (output, refresh) one way, a status (device list, default, playing device, fell back, message) the other; the mixer gets its input (ride parameters, cues, bus gains) through a bounded queue, read at the start of each device buffer (cues older than 0.4 s are dropped, so nothing stale plays after a device came back).
- *Device choice:* `audio.output` is a device name (cpal's device description, e.g. "USB Audio Device, USB Audio" on ALSA) or `default`; matching ignores case and surrounding spaces; several devices with one name (ALSA lists a card as `hw:`, `plughw:`, `sysdefault:`) are tried in order until one opens. A missing device falls back to the default device (shown as "NAME (gone)" on the Audio page). **Hot-plugging:** the device list is re-read every 3 s (cheap: cpal's ALSA listing does not open devices); a device that comes back is opened again, a stream that fails (unplugged: cpal's `DeviceNotAvailable`, `StreamInvalidated`, backend errors; underruns are ignored) is closed and the default opened. A failed open is retried after 15 s. Without any device (headless Xvfb / weston runs, CI) the app runs silently with one log line ("audio: no audio output: …"); `--no-audio` and frame capture (`--capture`, game time not real time) use the null backend.
- *Buses:* master × music / effects / voice; each volume is 0–100 % on a squared curve (gain = (v/100)², 50 % ≈ -12 dB), the bus gain is master gain × bus gain, mute sets all to 0; gains are smoothed (60 ms) so changes do not click; a soft limiter (linear to 0.8, tanh knee) keeps the sum within ±1. Pure and unit-tested (`mix.rs`, `mixer.rs`).
- *Sounds (all procedurally synthesised, our own work – no sample files, nothing to license, §13):* **wind** – pink noise, low-pass (250 Hz + 120 Hz per m/s) plus a resonant band, level ∝ (v / 54 km/h)^1.6 with slow gusts; **tyres** – rolling rumble on every surface, louder on gravel; gravel adds random grit bursts (6 per metre ridden), grass a high swish (surface from the track and the rider's lateral position, §4.4 / §5.2.1, as for the physics); **freewheel** – a short metallic click per pawl engagement (24 per wheel revolution, 2.1 m wheel) while coasting (cadence < 15 rpm), jittered, level kept even as the ticks become a buzz; **drivetrain** – noise pulsing at the chainring tooth rate (cadence × 50 teeth / 60) with a soft hum and a swell per pedal stroke, a little louder under load; without a cadence sensor the cadence is estimated from the power (75–100 rpm); **cues** – menu move / value / page open / back, countdown beeps (3, 2, 1) and a higher GO, a rising finish chime (results panel of a race or time trial), a two-tone workout step cue (each start, step change, end, stop), a gear-shift double click (`audio.play shift`; not triggered automatically yet: the protocol has no gear) and a test tone per bus. The mapping ride state → sound parameters (`ride.rs`) and the cue triggers (`cues.rs`: from the `MenuView`, the event countdown text, the results panel, the workout's change counter) are pure and unit-tested; a scripted 20 s demo ride renders offline through the whole mixer (`audio::demo`, `OCW_AUDIO_DEMO=FILE.wav` writes it).
- *Settings and commands:* `audio.master` (80), `audio.effects` (80), `audio.music` (60), `audio.voice` (100), `audio.mute` (off), `audio.output` (`default`; a new setting type *text*) in `[audio]` of `settings.toml`, live; console `audio.volume BUS PERCENT`, `audio.mute [on|off|toggle]`, `audio.output NAME|default`, `audio.devices`, `audio.status`, `audio.play SOUND`.
- *Uncertain / open:* the levels and timbres are set from the formulas and checked by measurement (levels, ranges) only – nobody has listened yet: the demo WAV and the kiosk are for that; device names are cpal's descriptions, which are not unique on every system (the first that opens wins) and may change with a driver update (then the default plays); the WASAPI and CoreAudio paths are build-checked but not yet heard; no sounds of the other riders (bots passing, drafting) yet, no music, no ambience (birds, traffic), no ducking under the menu; the freewheel is synthesised for a road hub – a trainer's own freewheel noise in the room is not compensated.

Voice (Phase 6) plays through the same output setting via its own LiveKit audio path, with its own microphone selection; its volume is the voice bus (`audio.voice`), either applied to LiveKit's playout or by feeding the decoded voice into the mixer's voice bus (decided with Phase 6).

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
- **Gradient limit across junctions:** the through-road filter fixes junction heights road by road, so a short connector between two roads filtered on their own (or a bridge between junctions of two roads) could stay far steeper than its class allows (on the pilot region 42 edges, up to 203 %). A last pass relaxes the junction heights again over all edges (bridges and tunnels included), re-limits every road whose end moved locally between its new ends (the correction fades out over 200 m), and flattens what is left sample by sample across the junctions. Every edge of the pilot region holds its class limit (tested).
- **Distances along the road** are the true arc length of the centreline (also for road strips and route files), not the chords between 5 m samples, which cut every bend.
- **Tunnels and bridges in the terrain:** tunnel edges (`tunnel=*` other than `no`; the `tunnel` flag in the road graph) are left out of terrain conformance and get no road strip, so the terrain stays the natural hill above them and the client draws its tunnel tube there (§3.2). Bridge edges cut the terrain under them to below the deck (the cutting rule below) but never pull it up to the deck. `layer` is not used yet.
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
  4. **Region borders:** edges cut by the package boundary are marked `boundary`; the cut end is the exact crossing point with the boundary, so neighbouring packages cut along the same line end at the same point and an installed neighbour package continues the road seamlessly (§2.2.2).

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
- Across a region border, the tiles of an installed neighbour package are streamed the same way, placed with its frame in the home package's metres; neighbour packages are loaded and dropped by distance (§2.2.2).

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

**Implementation (Phase 3, land use and buildings):** the first step uses OSM only, no imagery and no textures yet.

- **Classes:** OSM areas are classified into 17 land classes: default (no land use, grass), meadow, farmland, forest, orchard, vineyard, residential, commercial, industrial, park, water, wetland, rock, scree, glacier, bare (sand, quarries, construction sites) and scrub. Sources: closed ways and `type=multipolygon` relations (outer rings assembled from split member ways, inner rings as holes; even-odd fill) with `landuse`, `natural`, `leisure`, `waterway=riverbank` tags, plus `waterway=river|canal|stream|ditch|drain` lines buffered by their `width` (defaults 12 / 6 / 2 / 1.5 m; culverts and tunnels left out).
- **Splat map:** each 1 km tile gets a class per **5 m cell** (200 × 200, scanline rasterisation at the cell centres in global coordinates, so neighbouring tiles agree at their edges). Where areas overlap, a fixed **priority** decides, not the drawing order: water > glacier > wetland > rock > scree > bare > park > forest > scrub > orchard / vineyard > meadow > farmland > commercial / industrial > residential > default; specific surfaces beat the large built-up areas they lie in.
- **Water surfaces:** water polygons and rivers ≥ 8 m wide mark the terrain grid vertices in water. The water level at such a vertex is the mean terrain height of the water vertices within 50 m (flat on lakes, gently falling along rivers); the terrain there is carved 1 m below it. The level is also stored for the vertices next to water, so the client draws a flat water quad over every grid cell touching water and the shoreline is where the terrain rises through it. Narrower streams only colour the ground.
- **Rendering:** the client turns the splat map into a per-tile **colour map** (one texel per cell, sRGB, mipmapped, class colour turning to rock on slopes above 60 %, a little noise), the terrain vertices stay white. Water is a separate flat, glossy mesh. **Trees** come from a jittered 8 m grid with a chance per class (forest 85 %, orchard 45 % small, scrub 30 % small, park 10 %, residential 6 %, meadow / farmland / default 1.2 %, none on water, rock, scree, glacier, bare or vineyards), never in cells near road strips (3 m beyond the edge) or under buildings (1.5 m around the footprint), not on rock faces; the same trees are the camera's obstacles. Packages without land use keep plain grass and provisional tree patches. Tiling PBR materials, ESA WorldCover and the satellite tint (steps 1–3 above) follow later.
- **Buildings:** OSM `building=*` closed ways and multipolygons (not `building:part`). Wall height from `height` (metres, feet), else `building:levels` × 3 m, else by type: houses and farms 6 m, apartments 12 m, commercial and public 9 m, industrial 8 m, churches 12 m, garages / sheds / huts 3 m, unknown 6 m (3 m under 40 m²). Nearly rectangular footprints (≥ 85 % of their oriented bounding rectangle, no courtyard) of houses, farms, small buildings and churches up to 400 m² (barns 1,500 m²), or anything tagged with a pitched `roof:shape`, get a **gable roof** on that rectangle, ridge along the long side, 0.4 × the width high (`roof:height` overrides; `height` includes the roof); all others a flat roof. Placement: the walls start 0.5 m below the lowest terrain under the footprint (its corners and a 4 m grid inside) and end the wall height above the mean terrain, at least 2 m above the highest. Flat roofs are triangulated by ear clipping, courtyards bridged into the outline. Buildings belong to the tile of their centroid and are merged into one mesh per tile.

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

**Land use and buildings sections (Phase 3):** `LUSE` holds the splat map of every tile (layout version, cells per side, then per tile the classes run-length coded row by row and the water levels as (terrain vertex index, level) pairs); `BLDG` holds the buildings per tile (kind, roof shape and height, base and eave height, outline and courtyard rings in tile-local metres). Both are optional: older clients skip them as unknown sections (tested), newer clients without them fall back to plain grass. The pilot package grows from about 26 MB to about 30 MB with them.

**Region section (decided):** `REGN` holds the region the package was cut to: the latitude / longitude box and the polygon rings (outer and holes) of the `.poly` file, if any (layout version, four `f64`, then the rings as (lat, lon) `f64` pairs). It is what neighbouring packages are matched and drawn by (§2.2.2); `world-builder package --bbox S,W,N,E` narrows the `--poly` region to a box (cutting a region into neighbours), `--origin LAT,LON` sets the projection origin of the local metres (default: the box centre). Packages without it still load but cannot have neighbours. The manifest repeats the box as `bounds` (taken from the section when the package file is added), so clients find neighbours without reading every package.

**Routes section (decided):** `ROUT` holds the package's routes on the road graph (§2.7): per route its name, a loop flag and the steps (stable edge ID with from / to metres as `f64`, or a U-turn). A package with only routes and no graph is still version 2. `world-builder gpx` and `route --world` add routes; readers that do not know the section skip it.

**Route files (v1.9, interim):** a plain-text `ocw-route 1` file holds one route with its elevation and the terrain grid around it (`name`, `origin` lat/lon of the local metres, `width`, `points` x y z every 5 m, optional `surface FROM TO NAME` stretches that are not asphalt, `grid`). It is written by `world-builder route` and ridden with `--route-file`, optionally together with a world package (`--world`) for the surroundings. Routes as part of packages (above) replace it; it keeps working until the last users are moved to package routes.

Packages are **signed, versioned and updatable**:

- **Ed25519** signatures. An offline root key signs rotating online signing keys. The client ships the root public key.
- Each package manifest lists SHA-256 hashes of all files.
- **TUF** (The Update Framework) is evaluated for package and app updates.
- Target size: 200–500 MB for a mid-sized region.
- The format allows streaming over HTTP in tile-sized chunks (§3.5).

**Manifest, signatures and cache (implemented, Phase 3):**

- **Manifest** (`ocw_world::manifest`, pure and wasm-safe; ed25519-dalek + sha2): a JSON file next to the package (`<file>.ocwpkg.manifest.json`; `manifest.json` in published and cached directories). The signed part holds the format (1), the **package id** (lower-case path segments, e.g. `europe/liechtenstein`), region name, **version** (build number), creation time, generator (`world-builder 0.1.0`), package format (`OCWPKG02`), the **attributions** (© OpenStreetMap contributors with the ODbL Derivative Database notice for the road graph, the DEM source, §13) and every file with size and SHA-256 **plus each chunk** (header, every tile, every section) with offset, size and SHA-256, so single tiles fetched with range requests can be verified on their own.
- **Signature format:** the signed part is kept as raw JSON bytes (`"signed": {…}`) and signatures cover exactly those bytes behind a per-kind context string (`OCW world package manifest v1\0`, `OCW signing key v1\0`). No canonical JSON is needed, and fields added later do not break old signatures; unknown fields are ignored.
- **Key hierarchy:** the **root key** (Ed25519) is generated once (`world-builder keys init-root`) and kept offline; it only certifies **signing keys** (`keys new-signing`, default validity 365 days): a certificate with the signing key's public key, name, root key id and validity period. Each manifest carries its signing key certificate, so the client needs only the root public keys. Key id = the first 8 bytes of SHA-256 of the public key. Secret key files (`*.key`, mode 0600, never overwritten) live outside the repository (`~/.config/ocw/keys`, `$OCW_KEY_DIR`); `.gitignore` also ignores `*.key`.
- **Verification chain:** certificate signed by a trusted root → neither key revoked (a revocation list compiled into the client) → manifest signature by the signing key → manifest created within the signing key's validity period and not more than a day in the future → each file's size and SHA-256. Validity is checked against the creation time, not today, so a cached package keeps working offline after its signing key expires, while an expired or stolen-and-revoked key cannot sign new packages.
- **Trusted roots:** production roots are compiled into the client (none yet; they come with the distribution infrastructure). Development roots (`crates/ocw-worldstore/keys/dev-roots.json`, public keys only, marked development) are trusted by debug builds and with `--trust-dev-root`. More roots: `--trust-root FILE` and `~/.config/ocw/trusted-roots/*.pub.json` (private servers, own builds).
- **Policy for packages not signed by a trusted root:** integrity failures (hash or size mismatch, bad signature, revoked key, malformed manifest) are **always refused**. Packages that are intact but unsigned, signed by an unknown root or outside the key's validity are a trust failure: from the **cache** (by id) they are **refused** unless `--allow-unsigned`; from a **path** (local builds, power users, §4.2) they load **with a warning** unless `--require-signed`. A package without a manifest counts as unsigned (id `local/<name>`, version 0). Multiplayer sessions (Phase 5) will require trusted packages.
- **Identity:** id, version and the SHA-256 of the package file (`Manifest::identity()`, `PackageIdentity`) tell client and server whether they ride the same world (and the same stable edge IDs). Builds are deterministic, so the same input gives the same content hash on every build machine.
- **Cache:** `$OCW_WORLD_CACHE` or `~/.local/share/ocw/worlds/<id>/<version>/{manifest.json, <files>}`; downloads collect in `.partial/<id>@<version>/` and are moved into place only when every file is verified. Packages are verified again on every load. `--world` takes a path or a cache id (`ID` = newest version, `ID@VERSION`); `--list-worlds` and the console commands `worlds.list` / `worlds.verify` show and check the cache.
- **Download:** `ocw-desktop --fetch-world URL` fetches the manifest first and checks its signature before any large download, then the files next to it (the layout `world-builder publish` writes: `<base>/<id>/<version>/manifest.json`), resuming interrupted downloads with HTTP range requests (`Range: bytes=N-`; a server answering 200 restarts the file). Local directories and `file://` URLs install the same way. Downloads happen before the ride, never during it (§4.2). Tile-chunk streaming uses the chunk list of the manifest later; TUF remains under evaluation for app updates and key rotation metadata.
- **Rebuilds:** `world-builder diff OLD NEW` (`ocw_worldgen::diff::GraphDiff`) lists the edges whose stable ID survived (unchanged or moved), was removed or added; routes over surviving edges re-match directly, the others are flagged.

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

**Low speed and steep ramps (decided):** the drive force is P/v. Below 1 m/s it is taken at the speed at the end of the tick (implicit in that term, `m·(v₁ − v₀)/dt = P/v₁ − R`, one positive root): finite from standstill, stable however slow, and its fixed point is the exact steady speed P = R·v. So a rider with positive power never stays at 0 km/h: on a ramp too steep for the power they crawl at that steady speed (33 % at 200 W and 83 kg: ~2.6 km/h) instead of stalling; speed is never negative (no rolling back; at 0 W the rider stops). The rider's measured power is evidence that the cranks turn, so the bike moves (a real rider would get off and walk at a few km/h – the crawl stands in for it). With the brake key held, the drive force is taken as if riding at 1 m/s (the old floor), so the brake still holds a moderately pedalling rider at standstill. Server and client prediction share the function (`ocw-sim::physics::step`).

### 5.2 Simulation Model

- **Riders move in 1D along their path:** state = `distance_along_route: f64`, `lateral_offset: f32`, `speed: f64`. The 3D position is derived from the road geometry only for rendering. This makes the physics cheap, robust and easy to synchronize.
- In free ride the path is an **edge list that is extended on the fly** by each junction decision (§2.2.1). The lateral offset is blended across junctions so the avatar follows a smooth curve, not the raw graph corner.
- **Free-ride path (v1.6):** a list of steps – (part of) an edge with its riding direction, or a U-turn loop (§2.2.2). It is extended automatically where the road just continues (up to 2 km ahead), with a U-turn at dead ends and at the region border, and at the **lock point** (20 m before the next junction) with the chosen exit. Corners at junctions are rounded (a curve starting at most 10 m before the junction, so it is never under the rider at the lock point); the height along the curve follows the road by arc length, never bunched into the apex: in a window of up to 20 m of road before and after the curve, the path position maps linearly onto the road position (2t + 2X metres of road onto L + 2X of path) and takes the road's height there. The curve is shorter than the road it replaces but climbs as much; spread over the window, a hairpin on a 10 % road is ~12 % (the curve alone would be ~2×). The geometry is a pure function of the step list, so the server, the client's prediction and the client's copy of other riders' paths agree; steps more than 600 m behind the rider are dropped, step indices keep counting. Route files and test loops keep their fixed route; physics and rendering use either through the same interface (`Track`).
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
- **Road edge:** the rider stops at the edge (0.6 m inside); while still steering (or carried by a bend) into it, the rider scrapes along it with an extra deceleration. Rolling straight along the edge or steering away from it does not scrape. **Wild mode:** no edge; the rider may leave the road (up to 60 m either side, at up to ~70° to it) and rides on the terrain. Switching wild mode on also switches autodrive off (upright, manual lean steering; one steering intent carries both), since autodrive would hold the rider on its line on the road; switching it off keeps the steering mode (autodrive can be switched on again at any time). Off-road gradients are still the road's (a simplification until riders move freely in 2D).
- **Surfaces:** asphalt, gravel (OSM tracks and paths; the `surface` tag later), grass (anything beside the road). Each has a factor on the rolling resistance coefficient (1, 2.5, 6), applied in the physics and sent to the trainer in SIM mode (Crr), so gravel and meadows are harder to ride. The bike's physics profile (§2.5) scales the part above asphalt: factor = 1 + (surface factor − 1) × off-road share, so a gravel bike (0.5) rolls on gravel at 1.75 × its asphalt Crr and a mountain bike (0.3) at 1.45 ×; asphalt stays 1 for every bike. The world package has no per-edge surface yet (only the road class: tracks and paths are gravel); the OSM `surface` tag (paved / unpaved / compacted / …) in the road graph comes later and feeds the same three surfaces.
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

**ERG and SIM during a workout (decided):** while a workout runs it owns the trainer mode. Steps with a target put the trainer into ERG with that target (ramps update continuously; the ERG controller ramps steps at 50 W/s, updates at most 1 Hz and eases off on low cadence); *free* steps return to the mode from before the workout (normally SIM, the road's gradient); when the workout ends or is stopped, that mode is restored and the workout no longer touches the trainer. Manual `trainer.mode` / `erg.target` commands during a workout are overridden by its next update. Without a controllable trainer the targets are shown as guidance only. The virtual trainer acts as an ideal ERG trainer: it reports the target while its set power is above 0.

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

**Start point and test riders (v1.9):** a session starts all riders together at a configured distance (`ServerConfig::start_m`); the desktop app picks a random point by default (on a route that ends, at least 1 km before the end; off for screenshot and capture runs, which must be reproducible). Riders start in a formation: rows of up to three across the lane, 1.2 m apart and always 0.3 m inside the lateral limit (fewer per row on narrow roads), rows 3 m apart behind the start point; the front row's centre is kept for the client rider (bots fill the other places in order, so the formation is deterministic). The formation never starts before the road begins: on a route it is moved forward until its last row is at 0 m, in free ride every rider's path starts at their place on the start edge (moved forward if the edge behind is too short). Server-side test riders (bots, 10 by default) ride at twelve different strengths (85–280 W, easy and strong interleaved) so a field spreads out, push harder uphill (at least the power for 10 km/h on the current gradient, so easy bots do not crawl) and steer with autodrive.

### 6.2 Network Protocol

- **Tokio** as the asynchronous runtime.
- **Quinn** for QUIC-based communication (TLS 1.3).
- **Protobuf** via `prost` for all structured messages.
  - Schemas live in `proto/` and are versioned (`ocw.v1`). Changes follow protobuf compatibility rules: add fields, never reuse field numbers.
  - Position datagrams use compact quantized fields: distance in cm (`uint32`), speed in mm/s (`uint32`), lateral offset in cm (`sint32`), animation state (`uint32` enum).
  - A protocol version is exchanged during the handshake; incompatible clients are rejected with an update prompt.
- **Unreliable datagrams** for position updates; **reliable streams** for login, session management, steering intents and important events.
- The protocol is transport-independent, so a WebTransport endpoint can be added later (§3.5).

**QUIC transport (decided, Phase 5 groundwork).** `ocw-server::net` carries the unchanged encoded `ClientMessage` / `ServerMessage` bytes; `GameServer` stays transport-independent and the desktop's `ClientCore` runs on the same channel interface as with the local server.

- **Port:** UDP **47800** by default (`ocw-server --listen [ADDR]`, `ocw-desktop --server HOST[:PORT]`); no TCP port. ALPN `ocw/1`.
- **Threads:** the game loop stays one synchronous thread at 50 Hz. `NetServer` owns a small Tokio runtime (two workers) with one task per connection; each tick the loop calls `pump` (connection events and received messages → `GameServer::connect` / `handle` / `disconnect`, replies to the sender) and `broadcast` (the tick's output to every joined client). The client's `NetClient` has its own one-worker runtime; the Bevy side only sends and drains channels.
- **Streams and datagrams:** one bidirectional stream per connection, opened by the client, with every reliable message as a length-prefixed frame (4-byte little-endian length, at most 1 MiB): Hello / Welcome / Reject, steering, turn and U-turn intents, rejoins, dev commands, path updates. **Snapshots** (server → client) and **sensor samples** (client → server) – both superseded by the next one and sequence-numbered – go as unreliable QUIC datagrams. A message larger than the connection's datagram limit (a big field's snapshot exceeds ~1.2 kB; §6.3 interest management will shrink it) or a peer that accepts no datagrams falls back to the stream. The client sends samples on the stream until the server has answered its Hello, so no sample overtakes it. Datagrams may be reordered: the client drops a snapshot whose `server_tick` is not newer than the last one. Considered and not chosen: everything on one stream (simple, but a lost packet stalls all later snapshots – head-of-line blocking); a new unidirectional stream per snapshot (no head-of-line blocking, but per-stream overhead at 10 Hz × riders and still retransmits stale data).
- **Flow control:** each connection has a queue of 256 messages; when it is full, snapshots are dropped and a client that cannot take reliable messages any more is disconnected ("too slow") instead of growing memory.
- **Liveness:** keep-alive every 4 s, idle timeout 15 s; a vanished client's rider is removed after it. The client extrapolates snapshots by half the measured round trip when reconciling its prediction. The 2-minute session hold for reconnects (§6.3) is not implemented yet: a reconnect is a new join.
- **Shutdown:** `ocw-server` stops on Ctrl+C / SIGTERM, closes every connection with "server shutting down" and waits up to 1 s for acknowledgements; the desktop shows "Disconnected: …" in the HUD and keeps running.
- **Course check:** the client rides its own course data; besides the world package check (§6.5) it refuses a Welcome whose route (or free-ride mode) differs from its own.

**TLS in development mode (decided; until the backend exists).** TLS 1.3 only, via rustls with the ***ring*** crypto provider (licence Apache-2.0 AND ISC; already in the tree through `ureq`; chosen over the default aws-lc-rs to avoid its CMake/C toolchain on all three platforms). The server uses a self-signed certificate from `rcgen`, new at every start or kept in `--tls-dir DIR` (`cert.der`, `key.der` with mode 0600), and prints its SHA-256 fingerprint. The client must say how to trust it – there is no default: `--server-cert SHA256` pins exactly that certificate (any address), `--insecure-dev` accepts any certificate but is refused for non-loopback addresses (checked when the options are parsed and again when the TLS configuration is built). Both verify the handshake signature, so the server must hold the key. Not yet: certificates from a public CA (WebPKI roots or the platform verifier), OIDC sign-in and per-session tokens (§6.4, §12); until then a reachable server accepts anyone who has its fingerprint. New dependency licences: quinn, quinn-proto, quinn-udp, rcgen MIT OR Apache-2.0; rustls Apache-2.0 OR ISC OR MIT; ring Apache-2.0 AND ISC. They are used only by `ocw-server` (and the desktop through it), never by the wasm crates (§3.5).

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
- With neighbouring packages (§2.2.2) junction ids are vertex indices of the joined graph, `WorldPackageId.name` is the joined name (`Home + Neighbour`) and `graph_hash` the joined graph's fingerprint, so client and server must have the same packages installed; a missing neighbour on one side shows as a graph mismatch. (v1.6: plus a sequence number and the exit's riding direction; no `exit_edge_id` = no choice, the default; `junction_id` is the vertex index in the world package's graph – client and server use the same package.)
- A manual U-turn is a reliable `UTurn { distance, speed }`: where the client turned, so the server builds the same loop when it is close to its own state.
- **Same package on both sides (decided):** because junctions are vertex indices, the client sends `Hello.world = WorldPackageId { name, graph_hash }` and the server sends its own in `Welcome.world`. `graph_hash` is the FNV-1a 64 fingerprint of the road graph's package encoding (the `GRPH` payload), so any rebuild that changes the graph changes it. The server rejects a Hello with another hash ("world package mismatch … install the same package version"); a client refuses a Welcome with another hash (for servers that do not check). Sessions without a road graph (route files, test routes) skip the check. Package signing and manifests (§4.7) are separate.
- **Rider weight (v2.7):** `Hello.rider_weight_kg` carries the rider profile's weight; the server clamps it to 30–200 kg and uses its curated physics profile otherwise (`Hello.profile` is deprecated, §3.2.1).
- **Bike and look (Phase 4):** `Hello.bike_profile` (field 6) names the curated physics profile by id (unknown → default), `Hello.look` (field 7, `RiderLook`) the cosmetic look; `RiderUpdate.bike_profile` (11) and `RiderUpdate.look` (12) pass both to the other clients (§2.5).
- Route rides: `Welcome.graph_route` names the route; leaving it is an ordinary `TurnIntent`, rejoining a reliable `RouteRejoin { distance, speed }` (§2.2.1).
- The server sends each rider's path as reliable `PathUpdate`s (the steps from an index on, or a new path after joining or a teleport); clients use them for the other riders and to correct their own prediction.
- The **server decides** at the lock point and broadcasts the chosen edge.
- The client predicts with its own choice and is corrected only in the rare case of disagreement (e.g. the intent arrived too late → the server default applies).
- In group rides with "follow leader", the server applies the leader's choice to all followers.

---

## 7. Voice Chat

Infrastructure: **LiveKit** (WebRTC SFU).

Features: voice rooms, participant management, audio transmission, dynamic subscription to audio tracks, group communication. Always on with automatic join and open microphone; microphone mute and speaker mute toggles with keyboard and gamepad hotkeys (§2.4); no push-to-talk.

- The client uses the official **LiveKit Rust SDK**. It wraps libwebrtc, but LiveKit provides prebuilt binaries for all target platforms.
- It is isolated in the `ocw-voice` crate behind a trait, with a cargo feature `voice`, so the app also builds and runs without it (CI, minimal builds).
- **Platform plan (decided 2026-11-12): LiveKit on every platform.** One self-hosted LiveKit server for all clients; the game server issues the tokens and decides the rooms, so rooms, tokens, spatial selection and mute logic are shared. Each platform plugs its own LiveKit client in behind the `ocw-voice` trait:

  | Client | LiveKit client | Notes |
  |---|---|---|
  | Desktop (Linux, Windows, macOS) | LiveKit Rust SDK (libwebrtc) | Building it on Linux needs clang ≥ 21 (`make deps-llvm-debian`, apt.llvm.org; Debian 13 ships 19) |
  | Browser (future web client, §3.5) | `livekit-client` (JS), called from the Rust/wasm client through bindings | Uses the browser's own WebRTC; no native build |
  | iOS / Android (future apps) | Native LiveKit Swift / Android SDK, or the Rust SDK if the app is Rust | Prebuilt WebRTC from the SDKs |

  The trait therefore must not assume threads, a file system or a native audio device in its API (the browser backend is asynchronous JS); the pure logic (mute state, hotkeys, auto-join, spatial attenuation, nearest-N) stays in platform-independent code that also builds for wasm.
  Options considered: (1) LiveKit everywhere – chosen; (2) another WebRTC SFU (mediasoup, Janus, Jitsi, own on Pion / webrtc-rs) – no gain, the browser and mobile still use WebRTC and we would rewrite LiveKit's signalling and room logic per client; (3) own Opus over QUIC / WebTransport without WebRTC – possible in browsers (WebTransport + WebCodecs) and on mobile (OS echo cancellation), but needs own jitter buffer, loss concealment, NAT traversal and scaling, and the desktop still needs a C/C++ echo canceller; much more work for likely worse quality. Uncertainty: Rust SDK support for iOS / Android not verified yet.
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
5. Validate assets (`cargo run -p ocw-assets -- validate [DIR]`, including licence metadata, §13): the manifest `manifest.toml` next to the models has an entry with `license`, `source` and `author` for every `.glb` / `.gltf` (files without an entry are rejected), licences from the allowed list (§2.5), no physics keys, bikes name a known physics profile; every model reads as glTF 2.0, uses only paint slots as materials, has the nodes the app animates (bike: wheels, crank, pedals, handlebar, grip and saddle anchors, `lod1`; rider: the rig of §2.6, head wear and kit nodes), plausible rig dimensions and stays within the triangle budget (bike 15 000, rider 8 000, far LOD 4 000). The validator is a subcommand of the asset crate instead of a separate `tools/asset-validator` (one crate shares the reader with the app; low).
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
│   ├── ocw-worldstore/   # package files, keys, trusted roots, cache, download (not wasm)
│   ├── ocw-render/       # Bevy plugins: terrain, roads, avatars, bikes, HUD, console
│   ├── ocw-assets/       # bike/rider glTF generator and reader, asset validator, pose, animation states (no Bevy)
│   ├── ocw-fit/          # FIT activity writer (no dependencies, wasm-safe)
│   └── ocw-voice/        # LiveKit integration behind a trait (feature "voice")
├── proto/                # .proto schemas
├── artwork/              # original artwork: logos/ (app icon)
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
| `renderer`, `avatar`, `bicycle`, `*-editor` | `ocw-render` and `apps/desktop`; models, pose and animation logic in `ocw-assets` (Bevy-free) |
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

**Build id:** every desktop build carries a build id: the workspace version, the short git commit, `-dirty` for uncommitted changes to tracked files and ` (debug)` for debug builds (e.g. `0.1.0-63910f2-dirty`). It is shown small in the HUD's lower right corner, by `ocw-desktop --version` and in the startup log. Remote builds get no `.git`, so `scripts/remote.sh` passes the commit as `OCW_BUILD_ID` (`scripts/build-id.sh`); otherwise the build script asks `git`.

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
- **Releases:** binaries and world packages are published as **GitHub Releases** (pre-releases while in development), never committed to git (decided 2026-11-12; the history was rewritten to remove the former `release/<version>-<commit>/<platform>/` binaries, `release/commit-map.txt` maps old to new commit ids). Each release carries the AppImage and a tarball of the stripped Linux binaries, a Windows zip, the macOS DMG, the world package with its manifest and data licence (ODbL, §13), the OSM extract it was built from (the ODbL "offer the data" duty) and `SHA256SUMS`. `release/README.md` keeps the build notes. Screenshots and videos made during development are not in the repository (removed from the history on 2026-11-12, `artwork/build-history/` dropped entirely): they are published as one zip per day (`ocw-build-history-YYYY-MM-DD.zip`) with an index (`index.md`) on the GitHub release `build-history`. New test runs land in the git-ignored `test-output/` (with its `index.md`); the ones worth keeping go into that day's zip.

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
- Signed world packages and updates (§4.7): Ed25519 root key kept offline, rotating signing keys with a validity period, SHA-256 of every file and tile chunk, verification on every load. Tampered or damaged packages are always refused; unsigned or untrusted ones only load from local paths with a warning (or with `--allow-unsigned`), never silently. Secret keys never enter the repository.

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
| Own 3D assets (`assets/models/`, generated) | Own work | Like the code: **MIT / Apache-2.0** (Phase 4, §2.5); each listed in `assets/models/manifest.toml` |
| Own sounds | Own work | Decided separately; suggestion **CC BY 4.0**, or proprietary if monetized. The game sounds so far are synthesised in code (`apps/desktop/src/audio/`), so they are source code (MIT / Apache-2.0) and there are no sound files to license |
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
- Route format and route editor; GPX import (§2.7). The **route editor is postponed** (decided 2026-11-12): routes come from GPX import and the package for now.
- Junction steering and dead-end handling (§2.2.1, §2.2.2).
- In-game menu, first increments (§3.2.1, M1–M4): pause menu, settings file and Settings pages (controls, trainer, Strava, display, profile), Ride picker, Navi (routes, rides, Strava sync, GPX import).

- **Unit tests:** every world-builder step on fixtures — OSM extraction, projection, road graph and stable edge IDs, graph cleanup and dead-end flags, DEM sampling and smoothing (gradient limits, bridges, tunnels), terrain conformance, splat-map classification, tile cutting, package manifest and signature verification; route format, GPX parsing and map matching; junction detection, exit classification, default-exit rules, key selection and lock point, U-turn.
- **Integration tests:** build a small region end-to-end and load it in the client headless; rebuild with changed OSM data and check route re-matching.

Done so far for land use and buildings (§4.6): OSM land-use classes rasterised to a 5 m splat map per tile, flat water surfaces with the terrain carved below, land-use colour maps, trees by land use, extruded buildings with flat and gable roofs (package sections `LUSE`, `BLDG`, §4.7). Unit tests: tag classification, ring assembly, polygon rasterisation with holes, priorities, tile edges, water levels, footprint heights and roofs, ear clipping with holes, placement, section round trips and old readers skipping the new sections.

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

**Status (2026-11-12):** bike and character models, animations and the asset pipeline are done (§2.5, §2.6, §8.1): `crates/ocw-assets` generates six bikes and the rider as glTF, validates them (`ocw-assets validate`), poses the rider on the bike and selects the riding state; `ocw-render::riders` draws and animates them with a far LOD. Tests: mesh building (outward faces), glTF write / read round trip and the +Z file convention, the validator rules (no licence entry, unknown licence, missing file, physics keys, missing nodes, unknown paint slots), committed assets = generator output, frames from their geometry (wheelbase, saddle height), feet on the pedals and hands on the bars for every bike seated and standing all round the crank, torso lean by bike, state selection with hysteresis and dwell, cadence, crank easing, wheel spin, pose blending; integration (`crates/ocw-render/tests/shipped_assets.rs`): all shipped assets through the validator and through Bevy's own glTF loader in a headless app; the riders plugin spawns every bike with the right head wear and kit and animates standing, wheel spin and freewheeling headless. Still open in Phase 4: components (wheels, tyres, saddle …) as cosmetic choices, the 3D preview on the Workshop page, the Character page (skin, hair, body), *Equal* race mode, the remaining animations (cornering, U-turn, finish, victory, foot down).

**Result:** Individually customizable riders.

### Phase 5 – Multiplayer

- Accounts.
- Sessions.
- Networked server: the existing local-server logic runs remotely. (Started: QUIC transport, dev-mode TLS, `ocw-server --listen`, `ocw-desktop --server`, §6.2.)
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
- Always-on voice with automatic join; microphone and speaker mute toggles with keyboard and gamepad hotkeys and a HUD indicator (§2.4).
- Voice rooms.
- Spatial audio.
- Voice settings in the menu (§3.2.1, M8).

- **Unit tests:** voice room and token management (backend), spatial audio attenuation and nearest-N selection, microphone / speaker mute state and hotkeys (keyboard and gamepad), automatic join / leave on shared and solo rides, remembered mute state, device selection logic; app builds and runs with the `voice` feature disabled.
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

---

## Appendix B – Autonomous Development, 2026-11-11/12: Additions and Assumptions

In the night from 11 to 12 November 2026 the open tasks and phases were implemented without
user interaction (instruction: "where unsure, take the best guess but document your
uncertainty, your options and your choice"). This appendix lists what was added, every
decision taken on the user's behalf, how sure it is, and what is still open. The details sit
in the sections named in each entry; this appendix is the place to review the choices and
overrule them.

**How the work was done.** Each item was built in its own branch, tested on the Linux build
machines (marge, leela until it became unreachable at about 01:00; the Mac for
platform-independent checks), reviewed, merged into `main`, checked again on the merged tree
(clippy with warnings as errors, all unit tests, the wasm build of the engine-independent
crates), pushed and deployed to marge's kiosk. All screenshots and videos are in the build
history `test-output/index.md`. Uncertainty is given as low / medium / high.

### B.1 Overview

| Area | Spec | Merge |
|---|---|---|
| Build slots per machine (2 Linux, 1 Windows, 1 macOS), queue in the build dashboard | §11, README | fc136b7 |
| Video: REC indicator, smooth live recording | §3.2 | f67085e |
| Bots: 10 by default, start formation, 12 strengths | §6.1 | a1df35e |
| Camera: solid vs see-through objects, trees fade, buildings are obstacles | §3.2 | 196b5c0 |
| Windows build machine: full workspace builds and tests | §11.1 | 827792a |
| In-game menu M1 (shell, input contexts, gamepad) | §3.2.1 | a07845c |
| Gradient feel: 100 m average, elevation profile, camera tilt | §3.2 | b9dcb21 |
| Structured workouts with ERG (Phase 2) | §2.8, §5.3 | 0aea363 |
| QUIC transport, networked server (Phase 5 groundwork) | §6.2 | 902a48a |
| Menu M2: settings, rider profiles, weight in the physics | §3.2.1, §2.5, §6.5 | 7a8004e |
| Phase 4 groundwork: physics profiles, bike and look, bike types | §2.5, §5.2.1, §6.5 | d44f9da |
| Neighbour world packages at runtime (Phase 3) | §4.7 | 26c0f96 |
| UI font with full Latin coverage | §3.2, §13 | 7973c4b |
| Local events: race, group ride, time trial against the own ghost | §2.3.1 | d8ad641 |
| Menu M3: in-app ride switch, Navi pages, GPX import, paging | §3.2.1 | b65bc83 |
| Fix: U-turn on one-way roads (no double turn, no roundabout circling) | §2.2.1, §2.2.2 | d6f00c3 |
| Fix: riders start on any grade; grade limit across junctions; corner heights | §4.3, §5.1, §5.2 | 97a5805 |
| Fix: tunnels (tube and tunnel camera; Stegertunnel at km 8.8) | §3.2, §4.3 | d0a9576 |
| Events and workouts: FIT laps, ghost without shadows, event confirmation, Workouts page, FTP from the profile | §2.3.1, §2.8 | 2dc1892 |

### B.2 Decisions and assumptions

Each entry: the question, the options, the choice (**bold**) and the uncertainty.

**Build infrastructure**

- *Build slots.* Per machine: `OCW_REMOTE_SLOTS`, else `slots` in `machines.toml`, else the OS
  default (**Linux 2, Windows 1, macOS 1**). A waiting run polls all slots once a second, so
  there is no strict first-come order. Linux and macOS waiters give up when their ssh session
  ends; a Windows waiter cannot detect that and runs later. Low.
- *Build jobs.* A Bevy link with MSVC needs about 5.3 GB; the 8 GB Windows machine therefore
  needs 2 jobs (`jobs` key in `machines.toml`, or `OCW_REMOTE_JOBS`, now honoured on `main`
  too). **The user's `machines.toml` was not edited**; add `jobs = 2` for the Windows machine.
- *Disk space.* Agent checkouts on the build machines grow to 15–45 GB each; they are deleted
  after each merge. Low.
- *Build history.* `scripts/collect-test-output.sh` merges every worktree's screenshots and
  videos into `test-output/` with index rows (on the user's request). Low.

**Rendering and camera (§3.2)**

- *Camera classes.* Options: ray casts against meshes, or height functions per object layer.
  **Height functions** (`CameraObstacles` layers per tile): cheap, matches the terrain path.
  Buildings count as a prism up to the ridge with 0.5 m clearance (errs high). Medium.
- *Camera spot inside a building.* Options: move onto the roof, or pull in towards the rider.
  **Onto the roof.** In narrow streets with an orbited camera this gives a view from the roof
  edge. Medium – the other option may look better.
- *Trees.* **Screen-door dither** (no blending, shadows stay) in a cone between camera and
  rider: 3 m near the lens, 0.8 → 2 m wide, ending 1 m before the rider. Tuned by eye on one
  run; grainy at 720p. Medium.
- *Gradient display.* Average = height difference over horizontal distance across the next
  **100 m**; profile from 150 m behind to **1.5 km** ahead on a fixed 15 m grid, at least 40 m
  vertical range so flat roads stay flat; colour bands < −2 / 3 / 6 / 9 / 12 % (blue, green,
  yellow, orange, red, wine). In free ride the look-ahead is a **display-only preview** through
  the junctions ahead (selected exit, then defaults). Low.
- *Camera tilt.* **0.7 × the slope angle**, averaged from 5 m behind to 20 m ahead, 1.2 s
  smoothing, capped at **8°**, chase view only. The effect is subtle; `camera.tilt 1` makes it
  stronger. Medium – not yet tried on a real ride.
- *Video.* A screenshot is requested every 1/25 s (up to 4 in flight) and placed at its
  request time; the game never waits for ffmpeg; ffmpeg `ultrafast` (larger files, size not
  measured). Full 25 fps on marge (release); about 12 new frames/s in debug builds on leela,
  limited by Bevy's screenshot path. Medium.
- *Font.* **Fira Mono 3.206** (SIL OFL 1.1), the full version of Bevy's ASCII-only default
  font, so the look and the HUD's text widths stay; embedded in the binary (+171 KiB); not
  subset. Low.

**Menu (§3.2.1)**

- *Input routing.* Options: a Bevy state `UiMode` (as planned), or a resource. **Resource
  `InputContext { Ride, Menu, Console }`** set once per frame, so the console fits in too. Low.
- *Steering while the menu is open.* Autodrive takes over at the current lateral position;
  wild mode is suspended and restored; the junction choice falls back to the default and is
  **not** restored; manual steering comes back upright. Medium.
- *Confirmations* focus Cancel; mouse hover moves the focus only when the mouse moves. Low.
- *Settings.* `settings.toml` plus a separate **`riders.toml`** (profiles are a list). Order:
  default < file < `autoexec.cfg` < command line / `set --temp`; a setting replaces an option's
  default only if it is actually in the file, so scripted runs stay reproducible; `set` saves at
  once. Units are **global** (`display.units`), not per rider. Medium.
- *Ride switch (M3).* Options: re-exec the app (the planned restart path) or rebuild the
  session in-app. **In-app**: a re-exec reconnects the BLE trainer, which is slow and can fail.
  The old ride is saved; video recording continues; the ride time restarts at 0; switching is
  refused when connected to a remote server; `--at` now counts time since the app started.
  Medium.
- *Package discovery* for the menu: cache, files next to the executable, `data/`,
  `data/pilot/`, the current package's folder – a heuristic. Medium.
- *GPX import* needs a loaded package with a road graph (existing HMM matcher) and is saved as
  a route file; GPX files are offered from `<data>/ocw/gpx/` and Downloads (no file dialog on a
  gamepad). Matching runs on the main thread. Medium; tested only on a synthetic network.
- *Long lists* page at four rows (Previous / Next); rows are cut at 48 characters. Low.

**Riders, physics and bikes (§2.5, §5, §6.5)**

- *What the server trusts.* Options: keep the client's full physics profile in the hello, or
  only an id and the weight. **Id and weight only** (`Hello.rider_weight_kg`,
  `Hello.bike_profile`); the server computes everything from its own table. The old profile
  field is deprecated. Medium – changes behaviour for clients that sent custom profiles.
- *Weight.* 30–200 kg; typed values outside are refused, values from files or the network are
  clamped, NaN becomes 75 kg. Trainer SIM mass = rider + **8 kg**; only the Wahoo protocol can
  send it (FTMS has no mass field). Low–medium.
- *When changes apply.* Rider, weight, bike and look apply **at the next start or ride
  switch**, not live: there is no server message to change a rider mid-session. Medium.
- *Physics profiles.* Six: **road (default), road-aero, road-climbing, tt, gravel, mtb**; the
  bike type follows from the profile, so cosmetics cannot contradict physics. Values are own
  tuning. On flat asphalt at 250 W all stay within **±3 %** of road (spec cap), on a 6 % climb
  within ±5 %; off-road differences are deliberately larger (MTB about +17 % on grass). High
  for the exact values – they need review.
- *Surface.* Packages have Asphalt / Gravel / Grass by road class; each profile scales the
  off-road part of rolling resistance. The OSM `surface` tag is later. Medium.
- *Looks* (three colours, kit pattern) are resent in every snapshot; a reliable roster message
  can replace that in Phase 5. Low.

**Simulation, bots and events (§2.3.1, §6.1)**

- *Bots.* **10** by default (desktop and server). Rows of up to three, 1.2 m apart, 3 m between
  rows, behind the start; the **front-row centre is kept for the own rider**; if there is not
  enough road behind the start the whole formation moves forward (also on loops). Twelve
  strengths 85–280 W. Low.
- *Where an event starts.* Options: where the rider is, or at the course's start line. **Start
  line**, so laps and ghosts stay comparable. Medium.
- *Free ride and events.* **Refused** – bots take random exits, there is no fixed course. Low.
- *Who may start events.* Only the in-process local server (`allow_local_events`); a remote
  server leaves it to organisers. Low.
- *Ghost.* **Client-side only** (nobody can draft it), per route and time-trial length, best
  effort only, plain text (about 30 KB per 20 min). Only time trials record ghosts. Low.
- *Group ride.* Bots hold the target power ±10 % and regroup halfway between their own average
  and the rider. No ranking. Medium.
- *DNF.* Left the session, or still riding max(2 min, 25 % of the winner's time) after the
  first finisher. Gaps of riders still riding = distance ÷ own speed. Medium.

**Workouts (§2.8, §5.3)**

- *Location.* Options: own crate, `ocw-core`, `ocw-sim`. **`ocw-sim::workout`** (wasm-safe, next
  to the ERG controller); `ocw-sim` gains `serde` and `toml`. Medium.
- *Format.* Own TOML (steady, ramp, intervals, free; cadence and text cues; bare numbers = %
  FTP; unknown keys are errors); `.erg` / `.mrc` import is lenient, distance-based courses are
  refused, an `FTP =` header is ignored. Low.
- *Workout clock.* Options: wall clock, or pedalling time. **Pedalling time**: starts at the
  first power, pauses after 3 s without power; the ride itself never pauses. Medium.
- *Trainer mode.* While a workout runs it owns the trainer: ERG on target steps, the previous
  mode (normally SIM) on free steps and at the end. Manual trainer commands are overridden.
  The server is not involved. Low.
- *FTP.* `--ftp` if given, else the FTP of the rider profile riding now, else 200 W. Low.
- *Workouts page.* Files from `<data>/ocw/workouts/`, then **bundled examples built into the
  binary** (so the kiosk lists them too). Choosing one starts it at once (no confirmation: it
  ends nothing). Low.
- *FIT laps.* A lap at workout start, every step change, skip, end and stop; for events at GO,
  each lap and the finish. Event lap changes can be off by one 10 Hz state update. Low.

**World data (§4.7)**

- *Finding neighbour packages.* Options: neighbour ids in the manifest, or a geographic
  lookup. **Geographic**: a new `REGN` section (box and polygon) and `bounds` in the signed
  manifest; candidates from the cache and the `--world` folder whose box touches within 0.01°,
  verified with the same trust policy, joined only if a road crosses. Only direct neighbours;
  at most 2 loaded at once (tiles within 3 km, dropped beyond 5 km). Low–medium.
- *Matching roads at the border.* The builder now cuts roads **at the exact border crossing**,
  so both sides meet; stable ids of boundary edges change on rebuild (route re-matching
  handles it). Low.
- *No step at the seam.* `world-builder package --bbox` builds 1 km of context roads so both
  parts conform the terrain to the same road profiles. **Splitting from one extract is the
  supported way**; packages built from separate extracts may still show a small step. Medium.
- *Frame between packages.* Exact affine map, valid because the projection is
  equirectangular; a move to UTM / transverse Mercator would need to revisit it.

**Networking (§6.2, Phase 5 groundwork)**

- *Channels.* Options: one stream for everything, a stream per message, or a stream plus
  datagrams. **One reliable stream** (4-byte length prefix) for control, **datagrams** for
  snapshots and sensor samples, falling back to the stream if a message is too large. A full
  100-rider snapshot exceeds a datagram until interest management (§6.3) shrinks it. Medium.
- *Port.* **UDP 47800** (unassigned in the registered range; not certain nothing else uses it).
- *TLS.* TLS 1.3 with a self-signed certificate; the client **pins its SHA-256 fingerprint**
  (`--server-cert`) or uses `--insecure-dev`, which is refused for anything but loopback. No
  accounts yet: anyone who reaches the port and knows the fingerprint can join. Crypto
  provider **ring** (Apache-2.0 AND ISC), avoiding aws-lc-rs's C toolchain. Low.
- *Course.* The client rides its own course data; a server with a different course is refused.
  The server's test route defaults to `loop`, the desktop's to `hills` (remote clients need
  `--route loop`). Low.
- *Limits.* 256 queued messages per client (snapshots dropped first, then "too slow"),
  keep-alive 4 s, idle timeout 15 s. Medium.

**Fixes found during the night (§2.2, §3.2, §4.3, §5)**

- *U-turn on a one-way road.* The way back ran against the one-way, which left no exit and
  triggered a second, automatic U-turn (double turn, endless circling in roundabouts). Options:
  refuse the U-turn, keep it pending until a two-way road, or turn anyway. **Turn on the first
  stretch of the known path that may be ridden back (at most 300 m ahead), else turn in place;
  a rider against a one-way without a legal exit rides on against it** until the first legal
  exit. Medium – the rider may be sent into the first legal side road.
- *Starting on steep grades.* Below 1 m/s the force P/v used a 1 m/s floor, so a rider at
  200 W stayed at 0 km/h on 33 % for ever. Options: roll back, stop, or crawl. **Positive power
  always moves forward** (never negative speed while pedalling; the brake still holds). The
  rider accelerates faster from standstill than before. Medium.
- *Gradient artefacts in packages.* Short connectors and bridges kept impossible slopes (up to
  203 % in the pilot); a new pass enforces the road-class limit across junctions, and heights
  through rounded corners now follow the distance along the road (blended over up to 20 m).
  Pilot maxima: edges 203 % → 25.8 %, corners 242 % → 34 %. Steep tracks that are really
  steeper than 25 % are now flattened to the class limit. Medium.
- *Tunnels.* Options: tunnel flags from the road graph (route files have none), hiding the
  terrain above the road, or only lowering the camera. **The renderer detects tunnels from the
  ground height along the rider's track**, draws a dark tube with lamps and switches to a close
  tunnel camera that blends in and out. No package change. Limits: a short, very close camera
  at the entry; riders are lit by the sun inside; mouse orbit is off inside; only the own
  rider's track is searched. Medium.

**Privacy.** Agents used the user's first name and login as example data; it was replaced by
neutral names ("Alex", `someuser`) before anything was pushed, apart from one dashboard test
that had already been pushed and was fixed.

### B.3 Open items found during the night

- Events: no "follow the leader" or scheduled events; "End the event" is not confirmed.
- Workouts: FIT workout import, changing FTP mid-ride.
- Networking: reconnect and the 2-minute session hold (§6.3), tokens / OIDC, certificates from
  a public CA, rate limiting, tests under latency, loss and load.
- Neighbour packages: neighbours of neighbours, packages that touch without a crossing road,
  terrain-only neighbours.
- Phase 4: 3D preview, components, character page, *Equal* race mode (glTF assets and animations: done, §14 Phase 4).
- Menu: on-screen keyboard (renaming is console-only), more control options, bots per ride,
  named start places, route rename / delete, FIT / GPX export rows, package downloads.
- Free ride: a very short start edge can still squeeze the start formation together.
- Tiles may still be empty for a few seconds after a long teleport (streaming lag).
- Windows: running the app on the machine's screen (needs the interactive scheduled task),
  installer, tests in CI.
- `appimagetool` is downloaded from the continuous release without a pinned checksum.
- Tunnels: a very close camera for about half a second at the entry; riders lit by the sun
  inside; mouse orbit has no effect inside.
- Corners between very short edges can still reach about 34 %; riders crawl through.
- The Vaduz screenshot after the kiosk change showed the rider partly inside a building next
  to the road (route-file road against the package's buildings) – to check.
- The camera tilt values and the bike physics values are first tunings and need riding
  feedback.
- The build machine leela became unreachable at about 01:00 ("No route to host") and needs a
  look; marge and the Mac took over.

### B.4 Platform builds at the end of the night

Final runs on commit `8613029` (all features of the night plus this appendix):

| Platform | Machine | Checks | Result |
|---|---|---|---|
| Linux (Debian, AMD GPU) | marge | fmt, clippy `-D warnings`, wasm build of the engine-independent crates, all tests, Wayland screenshot from the world package | 726 passed, 0 failed; clean |
| macOS 26 (Apple Silicon M1) | Mac build machine | clippy `-D warnings`, all tests, app bundle | 726 passed, 0 failed; `release/0.1.0-8613029/macos-arm64/` (DMG, ad-hoc signed) |
| Windows 11 (x86-64, MSVC) | Windows build machine | clippy `-D warnings`, all tests, release build of `ocw-desktop` and `ocw-server` | 726 passed, 0 failed; release `ocw-desktop.exe` 106 MB built (not packaged) |

leela (Fedora) was unreachable from about 01:00 and is not part of the final runs. The marge
kiosk runs `2dc1892` (the same code without this appendix).
