# Open Cycling World

**Cross-Platform Indoor Cycling Simulation**

Project Description and Technical Architecture

Version 3.13 – November 2026

| Version | Change |
|---|---|
| 3.13 | Phase 9 **S5 built** (§14 Phase 9, 9I-S5): the weather machine – menu *World → Weather* (mode fixed / simulated, real weather shown dimmed "coming" until S7, the 17 presets as a grid of five columns with procedural icons, a custom editor on two pages – cloud cover and type, precipitation and intensity, fog; wind, gusts, direction, temperature –, the transition instant / 1 / 5 / 15 min, a random weather from a seed; who sets it under the title, what the rider may not change dimmed; keyboard, mouse, gamepad and touch, value rows with clickable ‹ › arrows; ten-foot sizes), pure in `ocw_weather::machine` (editor ranges, steps and validation, random weather, the event's weather) and `ocw_weather::icon` (icons from any weather state, SDF raster); event weather set by the organiser (`event … --weather PRESET [FIELD=VALUE…] \| custom … \| random SEED \| auto SEED`, protocol `EventStart.weather` (8), `EventState.weather` (15), new message `EventWeather`), shown in the event info and locked for everyone until the event is over (the server answers access *nobody*); `SessionWeather.sim_seed` (14); console `weather` extended (a preset with fields, `random [SEED]`, `fixed`, `real` → S7), `weather.list`, `weather.status`, `weather.transition`, `weather.seed`; the browser app gets the console commands (its menu with B6); scenes `menu-world-weather`, `menu-weather-presets`, `menu-weather-custom`, `event-weather-start`, video `weather-clear-to-thunderstorm-video` |
| 3.12 | Phase 8 **T6 built** (§14 Phase 8, 8E; §2.5, §2.6, §4.6, §6.1): riders and bikes get real materials – one bindless material with its own shader for the near models (`ocw-render::gear`, `gear.wesl`): carbon twill under a clear coat (frames painted *bare carbon*, rims, cockpits, crank spiders), spun metal with anisotropic highlights, rubber with a file tread, bar tape, saddle leather, jersey knit with mesh side panels, lycra, padded gloves; painted and plastic parts standard materials with a clear coat or a gloss / matte finish; three CC0 textures (ambientCG jersey knit, carbon twill, saddle leather, group `gear` of the library), the rest procedural; detail fading by the pixel size, the far LOD plain. The jersey designer in the character corner (*Jersey designer*, *Kit colours*; `kit.design`): our own patterns (plain, chest band, sash, stripes, chevrons, gradient) in two or three colours, a race number (paper bib on the back, digits on the hips), the pattern on the helmet, free colours for shorts, socks, shoes and gloves, gloss / matte helmet and shoes – free, saved in `riders.toml`, sent like the character (`RiderLook` fields 12–21, once per change). 30 riders near: 3.17 → 3.26–3.54 ms per frame on marge |
| 3.11 | Phase 9 **S4 built** (§14 Phase 9, 9I-S4): weather simulation and rendering – new crate `ocw-weather` (pure, wasm: `WeatherState` with cloud cover / type, precipitation type and intensity, fog, wind and gusts, temperature, humidity, pressure, lightning; eased transitions; 17 presets with the extremes of Q9; the session weather owned by the `GameServer` like the clock, fixed or a seeded simulation of fronts and showers; road wetness, puddles and snow cover from the precipitation history; gusts on session time; lightning at its rate with thunder at 343 m/s; Koschmieder visibility and particle maths; the cloud light), protocol `Welcome.weather` (17), `Snapshot.weather` (7), `ClientMessage.weather` (17, `WeatherRequest`), host-only changes checked on the server (Q2, `ocw-server --weather-control`, `--weather PRESET`); renderer `ocw-render::weather`: flat overcast light without shadows and passing cloud shade, a noise cloud dome per cloud type, the weather's fog, GPU rain / snow / hail particles, wet roads with puddles, snow on ground, roofs and trees, trees swaying in the wind, spray, lightning flashes and channels; procedural rain, hail, gust and thunder sounds; console `weather` (user command), `weather.fx`; World → Weather; frame times on marge within budget; scenes `weather-*` |
| 3.10 | Phase 8 T7 – orthophoto corridor (§4.6, §4.7, §14 Phase 8; decided by the user 2026-11-14: **on by default, in every world package**): the open orthophotos cut along the package roads (±50 m beside the road edges, 0.5 m per pixel or the source's own, forests and building footprints left out, shadows and cars toned down) as JPEG chunks in the new optional section `ORTC`; the ground shader colours the materials with the photo near the roads (blurred photo up close, sharp from 15–120 m, gone by 950 m), setting `display.orthophoto` (default on); sizes and frame times measured; streamed packages (browser, B2) leave the section out |
| 3.09 | Phase 10 **B4 built** (§14 Phase 10, 10G; §3.4, §5.3): devices in the browser – `WebBluetoothBackend` runs the **same** `ocw-devices` runner, protocol state machine and GATT codecs (FTMS, Cycling Power, CSC, Heart Rate, Wahoo) as the desktop over a `GattLink` on the browser's objects (`ble::web`: a pure adapter behind the `WebGatt` boundary, own wasm-bindgen bindings); the runner and the reconnect loop (`runner::supervise`) are executor-free (a `Timer`: Tokio natively, `setTimeout` in the browser) and the desktop's BLE uses them too; `DeviceEvent::Connected`; `TrainerControl` and the workout session moved to `ocw-client` (both apps), SIM follows the build in force (`set_profile`: Crr, CdA, mass from the server's `RiderBuild` effect); the page's Devices panel (chooser in the click), remembered devices per origin and reconnect without the chooser (`getDevices`), virtual power and a clear message without Web Bluetooth; `make test-web-ble` (fake `navigator.bluetooth`), `-emulation` (Chromium's own stack, DevTools `BluetoothEmulation`), `-real` (marge's KICKR ridden from headless Chromium over BlueZ) |
| 3.08 | Phase 9 **S2 built** (§14 Phase 9, 9I-S2): the night sky without stars – moon (Meeus ELP series: 2026 full / new moons within 1.3 min, 0.005° at the 2026-08-12 eclipse; phase, bright limb, earthshine, rise / set) as a CPU-drawn disc and as the night key light (bluish, soft shadows, sun disc off), planets from Standish J2000 elements (Venus 0.004° vs Meeus 33.a) with magnitudes and twilight fading, a procedural Milky Way band in galactic coordinates, street lamps from OSM `lit` (new optional package section `LITR`; towns by land use) with glowing heads and a pool of 12 point lights, light pollution and sky glow from the buildings, bike lights only with the fitted `lights` part (Q3), time-machine presets next full / new moon, console `camera.look`; decisions Q7 (no stars for now) and Q10 (order S2 → S4 → S5 → S7 → S8 → S6) |
| 3.07 | Phase 4 **W4 built** (§14 Phase 4, 4C, 4M; §2.6): the character corner – the mirror is the character editor (over the shoulder and a planar reflection: a second camera at the mirrored eye, half resolution, a sub view of the glass; `workshop.mirror off`: a direct front view), the rider's five hotspots, LB / RB through 13 slots and *Try on the bike* (the rider on the stand bike pedalling at 45 rpm); free character (`ocw_core::Character`: height from the profile, build slim / medium / strong, 12 skin tones, 4 faces, 6 hair styles, hair colour, 3 glasses; `character` command) and the kit as catalogue items (16 apparel items, 3–4 researched EUR list prices each, a free base kit; worn, bought, sold like parts); kit physics (finer aero steps 0.0008 m², a tiny efficiency step, helmet and shoe weights after the mass floor, a skinsuit covers the shorts; the same caps as the bike, now at 150 / 250 / 400 W; Open only); benefit lines by speed for parts and kit (`ocw_sim::build::Benefit`); the generated rider's optional nodes (kit variants, hair, faces, glasses), body height scaling with the saddle moved along the seat tube (`ocw_assets::pose::BodyFit`), build girth; protocol: `RiderLook` character fields 5–11 (not in snapshots), `RiderBuild.look` (9); the browser's mirror locked |
| 3.06 | Decision from the build dashboard (2026-11-14): Phase 13 Q3 – no WASD in Unicorn mode; cursor keys and + / − on the keyboard, or the gamepad |
| 3.05 | Aero effects depend on speed (§14 Phase 4, 4G; decided by the user 2026-11-14): aero parts and kit change only CdA, the physics makes them speed-dependent (watts saved ∝ v³: ~1 W at 20 km/h, ~4 W at 30, ~8 W at 40 for 0.010 m²; worthless below ~20 km/h, noticeable from 25–30, significant from 35–40 km/h; little on climbs; less in the draft); benefit scores shown by speed; caps checked at 150 / 250 / 400 W |
| 3.04 | Kit gives a slight benefit (§2.6, §14 Phase 4 4C; decided by the user 2026-11-14, replacing "kit is purely cosmetic"): apparel items have effect classes (aero, weight, a little efficiency) and a benefit score = the speed gain over the slot's standard item at the reference rider (flat and 6 %), shown and compared on the cards; kit counts against the same build caps (proposal) and only in Open mode |
| 3.03 | Phase 4 **W3 built** (§14 Phase 4, 4G, 4M): build physics in `ocw_sim::build` (pure, wasm; server, prediction and the workshop's cards share it) – the server's copy of `assets/parts.toml` (`PartTable`), bike mass = profile + Σ (part − base part) with the 6.8 kg / 9.0 kg floor, class steps (rolling ±4 % Crr, aero ∓0.004 m² CdA, efficiency ±0.3 %), the build capped to ±1.5 % on the flat and ±2.5 % on a 6 % climb at 250 W / 75 kg (the change scaled back towards the profile until it fits: the aero road and TT best builds sit on the cap); race modes *Open* / ***Profile*** (default for races and group rides) / *Equal* (`Event.physics`, `event … --physics`), free ride, single player and time trials *Open*; `BikeChange` applies the build (in an event: when it is over); protocol: `Hello.parts` (8), `EventStart.physics` (7), `EventState.physics` (14), `RiderUpdate.build_hash` (19), `ServerMessage.rider_build` (10, `RiderBuild`: parts and the server's effect, reliable, once per change and to joiners), `DevCommand.bot_bike` (4, `dev.bot_bike`); the others' parts on their avatars; the picker's speed lines and the compare card's difference |
| 3.02 | Phase 10 **B3 built** (§14 Phase 10, 10G; §6.2): multiplayer in the browser – a WebTransport endpoint (`ocw-server --webtransport`, UDP 47801, `web-transport-quinn` on our quinn endpoint) next to native QUIC in the same sessions; 13-day ECDSA certificates by hash (`serverCertificateHashes`), kept and rotated; the wasm-safe `ocw_server::wire` (frames, datagram rules, the client's `ClientLink` state machine) and `timing` (snapshot interval, jitter, age); the browser's `RemoteLink` with WebTransport bindings, `?server=…&cert-hash=…`, the anonymous rider look (Q5); `ocw-bot --server https://…`; `make test-web-multiplayer` (browser, QUIC and WebTransport bots, desktop: all see each other; browser snapshot jitter 16 ms vs 3 ms native) |
| 3.01 | Build dashboard defined as a whole (§11.2.1): service, machines, jobs with their purpose, machine toggles and slots, disk janitor, answerable open questions, build log, security, tests |
| 3.00 | Phase 4 **W1 + W2 built** (§14 Phase 4, 4M): the workshop scene – generated room (`ocw_assets::workshop`, own work, no brands), repair stand, wall hooks with name cards, the six starter bikes, camera states (overview, wall, stand orbit / zoom, part close-up, bench, mirror) as a pure eased state machine, ← / → bike cycling and the 1.2 s swap, zones ↑ / ↓, keyboard / mouse / gamepad / touch (gesture recogniser), the ride window (the world camera at a quarter size, 10 fps); W2: `assets/parts.toml` (76 parts: the 4I entries + base parts) and `parts-sources.toml` (validator: ceil(median), brand deny-list, 18-month age), `hs_<slot>` anchors and 30 part variants in the bike generator, hotspots with screen-space navigation, the part picker (equipped / owned / shop, locks, detail and compare cards, live preview), buy / equip / unequip / sell / paint, the local wallet (`workshop.toml`, append-only `hearts-ledger.jsonl`), hearts credited by the server (1 ♥ / 10 kJ + 1 ♥ / 20 m; none for virtual power, god mode, down, bots, captures, `--no-hearts`); protocol: `BikeChange` (client 16), `SensorSample.virtual_power` (7), `RiderUpdate.hearts` (18); part effects cosmetic until W3 |
| 2.99 | Phase 10 B2 built (§14 Phase 10, 10G): worlds over HTTP in the browser app – `ocw_worldstore::stream` (pure, wasm-safe: manifest check shared with `--fetch-world`, range planning, per-chunk SHA-256 verification with one refetch, the chunk cache behind a `ChunkStore` trait with LRU eviction by size, tile residency within a memory budget, the async driver), `ocw_world::package::StreamedPackage`, `WorldSet::update_tiles`; OPFS cache and `fetch()` with ranges in `apps/web`; loading screen; `?world=…&route=…`; `world-builder publish` writes `<id>/latest`; `make web-worlds` / `web-serve` with ranges / `test-web-world` (§4.5, §4.7) |
| 2.98 | Phase 12 K1 + K2 built (§14 Phase 12, 12I): rider collisions in `ocw-sim::collide` (footprints in the path frame or world space, sort-by-`s` / 4 m spatial hash, soft bumps with separation, inelastic momentum exchange and a wobble, crash thresholds by contact type, a deterministic faller, fallen bikes as obstacles), the crash sequence on the server (Falling → Down 2–8 s → Remount → Riding), autodrive avoidance for riders and bots in `ocw-sim::avoid` (pass on the region's side, hold at a 0.5 m wheel gap), `ride.collisions` / `ride.chaotic_bots` / `ride.proximity_cue`, protocol fields (`RulesRequest`, `CollisionMode` in `Welcome` / `Snapshot` / `EventState`, `EventStart.collisions`, `RiderUpdate.crash` / `.bump`, `DevCommand.stage`), `dev.bump` / `dev.crash`, procedural bump / crash / remount sounds, SIM flat while down and a bump pulse, ERG keeps running (Q4 still open); **ragdoll falls with Avian 0.8** (chosen by the K2 spike over Rapier: licence, ECS-native, wasm), client-side only; 100 riders: 0.078 ms per tick for the collisions in a full crush (release) |
| 2.97 | Phase 10 B1 built (§14 Phase 10, 10G; §3.5, §10): `apps/web` (`ocw-web`, wasm32 + WebGPU) runs the shared renderer and client with the `GameServer` as a frame-driven local server (`local::FrameServer`) and the frame-driven virtual trainer; the client logic moved into the shared crate `ocw-client` (desktop and web: one client, full parity); `ocw-devices` and the `ocw-server` lib build for wasm32; `make web` / `web-serve` / `test-web` (headless Chromium on WebGPU), CI job `web`; .wasm 63.8 MiB raw, 14.3 MiB gzip, 9.1 MiB brotli |
| 2.96 | Voice echo test (§7 *Echo bot*, §14 Phase 6; asked by the user 2026-11-14): an Echo rider (`EchoRequest`, `RiderUpdate.echo`, `Welcome.echo_bot`) stands 20 m ahead of the asker; near it (full to 30 m, fading to 60 m) the own microphone is played back from its place after 300 ms (20 ms – 3 s), through the voice processing, the spatial placement, the voice volume and the mute switches; local loopback on solo rides, a LiveKit echo participant for the real path; console `voice.echo`, `--echo-bot`, Settings → Audio → Voice (new sub-page: voice volume, Echo test, Echo delay; settings `audio.echo_bot`, `audio.echo_delay_ms`), HUD hint with level meters |
| 2.95 | Build infrastructure (§11.2): Windows tree sync fixed – a new checkout got no sources (the manifest diff's awk `NR == FNR` took the file hashes for the valid list when the machine's list was empty, so only the manifest was sent and an empty one written); now the whole tree goes to a new checkout, the manifest names its checkout, is only written after `tar.exe` succeeded, and files missing after a sync (e.g. `Cargo.toml`, `rust-toolchain.toml`) trigger one full send |
| 2.94 | Decisions from the build dashboard (2026-11-14): Phase 13 Q6 – the UI name is "Unicorn mode" (console commands stay `god`); Q4 – the trainer simulates a flat road while flying |
| 2.93 | Decision from the build dashboard (2026-11-14): Phase 12 Q7 – gates and barriers on roads hidden by default, a setting shows them (never solid on the routed road) |
| 2.92 | Decision from the build dashboard (2026-11-14): Phase 12 Q6 – damage and injuries carry over between rides; bike repairs are paid in hearts in the workshop, injuries heal over time |
| 2.91 | Phase 14 planned: standalone VR (§14 Phase 14, answer to Phase 11 Q5): a native Android OpenXR app for Meta Quest 2 / 3 / 3S / Pro and Pico 4 / 4 Ultra (Khronos loader, one APK, `apps/android` with `cargo-ndk`, the Phase 11 X1 integration), WebXR in the browser app (WebGL2 + multiview until the WebXR/WebGPU binding ships); platform facts as of 2026-11-14 (direct BLE to FTMS trainers works on Quest – store permission review –, no Web Bluetooth in the Quest Browser, so WebXR rides through the sensor bridge); standalone quality tier for XR2 (72 Hz Quest 2, 90 Hz Quest 3), mobile LOD set, world download on the device, QUIC / WebTransport, LiveKit, Android BLE backend, passthrough modes, sideload → Meta Horizon Store → Pico Store; milestones V1–V9, open questions Q1–Q5 |
| 2.90 | Decision from the build dashboard (2026-11-14): Phase 12 Q5 – falls go straight to a ragdoll with a physics engine (Avian or Rapier, chosen in K2), client-side visuals only |
| 2.89 | Decision from the build dashboard (2026-11-14): Phase 12 Q3 – "chaotic" bots that sometimes cause crashes, as a setting (off by default) |
| 2.88 | Decision from the build dashboard (2026-11-14): Phase 12 Q2 – group rides soft bumps only by default, races the full collision model |
| 2.87 | Research (2026-11-14) on Phase 9 Q7, star catalogue licences: Hipparcos, Tycho, Tycho-2 and Gaia DR3 are CC BY-NC 3.0 IGO (ESA), HYG / AT-HYG and Stellarium sky cultures CC BY-SA (or ND / GPL), Celestia's star data CC BY-NC 3.0 IGO, BSC5 without a licence; proposal: SKY2000 v5 (NASA GSFC) for stars, IAU WGSN names, d3-celestial lines (BSD-3); question stays open (§14 Phase 9, 9H) |
| 2.86 | Decision from the build dashboard (2026-11-14): Phase 12 Q1 – collisions on and damage off by default |
| 2.85 | Decision from the build dashboard (2026-11-14): Phase 11 Q6 – VR comes at the end, after the other phases |
| 2.84 | Decision from the build dashboard (2026-11-14): Phase 11 Q5 – a native standalone VR version (Quest / Pico) and WebXR are wanted; planned as a new phase (Phase 14) |
| 2.83 | Decision from the build dashboard (2026-11-14): Phase 11 Q4 – no head-lean steering in VR; steering stays on keys / gamepad / autodrive |
| 2.82 | God mode as built to 2.67 (§14 Phase 13, 13I): the plane model (`ocw_sim::god::FlightModel::Plane`, the hover model kept as `controls.god_flight = hover`), the coordinated turn `40 m/s² × tan(bank) / v` (bank ≤ 55°, ≤ 1.2 rad/s), the throttle 15 … 100 % of the speed law's cap (≥ 3 m/s), the nose ≤ 20°, the look-around easing back after 1.5 s (L or the middle mouse button at once), the camera rolling with half the bank; + / − by the typed character, learnt per key; touch: the left half steers, the right half looks; no saddle cloth; the bird-like wingbeat (shoulder, wrist and three groups of primaries per wing; 1.2 … 2.1 Hz; flap-gliding: 3 … 5 beats, then 0.6 … 1.4 s gliding; a 40 % downstroke; the hand 10 % of a beat behind; 0.9 s of braking flaps when landing); `camera.orbit` also orbits the flight camera (videos) |
| 2.81 | Decision from the build dashboard (2026-11-14): Phase 11 Q3 – the X1 spike builds both OpenXR integrations (port of bevy_mod_openxr and our own thin layer) and compares them on the Rift CV1 and Monado / SteamVR |
| 2.80 | Decision from the build dashboard (2026-11-14): Phase 11 Q2 – VR on Linux is a target (SteamVR / Monado / WiVRn; Index, Quest via WiVRn) |
| 2.79 | Decision from the build dashboard (2026-11-14): Phase 10 Q6 – the browser app has full parity with the desktop app |
| 2.78 | Decision from the build dashboard (2026-11-14): Phase 10 Q5 – anonymous riding in the browser is allowed with a plain default bike and character; customization (workshop, character editor, hearts) only for logged-in users |
| 2.77 | Decision from the build dashboard (2026-11-14): Phase 10 Q4 – the browser app and its world packages are hosted on our own servers for now |
| 2.76 | Decision from the build dashboard (2026-11-14): Phase 10 Q3 – the browser app starts now, in parallel with Phases 8 / 9 (B1 first) |
| 2.75 | Decision from the build dashboard (2026-11-14): Phase 10 Q2 – the sensor bridge (10D.2) will be built |
| 2.74 | Decision from the build dashboard (2026-11-14): Phase 10 Q1 – Chromium (desktop + Android) for trainer rides in the browser, Safari / iOS / Firefox with virtual power and the bridge |
| 2.73 | Decision from the build dashboard (2026-11-14): Phase 9 Q8 – historic weather is kept: ERA5 is CC BY 4.0 (most datasets) or under the Copernicus licence, both allowed by rule 8A; sources CDS, Open-Meteo historical API, MeteoSwiss / DWD station history |
| 2.72 | Decision from the build dashboard (2026-11-14): Phase 9 Q6 – weather data from MeteoSwiss + DWD with MET Norway as the global fallback, plus Open-Meteo; the provider is selectable in the menu (Automatic or a fixed provider) |
| 2.71 | Decision from the build dashboard (2026-11-14): Phase 9 Q5 – the wind load goes to trainers that support it (FTMS / Wahoo wind) |
| 2.70 | Decision from the build dashboard (2026-11-14): Phase 9 Q3 – night has a minimum brightness; bike lights are a part bought with hearts and fitted in the workshop (slot `lights`), not automatic |
| 2.69 | Decisions from the build dashboard (2026-11-14): Phase 9 Q2 – in shared sessions only the host / organiser controls the time and weather machines (no personal override); Phase 8 Q8 – 3D building data postponed |
| 2.68 | Phase 4 decisions (§14 Phase 4, 4L; answered by the user in the build dashboard, 2026-11-13): bike changes apply when leaving the workshop (`BikeChange`); parts have bounded effects with the *Profile* mode as the default for races and group rides; hearts: 1 ♥ per 10 kJ + 1 ♥ per 20 m climbing, no daily cap, 500 ♥ start, no back-credit of saved rides; kit (helmet, shoes, clothing) costs hearts, body, skin, hair and glasses are free; touch-screen testing postponed |
| 2.67 | God mode refined (§14 Phase 13, 13B / 13C; decided by the user 2026-11-13/14): plane-like flight – always forward at a cruise speed, ← / → bank and turn (coordinated turn), ↑ / ↓ throttle, + up / − down with the keys matched by the character they type (dead main-keyboard + / − on non-US layouts fixed), the mouse looks around without changing course and recentres; no saddle cloth on the unicorn (the rider's colour only in the mane band and the name tag); bird-like wingbeat – continuous flapping with glides, shoulder and wrist joints, asymmetric strokes, fanned primaries, body bob, own phase per rider |
| 2.66 | Build infrastructure (§11.2): build machines are enabled/disabled for building and their build slots (1–8) set in the build dashboard (toggle, stepper and a note per machine card, with the token of `config/dashboard-token`), stored in the git-ignored `config/build-machines.toml` (`machines.toml` stays untouched; every change in `build-log/DAY/machines.jsonl`); `remote.sh` refuses `run`/`sync` on a disabled machine (exit 3; ssh, fetch, put and deploys stay allowed) and takes the slot count from there (`OCW_REMOTE_SLOTS` > dashboard > `machines.toml` > OS default); initial state: leela disabled (kiosk only), Win11Builder enabled for Windows-related builds (1 slot) |
| 2.63 | Build infrastructure (§11.2): the build dashboard lists the open questions for the user parsed from this spec (by phase, proposals, decided ones hidden, milestones they block), answerable on the page with the token of `config/dashboard-token` (answers in `test-output/answers/answers.jsonl` and `inbox.log` for the coordinator, `GET /api/answers`), and watches the disks of the build machines; its janitor frees space by thresholds (75 %: `target/` of merged or gone agent checkouts idle 2 h and old replaced build units of the main checkout; 85 %: least recently used agent checkouts idle 24 h; 92 %: alert), one machine per 10 min, `--janitor off\|dry-run\|on`, every deletion in `build-log/DAY/janitor.jsonl`; `remote-gc.sh --branch` (cleanup on merge); `remote.sh` snapshots the tree at invocation and syncs it when the build slot is free, compares files by content and gives changed ones the time of the copy (stale builds from preserved older mtimes), removes `target/debug` once when the dev profile changes; `scripts/windows-move-toolchain.ps1` moves cargo, rustup and the sccache cache from C: to the build drive |
| 2.61 | Phase 13 G3 done, G1 / G2 / G4 gaps closed (§14 Phase 13, 13I): inspection – the flight panel (I: altitude ASL and above ground, speed, speed factor, heading, position in package metres, latitude / longitude, tile, local time of the place), a crosshair casting a ray from the camera (`ocw_world::inspect`: terrain, water and buildings, steps ≤ 20 m, bisection) that names the road (name, OSM way, class, `RSRF` surface, width, bridge / tunnel, gradient in the looking direction), the building (`BTAG` use, levels, roof shape, height), water, the `LUSE` area and the elevation; pins (P, `god.pin`; `god.pins` with coordinates and `--start X,Z`), magenta beams; look-ahead streaming (a ring of 2 … 3 tiles growing with the height, 3 × 3 tiles up to 3 s / 3 km ahead, neighbour packages within 3 km + 10 s of flight); trees and houses only below 1,500 m above ground; tree crowns in the unicorn's clearance (flight-only obstacles, the camera still sees through trees); unicorn LOD (≈ 2.9k triangles near, ≈ 670 from 80 m, an impostor card beyond 500 m); `ride.riders` and "riders: N (K flying)"; muffled hooves below 12 m/s; touch flight (one finger looks, two fingers move and pinch the speed, + / − buttons); FIT `user_marker` events at both ends of a flight |
| 2.60 | Phase 8 T5 – roads (§4.4, §4.6, §4.7, §4.8.3, §14 Phase 8): lanes, markings, cycle lanes, kerbs, sidewalks and crossings from OSM and the road class in the new package section `RDRS` (per strip: arc length, lanes, centre line, edge and lane lines, cycle lanes, sidewalk widths, kerb height, junction zones, crossings / stop / give-way lines; only added, older readers skip it); markings drawn in `road.wesl` as pixel-filtered coverage (crisp near, mean grey far, no shimmer) in regional styles (`assets/road-styles.toml`: German, Swiss / Liechtenstein with yellow zebras and cycle lanes, Spanish); raised kerbs (12 cm) and paved sidewalks beside the riding surface, tapering at junctions; wear (tyre tracks, patches, cracks) and grass between the ruts of tracks; two CC0 road layers (sidewalk pavers, kerb concrete); the road edge slopes into the terrain as a gravel verge instead of a vertical slab edge (Las Cañadas), walls only where the ground falls away more than 1.5 m |
| 2.59 | Build and world-build profiling (§11.2, §4.7): `scripts/build-profile.sh` / `.ps1` (clean, no-op, touched and edited builds with `--timings`), dev profile with line tables for the workspace and no debug info for dependencies (clean builds −23 to −28 %, `target/` 16–24 GB → 4–5 GB, edits up to −70 %), sccache on marge, the Mac and Windows (pinned, `make deps-sccache` / `deps-sccache-windows`), build slots in order of arrival, cargo times / crates / slot waits in the build log and the art journal, `make remote-gc`; copy-on-write seeding and a per-feature build directory implemented but off (measured without gain); world builder stage profile (`--profile`, `scripts/world-profile.sh`) and parallel / incremental passes with byte-identical packages (Black Forest north 164 s → 57–61 s, Tenerife 81 s → 36 s, Liechtenstein 5.7 s → 3.1 s) |
| 2.58 | Phase 13 G1 and G2 done, G4 in part (§14 Phase 13, 13I): god mode on G, `god [on\|off\|toggle\|back\|status]`, *World → God mode* (the Ride page is full: ten-foot rule), gamepad Select + North (Select + West flies back); the pure flight model `ocw_sim::god` (speed law, Shift ×4, Ctrl ×0.25, wheel ×0.25 … ×8 with the top speed capped at 1,600 m/s, climb at half the speed, smoothing 0.15 / 0.25 s, ≥ 2 m over terrain, water and buildings along each step and the next, ceiling 8,000 m, soft stop 200 m outside the world set); flight camera 8 m behind (C: first person, +10° field of view at 400 m/s, never under the ground), pointer capture (Esc frees it first), cursor keys / + − / mouse / gamepad, riding keys off; leaving glides (≤ 2 s) onto the nearest road (free ride) or route point (routes) marked by a glowing ring, Shift + G flies back to the take-off spot; the procedural unicorn `ocw_render::unicorn` (about 3k flat-shaded triangles, spiral golden horn, rainbow mane and tail, feathered pegasus wings beating, spreading, banking and folding, air gallop, the rider's colour on the saddle cloth and the first mane band), 0.6 s sparkle transformation, rainbow trail (Settings → Display → Effects), name tags over others' unicorns, chime, wing beats and flight wind; server: `GodRequest` / `GodResult`, `GodPose` at 10 Hz with plausibility checks, `RiderUpdate.mode` / `.god`, `Welcome.god_access`, `ServerConfig.god_access` (`ocw-server --god-control`), riders out of the physics and drafting, road position frozen, the landing placed by the server, refused from an event's countdown to the rider's finish and ended by an event's start formation; no distance credited while flying; trainer SIM flat (Q4 proposal) |
| 2.57 | Phase 8 T4 – houses (§4.6, §4.7, §14 Phase 8): roof shapes from OSM (`roof:shape` gabled, hipped, half-hipped, pyramidal – also on convex towers –, skillion, mansard, gambrel; `roof:height` / `roof:angle` / `roof:levels`, `roof:direction` / `roof:orientation`), `building:levels`, `min_height`, wall and roof material and colour and the finer building use in the new optional package section **`BTAG`** (old readers draw the `BLDG` flat / gable roofs); **regional house styles** `assets/building-styles.toml` (central, Alpine above 750 m, Black Forest uplands, Canary Islands) chosen like the material sets by time zone, box and elevation; procedural façades (windows per bay and storey, door, shutters, shop fronts, timber upper floors, lit windows at night) in the new building material with five more CC0 textures (slates, metal sheet, brick, stone, concrete); three levels of detail (near chunks with eaves, slabs, parapets, chimneys; merged mid and far per tile, coarser levels inset); `scripts/measure-fps.sh` |
| 2.56 | Tile cover and far terrain (§4.5, §4.7, bug 2026-11-13 "the Teide's summit is missing"): islands are built with `--land-tiles` (`TileCover::NearRoadsAndLand`: the tiles near roads plus every tile with land, still no open sea), so mountains far from roads are in the package (Tenerife: 2,690 → 2,744 tiles, 238.5 → 244.3 MB); `world-builder check --peak`; every package tile is drawn beyond the streamed tiles as a 100 m far mesh, hidden under loaded tiles; with a package the camera sees 120 km, the fog ends at the end of the terrain (60 km on islands, with a sea plane to the horizon) |
| 2.55 | Phase 8 T3 – region calibration (§4.6, §4.7, §13, §14 Phase 8 / 9C): imagery per region by `scripts/fetch-imagery.sh` (only the 1 km cells a built package covers; Orthophoto Liechtenstein 2025 + swisstopo SWISSIMAGE at 0.5 m, LGL DOP20 at 1 m, IGN PNOA at 1 m over the official WMS; own Sentinel-2 L2A seasonal median composites at 20 m, `scripts/imagery.py`), `world-builder calibrate`: regional palette per land class and road surface (`PALT`, with the sources and notices), roof colour per building (`ROOF`: orthophoto inside the footprint eroded by 1 m, OSM `roof:colour` wins), seasonal tint maps per tile (`SEAS`: spring / summer / autumn / winter, YCoCg + deflate); imagery notices in the manifest credits; client: near materials tinted towards the palette and the season, far colour map with the date-blended satellite colour, measured roof colours; package cost Liechtenstein +4.9 MB (30.2 → 35.0 MB, +16 %), Black Forest south +26.1 MB (258.5 → 284.5 MB, +10 %), Tenerife +23.1 MB (238.5 → 261.6 MB, +9.7 %) |
| 2.54 | Input model for the whole UI (§3.2.2, decided 2026-11-13: every screen and menu controllable by gamepad, mouse and touch where the device has one, keyboard as before): abstract actions per device, last-used device with matching glyphs in a hint bar, focus frame vs. pointer, touch mode (hit targets ≥ 9 mm / 48 px, gesture recogniser for tap / long press / swipe / pinch / two-finger pan, Windows emulated-mouse filter), orbit / zoom / pan in 3D scenes per device, a touch overlay in a ride (menu, autodrive, U-turn, lean buttons, tappable junction exits and HUD elements); platform facts of Bevy 0.20 / winit 0.30 (no touchscreens on macOS, pinch gestures only from the macOS trackpad); M10 and per-device reachability tests on every page (§3.2.1, §14 Phase 4, 10, 11, 13) |
| 2.53 | Phase 4 refined (§14 Phase 4, 4A–4L; §2.5, §2.6, §6.4, §8.2, §9): the Workshop opens a 3D **workshop scene** (decided 2026-11-13: selected bike on a repair stand in front, owned bikes on the wall chosen with left / right, parts selected on the bike, hearts, 1 heart = 1 EUR, prices rounded up), part hotspots and picker with live preview, compare, buy / equip / sell, the character editor as the changing corner; **hearts** earned by riding only (proposal: 1 ♥ per 10 kJ, climbing, workouts, events; no real money – no commercial release), server-side ledger and inventory; generic unbranded parts priced from researched real RRPs (median per tier, ceil) in `assets/parts.toml` / `parts-sources.toml`, refreshed by hand; parts within capped bands of the frame's physics profile, new race mode *Profile*; Phase 12 repairs in hearts only with persistent damage; milestones W1–W6, open questions Q1–Q5 |
| 2.52 | Phase 13 decisions (§14 Phase 13): other riders see the unicorn (Q1); leaving god mode lands the rider on the nearest road / route point below the unicorn, Shift + G flies back (Q2); the unicorn has large feathered wings like a pegasus (Q5) |
| 2.51 | Phase 13 planned: god mode (§14 Phase 13): hotkey G (console `god`), the rider turns into a flying unicorn; flight controls – mouse look, cursor keys move in the horizontal plane, + / − altitude, speed grows with height (20–400 m/s, Shift boost, Ctrl precise, wheel scale), terrain clearance; return to the road on leaving (Shift + G lands in free ride); inspection HUD and crosshair (road name, OSM id, surface, coordinates), pins; server state `God` (out of physics, standings and recording), client-driven `GodPose`, host access rules, not in running events; milestones G1–G5, open questions Q1–Q6 |
| 2.50 | Phase 12 planned: collisions, crashes and damage (§14 Phase 12): global switches `ride.collisions` and `ride.damage` (on / off; host or organiser decides in shared sessions and events), rider and object footprints in the path frame, soft bumps (slowed, pushed apart, wobble) and hard hits (crash: fall, down, remount; wheel touches and head-on hits), fallen bikes as obstacles, autodrive and bots avoid collisions (overtake on the region's side or hold back), manual riders must avoid them themselves, rider injury and bike damage meters with effects, damage indicator in the HUD; milestones K1–K7, open questions Q1–Q7 |
| 2.49 | Release versions (§11.3): every GitHub release now increments the **patch** version (0.2.0 → 0.2.1), not the minor version; `make release-prepare` / `version-bump` default to `PART=patch`, minor / major by hand; tags `vX.Y.Z` |
| 2.48 | Phase 11 decision (§14 Phase 11, Q1): the development headset is the **Oculus Rift (first generation, CV1)** on Windows through the Meta PC app's OpenXR runtime; the code stays OpenXR-only and vendor-neutral so other headsets work through their OpenXR runtimes; X1 targets the Rift CV1 first; the CV1 is promoted from "legacy" to "primary dev target" in the support table |
| 2.47 | Wheel/road contact (§2.5, bug 2026-11-13 "wheels 1 cm inside the road"): the drawn road surface (route ribbon, package road strips, tunnel floor) lies `ROAD_SURFACE_LIFT_M` = 2 cm above the profile and the riders stand on it (they stood on the profile: 2 cm in the road); each wheel rests as a circle on the surface (crests and dips) at the model's real wheelbase; bikes lean and rock about the tyre's crown line (anchor `tyre_crown`); the tyre's outermost tread (knob tips) is the wheel radius; finer tyre meshes; a route file is fitted onto the package's road strips whenever it differs by more than 1 mm (`route_fit::ON_ROAD_M`; the warning stays at 0.25 m). Measured by `wheel_contact` tests: within 2 mm for every bike, LOD, lean and slope on synthetic roads; on the Liechtenstein roads median 0 mm, residuals where strips overlap at junctions (up to 0.2 m) and from the 5 m sampling (≤ 15 mm) |
| 2.46 | Phase 8 T1 / T2 (§4.6, §8.1, §13, §14 Phase 8): material library of 21 CC0 materials (ambientCG, Poly Haven) – ground, roads, houses, volcanic ground for Tenerife – not in git but pinned by URL + SHA-256 in `assets/materials.toml`, fetched and converted to KTX2 (BC3 / BC5, 1024², mipmapped) by `scripts/fetch-materials.sh` into the git-ignored `assets/materials/` and shipped beside the binary (AppImage, macOS bundle, Windows zip, test deployment); licence rule of Phase 8A shared by the model and material validators; terrain drawn with the materials blended by the splat map (weights at cell corners, noise warp, height blend, rock and triplanar on slopes, macro scale, fade to the colour map in the materials' mean colours), roads by OSM `surface` (new package section `RSRF`) with category fallback; regional material sets by the package's time zone (`[[region]]` in the manifest: the volcanic set of the Canary Islands with lava, ash / cinder, black sand and pine needles); regional tint hook neutral; reference boards `art/reference/` |
| 2.45 | Licensing (§13): no licence change for now – the code stays MIT / Apache-2.0 (a switch to CC BY-SA was considered and declined) |
| 2.44 | Phase 8 decisions: no commercial release, 8A stays (Q2); orthophoto corridor size acceptable (Q4); seasonal variants (Q5); own photos postponed (Q6); Mapillary CC BY-SA, reference by eye only (Q7); 3D buildings: swissBUILDINGS3D 3.0 (Opendata BY) to confirm, LGL LoD2 probably not open, ask LGL (Q8); all texture areas in parallel (Q10) |
| 2.43 | Phase 8 decisions: no AI-generated textures in shipped assets (Q3), no real brands or logos (Q11) |
| 2.42 | Fourth world: Tenerife (§4.7), the whole island in one package (`europe/spain/tenerife`, 238 MB, 2,690 tiles, built with the world cleaning, time zone `Atlantic/Canary`) from the Geofabrik Canary Islands extract and the IGN MDT05 (5 m LiDAR, CC BY 4.0, via the IGN's OGC API Coverages; REGCAN95 / UTM 28N, grids moved onto the 5 m lattice); the sea from the OSM coastline (closed rings cut out of the package area, water at 0 m, sea floor 2 m down, shore 0.3 m up, kept at 0 m by the water cleaning, §4.6); `--tiles-near-roads` (no open-sea tiles); edge vertices of border tiles get water; routes Puerto de la Cruz - Teide, Los Cristianos - Teide, Masca - Santiago del Teide, Buenavista - Masca - Santiago del Teide, La Laguna - Anaga; showcase scenes and reference board `art/reference/tenerife.md`; missing materials noted (Phase 8) |
| 2.41 | Fixes from the kiosk (user, 2026-11-13; §3.2, §4.3): flat overlays (centre markings, road strips, the test-route ribbon) never cast shadows (`FlatOverlay` requires `NotShadowCaster`; they still receive shadows); no direct sun deeper than 25 m in a tunnel (the terrain casts no shadows in the renderer) and the bike lights on there; a route file ridden in a world package has its heights fitted onto the package's road strips when they differ by more than 0.25 m (a route file and a package from different terrain models put the rider up to 15 m under the road and made a 1.7 km tunnel of the Steg tunnel) |
| 2.40 | World cleaning (§4.8, §4.3, §4.6): `world-builder lint` measures steps, kinks, floating / buried roads, junction steps, near-miss ends, rivers flowing uphill, tilted lakes, floating buildings and land-use slivers; the builder cleans while building – nearest-point terrain conforming plus a least-lowering settle pass (the old 15 m cut zone cut the terrain under climbing roads to the height of the node behind: a third of the road length floated 0.8 m or more), duplicate nodes and spikes removed, near-miss ends snapped (≤ 3 m, no topology change), junction mouths eased, vertical curvature limited, rivers fitted monotonically downhill, lakes level, no carving under roads, culverts read, building sites levelled (≤ 1.5 m), slivers merged; bridges (section `BRDG`): tagged, `man_made=bridge` outlines and detected crossings of water or gaps, with deck, parapets or rails, piers, abutments, arches, truss / suspension / covered superstructures, rendered with plain material colours; `--no-clean` builds as before |
| 2.39 | World menu (§3.2.1, *World menu*): top-level *World* (Change world, About this world, Routes, Time, Weather) in place of *Restart*, which moved to the Ride page (the top page keeps seven items); *Ride → Change ride* became *World → Change world*; the time machine moved from Navi to *World → Time*; world discovery `ocw_worldstore::discover` (cache, `--world` and the packages next to it, data directories; one entry per manifest id, neighbouring packages as one world with parts, broken / unsigned packages listed with their reason, package summaries read without loading, `ocw_world::summary`); About this world (package, area and elevation, sun and twilight times at the rider, data sources with their notices, routes with length and climbing); Weather page as a hook (`WeatherState`, clear until Phase 9 S4, weather machine disabled until S5); in shared sessions only the host switches the world, checked on the server (`WorldRequest`, `WorldChange`, `Welcome.world_access`, `ocw-server --world-control`; everyone joins again on the same connection); console `world list / info / switch`; the three Black Forest packages (v2.36) list as one world with parts |
| 2.38 | Release versions (§11.3): every GitHub release increments the minor version; `make release-check` / `release-prepare` / `release-publish` / `version-bump`, tags `vX.Y.0` |
| 2.37 | Rider shadows (§8.1, far LOD): exactly one of an avatar's two models casts its shadow – the near model below 40 m from the camera, the far model from 40 m – because Bevy dithers the LOD cross-fade only in the camera passes and opaque shadow passes have no fragment shader, so both models cast full shadows in the fade band ("two shadows of the rider", 2026-11-13). The sun is the only shadow-casting light; bike lamps cast none (performance) |
| 2.36 | Black Forest in three packages (§4.7): south (rebuilt), middle (Elzach, Triberg) and north (Baden-Baden, Schwarzwaldhochstraße, Mummelsee, Hornisgrinde, Sasbachwalden) cut from both Regierungsbezirke on the lines 48.12 N and 48.45 N with one origin; chains of neighbours ride as one world (neighbours of neighbours joined, as skeletons without tiles until the rider comes near, §2.2.2); new routes; release zips with LICENSE-DATA.md; showcase scenes and reference board `art/reference/black-forest.md` |
| 2.35 | High-resolution terrain (§4.3, Phase 8 Q9): LGL DGM1 (1 m, imported at 2 m) for Baden-Württemberg and swissALTI3D (2 m) for Liechtenstein replace SRTM, which stays the fallback, feathered over 300 m at the edge of their coverage; pure-Rust GeoTIFF / XYZ reading with UTM and LV95 projections; package terrain grid stays 10 m (5 m measured: 3 × the size); freshest Geofabrik extracts; measured package size, build time, builder memory and road/terrain agreement before and after |
| 2.34 | Game audio (§3.3): the other riders' bikes are heard – bots and remote riders (the ghost stays silent) with the own bike's synthesis (freehub only while coasting, decided by their power; drivetrain, tyres; no wind), placed at the own rider's head: inverse distance from 2 m, faded out 50–70 m, equal-power stereo by direction, air absorption, Doppler at half strength within ±3 %; a character per rider from its id (hub engagement points, wheel, click, tuning, cadence, level, phases); the nearest 8 with their own voice, up to 48 more in one group layer, others 6 dB under the own bike, a crowd normaliser; volume `audio.others` (master × effects × others, 80 %) on Settings → Audio (Music leaves the page until there is music); the mix and the recordings are stereo |
| 2.33 | Phase 9 S1 and S3 implemented (§14 Phase 9, 9I): crate `ocw-sky` (jiff with the bundled tzdb, NOAA sun, world clock, time machine, light model; wasm), session `WorldClock` in `GameServer` on session time (`Welcome.clock`, `Snapshot.clock`, `ClockRequest`), host-only time changes checked on the server (`ClockAccess`, `ocw-server --time-control`), package section `TZID` (zone fallback from the origin), sun / atmosphere / exposure / fog / shadows / bike lights in `ocw-render::sky`, HUD world clock, console `time` as a user command, Navi → Time machine, settings `display.night_brightness` / `display.sky` / `display.bike_lights` (Q3 answered provisionally: dark night with a modest floor, default 10 %) |
| 2.32 | Phase 11 planned: VR mode (§14 Phase 11): OpenXR only (Meta Rift / Quest via Link, SteamVR headsets, Pico, Varjo; Linux via SteamVR / Monado / WiVRn; no macOS; WMR not supported after Windows 11 24H2), first-person riding with own body, mirror view for captures, world-space HUD on the stem, comfort options, spatial voice, performance preset; milestones X1–X8, open questions Q1–Q6 |
| 2.31 | Timestamped captures (§3.2, §11.4): every screenshot and video frame shows an ISO 8601 local timestamp with UTC offset at the top right (always, also with `hud off`), and every file carries it as metadata – screenshots as JPEG (quality 90) with EXIF `DateTimeOriginal` / `CreateDate` / `OffsetTimeOriginal` / `Software` (PNG with `tEXt` `Creation Time` / `Software` for a `.png` path and the lossless `--capture` frames), MP4s with `creation_time` and the build in `comment`; frame capture stamps the capture start plus game time. Test media at 2560×1440, H.264 CRF 18. Videos with game sound (§3.3): live recordings tap the device's mixer output (or render it with game time without a device), frame capture renders exactly one frame's worth of samples per frame; AAC 192k, 48 kHz stereo |
| 2.30 | Phase 10 planned: browser app for riding (§14 Phase 10): Bevy on WebGPU (WebGL2 fallback), local server in the browser, WebTransport endpoint, tile streaming with OPFS cache, Web Bluetooth (Chromium only) with a sensor bridge for Safari / iOS / Firefox, Web Audio + LiveKit, captures with timestamps; milestones B1–B8, open questions Q1–Q6 |
| 2.29 | Decisions: weather changes the physics realistically, races per organiser (Phase 9 Q4); best open data for every region – high-resolution official DTMs, freshest OSM, best open imagery (Phase 8 Q9); attribution licences accepted (Phase 8 Q1) |
| 2.28 | Phase 9 decisions: time = local time of the rider's place in the virtual world, never the person's physical location (Q1); weather realistic including extreme weather, no cap (Q9) |
| 2.27 | Second world: the southern Black Forest (§4.7): Geofabrik Regierungsbezirk Freiburg + SRTM (`fetch-pilot.sh blackforest`), one 260 MB package of 2,260 km² (47.72–48.12 N, 7.62–8.30 E, origin 47.92 N 7.96 E) chosen by measured build and client memory (core 1,000 km² / 124 MB, large 5,370 km² / 564 MB over the §15 memory budget), more of it as neighbour packages later; routes Freiburg – Schauinsland and Titisee – Feldberg (`build-blackforest.sh`); `world-builder route --bbox / --origin`, `world-builder areas`; fixes: gutters (`ditch` / `drain` under 2 m) no longer water (§4.6), bike-open pedestrian streets paved (§4.4), a package rewritten in place gets a new manifest (§4.7); scene harness with named worlds (§11.4) |
| 2.26 | Phase 8D: reference photos (Wikimedia Commons, web image search, own photos) to learn how things look; looking is free, copying only under 8A licences; reference boards `art/reference/<region>.md` |
| 2.25 | Phase 9 planned: sky, time, seasons and weather (§14 Phase 9): real local time and sky at the rider's world position (sun, moon, stars moving, twilight, night lighting), seasons by date and altitude, time machine (any date/time, time speed, presets), weather simulation and weather machine (presets, custom, transitions), optional weather physics (wind, wetness, air density; per session), real weather from open providers via the server (MeteoSwiss, DWD, MET Norway; Open-Meteo non-commercial; ERA5 for historic weather), milestones S1–S8, open questions Q1–Q10 |
| 2.24 | Phase 8 planned: textures and regional look (§14 Phase 8): licence rule for textures/imagery/models, research results on open aerial and satellite imagery (Orthophoto Liechtenstein / SWISSIMAGE, LGL DOP20, own Sentinel-2 composite, Copernicus forest type), CC0 material libraries, procedural generation, region calibration (palettes, roof colour per building), milestones T1–T7 and open questions Q1–Q11; §4.6 imagery table corrected (EOX licences per year) |
| 2.23 | Release builds with voice on all platforms (§11.3, §7): every release binary is built with the LiveKit backend (`--features ocw-desktop/voice`; `OCW_FEATURES` in the packaging scripts); Windows links the C runtime statically (`.cargo/config.toml`, required by LiveKit's /MT libwebrtc, no VC++ redistributable needed); the macOS app script works around Command Line Tools that hide the SDK's libc++ headers; dropping the LiveKit backend joins its threads (Windows access violation in tests). Release `v0.1.0-72238db` |
| 2.22 | Render rate and FPS counter (§3.2): wall-clock FPS badge, `--log-fps`, `--no-vsync`, `--max-fps`; multiplayer test (§11.4.1): no client-side multiplayer bottleneck (measured), real-time 25 fps kiosk/camera videos with the FPS counter, locked kiosk draws with `--no-vsync --max-fps 60`, camera on the release build; §15 frame-rate measurement |
| 2.21 | Game audio (§3.3): the freehub ticks only while the wheel overruns (coasting: power ≤ 10 W and no cadence able to drive the wheel), fades in 80 ms / out 8 ms, exactly silent while pedalling; drivetrain is a continuous whir, not a click train; demo ride in separated sections |
| 2.20 | Multiplayer test across machines (§11.4.1): headless `ocw-bot` client and verdict, `ocw-server --route-file` / `--stats-file`, kiosk and camera clients, locked-screen and Mac-sleep handling; every test is a `test-*` make target, old names deprecated (§11.4); cross-references §6.2, §10, §14 |
| 2.19 | Fixes: event bots varied; every menu page fits 16:9 (layout maths); riders stop at the end of a route, teleport clamped; near tiles built at once after a jump (§4.5); tunnel portals cut into the terrain; `--world` alone rides the first package route or free ride |
| 2.18 | Scene-rendering harness (§11.4): `scripts/scenes.toml` + `scripts/render-scenes.sh` (`make test-scenes`, `--remote`, `--pick`) renders reproducible screenshots and videos headlessly on Wayland for the build history (zips on the GitHub release `build-history`) |
| 2.17 | Phase 6 voice chat, first increment (§2.4, §7, §6.5, §3.2.1 M8, §14): game server issues LiveKit tokens per room (VoiceTokenRequest / VoiceGrant, Welcome.voice, Snapshot.human_riders); ocw-voice crate (pure, wasm-safe: mute, hotkeys, auto join, spatial audio, nearest-N, devices, mixer); LiveKit backend behind the voice feature (livekit 0.10.1, Linux needs clang ≥ 21); HUD indicator, Settings → Voice; the Strava account page moved from Settings to Navi → Strava (Settings keeps at most 7 entries) |
| 2.16 | Game audio (M5, §3.3): own audio module (cpal behind an `OutputBackend` trait on its own thread; kira rejected, Bevy audio disabled), output device by name with default fallback and hot-plugging, buses master × music / effects / voice, ride sounds and cues synthesised in code; Settings → Audio, `audio.*` settings and console commands, `--no-audio` (§3.2.1); synthesised sounds are code (§13) |
| 2.15 | `artwork/build-history/` removed from the repository; the build history (zips per day and `index.md`) lives only on the GitHub release `build-history` (§11) |
| 2.14 | Phase 4 models and animation (§2.5, §2.6, §8.1, §14): generated glTF bikes per physics profile and a rigid-part rider (ocw-assets, MIT OR Apache-2.0), asset manifest and validator, rider-on-bike IK pose, riding states (seated, climbing, standing, sprint, coasting) and far LOD at 40 m |
| 2.13 | Build-history media out of git: one zip per day on the GitHub release `build-history`, day folders git-ignored, history rewritten again (`release/commit-map.txt`) (§11) |
| 2.12 | Voice platform plan: LiveKit on every platform – Rust SDK on the desktop (Linux build needs clang ≥ 21), `livekit-client` from wasm in the browser, native or Rust SDKs on iOS / Android; `ocw-voice` trait stays open for them (§7) |
| 2.11 | Releases on GitHub Releases with the world package (ODbL data licence, OSM source attached); binaries removed from the git history (`release/commit-map.txt`) (§11) |
| 2.10 | Voice chat always on like a call (Teams / Meet): automatic join, open microphone, no push-to-talk; hotkeys on keyboard (`M`, `Shift+M`) and gamepad (Select, right-stick click) to mute the own microphone and the speaker (all voices); mute state on the HUD (§2.4, §7, M8, §14 Phase 6) |
| 2.9 | Priorities 2026-11-12: the route editor (§2.7) is postponed (Phase 3 is closed without it); Phase 4 models, animations and game audio (M5, §3.3) and Phase 6 voice chat (§7) start in parallel, ahead of the Phase 5 backend |
| 2.8 | Autonomous night 2026-11-11/12 (Appendix B): menu M1–M3 (in-app ride switch, Navi, GPX import), workouts with ERG (§2.8), local events with ghost (§2.3.1), gradient aids and camera tilt, camera classes and tunnels (§3.2), physics profiles and bike looks (§2.5), neighbour packages (§4.7), QUIC transport (§6.2), fixes for one-way U-turns, steep starts and gradient artefacts; all assumptions listed in Appendix B |
| 2.7 | Menu M2 (§3.2.1): settings registry (keys, scopes, layers) and `settings.toml`, `set` / `get`; rider profiles in `riders.toml`, weight 30–200 kg in `Hello.rider_weight_kg` (the client's physics parameters are ignored, §2.5, §6.5) and as Wahoo SIM mass; Settings / Workshop pages as rows of command lines; HUD on / off and imperial units |
| 2.6 | Land use and buildings from OSM (§4.6, §4.7): 17 land classes on a 5 m grid per tile (`LUSE`), flat water with carved terrain, colour maps and trees by land use, extruded buildings with flat or gable roofs (`BLDG`) |
| 2.5 | Signed, cached world packages (§4.7, §12): manifest with file and chunk hashes, Ed25519 root → signing key chain (dev root committed, public only), trust policy for unsigned packages, cache `~/.local/share/ocw/worlds/<id>/<version>`, verified download with resume; end-to-end test from an OSM fixture to a bot ride and a rebuild |
| 2.4 | Routes on the road graph (§2.2.1, §2.7, §4.4, §4.7): GPX import with HMM map matching, package routes (section `ROUT`, `ocw-graph-route` files), route arrows with leave / rejoin (J), re-matching after rebuilds; U-turn loop from the road width (§2.2.2); package check in Hello / Welcome (§6.5); border matching for neighbouring packages (runtime loading still open) |
| 2.3 | macOS app bundle (§11.3): `scripts/build-macos-app.sh` (Info.plist with Bluetooth usage text, icon, ad-hoc signature, DMG and zip), logs to `~/Library/Logs/OCW/` when started from Finder, CI job building the app on pushes to main |
| 2.2 | Menu decisions (§3.2.1): the ride never pauses (trainer unchanged, time and recording continue, autodrive while the menu is open); Esc never quits; rider weight affects physics, rider profiles in Settings, chosen in the Workshop |
| 2.1 | In-game menu plan (§3.2.1): structure (Ride, Workshop, Navi, Settings), gamepad navigation, settings file, increments M1–M9 mapped to phases; audio output device plan (§3.3) |
| 2.0 | Restart key R, camera reset on C, video recording on V; gamepad ↑ / ↓ also on the D-pad and the left stick, analog triggers, U-turn on East (§3.2) |
| 1.9 | Catch-up of implemented decisions: elevation filter along through-roads and terrain cut under roads (§4.3), road surfaces (§4.4), route file format (§4.7), console scripting (§2.9), input devices incl. gamepad, camera and avatar on the terrain (§3.2), start point and bots (§6.1), repository layout (§10), build machines, desktop integration, AppImage, release and build-history folders, CI resilience (§11), own artwork licence (§13) |
| 1.8 | Steering fixes (§5.2.1): only pushing into the road edge scrapes; a held opposite lean key pauses at upright, then leans on to the other side |
| 1.7 | Free ride on the road graph: junction keys next to lean steering, U-turn key and loop (§2.2.1, §2.2.2); road graph in the world package, format 2 (§4.7); free-ride path and slow-downs (§5.2); `TurnIntent`, `UTurn`, `PathUpdate` (§6.5) |
| 1.6 | FIT export: own writer crate `ocw-fit`, rides saved in the user data directory; interim direct Strava upload from the desktop with user-supplied API credentials until the backend exists (§2.8) |
| 1.5 | Lateral steering: autodrive, manual lean steering, road edge, wild mode; surfaces with their own rolling resistance (§5.2.1) |
| 1.4 | Wahoo's proprietary trainer control as a fallback for trainers without FTMS (§3.4, §5.3) |
| 1.3 | Graphics engine spike results (§3.1.1): Bevy confirmed on Linux; trees merged per tile |
| 1.2 | Testing strategy (§11.4); unit tests required for every development step (§14) |
| 1.1 | Decisions from the specification review merged in (former PROPOSALS.md, P-1 … P-20; see Appendix A) |
| 1.0 | Original specification (translated from German) |

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
| Full web client (riding in the browser via Web Bluetooth) | – | Planned as Phase 10 (§14) |
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
  - **Adjacency is geographic** (no list of neighbour ids in a package, which would need every package rebuilt when a neighbour appears): each package stores its region (section `REGN`, §4.7), its manifest the region box (`bounds`). At the start the client (and the headless server) look for installed packages whose box touches the home package's (within 0.01°): the newest version of every other id in the cache, and the packages in the directory of a `--world` path. Each candidate is loaded and verified like the home package (same trust policy, §4.7) and joined only if at least one road crosses to it (`WorldSet::add_neighbour`); a package covering the same area (another build of the same region) or without a region is refused. The search goes on from every joined neighbour (v2.34), so a row of packages (the Black Forest: south – middle – north) rides as one world from any of them, up to 8 packages; neighbours are loaded as **skeletons** (graph, routes, region and tile keys, verified like a full load) and joined without their tiles, which come when the rider comes near (below). `--no-neighbours` rides the package alone.
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
- **Implemented (Phase 6, first increment):** "shared ride" is decided by the client from two new protocol fields: `Welcome.voice` (the server issues voice tokens) and `Snapshot.human_riders` (riders of connected clients, bots not counted). The client joins when the server offers voice and there are at least two human riders, or when the rider chose a voice group (`voice.group NAME`, then even alone); it leaves 20 s after the ride became solo (a short gap or a reconnect does not drop the call), at once on a ride switch or when `voice.enabled` is turned off. Failed joins are retried after 2, 4, 8 … 60 s. The local server never offers voice, so solo rides never connect. The mute switches are `voice.mic_muted` / `voice.speaker_muted` in `settings.toml`; the confirmation sounds (rising tone = on, falling = muted, softer two-tone for join / leave) are played by the voice mixer, so they are heard only while in a call (outside one no audio device is open). The HUD indicator (bottom left, above the elevation profile) shows "MIC on / muted", "SPEAKER on / muted", the status and the names of riders talking; talking riders get a small green marker above their avatar. Hotkeys work in the ride input context only (not while the menu or console has the keys).
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
- Component unlocking: parts are bought with **hearts** earned by riding (1 heart = 1 EUR of the real price, rounded up; §14 Phase 4, 4E).
- 3D preview: the **workshop scene** – the selected bike on a repair stand, the owned bikes on the wall, parts chosen on the bike itself (§14 Phase 4, 4A–4D).

The bicycle configuration consists of reusable components. Each component has its own properties, such as weight, aerodynamics, rolling resistance, geometry and visual appearance.

#### Cosmetics vs. Performance

Visual properties are separated from physical properties:

- Each component has a `visual` part (mesh, materials, paint slots) and an optional reference to a **physics profile**.
- Only a **small, curated set of physics profiles** exists per bike type (e.g. "road – aero", "road – climbing", "road – standard"). Cosmetic variants share profiles.
- Unlocks and paint schemes are **always cosmetic**.
- The client never sends physics parameters (v2.7): it sends only the rider's weight (`Hello.rider_weight_kg`, §3.2.1) and the id of the bike's physics profile (`Hello.bike_profile`); the server takes drag, rolling resistance, bike mass and drivetrain efficiency from its own copy of the curated table (an unknown or absent id is the default profile).
- Differences between profiles are capped (e.g. at most ±3 % speed at 250 W on flat road).
- **Parts (planned v2.53, built v3.03, §14 Phase 4, 4G):** the frameset decides the profile; other parts carry real weights and a few curated *classes* (rolling, aero, efficiency) that move the build only within a capped band of the frame's profile (best build at most +1.5 % on the flat, +2.5 % on a 6 % climb; at least 6.8 kg, 9.0 kg for MTBs); the client sends part ids, never values. Race mode *Profile* (the default for races and group rides): the frame's profile counts, parts are cosmetic.

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

- *Caps (tested):* at 250 W, 75 kg, on flat asphalt every profile is within ±3 % of `road`'s steady speed (TT +2.7 %, aero +1.5 %, MTB −2.3 %); on a 6 % climb within ±5 % (climbing +1 %, MTB −4.4 %). Off the asphalt the differences are larger on purpose (grass: MTB about +17 %, gravel +12 %), since choosing the bike for the terrain is the point of the types. The race modes *Open*, *Profile* and *Equal* are built (v3.03, §14 Phase 4, 4G).
- *Look* (`RiderLook`, cosmetic): frame, jersey and helmet colour (sRGB `0xRRGGBB`; the server keeps only the low 24 bits) and a kit pattern (plain, band, sash; since Phase 8 T6 also stripes, chevrons, gradient; unknown values are plain). Since T6 also the kit design (`ocw_core::KitDesign`: second and third colour, race number, the pattern on the helmet, the colours of shorts, socks, shoes and gloves, helmet and shoe finish), which travels like the character: in the full look of `Hello`, `BikeChange` and `RiderBuild` (once per change; fields 12–21 of `RiderLook`, only added), not in the snapshots. Sent as `Hello.look`, stored per rider by the server and passed through in every `RiderUpdate` (`bike_profile`, `look`) together with the display name; a reliable roster message can replace these per-snapshot fields when bandwidth matters (Phase 5). Riders without a look (old servers) are drawn in the colours by id.
- *Persistence:* per rider in `riders.toml` (`bike`, `frame_color`, `jersey_color`, `helmet_color`, `kit`; all optional, so files from before are read unchanged; invalid values are dropped with a warning).
- *Models (§8, Phase 4):* glTF 2.0 files in `assets/models/`: one bike per profile (`bike-<profile>.glb`) and one rider (`rider.glb`), listed in `assets/models/manifest.toml`. They are made by a generator in `crates/ocw-assets` (`cargo run -p ocw-assets -- generate`) from real geometry numbers (stack, reach, seat and head angle, chainstay, bottom-bracket drop, fork rake, wheel and tyre size, crank length), so the silhouettes differ the way real bikes do:
  - `road`: compact frame with a slightly sloping top tube, 35 mm rims, 24 spokes, two bottles, rim brakes;
  - `road-aero`: deep aero tubes, dropped seat stays, aero seat post, 60 mm rims;
  - `road-climbing`: thin round tubes, level top tube, shallow 28-spoke wheels;
  - `tt`: 78° seat angle, low front, base bar with extensions and arm pads, three-spoke front wheel, disc rear wheel;
  - `gravel`: taller head tube, flared drops, 40 mm tan-wall tyres with tread, disc brakes, top-tube bag;
  - `mtb`: long and slack (66°), suspension fork, 760 mm flat bar, 2.3" knobby tyres, disc brakes.

  Nodes: `bike` (frame, fork, drivetrain), `wheel_front` / `wheel_rear` (spin about X), `crank` with `pedal_l` / `pedal_r`, `handlebar` with the grip anchors `grip_hoods_*`, `grip_tops_*`, `grip_drops_*`, `saddle` (where the sit bones rest) and `lod1` (far model). Materials are *paint slots* (`ocw_assets::slots`): `frame`, `jersey`, `helmet` and `accent` (kit pattern; white on dark jerseys, black on light) take the rider's look, the others (tyre, rim, metal, component, bar tape, decal, bottle, shorts, skin, shoe, sock, lens, hair, …) keep the colour in the file; since T6 `shorts`, `sock`/`sock_aero`, `shoe`/`shoe_race`/`overshoe` and `glove` take the kit design's colours when chosen, and every slot is drawn with a real material (§14 Phase 8 T6). An avatar is rebuilt when its bike or look changes; riders with the same colours share materials, and all avatars share the meshes.
- *Asset decisions (Phase 4):*
  - *Where the models come from.* Options: model them in Blender and export; download CC0 models; generate them in code. **Generated in code**: reproducible, reviewable as a diff, the licence is clear (own work), the six bikes follow from their geometry numbers, and the anchors the pose uses come from the same numbers. The test `shipped_assets_match_the_generator` fails when the files are stale. Medium – the look is "engineered low-poly"; an artist's models can replace any file later, since the validator and the loader accept any rigid-part glTF that has the required nodes and paint slots.
  - *Licence of the own 3D assets.* §13 suggested CC BY 4.0. **MIT OR Apache-2.0**, like the code and `artwork/`. The validator accepts `MIT OR Apache-2.0`, `MIT`, `Apache-2.0`, `CC0-1.0` and `CC-BY-4.0` (with an author) and rejects anything else (share-alike, non-commercial, no licence). Medium – review by a lawyer before a commercial release (§13).
  - *Loading.* Options: Bevy's glTF loader (asset server, scenes, asynchronous), or reading the files with the `gltf` crate (the one Bevy uses) into our own model and building Bevy meshes directly. **Own reader, files embedded in the binary** (`include_bytes!`): spawning is synchronous (no "scene not loaded yet" frames, one rebuild path), there are no asset paths to package on Linux / Windows / macOS / AppImage, and the validator and the app read the same way. The files are plain glTF facing +Z (glTF convention); the reader turns them to the app's −Z. Medium – textures, skins and morph targets are not supported yet (not needed by the generated models).
  - *Far LOD.* Each bike file has a `lod1` node: frame, wheels, bars and a seated rider (posed by the same code as the near model) in at most 8 meshes (≈ 1/5 of the triangles). Switch at **40 m** from the camera with Bevy's `VisibilityRange` (4 m dither cross-fade); beyond it the near model is hidden and not posed. Shadows do not cross-fade: exactly one model casts (`NotShadowCaster` on the other), switched hard at 40 m, inside both models' ranges (v2.37). Low – the distance is a first tuning.
  - *Wheel/road contact.* The tyres touch the road surface as drawn, within 2 mm (no sinking, no visible gap), for every bike, both LODs, any wheel angle, slope, crest or dip, lean and rocking (unit tests `ocw-render::wheel_contact` place every shipped model on known roads and on the Liechtenstein package's Vaduz–Malbun route and compare the lowest tyre vertex with the road triangles under it). How: every drawn road surface lies `ROAD_SURFACE_LIFT_M` (2 cm, clear of the conformed terrain) above the profile and the riders stand on it; each wheel rests as a circle (hub height = highest support within ±r) at the hub positions of the model (`BikeRig::wheelbase`); lean and rocking turn about the tyre's crown line (node `tyre_crown`, `BikeRig::tyre_crown`), so the tyre rolls onto its side; the tyre's outermost tread (knob tips) is the wheel radius, and the tyre meshes are fine enough that their polygon sag stays within the 2 mm (near 48 × 12, knobby 96 × 12, far 12 × 10 with a vertex straight down). Low – a tyre does not flatten under load.
- *Bike type is the profile's.* The model follows only from `ProfileId`; the manifest names the profile a bike shows and the validator rejects physics values (`mass`, `cda`, `crr`, …) in an asset entry: cosmetic assets never change physics.
- *Bots* ride the profiles in turn (road, TT, gravel, MTB, aero, climbing) with palette colours and patterns (never the local rider's #FF1472 jersey).
- **Race modes:**
  - *Open* – the chosen profile applies with the parts, within the build band (§14 Phase 4, 4G); free ride, single player, workouts and time trials.
  - *Profile* (decided v2.68, built v3.03) – the frame's profile applies, parts are cosmetic; the default for races and group rides.
  - *Equal* – standardized physics for everyone (the `road` profile with the rider's own weight); chosen equipment is shown only visually.
  - The organiser chooses (`EventStart.physics`, console `event race|group|tt --physics open|profile|equal`); the mode is in `EventState.physics` and in the event's info on the start line and in the results.

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

The character editor is the **changing corner** of the workshop scene (mirror, bench, locker; §14 Phase 4, 4C). Body, skin, face, hair and glasses are always free; the rest of the kit (helmet, shoes, gloves, clothing) costs hearts (decided 2026-11-13, Phase 4 Q4) and **gives a slight benefit** (decided 2026-11-14): e.g. a time-trial helmet makes the rider a little faster than without; every item has a benefit score (§14 Phase 4, 4C). Anonymous riders of the browser app get a plain default rider and cannot customize (Phase 10 Q5).

**Implemented (Phase 4: rider model and riding animation):**

- *Model:* one generated low-poly rider (`assets/models/rider.glb`, about 1.78 m) for everyone: jersey (look colour) with a zip and the kit pattern as separate nodes (`kit_band`, `kit_sash`; since Phase 8 T6 the patterns are drawn by the gear shader and these nodes are never shown), bib shorts, gloves, socks, shoes, glasses. **W4 (v3.07):** everything a look chooses is an optional node (`ocw_assets::rider::opt`, `visible`): the kit's variants (`rider::VARIANTS`, the `model` of the apparel items: lightweight, aero road and time-trial helmets, the cotton cap worn under the helmet, sleeves to the elbow, long sleeves, a skinsuit over the shorts, winter tights, longer aero bibs, tall and aero socks, race shoes, overshoes, gel / full-finger / winter gloves; a slot without an item wears its base item), hair styles (short, buzz cut, long, ponytail, bun, bald), faces (classic, beard, moustache, stubble) and glasses (sport wrap, round; under the TT helmet its visor), all generated in code (licence rule 8A). Skin and hair are paint slots tinted from the character; the body height scales the whole rider (the rigid parts and so the rig's lengths together; the saddle moves along the seat tube until the legs reach 95 % again, `ocw_assets::pose::BodyFit`, 1.50–2.05 m on every bike), the build (slim / medium / strong) widens the torso and limbs' meshes (not head, hands, feet). The far LOD keeps the base look.
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
| `user` | Always | `help`, `fps`, `time 18:30` (the server checks who may change the time, §14 Phase 9, 9I), `net.stats`, `devices.list`, `trainer.difficulty 50`, `camera.mode chase`, `screenshot` |
| `dev` | Single-player / local server, or debug builds | `sim.power 250` (virtual power without a trainer – useful for testing), `teleport km 12.3`, `weather rain`, `world.reload_tile`, `render.wireframe`, `render.lod_bias` |
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

**Bevy versioning:** Bevy has breaking changes in every release until 1.0. An exact Bevy version is pinned in the workspace and upgraded in dedicated milestones, typically one release behind current, once the ecosystem crates we use have caught up. Only `ocw-render`, `ocw-client` and the apps depend on Bevy (§10), so upgrades stay local.

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

**Input devices (v1.9):** keyboard and gamepads (Bevy's gamepad support, gilrs: Xbox, PlayStation, Switch Pro and generic pads, hot-plugged). Keyboard: ↑ adds power (25 W per press, more while held, on top of any device), ↓ stops (power 0 and brake), ← / → steer (§5.2.1) or choose exits (§2.2.1), A autodrive, W wild mode, U U-turn, Q / E exits, PgUp / PgDn ±10 W, F11 next screen, C camera reset, R restart (v2.0), V video (v2.0). Gamepad: left stick left / right steers (sets the lean directly, centred = upright); ↑ = D-pad up, left stick up or right trigger, ↓ = D-pad down, left stick down or left trigger (triggers and stick read as analog values, v2.0); South = autodrive, West = wild, D-pad left / right = exits, East = U-turn (v2.0). Several pads: the largest stick deflection steers. **Whole UI (decided 2026-11-13, v2.54):** every screen and menu is also controllable with mouse, gamepad and – where the device has one – a touchscreen; the input model, touch in a ride and the platform facts are in §3.2.2.

**Restart and video (v2.0):** R (console `reset`) starts the whole app again as at program start, with the same command line: the app exits normally (ride saved, devices shut down) and replaces its process with a fresh one (Unix `exec`, same process id for the kiosk). V (console `video`) records the window: a screenshot is requested on every 1/25 s tick (several in flight, since the GPU readback takes a few frames), each image is placed at the video frame of its request time, and the frames are streamed as raw pixels into `ffmpeg` (H.264 MP4, CRF 18 with the `veryfast` preset since v2.30 – before `ultrafast` CRF 20 –, at most 2560 px wide, in the user's video directory, with the game sound since v2.30, §3.3); a tick without an image repeats the previous one to keep real time, the game never waits for the encoder (an image it cannot take is dropped and counted), a screenshot requested by the user or a script in the same frame takes precedence, and a running recording is finished on exit. V again stops it (the key is ignored while the console is open). While recording, the HUD shows a small blinking red dot with "REC m:ss" (recording length) at the top right; after stopping, "Video saved: <file>" (or the error) for 4 s.

**FPS counter (decided 2026-11-12):** console `fps on|off`, setting `display.fps` (*Settings → Display*): a small green badge at the top right, below the REC indicator, e.g. `58 fps  max 21 ms` – the frames drawn in the last second and the longest of them, renewed twice a second. It measures the **real render rate on the wall clock** (`ocw-render::frame_rate`, one `Instant::now()` per frame), not Bevy's `Time<Real>`, which frame capture (`--capture`) steps by a fixed 1/25 s per frame and would always show 25. `--log-fps S` logs the same rate every S seconds (`render rate fps=… worst_ms=…`, mean and longest frame over the period). Options considered: on by default for everyone (a number riders do not need, and the HUD is busy enough), only in debug builds (the release kiosk and the test videos need it too), **off by default for riders and on in every test picture and video (chosen):** `drive.sh` (screenshots, `--capture` videos, scene harness) passes `--cmd "fps on" --log-fps 5` unless `OCW_FPS=0`, and the multiplayer test turns it on for the kiosk and the camera client (§11.4.1). Before, the counter was one more line at the end of the small HUD text, off by default and never on in tests – the user could not find it in the multiplayer video. For benchmarks `--no-vsync` draws without waiting for the display's refresh and `--max-fps N` caps the rate by sleeping at the end of a frame.

**Timestamp on every capture (decided 2026-11-12, v2.30):** the top-right column starts with a small ISO 8601 local time with UTC offset and seconds, e.g. `2026-11-12T22:14:05+01:00` (Fira Mono 14 px on a dark pill, above REC, the app message and the FPS counter; the event panel starts at 21 % of the height, below them), so every screenshot and every video frame shows when it was taken (the build diary, `art/ART-PROJECT.md`). It is **part of the capture, not of the optional HUD**: `hud off` and `display.hud` do not hide it, and it is drawn above every other UI (menus). Options considered: only while a screenshot or video is being taken (cleaner screen, but a screenshot captures the frame it is requested in, so the stamp would have to appear one frame early, and pictures the app does not take itself – OS screenshots, screen recorders, the kiosk photographed – would carry none), a setting (one more thing a test could leave off), **always visible (chosen):** a clock in the corner is common in games and costs a line of pixels. Clock: the wall clock in normal play, for screenshots and for live recordings (V); during frame capture (`--capture`) the wall clock at the start of the run plus the stepped game time (`Time<Real>`, advanced exactly 1/fps per frame), so the seconds advance once per 25 frames however slowly the frames render (`ocw-core::timestamp::MediaClock`). Formatting, the clock and the UTC offset (chrono's local zone, looked up once a second, daylight saving included) are pure functions with unit tests; the file metadata is described in §11.4.

**GPS position:** bottom-left, the HUD shows the local rider's position as latitude / longitude (5 decimals, N/S, E/W) and altitude in metres, unprojected from the predicted position with the route's or world package's geographic origin (the same as the FIT track); hidden on routes without one (built-in test routes). Console: `rider.gps`. The HUD panel never extends past the window: long lines break at their " | " separators (then at commas) to fit the window width.

**Camera and avatar (v1.9):** the chase camera follows the avatar itself (on the terrain when off the road) along the rider's own heading; it can be orbited with the mouse. It stays above everything camera-solid and lifts until the line of sight to the rider clears it; its look-at point is smoothed. **Tunnels:** where the road runs under the terrain (the terrain more than 0.5 m above the road centre over at least 10 m, gaps under 20 m merged; found by the client from its track and the ground, so route files, package routes and free ride all work without extra package data), the client draws a simple tube (floor, walls, arch, 1 m beside the road edge, 6 m high, unlit and dark with a strip of lamps along the crown, since sunlight and tree shadows would reach through the hill) reaching 4 m out of the hill at each end as the portal. From 15 m before it until 12 m after it the camera (any mode) blends over 0.4 s into a tunnel camera on the road 4.5 m behind and 2.2 m above the rider, looking 10 m ahead; terrain and buildings above do not lift it, the mouse orbit is ignored there. **Portals cut into the terrain (decided, replaces the eye under the hillside):** at a portal the hillside slopes through the tube (the 10 m terrain grid cannot form a vertical face), so riding in, the camera saw a dark-green wall of hillside inside the tube for a second or more (scene `tunnel-video`, Steg tunnel at km 8.77). The terrain is now drawn with its own material (`ocw-render::tunnel_cut`, a `StandardMaterial` extension) whose fragment shader discards terrain fragments inside the tube – walls up to 4 m, then the arch up to the 6 m crown, from the road surface up, following the gradient – given as up to 64 straight 2 m pieces along the track where the terrain beside or over the road is less than 1.5 m above the crown (the portals; the nearest to the rider, updated with the tunnel scan). The cut is exact to the pixel along the tube, which is drawn double-sided, so the portal shows the tube's mouth in the hillside and the eye stays at its place behind the rider (before, it dived close behind the rider and under the terrain once the wheels were covered, which still showed the hillside for a few frames and then jumped). Regression test: `the_terrain_inside_the_tube_is_cut_at_the_portal` (`ocw-render`). Considered and not chosen: tunnel stretches written into the route file / taken from the graph's `tunnel` flag (route files have no flags; tunnels the terrain does not cover need no special camera), removing whole terrain cells around the tube (10 m holes larger than the tube show the inside of the hill), and lowering the camera only (rider still hidden by the hill in curves without a tube). The avatar stands on the surface under both wheel contacts (front wheel first over a bump), leans by atan(v² κ / g) for the curve it rides, and is blended from road height onto the ground over 2 m beside the road.

**Sun, overlays and tunnels (v2.41):** with the sun following the time of day (v2.33) shadows are long at low sun, so what lies flat on a surface casts none: road markings, package road strips and the test-route ribbon carry `FlatOverlay`, which requires `NotShadowCaster` (they still receive shadows, so a marking in a tree's shadow stays shaded; regression test `markings_and_flat_overlays_cast_no_shadows`). The terrain casts no shadows in the renderer (for speed), so only the tube's thin shell stood between a low sun and the rider in a tunnel, and light leaked through in shadow-map squares: the direct sun now fades out over the first 25 m behind a portal (`tunnel::daylight_share`) and is off deeper inside, with the bike lights on there in `auto` mode (tests `no_direct_sun_inside_a_tunnel`, `no_direct_sun_on_the_rider_in_a_tunnel`). **Route files in world packages:** a route file (`world-builder route`) and a package built from different terrain (swissALTI3D vs SRTM: the kiosk on 2026-11-13, after a deploy rebuilt the package with an older builder) differ by metres in height while their roads coincide in plan; the rider sank up to 15 m into the drawn road before the Steg tunnel and the tunnel finder made one 1.7 km tunnel from km 8.2, whose tube the hillside and the real road cut through (broken ceiling, terrain squares inside). The desktop therefore fits a route file's heights onto the package's road strips when they differ anywhere by more than 0.25 m (`ocw_world::route_fit`: the strip under each point that continues the route's height, the offset interpolated across tunnels and gaps; with a warning in the log; not for a remote server, which rides the file as it is). Test `pilot_tunnel` (`ocw-render`) checks it on the real pilot data: the rider on the road at km 8.40–8.77 and one Steg tunnel at km 8.80–9.52.

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

| | Keyboard / mouse | Gamepad | Touch (v2.54, §3.2.2) |
|---|---|---|---|
| Open / close | Esc (closes the console first if it is open) | Start / Menu (☰, Options) | tap ☰ (top left) |
| Move focus | ↑ ↓ ← →, Tab / Shift+Tab, mouse hover | D-pad, left stick (with repeat) | – (taps act directly) |
| Confirm | Enter, Space, click | South (A / ✕) | tap |
| Back (one page up; closes at the top) | Esc, Backspace, right click | East (B / ○) | on-screen Back button, swipe in from the left |
| Change a value (slider, choice) | ← / → on the focused row, mouse drag / wheel | D-pad / stick left / right, shoulder buttons for big steps | tap ‹ ›, drag the slider |
| Switch tab | Q / E, PgUp / PgDn | LB / RB (L1 / R1) | tap the tab, swipe on the tab bar |

- While the menu is open, ride inputs (↑ power, ← → steering, A, W, U, R, V, …) are **not** passed to the ride; one input-context resource (`Ride`, `Menu`, `Console`) replaces today's per-system `console.is_open()` checks. The power from the trainer keeps flowing.
- **The ride never pauses (decided, v2.2):** the menu is an overlay over the running ride, in single-player exactly as in multiplayer (in a shared ride the others keep riding, so one rider's menu cannot stop time). The trainer is left as it is (SIM keeps following the road, ERG keeps its target), the ride time and the FIT recording continue. While the menu is open, steering falls back to autodrive and junctions take the default exit (§2.2.1), so the rider stays on the road; the previous steering mode is restored on closing. Pages that would end the ride (Ride, Restart) ask for confirmation.
- **Esc never quits** (decided, v2.2): only *Quit* in the menu ends the app (kiosk-safe).
- The top page shows a status strip: power, speed, connected trainer, Strava sync state.

**Menu tree**

| Top level | Page | Contents | Phase |
|---|---|---|---|
| **Resume** | | Close the menu, continue the ride | now |
| **Ride** | Events | Local events now (race, group ride, time trial, end the event, §2.3.1); scheduled group rides, races, time trials (§2.3): calendar, sign-up, join a running session, private sessions and invitations | now (local); Phase 5 (sessions), Phase 7 (calendar) |
| | Workouts | Structured workouts (§2.8): list, preview of the profile, start in ERG mode | with workouts (§2.8: basics Phase 2, full Phase 7) |
| | Restart | Same as R: start everything again, ride saved first (moved here from the top page, v2.39) | now |
| **World** (v2.39) | Change world | The worlds found (region, version, size, verified / unsigned / broken, number of routes; neighbouring packages as one world with parts) and the built-in test routes; a world: its default ride (first route, else free ride), free ride (random start, from here), its routes, its parts. In-app switch (M3) | now; map-based start point: Phase 7; package downloads: Phase 5 |
| | About this world | Region and area, package id / version / date, signature, elevation range, data sources with their notices (§13), time zone and today's sun and twilight times at the rider, the routes (length, climbing) | now |
| | Routes | The routes of the world (as Navi → Routes) | now |
| | Time | The time machine (moved here from Navi, v2.39; §14 Phase 9, 9I) | now |
| | Weather | The weather machine (Phase 9 S5, 9I-S5): mode (fixed / simulated; real weather with S7), the presets as a grid with icons, a custom editor, the transition, a random weather; the weather now and who sets it | S5 done; real weather S7 |
| **Workshop** | Bike | Bike type, frame, wheels, tyres, handlebars, saddle, accessories, paint; 3D preview; the physics profile shown separately ("road – aero", caps per §2.5), cosmetic choices never change it. Built (v3.00, W1 + W2): opens the **workshop scene** instead of a page – bike on the repair stand, owned bikes on the wall (← / → to choose), part hotspots, part picker with hearts prices (§14 Phase 4, 4A–4E) | Phase 4 |
| | Character | Body, skin, face, hair, clothing, shoes, helmet, glasses, gloves (§2.6); 3D preview with pedalling animation. Planned: the changing corner of the workshop scene (4C) | Phase 4 |
| **Navi** | Routes | Route library: the package routes and the user's own routes with distance, climbing, elevation profile, region; ride, rename, delete | now (list) |
| | GPX import | Pick a GPX file, region check, map matching with the problem sections highlighted, accept / cancel (§2.7); offers the missing package's download (after the ride, §4.2) | Phase 3 (with GPX import) |
| | Rides and export | Saved rides (`rides/*.fit`): date, route, distance, time, Strava state; export as FIT / GPX, edit title and description, delete | now (list, FIT); GPX export with §2.8 |
| | Strava sync | Per-ride sync state (uploaded / processing / failed / rejected), *Sync now*, *Retry failed* | now (interim upload, §2.8); backend upload Phase 5 |
| **Settings** | Controls | Key and gamepad bindings (view now, rebinding later), autodrive on at start, steering sensitivity (lean rate, stick dead zone), power step (↑, PgUp/PgDn), invert camera, mouse orbit speed | now (view + options), rebinding Phase 7 |
| | Trainer | Devices: scan, connect, forget, the running devices with battery and signal; virtual trainer and its power; control mode SIM / ERG / off; trainer difficulty (§5.3); downhill handling; spin-down calibration where supported | now; calibration with §5.3 |
| | Strava | Link / unlink, automatic upload on / off, account name, last result; without client credentials (§2.8 interim) a short explanation | now (interim), Phase 5 (via backend) |
| | Audio | Mix: master, music, effects (bike, wind, ambience), voice; output device; later voice: microphone, microphone / speaker mute, spatial voice on / off | output + mix with game audio (see §3.3); voice Phase 6 |
| | Display | Monitor, fullscreen / window, resolution, graphics quality (shadows, view distance, tree density, LOD bias), FPS counter, HUD size | now |
| | Riders | Rider profiles (decided, v2.2): add / edit / delete riders with display name, weight, height, FTP, units (metric / imperial); the active rider is chosen in the Workshop; later account sign-in, privacy | now (local); account Phase 5 |
| **Quit** | | Save the ride and exit | now |

**Interaction design**

- **Ten-foot UI:** the app is used on a trainer, often from 2–3 m away and with a gamepad. Text at least 3 % of the window height (scales with it), high contrast, a clearly visible focus frame, at most about seven rows per page, no hover-only functions, no text input where a choice works (names and titles get an on-screen keyboard on the gamepad).
- **Every page fits the window (decided):** all menu sizes are in % of the window height, so a page that fits a 16:9 window fits every 16:9 size (1280 × 720 and up). The panel is 110 % of the window height wide (at most 94 % of its width), an empty status or description line takes no room, and the description texts are short enough that every page – sub pages with the full seven items of a paged list too – fits into 94 % of the height with all its text (unit test `every_menu_page_fits_a_16_9_window_with_its_text`; `ocw-render::menu::page_height_vh` adds up the layout, wrapping text as the monospaced UI font does). In a narrower window a page that would still be too tall drops description paragraphs from the end, never items. Before, the Ride page (seven items, two paragraphs) ran off the bottom at 1280 × 720 and cut off *Back*.
- **Focus navigation** works identically for keyboard and gamepad (spatial navigation between focusable items, a page stack for Back); the mouse is optional. Mouse and touch reach every item too (v2.54, §3.2.2: reachability tested per device on every page; touch-mode hit targets ≥ 9 mm / 48 px). Every item is reachable without a mouse, in the kiosk's borderless fullscreen (§11.2).
- **Values change in place** (slider, ← / → choice) and apply immediately; there is no "Apply" button. Settings are saved when the menu closes. Device scans and Strava actions show their progress inline.
- **Rendering: Bevy UI** for the whole menu, as for the HUD (§3.2): it shares the HUD's theme and scaling, renders in the same window and screenshot / capture paths, and Bevy's input-focus and directional-navigation support fits gamepad control; egui's gamepad navigation is weak and it looks like a tool, not a game. egui stays for the console, dev panels and, in Phase 4, the advanced parameter panels of the editors (§3.2); the Workshop's main selection is Bevy UI.

**Architecture**

- **State:** a Bevy state `UiMode { Ride, Menu }` (the console stays an overlay flag); ride systems run only in `Ride` or check the input context. The menu itself is a **Bevy-free model** in `apps/desktop` (like `ClientCore`): the page tree, page stack, focused item, item values. Input is translated into abstract actions (`Up`, `Down`, `Left`, `Right`, `Confirm`, `Back`, `TabNext`, `TabPrev`); `ocw-render::menu` only draws a `MenuView` resource, as the HUD draws `RideView`.
- **Actions are console commands.** Each menu item either runs a command line (`trainer.difficulty 60`, `devices.connect 2`, `strava.sync`, `reset`) through `ConsoleState::queue`, or reads / writes a setting variable. Menu, console, `--cmd` and `autoexec.cfg` therefore share one code path, and every menu function is scriptable and testable headless.
- **Settings: one registry, one file.** Settings are variables with a name, type, range, default and an apply function (`set controls.autodrive_default on`, `get trainer.difficulty`, §2.9). They persist in `<user config dir>/ocw/settings.toml` (sections like the menu: `[controls]`, `[trainer]`, `[audio]`, `[display]`, `[profile]`, `[ride]` for the last world / route). Precedence: defaults < `settings.toml` < `autoexec.cfg` < command-line options; command-line options and `set --temp` apply only to the session and are not saved. Unknown keys are kept and reported, invalid values fall back to the default with a warning. Device memory (`devices.toml`), Strava credentials and tokens (`strava.toml`, `strava-token.toml`) stay in their own files – tokens never go into the settings file.
- **Starting another ride** (Ride page; decided in M3): an **in-app switch**, not the restart path – the process, the window and the devices stay. (The earlier plan, writing the choice to `[ride]` and re-executing the app, was dropped: a re-exec reconnects the BLE trainer, which takes seconds and can fail.) Nothing is written to `[ride]` yet; the app still starts with its command line.
- **Implemented (M3):**
  - *Ride choice* (`apps/desktop/src/ride/choice.rs`, Bevy-free): a test route (hills, loop), a route file, or a world package in free ride (random start or `--start X,Z`), on a route of the package by name or an `ocw-graph-route` file, on a GPX file matched at the start, or with a route file. It is built from the command line, from the console command `ride.switch --route NAME | --route-file FILE | --world PKG [--free-ride] [--start X,Z] | --world PKG --route NAME|FILE | --world PKG --route-gpx FILE` (a world alone is a free ride) and turns back into the ride options of the arguments; on the command line `--world PKG` without a route option (`--route`, `--route-file`, `--route-gpx`, `--free-ride`, `--start`) rides the package's first route (section `ROUT`), else a free ride (with `--server` always a free ride, as `ocw-server --world` alone), and logs which (before, it rode the built-in hills test route through the package's terrain, with the camera in a hillside; a package without a road graph still gets the test route); any route option wins, e.g. the kiosk's own `--route-file` over a `--world` from its defaults file, so the start and the switch share one setup (`ride/setup.rs`: route, route-file terrain, package with neighbours and road network, free-ride start or route plan; errors are messages, not exits). `ride.current` prints the current ride as a `ride.switch` line; `ride.restart` starts it again.
  - *Switch sequence* (`ride/mod.rs`): the new ride is loaded on a background thread (package reading and verification, neighbours, road network, GPX matching) while the old one goes on; the HUD notice says "Loading … ". A failed load keeps the ride and says why. When it is loaded, in one frame: the ride so far is saved as a FIT file like on exit (under a minute: dropped) and handed to the Strava sync; the ride options, `WorldInfo`, `PackageWorld` and `FreeRideSetup` are replaced and the `RideView` gets the new route; a new session starts (`connection::start_session`: dropping the old local server's link ends its thread, a new local server – threaded, or stepped in frame capture – and a new `ClientCore` join, with the rider profile active *now*: the Workshop's rider and bike choice applies from the next ride, not only the next start); the recorder starts a new ride, the ride time starts at 0 (`SessionStart`), and `ocw_render::rebuild_scene` despawns the old world (light, terrain, road, trees, streamed tiles, camera obstacles, avatars) and spawns the new one from the new `RideView` / `PackageWorld`. Devices (BLE trainer, sensors, gamepads), the trainer mode (SIM / ERG, difficulty), a running workout, the camera, HUD, menu and console stay; a running local event (§2.3) ends with its session (server and client are new) and its HUD state is cleared. **A video recording continues** (decided): it records the window, and the switch is part of what the user sees; V stops it as before. With `--server` (a remote server) the switch is refused: the server chooses the course.
  - *R after a switch* restarts the current ride **in the app** (decided): the process' command line would start the first ride again. Without a switch, R still re-executes the app.
  - *Menu:* Ride → *Change ride* (a sub page next to the event rows; since v2.39 *World → Change world*, see *World menu*) lists *World packages* (the newest version of each package in the cache and the `.ocwpkg` files next to the executable – the kiosk installs one there –, in `data/` and `data/pilot/` of the working directory and in the directory of the package ridden now; one package: *Free ride* from a random road, *Free ride from here* (the current position, in the package ridden now) and its *Routes*) and the test routes. Navi has *Routes* (the routes of the loaded package from section `ROUT` with their length, and the user's routes for this package), *Rides* (`rides/*.fit`, newest first, with the route from the sidecar and the Strava state from `strava-sync.toml`), *GPX import* and *Strava* (a page describing what comes with the backend; no network calls). Rows that start another ride ask first (*Change the ride?*, Cancel focused); confirmed, the menu closes and the switch runs. The lists are read from disk when the menu opens and after a switch or import, not every frame.
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
- **Settings entries (decided 2026-11-12):** with Audio (M5) and Voice (M8) the Settings page would exceed the ten-foot limit of 7 entries, so the Strava account page moved to *Navi → Strava* (it replaces the former placeholder there; it stays a Q / E tab). Settings: Riders, Controls, Trainer, Audio, Display, Voice, Back. Option not taken: one combined *Sound* page for Audio and Voice (an extra level).
- **Local now vs. backend later:** settings, devices, routes, GPX import, rides, the interim Strava upload and the Workshop configuration are local. Accounts and profile sync (§2.1), Events and sessions (§2.3), the backend Strava upload (§2.8), sharing routes and friends need the Phase 5 backend; those pages appear disabled with a short hint until it exists.

**World menu (decided 2026-11-13, v2.39)**

The user asked for a *World* entry where the rider chooses the world they ride in and sees its conditions (time, weather) and its data. Decisions:

- **Menu tree:** the top page has seven items, the ten-foot maximum: Resume, Ride, **World**, Workshop, Navi, Settings, Quit. *Restart* moved to the Ride page (Race, Group ride, Time trial, End the event, Workouts, Restart, Back – seven items; *Change ride* left it), so every page still fits 16:9 (the page-fit test `every_menu_page_fits_a_16_9_window_with_its_text` covers all new pages, the data-source pages with the longest notice, the OSM one). Options not taken: a combined *Restart / Quit* page (Quit is the kiosk-safe exit and stays one step away), World under Ride (an extra level for something every rider uses).
- **World** (`settings::pages::Page::WorldHome`, rows from `apps/desktop/src/ride/world.rs`): *Change world* (value: the world ridden now), *About this world*, *Routes* (the same page as Navi → Routes), *Time* (value: world time and mode, e.g. "14:05 CET, x600"), *Weather* (value: "clear"), Back.
- **The time machine moved to World → Time** (decided): time of day, date and speed are conditions of the world, like the weather; Navi keeps routes, rides, GPX import and Strava. The page itself is unchanged (§14 Phase 9, 9I).
- **Change world** lists what `ocw_worldstore::discover` finds, read when the menu opens and after a switch:
  - *sources:* the world cache (`<data dir>/ocw/worlds`, the newest version of each id), the package given with `--world` or ridden now, the packages **next to it** (the kiosk installs several in `/var/tmp/ocw/`), and the `.ocwpkg` files next to the executable, in `data/` and `data/pilot/` (one level of sub directories);
  - *each entry* shows its name, size and – when not verified – "unsigned" / "untrusted"; its page shows the version, the size, the signature state and the routes. The package is **not loaded** for the list: the manifest (id, name, version, date, credits, size; its signature is checked – cheap) and a summary of the package file (`ocw_world::summary::summarize`: header, tile count, the small sections `ROUT`, `REGN`, `TZID`; the terrain, the road graph, land use and buildings are skipped with seeks, so a 260 MB package takes milliseconds; summaries are kept per file by size and time). The full verification (hashes) happens when it is ridden; a package the list could not judge fails then, and the ride goes on with a message;
  - *duplicates:* a file reached twice is listed once (canonical path); packages with a manifest are one entry per id – the highest version, on a tie the one given with `--world`, then the cache, then a file (`world list` shows the others as "also"); packages without a manifest are told apart by their file, and two entries with the same name show their file names;
  - *broken and blocked packages* are listed with the reason and cannot be chosen: not a package, a version this build cannot read, truncated, a size that differs from the manifest, an unreadable manifest, a bad signature; an unsigned package in the cache without `--allow-unsigned`, an unsigned file with `--require-signed` (the trust policy of §4.7 / §12);
  - *parts:* packages whose regions meet at a border (boxes touching within 0.01°, less than 10 % of the smaller region shared – sampled on the region polygons) are **one world with parts**; riding one joins the installed neighbours anyway (§2.2.2). Two cuts of the same area (a test build next to the release) stay two worlds;
  - the **built-in test routes** (Hills, Loop) come last.
- **A world's page:** *Ride* (its default: the package's first route, else a free ride – as `--world` alone does, M3), *Free ride* (random start), *Free ride from here* (the world ridden now), *Routes* (of the world ridden now: Navi's list with length and climbing; of another world: its routes from the summary), *Parts* (a free ride in one of them), *Package*. Every row asks *Change the ride?* first and then runs the M3 in-app switch (`ride.switch …`): process, window and devices stay, the ride so far is saved.
- **About this world:** *World* (region name, signature state, parts), *Package* (id, version, build date), *Area* (km² of the regions, else of the tiles; the lowest and highest terrain), *Sun and time* (a page: the time zone with its UTC offset, the world time, sunrise / sunset with solar noon, civil, nautical and astronomical twilight at the rider's position on the world's local date, from `ocw-sky`, recomputed when the date or the place changes), *Data sources* (one row per attribution of the package and its parts, licence as the value; a source's page shows source, licence, web address and the **full notice** as its text – the same texts as the manifest's credits, which the package's `LICENSE-DATA.md` and the credits screen carry, §13), *Routes*. A test route has no package: the page says so and shows the sun.
- **Weather** (`apps/desktop/src/weather.rs`): the **weather machine** (Phase 9 S5, 9I-S5) over the session weather (since S4 `WeatherState` comes from the server like the world clock). Under the title the weather now (condition, temperature, wind, road) and who sets it; rows *Mode* (fixed ↔ simulated with the seed; "-> rain in 40 s" during a transition), *Real weather* (dimmed: coming with S7), *Presets* (a grid page), *Custom* (two pages), *Transition* (instant / 1 / 5 / 15 min), *Random weather* (← / → the seed, Enter applies it). Every row is a console line (`weather …`, `weather.transition`, `weather.seed`); rows the rider may not use are dimmed (`Activate::Disabled`).
- **Shared sessions (decided):** the server owns the session (§6.1), so in a shared ride the switch is a request to the server, and **only the host** may switch the world, checked on the server like the time machine: `ClientMessage.world` = `WorldRequest` (the package's manifest id – or the file name of a local build – and a route; empty: free ride), answered with a `DevCommandResult`; `ServerConfig.world_access` (`ClockAccess`: anyone on the local servers, the host by default, nobody for organizer-set sessions; `ocw-server --world-control host|anyone|nobody`), sent in `Welcome.world_access` so the menu can say who decides. The server resolves the request in **its own catalog** (`ocw_server::worlds::resolve`: its cache and the packages next to its `--world`; it never loads a path a client names), loads the package on a thread while the session goes on, then `GameServer::switch_world` starts a new session on it (bots again, the world clock goes on: session time is kept across the switch) and sends `ServerMessage.world_change` = `WorldChange` (manifest id, version, name, graph id, route, package SHA-256, who asked) to every rider. The riders leave the old session (their rider ids belong to it), and each client loads the same package from what it has installed (by id and version; the SHA-256 must match) and **joins again with a Hello on the same connection**, so the host stays the host; a client without the package says so ("The host rides … which is not installed here") and waits. Test routes and the rider's own route files are not offered to a shared ride. Local servers keep the client-driven M3 switch.
- **Console:** `world list` (numbered, with id, version, size, signature state, routes, source, parts, other copies), `world info` (the About page as text, with the full notices and the sun times), `world switch N|ID|NAME|PATH` (a number of `world list`, an id, `id@version`, a name or file name, a package path, or a test route; the world's default ride) – user commands, like `ride.switch`, with the same rules (asking the server in a shared ride).
- **Kiosk:** `scripts/deploy-test.sh` installs every world package built on the machine side by side in `/var/tmp/ocw/` (v2.36), so the kiosk's World → Change world lists Liechtenstein and the Black Forest – south, middle and north as **one world with three parts** (their regions meet on 48.12 N and 48.45 N) – and switches between them in the app; the kiosk's default ride (`ocw-desktop.args`, `OCW_KIOSK_WORLD`) is unchanged.
- **Not yet:** downloading packages from the list (Phase 5 backend), a map of the worlds and start places (Phase 7), the `LICENSE-DATA.md` file inside packages (the notices come from the manifest), the weather itself (S4 / S5), following a server's switch to a route file or a test route (the server rides packages only).

**Increments** (each with unit tests, §11.4; the menu model, settings and actions are tested without Bevy or with `MinimalPlugins` + `ConsoleCorePlugin`)

| # | Increment | Phase | Size | Tests |
|---|---|---|---|---|
| M1 | Input contexts, menu shell (Resume, Restart, Quit), Esc / Start, focus navigation for keyboard, mouse and gamepad, ten-foot theme; the ride continues under the menu (autodrive and default exits while it is open) | 3 (now) | M | navigation model (focus moves, wrap, page stack, Back at the top closes); key / button → action mapping; ride inputs ignored in `Menu`; Esc closes the console first and never quits; steering falls back to autodrive while the menu is open and is restored after; ride time, trainer and recording unaffected; headless app opens the menu and selects Resume; X11 + Wayland smoke screenshot of the menu |
| M2 | Settings registry, `settings.toml`, `set` / `get` commands; Settings pages Controls (options), Trainer, Strava, Display, Riders (profiles: name, weight via the join message and SIM mass, FTP, units), rider choice in the Workshop | 3 (now) | L | registry validation and ranges, TOML round-trip, precedence (file < autoexec < CLI, temp values not saved), unknown keys kept; each page item → expected command line; weight reaches the server's physics profile |
| M3 | Ride → World mode picker (installed packages, free ride / package routes / test routes, bots) as an in-app switch (done: see *Implemented (M3)*; bots per ride still open); Navi → Routes list, Rides list with Strava state and FIT export, Strava sync page | 3 (end) | M | package discovery on a fixture directory, `[ride]` choice → command line of the restarted process, ride list from fixture FIT + sidecar + `strava-sync.toml` |
| M4 | Navi → GPX import with map-matching report; GPX export of rides | 3 (with §2.7) | M | GPX fixtures through import → route; problem sections listed; GPX export round-trip |
| M5 | Game audio and Settings → Audio (mix, output device) (done: see §3.3 *Implemented (M5)*) | with game audio (see §3.3) | M | bus volume maths (master × bus), device selection fallback to the default device when the saved one is missing, settings round-trip |
| M6 | Workshop: bike and character pages with 3D preview; planned v2.53: the workshop scene, parts and hearts (§14 Phase 4, 4A–4K, milestones W1–W6) | 4 | L | configuration model and persistence (§14 Phase 4); cosmetic choices never change the physics profile; menu items → configuration; 4J |
| M7 | Accounts / profile sync, Events (sessions, join), backend Strava, route sharing; start-screen dashboard (§2.1) | 5 (calendar 7) | L | API client against a mocked backend, disabled-page states offline, event list and join flow |
| M8 | Voice settings (microphone device, microphone mute, speaker mute, voice volume, per-participant mute, spatial voice on / off) | 6 | S | device selection and mute state (§14 Phase 6) |
| M9 | Rebinding of keys and buttons, accessibility (text size, colour-blind safe HUD, Confirm / Back swap per pad) | 7 | M | binding conflicts, persistence, defaults restore |
| M10 | Input model for the whole UI (§3.2.2, v2.54): action mapper for keyboard, mouse, gamepad and touch, `ActiveInput` and device glyphs in a hint bar, touch mode (hit targets, gesture recogniser, emulated-mouse filter), ride touch overlay | 4 (with M6) | M | §3.2.2 tests: mapper, gestures, reachability of every page per device, hit-target sizes, overlay |

**M8 implemented (Phase 6, first increment):** Settings → Voice (`Page::Voice`, rows from `apps/desktop/src/voice/page.rs`): *Microphone* and *Speaker (voices)* toggles (`voice.mute-mic`, `voice.mute-speaker`), *Voice volume (%)*, *Participants (n)* (a sub page: each rider with "heard 40 %", "too far away", "microphone off" or "muted by you"; Confirm = `voice.mute IDENTITY`), *Devices and options* (a sub page: microphone and speaker – ← / → step through *default* and the present devices, a saved device that is missing shows "(not found: default)" –, *Spatial voice*, *Voices heard at most*) and *Voice chat* (on / off with the status). Settings: `voice.enabled`, `voice.mic_muted`, `voice.speaker_muted`, `voice.volume` (0–200 %), `voice.spatial`, `voice.max_voices` (1–20), `voice.input`, `voice.output` (device names; a new registry kind *text*). The Settings top page now has seven items (Riders, Controls, Trainer, Display, Strava, Voice, Back): the ten-foot maximum; Settings → Audio (M5) needs a slot (e.g. Strava, which is also under Navi, could leave Settings).

**M1 implemented:** Esc / Start opens the menu over the running ride: Resume, Ride, Workshop, Navi, Settings, Restart, Quit; Ride, Workshop, Navi and Settings are placeholder pages that say what their increment brings (Settings already toggles the FPS counter). Restart and Quit ask first, with *Cancel* focused, and run the console commands `reset` / `quit`; the console command `menu open|close|toggle|up|down|select|back` drives the menu from scripts and `--cmd`. Code: the Bevy-free model (`apps/desktop/src/menu/model.rs`: pages, page stack, focus with wrap-around), the key / button → action mapping with hold repeat (`menu/controls.rs`), the drawing of a `MenuView` in `ocw-render::menu` (text sizes in % of the window height). Deviations from the plan above:

- No Bevy state `UiMode`: one resource `InputContext { Ride, Menu, Console }` (console over menu over ride) is decided once per frame in `PreUpdate`, and the ride's input systems (power, steering, A / W / U / J / Q / E, R, V, F11, the mouse camera and C) run only in `Ride` (`run_if(ride_input)`); a key that closes the menu is therefore not also seen by the ride in that frame. The console keeps its own open flag.
- Steering while the menu is open (`ClientCore::hold_steering` / `release_steering`, so server and prediction agree): wild mode off, autodrive on – from manual steering at the rider's current lateral position (no swerve), an autodrive line stays where it was; the next junction's choice is reset to its default exit and not restored on closing. Closing restores manual steering upright (the lean is not carried over) and wild mode; autodrive keeps its line. The HUD shows "autodrive (menu)".
- The mouse hover moves the focus only while the mouse moves (rows of a new page appearing under a resting pointer do not take the focus); right click = Back.
- ← / → and tab switching (Q / E, PgUp / PgDn, LB / RB) had no rows or tabs to act on in M1; M2 (v2.7) uses them on the Settings sub pages and the Workshop (see *Implemented (M2)* above). The FPS toggle moved to *Settings → Display*.
- The status strip shows power, speed and the device / trainer / steering status line; the Strava sync state joins it with the Navi pages (M3).
- Smoke screenshots of the open menu are taken on Wayland (1280×720 and 800×600); the build machines with a GPU have no X11 server.

**Open questions**

Decided (v2.2): no pause (trainer unchanged, ride time and recording continue); Esc never quits; rider weight affects physics, riders managed in Settings and chosen in the Workshop. Decided: local events before the backend (§2.3.1) - the Ride page starts a race, a group ride or a time trial on the current route (console command lines `event race --bots 5`, `event group --watts 150`, `event tt`, `event cancel`; the starts ask first, then the menu closes for the countdown). Still open: switching rides by restart vs. in-app session switch (M3).

#### 3.2.2 Input Model: Keyboard, Mouse, Gamepad and Touch (planning, 2026-11-13, v2.54)

**Rule (decided by the user, 2026-11-13):** the **entire UI** – every menu page, the workshop scene (§14 Phase 4, 4A), the character corner, dialogs, the console's menu-side functions, the HUD's interactive elements and later the start screen – is controllable with **gamepad and mouse, and touch where the device has a touchscreen**; the keyboard keeps working everywhere (§3.2.1). No function may need one particular device (the console's text entry is the one exception: it needs a keyboard, physical or on-screen). The rest of §3.2.2 is a proposal.

**Platform facts (Bevy 0.20.1 / winit 0.30.13, as read in their sources 2026-11-13):**

| Input | Linux X11 | Linux Wayland | Windows | macOS | Notes |
|---|---|---|---|---|---|
| Keyboard, mouse, wheel | yes | yes | yes | yes | trackpad two-finger scroll arrives as `MouseWheel` (pixel deltas) |
| Gamepad (gilrs 0.11) | yes (evdev) | yes | yes (XInput / WGI) | yes | hot-plug; `Gamepad::vendor_id()` / `product_id()` tell Xbox / PlayStation / Switch / generic apart for the glyphs |
| Touchscreen (`TouchInput`, `Touches`) | yes (XInput 2.2 touch; winit drops the emulated pointer events) | yes (`wl_touch`) | yes (winit does **not** filter the mouse events Windows synthesises from touch: we must) | **no touchscreens** (winit: touch unsupported) | multi-touch with ids; no built-in pinch on Linux / Windows – our recogniser computes it from the touches |
| Gestures (`PinchGesture`, `RotationGesture`, `DoubleTapGesture`, `PanGesture`) | – | – | – | pinch, rotation, double tap from the trackpad | `PanGesture` only on iOS; on macOS pan comes as scroll |
| Picking (`bevy_picking`) | mouse and touch are both *pointers* (`PointerId::Mouse`, `PointerId::Touch(id)`): UI and mesh picking (hotspots) work for both | | | | the menu rows already use `PointerClick` / `PointerOver` (`ocw-render::menu`), so taps reach them today (untested) |
| Focus (`bevy_input_focus`) | `InputFocus`, tab navigation, directional navigation (`AutoDirectionalNavigation` in `bevy_ui`) | | | | usable for the Bevy UI pages; the menu model keeps its own focus (§3.2.1) and may adopt it |

Devices that have all of them: Windows tablets and touch laptops, Linux touch laptops, the Steam Deck (touchscreen + gamepad, Linux), a kiosk with a touch monitor. Browser (Phase 10): touch, pointer and Gamepad API all available; phones / tablets are touch-first. VR (Phase 11): the controller ray is one more pointer.

**Abstract actions (one layer for all devices):** today's menu actions (`Up`, `Down`, `Left`, `Right`, `Confirm`, `Back`, `TabNext`, `TabPrev`, §3.2.1) are extended by `Secondary` (context: compare, details), `Tertiary` (a page's extra action: buy, sort), `Orbit(Δ)`, `Zoom(Δ)`, `Pan(Δ)`, `Point(position)` and `Activate(position)` (click / tap at a point) and `Scroll(Δ)`. A pure mapper in `apps/desktop/src/input/` (Bevy-free, with the existing hold-repeat of `menu/controls.rs`) turns device events into actions; pages and scenes see only actions, so every page and scene can be tested once with actions and once per device.

| Action | Keyboard | Mouse / trackpad | Gamepad | Touch |
|---|---|---|---|---|
| Move focus | arrows, Tab / Shift+Tab | hover (moves focus only while the pointer moves, §3.2.1) | D-pad, left stick (repeat) | – (no focus frame; tap acts directly) |
| Confirm | Enter, Space | left click | South (A / ✕) | tap |
| Back | Esc, Backspace | right click, mouse "back" button | East (B / ○) | the on-screen **Back** button (top left, always shown in touch mode), swipe from the left edge inward (not the OS edge: starts 16 px in) |
| Change a value | ← / → | drag the slider, wheel over it | D-pad / stick ← / →, LB / RB big steps | drag the slider, tap the ‹ › arrows |
| Switch tab / category | Q / E, PgUp / PgDn | click the tab | LB / RB | tap the tab, swipe horizontally on the tab bar |
| Secondary (compare, details) | C | middle click, or the card's button | North (Y / △) | long press (500 ms) |
| Tertiary (buy, sort…) | X | the card's button | West (X / □) | the card's button |
| Scroll a list | arrows, PgUp / PgDn | wheel, drag the scroll bar | right stick ↕, focus moves | one-finger drag (with inertia) |
| Orbit (3D scenes) | I / J / K / L, or Shift + arrows | left drag (in empty space), trackpad drag | right stick | one-finger drag |
| Zoom | + / − | wheel, trackpad pinch (macOS gesture) or two-finger scroll | RT zoom in, LT zoom out (analog) | pinch |
| Pan | Ctrl + arrows | middle drag, Shift + left drag | right stick click + right stick | two-finger drag |
| Reset view / focus | Home | double click | right stick click (R3) | double tap |
| Open / close menu | Esc | the ☰ button (top left in a ride, shown after a mouse move) | Start / Menu | tap ☰ |

**Last-used device and hints:** a resource `ActiveInput { Keyboard, Mouse, Gamepad(kind), Touch }` follows the device of the last meaningful input (a key, a click, a stick beyond the dead zone, a touch; mouse motion of less than 4 px is ignored, so a nudged table does not flip it). The **hint bar** at the bottom of every page and scene ("Ⓐ Select  Ⓑ Back  Ⓨ Compare  LB/RB Category") shows the glyphs of that device: keycaps, mouse icons, Xbox letters (also for generic pads), PlayStation shapes, Switch letters; touch shows short words ("Tap to select · Long press: compare"). The glyphs are our own drawings (Phase 8 Q11: no logos; button letters and shapes are not logos). **Confirm is the bottom face button on every pad** (positional, as today: Bevy's `South`), so on a Switch pad the hint shows "B" for Confirm – one rule for all pads; a per-pad swap is a later accessibility setting (M9).

**Focus frame vs. pointer:** with keyboard or gamepad active the focus frame is shown (ten-foot rule, §3.2.1); with mouse or touch active it is hidden and hover / press states show instead; the focus is kept and returns on the next key or button. Pointer and focus never fight: a click or tap also moves the focus there.

**Touch mode** (when `ActiveInput` is `Touch`, or the setting `controls.touch on|off|auto`, default `auto`):

- **Hit targets:** at least **9 mm** and at least **48 logical px** (whichever is larger; 44 px is the bare minimum) high and wide for every interactive element, 2 mm (≥ 8 px) apart. The ten-foot layout (text ≥ 3 % of the window height, rows of ≥ 6 %) already gives 43 px rows at 1280 × 720; in touch mode rows grow to the minimum where the window is smaller, and pages page earlier (§3.2.1 paging) rather than shrink. Millimetres from the window's scale factor (1 logical px ≈ 0.19 mm at scale 1 on a 96 dpi panel; winit gives no physical screen size, so a setting `display.touch_scale` corrects odd panels).
- **Gesture recogniser** (`apps/desktop/src/input/gestures.rs`, pure, fed with `TouchInput` events and their times): tap (< 10 px movement, < 300 ms), double tap (< 300 ms apart, < 20 px), long press (500 ms without moving), drag (beyond 10 px), swipe (a drag faster than 0.5 px/ms, released), pinch (distance of two touches: zoom factor), two-finger drag (centroid: pan), two-finger tap (= Back in 3D scenes). A third touch cancels. On macOS the trackpad's `PinchGesture` / `RotationGesture` / `DoubleTapGesture` map onto the same actions.
- **Emulated mouse events:** Windows sends a synthesised mouse event for the primary touch; winit 0.30 passes it on. The mapper ignores mouse buttons and motion within **500 ms** of a touch event at the same place (X11 already filters them in winit), so one tap is never also a click. (To check on a real Windows touchscreen.)
- **Text:** text fields (names, titles) open the OS on-screen keyboard where the platform offers one through IME (Windows touch keyboard, GNOME / KDE on Wayland), else our own on-screen keyboard (the one planned for the gamepad, §3.2.1).
- **System gestures:** the OS edge swipes (Windows, GNOME, KDE) are left to the OS; our gestures start at least 16 px from the window edge.

**Mouse:** everything that has a focus has a pointer equivalent (no hover-only function, §3.2.1); right click = Back; the wheel scrolls lists and zooms 3D scenes; in a ride a mouse move shows the ☰ menu button and the clickable HUD elements for 3 s.

**Gamepad:** the full menu and every scene are reachable (D-pad / left stick focus, South / East / North / West, LB / RB, LT / RT analog, right stick for scenes and long lists, R3 reset, Start menu, Select = the voice mute of §2.4 in a ride); several pads: any pad drives the UI (the ride's steering rule stays: the largest deflection steers).

**During a ride (touch proposal):** riding is driven by the trainer; touch is for the HUD and the camera. A **touch overlay** (shown on the first touch, fades after 8 s without one; `controls.touch_overlay auto|on|off`):

- ☰ menu (top left), **autodrive** toggle, **U-turn** (asks with a second tap, as the key does not), **lean ‹ ›** buttons at the lower left and right edges (held = lean, as the ← / → keys; only shown when autodrive is off), and at a junction the **exit arrows** of the junction overlay (§2.2.1) become tap targets (tap = choose that exit).
- **Camera:** one-finger drag on the 3D view orbits (as the mouse), pinch changes the chase distance, double tap = camera reset (C), two-finger tap cycles the camera mode.
- **Tap HUD elements:** the voice indicator (mute the microphone / speaker, §2.4), the mini elevation profile (zoom 1.5 km ↔ 5 km), the bike computer (cycle its fields), the event panel (expand the standings), the damage widget (Phase 12: the part list), the REC dot (stop recording).
- **Virtual power** (no trainer, §2.9): + / − buttons beside the power field.
- The overlay is part of the HUD layer (hidden with `hud off`, drawn in captures only when shown).

**Workshop and other 3D scenes** (the workshop, the world preview / map later, Phase 13's view later): orbit, zoom and pan as in the table, with limits per scene (workshop: 0.8–4 m around the stand, pitch −10° … 70°), inertia after a flick (0.3 s), and the same camera state machine whatever the device (§14 Phase 4, 4A).

**Settings → Controls** gains: *Touch mode* (auto / on / off), *Touch overlay* (auto / on / off), *Pointer speed* (orbit speed, existing), *Invert orbit*, *Stick dead zone* (existing), *Gamepad glyphs* (auto / Xbox / PlayStation / Switch). Rebinding stays M9 (Phase 7).

**Tests (spec §11.4: no step without tests):**

- **Mapper** (pure): every row of the table above, per device, → the expected action; hold repeat; dead zones; `ActiveInput` switching (and not switching on 1–3 px mouse jitter); emulated-mouse suppression after touches (a tap is one `Activate`, never two); glyph sets per gamepad vendor id.
- **Gesture recogniser** (pure, synthetic `TouchInput` sequences with times): tap / double tap / long press / drag / swipe thresholds, pinch factor, two-finger pan, cancellation, three-finger cancel.
- **Reachability on every page** (headless app with `MinimalPlugins` + `ConsoleCorePlugin` + the menu and workshop models, input injected as Bevy input messages `KeyboardInput`, `MouseButtonInput` / `CursorMoved`, `TouchInput`, gamepad events): for **every page of the menu tree and every workshop / character state**, every focusable item can be reached and activated with the keyboard alone, the gamepad alone, the mouse alone (click at the item's laid-out rectangle) and touch alone (tap at its rectangle); Back works with each; no page needs a second device.
- **Hit-target sizes:** in touch mode every interactive rectangle on every page is ≥ 48 logical px and ≥ 9 mm at 1280 × 720 and 800 × 600, scale factors 1, 1.5 and 2, with ≥ 8 px gaps (extends `every_menu_page_fits_a_16_9_window_with_its_text`).
- **3D scenes:** orbit / zoom / pan reach the same camera states with each device; limits hold.
- **Ride overlay:** taps on lean / autodrive / U-turn / exit arrows produce the same `ClientCore` inputs as the keys; overlay hidden with `hud off`.
- **Screenshots:** the hint bar for each device and the touch overlay, X11 + Wayland (scene harness).

### 3.3 Audio

Two separate audio systems:

**Game audio** (own audio module, decided in M5): ambient sounds, bicycle sounds, wind, tyres, freewheel, gear shifting, UI and event cues; background music later.

**Implemented (M5):** `apps/desktop/src/audio/`, Bevy-free except its plugin (`mod.rs`).

- *Options evaluated:* (a) `bevy_audio` (rodio) – plays only on the default device, no device list, no bus mixer: rejected (it is now disabled in the app). (b) `kira` 0.12 – a good game mixer (tracks, tweens, spatial), but it links **cpal 0.18** while Bevy 0.20 links cpal 0.17, so the app would carry two cpal copies (two ALSA bindings), and it is built around sample files we do not have; its device choice is no better than cpal's. (c) **cpal directly with an own small mixer** (chosen): cpal 0.17 is already in the lock file through Bevy (Apache-2.0, no new system library: `libasound` is linked anyway), it lists and opens devices by name on ALSA / WASAPI / CoreAudio, and since every sound is synthesised in code a sample-playing layer (rodio) is not needed – the mixer renders straight into the device buffer at the device's rate, channel count and sample format (f32, i16, u16, i32). If music files come later, rodio's decoders (or symphonia) can feed the music bus.
- *Engine:* an `OutputBackend` trait (list devices, default device, open by name, close, stream failed) with the cpal backend, a `NullBackend` (no sound) and a mock for tests. The engine runs on its own thread (cpal streams are not `Send` on every platform) and talks to the game through channels: commands (output, refresh) one way, a status (device list, default, playing device, fell back, message) the other; the mixer gets its input (ride parameters, cues, bus gains) through a bounded queue, read at the start of each device buffer (cues older than 0.4 s are dropped, so nothing stale plays after a device came back).
- *Device choice:* `audio.output` is a device name (cpal's device description, e.g. "USB Audio Device, USB Audio" on ALSA) or `default`; matching ignores case and surrounding spaces; several devices with one name (ALSA lists a card as `hw:`, `plughw:`, `sysdefault:`) are tried in order until one opens. A missing device falls back to the default device (shown as "NAME (gone)" on the Audio page). **Hot-plugging:** the device list is re-read every 3 s (cheap: cpal's ALSA listing does not open devices); a device that comes back is opened again, a stream that fails (unplugged: cpal's `DeviceNotAvailable`, `StreamInvalidated`, backend errors; underruns are ignored) is closed and the default opened. A failed open is retried after 15 s. Without any device (headless Xvfb / weston runs, CI) the app runs silently with one log line ("audio: no audio output: …"); `--no-audio` and frame capture (`--capture`, game time not real time) use the null backend (the mixer still runs for recordings, below).
- *Sound in videos (decided 2026-11-12, v2.30):* recorded videos carry the game mix (`audio::record`), muxed as AAC 192 kbit/s, 48 kHz stereo. **Live recording** (V, console `video`): while a device plays, the mixer copies every buffer it renders for the device to the recorder (a tap: exactly what the rider hears, in step with the real-time video); without a device (`--no-audio`, headless) the game thread renders the mixer itself, as many samples as game time has passed. The samples go to a hidden WAV beside the hidden H.264 file; after the stop ffmpeg muxes both into the final MP4 (`-c:v copy -c:a aac`), else the picture alone is kept. **Frame capture** (`--capture`): a capture source pulls exactly `frame × rate / fps` samples (integer accounting, no drift; 1920 per frame at 48 kHz and 25 fps) from the mixer per rendered frame – also during the warm-up, so the sound is settled, but only the captured frames are written to `DIR/audio.wav` – so picture and sound match whatever the render speed, with no audio device; the scripts mux it (`drive.sh`). Options considered: recording the system's output (PulseAudio monitor, WASAPI loopback: platform-specific, records other apps too, impossible headless), an offline backend behind the `OutputBackend` trait that opens like a device (the engine thread would drive it on its own clock, not the frame's), **pulling from the shared mixer in the game thread (chosen)** for capture and the tap for live. Voice (LiveKit) plays through its own path and is not in the recordings yet.
- *Buses:* master × music / effects / voice, and the other riders as a part of the effects (master × effects × others); each volume is 0–100 % on a squared curve (gain = (v/100)², 50 % ≈ -12 dB), the bus gain is master gain × bus gain, mute sets all to 0; gains are smoothed (60 ms) so changes do not click; a soft limiter (linear to 0.8, tanh knee) keeps the sum within ±1. Pure and unit-tested (`mix.rs`, `mixer.rs`).
- *Sounds (all procedurally synthesised, our own work – no sample files, nothing to license, §13):* **wind** – pink noise, low-pass (250 Hz + 120 Hz per m/s) plus a resonant band, level ∝ (v / 54 km/h)^1.6 with slow gusts; **tyres** – rolling rumble on every surface, louder on gravel; gravel adds random grit bursts (6 per metre ridden), grass a high swish (surface from the track and the rider's lateral position, §4.4 / §5.2.1, as for the physics); **freehub (freewheel)** – the clicking is heard **only while the wheel turns and the rider does not drive it** (coasting): a short metallic click per pawl engagement, 24 engagement points per wheel revolution (a typical road hub; common hubs have 18–54), so ticks/s = 24 × speed / 2.1 m wheel (about 95/s at 30 km/h), jittered, level kept even as the ticks become a buzz. The rider drives the wheel – and the freehub is **silent (gain exactly 0)** – when the power is above 10 W (whatever the cadence reads: no sensor, a dropout, a power-only trainer), or when the cadence is at least 15 rpm and the top gear (50 × 11) at that cadence reaches the speed; turning the legs slower than that (spinning out downhill without power) lets the wheel overrun and tick. Stopped (< 0.3 m/s): no ticks. Transitions: the ticks fade in over about 0.25 s when pedalling stops (time constant 80 ms, no burst) and are gone within about 30 ms when it starts again (8 ms); other parameters glide with 60 ms. **drivetrain** – while pedalling, a soft continuous whir (broad band noise around 350 Hz + 5 × the chain's tooth rate, low-passed), a faint hum at the chainring tooth rate (cadence × 50 teeth / 60) and a gentle swell per pedal stroke, a little louder under load – no clicks (unit-tested: its 10 ms loudness varies little, the freehub's a lot); without a cadence sensor, or with a sensor reading 0 under power, the cadence is estimated from the power (75–100 rpm); **cues** – menu move / value / page open / back, countdown beeps (3, 2, 1) and a higher GO, a rising finish chime (results panel of a race or time trial), a two-tone workout step cue (each start, step change, end, stop), a gear-shift double click (`audio.play shift`; not triggered automatically yet: the protocol has no gear) and a test tone per bus. The mapping ride state → sound parameters (`ride.rs`) and the cue triggers (`cues.rs`: from the `MenuView`, the event countdown text, the results panel, the workout's change counter) are pure and unit-tested; a scripted 20 s demo ride renders offline through the whole mixer (`audio::demo`, `OCW_AUDIO_DEMO=FILE.wav` writes it) in separated sections: 0–6 s pedalling, 6–10 s coasting (freehub), 10–14 s pedalling again without a cadence sensor, 14–17 s pedalling on gravel, 17–20 s coasting on grass.
- *Other riders (decided 2026-11-13, v2.33):* every other rider – server bots and remote riders – makes the own bike's sounds (`audio::others`, placement in `audio::spatial`, both pure and unit-tested): the freehub only while coasting (§3.3 freehub rule, 2.21), the drivetrain whir, the tyres on their surface (from their track and lateral offset); no wind (that is the listener's own air). The snapshot carries their power but no cadence, so the power decides: above 10 W they pedal (at their own cadence, below), else they coast; a cadence field in `RiderUpdate` would refine it later. The **time-trial ghost stays silent**: it is a replay of the rider's own effort, not someone on the road, and a silent ghost does not mask the real field. *Placement:* the listener is the **own rider's head** (1.5 m above the road, facing the direction of travel; the chase camera's position is not used, so the scene sounds the same in every camera mode), the sources sit at the bikes (0.5 m). Level: 1 up to 2 m (a wheel beside the own), then inverse distance (roll-off 1: -6 dB per doubling), faded smoothly (smoothstep) from 50 m to exactly 0 at 70 m – beyond that a rider costs nothing. Stereo: equal-power panning by the direction in the horizontal plane relative to the facing (centre = full level on both channels), narrowed to the centre within 1 m; behind the head 25 % quieter and duller (head shadow; stereo cannot place front and back otherwise). Air absorption: a low-pass at 16 kHz × 10 / (10 + d) (8 kHz at 10 m, 2.3 kHz at 60 m – stronger than the physical absorption, as a distance cue). Doppler: half the physical shift, at most ±3 % (a passing rider is a touch higher, then lower; nobody wails). *A group, not a chaos of clicks:* each rider has a **character derived from the rider id** (splitmix64: the same on every run and client): hub engagement points from {16, 18, 24, 24, 30, 36, 54}, wheel 2.08–2.18 m, cadence 78–96 rpm, level 0.8–1.2, click band 3.0–5.8 kHz, ring 4.2–6.6 kHz, click decay 0.8–1.5 ms, drivetrain and tyre tuning ± 20 %, own noise seeds and start phases of ticks and pedal strokes. **Voices:** the nearest 8 riders get their own synthesis (a rider already playing counts 0.8 × its distance, so two at about the same distance do not swap); up to 48 more of the audible field are summed into **one group layer**: decorrelated rolling and whir noise left and right at the energy sum of their levels per channel, and one random stream of ticks at the coasting riders' summed rate, each tick taking a rider (by rate) for its level, pitch and side; riders leaving a voice fade out, new ones fade in, at most 16 voices live. Another bike at 2 m plays 6 dB under the own one (that sits about 1 m from the ears). A **crowd normaliser** scales all other riders together when their energy (Σ gain²) exceeds 2 riders at 2 m, so a bunch of 30 stays clear of the limiter (unit-tested: 30 riders around the listener peak below the knee at full volume). *Mixer:* the mix is now **stereo** – the own bike and the cues in the centre, the other riders placed; a mono device gets the average, channels beyond two the centre; the device tap and the recordings carry the stereo mix, so `--capture` and live videos have the other riders too. The game thread computes a frame of targets (voices, group) from the `RideView` every frame and sends it through the mixer queue. Volume: bus *others* = master × effects × others (`audio.others`, 80 %), on Settings → Audio as "Other riders" (Enter plays `test-others`); **Music leaves the Audio page** (seven items at most, §3.2.1) until there is music – `audio.music` and `audio.volume music` stay. Voice chat stays separate (LiveKit, own path, §7).
- *Settings and commands:* `audio.master` (80), `audio.effects` (80), `audio.others` (80), `audio.music` (60), `audio.voice` (100), `audio.mute` (off), `audio.output` (`default`; a new setting type *text*) in `[audio]` of `settings.toml`, live; console `audio.volume BUS PERCENT` (master, effects, others, music, voice), `audio.mute [on|off|toggle]`, `audio.output NAME|default`, `audio.devices`, `audio.status`, `audio.play SOUND` (also `test-others`).
- *Uncertain / open:* the levels and timbres are set from the formulas and checked by measurement (levels, ranges) only – nobody has listened yet: the demo WAV and the kiosk are for that; device names are cpal's descriptions, which are not unique on every system (the first that opens wins) and may change with a driver update (then the default plays); the WASAPI and CoreAudio paths are build-checked but not yet heard; the other riders' sounds are heard in tests and captures only so far (levels from the formulas; bots never coast, so their freehubs are silent until a human rider coasts or bots learn to), no drafting sound, no music, no ambience (birds, traffic), no ducking under the menu; the freewheel is synthesised for a road hub – a trainer's own freewheel noise in the room is not compensated.

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

- `ocw-sim`, `ocw-protocol`, `ocw-world` and `ocw-core` must **compile to `wasm32-unknown-unknown`** (since Phase 10 B1 also `ocw-sky`, `ocw-voice` without `livekit`, `ocw-devices` – without btleplug and the threaded virtual trainer there; since B4 its protocol and runner drive Web Bluetooth there – and the `ocw-server` lib – without QUIC, world files and the server thread – which the browser app uses). They may not use threads, the file system or sockets directly; those go through traits. A CI check builds them for wasm from Phase 1.
- The device layer stays behind `DeviceBackend` (§3.4), so Web Bluetooth (web; built in Phase 10 B4: `WebBluetoothBackend`, the same protocol over a `GattLink` on the browser's objects) or CoreBluetooth (iOS) can be added as further backends.
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
- **Terrain under the road:** within the road width plus 1.5 grid cells the terrain is cut down to just below the road surface (0.3 m), so the 10 m grid cannot interpolate it through the road on side slopes; further out it blends into the DEM over 60 m. (v2.40: with the world cleaning each road acts through its nearest point only, the cut is the road's half width plus 0.5 m, and the grid is settled under the strips afterwards, §4.8.3.)
- **Checks:** `world-builder check` measures where the terrain rises above a road strip (sampled every metre along and 0.25 m across, with the same triangle split as the mesh); `world-builder graph --probe X,Z` lists the roads near a point with lengths, end heights and gradients. The pilot package's test requires < 0.1 % of road samples under terrain.

**High-resolution terrain models (v2.35, decided Phase 8 Q9):** the official open DTMs replace SRTM wherever they exist; SRTM (a 30 m *surface* model: tree tops in forests) remains the fallback.

| Region | Model | Resolution used | Licence / attribution | Format |
|---|---|---|---|---|
| Baden-Württemberg (Black Forest packages) | **LGL DGM1** (airborne laser scanning, captured 2016–2022) | 1 m, averaged to 2 m on import | Datenlizenz Deutschland – Namensnennung 2.0: "Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0" (modified) | XYZ text in 2 km zips (4 × 1 km), ETRS89 / UTM 32N |
| Liechtenstein, Switzerland (pilot) | **swissALTI3D** (swisstopo, newest year per tile, 2019–2026) | 2 m | swisstopo OGD, free use incl. commercial, source "© swisstopo" | Cloud-optimised GeoTIFF, 1 km tiles, LV95 |
| Tenerife (Canary Islands) | **IGN MDT05** (first PNOA LiDAR coverage, the islands flown 2009–2011) | 5 m | CC BY 4.0 (IGN licence, Orden FOM/2807/2015): "MDT05-cob1 2008-2015 CC-BY 4.0 scne.es" (derived work) | Cloud-optimised float GeoTIFF from the IGN's OGC API Coverages (`api-coverages.idee.es`, collection `EL.ElevationGridCoverage_4083_5_C`), requested in 2 km tiles, REGCAN95 / UTM 28N |
| elsewhere (Austria, France) | SRTM 1″ | 30 m | public domain | HGT |

- **Import:** `scripts/fetch-pilot.sh` downloads only the tiles of the package boxes plus 0.04° (`world-builder dem-tiles` lists them; DGM1 1,908 zips of 2 km, 127 of them outside Baden-Württemberg; swissALTI3D 560 tiles from the STAC API) with a licence table (`SOURCES.md`, capture years from LGL's metadata). `world-builder dem-import` turns the 1 m XYZ into 2 m GeoTIFFs (block means; partial tiles at the Rhine padded with voids to whole kilometres): 7,053 tiles, 4.1 GB instead of 27 GB of zips.
- **Reading (pure Rust):** GeoTIFF with the `tiff` crate (LZW, Deflate; tiled; a tile-by-tile LZW fallback for cloud-optimised files with empty "sparse" tiles or no end code, as some swissALTI3D 2026 tiles have, which GDAL reads); XYZ text; projections in `ocw_worldgen::crs`: UTM with Krüger's series (millimetres, tested against PROJ) and swisstopo's approximate LV95 formulas (≤ 1 m). Vertical datums (DHHN2016, LHN95, EGM96) differ by well under a metre here and are not converted.
- **Layers:** tiles of one CRS, cell and size form a layer on one lattice (bilinear sampling across tile edges); the first layer covering a point wins, and over **300 m at the edge of its coverage** (state border, voids, end of the downloaded tiles) it is blended into the next source by a smoothstep weight, so a bare-earth DTM meets SRTM without a cliff. Only tiles near the region are read (`--dem`, `--dem-cell M` averages further when memory is short).
- **Tenerife (v2.42):** the IGN's coverage API returns cells whose *centres* lie on whole multiples of 5 m (corners at x.5 cells, 401 × 401 for 2 km, the GeoTIFF's projected CRS user-defined with ProjectionGeoKey 16028 = UTM 28N). `dem-import` moves such grids onto the lattice by bilinear interpolation (half a cell; 400 × 400 cells per 2 km tile), reads the user-defined UTM key, and the open-sea tiles without any data are dropped (755 of 1,927 kept, 483 MB in the builder). REGCAN95 agrees with ETRS89 / WGS 84 within a metre and is treated as the same (`--crs regcan95`, EPSG:4083). The 2 m **MDT02** of the second coverage (Canary Islands 2015–2016, CC BY 4.0 "MDT02-cob2 2015-2021 CC-BY 4.0 scne.es") exists too, but only from the CNIG download centre, whose direct downloads are guarded against automated clients; the package's 10 m grid gains little from 2 m over 5 m, so MDT05 is used and MDT02 is left for a manual download later. GRAFCAN's open data has only a 25 m MDT.
- **Resolution decision:** the DTM is sampled at 2 m (the road profiles every 5 m, the terrain vertices), the package terrain grid stays **10 m**. A 5 m grid was measured on the pilot: package 30 → 93 MB, build 4 → 9 s, client +0.1–0.2 GB, road samples under the terrain 0.001 % → 0 %; for a Black Forest package it would triple 200–260 MB packages beyond the memory budget (§15), so it does not pay off yet. Tile streaming from disk (§4.7) would allow it later.
- **Measurements** (marge, release build; "under terrain" = `world-builder check`, road samples the terrain pokes through; DTM − SRTM = `world-builder dem-diff` on a 50 / 100 m lattice where the DTM covers):

| Package | Terrain | Size | Build | Builder RSS | Road samples under terrain | DTM − SRTM |
|---|---|---|---|---|---|---|
| Liechtenstein | SRTM | 30.2 MB | 4 s | 0.2 GB | 296 of 18.2 M (0.002 %), worst 0.25 m | – |
| Liechtenstein | swissALTI3D 2 m | 30.2 MB | 4 s | 0.8 GB | 117 (0.001 %), worst 0.16 m | mean −4.9 m, RMS 12.1 m, 5–95 % −23.6 … +6.6 m |
| Black Forest south | SRTM | 259.9 MB | 165 s | 4.3 GB | 8,818 of 333 M (0.003 %), worst 4.7 m | – |
| Black Forest south | DGM1 2 m | 259.9 MB | 206 s | 7.6 GB | 5,640 (0.002 %), worst 3.8 m (the same track at the western border) | mean −7.3 m, RMS 11.4 m, 5–95 % −22.4 … +4.0 m |

  The DTM is lower on average because SRTM sees the forest canopy; the roads (the elevation reference) follow the real ground now: Freiburg – Schauinsland climbs 995 m instead of 1,055 m. Package sizes do not change (same 10 m grid); the builder holds the 2 m grid of the region (3.3 GB for the southern package).

Smoothing of the gradient sent to the trainer is covered in §5.3.

### 4.4 Road Reconstruction and Road Graph

Each road is converted into a 3D road segment with these properties: geographic centerline, road width, surface material, gradient, curve radius, road marking, connectivity information and road category.

The road surface is generated from the line geometries.

**Surfaces (v1.9):** asphalt, gravel and grass (off the road), with a rolling-resistance factor each (§5.2.1). Without an OSM `surface` tag, tracks and paths are gravel and all other classes asphalt; the `surface` tag refines this later. Pedestrian streets open to bikes (`highway=pedestrian` with `bicycle=yes|designated|permissive`) are residential streets (paved, residential width), not paths (v2.24, Freiburg's old town).

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
- **Resident tiles (v2.99, the browser app, §14 Phase 10 B2):** a streamed package keeps only the decoded tiles around the rider in memory (`ocw_worldstore::stream::resident`): tiles within a fetch radius (default 3 rings, so the renderer's 5 × 5 plus one) are fetched, tiles beyond a keep radius (default 5) are dropped, and over the memory budget (default 96 MB of decoded tiles) the farthest beyond the fetch radius go first. The renderer gets the changed package through `WorldSet::update_tiles` (the tiles already meshed stay; the far terrain is rebuilt from the resident tiles).
- **Jumps (decided and implemented):** when the rider moves more than 200 m between two frames (a teleport, the start of a ride), the renderer drops the tile work for the old place and builds the 3 × 3 tiles around the new position at once – their tasks run in parallel on the task pool and that frame waits for them (tens of milliseconds, a hitch only on the jump itself) – and uploads them in the same frame; the outer ring streams as before, one tile per frame. Before, every tile was built in the background and uploaded one per frame, so for about half a second after a teleport only the sky and the route's centre line were drawn. Considered: fading or holding the camera until the tiles are there (the jump would still show an empty world for that time). Regression test: `after_a_teleport_the_near_tiles_are_there_in_the_next_frame` (`ocw-render`).
- **Far terrain (v2.56).** Beyond the streamed 5 × 5 tiles every tile of every resident package is drawn as a coarse mesh (`ocw-render::far_terrain`): every 10th sample of the 10 m grid (100 m; the largest divisor of the grid's cells not above 100 m), vertex colours from the land use averaged over the far cell (the class colours of the streamed tiles – the regional palette tinted by the region look at the world date – and the slope rock rule of the colour map; the far tiles have no trees, so each class is tinted towards the crowns' colour by its tree cover: a forest far away is dark green, not its brown forest floor), skirts 40 m deep along each tile border for the cracks to the full tiles, double-sided, no shadows. A far tile is hidden exactly while its full tile is loaded (no z-fighting, no coarse triangles poking through the 10 m terrain). It is built once per world set on a background task (Tenerife: about 3,000 tiles of 121 vertices, 0.6 M triangles in all, frustum-culled per tile). With a package the camera's far plane is 120 km (Bevy's default of 1 km, with the fog ending at 1.8 km, had hidden everything beyond the near tiles: even with its tiles in the package the Teide 7 km ahead was not drawn), and the linear fog runs from 250 m to the end of the terrain around the rider (the border of the box of all tiles, at least 1.8 km, at most 60 km); a package with sea (water at 0 m in its land use) has no such end: a sea plane at −5 cm (under the tiles' water, above their carved sea floor) reaches 100 km beyond its tiles, and the fog ends at 60 km. Route worlds keep the 1 km far plane and the 250–1,800 m fog. Regression test: `far_tiles_cover_the_package_except_under_the_loaded_tiles`.
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
4. **Optional orthophotos:** high-resolution orthophotos only for selected regions, used for the measured palettes and roof colours at build time (Phase 8 T3) and as an **orthophoto corridor** of ±50 m along the roads that colours the ground near the rider (Phase 8 T7, below); never as the close-up texture.
5. **Vegetation:** instanced tree, bush and grass models placed by land cover and density, so the 3D look comes from geometry rather than textures.
6. **Roads:** material from the OSM `surface` tag (asphalt, gravel, paving stones, …), with a fallback per road category. Markings are generated from `lanes` and the road category.

**Why satellite imagery is not the main ground texture:**

- It is a top-down photo with baked-in shadows, cars, tree crowns and seasonal colors. Seen from a bike at 1.7 m height it looks blurry and flat.
- At 20 cm resolution, 1 km² is 25 megapixels (roughly 10–25 MB compressed). A 1,000 km² region would be 10–25 GB, far above the package size target (§15).

**Permitted imagery and land-cover sources:**

| Source | Resolution | Licence | Usable? |
|---|---|---|---|
| **Copernicus Sentinel-2** (own cloud-free composite; EOX Sentinel-2 cloudless only the 2016 layer) | 10 m | Copernicus data: free, attribution. EOX: 2016 CC BY 4.0, 2018–2025 CC BY-NC-SA 4.0 (not usable) | Yes: own composite (Phase 8) |
| **ESA WorldCover** (land-cover classification, not imagery) | 10 m | CC BY 4.0 | Yes |
| **Official orthophotos** (e.g. German states' DOP20/DOP40, many under dl-de/by-2-0 or dl-de/zero-2-0; other EU countries similar) | 0.2–0.4 m | Open, varies by country/state | Yes, region by region |
| **USGS NAIP** (USA) | 0.6–1 m | Public domain | Yes |
| Google, Bing, Apple, Esri, Mapbox satellite layers | high | Proprietary; terms forbid extraction / offline use like this | **No** |

Every imagery and land-cover source used in a package is listed in its metadata and shown on the credits screen (§13). Region sources found so far (Liechtenstein, Black Forest) and how imagery calibrates the look: Phase 8 (§14).

**Implementation (Phase 3, land use and buildings):** the first step uses OSM only, no imagery and no textures yet.

- **Classes:** OSM areas are classified into 17 land classes: default (no land use, grass), meadow, farmland, forest, orchard, vineyard, residential, commercial, industrial, park, water, wetland, rock, scree, glacier, bare (sand, quarries, construction sites) and scrub. Sources: closed ways and `type=multipolygon` relations (outer rings assembled from split member ways, inner rings as holes; even-odd fill) with `landuse`, `natural`, `leisure`, `waterway=riverbank` tags, plus `waterway=river|canal|stream` lines buffered by their `width` (defaults 12 / 6 / 2 m; culverts and tunnels left out); `ditch` and `drain` lines only when tagged at least 2 m wide (v2.24: Freiburg's Bächle, gutters 0.15–0.3 m wide along every old-town street, were drawn as water bands a cell diagonal wide and turned the old town blue).
- **Splat map:** each 1 km tile gets a class per **5 m cell** (200 × 200, scanline rasterisation at the cell centres in global coordinates, so neighbouring tiles agree at their edges). Where areas overlap, a fixed **priority** decides, not the drawing order: water > glacier > wetland > rock > scree > bare > park > forest > scrub > orchard / vineyard > meadow > farmland > commercial / industrial > residential > default; specific surfaces beat the large built-up areas they lie in.
- **Water surfaces:** water polygons and rivers ≥ 8 m wide mark the terrain grid vertices in water. The water level at such a vertex is the mean terrain height of the water vertices within 50 m (flat on lakes, gently falling along rivers); the terrain there is carved 1 m below it. The level is also stored for the vertices next to water, so the client draws a flat water quad over every grid cell touching water and the shoreline is where the terrain rises through it. Narrower streams only colour the ground.
- **The sea (v2.42, Tenerife):** OSM has no sea polygons, only `natural=coastline` ways with the land on their left. The builder keeps every coastline way when it cuts the data to the region, joins them into closed rings (the coasts of islands and islets) and cuts these rings out of a frame of the package's tiles plus 1 km: the rest is **one water area with a fixed level of 0 m** (the DTMs are in orthometric heights) instead of the mean ground height of lakes. Sea vertices are carved at least **2 m** below the surface, so a DTM that disagrees with the coastline (a cliff vertex at 30 m on the sea side) does not lift the water; land vertices next to the sea are raised to at least **0.3 m**, so flat beaches and quays (the MDT05 sets the sea near the coast to 0 m) do not flicker with the water. Coastline pieces in the area that do not close (a mainland coast cut off by the extract) would need closing along the frame: until that is done such a package gets no sea and the builder warns. The world cleaning's water levelling (§4.8.4) leaves the sea alone: a water vertex at exactly the sea level outside every lake is sea or its shore, so a river mouth (a barranco) never lifts it. Tenerife: 74 rings in the frame (the island, the Roques de Anaga and other islets), 26 % of its land-use cells are sea. Fix found on the way: the last row and column of a tile at the edge of the package took their heights from the missing next tile and got no water; they now take them from the tile itself.
- **Rendering:** the client turns the splat map into a per-tile **colour map** (one texel per cell, sRGB, mipmapped, class colour turning to rock on slopes above 60 %, a little noise), the terrain vertices stay white. Water is a separate flat, glossy mesh. **Trees** come from a jittered 8 m grid with a chance per class (forest 85 %, orchard 45 % small, scrub 30 % small, park 10 %, residential 6 %, meadow / farmland / default 1.2 %, none on water, rock, scree, glacier, bare or vineyards), never in cells near road strips (3 m beyond the edge) or under buildings (1.5 m around the footprint), not on rock faces; the same trees are the camera's obstacles. Packages without land use keep plain grass and provisional tree patches. ESA WorldCover and the satellite tint (steps 2–3 above) follow later; the tiling materials (step 1) came with Phase 8 T1 / T2 (below).
- **Buildings:** OSM `building=*` closed ways and multipolygons (not `building:part`). Wall height from `height` (metres, feet), else `building:levels` × 3 m, else by type: houses and farms 6 m, apartments 12 m, commercial and public 9 m, industrial 8 m, churches 12 m, garages / sheds / huts 3 m, unknown 6 m (3 m under 40 m²). Nearly rectangular footprints (≥ 85 % of their oriented bounding rectangle, no courtyard) of houses, farms, small buildings and churches up to 400 m² (barns 1,500 m²), or anything tagged with a pitched `roof:shape`, get a **gable roof** on that rectangle, ridge along the long side, 0.4 × the width high (`roof:height` overrides; `height` includes the roof); all others a flat roof. Placement: the walls start 0.5 m below the lowest terrain under the footprint (its corners and a 4 m grid inside) and end the wall height above the mean terrain, at least 2 m above the highest. Flat roofs are triangulated by ear clipping, courtyards bridged into the outline. Buildings belong to the tile of their centroid and are merged into one mesh per tile.

**Material library and terrain materials (Phase 8 T1 / T2, v2.46):**

- **Library:** 21 curated CC0 PBR materials (26 since T4, v2.57: roof slates, metal sheet, brick, stone, concrete for the houses; 28 with T5's pavers and kerb; 31 since T6, v3.12: jersey knit, carbon twill and saddle leather for riders and bikes, group `gear`) – ground: meadow grass, farmland soil, forest floor (spruce needles), rock (grey limestone), scree, gravel (river pebbles), sand, snow, wetland; roads: asphalt, worn asphalt, gravel road, paving stones / cobbles, dirt track; houses (for T4): clay roof tiles, plaster, timber; volcanic ground for Tenerife (v2.46): lava rock (dark basalt), volcanic ash / cinder, black sand, Canary pine needles – from ambientCG (9) and Poly Haven (12), chosen after looking at reference photos of both regions (`art/reference/`, Phase 8D). Each is albedo + normal (OpenGL) + roughness, ambient occlusion where the source has it.
- **Shipping (decided):** the textures are **not in git** (the repository dropped binary media from its history; 57 MB of textures would be most of a clone). `assets/materials.toml` (in git) pins every source file – a URL, or a member of a pinned archive – by **SHA-256**, with `license`, `source` (the asset page where the licence was checked) and `author`. `scripts/fetch-materials.sh` (`make materials`, `ocw-assets materials fetch`) downloads the sources once into a per-machine cache, refuses any file whose hash differs (the source changed: check it again), converts and writes the git-ignored `assets/materials/` (textures, a copy of the manifest, `CREDITS.md`); materials whose stamp (converter version + pins) is unchanged are skipped, so every build path can run it. Packaging copies the folder next to the binary (`scripts/install-materials.sh`): AppImage `usr/share/ocw/materials`, macOS bundle `Contents/Resources/materials`, test deployment `/var/tmp/ocw/materials`, Windows zip `materials\`; the app also finds the library in the checkout it was built from and in `$OCW_MATERIALS` (`off` disables it). CI fetches with a cache keyed on the manifest and validates. Rejected alternative: committing small compressed textures (even 512² BC files are ~20 MB per revision and would grow the history with every change).
- **Format:** KTX2 (Khronos), written by `ocw-assets` itself: per material `<id>.albedo.ktx2` (BC3 sRGB: albedo with the ambient occlusion baked in at 60 %, roughness in alpha) and `<id>.normal.ktx2` (BC5: normal x, y; z rebuilt in the shader), 1024² with a full mip chain (albedo averaged in linear light, normals renormalised per level), compressed with the pure-Rust `texpresso` (deterministic, no C++ toolchain on any build machine), the mean albedo in the KTX2 metadata. BC formats are supported by every desktop GPU we target (AMD, Apple Silicon, Windows drivers, Mesa lavapipe for the headless tests); Basis Universal (and a transcoder for the browser app, Phase 10) can replace them later without changing the manifest. Without BC support the app falls back to the flat colours.
- **Validator:** `ocw-assets materials validate [DIR]` and unit tests: every entry needs `license`, `source` (https), `author`, a SHA-256 per file and a roughness source; the licence rule of Phase 8A is shared by models and materials (`ocw_assets::licence`: CC0, public domain, own work, CC BY 4.0, DL-DE-BY/ZERO 2.0, OGD BY, Copernicus accepted; share-alike, non-commercial, no-derivatives and unknown rejected with the reason). Credits come from the manifest (`CREDITS.md`, console command `materials`).
- **Terrain (T2):** each land class is a fixed mix of the nine ground materials (e.g. farmland 55 % soil / 45 % meadow, forest 60 % needles / 40 % moss, residential meadow + gravel, water banks gravel + sand). Per tile the weights are stored at the **corners of the 5 m cells** (mean of the four cells around each corner, neighbour tiles included) in two RGBA8 textures of 201², sampled bilinearly: class borders blend over one cell, never show the 5 m grid, and neighbouring tiles agree on their shared edge. The fragment shader (`ground.wesl`; Rust twins `ground::blend_weights`, `rock_weight`) warps the lookup by ±2 m of noise (organic borders), sharpens the weights with a per-material noise ("height blend": a mix becomes patches, not a smear), turns slopes of 75–120 % into rock, less under forest (forests hold on steep slopes; the flat colour map used 60–100 %; with the limestone texture the steep alpine meadows of the 10 m grid read as bare cliffs) (**triplanar** on steep ground, no stretched texels on cliffs), samples every material at its real-world repeat size (`size_m`) and at a rotated macro scale ×7.3 that hides the tiling and takes over with distance, and fades between 90 and 350 m to the land-use colour map, whose class colours are now the materials' mean colours (so near and far agree; slopes turn to the rock colour as in the shader). Only the PBR input changes (albedo, roughness, normal); lighting, shadows and fog stay Bevy's, independent of the light setup (Phase 9).
- **Roads (T2):** the world builder keeps OSM `surface` (refined by `smoothness`, `tracktype` on tracks) as one byte per road strip in the new package section **`RSRF`** (only tiles where a strip differs from its category's default; older readers skip it, older packages get the category default: tracks and paths gravel, the rest asphalt). The road material (`road.wesl`) picks the layer per strip – asphalt, worn asphalt, gravel road, paving, dirt – with (across, along) texture coordinates in metres, fading to the material's mean colour far away; edge faces and the generated centre line (markings) stayed as they were until T5 (v2.60: markings, kerbs, sidewalks and the sloping road edge, Phase 8 T5). Physics still uses the three rolling-resistance classes (`RoadSurface::physics`).
- **Bridges (§4.8.6):** the deck surface is the road strip on it, so it already gets the asphalt / paving / gravel material of its OSM `surface`; the structure (parapets, piers, arches) keeps the flat colours of `bridges::bridge_colour` until its meshes get texture coordinates (T4/T5). Road strips, textured or flat, are flat overlays without shadows of their own (§4.5).
- **Regional material sets (v2.46):** the manifest's `[[region]]` entries apply to packages whose time zone (section `TZID`) they list. A set fills the nine ground layer slots with other library materials (`layers`, e.g. `rock = "lava_rock"`) and gives land classes another slot mix (`classes`, e.g. `default = { meadow = 0.45, gravel = 0.55 }`); the client builds one pair of texture arrays per set and picks it per package, the far palette follows the set's means and mix. The first set, `volcanic` for `Atlantic/Canary` (Tenerife: the island rendered green before), draws rock as lava (Poly Haven Dark Rock), scree and gravel as volcanic ash / cinder (ambientCG Gravel 024), sand and beaches (`natural=beach` and `surface=sand` are the class bare) as black sand (Poly Haven Coast Sand 05) and forest floor as Canary pine needles (ambientCG Ground 077); ground without land use is half ash. Data-driven, so T3 can add regions, slot materials and mixes (and measure the mixes from imagery) without code.
- **Regional tint hook (T3):** `TerrainLook::material_tint` per ground material (near) and a per-class tint in `ground::class_palette` (far), neutral (1.0) without a measured look.
- **Region calibration (T3, v2.47):** packages calibrated with `world-builder calibrate` carry their measured look (§4.7 "Look sections"). The client (`ocw-render::region_look`) tints each ground layer towards the measured median colour of the land class it stands for (meadow layer → meadow, farmland → farmland, forest floor → forest, rock / scree → rock / scree, sand → bare, wetland → wetland; snow and gravel stay neutral): 80 % of the way in chroma and 50 % in brightness (the aerial photo is taken in full sun from above), the forest floor only 35 % of that (the orthophoto shows the canopy, not the floor), within a factor 0.4 … 2.5. The season multiplies in: the class's median colour in the Sentinel-2 composites blended for the world date over its summer colour, mostly as brightness (25 % of the chroma, 50 % of the brightness change, within 0.6 … 1.6: a factor cannot turn green grass into snow – the first winter renders went pink), so the real seasonal colour comes from the far map. The far colour map's class colours get the tint that matches the near layers (their mix of tinted layer means), then the tile's seasonal satellite map is laid over them at 70 %; the date comes from the world clock (`ocw-sky`, Phase 9), in 4-day buckets, and the loaded tiles are rebuilt when the bucket changes (time machine). Buildings take their measured roof colour (an OSM `roof:colour` a quarter of the way to the default, so a named "red" reads as tiles).
- **Forests:** the forest floor is 60 % needle litter and 40 % moss, its albedo darkened under the canopy (`CANOPY_SHADE` 0.55 at full forest floor; the low-poly trees' own shadows leave the ground too bright), and forests keep 70 % of their ground on slopes that would otherwise turn to rock.
- **Orthophoto corridor (T7, v3.10):** where a package has the section `ORTC` (§4.7), the ground near the roads takes its colour from the real place's orthophoto: fields, gardens, driveways, parking lots, lava flows. The photo never replaces the tiling materials; it gives the colour, the materials give the detail – the photo times the materials' brightness relative to the land-use colour they stand for (`ortho::apply`; a per-channel ratio gave purple lawns where a material's hue differs from its class colour). Up close the colour comes from the photo blurred to about 4 m (no magnified 0.5 m pixels beside the rider), between 15 and 120 m the sharp photo takes over, and where the materials have faded to the colour map (beyond 350 m) the ground is the photo itself; the corridor weight (alpha) and a view-distance fade (650 → 950 m) scale it all. The client builds one texture per tile for the tiles around the rider's (3 × 3, the fade ends before the next ring): the JPEG chunks decoded at 2,000² (0.5 m) or 1,000² (1 m), the weight in alpha, a mip chain averaged in linear light, BC3 – 5.3 MB per 0.5 m tile instead of 21 MB uncompressed – in the background; without the material library (no BC support) there is no corridor. Setting `display.orthophoto` on / off (default on; console `ortho`), off drops the textures.
- **Performance:** see Phase 8 T2 status (§14) for the measured frame rates.

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

**Look sections (Phase 8 T3, v2.47):** `PALT` holds the imagery sources of the package (id, name, licence, notice, URL, date, resolution) and the **regional palette**: the median colour per land class (interior land-use cells only – all four neighbours of the same class, away from roads and buildings), of the paved and of the unpaved roads (every 2 m along the graph's centre lines, no tunnels or bridges) and of all roofs, with the number of pixels and the source; classes the orthophoto does not cover come from the Sentinel-2 summer composite. `ROOF` holds one colour per building (r, g, b and its origin: orthophoto or OSM) in the order of the `BLDG` section: the per-channel median of the orthophoto pixels inside the footprint eroded by 1 m (fewer than 6 pixels: the whole footprint), OSM `roof:colour` / `roof:color` (CSS names, `#rgb`, `#rrggbb`) wins. `SEAS` holds the seasonal tint maps: per season its date window and centre day, the number of scenes and the median colour per land class; per tile one map per season of 50 × 50 cells (20 m; `--season-cells 100` for 10 m), each stored as YCoCg with chroma at half resolution, LOCO-I predicted and deflated (lossless within a few levels), gaps (cloud all season) filled from the summer composite, tiles outside the composites left out. Readers that do not know the sections skip them (tested); a skeleton reads only `PALT` (credits). Package sizes before → after: Liechtenstein 30.2 → 35.0 MB (`SEAS` 4.8 MB for 510 tiles, `ROOF` 0.07 MB for 17,897 buildings), Black Forest south 258.5 → 284.5 MB (`SEAS` 24.8 MB for 2,800 tiles, `ROOF` 1.25 MB for 306,478 buildings), Tenerife 238.5 → 261.6 MB (`SEAS` 22.6 MB for 2,690 tiles, `ROOF` 0.48 MB for 116,479 buildings); `PALT` is a few hundred bytes. The tint maps cost about 9 KB per tile and season set at 20 m (4 × 50²); at 10 m they would be about four times as large, which is why 20 m is the default.

**Orthophoto corridor (Phase 8 T7, v3.10):** `ORTC` (optional, only added; `ocw_world::ortho`) holds the orthophoto along the package's roads: layout version, pixel size, corridor half width, chunks per tile side (8: 125 m chunks), weight cells per tile side (250: 4 m), the encoding (1 = baseline JPEG, sRGB, rows along +z), the indices of its sources in `PALT` (their notices are the credits), then per tile its **corridor weights** at the cell corners (0 … 255; LOCO-I predicted and deflated like the tint maps) and its stored chunks (column, row, JPEG bytes). Only chunks the corridor touches are stored; pixels outside it are the local ground colour. The weight is 1 up to 30 m beside the nearest road edge (the strips of the tile and its neighbours) and fades to 0 at 50 m (`corridor_weight`); it is 0 in forests (the photo shows the canopy from above, not the floor under our trees; this also halved the Black Forest's corridor) and on building footprints, rising from 1.5 to 4 m around them (roofs lean in the photo and would be painted on the ground beside the 3D houses). Readers that do not know the section skip it, a newer layout is skipped too, a skeleton leaves it out, and streaming clients (B2) do not fetch it with the base (`STREAM_SKIPPED_SECTIONS`; tested).

**Land use and buildings sections (Phase 3):** `LUSE` holds the splat map of every tile (layout version, cells per side, then per tile the classes run-length coded row by row and the water levels as (terrain vertex index, level) pairs); `BLDG` holds the buildings per tile (kind, roof shape and height, base and eave height, outline and courtyard rings in tile-local metres). Both are optional: older clients skip them as unknown sections (tested), newer clients without them fall back to plain grass. The pilot package grows from about 26 MB to about 30 MB with them.

**Building tags section (Phase 8 T4, v2.57):** `BTAG` holds, per tile and only for buildings OSM says something about, the facts the houses are styled by (`ocw_world::buildings::BuildingTags`): the finer use (chalet, farmhouse, barn, garage, shed, church, chapel, greenhouse, canopy, hut, hotel, retail, terrace), the tagged roof shape, a roof height from `roof:height`, `roof:angle` or `roof:levels`, `building:levels`, `roof:levels`, `min_height` / `building:min_level`, the wall material (`building:material`, `building:facade:material`), `building:colour`, `roof:material` and `roof:colour`. Layout: version, tiles, per record the building's index in the tile's `BLDG` list, a record length and a flag word, then only the flagged fields – readers skip flags and bytes they do not know, so fields can be added without a new layout; an unknown layout version is skipped, not fatal. `BLDG` itself is unchanged: pitched roofs keep their four-point rectangle and `Roof::Gabled` height, with the corners rotated so that the first edge carries the ridge (`roof:orientation=across`, `roof:direction`) or, for a skillion, the low eave – an old reader draws a gable along the right axis. A pyramid on a polygon (towers, spires; `roof:shape` pyramidal, cone, dome, onion on a convex footprint up to 600 m²) is `Roof::Flat` in `BLDG` and its height is in `BTAG`. Cost: `BTAG` is 0.02 MB in Liechtenstein (2,105 of 17,897 buildings with facts, 660 with a roof shape; +0.06 % of the package), 0.51 MB in the Black Forest south (51,729 of 306,478; 17,039 shapes; +0.18 %) and 0.53 MB in Tenerife (63,588 of 116,479; 949 shapes; +0.2 %).

**Road dress section (Phase 8 T5, v2.60):** `RDRS` holds, after a layout version byte, per tile with a strip that differs from its class default and per road strip of the tile (in strip order): the arc length of the strip's first point along its road edge and the edge's length (f32 each), lanes forward / backward (a nibble each), the centre line and the edge / lane line flags, the cycle lanes left / right, the kerb height (cm), the sidewalk widths left / right and the junction zones at the edge's start and end (dm), and the transverse markings (count, then per marking its position along the strip (f32), kind and half). Older readers skip the section, an unknown layout version is skipped, packages without it draw their roads with the class defaults (`RoadDress::of_class`). Cost: about 13–19 bytes per strip: +0.18 MB in Liechtenstein (+0.5 %), +2.9 MB in the Black Forest south (+1.0 %), +1.5 MB in Tenerife (+0.5 %).

**Region section (decided):** `REGN` holds the region the package was cut to: the latitude / longitude box and the polygon rings (outer and holes) of the `.poly` file, if any (layout version, four `f64`, then the rings as (lat, lon) `f64` pairs). It is what neighbouring packages are matched and drawn by (§2.2.2); `world-builder package --bbox S,W,N,E` narrows the `--poly` region to a box (cutting a region into neighbours), `--origin LAT,LON` sets the projection origin of the local metres (default: the box centre). Packages without it still load but cannot have neighbours. The manifest repeats the box as `bounds` (taken from the section when the package file is added), so clients find neighbours without reading every package.

**Routes section (decided):** `ROUT` holds the package's routes on the road graph (§2.7): per route its name, a loop flag and the steps (stable edge ID with from / to metres as `f64`, or a U-turn). A package with only routes and no graph is still version 2. `world-builder gpx` and `route --world` add routes; readers that do not know the section skip it.

**Route files (v1.9, interim):** a plain-text `ocw-route 1` file holds one route with its elevation and the terrain grid around it (`name`, `origin` lat/lon of the local metres, `width`, `points` x y z every 5 m, optional `surface FROM TO NAME` stretches that are not asphalt, `grid`). It is written by `world-builder route` and ridden with `--route-file`, optionally together with a world package (`--world`) for the surroundings. Routes as part of packages (above) replace it; it keeps working until the last users are moved to package routes.

**Second world: the Black Forest (decided 2026-11-12, v2.24).** After the Liechtenstein pilot (about 160 km², 30 MB) the second package is the **southern Black Forest** (Schwarzwald, Germany), id `europe/germany/black-forest`, file `blackforest.ocwpkg`.

- **Sources (first build; since v2.36 both Regierungsbezirke and LGL DGM1, below):** the Geofabrik extract of the Regierungsbezirk Freiburg (Baden-Württemberg, 160 MB PBF; it holds the southern and central Black Forest, the northern part lies in the Regierungsbezirk Karlsruhe) and the SRTM tiles N47E007, N47E008, N48E007, N48E008; `scripts/fetch-pilot.sh blackforest` downloads them into `data/blackforest/` with a licence table (`SOURCES.md`; OSM ODbL 1.0, SRTM public domain). `scripts/build-blackforest.sh` builds the package and the routes.
- **Extent:** the box 47.72–48.12 N, 7.62–8.30 E (`--bbox`, 44 × 51 km, about 2,260 km² or 14 times the pilot), projection origin 47.92 N 7.96 E (`--origin`, fixed so that later neighbour parts share its metres and tile grid). It reaches from the Rhine plain west of Freiburg to Neustadt and from Waldkirch / the Kandel to St. Blasien: Freiburg, Schauinsland, Feldberg, Titisee, Belchen, Blauen, Schluchsee, Todtnau.
- **Measurements** (build machine `marge`, release build, one package each; client = `ocw-desktop` free ride on software Vulkan, whose RSS includes what a GPU would hold as VRAM):

| Extent | Area | Tiles | Edges | Package | Build | Builder RSS | Client peak RSS |
|---|---|---|---|---|---|---|---|
| Liechtenstein pilot | 160 km² | – | – | 30 MB | – | – | 0.68 GB |
| Core: Freiburg – Schauinsland – Feldberg – Titisee (47.84–48.08 N, 7.70–8.20 E) | 1,000 km² | 1,344 | 66,340 | 124 MB | 53 s | 2.9 GB | 0.99 GB |
| **South (chosen)** | 2,260 km² | 2,800 | 141,245 | 260 MB | 101–131 s | 3.2 GB | 1.76 GB |
| Large: Rhine to Baar, Waldshut to Elz (47.58–48.30 N, 7.55–8.45 E) | 5,370 km² | 6,192 | 300,190 | 564 MB | 251 s | 3.9 GB | 3.31 GB |

- **Why this extent:** building is cheap at every size (minutes, under 4 GB), so the client decides. It decodes the whole package into memory (tiles, graph, land use, buildings; about 5.5 MB of RAM per MB of package on top of 0.65 GB), so the large extent breaks the memory budget of §15 (≤ 2 GB) and is above the 200–500 MB disk target, while the south extent stays within both and holds every iconic climb of the southern Black Forest. More of the Black Forest (north of the Elz, the Karlsruhe part, the Baar) comes as **neighbour packages** (§2.2.2): the same `--poly` extract or the Karlsruhe one, their own `--bbox`, the same `--origin`; only their tiles near the rider are loaded. Loading tiles from a package on demand instead of decoding all of it would allow larger single packages later.
- **Routes:** `Freiburg - Schauinsland` (19.9 km, 1,055 m climbing: Bertoldsbrunnen 281 m – Günterstal – Schauinslandstraße – road end below the summit at 1,267 m) and `Titisee - Feldberg` (13.5 km, 500 m climbing: Titisee 857 m – B317 via Bärental – Feldberg pass, about 1,280 m), both as package routes (`ROUT`; the first is ridden by `--world` alone) and as route files (`freiburg-schauinsland.route`, `titisee-feldberg.route`). `world-builder route` takes `--bbox` / `--origin` like `package`, so a route file is in the package's metres with its road heights (the context profiles of the part).
- **Check:** 0.003 % of the road samples lie under the terrain (8,818 of 333 million, worst 4.7 m on a track at the western border of the box), below the 0.1 % of the pilot test; land use 55 % forest, 20 % meadow, 10 % farmland, 5 % residential; 306,476 buildings.

**The whole Black Forest in three packages (decided 2026-11-13, v2.36).** The southern package gets two neighbours to the north, so that free ride runs from Freiburg to Baden-Baden; all three are built from the same day's Geofabrik extracts of both Regierungsbezirke, Freiburg and Karlsruhe (`--pbf` and `--poly` twice: Ortenau with Sasbachwalden and the Mummelsee lies in Freiburg, Baden-Baden, Bühl and Freudenstadt in Karlsruhe), with the LGL DGM1 terrain (§4.3), the projection origin 47.92 N 7.96 E of the south and cut with `--bbox` on two shared border lines:

| Package (id `europe/germany/…`) | Box | Area | Tiles | Edges | Buildings | Package | Build | Builder RSS | Under terrain | Client alone |
|---|---|---|---|---|---|---|---|---|---|---|
| `black-forest` (south, rebuilt) | 47.72–48.12 N, 7.62–8.30 E | 2,260 km² | 2,800 | 141,244 | 306,478 | 259.9 MB | 206 s | 7.6 GB | 0.002 % | 2.02 GB |
| `black-forest-middle`: Elzach, Triberg, Hornberg, Wolfach, Schramberg, Offenburg's east | 48.12–48.45 N, 7.70–8.40 E | 1,900 km² | 2,337 | 104,359 | 238,882 | 213.9 MB | 178 s | 7.0 GB | 0.001 % | 1.78 GB |
| `black-forest-north`: Baden-Baden, Schwarzwaldhochstraße (B500), Mummelsee, Hornisgrinde, Sasbachwalden, Allerheiligen, Ruhestein, Bühlerhöhe, Kniebis, Achern, Bühl, Freudenstadt | 48.45–48.80 N, 7.85–8.45 E | 1,720 km² | 2,200 | 96,973 | 219,529 | 194.4 MB | 151 s | 6.5 GB | 0.001 % | 1.74 GB |

- **Cut lines from the measurements:** a package of about 2,300 km² is the most the client holds within §15 (it decodes the whole home package; 2.0 GB for the south at 2560 × 1440, 1.76 GB at 1280 × 720), so the region north of the south package (Elz valley to Baden-Baden, 75 km) needs two more: the line **48.12 N** (the south's border, Waldkirch / Kandel south of it, Elzach north) and **48.45 N** (between Hausach / Wolfach and Oppenau / Kniebis), which leaves the Schwarzwaldhochstraße with all its places (Kniebis 48.47 N to Baden-Baden 48.76 N) in one package. West and east edges follow the Rhine plain (the Rhine is the state border; France is not in the extracts) and the Kinzig / Murg uplands; 163 roads cross 48.12 N, 155 cross 48.45 N.
- **Riding them as one world:** the neighbour search follows joined neighbours (§2.2.2), so any of the three loads the other two (as skeletons). Measured client peak (free ride, release build, software rendering at 2560 × 1440, 4 bots): south with both neighbours 2.62 GB, north with both 2.50 GB, middle with both 3.45 GB (its neighbours' tiles come near on both sides) – above the 2 GB budget of §15 for the middle; loading tiles of the home package on demand (streaming from the file, §3.5 / §4.5) is the fix, not smaller packages. Joining needed 80 s at first (the edge at each boundary vertex was looked up by a scan per vertex pair); with a one-pass index the start takes 25 s like a single package.
- **Projection:** one origin keeps the tile grids and metres identical across the seams (no frame needed), at the price of the local projection's scale error far from the origin: x is about 1.4 % long at the Mummelsee (cos 48.6° / cos 47.92°). The same error appears in any joined world ridden from the south; own origins per package would only help a package ridden alone.
- **Routes** (package routes and route files): `Baden-Baden - Mummelsee` (27.3 km, 914 m climbing: Lichtentaler Straße – Geroldsau – Malschbach – Schwarzwaldhochstraße via Hundseck and Unterstmatt to the lake, 1,032 m), `Sasbachwalden - Mummelsee` (12.3 km, 780 m: Brandmatt), `Bühl - Bühlerhöhe` (9.3 km, 446 m: Bühlertal), `Hornberg - Triberg` (middle, 11.0 km, 352 m: B33 / Gutach valley); the south's routes on DGM1: `Freiburg - Schauinsland` 19.9 km, 995 m, `Titisee - Feldberg` 12.7 km, 449 m.
- **Releases** (`scripts/prepare-world-release.sh`, nothing uploaded): per package `ocw-world-<file>-v<N>.zip` (package, manifest, route files, `LICENSE-DATA.md` with every source's attribution from the manifest, `README.md`) and per region `<region>-osm-source.zip` (the extracts and `.poly` files with `SOURCES.md`: the ODbL offer of the road graphs).
- **Showcase:** scenes `showcase-*` in `scripts/scenes.toml` (Mummelsee, Hornisgrinde, Schwarzwaldhochstraße, Baden-Baden, Sasbachwalden, Feldberg, Titisee, Schauinsland, Freiburg, Schluchsee, Belchen, and the before/after viewpoints Malbun, Schauinsland, Feldberg pass); reference photos to compare with in `art/reference/black-forest.md` (Phase 8D).

**Fourth world: Tenerife (decided 2026-11-13, v2.42).** The first island and the first region outside Central Europe: `europe/spain/tenerife`, file `tenerife.ocwpkg`, the **whole island in one package**.

- **Sources:** the Geofabrik extract of the Canary Islands (`africa/canary-islands`: Geofabrik files the islands under Africa, there is no `europe/spain/canarias`; 60 MB, 2026-11-12) with its `.poly`, the IGN MDT05 (§4.3) and the SRTM tiles N28W017 and N27W017 (the sea beyond the DTM); `scripts/fetch-pilot.sh tenerife` downloads them into `data/tenerife/` with `SOURCES.md`, `scripts/build-tenerife.sh` builds the package and the routes.
- **Builder options:** `--tiles-near-roads` (or `--land-tiles`) and `--no-clean` combine freely; the sea's frame is the region's box plus 5 km, the sea is kept out of the bridge water test (one area over the whole region) and passed to the land use beside the other areas.
- **Extent:** the box 27.98–28.61 N, 16.94–16.10 W around the island (the other islands lie outside it), projection origin 28.29 N 16.52 W, time zone `Atlantic/Canary` (WET / WEST, from the origin: the region table of `ocw_sky::zone` has the Canary Islands now; spec §14 Phase 9I). Islands are built with **`--land-tiles`** (v2.56, see *Tile cover* below): the tiles within the margin (2 km) of a tile with a road plus every tile with land, not the whole box of the roads, which was 64 % open sea. Until v2.55 it was `--tiles-near-roads` alone, which left out the Teide's summit (3.3 km from the nearest road, the cable car's base station) and Pico Viejo: the mountain was missing.
- **One package or two, from the measurements** (marge, release build; client = `ocw-desktop` free ride in Las Cañadas, software Vulkan at 2560 × 1440, 4 bots, as for the Black Forest):

| Build | Tiles | Edges | Buildings | Package | Build | Builder RSS | Under terrain | Client peak |
|---|---|---|---|---|---|---|---|---|
| Whole box (first build) | 5,460 | 69,103 | 116,479 | 577 MB | 68 s | 3.2 GB | 0.005 % | – |
| Tiles near roads, before the world cleaning (§4.8) | 2,690 | 69,103 | 116,479 | 239 MB | 66 s | 1.8 GB | 0.005 %, worst 0.6 m | 1.42 GB (1.35 GB at 1280 × 720) |
| Tiles near roads, with the world cleaning (v2.42–2.55) | 2,690 | 69,103 | 116,479 | 238 MB | 80 s | 2.3 GB | 291 samples (0.000 %), worst 0.36 m | – (same tiles) |
| **Tiles near roads and on land, with the world cleaning (chosen, v2.56)** | 2,744 | 69,103 | 116,479 | 244 MB | 87 s | 2.3 GB | 291 samples (0.000 %), worst 0.36 m | – |

  About 2,034 km² of land with 9,088 km of roads give a package the size of the southern Black Forest, within the disk target and the client's memory budget of §15, so the island stays **one package**: every signature climb (Teide from the north and from the south, Masca, Anaga) is in it and a cut through the Teide would only add a seam. The 3,700 m of relief cost nothing extra (the same 10 m grid).
- **Lint** (`world-builder lint --pbf`, with cleaning, §4.8.1): 9,038 km of roads (+ 36 km bridges: 516 tagged, 15 from outlines, 4 over water, 179 over gaps); floating untagged roads 0.8–1.5 m 2.4 %, 1.5–3 m 1.0 %, 3–6 m 0.3 %, > 6 m 0.04 %; 1,629 samples over the class grade limit (8.1 km, mostly paths), 43 kinks, 1 step, 1 near-miss end; 0.64 % of the road samples over untagged water (coastal roads where the OSM coastline runs on or beside them, and the irrigation ponds), 7,401 water level steps and 132 river steps (the barrancos: steep dry riverbeds the river rule levels downhill), 27,585 half-buried buildings on the steep slopes. The water cleaning levels 5,467 lakes and 2 river chains and keeps the sea at 0 m (a river mouth does not lift it).
- **Routes** (package routes and route files; gradients from the road profiles, with cleaning): `Puerto de la Cruz - Teide` (42.0 km, 2,349 m climbing: Puerto de la Cruz at the sea – La Orotava – TF-21 via Aguamansa and El Portillo – the cable car base station at 2,356 m; steepest 20 % on a short stretch of the Carretera de las Medianías above La Orotava, a shortcut the router takes off the TF-21), `Los Cristianos - Teide` (48.6 km, 2,509 m: Arona – TF-51 Vilaflor – TF-21 Boca de Tauce – Parador – cable car; steepest 14 %), `Masca - Santiago del Teide` (5.1 km, 426 m: the TF-436 hairpins to the Cherfe pass; steepest 15 %), `Buenavista - Masca - Santiago del Teide` (21.9 km, 1,230 m: the whole TF-436), `La Laguna - Anaga` (10.9 km, 449 m: Las Mercedes – TF-12 through the laurel forest – Cruz del Carmen – the TF-114 junction below the Pico del Inglés; steepest 8 %). The road and terrain cleaning in progress (bridges, steep spikes) may change these numbers.
- **Land use** (existing classes, no new ones): the lava fields and the Teide's slopes are `natural=bare_rock` / `scree` → rock / scree, the retama scrub of Las Cañadas and the coastal tabaibal `natural=scrub` → scrub, the Canary pine forest and the laurel forest of Anaga `natural=wood` / `landuse=forest` → forest, banana plantations (`landuse=orchard`, `trees=banana_plants`, 3,007 areas) → orchard, greenhouses (`landuse=greenhouse_horticulture`, 3,088) → farmland, terraces `landuse=farmland`, vineyards, the black-sand beaches `natural=beach` / `sand` → bare, the ports (`landuse=port` / `harbour`) → industrial; 15,659 small `natural=water` and 7,136 `landuse=reservoir` are the irrigation ponds (water). Shares: default 28 %, sea 26 %, forest 18 %, scrub 14 %, rock 3 %, residential 3 %, farmland 3 %, orchard 2 %. Missing for Phase 8 (the materials work of another branch, nothing added here): lava rock (black / red-brown, blocky), volcanic ash and pumice (the ochre caldera floor), black sand, Canary pines (long needles, open forest) and laurel forest as their own tree types, banana terraces and net houses, flat-roofed whitewashed houses.
- **Tile cover (v2.56, bug 2026-11-13 "the Teide's summit is missing").** Which 1 km tiles a package has (`ocw_worldgen::package::TileCover`): `Box` (default, inland regions) – the box of all roads plus `--margin-tiles` (2); `NearRoads` (`--tiles-near-roads`) – only tiles within the margin of a tile with a road; **`NearRoadsAndLand` (`--land-tiles`, islands)** – `NearRoads` plus every tile of the box that has **land**: its natural ground (the DEM, before the roads shape it) rises above 1 m at any point of a 21 × 21 test grid (50 m), so islets count and the open sea (0 m in MDT05 and SRTM) does not. Mountains far from roads – the Teide, Pico Viejo, the ridges of Teno and Anaga – are then in the package as terrain-only tiles. They keep the package's one grid (the format has one `samples` per package; a coarser grid for terrain-only tiles would need a format change and saves little, they are few); the far terrain (§4.5) draws them at distance. Tenerife: 2,690 → 2,744 tiles (+54: the summit area of the Teide and Pico Viejo and the inner Las Cañadas, the rest of the land was already within 2 km of a road), 238.5 → 244.3 MB, build 89 → 87 s (2.3 GB builder RSS, unchanged). `world-builder check --peak LAT,LON,M` fails unless the highest terrain within 150 m of the point is within 50 m of M; `build-tenerife.sh` checks the Teide (28.2724 N, 16.6424 W, 3,715 m): terrain 3,712 m at x −11,980 z 1,920 (missing before: no tile). Unit tests (`ocw-worldgen`): a cone far from two coast roads is in the package with its summit height and the sea tiles are not (`land_cover_keeps_a_mountain_far_from_roads_and_drops_the_sea`), the land test sees a 60 m islet.
- **Releases** (`scripts/prepare-world-release.sh`, nothing uploaded): `ocw-world-tenerife-v<N>.zip` and `tenerife-osm-source.zip`.
- **Showcase:** scenes `showcase-tf-*` in `scripts/scenes.toml` (Las Cañadas and the Teide, the cable car station, sunrise and sunset in the caldera with the time machine, the Masca hairpins, Los Gigantes and the sea, Santa Cruz, La Orotava, Anaga), the clock fixed on 13 November 2026; reference photos in `art/reference/tenerife.md`.

Packages are **signed, versioned and updatable**:

- **Ed25519** signatures. An offline root key signs rotating online signing keys. The client ships the root public key.
- Each package manifest lists SHA-256 hashes of all files.
- **TUF** (The Update Framework) is evaluated for package and app updates.
- Target size: 200–500 MB for a mid-sized region.
- The format allows streaming over HTTP in tile-sized chunks (§3.5).

**Build profile (decided 2026-11-13, v2.59).** `world-builder package` prints a stage table at the end of every build: wall time, share, CPU time, the cores in use (CPU / wall) and their share of the machine, peak and final resident set per stage (`ocw_worldgen::stages`; Linux reads `/proc/self`, a sampler thread follows the resident set every 10 ms; elsewhere only times), and writes it as JSON with `--profile FILE`. `scripts/world-profile.sh` builds Liechtenstein, a Black Forest part and Tenerife into a scratch directory with the arguments of the build scripts and compares each package byte for byte with a reference build. The profile showed the build mostly single-threaded (1.3 of 12 cores in the Black Forest): five sequential PBF passes, the road profiles (twice for a part: its own graph and the context graph; most of it the gradient limit across junctions), bridge detection and the settle pass. Accelerated, with byte-identical packages:

- PBF passes decode their blobs on all cores (`osm::par_elements`, results in file order);
- `ocw_worldgen::par::map`, a parallel map with dynamic scheduling (each thread takes the next item, results in item order), replaces one contiguous chunk per thread for the DTM files (outside files cost only their header), the terrain heights of the tiles, and runs the profile DEM sampling, the bridge detection and the settle pass's constraint collection per edge or tile;
- the settle pass relaxes over vertex indices instead of a hash-map lookup per use (the same arithmetic in the same order);
- the class gradient across junctions (`limit_grades_across_junctions`, up to 2,000 Gauss-Seidel sweeps over every profile sample) skips an edge while neither its own last sweep nor a neighbour changed anything at its ends – exactly the operations that would have changed something, in the same order (2 × 23 s → 2 × 0.5 s for the Black Forest).

Measured on marge (12 threads; wall time, cores in use = CPU / wall, peak resident set; the old code on the same input for comparison):

| World | old code | new code | Hot stages (old → new) |
|---|---|---|---|
| Liechtenstein | 5.7 s, 2.8 cores, 0.81 GB | 3.1 s (−46 %), 5.6 cores, 0.83 GB | profiles 1.0 → 0.3 s, settle 1.3 → 0.25 s, bridge detection 0.5 → 0.1 s |
| Black Forest north | 164 s (load 3–10; 180–243 s at load 17–27), 1.3 cores, 6.26 GB | 57–61 s (−63 to −65 %, load 7–11), 3.5–3.7 cores, 6.5 GB | profiles 2 × 28 → 2 × 4.5 s (grades across junctions 2 × 23 → 2 × 0.5 s), PBF passes 43 → 16 s, bridge detection 2 × 7.5 → 2 × 1.3 s, settle 19.4 → 3.7 s, DTM files 2.9 → 1.7 s |
| Tenerife | 81 s (90–129 s at load 17–27), 1.5 cores, 2.23 GB | 36 s (−55 %), 3.5 cores, 2.41 GB | profiles 2 × 16 → 2 × 6.6 s, PBF passes 8.4 → 4 s, bridge detection 2 × 4.3 → 2 × 0.7 s, settle 14.8 → 2.7 s |

The machine was shared with other builds throughout; on a saturated machine the parallel passes help less (an intermediate version without the junction-grade change took 153–178 s for the Black Forest at load 17–27). The new code takes about 0.2 GB more peak memory (per-thread buffers of the parallel passes).

**Equivalence:** every package of the new code is byte-identical to the old code's on the same input (`scripts/world-profile.sh --ref`, Liechtenstein, Black Forest north and Tenerife, five rounds; the old binary built from the unchanged tree, run with `--no-profile`). One Tenerife comparison differed only because the SRTM tiles in `data/tenerife` were downloaded again between the two builds (SRTM fallback 0 → 2 tiles); old and new code on the new data are identical again, and repeated runs of the new code are identical.

**What remains (open):** the area extract (`read OSM areas` 21 s for the Black Forest: the PBF passes 12 s, `retain near` 6 s, single-threaded hash-set work), the through-road filter (2 × 3 s), land-use cleaning and water levels; the region graph's profiles are computed and then replaced by the context graph's (`transfer_profiles` takes all of them), which would save another 8 s but changes the bridge spans derived from them, so it needs its own equivalence check.

**Manifest, signatures and cache (implemented, Phase 3):**

- **Manifest** (`ocw_world::manifest`, pure and wasm-safe; ed25519-dalek + sha2): a JSON file next to the package (`<file>.ocwpkg.manifest.json`; `manifest.json` in published and cached directories). The signed part holds the format (1), the **package id** (lower-case path segments, e.g. `europe/liechtenstein`), region name, **version** (build number), creation time, generator (`world-builder 0.1.0`), package format (`OCWPKG02`), the **attributions** (© OpenStreetMap contributors with the ODbL Derivative Database notice for the road graph, the DEM source, §13) and every file with size and SHA-256 **plus each chunk** (header, every tile, every section) with offset, size and SHA-256, so single tiles fetched with range requests can be verified on their own.
- **Signature format:** the signed part is kept as raw JSON bytes (`"signed": {…}`) and signatures cover exactly those bytes behind a per-kind context string (`OCW world package manifest v1\0`, `OCW signing key v1\0`). No canonical JSON is needed, and fields added later do not break old signatures; unknown fields are ignored.
- **Key hierarchy:** the **root key** (Ed25519) is generated once (`world-builder keys init-root`) and kept offline; it only certifies **signing keys** (`keys new-signing`, default validity 365 days): a certificate with the signing key's public key, name, root key id and validity period. Each manifest carries its signing key certificate, so the client needs only the root public keys. Key id = the first 8 bytes of SHA-256 of the public key. Secret key files (`*.key`, mode 0600, never overwritten) live outside the repository (`~/.config/ocw/keys`, `$OCW_KEY_DIR`); `.gitignore` also ignores `*.key`.
- **Verification chain:** certificate signed by a trusted root → neither key revoked (a revocation list compiled into the client) → manifest signature by the signing key → manifest created within the signing key's validity period and not more than a day in the future → each file's size and SHA-256. Validity is checked against the creation time, not today, so a cached package keeps working offline after its signing key expires, while an expired or stolen-and-revoked key cannot sign new packages.
- **Trusted roots:** production roots are compiled into the client (none yet; they come with the distribution infrastructure). Development roots (`crates/ocw-worldstore/keys/dev-roots.json`, public keys only, marked development) are trusted by debug builds and with `--trust-dev-root`. More roots: `--trust-root FILE` and `~/.config/ocw/trusted-roots/*.pub.json` (private servers, own builds).
- **Policy for packages not signed by a trusted root:** integrity failures (hash or size mismatch, bad signature, revoked key, malformed manifest) are **always refused**. Packages that are intact but unsigned, signed by an unknown root or outside the key's validity are a trust failure: from the **cache** (by id) they are **refused** unless `--allow-unsigned`; from a **path** (local builds, power users, §4.2) they load **with a warning** unless `--require-signed`. A package without a manifest counts as unsigned (id `local/<name>`, version 0). Multiplayer sessions (Phase 5) will require trusted packages.
- **Identity:** id, version and the SHA-256 of the package file (`Manifest::identity()`, `PackageIdentity`) tell client and server whether they ride the same world (and the same stable edge IDs). Builds are deterministic, so the same input gives the same content hash on every build machine.
- **Cache:** `$OCW_WORLD_CACHE` or `~/.local/share/ocw/worlds/<id>/<version>/{manifest.json, <files>}`; downloads collect in `.partial/<id>@<version>/` and are moved into place only when every file is verified. Packages are verified again on every load. `--world` takes a path or a cache id (`ID` = newest version, `ID@VERSION`); `--list-worlds` and the console commands `worlds.list` / `worlds.verify` show and check the cache.
- **Download:** `ocw-desktop --fetch-world URL` fetches the manifest first and checks its signature before any large download, then the files next to it (the layout `world-builder publish` writes: `<base>/<id>/<version>/manifest.json`), resuming interrupted downloads with HTTP range requests (`Range: bytes=N-`; a server answering 200 restarts the file). Local directories and `file://` URLs install the same way. Downloads happen before the ride, never during it (§4.2). `world-builder publish` also writes `<base>/<id>/latest` (the newest version number) for clients that name a world by id. TUF remains under evaluation for app updates and key rotation metadata.
- **Streaming (v2.99, §14 Phase 10 B2):** the browser app streams a package by chunk instead (`ocw_worldstore::stream`, pure and wasm-safe; the manifest check `accept_manifest` is shared with `--fetch-world`): manifest and signature chain first, then the base (`header`, `sections`, every `section …`) and the `tile X,Z` chunks around the rider with merged `Range: bytes=a-b` requests; each chunk is checked against its SHA-256 in the manifest before use or caching, a mismatch is fetched once more on its own and then refused with an error naming the chunk. Verified chunks are cached per id / version / chunk (the browser's OPFS) with least-recently-used eviction by a size budget. `ocw_world::package::StreamedPackage` decodes the base once and the tiles one by one; the result equals the whole file's decode with those tiles.
- **Rebuilds:** `world-builder diff OLD NEW` (`ocw_worldgen::diff::GraphDiff`) lists the edges whose stable ID survived (unchanged or moved), was removed or added; routes over surviving edges re-match directly, the others are flagged.
- **Rewrites (v2.24):** commands that change a built package in place (`route --world`, `gpx`, `routes --from / --remove`) write its manifest again (same id, name and attribution, next version), because the old one lists the old SHA-256 and every client refused the package as damaged. A signed manifest becomes unsigned (the key is not at hand); the command says to sign it again.

### 4.8 World Cleaning (decided 2026-11-13, v2.39)

OSM and the DEM are not physically consistent: roads step and kink, ends stop a metre short of the road they join, rivers on SRTM step up and down, culverts were drawn as open water, and the terrain grid cannot follow two roads at different heights a few metres apart. The world builder therefore **cleans the world while building it**. It may deviate a little from the source data to get a physically believable world, within the limits below; what riders ride stays the same road network.

**Never changed:** the topology of the road graph (vertices, edges, junctions, one-ways), the stable edge IDs and therefore the routes (§2.7, §4.4); the road classes, names and widths; the OSM land classes (only slivers are merged). `world-builder package --no-clean` (and `route --no-clean`) builds without the cleaning, for comparisons.

#### 4.8.1 Lint: measuring what looks wrong

`world-builder lint --world PKG [--pbf EXTRACT]` (`ocw_worldgen::lint`, pure) counts every finding with its extent and the worst cases (local metres, latitude / longitude, OSM way):

| Check | Finding |
|---|---|
| `road.vertical.grade_over_limit` / `.step` / `.kink` | a 5 m interval steeper than its class limit + 1 %; a step (> 1 m and > limit + 5 %); a gradient change > 8 points between two intervals |
| `road.horizontal.duplicate_node` / `.spike` / `.self_crossing` | points < 0.25 m apart; an out-and-back point (turn > 150°, a leg < 15 m); a centreline crossing itself |
| `road.near_miss_end` | a dead end within 3 m of another road it does not join |
| `junction.height_mismatch` / `junction.surface_step` | profile ends at a junction differing by > 5 cm; a road strip lying on another road's strip near their junction with > 0.25 m height difference |
| `road.crossing_step_untagged` | strips of two roads without a shared junction overlapping at different heights, neither tagged as bridge or tunnel |
| `road.float.{untagged,bridge}.BAND`, `road.buried.*` | sampled every 2 m at the centre and both edges: terrain more than 0.8 m below the road surface (bands 0.8–1.5, 1.5–3, 3–6, > 6 m), or above it; floating untagged roads get a cause: `water` (a water vertex under the road), `lower_road_nearby` (the cut zone of a lower road within 15 m), `other` |
| `road.over_water_untagged` | road length over water cells without a bridge (a missing bridge) |
| `water.river_uphill` / `.river_step` / `.lake_not_level` / `.level_step` | along each river with a water surface (in OSM flow direction, every 10 m) the level rising > 0.3 m above the lowest so far, or jumping > 0.5 m; a lake whose levels span > 0.25 m; neighbouring water vertices > 0.5 m apart |
| `building.floating` / `.half_buried` / `.on_steep_slope` | wall foot above the ground; ground above half the wall height; > 3 m of slope under the footprint |
| `landuse.sliver` / `.gap` | patches of ≤ 2 cells (≤ 50 m²) of one class; unclassified holes of ≤ 6 cells enclosed by one class |
| `bridge.SOURCE.STRUCTURE` | the bridges built (§4.8.6), by source and structure |

#### 4.8.2 Roads: horizontal

- **Duplicate nodes** (< 0.25 m) are removed; **spikes** (turn > 150° with a leg < 15 m, the point at most 15 m off its neighbours' chord) are removed, repeatedly. Graph vertices never move here.
- **Near-miss joins:** a dead end (one road, not at the region border, not a bridge or tunnel) within **3 m** of another road is moved onto that road's centreline (the largest horizontal deviation the builder allows), and after the profiles its end takes that road's height there; its road is re-limited to its class gradient. No junction is added (that would change topology and stable IDs).

#### 4.8.3 Roads: vertical

On top of §4.3 (sampling, median and Gaussian filters, class gradient limits along through-roads and across junctions):

- **Gradient limits per class** stay those of §4.3: 15 % trunk / primary / secondary, 20 % tertiary / unclassified / residential / cycleway, 25 % service / track / path; bridges and tunnels are straight between their ends.
- **Vertical curvature:** inside each road (not at its junction ends) the gradient may change by at most **0.5 % per metre** on trunk / primary / secondary and **1 % per metre** on all others; kinks are rounded into vertical curves (a sample moves towards its neighbours' chord, so no interval gets steeper).
- **Junction mouths:** a wider road is drawn flat across, so a side road leaving it must not climb or fall within the wider road's half width: its first 5 m interval is limited to a **0.15 m** step at the wider road's edge (never so far that its far end leaves the class gradient), the rest re-limited from there.
- **Terrain under roads:** after the road-conformed terrain (§4.3) and the water (§4.8.4) the terrain grid is **settled**: every road strip and bridge deck is sampled every metre along and 0.5 m across, and wherever a terrain triangle (split as in the client mesh) rises closer than **5 cm** to the surface, its three vertices are lowered by the least amount (a Gauss–Seidel projection over the violated samples; lowering never breaks a satisfied one). Vertices shared by neighbouring tiles change in every copy. The result: no terrain shows through any road.
- **Terrain conforming per road (the cause of floating roads):** before v2.39 every segment within a road's cut zone (half width + 15 m) cut the terrain to its height at its nearest point – for the segment behind the one under a point that is the node between them, up to 15 m back and on a 10 % climb 1.5 m lower, and the blend could take that height as well. One third of Liechtenstein's and 28 % of the Black Forest's road length stood 0.8 m or more above the ground (`road.float_cause.other`). Now each road shapes the terrain through its **nearest point only**, the cut is its half width + 0.5 m, and the settle pass does the rest.
- **Where the grid cannot follow** (the upper leg of a hairpin 10–15 m above the lower one, a road beside a lower road): the road's edge face reaches down to 0.3 m below the terrain at its edge (at least 2 m, at most 40 m), so it stands on a **retaining wall** instead of floating as a slab; on bridge decks the strip has no edge face (the deck has its own sides).
- **Road edge into the terrain (v2.60, Phase 8 T5):** the drawn surface lies `ROAD_SURFACE_LIFT_M` (2 cm) above the profile and the settled terrain at least 5 cm below it, so on flat ground the old vertical edge face (light grey, 2 m deep) stood above the terrain as a slab edge (Las Cañadas). Now the edge has a 3 cm lip, then a **shoulder** (verge, the gravel layer beside paved roads, the road's own material beside unpaved ones) sloping 1 : 1.5 down to 4 cm under the terrain surface, 0.4 m wide plus 1.5 × the drop (at most 2.65 m), with a 0.3 m skirt below its outer edge; in junction zones it stays 0.4 m wide (the crossing road's surface covers the lip). Where the ground lies more than 1.5 m below the edge the retaining wall of the previous rule remains (now in the kerb concrete). No edge geometry on bridge decks. The riding surface and the wheel contact (v2.47) are unchanged; riders never touch the shoulder (road edge rule §5.2.1).

#### 4.8.4 Water

- **Rivers** (OSM `waterway` lines with a drawn surface, ≥ 8 m wide; their direction is the flow direction) are joined across way splits into chains (same name or width, one way continuing at the shared node). The natural DEM along each chain every 10 m is fitted by the best **non-increasing** profile (least squares, pool adjacent violators): rivers never flow uphill and never step up. Main rivers are levelled first; a tributary ends at least at the main river's level, a branch starts at most at it.
- **Lakes** (water areas that are not river surfaces: not `water=river|stream|canal|…`, not `waterway=riverbank`) get one level: the median of the DEM under them.
- Every water vertex takes its lake's level, else the nearest river chain's within its half width plus three grid cells; the terrain under water is carved 1 m below the new level.
- **No water under roads:** vertices within a road's half width plus one grid cell (roads on the ground; bridges excepted) are never carved and get their road-conformed height back – before, carving lowered the terrain under roads beside rivers and lakes by a metre and more, and left them floating.
- **Culverts:** `tunnel=*` and `location=underground` on waterways are now read (they were dropped when reading the areas), so culverts under roads are no longer water bands.

#### 4.8.5 Buildings and land use

- **Building sites:** the terrain vertices inside a footprint (and within 1 m of it) move to the footprint's mean ground, by at most **1.5 m** (steeper sites keep their slope: a house set into the hillside), never under roads or water; the building is then placed as in §4.6 (walls from 0.5 m below the lowest ground).
- **Land use:** patches of at most 2 cells of one class (not water) take the most common class around them; unclassified holes of at most 6 cells enclosed by one class take that class. Patches at a tile edge are kept.

#### 4.8.6 Bridges

**Which stretches are bridges:**

1. OSM `bridge=*` (not `no`) ways – the whole edge (as before).
2. Untagged roads inside a `man_made=bridge` outline.
3. **Detected:** an untagged road crossing water – a water area or a waterway line ≥ **3 m** wide (narrower streams pass in a culvert) – with water on both sides of the road (a road along a shore is not a bridge; water stretches > 250 m along a road are mapping overlaps); or a road more than **5 m** above the natural ground (at least 10 m somewhere) for **8–150 m** (a gap the elevation filter bridged; longer is an embankment). Each detected span reaches 3 m beyond the obstacle (the abutments), merges with spans less than 8 m apart, and is left out where a tagged bridge already starts at that end. The build log counts them; their deck is planned **straight between the abutments**, and the terrain is cut under them but not pulled up to the deck, as under tagged bridges.

**The structure** (`ocw_world::bridges`, section `BRDG`, additive; older clients skip it and draw plain road strips):

- **Structure type:** `bridge=covered` → covered; `bridge:structure` arch / humpback → arch, truss → truss, suspension / simple-suspension / cable-stayed → suspension, anything else → beam; untagged → beam, except **stone** (`bridge:material` stone / masonry / brick) bridges up to 40 m, which are **arches**.
- **Material:** `bridge:material`, else by structure (covered → wood, truss / suspension → steel, arch → stone), paths up to 25 m → wood, other paths and cycleways → steel, roads → concrete. **Railing:** stone and concrete road bridges have solid **parapets** (0.9 m high, 0.35 m thick), steel, wood and path bridges **rails** (posts every 2 m, rails at 0.55 and 1.1 m).
- **Deck:** the road strip on top (the deck surface 1 cm under it), 0.6 m kerbs beyond the carriageway on each side; thickness by structure and span between supports: beam span / 18 (0.45–2.5 m; wood span / 25, 0.3–0.8 m), arch 0.45 m, truss 0.6 m, suspension 0.8 m, covered 0.5 m. Road strip points on a deck get no edge faces into the terrain (the deck has its own sides).
- **Supports:** abutments at both real ends (a block from the deck bottom to 0.6 m below the ground just outside the end; none where the bridge continues on the next bridge edge); **piers** where the bridge is longer than the structure's maximum span – concrete 35 m, steel 45 m, wood 12 m, stone beam 15 m, stone arch 18 m, other arches 50 m, truss 60 m, covered 30 m, suspension none – evenly spaced, each a wall pier from the deck bottom to 0.6 m into the ground (or river bed), only where the deck is more than 1 m above it.
- **Arches:** between consecutive supports, springing from the lower ground at the two supports (at most half the span below the crown), crown 0.35 m under the deck bottom; spandrel walls on both faces and the arch underside. Too low for 0.8 m of rise → a slab.
- **Above the deck:** truss sides (height length / 8, 2.5–7 m; chords, verticals, alternating diagonals), suspension towers and main cables with hangers, covered bridges with posts every 3 m, board walls to 1.2 m and a gable roof.
- **Rendering:** `ocw-render::bridges` builds the meshes per tile (a bridge belongs to the tile of its middle) with plain colours per material and part (`bridge_colour`, the hook for the material library of Phase 8). The terrain and water continue under the deck.

Reference photos (Commons links only): `art/reference/bridges.md`.

#### 4.8.7 Results (2026-11-13)

`scripts/world-clean-compare.sh li|bf` builds each package with `--no-clean` and with the cleaning and lints both; the high-resolution DEMs of v2.35 (swissALTI3D, DGM1) are used. Share of the road length (untagged roads):

| Check | Liechtenstein before | after | Black Forest south before | after |
|---|---|---|---|---|
| floating > 0.8 m | 32.2 % | 7.2 % | 26.2 % | 5.3 % |
| floating > 3 m | 5.2 % | 0.7 % | 3.2 % | 0.6 % |
| buried | 2 m | 2 m | 326 m | 6 m |
| vertical kinks (> 8 points) | 774 | 6 | 11,678 | 40 |
| junction surface steps | 1,045 | 792 | 11,957 | 8,880 |
| near-miss ends / duplicate nodes / spikes | 3 / 6 / 1 | 0 / 0 / 0 | 30 / 40 / 10 | 6 / 4 / 0 |
| rivers uphill (length) | 6.5 km | 0 | 31.5 km | 70 m |
| tilted lakes | 41 | 0 | 286 | 4 |
| land-use slivers / gaps | 3,301 / 1,588 | 16 / 5 | 177,971 / 24,957 | 1,083 / 264 |
| bridges built (`BRDG`) | – | 274 (2 over water, 6 over gaps, 15 in outlines detected) | – | 2,826 (47 over water, 82 over gaps, 453 in outlines detected; 9 covered) |

With SRTM (before v2.35) the same passes took Liechtenstein from 33.9 % to 3.9 % and the Black Forest from 28.0 % to 4.1 % floating road length. The rest is mostly where the 10 m grid cannot follow (`lower_road_nearby` 2.9 %, hairpins and steep river banks with the sharp DTM): there the road's edge face now reaches the ground as a retaining wall. Build time of the Black Forest package: 228 s, settling 26 s.

**Open:** no structure of the two packages is an arch yet (OSM rarely tags `bridge:material=stone`); junction steps between a wide road drawn flat across and a steep side road remain (a junction surface would fix them, §3.2); `water.river_step` counts natural drops of steep mountain rivers; the grade limits of §4.3 are exceeded by 76 short stretches already before the cleaning (with the DTM).

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

**End of a route (decided):** on a route that is not a loop (route files, routes that end) riders stop at its end: the distance stays at the route's length and the speed at 0 (`ocw-sim::ride::stop_at_finish`, used by the server's physics and the client's prediction alike), so HUD, avatar, gradient and profile stay consistent. Before, the distance ran on past the end (the HUD counted on to 18 km of a 14 km route while the avatar, GPS position and gradient stayed at the last point and the profile was empty). The dev command `teleport KM` likewise moves a rider past the end to the end, and before the start to the start, and its reply says so (`teleported to 14.023 km, the end of the route …`); on a loop any distance is taken (that many laps on). Regression tests: `teleport_and_riding_stop_at_the_end_of_a_route` (`ocw-sim`), `teleport_past_the_end_of_a_route_goes_to_its_end` (`ocw-server`).

**Start point and test riders (v1.9):** a session starts all riders together at a configured distance (`ServerConfig::start_m`); the desktop app picks a random point by default (on a route that ends, at least 1 km before the end; off for screenshot and capture runs, which must be reproducible). Riders start in a formation: rows of up to three across the lane, 1.2 m apart and always 0.3 m inside the lateral limit (fewer per row on narrow roads), rows 3 m apart behind the start point; the front row's centre is kept for the client rider (bots fill the other places in order, so the formation is deterministic). The formation never starts before the road begins: on a route it is moved forward until its last row is at 0 m, in free ride every rider's path starts at their place on the start edge (moved forward if the edge behind is too short). Server-side test riders (bots, 10 by default) ride at twelve different strengths (85–280 W, easy and strong interleaved) so a field spreads out, each on its own bike and kit (the bike types in turn, colours from the palette, never the local rider's pink – also the bots a local event brings, which used to be the default pink rider on the standard road bike), push harder uphill (at least the power for 10 km/h on the current gradient, so easy bots do not crawl) and steer with autodrive.

### 6.2 Network Protocol

- **Tokio** as the asynchronous runtime.
- **Quinn** for QUIC-based communication (TLS 1.3).
- **Protobuf** via `prost` for all structured messages.
  - Schemas live in `proto/` and are versioned (`ocw.v1`). Changes follow protobuf compatibility rules: add fields, never reuse field numbers.
  - Position datagrams use compact quantized fields: distance in cm (`uint32`), speed in mm/s (`uint32`), lateral offset in cm (`sint32`), animation state (`uint32` enum).
  - A protocol version is exchanged during the handshake; incompatible clients are rejected with an update prompt.
- **Unreliable datagrams** for position updates; **reliable streams** for login, session management, steering intents and important events.
- The protocol is transport-independent: the browser app joins over a WebTransport endpoint next to the native one (§3.5, §14 Phase 10 B3; below).

**QUIC transport (decided, Phase 5 groundwork).** `ocw-server::net` carries the unchanged encoded `ClientMessage` / `ServerMessage` bytes; `GameServer` stays transport-independent and the desktop's `ClientCore` runs on the same channel interface as with the local server.

- **Port:** UDP **47800** by default (`ocw-server --listen [ADDR]`, `ocw-desktop --server HOST[:PORT]`); no TCP port. ALPN `ocw/1`.
- **Threads:** the game loop stays one synchronous thread at 50 Hz. `NetServer` owns a small Tokio runtime (two workers) with one task per connection; each tick the loop calls `pump` (connection events and received messages → `GameServer::connect` / `handle` / `disconnect`, replies to the sender) and `broadcast` (the tick's output to every joined client). The client's `NetClient` has its own one-worker runtime; the Bevy side only sends and drains channels.
- **Streams and datagrams:** one bidirectional stream per connection, opened by the client, with every reliable message as a length-prefixed frame (4-byte little-endian length, at most 1 MiB): Hello / Welcome / Reject, steering, turn and U-turn intents, rejoins, dev commands, path updates. **Snapshots** (server → client) and **sensor samples** (client → server) – both superseded by the next one and sequence-numbered – go as unreliable QUIC datagrams. A message larger than the connection's datagram limit (a big field's snapshot exceeds ~1.2 kB; §6.3 interest management will shrink it) or a peer that accepts no datagrams falls back to the stream. The client sends samples on the stream until the server has answered its Hello, so no sample overtakes it. Datagrams may be reordered: the client drops a snapshot whose `server_tick` is not newer than the last one. Considered and not chosen: everything on one stream (simple, but a lost packet stalls all later snapshots – head-of-line blocking); a new unidirectional stream per snapshot (no head-of-line blocking, but per-stream overhead at 10 Hz × riders and still retransmits stale data).
- **Flow control:** each connection has a queue of 256 messages; when it is full, snapshots are dropped and a client that cannot take reliable messages any more is disconnected ("too slow") instead of growing memory.
- **Liveness:** keep-alive every 4 s, idle timeout 15 s; a vanished client's rider is removed after it. The client extrapolates snapshots by half the measured round trip when reconciling its prediction. The 2-minute session hold for reconnects (§6.3) is not implemented yet: a reconnect is a new join.
- **Shutdown:** `ocw-server` stops on Ctrl+C / SIGTERM, closes every connection with "server shutting down" and waits up to 1 s for acknowledgements; the desktop shows "Disconnected: …" in the HUD and keeps running.
- **Course check:** the client rides its own course data; besides the world package check (§6.5) it refuses a Welcome whose route (or free-ride mode) differs from its own. `ocw-server --route-file` rides a route file (as `ocw-desktop --route-file`; both build the route with `RouteFile::to_route`).
- **Tested over the network:** `make test-multiplayer` runs the server and the kiosk's client on one build machine and headless `ocw-bot` riders on the others (Linux, macOS, Windows) and checks joins, snapshots, mutual visibility, progress, prediction error and tick timing (§11.4.1).

**TLS in development mode (decided; until the backend exists).** TLS 1.3 only, via rustls with the ***ring*** crypto provider (licence Apache-2.0 AND ISC; already in the tree through `ureq`; chosen over the default aws-lc-rs to avoid its CMake/C toolchain on all three platforms). The server uses a self-signed certificate from `rcgen`, new at every start or kept in `--tls-dir DIR` (`cert.der`, `key.der` with mode 0600), and prints its SHA-256 fingerprint. The client must say how to trust it – there is no default: `--server-cert SHA256` pins exactly that certificate (any address), `--insecure-dev` accepts any certificate but is refused for non-loopback addresses (checked when the options are parsed and again when the TLS configuration is built). Both verify the handshake signature, so the server must hold the key. Not yet: certificates from a public CA (WebPKI roots or the platform verifier), OIDC sign-in and per-session tokens (§6.4, §12); until then a reachable server accepts anyone who has its fingerprint. New dependency licences: quinn, quinn-proto, quinn-udp, rcgen MIT OR Apache-2.0; rustls Apache-2.0 OR ISC OR MIT; ring Apache-2.0 AND ISC. They are used only by `ocw-server` (and the desktop through it), never by the wasm crates (§3.5).

**WebTransport endpoint for browsers (decided, v3.02, §14 Phase 10 B3).** `ocw-server --listen … --webtransport [ADDR]` opens a second QUIC endpoint (HTTP/3, ALPN `h3`, **UDP 47801** by default) feeding the same `GameServer` session through the same `NetServer` (one event channel, one peer table, one broadcast): desktop, `ocw-bot` and browser riders share one session.

- **Library: `web-transport-quinn` 0.13** (MIT OR Apache-2.0; with its `ring` feature, no aws-lc-rs): it does HTTP/3's SETTINGS and the extended CONNECT on a `quinn::Connection` of *our* endpoint, so our quinn 0.11, rustls config, *ring* provider and transport settings (keep-alive 4 s, idle 15 s) stay; its streams are tokio `AsyncRead` / `AsyncWrite`, so the native framing (`net::framing`) is reused unchanged, and it has a client (`NetClient::connect_web`, used by the loopback tests and `ocw-bot --server https://…`). Considered: `wtransport` 0.7 (MIT OR Apache-2.0, complete, but its own endpoint and configuration types around quinn, so a second TLS / transport setup to keep in step) and `h3` + `h3-webtransport` (still experimental, much more glue). New transitive licences: `web-transport-proto`, `web-transport-trait`, `kio`, `rustls-platform-verifier`, `fastbloom`, `sfv` MIT OR Apache-2.0; `webpki-root-certs` CDLA-Permissive-2.0 (data, as `webpki-roots` already in the voice build). All native only.
- **Same channels:** the browser opens one bidirectional stream with the length-prefixed frames of every reliable message; snapshots and samples go as WebTransport datagrams (QUIC datagrams with the session id in front; `max_datagram_size` minus that header), with the same fallback to the stream. The wire rules without I/O moved to the wasm-safe `ocw_server::wire` (frames, the incremental `FrameDecoder`, which messages are unreliable, `LinkStatus`) together with the client's connection state machine `ClientLink` (queued until connected, Hello and early samples on the stream until the server answers, datagrams only when the peer takes them and they fit, a broken frame closes the link, the first close reason stays, RTT from the transport or Hello → Welcome). The session path is `/ocw/1` (or `/`); other paths get 404.
- **Certificates (development):** browsers take a self-signed certificate by its SHA-256 (`serverCertificateHashes`) only if it is ECDSA (P-256) and valid for at most 14 days. `net::tls::WebIdentity` makes one valid for 13 days (from an hour back), kept in `--tls-dir` (`web-cert.der`, `web-key.der` mode 0600, `web-cert.validity`) and reused while more than two days are left; the running server checks every minute and **rotates** it two days before the end (`Endpoint::set_server_config`: new sessions get the new certificate, open ones go on), logs and prints the new hash and rewrites `--web-info-file` (JSON: URL, hash, expiry, page options). Pages then need the new hash (`cert-hash=OLD,NEW` takes two). It is a separate certificate from the native one (whose fingerprint desktop clients pin for good). Production: certificates from a CA with hosting (B8), then without `cert-hash`.
- **Counters:** `NetServer::counters(Transport)` (connections, messages, datagrams per endpoint), in the log at the end and in `--stats-file` (`web_sessions`, `web_messages_in`, `web_datagrams_in`).

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
6. **Hearts (v2.53, §14 Phase 4, 4E):** hearts are credited only by the server – the game server that saw the power stream, or the backend from an uploaded ride after these checks; never for virtual power, god mode, bots or captures; flagged rides hold their hearts until reviewed; a daily cap. Purchases are server-side transactions against the server's catalogue price. Since hearts have no money value (no purchase, no cash-out) and races can ignore parts (*Profile* / *Equal* modes), a cheated balance gains little.

### 6.5 Steering in Multiplayer

- The client sends a reliable `TurnIntent { junction_id, exit_edge_id }` as soon as the choice changes (§2.2.1).
- With neighbouring packages (§2.2.2) junction ids are vertex indices of the joined graph, `WorldPackageId.name` is the joined name (`Home + Neighbour`) and `graph_hash` the joined graph's fingerprint, so client and server must have the same packages installed; a missing neighbour on one side shows as a graph mismatch. (v1.6: plus a sequence number and the exit's riding direction; no `exit_edge_id` = no choice, the default; `junction_id` is the vertex index in the world package's graph – client and server use the same package.)
- A manual U-turn is a reliable `UTurn { distance, speed }`: where the client turned, so the server builds the same loop when it is close to its own state.
- **Same package on both sides (decided):** because junctions are vertex indices, the client sends `Hello.world = WorldPackageId { name, graph_hash }` and the server sends its own in `Welcome.world`. `graph_hash` is the FNV-1a 64 fingerprint of the road graph's package encoding (the `GRPH` payload), so any rebuild that changes the graph changes it. The server rejects a Hello with another hash ("world package mismatch … install the same package version"); a client refuses a Welcome with another hash (for servers that do not check). Sessions without a road graph (route files, test routes) skip the check. Package signing and manifests (§4.7) are separate.
- **Rider weight (v2.7):** `Hello.rider_weight_kg` carries the rider profile's weight; the server clamps it to 30–200 kg and uses its curated physics profile otherwise (`Hello.profile` is deprecated, §3.2.1).
- **Voice (Phase 6):** `ClientMessage.voice` (9, `VoiceTokenRequest`) asks for a LiveKit token, `ServerMessage.voice` (7, `VoiceGrant`) answers it; `Welcome.voice` (9) says the server offers voice, `Snapshot.human_riders` (3) counts the riders of connected clients (§2.4, §7). Voice itself never uses this connection.
- **Bike and look (Phase 4):** `Hello.bike_profile` (field 6) names the curated physics profile by id (unknown → default), `Hello.look` (field 7, `RiderLook`) the cosmetic look; `RiderUpdate.bike_profile` (11) and `RiderUpdate.look` (12) pass both to the other clients (§2.5).
- **Workshop and hearts (Phase 4, W1 / W2, v3.00):** `ClientMessage.bike_change` (16, `BikeChange`: garage bike id, frame profile id, look, part ids; reliable) is sent when the rider leaves the workshop; the server applies profile (its own table, the rider's weight) and look at once outside events, refuses it for an entrant of an unfinished event ("from the next start") and answers with a `DevCommandResult`; part ids are cosmetic until W3. `SensorSample.virtual_power` (7) flags keyboard / virtual-trainer power; `RiderUpdate.hearts` (18) is the server's running total of the rider's credited hearts in the session (1 per 10 kJ of accepted work + 1 per 20 m climbed; none for virtual power, god mode, while down, for bots or the Echo rider).
- **Build physics (Phase 4, W3, v3.03):** the parts of `BikeChange` now count: the server computes the build from its own catalogue copy (§14 Phase 4, 4G) and applies it at once outside events; for an entrant of an event that is not over it waits and applies when the event ends (from the next start) – the answer says which. `Hello.parts` (8) is the build the rider joins with. `ServerMessage.rider_build` (10, `RiderBuild`: rider, `build_hash`, profile, the fitted part ids in slot order, the server's effect against the stock bike – flat and 6 % climb in 1e-4, bike mass in g, capped) goes reliably to everyone once per change (and to a joining client for every rider with parts); the snapshots carry only `RiderUpdate.build_hash` (19, fixed32, 6 bytes; 0 = stock), and a client draws a rider's parts only while the hash matches the last `RiderBuild`. `EventStart.physics` (7) / `EventState.physics` (14) carry the race mode (`PhysicsMode`: unspecified = by the kind; an older server's state = open). `DevCommand.bot_bike` (4, `BotBike`: a bot's profile, parts, steady power, fixed lane, name; local servers only) compares builds side by side.
- **Character and kit (Phase 4, W4, v3.07):** `RiderLook` gains the character (spec §14 Phase 4, 4C) as optional fields – `height_cm` (5), `body_build` (6), `skin` (7), `face` (8), `hair` (9), `hair_rgb` (10, fixed32), `glasses` (11); absent = the default, out-of-range values the default's, the height clamped to 150–205 cm. They travel in `Hello.look` and `BikeChange.look` and from the server once per change in `RiderBuild.look` (9; also sent when only the character changed, and to a joining client for every rider with parts or a non-default character); the snapshots' `RiderUpdate.look` keeps only the colours and the pattern. The worn kit is catalogue ids among the parts (`Hello.parts`, `BikeChange.parts`, `RiderBuild.parts`; kit counts in the build and its hash).
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

**Implementation (Phase 6, first increment; decided 2026-11-12):**

- **Crate layout.** `crates/ocw-voice` holds the logic, all pure and unit-tested: `mute` (the two switches), `join` (auto join / leave, retry back-off), `spatial` (attenuation, pan, nearest-N), `device` (device choice with fallback), `mix` (the mixer, resampler, confirmation sounds) and `controller` (`VoiceController`: drives a `VoiceBackend` trait with commands only when something changed). The LiveKit backend (`ocw-voice::livekit`) is behind the crate feature `livekit`; the desktop's cargo feature **`voice`** turns it on and is **off by default**, so CI, minimal builds and the other platforms build without libwebrtc. Without it the same controller runs on a null backend: switches, settings, menu and console work, the status says "not built in". `apps/desktop/src/voice/` is the Bevy glue (plugin, hotkeys, console commands, Settings → Voice pages); `ocw-render::voice` draws the HUD indicator.
- **Game server: rooms and tokens.** The client sends `VoiceTokenRequest { channel, group }` (reliable, on the game connection); the server answers `VoiceGrant { url, token, room, identity, expires_unix_s, channel, error }`. The server decides the room: the session room `ocw-<instance>` (default), the event room `…-event-<id>` (only for entrants of the running event), or a group room `…-group-<name>` (names of 1–32 characters a–z, 0–9, `-`); `<instance>` is a per-start id so servers sharing one LiveKit never share rooms. The token is a LiveKit JWT (HS256, signed with `sha2`-based HMAC in `ocw-server::voice`, no new dependency) valid for **10 minutes** (LiveKit checks it at join and refreshes it for connected clients; the client asks again before every reconnect), allowing join, subscribe and publishing the **microphone only** (no data, video or admin rights). One token per connection per 2 s. URL, API key and secret come from `OCW_LIVEKIT_URL`, `OCW_LIVEKIT_API_KEY`, `OCW_LIVEKIT_API_SECRET` (`--livekit-url`, `--livekit-api-key` override the first two; the secret is never a command-line option, it would show in `ps`) and never enter the repository. **Dev-mode assumption until accounts exist (Phase 5):** the identity is `rider-<rider id>` of the connection and the token name its display name; anyone in the session who knows a group's name can join it. With OIDC the identity becomes the account and groups get invitations; the token issuing moves to the API (§12) unchanged in shape.
- **Platform independence (platform plan: LiveKit everywhere).** Everything in `ocw-voice` outside the `livekit` module – mute state, hotkey mapping (`hotkeys`: M / Shift+M, Select / R3 as library-free inputs), auto join / leave, spatial attenuation, nearest-N, device fallback, mixer, controller – uses no threads, clock, file system or audio device (times are passed in) and is part of the wasm check (`make test-wasm`, CI). The `VoiceBackend` trait only queues commands (`send`) and returns events already received (`poll`), and device lists are what the backend last learnt, so a browser backend (livekit-client JS called from Rust/wasm, asynchronous) or a mobile one (native LiveKit SDKs or the Rust SDK) fits behind it without changing the controller.
- **Client audio path.** Options were (a) the SDK's platform audio (WebRTC's own device module: capture, playout, echo cancellation, device selection built in) or (b) own audio I/O. (a) mixes all remote voices inside libwebrtc with no per-participant volume, which spatial audio needs, so **(b) is chosen**: cpal (already in the tree through Bevy's audio) captures the microphone and plays a stereo mix; every subscribed voice comes from a `NativeAudioStream` (48 kHz mono) into the `Mixer` with its own gain (spatial × voice volume) and constant-power pan; WebRTC's audio processing module (from libwebrtc) does echo cancellation, noise suppression, gain control and a high-pass filter on the microphone, fed with the played mix as the echo reference; the processed 10 ms frames go to a `NativeAudioSource` published as the microphone track. Devices are opened only while in a call; devices without 48 kHz are resampled (linear). Uncertainty: the echo canceller's delay estimate with cpal's buffering is untuned (headphones avoid the issue); if it proves poor, option (a) with nearest-N only (no attenuation) is the fallback.
- **Mute.** Microphone mute mutes the published track (others see it muted, unmuting is instant) and stops sending samples; the speaker mute silences the mix but keeps the voices subscribed.
- **Spatial audio and nearest-N.** Gain 1 up to 10 m, then 10 m / distance, floored at 0.15 inside 300 m, nothing beyond; pan up to ±0.5 by the lateral offset, only for riders beside the listener. The client subscribes to the nearest `voice.max_voices` (default 10) riders in range; a voice already heard counts 15 m nearer when ranking, so riders at almost the same distance do not swap. Distance: on a route the gap along it (the shorter way round on a loop); in free ride, where riders ride their own paths, the straight-line distance (an approximation of the road distance; good enough for "near" vs. "far"). Participants that are not riders, or riders not yet placed, are heard at the floor gain while there is room. In a group room everyone is heard at full voice volume (chosen partners), as with `voice.spatial off`.
- **Platform status (2026-11-12).** The SDK used is `livekit` 0.10.1 (libwebrtc downloaded prebuilt by its build script). **Linux:** from `webrtc-sys` 0.3.43 the prebuilt libwebrtc is built against Chromium's hermetic libc++, and the C++ bindings must be compiled with **clang ≥ 21** (`CXX=clang++-21`; GCC would silently break the ABI, the build refuses it). Debian 13 ships clang 19, so **Linux voice builds need clang >= 21: `make deps-llvm-debian` (apt.llvm.org)** – `clang-21`, `lld-21`, `libclang-21-dev`; `make deps-linux-debian` runs it and installs `libglib2.0-dev`. Considered: pinning `livekit` 0.8.3 / `webrtc-sys` 0.3.42 (builds with GCC, needs only the glib headers, but an older SDK), clang 21 from trixie-backports, or waiting for Debian's clang 21; the user chose apt.llvm.org's clang with the current SDK. Checked on marge (Debian 13, clang 21.1.8 from apt.llvm.org, `CC=clang-21 CXX=clang++-21`): the workspace with the `voice` feature builds, is clippy-clean and passes its tests; `make build` / `make release` and `scripts/deploy-test.sh` enable the feature by themselves when `clang++-21` is installed (`VOICE=0` / `VOICE=1` force it), so the kiosk build has voice. **macOS:** builds with Apple's toolchain (checked on the M1 build machine; its Command Line Tools lack the C++ headers on the default include path, worked around with `CXXFLAGS=-isystem …/MacOSX15.2.sdk/usr/include/c++/v1` – a broken install, reinstalling the Command Line Tools fixes it). **Windows:** `ocw-voice` with the `livekit` feature compiles on the Windows build machine (MSVC); the desktop with `voice` and a run with audio devices are not checked yet.
- **Integration test.** `crates/ocw-voice/tests/livekit_two_participants.rs` (feature `livekit`) connects two headless participants (no sound card: microphone samples pushed in, the mix read out) with tokens minted like the game server's; one sends a tone, the other must hear it and see the mute. It runs only when `OCW_LIVEKIT_URL` / `_API_KEY` / `_API_SECRET` point at a LiveKit server (e.g. `livekit-server --dev`) and passes without doing anything otherwise, so `cargo test --workspace` stays network-free. CI with a LiveKit service container is still open.
- **Echo bot (the echo test; asked for by the user 2026-11-14, v2.96).** To test voice alone, an **Echo** rider stands on the road and plays the rider's own microphone back to them, delayed, from its place.
  - *Server.* `EchoRequest { on, here }` (reliable, answered with a `DevCommandResult`) puts the session's one Echo rider on the road – 20 m ahead of the asker on their track (in free ride on a copy of their path), 1.5 m right of the centre (less on narrow roads) – or takes it away; asked again it stays where it is, `here` moves it. It is a rider of its own kind in `ocw-sim` (`Control::Echo`): it stands still, is out of the physics and drafting, takes no start-formation slot, is not a bot, in no event, entrant list or standings and earns nothing; snapshots mark it (`RiderUpdate.echo`), its name tag is "Echo", it is not counted in `human_riders`. Allowed on the local servers (`ServerConfig.allow_echo_bot`, `Welcome.echo_bot`); `ocw-server --echo-bot` allows it on a shared server and puts one at the start of every session (logging its id and voice identity). The client asks for it once per session while the echo test is on (again after a ride switch), and takes it away when it is turned off.
  - *Near.* The controller (`ocw-voice::echo`, `VoiceController::set_echo`) sees the Echo rider like any rider (the gap on the route, the straight-line distance in free ride): the microphone is recorded in full up to **30 m**, linearly less up to **60 m**, nothing beyond (what was recorded still plays out, quieter, as the rider rides on, so passing it is heard as approach, echo, fade); the playback is placed like a voice (§7: gain 1 to 10 m, then 10 m / distance, pan by the lateral offset beside it; × the voice volume; without spatial voice at the volume). The microphone is opened for it only within the voice range (300 m).
  - *Delay.* **300 ms by default** – clearly an echo, apart from the own voice (a few ms would only colour it like a comb filter), short enough to answer oneself; 20 ms – 3 s (`voice.echo delay MS`, setting `audio.echo_delay_ms`, 100 ms steps in the menu). A `DelayLine` (fixed ring buffer, one allocation) delays sample-exactly at 48 kHz: every block gives as many samples out as went in, so nothing drifts or piles up (unit-tested over 5 minutes of odd block sizes; the whole loop over 3 minutes with the mixer's buffer never above one 10 ms frame). The record gain ramps across each 10 ms block (no clicks).
  - *The path.* **Solo rides (local loopback, the default; no infrastructure, works on the kiosk):** the LiveKit backend opens the microphone and speaker even without a call while the echo test is active; the microphone frames go through WebRTC's audio processing (noise suppression, gain control, high-pass, echo cancellation – fed with the played mix, so the echo is not echoed again through speakers), then through the `EchoLoop` into the mixer as the voice `echo`, played with its gain and pan, the speaker mute and the voice volume like any received voice. Not in this path: Opus and the SFU (libwebrtc encodes inside the published track, which needs a room). In a call the same frames feed both the published track and the loop. Mute switches: microphone muted → silence goes into the delay line (nothing said while muted is ever echoed; what was said before still plays out within the delay); speaker muted → the echo is silent like all voices. **With a LiveKit server:** `cargo run -p ocw-voice --features livekit --example echo_participant -- ROOM RIDER_ID [DELAY_MS]` joins the room *as the Echo rider* (`rider-<id>`, token minted like the game server's), hears everyone and publishes it delayed; clients in the call see that participant and switch their loopback off (status "via server"), so the echo crosses Opus and the SFU twice and is placed by the normal nearest-N logic. Without the `voice` feature the echo test says "unavailable" (no audio devices in the null backend).
  - *UI.* Console `voice.echo [on|off|here|delay MS|status]` (on / off and the delay are remembered as `audio.echo_bot` and `audio.echo_delay_ms`, scope live), `--echo-bot` (on for the run), **Settings → Audio → Voice** (asked for by the user): the Audio page would exceed the ten-foot limit of seven items (§3.2.1) with two more rows, so its Voice row now opens a sub-page with the voice volume (Enter plays the test tone), "Echo test" (on / off, with its state: speak now, N m away …) and "Echo delay (ms)" (20 ms – 3 s in 100 ms steps). HUD: while it is on, a line under the voice indicator – "ECHO TEST - speak now (300 ms echo)" when near, else where the Echo rider is – with two level meters (microphone, echo as played; −60 … 0 dBFS).
  - *Tests.* Delay exactness and bounded buffers, clamping, the record gain by distance, the loop and the microphone mute (`echo.rs`); end to end with the loopback backend (`tests/echo_loopback.rs`: a tone in → the same tone exactly 14 400 samples later, at the Echo rider's gain and pan; fading with distance, silent far away; mute switches; 3 minutes without drift; delay change); the headless LiveKit backend's local loopback (feature `livekit`); the server's request, marking and refusal; the sim's standing, slots, events and free-ride copy; the client's request; the app's console, menu rows, HUD and request-once logic; the protocol round trip. `OCW_ECHO_DEMO=FILE.wav cargo test -p ocw-voice --test echo_loopback` writes a 24 s demo: a synthetic voice riding past the Echo rider at 6 m/s (silent far away, the echo fading in, loud and right-panned beside it, fading out). *Uncertain:* not heard on real audio yet (the build machine marge has no sound card; the kiosk's microphone is untested), the echo canceller may partly cancel the echo picked up by the microphone through speakers – intended (no feedback loop), but it can make the echo sound thinner without headphones; the echo participant is build-checked only (no LiveKit server on the build machines).
- **Not yet:** voice for event and group rooms from the menu (console only), push-to-talk (dropped, §2.4), the participants' speaking marker on the minimap, hot-plug of audio devices during a call (a changed device reopens the streams), a Windows run of the `voice` feature, private rooms with invitations (Phase 5).

---

## 8. Bicycle and Character Assets

### 8.1 Asset Pipeline

3D format: **glTF 2.0**. Models are created, for example, with Blender.

Import process:

1. Create model.
2. Define materials.
3. Create skeleton and animations.
4. Export glTF.
5. Validate assets (`cargo run -p ocw-assets -- validate [DIR]`, including licence metadata, §13): the manifest `manifest.toml` next to the models has an entry with `license`, `source` and `author` for every `.glb` / `.gltf` (files without an entry are rejected), licences from the allowed list (§2.5), no physics keys, bikes name a known physics profile; every model reads as glTF 2.0, uses only paint slots as materials, has the nodes the app animates (bike: wheels, crank, pedals, handlebar, grip and saddle anchors, `lod1`; rider: the rig of §2.6, head wear and kit nodes), plausible rig dimensions and stays within the triangle budget (bike 15 000, rider 8 000, far LOD 4 000). The validator is a subcommand of the asset crate instead of a separate `tools/asset-validator` (one crate shares the reader with the app; low). Textures (the material library, §4.6) have their own manifest `assets/materials.toml` and validator (`ocw-assets materials validate`) with the same licence rule (`ocw_assets::licence`, Phase 8A) plus a SHA-256 pin per source file.
6. Import into Bevy.
7. Register components.

### 8.2 Bicycle Components

Bicycles consist of modular parts: frame, fork, handlebars, wheels, tires, saddle, pedals, drivetrain, accessories.

Each component has a unique ID, a configuration, a visual part and an optional physics profile reference (§2.5).

The catalogue of parts is `assets/parts.toml` (planned v2.53: generic unbranded names, tier, price in hearts from researched real prices in `assets/parts-sources.toml`, real weight, compatible bike types, physics classes, model reference; §14 Phase 4, 4H–4I). Every bike model has a hotspot anchor `hs_<slot>` per part slot for the workshop scene (4B).

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
- Profile Service (incl. the workshop: hearts ledger, inventory and bikes, purchases; §14 Phase 4, 4E).
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

**Dependency rule:** only `ocw-render`, the apps `apps/desktop` and `apps/web`, and `ocw-client` (the shared client logic, which uses `ocw-render`'s view types but no Bevy systems) depend on Bevy. `ocw-core`, `ocw-sim`, `ocw-protocol` and `ocw-world` must also build for `wasm32-unknown-unknown` (§3.5).

```
open-cycling-world/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── apps/
│   ├── desktop/          # Bevy app, wires all plugins together
│   ├── web/              # the same client in the browser: wasm32 + WebGPU, local server in the frame loop (§14 Phase 10)
│   ├── bot/              # ocw-bot: headless remote riders over QUIC, multiplayer test verdict (no Bevy, §11.4.1)
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
│   ├── ocw-client/       # client logic shared by desktop and web: ClientCore, events, god state, keys, sky (wasm-safe)
│   ├── ocw-assets/       # bike/rider glTF generator and reader, asset validator, pose, animation states (no Bevy)
│   ├── ocw-fit/          # FIT activity writer (no dependencies, wasm-safe)
│   └── ocw-voice/        # LiveKit integration behind a trait (feature "voice")
├── proto/                # .proto schemas
├── artwork/              # original artwork: logos/ (app icon)
├── packaging/            # desktop entry (Linux), later installers
├── release/              # release builds per version and platform (+ SHA256SUMS)
├── scripts/              # build machines, deploy, kiosk, AppImage, Windows build server setup, multiplayer test
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

**Build profile and speed-up (decided 2026-11-13, v2.59).** `scripts/build-profile.sh` (Linux, macOS; `scripts/build-profile.ps1` on Windows; `make build-profile MACHINE=…`) times clean, no-op, touched-file and edited-file builds of `cargo build --workspace`, `cargo test --workspace --no-run`, `cargo clippy --workspace --all-targets`, the voice release build of `ocw-desktop` and the wasm check, with cargo's `--timings` reports, into `test-output/build-profile/LABEL/` (`results.jsonl`, `summary.md` with the slowest crates, `scripts/build_profile.py compare A B` for before/after tables). Measured on 2026-11-13 (marge: 12 cores, shared with other builds; macOS: M1, 8 cores; Windows: 2 jobs):

| Machine | Command | before: clean / edit one file (full debug info) | after: clean / edit (v2.59 profile) | `target/` after clean | sccache, same checkout path | sccache, a second checkout (other path) |
|---|---|---|---|---|---|---|
| marge | `build --workspace` | 646 s / 5.3–7.8 s | 464 s (−28 %) / 3.7–6.7 s | 16.2 → 4.2 GB | – | 380 s cold → 322 s (−15 %, 87 % hits) |
| marge | `test --workspace --no-run` | 613 s / 4.5–16.8 s | 457 s (−25 %) / 2.3–5.6 s | 24.0 → 5.3 GB | 173 s (−62 %) | 173 s |
| marge | `clippy --all-targets` | 99 s / 1.4–2.6 s (touch) | – | 1.3 GB | – | 125 s cold → 95 s (−24 %) |
| marge | release, voice | 276 s / 11–27 s (touch) | unchanged profile | 3.9 GB | – | 429 s cold → 309 s (−28 %) |
| Mac | `build --workspace` | 642 s / 2.0–4.6 s (touch) | 496 s (−23 %) / 2.0–4.5 s | 4.3 GB | 119–125 s (−76 %) | 508 s cold → 437 s (−14 %, 81 % hits) |
| Mac | `test --workspace --no-run` | 690 s / 2.0–6.6 s (touch) | – | 4.6 GB | 142–145 s (−79 %) | 360 s → 357 s |
| Mac | `clippy --all-targets` | 151 s / 1.3–2.4 s (touch) | – | 1.2 GB | 111–120 s | – |
| Mac | release, voice | 456 s / 15–34 s (touch) | unchanged profile | 4.8 GB | – | 471 s cold |
| Win11Builder (2 jobs) | `build --workspace` | 3,673 s / 158 s (touch and edit) | 2,564 s (−30 %) / 34–37 s (−78 %); no-op 9.2 → 5.1 s | 18.0 → 5.6 GB | – | not measured yet: sccache is installed and used by `remote.sh`, but the single Windows slot stayed taken by other builds for over an hour (`scripts/build-profile.ps1` in two checkouts, as on the Mac) |

No-op builds take 0.3–0.8 s everywhere; the wasm check is cached by cargo like any other build. The machines were shared with other agents' builds during the measurements (load 3–28 on marge's 12 cores), so single numbers vary by ±20 %; before/after pairs were run back to back. Within a cold run sccache already hit 30–50 % (the same crate built for build, test and clippy). Across checkouts it hits 80–87 % of the compile requests but saves only 14–28 % of the time: the misses are the big crates (Bevy, wgpu, naga, x11rb, …), whose inputs name the checkout path (the `OUT_DIR` of build scripts ends up in the metadata of every crate built on them), and binaries, proc macros and build-script runs are never cached. `SCCACHE_BASEDIRS` (set by `remote.sh`, client-side hashing) did not change that for Rust (Mac: 437 s → 477 s at higher load), and `--remap-path-prefix` with the checkout path made every Rust key unique (0 % hits). The full win needs the same path: a checkout rebuilt after `make remote-gc` or `cargo clean` takes Bevy from the cache (−62 to −79 %).

Decisions:

- **Debug info:** `[profile.dev]` `debug = "line-tables-only"` for the workspace crates (backtraces keep file and line), `debug = false` for the dependencies. Full debug info made each checkout's `target/` 30–80 GB on marge (148 GB in the main checkout) and is what the links of the many Bevy test binaries spent their time on.
- **Build caching across agent checkouts, without interference.** Every agent worktree keeps its own remote checkout and its own `target/` – a `target/` is never shared (cargo locks it for a whole build, so the two slots would run one after the other, and different feature sets and checkouts would overwrite each other's artefacts and binaries). What was built and measured:
  - **Compiler cache:** `sccache` on the machine's local disk, content-addressed (a hit needs the same compiler, arguments, environment and sources, so parallel checkouts cannot hand each other a wrong artefact), shared by all checkouts there. Pinned release, SHA-256 checked: `make deps-sccache` (Linux x86_64, macOS arm64; `~/.local/bin/sccache`), `make deps-sccache-windows MACHINE=…` (MSVC build, `%USERPROFILE%\.local\bin\sccache.exe`, `scripts/install-sccache.ps1`); installed on marge, the Mac and Win11Builder. `scripts/remote.sh` sets `RUSTC_WRAPPER` on all three whenever it is installed (`OCW_SCCACHE=0` turns it off; cache size `sccache_size` in `machines.toml`, default 30 GB). Most dependency compile requests hit across checkouts (80–87 %), the big crates do not (see above); workspace crates of debug builds are incremental and compiled as before; binaries, proc macros and build-script runs are never cached.
  - **Seeding:** a new checkout (no `target/` yet) on Linux or macOS starts with a copy-on-write clone of the main checkout's `target/debug`, `target/release` and `target/build-voice` (`cp --reflink=always` on Btrfs/XFS, `cp -c` – clonefile – on APFS), so cargo finds the dependencies built; the clone shares disk blocks until a file is rewritten. A probe file tells whether the file system clones; where it does not (marge: ext4, Windows: NTFS) nothing is copied and sccache does the work. Skipped while a build runs in the main checkout (no half-written artefacts). **Opt-in** (`OCW_SEED=1`): on the Mac the clone took 46 s and no extra disk, but cargo still rebuilt 358 of 529 crates (899 s, slower than a clean build under the same load), because the main checkout's `target/` came from another dependency and profile state; so seeding stays off until a seed from the same dependency state is measured to beat sccache alone.
  - **One build directory per feature set (measured, opt-in):** the premise was that alternating plain and voice clippy runs in one `target/` rebuilt each other's dependencies. `scripts/cargo-shim/cargo`, first on `PATH` with `OCW_CARGO_SPLIT=1` (the Makefile keeps it first via `OCW_CARGO_SHIM`), gives a cargo call naming the voice feature `CARGO_BUILD_BUILD_DIR={workspace-root}/target/build-voice` (cargo's `build.build-dir`; the binaries still land in `target/debug` and `target/release`). Measured on marge: without the split, plain / voice / plain / voice clippy took 199 / 153 / 0.7 / 0.5 s (with `CC=clang-21` only on the voice calls, as `make` and `deploy-test.sh` do; 98 / 81 / 0.3 / 0.4 s with the same environment) – cargo 1.99 keeps both feature sets' artefacts side by side and nothing is rebuilt; with the split 153 / 216 / 0.7 / 0.7 s and 2.3 GB more, because the voice build compiles the shared dependencies again. So the split stays off; the commands users type are unchanged either way.
- **Disk:** `make remote-gc` (`scripts/remote-gc.sh`) lists the `target/` of every remote agent checkout whose branch is gone, or merged into main with its worktree removed (a new agent's branch looks merged: it has no commits yet), with its size per machine; `GC=1` deletes them. The main checkout, the sccache cache, the checkouts themselves and checkouts with a running or waiting task are kept. On 2026-11-13 it found 394 GB on marge, 77 GB on the Mac and 12 GB on Win11Builder.
- **Cleanup on merge (v2.63):** `scripts/remote-gc.sh --branch <branch>` (`make remote-gc-branch BRANCH=…`, `--dry-run` lists only) deletes the `target/` of that branch's checkout on every reachable machine (unreachable ones are skipped); the coordinator runs it after merging or abandoning an agent branch. A checkout with a live task marker (running or waiting) is never touched – the machine checks again right before deleting, and `target/` is renamed before it is removed, so a run starting meanwhile never sees half of it. Deletions are logged like the janitor's.
- **Disk watcher and janitor (v2.63, `tools/build-dashboard/janitor.py`):** the dashboard's poll reports per machine the disk of the home directory and, if another, the one holding the checkouts (Windows: the checkouts' drive and C:, which holds the user profile), as df's use (used / (used + free)). When the checkouts' disk is above a threshold, the janitor frees space, one machine per pass and at most one pass every 10 minutes (the fullest machine first):
  - **> 75 %:** the `target/` of agent checkouts whose branch is merged into main (worktree removed) or gone, unused for 2 h (last use: `.ocw-last-run`, which `remote.sh` touches on every run, else the newest of `target/*/deps`, `.fingerprint`, `incremental`); and in the main checkout's `target/` the build units unused for 3 days that a newer unit of the same crate and kind replaced – a unit is cargo's `<crate>-<hash>` (fingerprint, `deps/` files with that hash, build-script output), its age the later of access and modification time of its files (marge mounts with `noatime`), the kind (lib, bin, test, build script, its run) from the fingerprint's file names – plus incremental session directories unused for 3 days (like cargo-sweep; a unit still in use is at worst rebuilt from sccache);
  - **> 85 %:** also the `target/` of the least recently used agent checkouts idle for more than 24 h (sources kept), oldest first, until the disk is estimated below 85 %;
  - **> 92 %:** a red banner on the dashboard and a log line (no other notice).
  Never deleted: the sources of any checkout, checkouts with a running or waiting task, the sccache cache, `data/` (world data), `test-output/` (the art project keeps everything: its size on the coordinator is only shown), anything not ending in `<checkout>/target` or inside the main checkout's units. Every deletion (and every planned one in dry-run mode, once per plan) is a JSON line in `test-output/build-log/DAY/janitor.jsonl` (machine, path relative to the checkouts' directory, size, reason, threshold) for the art project. `--janitor off|dry-run|on` (default on); `--janitor-plan` / `make janitor-plan` prints what each threshold would delete on every reachable machine now. The decisions are pure and unit-tested (`test_janitor.py`, also the sweep against a scratch `target/`).
- **Dev profile change (v2.63):** `remote.sh` keeps a hash of Cargo.toml's `[profile.dev*]` / `[profile.test*]` sections in `.ocw-profile-hash` next to `target/`; when it changes, `target/debug` is removed once before the build (cargo would rebuild everything anyway and keep the old artefacts beside the new ones), unless another run of the checkout is building; logged to `janitor.jsonl`. `OCW_PROFILE_SWEEP=0` turns it off.
- **Fixed limits:** the sccache cache stays at 30 GB per machine (`SCCACHE_CACHE_SIZE`, the machine's `sccache_size`). On Windows the toolchain and the cache move off C: (the system drive, which also holds the user profile) to the build drive: `scripts/windows-move-toolchain.ps1` (`make windows-move-toolchain MACHINE=…`, a dry run with the sizes on C: by default; `APPLY=1` copies with robocopy, checks file count and bytes, sets the user variables `CARGO_HOME`, `RUSTUP_HOME`, `SCCACHE_DIR` and `PATH`, verifies `cargo --version` and `rustc --version` from the new place, restores the old variables on failure, and only then removes the old directories); `remote.sh` puts `%CARGO_HOME%\bin` first on `PATH`.
- **Tree copy (v2.63):** `remote.sh run` / `sync` take a **snapshot** of the working tree (tracked and untracked, not ignored, without `config/`) into a temporary directory the moment they are called; a run syncs that snapshot only once it holds its build slot: the remote side prints `@@ocw-sync PID` after taking the slot and waits for a line on stdin, `remote.sh` syncs and answers `go` (or `fail`), then closes stdin (Linux, macOS; on Windows opt-in with `OCW_WIN_HANDSHAKE=1`, because in its first runs the PowerShell side stayed waiting after the sync and it could not be tested further – by default Windows syncs the snapshot right away, before waiting for its slot). Before, a run waiting for a slot could build a tree it was not called with (a half-merged main checkout, or a later run's sync). **No source mtimes:** rsync and tar kept the files' original mtimes, so a file replaced by an older version (a revert, a branch switch) looked older than the build that had compiled the newer one, and cargo kept the stale artefact. Now rsync compares by content (`--checksum`, no `-t`): unchanged files are not touched, changed ones get the time of the copy. Windows (tar): a manifest in the checkout (`.ocw-sync/manifest.tsv`: SHA-1, size and mtime of each file as written) tells which files are unchanged there; only the others are sent, unpacked with `tar -m`. Nothing relies on the remote mtime being older: a changed file is always written at the time of the copy, so it is newer than any earlier build there – also after the remote files were touched into the future to force a rebuild and newer content then arrived with an older local (merge-time) mtime, which left stale artefacts on Win11Builder on 2026-11-13 (regression test in `scripts/test-remote.sh`); a file touched on the machine no longer matches its manifest entry and is re-sent. **New checkouts (v2.95):** the manifest diff used awk's `NR == FNR`, which is also true for the second file when the first (the machine's list of unchanged files) is empty – so a new agent checkout on Win11Builder got "sending 0 changed of 397 files", only `.ocw-sync/` and an empty manifest, and cargo failed without `rust-toolchain.toml`. Now: the list is read with `getline`; a file is only skipped when it exists there with the manifest's size and mtime and the manifest's header names this checkout (`# ocw-sync v2<TAB>REPO`; a copied or older manifest counts as none, so existing checkouts get one full send); the manifest is only written when `tar.exe` succeeded; and when files of the snapshot are missing there after the sync, the whole tree is sent once more (still missing = failed sync). `scripts/test-remote.sh` tests the snapshot, the mtimes, the handshake, the manifest diff, the Windows sync against a simulated machine (new checkout, re-sync, files deleted there, stale manifests, failed extraction, the safety net) and the profile check.
- **Open questions in the dashboard (v2.63):** `tools/build-dashboard/spec_questions.py` reads every `- **Q<n> – …**` bullet of this spec (mostly the *Open questions (for the user)* subsections of §14): phase, section, id ("Phase 12 Q1"), title, text, the proposal ("Proposal: …", "(proposal)", "(recommended)"), and the status – decided when the bold label says *decided* or *postponed*. Milestones that name an open question are listed as blocked work. The spec stays the single source; the dashboard re-reads the main checkout's file when its mtime changes and shows the questions by phase (decided ones behind a toggle), in `--once` and in `/api/status`. **Answering on the dashboard:** each open question has an answer box and quick buttons ("Accept proposal", the options it lists); an answer is appended to `test-output/answers/answers.jsonl` of the main checkout (git-ignored, for the art project too; ISO time, question id, the exact question text from the spec, answer; a later answer supersedes an earlier one) and as one line to `test-output/answers/inbox.log`, which the coordinator watches (also `GET /api/answers?since=<ts>`) to record the decision in this spec; until then the question shows "answered – waiting for the coordinator", afterwards the parser shows it decided. Writing needs the random token in the git-ignored `config/dashboard-token` (created on first start, 0600; entered once on the page, kept in the browser, sent as a header; 403 without it); reading stays open; the token is never logged. Mobile, keyboard and touch usable; all text HTML-escaped (tested in `test_answers.py`, also the round trip, superseding and the 403).
- **Machines enabled/disabled and their slots (v2.66):** each machine card of the build dashboard has an Enabled/Disabled toggle (disabling asks for an optional reason), a build slots stepper (1–8; 0 = no limit only by hand or with `OCW_REMOTE_SLOTS=0`) and a short note; changes need the dashboard token (403 without it; machine names only from `machines.toml`, slots a JSON integer, text without control characters, at most 200 characters, HTML-escaped on the page). They are stored in `config/build-machines.toml` of the main checkout (git-ignored, the `[[machine]]` format of `machines.toml`, which stays the user's hand-edited file; `tools/build-dashboard/build_machines.py list|set` from a shell) and logged as JSON lines in `test-output/build-log/DAY/machines.jsonl` (who, when, old and new values). `scripts/remote.sh` reads it: `run` and `sync` on a disabled machine fail at once with the reason and exit code 3; `ssh`, `field`, `fetch`, `put` stay allowed, and deploys too (`deploy-test.sh` sets `OCW_REMOTE_DEPLOY=1`, so a kiosk on a disabled machine still gets new builds); `remote.sh MACHINE enabled` / `slots` answer without ssh (the multiplayer test skips disabled bot machines). Slot count: `OCW_REMOTE_SLOTS` (one call) > the dashboard's > `machines.toml` > OS default; a changed count applies to runs started afterwards, a running run keeps its slot (also one above a lowered count) and finishes normally. The dashboard's poll and the janitor still probe disabled machines (read-only, and the janitor's `target/` cleanup); disabled machines' slots are left out of the overview's total. Initial state (2026-11-13): leela disabled ("excluded by the user", kiosk only), Win11Builder enabled with 1 slot for Windows-related builds only, marge (2) and the Mac (1) enabled. Tested in `test_build_machines.py` (parsing, defaults, the endpoint, escaped cards) and `scripts/test-remote.sh` (refusal, deploy exception, slot precedence, a lowered count with a run still in a higher slot).
- **Build slots in order of arrival:** a waiting run leaves a free slot to an older waiting run (Linux/macOS; before, the first poll won and a run waited 28 min while later ones went ahead; the Windows path still takes the first free slot). `scripts/test-remote.sh` (`make test-remote`) tests the rule and the choice of `remote-gc`.
- **Build log:** each run in `test-output/build-log/*/runs.jsonl` also records `sync_s` (copying the tree: 341 files, 7.7 MB, about 1 s – not worth optimising), cargo's own `Finished … in` time (`cargo_s`), the crates compiled (`compiled`) and the build-slot wait (`wait_s`); the art journal (`art/diary.py`) shows compile hours, crates and waiting hours per machine.
- **Rejected:** a faster linker on Linux (Rust 1.99 links x86_64 Linux with `rust-lld` already; the incremental builds are 2–13 s); `lld-link` on Windows (the MSVC CRT setup of the voice backend, `+crt-static`, would need its own verification, and Windows builds are rare); Bevy feature trimming (UI, picking, audio, glTF, gamepads are used; `bevy_ui_render` pulls in `bevy_sprite_render`; it would need a screenshot check for each feature for a gain of a few percent of a clean build); a shared `CARGO_TARGET_DIR` or build directory per machine (see above); seeding by a full copy or hard links where the file system cannot clone (a copy of 30+ GB takes minutes; hard links are unsafe because cargo rewrites fingerprint files in place); incremental release builds (shipped binaries stay as they are); limiting the wasm check to changed crates (a no-op check takes under a second).

**CI resilience (v1.9):** the Linux job's package installation retries with short timeouts and a step timeout, so an unreachable distribution mirror fails within minutes instead of holding a runner for hours.

#### 11.2.1 Build dashboard

The **build dashboard** (`tools/build-dashboard/`, details in its `README.md`) is the one place to see and steer the build infrastructure; it was asked for by the user on 2026-11-12 and grown since. Requirements:

- **What it is:** a small web page and JSON API (`/api/status`), Python 3 standard library only, running on the coordinator (deb) as the systemd user service `ocw-build-dashboard` on port 8765 (`make dashboard-install`; `systemctl --user restart ocw-build-dashboard`); `--once` prints the status as text, `--once --json` as JSON. It reads the main checkout's `config/` and never shows host names, users, addresses or keys (everything goes through the same masker as the build log).
- **Machines:** every machine of `config/machines.toml` polled every 5 s over one SSH call (read-only snippet, shared control socket): online / unreachable with the error kind, CPU, load, memory, swap, disks (home and checkout volumes; Windows: the checkout drive and C:), uptime; the kiosk state where `/var/tmp/ocw` exists (app running, deployed build, connected Bluetooth devices by name).
- **Jobs:** the running `remote.sh` runs (from their task markers) with checkout, command, running time, slot and the CPU / memory per process kind; the queue of runs waiting for a build slot (oldest first); the slots in use per machine. **Each job is described in one sentence by what it tries to achieve** (decided 2026-11-13, replacing the user's prompt): built by fixed rules from the command (checks, tests, wasm, release builds, deploys, world builds, renders, profiling …) and its context (the agent's task, or the latest commit / merge of the main checkout), at most 140 characters, the full breakdown as a tooltip (`--show-prompts` still puts the prompt into the JSON).
- **Machines enabled / disabled and build slots** (v2.66): a toggle and a slot stepper (1–8) per machine card, with an optional reason and note; stored in `config/build-machines.toml`; `scripts/remote.sh` refuses `run` / `sync` on a disabled machine (deploys stay allowed) and takes the slot count from it.
- **Disk janitor** (v2.63): disk use per machine, automatic cleanup of build output above 75 % / 85 %, a red banner above 92 %; never sources, data, test output, the sccache cache or busy checkouts; every deletion logged.
- **Open questions and answers** (v2.63): every question for the user in this spec, grouped by phase with its proposal and options, decided ones hidden; **answerable on the page** (text box, "Accept proposal", option buttons) with the token from `config/dashboard-token`; answers go to `test-output/answers/answers.jsonl` and `inbox.log`, which the coordinator watches and records in this spec (the question then shows as decided). Blocked milestones are listed.
- **Build log and art project:** every `remote.sh run` is recorded in `test-output/build-log/DAY/` (`runs.jsonl` with machine, command, times, exit code, sync / cargo / slot-wait seconds, and the run's output; `machines.jsonl` and `janitor.jsonl` for machine and janitor changes) – material for the OCW Build Diary (`art/ART-PROJECT.md`).
- **Security:** reading is open on the LAN; every change (answers, machine settings) needs the token (header `X-OCW-Token`, 403 without it); all text shown is HTML-escaped; inputs are validated (machine names only from `machines.toml`, slots 1–8).
- **Phone friendly:** the page works at phone width and with touch, keyboard and mouse.
- **Tests:** `python3 -m unittest discover -s tools/build-dashboard -p "test_*.py"` (run on a build machine via `remote.sh`), no network needed.

### 11.3 Packaging

**Linux:** AppImage, Flatpak, optionally native packages.

**Windows:** EXE/MSI, code signing, installer.

**macOS:** app bundle, code signing, Apple notarization, DMG.

Build and release pipelines are automated for all three operating systems, introduced in the rollout order (§11.1).

**Implemented so far (v1.9):**

- **App icon and desktop integration:** the logo (`artwork/logos/`, a unicorn riding a bike over a rainbow) is the window icon (X11, Windows); the window's app id `ocw-desktop` matches the desktop entry (`packaging/linux/ocw-desktop.desktop`), which Wayland compositors, docks and menus use. On Linux the app installs its desktop entry and hicolor icons into the user's data directory at start (only changed files).
- **AppImage:** `scripts/build-appimage.sh` assembles the AppDir (binary, desktop entry, icons, `.DirIcon`) and packs it with `appimagetool` when installed.
- **macOS app bundle:** `scripts/build-macos-app.sh` (`make macos-app`, runs on macOS) builds `Open Cycling World.app` (bundle id `org.opencyclingworld.desktop`; stripped binary; `Info.plist` with version + commit, minimum macOS read from the binary, `NSBluetoothAlwaysUsageDescription`, without which macOS ends the app when it uses CoreBluetooth; `ocw.icns` from `artwork/logos/`), signs it ad hoc and packs a DMG (with an Applications link) and a zip. Started from Finder, the app logs to `~/Library/Logs/OCW/ocw-desktop.log` (stderr has no terminal there). CI builds and uploads both on pushes to `main`. Still open: Developer ID signing with the hardened runtime and notarisation (`CODESIGN_IDENTITY` already signs with a given identity), a 1024 px icon render for the 512@2x slot.
- **Versions (decided 2026-11-13, changed in v2.49):** every GitHub release increments the **patch** (last) version of the workspace (`[workspace.package] version` in `Cargo.toml`, inherited by every crate; 0.2.0 → 0.2.1 → 0.2.2 …), and the tag is `vX.Y.Z`. (Until v2.49 the minor version was incremented: 0.1.0 → 0.2.0.) All of it is in the Makefile: `make release-check` (current, latest released and next version), `make release-prepare` (on a clean, pushed `main`: bump with `make version-bump`, commit "Release X.Y.Z", push; the release binaries are built from that commit, so `--version` reads `X.Y.Z-<commit>`), `make release-publish DIR=… NOTES=…` (refuses unless the version is above every existing release; tags, pushes the tag, creates the pre-release with `SHA256SUMS`). Minor / major bumps only by hand (`make version-bump PART=minor|major`). Tested by `make test-release-version`. Older releases were tagged `v0.1.0-<commit>`.
- **Releases:** binaries and world packages are published as **GitHub Releases** (pre-releases while in development), never committed to git (decided 2026-11-12; the history was rewritten to remove the former `release/<version>-<commit>/<platform>/` binaries, `release/commit-map.txt` maps old to new commit ids). Release binaries are built with voice (`--features ocw-desktop/voice`, Windows with the static C runtime). Each release carries the AppImage and a tarball of the stripped Linux binaries, a Windows zip, the macOS DMG, the world package with its manifest and data licence (ODbL, §13), the OSM extract it was built from (the ODbL "offer the data" duty) and `SHA256SUMS`. `release/README.md` keeps the build notes. Screenshots and videos made during development are not in the repository (removed from the history on 2026-11-12, `artwork/build-history/` dropped entirely): they are published as one zip per day (`ocw-build-history-YYYY-MM-DD.zip`) with an index (`index.md`) on the GitHub release `build-history`. New test runs land in the git-ignored `test-output/` (with its `index.md`); the ones worth keeping go into that day's zip.

---

### 11.4 Testing Strategy

**Rule: no step is done without tests.** Every development step in §14, every feature and every pipeline step comes with unit tests that are merged together with the code. CI runs them on every change; a failing test blocks the merge.

| Level | What | Where | When |
|---|---|---|---|
| **Unit tests** | Functions and modules in isolation: physics, route sampling, protocol encoding, session logic, parsers, pipeline steps | `#[cfg(test)]` modules next to the code in every crate | Every commit, all platforms in CI |
| **Integration tests** | Several crates together: client ↔ local server over the protocol, world-builder on a small OSM extract, device backend ↔ app channel | `tests/` directory of the crate | Every commit (Linux); per platform once ported |
| **End-to-end smoke test** | The real desktop app under Xvfb with software rendering (Mesa lavapipe): launch, ride on the virtual trainer, run console commands, take a screenshot, exit | `tools/` / run skill driver | Every commit on Linux |
| **Scene renders** | Reproducible scenes of the real desktop app (`scripts/scenes.toml`: arguments, timed console commands, screenshot / short video; a scene names the world package it needs, `world = true` for Liechtenstein, `world = "blackforest"` for the Black Forest) rendered headlessly on Wayland on a Linux build machine (`scripts/render-scenes.sh`, `make test-scenes`); reviewed by eye, the good ones kept in the build history | `scripts/` + run skill driver | After user-visible changes, before releases |
| **Multiplayer over the network** | Server + kiosk client on one build machine, headless `ocw-bot` riders on the others (Linux, macOS, Windows); joins, snapshots, visibility, prediction, tick timing; kiosk screenshots and video | `scripts/test-multiplayer.sh` (`make test-multiplayer`, §11.4.1) | On demand, before Phase 5 milestones |
| **Hardware-in-the-loop** | Real trainers and sensors (FTMS, power meter, heart rate) | Manual test protocol, later a test bench | Before each release |
| **Performance tests** | Frame time, streaming hitches, memory (§15) | `tools/performance-test` (console `exec` scripts, §2.9) | Nightly / before releases |

**Principles:**

- **Tests run without hardware, network or GPU.** Devices are replaced by the virtual trainer or mock backends (`DeviceBackend`, §3.4); network transports by in-process channels; rendering-free logic lives outside Bevy (§10) and is tested headless.
- **Core crates are tested for wasm too:** `ocw-core`, `ocw-sim`, `ocw-protocol` and `ocw-world` must build for `wasm32-unknown-unknown` (§3.5).
- **Physics tests check physical plausibility**, not only code paths: steady-state speed for given power and gradient, energy balance, no rolling backwards, stability at the 50 Hz tick.
- **Protocol compatibility tests:** encoded messages of earlier protocol versions are kept as fixtures and must still decode (§6.2).
- **World-builder tests** use small, checked-in OSM / DEM fixtures with known expected results (golden files), e.g. a junction, a bridge, a dead end.
- **Test media carry their time (decided 2026-11-12, v2.30):** every picture and video the app writes has the ISO 8601 timestamp in the image (HUD, §3.2) and in the file: screenshots (console `screenshot`, `--screenshot`, `--at "T=screenshot …"`) are **JPEG, quality 90** by default with EXIF `DateTime`, `DateTimeOriginal`, `DateTimeDigitized` (`CreateDate`) in local time, `OffsetTime` / `OffsetTimeOriginal` / `OffsetTimeDigitized` (`+01:00`) and `Software` = `Open Cycling World <build id>` (`ocw-render::media`, own TIFF writer, the image crate's JPEG encoder); a path ending in `.png` gives a lossless PNG with `tEXt` chunks `Creation Time` (the registered keyword, ISO 8601 with offset) and `Software`. PNG stays where the picture is the input of another step: the `--capture` frames (lossless input of the H.264 encode, each stamped with its stepped time); `--capture` also writes `still.jpg` (last frame) and `metadata.txt` (ffmpeg metadata: `creation_time` of the first frame in UTC with `Z`, `comment` = software), which `drive.sh` maps into the MP4. MP4s the app encodes itself (V) get `creation_time` (start of the recording, UTC) and `comment`. The scripts that re-encode media keep it (`render-scenes.sh --pick` copies the EXIF / text chunks into a shrunk image and the metadata and sound into a re-encoded video). Test pictures and videos are **2560×1440** (`drive.sh` `OCW_SIZE`, default of the scene harness, the multiplayer camera client; the kiosk keeps its screen), videos H.264 CRF 18 with `-preset slow` (`OCW_X264_PRESET`), yuv420p, `+faststart`, 25 fps, AAC sound.
- **Build history:** the harness keeps every scene reproducible (fresh config and data folders, virtual trainer, fixed start, world package from `--world` / `$OCW_WORLD` / `data/pilot/` / the kiosk install); `render-scenes.sh --pick` copies the reviewed renders, kept small (images above 600 KB: PNG palette or JPEG quality ~80, metadata kept; videos ≤ 3 MB, sound and metadata kept), into `test-output/build-history/` with an index, for the zip on the GitHub release `build-history` (media are not committed to git).
- **Regression rule:** every fixed bug gets a test that failed before the fix.
- Commands: `cargo test --workspace` runs all unit and integration tests; `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- **Make targets (decided 2026-11-12):** every test is a `test-*` target: `test-unit` (`cargo test --workspace`; `make test` runs the same), `test-wasm` (the §3.5 build check), `test-kiosk` (`scripts/test-kiosk.sh` with a fake app), `test-screenshot` / `test-screenshot-wayland` / `test-smoke` (both) and `test-video` / `test-video-wayland` (headless desktop runs, run-ocw skill), `test-multiplayer` (§11.4.1) and `test-multiplayer-local` (the same pieces on one machine over loopback: `ocw-server` with a bot, three `ocw-bot` riders, the verdict; in CI). `check` (= `lint test-unit test-wasm`), `lint`, `fmt`, `clippy` keep their names – they are checks, not tests – and so does `deploy-test` (a deployment). Options for the old names (`smoke`, `screenshot`, `screenshot-wayland`, `video`, `video-wayland`, `wasm`): remove them at once (clean, but breaks habits and notes), keep silent aliases forever (no reason to move), or **deprecated aliases** that print "use make test-…" and run the new target (chosen); they are left out of `make help` and can go once nobody uses them.

#### 11.4.1 Multiplayer Test over the Network (decided 2026-11-12)

**Purpose.** The loopback tests (`services/server/tests/quic_loopback.rs`) prove the QUIC transport on one host. `make test-multiplayer` (`scripts/test-multiplayer.sh`) checks the whole Phase 5 path (§6.2, §14) on real machines and a real network: several operating systems as clients at once, one remote server, the GUI client among them, and a picture of the result on a screen. It is an on-demand test (it needs the build machines of `config/machines.toml`, a few minutes and the kiosk screen), not part of CI; CI runs `make test-multiplayer-local`, the server, three bot riders and the verdict on one machine over loopback.

**Topology and roles.**

| Machine (`config/machines.toml`) | Role |
|---|---|
| `marge` (Linux, the kiosk screen; `OCW_MP_SERVER`) | `ocw-server --listen 0.0.0.0:47800 --bots 0 --seconds S --stats-file …`; the kiosk's `ocw-desktop`, which joins it over `127.0.0.1` as rider "Kiosk"; if needed a headless `ocw-desktop` "Camera" (below); one bot process; the verdict |
| `macos`, `Win11Builder`, `leela` (`OCW_MP_BOTS`) | one `ocw-bot` process each with `OCW_MP_RIDERS` (default 2) riders, joining the server over the LAN |

Machines that do not answer ssh within the connect timeout are skipped and listed as such; the server machine must be reachable. Riders are named by OS (`lin-1`, `mac-2`, `win-1`), never by host name. The script first builds the release binaries on all machines in parallel (`ocw-server` + `ocw-bot` on the server machine, `ocw-bot` elsewhere; `remote.sh` queues for build slots).

**Bot client (`apps/bot`, binary `ocw-bot`).** Options considered: a headless mode of `ocw-desktop` (reuses `ClientCore` exactly, but `ClientCore` uses the renderer's view types, so every bot machine would build Bevy – about an hour on the 8 GB Windows machine – for a client that never draws); moving `ClientCore` out of the desktop crate (right eventually; a larger refactor of the view types); **a small Bevy-free crate (chosen)** that builds in minutes everywhere and reuses the pieces `ClientCore` is made of: `NetClient` (§6.2), `Predictor` and the drafting rule (`ocw-sim`), the protocol helpers, the server's `physics_profile` (the prediction uses exactly the profile the server derives from the bot's Hello) and the `bot_bike` looks. Uncertainty: the bot and `ClientCore` can drift apart; the kiosk's real client in the same session covers the desktop path. A process runs `--riders N` riders, one QUIC connection each, in one 50 Hz loop: samples at 4 Hz like a trainer (power, a plausible cadence, brake), prediction every tick, reconciliation on every own snapshot with half the round trip as the snapshot's age. Pacing: its own power (`--power`, a few per cent apart per rider, ±10 % slow variation), or `--follow NAME`: keep 6, 10, 14 … m ahead of that rider (wanted speed from the gap error; the leader's power plus a speed correction; brake when much too fast), so the remote riders gather in front of the kiosk's camera; without that rider in the session a bot rides its own power. `ocw-bot verdict` evaluates a run (below). All logic is in the library and unit-tested against an in-memory `GameServer`; the binary only adds the network loop.

**Configuration and address discovery.** The server address is the server machine's `host` from `config/machines.toml`, read by the script with `scripts/remote.sh MACHINE field host` and passed only on the bots' command lines (`OCW_MP_HOST` overrides it, e.g. for another interface). It is never shown: everything the script prints or keeps goes through a filter that replaces the `host` of every machine involved, the peer addresses in the server's connection log (IPv4 or IPv6) and every other IPv4 address except loopback (ssh errors and the server's log contain them). Options considered: mDNS / broadcast discovery (no configuration, but multicast through every firewall and a new dependency, while the address is configured anyway); a rendezvous service (comes with the backend, §9). Further settings: `OCW_MP_SECONDS` (server run time, default 180 s), `OCW_MP_PORT` (47800), `OCW_MP_RIDERS`, `OCW_MP_KIOSK=0` (bots only), `OCW_MP_CAMERA=auto|1|0` (the camera client), `OCW_MP_BUILD=0` (skip the builds), `OCW_MP_T1` (app time of the kiosk's first screenshot).

**Network, port and TLS.** One UDP port (47800, §6.2), no TCP. TLS as in §6.2's development mode: the server keeps its certificate in the checkout's `target/mp/tls` (`--tls-dir`); the script reads the SHA-256 fingerprint from the server's output and passes it as `--server-cert` to every bot and to the kiosk (pinning; `--insecure-dev` is only valid for loopback and not used). **Firewalls are never changed** by the test. A blocked port shows as bots without an answer: the QUIC handshake times out ("no answer from the server" after the 15 s idle timeout; a bot gives up on a rider without a Welcome after 25 s), the bot prints a hint (is the server running, does a firewall let UDP 47800 through?), and the script reports per machine that UDP 47800 on the server machine seems blocked from there and what to open. The bot on the server machine itself separates "server down" from "port blocked from outside". A port already in use (another server) fails at the start with a message.

**World data and course.** Both sides must ride the same course (§6.2 course check, §6.5). The kiosk normally rides a route file (`--route-file …/vaduz-malbun.route`, with the world package for the terrain, §4.7), i.e. a 1D `Route`. The script reads the kiosk's effective arguments on the server machine (its command line and `ocw-desktop.args`) and makes the server ride exactly that: `ocw-server --route-file` (new) with the kiosk's file, which the script copies to every bot machine (`scripts/remote.sh … put` into the checkout's git-ignored `target/mp/`); the bots load it with `ocw-bot --route-file`. Every side builds the route with the same `RouteFile::to_route`; the server records the file's SHA-256 in its statistics, every bot reports the SHA-256 it loaded, and the verdict compares them and the route ids (a bot also refuses a Welcome for another route, like the desktop). Without a route file the kiosk's test route is used (`--test-route` / `--route`); without a kiosk the test loop. Options considered: every bot machine fetching the pilot data and building the world package (slow and large, and the bots need no terrain); sending the route in the Welcome (a protocol change for a test); thin bots that send no world and do not predict (no desync check). Limitation: the bot rides no road graph (free ride and package routes need the package's graph and the path messages of §6.5); with a kiosk in free ride the run is "bots only".

**Duration and timeline.** The server runs `OCW_MP_SECONDS` (180 s) and then shuts down, which ends every client normally ("server shutting down"); the bots' own limit is 60 s longer, as a safety net. The kiosk's app rides at 200 W from its start (virtual trainer); the bots, joining a few seconds later behind it, catch up (at most 4 m/s faster) and stay 6–14 m ahead of it. A first idea – the kiosk waits at 0 W and `--at "T=sim.power 200"` starts it once the bots are there – does not work: `sim.power` is a dev command, refused on a remote server (§2.9).

**Kiosk handling.** The kiosk (`ocw-kiosk.sh`, in the desktop user's session) cannot be restarted with other arguments over ssh, but it restarts the app whenever the binary changes, and the app reads `ocw-desktop.args` beside the binary first (the kiosk's own command line wins). So the script saves `ocw-desktop.args`, writes it with the saved content plus `--server 127.0.0.1:47800 --server-cert FP --name Kiosk --virtual --power 200 --record false` and `--cmd "fps on"` (the FPS counter in every picture), `--video-dir` and timed console commands (`--at`: three screenshots from app time T1 and a real-time video of `OCW_MP_VIDEO_S` (15) s from the app's own recorder, `video start` / `video stop`; a kiosk build without `--video-dir` gets the screenshots only), and replaces the binary by a copy of itself (a new inode: the kiosk restarts the app). Afterwards – also after a failure or Ctrl+C (exit trap) – it puts the saved file back and restarts the app the same way, but only if the file is still the one it wrote (a deployment in between wins). The kiosk is never left in multiplayer mode, and no FIT file is recorded for the test ride. The app (desktop user) writes its screenshots into a fresh `/var/tmp/ocw-mp/<run>/` (mode 0777) that the script owns and deletes afterwards.

**Locked kiosk screen and the camera client.** *Update 2026-11-12:* the cause is the present mode: the kiosk session is X11, and while the session is locked the screen is off; a window that waits for the display's refresh (vsync, the app's default) is then paced by the X server at one frame a second (measured: the FPS counter shows `1 fps  max 1000 ms`). With `--no-vsync` the same locked kiosk drew 312 fps at 2560×1600 on marge's GPU. So the script now gives a kiosk build that has them `--no-vsync --max-fps 60` for the test (60 fps at most, restored afterwards with the other arguments), and the kiosk takes its stills and video while locked too; the camera client starts automatically only without a kiosk or with an older kiosk build (`OCW_MP_CAMERA=1` still forces it). The rest of this paragraph describes the first runs. Found in the first runs: when the desktop session on the kiosk machine is locked (`loginctl … LockedHint=yes`), the compositor hides the app's window and lets it draw only about once a second. The app keeps riding (device samples still reach the server), but its game time – which `--at` uses and which advances at most 0.25 s per frame – crawls, so no screenshot is ever taken. The test cannot unlock the screen (it changes no system state), so it detects the lock, says so, and – with `OCW_MP_CAMERA=auto` (default: when the screen is locked or there is no kiosk), or always with `1` – starts a **camera client**: the desktop app (release build, `OCW_PROFILE=release`) under a headless weston on the server machine (`drive.sh shot-wayland`; weston's GL renderer, so the app draws on marge's GPU), joining as rider "Camera" with the same course and world package, at 200 W, taking three stills and a real-time 25 fps video of `OCW_MP_VIDEO_S` s with the app's own recorder (`video start` / `video stop`, `--video-dir`), with the FPS counter on and `--log-fps 5`. The bots then follow "Camera", so the pictures show the remote riders around it. Options considered: only reporting the missing pictures (no picture at all while the screen is locked); recording the screen of the other user's session (not possible over ssh without its credentials); `--at` on wall-clock time (changes the meaning of `--at` for scripted and captured rides). The camera client is a second real `ClientCore` in the session, so the desktop path is covered even when the kiosk cannot be watched.

**Render rate and videos (2026-11-12).** The first multiplayer videos looked choppy: 5 frames per second. Measured with the wall-clock render rate (`--log-fps 5`, FPS counter) on marge, release build, headless weston on the GPU, Vaduz–Malbun with the world package, 9 riders: single player (in-process server, 8 bots) and multiplayer (`ocw-server --listen` + 8 `ocw-bot` riders over QUIC, following the client) both draw 37–40 fps with vsync (the headless weston paces there; even a 320×180 window without riders reaches only 39.7 fps) and the debug build the same (39.5); without vsync both reach about 80–200 fps (varies with the view and with the other load on the machine – builds, the kiosk sharing the GPU – more than between the modes; no difference above that noise); the locked kiosk drew 1 fps (above). So the client has no multiplayer-specific bottleneck: snapshot handling is non-blocking (`NetClient::receive` drains a channel), the interpolation works on 9 riders at 10 Hz, avatars are moved, never respawned, and no mesh or material is added per snapshot (regression test `riders::moving_riders_keep_their_avatars_and_assets`); a rider's looks are stable per snapshot, so `sync_riders` does not rebuild them. The 5 fps came from the recording: the camera client took one screenshot per 1/5 s of game time and the script encoded them at 5 fps (the kiosk: 3 per second). Now both record with the app's own real-time recorder (console `video`, 25 fps, every tick a new image as long as the app draws ≥ 25 fps; the recorder's log line counts unique, repeated and dropped frames), the camera runs the release build, and the FPS counter is in every picture. Frame capture with a fixed game-time step per frame (`--capture`, what `make test-video` uses) cannot record a networked client: its game time must run in real time with the server's (`--server` conflicts with `--capture`).

**Checks and pass criteria** (`ocw-bot verdict`; every check is printed as PASS / FAIL with its numbers):

| Check | Passes when |
|---|---|
| Bot processes reported | every started process printed its `OCW-BOT-REPORT` line, with all its riders |
| All clients joined | every bot rider and the kiosk are in the server's `joined` list |
| No client left early | nobody disconnected before the server's end (it writes its statistics before closing the connections) |
| All riders at once | the server's peak rider count ≥ bot riders + kiosk |
| Real time, tick timing | a real-time run; mean tick work ≤ 25 % of the 20 ms budget, ≤ 0.1 % of the ticks over budget, ≤ 1 % of the ticks started a whole tick late |
| Same course | every bot's route id (and route-file SHA-256) equals the server's |
| Per rider | joined (no Reject, no course mismatch); ended normally (server shutdown or its own end; a reset or timeout counts as normal only if its last snapshot came within 5 s of the server's end – the server's closing message was lost – not earlier); ≥ 90 % of the 10 Hz snapshots over its connected time and no gap > 1 s; saw every other client by name and each of them moving (≥ 5 m); one snapshot with all riders; rode ≥ 20 m; the server applied its samples (`last_input_seq`); prediction error at reconciliation after a 3 s warm-up: mean ≤ 0.5 m, max ≤ 5 m (the snap threshold, §5.2) |

The thresholds are first guesses for a LAN (uncertain; adjust after more runs). **Suspended machines:** a bot compares the wall clock with the monotonic clock (which stops while a machine sleeps, on macOS) and reports the difference; the verdict notes it next to the rider's numbers. Found in the first runs: the Mac (on Wi-Fi) suspended for about 2 s twice a run, which showed as 2.6 s snapshot gaps, prediction errors just over 5 m and a broken link at the end; `pmset -g log` showed why: the Mac (system sleep after 1 min idle) was in dark wakes, entering "Maintenance Sleep" for 5–6 s every minute even while the bot ran. The script therefore runs the bot there under `caffeinate -i -s` (an assertion that prevents idle and system sleep while the process runs, on AC power; no setting is changed). The server's numbers come from `ocw-server --stats-file` (`ocw_server::stats`: ticks, mean / max tick work, ticks over budget, late ticks, peak riders, who joined and left, route id and SHA-256).

**Outputs.** On the terminal: machines used and skipped, the course, a note if the kiosk's screen is locked, the verdict's lines and `RESULT: PASS` / `RESULT: FAIL` (exit code 0 / 1). Kept in `test-output/multiplayer/<run>/` (git-ignored): `summary.txt`, the build, server and bot logs, `stats.json`, `verdict.txt`, the kiosk's arguments and `kiosk/` with the stills and `kiosk.mp4` (the app's recorder: 25 fps, at most 1920 px wide), and `camera/` with the camera client's stills, last screenshot, `camera.mp4` (25 fps, 1280×720) and its log `app.log`; the summary gives each video's size, frame rate and length, the camera's measured render rate (mean and lowest of the 5 s log lines) and its recorder's unique / repeated / dropped frames. Stills, video, verdict and summary are also archived like the run-ocw skill's runs (`test-output/YYYY-MM-DD/HHMMSS-<commit>-multiplayer-*` with a row in `test-output/index.md`, mode `multiplayer`).

**Limitations.** Route rides only (no road graph, see above); one server machine; no packet loss or latency injection (later: `tc netem` needs root, a userspace UDP relay would not); the videos are the app's own real-time recordings (another user's desktop session cannot be recorded over ssh): a tick the app did not draw repeats the previous frame, so they are only as smooth as the app's render rate (the FPS counter in the picture shows it); frame capture with a fixed game-time step (`--capture`) cannot be used, because a networked client's game time must run in real time with the server's; the kiosk takes pictures only while its screen is unlocked; the kiosk's trainer is the virtual one during the test; the thresholds are uncertain; the test uses the kiosk build that is deployed – if its protocol version differs from the server's, it is rejected and "all clients joined" fails (deploy first: `make deploy-test`).

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
| Tiling materials (`assets/materials.toml`, v2.46) | External, CC0 1.0 (ambientCG, Poly Haven) | Not in git: pinned by URL + SHA-256, fetched and converted by `scripts/fetch-materials.sh`, shipped beside the binary with `CREDITS.md`; credited on the credits list (console `materials`) although CC0 needs no attribution; the validator rejects share-alike, non-commercial, no-derivatives and unknown licences (Phase 8A) |
| Road graph + attributes in world packages | **Derivative Database** (ODbL) | Published under ODbL; extracted data offered for download alongside each package |
| Meshes, textures, rendered views | **Produced Work** (ODbL) | Attribution "© OpenStreetMap contributors" |
| DEM-derived terrain | Depends on source | Each source's required attribution notice (Copernicus, regional providers) |
| Imagery and land cover | Depends on source | Only sources permitted in §4.6; attribution per source. T3 (v2.47): Orthophoto Liechtenstein 2025 (Opendata BY, ATG / swisstopo), swisstopo SWISSIMAGE (OGD), LGL DOP20 (dl-de/by-2-0), IGN PNOA (CC BY 4.0 scne.es), Copernicus Sentinel-2 ("Contains modified Copernicus Sentinel data 2023-2025"): stored with each package (section `PALT`), copied into its manifest's attribution, hence into `LICENSE-DATA.md` of the release zips, the credits list and *World → About this world → Data sources* |

- **Licence change considered and declined (2026-11-13):** switching to CC BY-SA (for the code, or code and assets) was discussed (CC licences suit assets, not software: no patent grant, no source obligation, narrower compatibility) and the user decided to **keep MIT / Apache-2.0** for now; rule 8A (permissive third-party licences only) stays.
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
- **Workshop scene, parts and hearts (planned 2026-11-13, v2.53):** 4A–4L below.
- **Input model for the whole UI** – keyboard, mouse, gamepad, touch (v2.54, §3.2.2, M10).

- **Unit tests:** component and configuration model (IDs, physics profile references, cosmetic parts never change physics), profile caps, persistence round-trip, animation state selection from cadence / speed / riding state, asset validator rules (including licence metadata).
- **Integration tests:** load all shipped glTF assets through the validator and into a headless Bevy app.

**Status (2026-11-12):** bike and character models, animations and the asset pipeline are done (§2.5, §2.6, §8.1): `crates/ocw-assets` generates six bikes and the rider as glTF, validates them (`ocw-assets validate`), poses the rider on the bike and selects the riding state; `ocw-render::riders` draws and animates them with a far LOD. Tests: mesh building (outward faces), glTF write / read round trip and the +Z file convention, the validator rules (no licence entry, unknown licence, missing file, physics keys, missing nodes, unknown paint slots), committed assets = generator output, frames from their geometry (wheelbase, saddle height), feet on the pedals and hands on the bars for every bike seated and standing all round the crank, torso lean by bike, state selection with hysteresis and dwell, cadence, crank easing, wheel spin, pose blending; integration (`crates/ocw-render/tests/shipped_assets.rs`): all shipped assets through the validator and through Bevy's own glTF loader in a headless app; the riders plugin spawns every bike with the right head wear and kit and animates standing, wheel spin and freewheeling headless. Still open in Phase 4: components (wheels, tyres, saddle …) as cosmetic choices, the 3D preview on the Workshop page, the Character page (skin, hair, body), *Equal* race mode, the remaining animations (cornering, U-turn, finish, victory, foot down).

#### 4A – Workshop scene (planning, 2026-11-13, v2.53)

The user asked (2026-11-13): selecting *Workshop* changes the screen to a **rendering of a bicycle workshop** showing all the rider's bikes; the selected bike stands on a **repair stand in the foreground**, the other owned bikes **hang on the wall** and are chosen with **left / right**; the bike on the stand can be selected **at its parts**, which are tuned or exchanged; parts are bought with **hearts** (4E); everything works with **gamepad, mouse and touch** (and keyboard, §3.2.2). Decided by the user: the scene, the stand, the wall, left / right, part selection on the bike, hearts, 1 heart = 1 EUR, prices rounded up to whole hearts. The rest of 4A–4L is a proposal.

- **A 3D scene, not a menu page.** *Workshop* in the top menu (§3.2.1) opens the workshop scene full-screen instead of the Workshop sub page. The ride goes on under it exactly as under the menu (decided v2.2: no pause; autodrive and default exits while it is open, trainer unchanged); the status strip of the menu (power, speed, trainer) stays at the top, and a small live **ride window** (picture-in-picture, 20 % of the width, off with `workshop.ride_window off`) shows the ride. Back / Esc / B leaves the workshop to the menu's top page (Esc twice: back to the ride). Later the start screen (Phase 5 dashboard, §2.1) opens the same scene without a ride.
- **Rendering (`ocw-render::workshop`):** its own small scene on its own `RenderLayers` layer with its own camera, spawned when the workshop opens and despawned on leaving (the world's camera stops rendering meanwhile: the GPU budget goes to the workshop; the ride window renders the world at a quarter resolution and 10 fps). The room is generated in code like the bikes (§2.5 asset decisions; `ocw-assets` generator, own work, MIT OR Apache-2.0, no brands, Q11): concrete floor, a back wall with **bike hooks**, a pegboard with tools, a workbench, a parts shelf, a window, warm interior light (one shadow-casting spot over the stand, ambient fill, no sky); the material library (Phase 8 T1, CC0) for the floor, wood and wall. Triangle budget: room 40 k, plus the bikes at their near LOD.
- **Layout:** the **repair stand** in the foreground centre (clamp on the seat post, the bike at working height, wheels free so they can spin), the **wall** behind it with one hook per owned bike (wall-mounted by the front wheel at an angle, all at the same height, a name card under each), an **empty hook with a "+"** at the end of the row (buy a bike, 4D), the **changing corner** on the right (mirror, bench, locker: the character editor, 4C) and the **workbench** on the left with the parts shelf (the parts catalogue, 4D). The wall scrolls with the camera when there are more bikes than fit (up to 24 hooks).
- **Camera states** (a pure state machine in `apps/desktop/src/workshop/camera.rs`, eased moves of 0.6 s, no cuts):
  - *Overview* (on entering): stand and wall in view, focus on the bike on the stand.
  - *Wall*: the camera pans along the wall to the focused bike (left / right step one hook; the focused bike gets a rim light and its card grows: name, bike type and profile, weight, value in hearts).
  - *Stand*: close to the bike on the stand, orbit around it (all devices, §3.2.2), zoom 0.8–4 m, the part hotspots shown.
  - *Part*: flies to the selected part (framed at a third of the screen, the part picker on the right).
  - *Mirror*: the changing corner with the rider in front of the mirror (4C).
- **Choosing a bike:** in Overview and Wall, **left / right** cycle through the owned bikes (on the stand first, then the hooks left to right; wrap-around); **Confirm** on a wall bike plays the swap (1.2 s: the stand's bike is lifted back onto its hook, the chosen one taken down and clamped on the stand) and makes it the active rider's bike (`riders.edit NAME --bike-id ID`); Confirm on the stand bike goes to *Stand*. Up / Down move between the zones *wall*, *stand*, *mirror*, *bench* (spatial: up = wall, right = mirror, left = bench, down = stand), so a gamepad never needs a pointer. Mouse / touch: click / tap a bike on the wall (the same swap), a click on the stand bike goes to *Stand*; swipe left / right anywhere on the wall cycles.
- **When a change applies:** today a bike or look applies from the next start (v2.7, decided: the server has no message to change a rider's profile mid-session). Proposal: a new reliable **`BikeChange`** client message (bike id, frame and part ids, look; additive, §6.2) sent when the workshop closes; the server validates it (catalogue ids, ownership, 4E, event lock) and applies it at once in free ride and outside events, from the next start of an event otherwise (riders in a running event keep their bike: no pit stops in v1). This replaces the "next start" rule when accepted (Q1).

#### 4B – Parts on the bike: hotspots, picker, compare, buy / equip

- **Part slots** (the components of §2.5 and §8.2, one hotspot each, its anchor a node in every bike model – the generator adds `hs_<slot>` empty nodes, the validator requires them): `frame` (frameset: frame and fork, decides the physics profile, 4G), `wheels` (pair), `tyres` (pair), `groupset` (shifters, derailleurs, brakes; cranks, cassette and chain as their own slots below, so they can be upgraded alone), `crankset` (cranks and chainrings; optional power meter as a variant), `cassette`, `chain`, `cockpit` (handlebar and stem, or the integrated bar; TT: base bar and extensions), `bar_tape` (road / gravel; grips on the MTB), `saddle`, `seatpost`, `pedals`, `bottle_cages` (0–2), `lights` (front and rear set), `computer` (head unit and its out-front mount), `bag` (top-tube or saddle bag), `paint` (the paint slots of the frame, the kit stays on the character, 4C). The slots a bike type has come from its frame (no bar tape on an MTB, no extensions on a gravel bike); events may forbid bags and lights.
- **Groupset and its parts:** a groupset bought as a whole also equips the crankset, cassette and chain of its tier (its price covers them, as the real groupset prices in 4H do); those three slots can then be changed alone (e.g. a crankset with a power meter).
- **Hotspots:** small glowing markers (a dot with a slot icon, 4 mm at 1 m, drawn above the model with depth test off and an occlusion fade) at the `hs_*` anchors of the bike on the stand, visible in *Stand*. Selecting one: gamepad / keyboard by **directional navigation in screen space** (the next hotspot in the pressed direction, by the projected positions, recomputed when the camera moves; LB / RB or Q / E step through the slots in list order), mouse hover highlights and click selects (Bevy picking on the hotspot's sphere collider, `bevy_picking` mesh backend), touch tap (the hit circle is at least the touch minimum of §3.2.2, 9 mm, even when the marker is drawn smaller). The hotspot's name is shown next to it on focus ("Rear wheel – entry alloy wheelset"), so a hotspot is never icon-only.
- **Part picker** (Bevy UI panel on the right, 40 % of the width, ten-foot sizes, §3.2.1): the slot's options in groups *equipped*, *owned* (spare parts in the inventory), *shop* (by tier, cheapest first); each row: name, tier badge, weight, price in hearts (or "owned" / "equipped"), a lock with the missing amount when the balance is too low; compatibility filters the list (4F: only parts that fit the frame's bike type and standards). The **detail card** of the focused row: the main figures (weight, and per kind: tyre width, rim depth, gear range, rolling-resistance class, aero class), the effect on the bike's physics in plain words and numbers against the equipped part ("−240 g · rolling resistance −8 % · ≈ +0.4 % speed at 250 W on the flat · ≈ −6 s on 5 km at 6 %"; computed by the server's physics code, 4G), and in races: "has no effect in Equal / Profile races".
- **Live preview:** focusing a row puts that part on the bike on the stand at once (preview only, not bought; the camera keeps the part framed); leaving the picker restores the equipped part. Paint previews the colour on the frame.
- **Compare:** North (Y / △) / C / long press pins the focused part as *A*; moving to another row shows both detail cards side by side with the differences coloured (better / worse, with a word, never colour alone); North again unpins.
- **Actions** (West = X / □, or the buttons on the card; the confirmations are dialogs with *Cancel* focused, as for Restart):
  - **Buy** (shop row): "Buy *Performance road tyres 28 mm* for 164 ♥? Balance 1,420 ♥ → 1,256 ♥" → the part is added to the inventory and equipped.
  - **Equip** (owned row): moves the part from the inventory or from another of the rider's bikes ("now on the stand bike; the gravel bike gets its base part back").
  - **Unequip:** back to the slot's **base part** (every frame comes with a free base part per slot, the entry tier, so a bike is always complete and rideable; the unequipped part goes to the inventory).
  - **Sell:** for a share of the price paid (4E).
  - **Repair** (with persistent damage only, Phase 12 Q6): shows the part's condition and the repair price.
- **Paint and bar tape colours** stay free (cosmetic, today's palette and `#RRGGBB`; v2.7 look fields), special paint patterns (fades, metallic, team-style designs without brands) are paint "parts" with a price from real respray / paint-option prices (4H).

#### 4C – Character editor: the changing corner

- The **changing corner** to the right of the stand (mirror, bench, locker with kit on hangers) is the character editor of §2.6 in the same scene: the camera turns to it (*Mirror*), the rider stands in front of the mirror, seen from behind over the shoulder and in the mirror from the front (a planar reflection with a second camera at half resolution; on low graphics quality a direct front view instead).
- **Slots:** body (height from the rider profile, build: slim / medium / strong), skin tone (a 12-step palette), face (a few presets), hair (style and colour), jersey, bib shorts / tights, socks, shoes, gloves, helmet, glasses, cap; the kit patterns of v2.7 (Phase 8 T6: the jersey designer – two more slots, *Jersey designer* and *Kit colours*, §14 Phase 8 T6). Selection as for bike parts: the rider's hotspots (head, torso, legs, feet, hands) or LB / RB through the slot list, the picker panel on the right, live preview, compare.
- **Kit costs hearts too** (decided 2026-11-13, Q4): helmet, shoes, gloves and clothing are priced from real prices like bike parts (4H, category *apparel*); **glasses are free** like body, skin, face and hair (who someone is never costs anything). Every rider starts with a full free base kit.
- **Kit gives a slight benefit (decided by the user 2026-11-14, replacing "kit is purely cosmetic"):** accessories change the physics a little, like bike parts – e.g. a **time-trial helmet makes the rider a little faster than a standard helmet or none**. Hence a **scoring system** of how much each item gains over the standard item (and over the others):
  - *Effect classes* as for bike parts (4G): each apparel item has curated steps for **aero** (CdA: TT / aero helmet, skinsuit, aero jersey and bib shorts, overshoes, aero socks), **weight** (real grams: light shoes, light helmet) and a few for **efficiency** (stiff race shoes – a tiny drivetrain-efficiency step); comfort items (gloves, caps, warm kit) have none. Body, skin, face, hair and glasses never change the physics.
  - *The benefit score:* every item gets a number = its speed gain over the **standard (base) item of its slot** at the reference rider (250 W, 75 kg) on the flat and on a 6 % climb, in % (e.g. "TT helmet: +0.6 % flat, +0.1 % at 6 %"); shown on the picker cards, and the compare card shows the difference between two items. The scores come from the same physics model as the server, so they are exact, not marketing numbers.
  - *Caps and modes (proposal):* the kit's effect adds to the bike build and **counts against the same build caps** (4G: best bike + kit ≤ +1.5 % flat, ≤ +2.5 % at 6 %), so buying kit can never break the fairness limit; in *Profile* and *Equal* race modes kit is cosmetic like parts, in *Open* it counts. The server computes it (`ocw-sim::build`), the client only sends item ids.
  - *Calibration:* the class steps are set so that the best kit alone is worth roughly a third of the cap (e.g. TT helmet + skinsuit ≈ +0.5 % flat), a single typical upgrade 0.1–0.3 %; tuned by measured physics tests, recorded per item in `assets/parts.toml`.
- **Poses:** the rider stands, turns with the orbit, and Confirm on *Try on the bike* puts the rider on the bike on the stand pedalling slowly (the riding animation of §2.6), to see bike and kit together.

#### 4D – The bench: buying bikes and spare parts

- **Buying a bike:** the "+" hook or the bench opens the catalogue of **framesets** (frame + fork, 4F) by bike type and tier; a bought frameset comes with the base parts (entry tier) and hangs on a new hook; it is then built up on the stand. *Complete builds* ("buy the frameset with the mid-tier groupset and wheels") are a convenience row that buys the frameset and the parts in one confirmation, at the sum of their prices (no discount, so the prices stay comparable).
- **Spare parts:** the shelf lists the inventory (parts not on a bike): equip on the stand bike, sell, compare.
- **Selling a whole bike** gives the frameset's and its non-base parts' share back and frees the hook; the last bike of the rider cannot be sold, and the six starter bikes (4E) cannot be sold.

#### 4E – Hearts: economy (unit decided by the user; the rest a proposal)

- **Unit (decided, 2026-11-13):** **1 heart (♥) = 1 EUR** of a part's real-world price; prices are **rounded up to whole hearts** (`hearts = ceil(price_eur)`, no cents anywhere). Shown as "1,239 ♥" with the heart glyph and the word "hearts" in screen-reader / tooltip text.
- **No real money, ever (flag):** the user decided "no commercial release" (Phase 8 Q2, v2.44). Hearts are therefore **earned only by riding**; they cannot be bought, sold for money, transferred between accounts or cashed out, and there is no shop for them outside the game. "1 heart = 1 EUR" is a **price scale**, not an exchange rate. (A paid currency would bring consumer-protection, tax and loot-box rules and contradict the free / open project; it would need a new user decision.)
- **Earning (proposal; measured from the server's accepted power, §6.1):**
  - **Work:** **1 ♥ per 10 kJ** of mechanical work (≈ 72 ♥ for an hour at 200 W, ≈ 100 ♥ at 280 W). Work, not distance: it does not reward drafting, descending, a better bike or a faster profile (the rich would get richer), and it counts the same on a route, in free ride, in workouts and in ERG.
  - **Climbing:** +1 ♥ per 20 m of elevation gained (a hilly hour ≈ +30 ♥), so the world's mountains pay a little extra.
  - **Workouts:** +25 % of the workout's work hearts when it is completed (all steps, ≥ 90 % compliance).
  - **Events:** finishing a race / group ride / time trial +20 ♥ (local events: +10 ♥), podium +30 / +20 / +10 ♥; never more for winning big (no snowballing).
  - **Achievements** (Phase 7): one-off amounts, e.g. first 100 km 100 ♥, first 1,000 m climb in one ride 50 ♥, a world's every package route ridden 200 ♥, 7-day streak 70 ♥.
  - **Daily first ride:** +10 ♥ for the first ride of the day of at least 20 min.
  - **Caps:** **no daily cap** (decided 2026-11-13, Q3: the proposed 400 ♥ per day was dropped); the per-ride plausibility checks stay.
  - **Never earned:** with **virtual power** (keyboard ↑, `sim.power`, the virtual trainer, §2.9 `dev`), in **god mode** (Phase 13: no distance, no hearts), by bots, in the kiosk / show mode, in captures (`--capture`), while the rider is *down* after a crash (Phase 12), and from rides flagged as implausible by §6.4 (held until reviewed).
- **What things cost then (from 4H):** an hour at 200 W ≈ 70–100 ♥ (with climbing); a performance tyre pair ≈ 2 h, a mid-tier mechanical groupset ≈ 15–20 h, a top carbon wheelset ≈ 45 h, a top-tier frameset ≈ 60 h; a full top-tier race bike (≈ 15,000 ♥) ≈ 175 h – about a year at three to four hours a week. This follows the real market on purpose (decided scale), and the physics gain of a top bike is small and capped (4G), so it is a long-term goal, not a requirement.
- **Starting state:** every rider starts with the **six starter bikes** (one per physics profile, today's models, entry-tier base parts, §2.5) on the wall, a full base kit, and **500 ♥** to try the shop at once. Riders who rode before hearts existed get hearts for their saved rides once (from the FIT files' work, at most 2,000 ♥, Q3).
- **Selling:** an unused part (bought, never ridden – no ride of ≥ 1 km with it equipped) is refunded **in full**; afterwards **50 %** of the price paid, rounded down (`floor`), so selling never makes hearts. Base parts and starter bikes cannot be sold (they are free).
- **Damage and repair (Phase 12):** with damage reset per ride (Phase 12 Q6 proposal), repairs are free and automatic – no hearts. If the user decides for persistent damage (Q6 career mode), a part's condition carries over and its repair costs `ceil(price_paid × (1 − condition) × 0.3)` ♥ (a part at 60 % costs 12 % of its price; frames and wheels may be "beyond repair" below 10 % and need replacing); a broken part is replaced by its base part until repaired, so the bike stays rideable. Damage never destroys hearts in non-persistent mode.
- **Storage (server-authoritative):**
  - *With the backend (Phase 5):* the **account** owns a hearts **ledger** (append-only table `hearts_ledger`: account, delta, reason – ride, workout, event, achievement, purchase, sale, repair, grant –, reference id, time; the balance is the sum, cached) and an **inventory** (`items`: item id, catalogue part id, price paid, bought at, condition, ridden flag; `bikes`: bike id, frame item, slot → item, look). Purchases are API calls (Axum, §9: `POST /workshop/purchase` with an idempotency key, a transaction that checks the balance, the catalogue price and the compatibility). Workshop actions are **not** on the QUIC game connection. Hearts for a ride are credited by the **game server** that hosted it (it saw the power stream), or for a ride on the local server from the uploaded ride summary after the §6.4 plausibility checks (power-duration curve, cadence, device trust level).
  - *Before the backend (now):* a local wallet in the user data directory (`<data dir>/ocw/workshop.toml`: inventory and bikes per rider profile; `hearts-ledger.jsonl` append-only), written by the local server's ride results. It is **trivially editable** – accepted, since it only matters on the local server; on the first sign-in the local balance is imported once, capped (Q3).
  - The client never sends physics or prices: it sends bike id and part ids; the server takes the physics from its own catalogue copy (4G) and – with accounts – checks ownership. An unknown or not-owned part counts as the base part.
- **Console** (user level unless noted; the menu runs the same commands, §3.2.1): `hearts` (balance and the last ledger entries), `workshop.parts [SLOT]`, `workshop.buy PART [--bike ID]`, `workshop.equip ITEM [--bike ID]`, `workshop.unequip SLOT`, `workshop.sell ITEM`, `bikes.list`, `bikes.select ID`, `bikes.buy FRAME`; `hearts.grant N` (`dev`, local server only, for tests and screenshots).

#### 4F – Compatibility and bike types

- Each frameset has a **bike type** (road, TT, gravel, MTB) and a **profile family** (road: standard, aero or climbing; TT; gravel; MTB) – the physics profile of §2.5. Parts list the bike types they fit (`fits`) and, where it matters, a standard: drivetrain speeds (11 / 12 / 13), `mech` / `electronic`, road / MTB / gravel drivetrain family, brake type (rim / disc), wheel size (700c / 29"), tyre width range per frame (clearance), crank spindle family. The generated bikes are all disc-brake 12-speed except where §2.5 says otherwise (the `road` profile has rim brakes: it stays the only rim-brake frame). A part that does not fit is not offered for that bike; a groupset change replaces cassette and chain by matching base parts when they no longer fit.
- Kept simple on purpose (no bottom-bracket or axle standards): the aim is believable choices, not a fitting simulator.

#### 4G – Physics: parts within the curated profiles (fairness)

**Tension (flag):** the user wants real parts with real prices that can be tuned and exchanged; real parts differ in weight, aerodynamics and rolling resistance. §2.5 decided that physics comes from **a small curated set of profiles**, cosmetics never change physics, and the difference between profiles is capped (±3 % speed at 250 W on the flat). Parts that make the bike faster turn hearts into speed: in multiplayer this would be "ride to win" (better-equipped riders win), and real-world part prices are not balanced for a game.

**Proposal – bounded part effects on top of the frame's profile:**

1. The **frameset decides the profile** (as the bike type does today): six profiles stay the only physics families, and the curated profile table stays the base on the server.
2. Parts have **real weights** (grams) and, where they matter physically, a **class** on a few curated axes: `rolling` (tyres: classes −2 … +2, each ±4 % Crr), `aero` (wheels, cockpit, frame within its family: −2 … +2, each ±0.004 m² CdA), `efficiency` (groupset / chain: −1 … +1, each ±0.3 %). Classes, not free numbers: the server's catalogue copy maps them to values, the client never sends values (as for profiles, v2.7).
3. The server computes the build: bike mass = the frameset's base mass + Σ (part weight − base part weight), Crr / CdA / efficiency = profile value adjusted by the class steps; then **clamps the whole build** to the frame's profile band: at 250 W, 75 kg on flat asphalt the best possible build of a profile is at most **+1.5 %** faster and the base build is the profile itself (so upgrades are a small, visible but never decisive gain); on a 6 % climb at most +2.5 %; a mass floor of 6.8 kg (road, TT, gravel; the UCI limit, also realistic), 9.0 kg for MTB. The caps are unit-tested like the profile caps (§2.5).
4. **Race modes** (extends §2.5): *Open* – profile and parts count; ***Profile*** (new, proposed default for races and group rides) – the frame's profile counts, parts are cosmetic (everyone on a "road – aero" frame is equal, whatever their wheels cost); *Equal* – one profile for all. The organiser (or the host) chooses, shown in the event's info. Free ride, workouts and single player use *Open*. Time trials against the own ghost: *Open*.
5. **Hearts are earned only from work, not speed** (4E), so better parts never earn faster.

Options considered: parts purely cosmetic (fairest, but then weights and prices mean nothing, which loses the point of the user's request); parts changing physics freely (real but pay/ride-to-win, and unbalanced by real prices); **bounded effects + Profile mode for races (proposed)** (Q2).

**Implemented (W3, v3.03; `ocw_sim::build`, pure and wasm-safe, used by the server, the client's prediction and the workshop's cards):**

- *Catalogue copy:* `PartTable` reads the physics keys of the shipped `assets/parts.toml` (`include_str!`, the same file the workshop reads; names and prices are skipped); classes are clamped to −2 … +2 (rolling, aero) and −1 … +1 (efficiency). Of the ids sent, a part counts if it is known, not a base part, fits the bike type (a frameset only its own profile) and is the first of its slot; anything else counts as the base part. Ownership is checked with accounts (Phase 5).
- *Sum:* bike mass = profile mass + Σ (part − the slot's base part), floored at 6.8 kg (9.0 kg for MTBs; never above the profile's own mass); Crr × (1 + 0.04 · Σ rolling); CdA − 0.004 m² · Σ aero; efficiency + 0.003 · Σ efficiency. Signs as in the catalogue: race tyres are rolling −2, aero wheels +1, the TT disc +2, a top-tube bag aero −1.
- *Caps:* the speed of the build against the profile's base build at 250 W, 75 kg (steady state, flat asphalt and 6 %) must stay within ±1.5 % / ±2.5 % (a band, so a heavy, draggy build is capped too). A build outside keeps the largest share t of its change (every value moved back towards the profile by the same share, bisection) that fits, so the best build sits exactly on the cap. Best builds (all fastest parts of each slot): `road` +1.39 % flat / +2.05 % at 6 % (6.80 kg, on the mass floor), `road-aero` **+1.50 %** / +2.16 % (capped, t = 0.79), `road-climbing` +1.29 % / +0.83 % (already at 6.8 kg), `tt` +1.22 % / **+2.50 %** (capped, t = 0.80), `gravel` +0.45 % / +2.10 %, `mtb` +0.45 % / +1.68 % (9.83 kg). Off the asphalt the profile's off-road share stays as it is.
- *Modes:* `PhysicsMode` *Open* (the build), *Profile* (the frame's curated profile with the rider's weight), *Equal* (`road` with the rider's weight); the session refreshes every rider's physics every tick from the mode in force: the event's for its entrants until it is over, else *Open*. Defaults (`PhysicsMode::for_event`): races and group rides *Profile*, time trials *Open*.
- *Tests:* class steps per class, weights, the floors, the stacked best build exactly on the cap (and a slow build on the lower edge), the three modes (Profile ignores parts, Equal equalises even different frames, Open applies within the caps: a stock and an upgraded bot at a steady 300 W out of each other's draft – the upgraded one ≈ 1.4 % faster in *Open*, the same speed and finish time to the hundredth in *Profile*), the BikeChange timing (at once / after the event, reported once), the protocol round trips (mode, build, hash in 6 bytes, Hello parts, BotBike), the prediction riding the same build and mode as the server, the others' parts in the view and on the avatars (part ids → model variants).

- **Aero effects depend on speed (decided by the user 2026-11-14: aero bikes and aero components pay off at higher speeds, more the faster).** Aero frames, deep wheels, aero cockpits, TT / aero helmets, skinsuits, overshoes change **only the drag area CdA**; the physics (§5.2) then gives them their real, speed-dependent effect – no artificial switch-on speed. Investigated with our model (75 kg rider + 10 kg bike, Crr 0.004, CdA 0.32 m², ρ 1.2 kg/m³), for a CdA saving of 0.010 m² (about a deep wheelset, or a TT helmet + skinsuit):
  - *Watts saved at the same speed* grow with the **cube** of speed: 0.4 W at 15 km/h, 1.1 W at 20, 2.1 W at 25, 3.6 W at 30, 5.6 W at 35, 8.4 W at 40, 12 W at 45, 16.5 W at 50 km/h (a 0.020 m² frame + wheels + kit build: double).
  - *Share of air drag in the resistance on the flat:* 31 % at 10 km/h, 50 % at 15, 64 % at 20, 74 % at 25, 80 % at 30, 88 % at 40, 92 % at 50 km/h.
  - **Thresholds:** below about **20 km/h** aero parts are practically worthless (≈ 1 W); from about **25–30 km/h** they become noticeable (2–4 W); from **35–40 km/h** they matter (6–8 W and more) – and the faster, the more (descents, sprints, fast groups). On climbs the speed is low: at 6 % a rider at 250 W gains only +0.17 % speed (+0.04 % at 100 W), at 10 % almost nothing – there weight wins.
  - *Speed gain at the same power on the flat* stays about **+1 %** for every rider (100 W: +0.96 %, 400 W: +1.02 %), because the air dominates there at any power; in **watts and seconds saved per hour** the fast rider gains more, and over a fixed 40 km the slow rider saves slightly more time (46 s at 150 W, 37 s at 300 W) because they are out there longer.
  - *Drafting:* in another rider's slipstream the air drag is reduced (§5.2), and so is the aero part's benefit – aero kit matters most alone in front, in breakaways and time trials.
- **Consequences for the scores and caps:**
  - The **benefit score** of an aero item is shown **by speed**, not as one number: the watts saved at 25 / 35 / 45 km/h on the flat and the speed gain at the reference rider on the flat and at 6 % (e.g. "Deep wheels: 2 W @25, 6 W @35, 12 W @45 km/h; +0.4 % flat, +0.1 % at 6 %"), with a hint "pays off above ~30 km/h". Weight items show their gain on climbs instead ("−300 g: +0.4 % at 6 %, +0.05 % flat").
  - The **build caps** (best bike + kit ≤ +1.5 % flat, ≤ +2.5 % at 6 %) are checked at **three reference powers** (150, 250 and 400 W, 75 kg) and must hold at each (proposal) – aero builds are bounded by the flat cap at every speed, weight builds by the climb cap. Since the server computes every rider's effect with the real physics at their real speed, nothing else needs scaling.
  - Tests: a CdA step gives the watts above at those speeds (within 2 %), zero effect at standstill, monotonic growth with speed, the drafting reduction, and the caps at all three reference powers.

#### 4H – Part price catalogue: research (2026-11-13)

**Brands (decided, Phase 8 Q11):** no real brands or logos in the game. In-game parts are **generic and unbranded** ("Race 12-speed electronic groupset, top tier"); each part's price is the **median of real reference products** of the same kind and tier (manufacturer RRP / UVP in EUR including VAT; a street price only where no RRP was found, marked), rounded **up** to whole hearts. The brand names below are research data in the spec and in the sources file only; they never appear in the game, the assets or the UI.

Research of 2026-11-13 (web search by agents; most brands publish no EUR prices on their own sites, so most figures are the manufacturer's RRP / UVP **as shown by EU retailers**). Type: **R** = manufacturer list or launch RRP, or a UVP shown by a retailer; **R\*** = list price reported by a third party (magazine, price tracker, retailer "regular price"), not verified on the brand's page; **S** = street price (only where no RRP was found). Sources: *SRAM-25* = SRAM MSRP price list 2025 (EUR incl. VAT, slowtwitch.com, component sums added by us); *BR* = BikeRadar launch / review articles; *bc* = bike-components.de product pages (UVP); *Rose* = rosebikes.de / .com product pages (UVP); *Mantel* = mantel.com RRP; *99s* = 99spokes. The full list with URLs goes into `assets/parts-sources.toml`. Median of all references of a tier (two references: their mean); **♥ = ceil(median)**; pairs (tyres) = ceil(2 × median per tyre).

| Category / tier | References (EUR, type, source) | Median € | ♥ |
|---|---|---|---|
| **Road groupset** entry, 11–12 s | Shimano Tiagra R4000 (sum of parts, 2026) 633 R (BR); SRAM Apex XPLR AXS 1x12 1,269 R (Mantel) | 951 | **951** |
| mid, mechanical | Shimano 105 R7100 1,183 R (BR, parts); Campagnolo Chorus 12 1,699 R (Mantel; launch 1,819) | 1,441 | **1,441** |
| mid, electronic | Shimano 105 Di2 R7170 1,869 R (BR, Mantel); SRAM Rival AXS E1 1,765 R (SRAM-25) | 1,817 | **1,817** |
| upper, electronic | Shimano Ultegra Di2 R8170 2,549 R (Mantel); SRAM Force AXS E1 2,423 R (SRAM-25); Campagnolo Record 13 2,699 R (2026) | 2,549 | **2,549** |
| top, electronic | Shimano Dura-Ace Di2 R9270 3,999 R (Mantel); SRAM Red AXS E1 4,135 R (BR 2024); Campagnolo Super Record 13 4,300 R (2025) | 4,135 | **4,135** |
| **Gravel groupset** mid | SRAM Apex XPLR AXS 1,269 R (Mantel); SRAM Rival XPLR AXS E1 1,560 R (SRAM-25) (Shimano GRX RX820 only S 819–849, not counted) | 1,415 | **1,415** |
| top | Shimano GRX Di2 RX825 2,479 R (Mantel); SRAM Force XPLR AXS E1 2,135 R (SRAM-25); Campagnolo Super Record X 3,375 R | 2,479 | **2,479** |
| **MTB groupset** (with brakes) entry | Shimano Deore Di2 M6250 650 R (Mantel); SRAM Eagle 90 T-Type 730 R (bc) | 690 | **690** |
| mid | Shimano XT M8100 980 R, XT Di2 M8250 1,240 R (Mantel); SRAM GX Eagle Transmission 1,300 R (BR) | 1,240 | **1,240** |
| top | Shimano XTR Di2 M9250 2,005 R (Mantel); SRAM XX Transmission 2,450 R; SRAM XX SL Transmission 2,650 R | 2,450 | **2,450** |
| **Crankset** entry / mid / upper / top | Tiagra 135 R; 105 FC-R7100 175 R (Rose), Rival E1 220 R; Ultegra FC-R8100 295 R (Rose), Force E1 455 R; Dura-Ace FC-R9200 580 R (Rose), Red E1 785 R (BR) | 135 / 197.5 / 375 / 682.5 | **135 / 198 / 375 / 683** |
| crankset with power meter (dual) | Shimano Ultegra FC-R8100-P 1,099 R, 4iiii Precision 3+ Pro 899 R (Rose); SRAM Force E1 spider 855 R (SRAM-25) | 899 | **899** |
| **Power-meter pedals** dual | Favero Assioma Pro RS-2 699 R (BR); Garmin Rally 210 1,100 R (2025); Favero Assioma Duo 388 R (VAT basis unclear) | 699 | **699** |
| **Cassette** 12 s entry / mid / upper / top | Tiagra 11 s 60 R; 105 CS-R7100 80 R, Rival XG-1250 132 R; Ultegra 115 R (unverified), Force XG-1270 215 R; Dura-Ace CS-R9200 340 R (Rose) | 60 / 106 / 165 / 340 | **60 / 106 / 165 / 340** |
| **Chain** mid / top | 105 CN-M7100 37 R (Rose), Rival E1 50 R; Dura-Ace CN-M9100 73 R (Rose), Red Flattop 100 R | 43.5 / 86.5 | **44 / 87** |
| **Wheelset** (pair) alloy entry | Fulcrum Racing 5 DB 400 R; Mavic Aksium Elite UST disc 299 R, Mavic Aksium 299 R (Rose) | 299 | **299** |
| alloy mid | DT Swiss PR 1600 DB 600 R (Rose); Fulcrum Racing 3 DB 620 R\*; Mavic Ksyrium 30 Disc 437 R (Rose) | 600 | **600** |
| carbon mid | DT Swiss ERC 1400 DB 45 2,050 R (Rose); Fulcrum Wind 42 DB 1,415 R (Mantel); Zipp 303 S 1,100 R (BR) | 1,415 | **1,415** |
| carbon top, aero | DT Swiss ARC 1100 DB 50 2,500 R (Rose); Mavic Cosmic Ultimate 45 3,900 R (BR 2023); Zipp 454 NSW 4,400 R\* | 3,900 | **3,900** |
| TT disc rear (single wheel) | Zipp Super-9 Disc 2,750 R (launch) – one reference | 2,750 | **2,750** |
| gravel alloy / gravel carbon / MTB 29" alloy | DT Swiss GR 1600 600 R; DT Swiss GRC 1400 2,050 R; DT Swiss XM 1700 750 R (Rose) – one reference each | 600 / 2,050 / 750 | **600 / 2,050 / 750** |
| **Road tyre** (per tyre) training | Continental Ultra Sport III 30.95 R; Schwalbe Lugano II 29.90 R (bc) | 30.43 | **61** pair |
| performance | Continental GP 5000 S TR 94.95 R\*; Schwalbe Pro One Evo 78.90 R; Pirelli P Zero Race TLR 84.95 R; Vittoria Corsa N.EXT 69.95 R (bc) | 81.93 | **164** pair |
| race / TT | Continental GP 5000 TT TR 108.95 R\*; Vittoria Corsa Pro Speed 94.95 R; Schwalbe Pro One TT 78.90 R (bc) | 94.95 | **190** pair |
| **Gravel tyre** | Schwalbe G-One RS Pro 78.90 R; Continental Terra Speed 69.95 R; Pirelli Cinturato Gravel H 70.00 R | 70.00 | **140** pair |
| **MTB tyre** 29" | Maxxis Rekon Race 59.90 R; Schwalbe Racing Ray 72.90 R; Continental Race King 43.95 R | 59.90 | **120** pair |
| **Saddle** entry | Selle Italia X-Bow 60 R; Specialized Bridge Sport 40 S; Fizik Vento Argo X5 109 R | 60 | **60** |
| mid | Fizik Vento Antares R5 109 R; Specialized Power EVO Comp 119 R; Selle Italia SLR Advan 3 150 R; Prologo Scratch M5 Tirox 129 R | 124 | **124** |
| top, carbon | Fizik Antares R1 Adaptive 299 R; Prologo Scratch M5 CPC Nack 269 R; Selle Italia SLR Carbon 3 320 R; Specialized S-Works Power EVO Mirror 449 R\* | 309.5 | **310** |
| **Cockpit** alloy bar + alloy stem | bars: Zipp Service Course 70 68 R, Deda Zero2 55 R, Ritchey Comp ErgoMax 56 R; stems: Ritchey Comp 4-Axis 54.40 R, Deda Zero2 54.95 R, PRO Vibe 110 R | 55.99 + 54.95 | **111** |
| carbon bar + alloy stem | Zipp SL-70 Ergo 339 R; Deda Superzero RS 360 R; PRO Vibe Carbon 350 R (+ stem 54.95) | 349.95 + 54.95 | **405** |
| integrated carbon | Black Inc bar/stem 679 R; PRO Vibe Evo 800 R; Deda Alanera RS DCR 539 S | 679 | **679** |
| gravel flared bar + stem / MTB bar + stem | Zipp SC 70 XPLR 68 R, Ritchey VentureMax 59 R, PRO Discover 70 R / Renthal Fatbar Lite 95 R, Race Face Chester 60 R, Ritchey Comp Flat 40 R (+ stem 54.95) | 68 + 54.95 / 59.99 + 54.95 | **123 / 115** |
| TT extensions | Profile Design Airstryke 2 198 R; Zipp Vuka Clip 173 R; Deda Superzero TT 220 R | 198 | **198** |
| **Seatpost** alloy / carbon | Deda Zero2 65 R, Ritchey WCS 1-Bolt 104 R / Zipp SC SL 194 R, Roval Alpinist 240 R, PRO Vibe Superlight 300 R | 84.48 / 240 | **85 / 240** |
| **Pedals** road entry / mid / top | Shimano 105 R7000 155 R, Look Kéo Classic 3 52 R, Time Xpresso 105 R / Ultegra R8000 165 R, Look Kéo 2 Max Carbon 112 R, Speedplay Comp 150 R / Dura-Ace R9100 255 R, Look Keo Blade Carbon 149 R, Speedplay Zero 230 R\* | 105 / 150 / 230 | **105 / 150 / 230** |
| pedals SPD entry / mid | Shimano PD-M520 57 R, Crankbrothers Candy 1 65 R / Shimano XT M8100 135 R, Candy 3 150 R, Look X-Track Race Carbon Ti 210 R | 61 / 150 | **61 / 150** |
| **Frameset** road alloy | Cannondale CAAD14 1,799 R\* (BR 2026); Specialized Allez Sprint 1,390 S; Blackheart Road AL 1,290 S (bc) | 1,390 | **1,390** |
| road carbon mid | Giant TCR Advanced Pro 2,499 R\* (99s); Factor O2 Disc 2,839 R (bc); Cervélo Soloist 3,499 R\* | 2,839 | **2,839** |
| road carbon top | Specialized S-Works Tarmac SL8 5,499 R\*; Cervélo R5 5,699 R\* (99s); BMC Teammachine SLR 01 4,999 R (BMC store); Trek Madone SLR Gen 8 5,059 R\* | 5,279 | **5,279** |
| TT | Cervélo P5 5,999 R\*; BMC Speedmachine 01 Module 6,499 R (BMC); Giant Trinity Advanced Pro 3,499 R\* (2022) | 5,999 | **5,999** |
| gravel carbon | Scott Scale Gravel RC 2,299 R\*; Cervélo Aspero 2,799 R\*; OPEN U.P. 2.0 4,500 S | 2,799 | **2,799** |
| MTB carbon hardtail | Mondraker Podium 2,699 R\*; Trek Procaliber C Gen 3 3,999 R\*; Canyon Exceed CF SLX 1,799 R\* (2019, stale) | 2,699 | **2,699** |
| **Bar tape** entry / mid | Fizik Tempo 19.90 R, Ritchey Comp Cork 19.99 R, Selle Italia Smart Tape 13 R / Supacaz Super Sticky Kush 41.90 R, Fizik Vento Tacky 39.90 R, Zipp SC CX 31 R | 19.90 / 39.90 | **20 / 40** |
| **Bottle cage** (each) plastic / alloy / carbon | Elite Custom Race 15.60, Tacx Deva 14.99, SKS Topcage 9.99 / Elite Ciussi 13.95, Lezyne Power Cage 12.95, Supacaz Fly Cage 24.90 / Elite Vico Carbon 31, Ritchey WCS Carbon 64.40, Tacx Ciro 17.99 (all R) | 14.99 / 13.95 / 31 | **15 / 14 / 31** |
| **Lights** front + rear (StVZO) | busch+müller Ixon Fyre + IXXI 79.90 R; Lezyne Micro 300 + KTV Drive 74.95 R; Knog Blinder 80 + Plus 99.99 R | 79.90 | **80** |
| **Computer** entry / top | Garmin Edge 540 399.99 R, Wahoo Elemnt Bolt 3 329.99 R / Garmin Edge 1050 749.99 R, Wahoo Elemnt Roam 3 449.99 R | 364.99 / 599.99 | **365 / 600** |
| out-front mount | K-Edge Max 65 R; Garmin aero mount 39.99 R; PRO stem mount 59.95 R | 59.95 | **60** |
| **Top-tube bag** | Topeak Fuel Tank 42.95 R; Ortlieb Fuel-Pack 55 S; Apidura Racing Top Tube Pack ~62 S | 55 | **55** |

**Notes on the research:** most brands' own pages show no EUR price or block automated access, so retailer-shown UVPs dominate; they can lag behind brand price changes or sit a little above the launch price (Ultegra Di2 at 2,549 vs. lower list prices elsewhere). Street prices for groupsets are currently 40–55 % below RRP – the catalogue uses RRP throughout, so in-game prices are "list prices". Gaps: no EUR RRP for Shimano GRX (only GBP and street), DT Swiss P 1800, Vision Metron 5D, Canyon framesets; framesets are the least certain tier (several R\* values) and are the first to re-check. Weights in 4I are typical catalogue values and are researched with the prices in the same file.

**Refreshing prices:** never live scraping and no price API at runtime or build time (prices change rarely; scraping is fragile and against many shops' terms). The references live in `assets/parts-sources.toml` (per tier: product, brand, price, RRP or street, URL, date checked); prices are **re-researched by hand** about twice a year or when a catalogue tier is added (a person or an agent with web search fills in the file), and `cargo run -p ocw-assets -- parts reprice` recomputes the medians, applies `ceil` and rewrites the `hearts` of `assets/parts.toml`, printing the changes; the validator fails when `hearts ≠ ceil(median(sources))` or a source is older than 18 months (warning at 12). A price change never touches owned parts (refunds use the price paid).

#### 4I – Catalogue file `assets/parts.toml` (schema and examples)

Next to `assets/models/manifest.toml`, validated by `ocw-assets parts validate` (with the model and licence rules of §8.1). One `[[part]]` per catalogue entry:

| Key | Type | Meaning |
|---|---|---|
| `id` | string | stable id, `<slot>.<kind>.<tier>` (never reused, §6.2 rule for ids) |
| `slot` | enum | one of the slots of 4B (`frame`, `wheels`, `tyres`, `groupset`, `crankset`, `cassette`, `chain`, `cockpit`, `bar_tape`, `saddle`, `seatpost`, `pedals`, `bottle_cages`, `lights`, `computer`, `bag`, `paint`, apparel slots) |
| `tier` | enum | `base`, `entry`, `mid`, `upper`, `top` |
| `name` | string | generic English name, no brand (translations later) |
| `hearts` | int | `ceil(median EUR)` of the sources; 0 for `base` |
| `weight_g` | int | real typical weight (pair for wheels, tyres, pedals) |
| `fits` | list | bike types: `road`, `tt`, `gravel`, `mtb` |
| `standard` | table | optional: `speeds`, `shifting` (`mech` / `electronic`), `brake`, `wheel` (`700c` / `29`), `width_mm`, `family` |
| `profile` | string | frames only: the physics profile id (§2.5) |
| `rolling` / `aero` / `efficiency` | int | optional class steps (4G), default 0 |
| `model` | string | glTF node set / file in `assets/models/manifest.toml` (generated, §2.5) |
| `sources` | string | key into `assets/parts-sources.toml` (references, prices, dates) |
| `priced` | date | date of the research the price comes from |

Example entries (TOML):

```toml
[[part]]
id = "groupset.road-electronic.top"
slot = "groupset"
tier = "top"
name = "Race 12-speed electronic groupset"
hearts = 4135                      # ceil(median 4,135.00)
weight_g = 2450
fits = ["road", "tt"]
standard = { speeds = 12, shifting = "electronic", brake = "disc", family = "road" }
efficiency = 1
model = "groupset-electronic-top"
sources = "groupset.road.top"
priced = 2026-11-13

[[part]]
id = "tyres.road.performance"
slot = "tyres"
tier = "upper"
name = "Performance road tyres 28 mm (pair)"
hearts = 164                       # ceil(2 × median 81.93)
weight_g = 540
fits = ["road", "tt"]
standard = { wheel = "700c", width_mm = 28 }
rolling = -1
model = "tyre-road-slick-28"
sources = "tyres.road.performance"
priced = 2026-11-13

[[part]]
id = "frame.road-aero.top"
slot = "frame"
tier = "top"
name = "Aero race frameset, carbon"
hearts = 5279
weight_g = 1250
fits = ["road"]
profile = "road-aero"
aero = 1
model = "bike-road-aero"
sources = "frame.road.top"
priced = 2026-11-13
```

The first catalogue (about 30 entries; base parts at 0 ♥ for every slot and bike type come in addition; weights typical, to be confirmed with the research):

| id | Name (in game) | ♥ | Weight | Fits | Classes |
|---|---|---|---|---|---|
| `frame.road.mid` | Alloy road frameset | 1,390 | 1,650 g | road | profile `road` |
| `frame.road.upper` | Carbon road frameset | 2,839 | 1,250 g | road | `road` |
| `frame.road-climbing.top` | Lightweight climbing frameset, carbon | 5,279 | 950 g | road | `road-climbing` |
| `frame.road-aero.top` | Aero race frameset, carbon | 5,279 | 1,250 g | road | `road-aero`, aero +1 |
| `frame.tt.top` | Time-trial frameset, carbon | 5,999 | 1,800 g | tt | `tt` |
| `frame.gravel.upper` | Carbon gravel frameset | 2,799 | 1,350 g | gravel | `gravel` |
| `frame.mtb.upper` | Carbon cross-country hardtail frame | 2,699 | 1,500 g | mtb | `mtb` |
| `groupset.road-mech.entry` | Endurance 11-speed mechanical groupset | 951 | 2,950 g | road | |
| `groupset.road-mech.mid` | Endurance 12-speed mechanical groupset | 1,441 | 2,700 g | road, tt | |
| `groupset.road-electronic.mid` | Endurance 12-speed electronic groupset | 1,817 | 2,850 g | road, tt | |
| `groupset.road-electronic.upper` | Race 12-speed electronic groupset, upper tier | 2,549 | 2,600 g | road, tt | efficiency +1 |
| `groupset.road-electronic.top` | Race 12-speed electronic groupset | 4,135 | 2,450 g | road, tt | efficiency +1 |
| `groupset.gravel.mid` | Gravel 1×12 electronic groupset | 1,415 | 2,800 g | gravel | |
| `groupset.gravel.top` | Gravel electronic groupset, top tier | 2,479 | 2,550 g | gravel | efficiency +1 |
| `groupset.mtb.mid` | Trail 1×12 groupset | 1,240 | 2,500 g | mtb | |
| `groupset.mtb.top` | Cross-country 1×12 electronic groupset | 2,450 | 2,200 g | mtb | efficiency +1 |
| `crankset.road.power` | Road crankset with dual power meter | 899 | 720 g | road, tt, gravel | (cosmetic) |
| `wheels.road-alloy.entry` | Alloy training wheelset | 299 | 1,900 g | road, tt | |
| `wheels.road-alloy.mid` | Alloy performance wheelset | 600 | 1,600 g | road, tt | |
| `wheels.road-carbon.mid` | Carbon 45 mm wheelset | 1,415 | 1,450 g | road, tt | aero +1 |
| `wheels.road-carbon.top` | Carbon 50 mm race wheelset | 3,900 | 1,350 g | road, tt | aero +1 |
| `wheels.tt.disc` | Disc rear wheel (with 80 mm front from the set) | 2,750 | 1,000 g | tt | aero +2 |
| `wheels.gravel.carbon` | Carbon gravel wheelset | 2,050 | 1,450 g | gravel | |
| `tyres.road.training` | Training road tyres 28 mm | 61 | 640 g | road, tt | rolling +1 |
| `tyres.road.performance` | Performance road tyres 28 mm | 164 | 540 g | road, tt | rolling −1 |
| `tyres.road.race` | Time-trial tyres 25 mm | 190 | 460 g | road, tt | rolling −2 |
| `tyres.gravel.fast` | Fast gravel tyres 40 mm | 140 | 900 g | gravel | rolling −1 |
| `tyres.mtb.xc` | Cross-country tyres 29 × 2.25" | 120 | 1,300 g | mtb | rolling −1 |
| `cockpit.road.carbon` | Carbon handlebar and alloy stem | 405 | 420 g | road | |
| `cockpit.road.integrated` | Integrated carbon aero cockpit | 679 | 380 g | road | aero +1 |
| `cockpit.tt.extensions` | Clip-on aero extensions | 198 | 450 g | road | (cosmetic on road; TT position is in the `tt` profile) |
| `saddle.road.top` | Short-nose carbon saddle | 310 | 160 g | road, tt, gravel | |
| `seatpost.carbon` | Carbon seatpost | 240 | 190 g | road, gravel, mtb | |
| `pedals.road.top` | Carbon road pedals | 230 | 230 g | road, tt | |
| `pedals.spd.mid` | Two-sided off-road pedals | 150 | 340 g | gravel, mtb | |
| `bottle_cages.carbon` | Carbon bottle cage (each) | 31 | 20 g | all | |
| `lights.set` | Front and rear light set | 80 | 120 g | road, gravel, mtb | |
| `computer.top` | Large-screen bike computer with out-front mount | 660 | 210 g | all | (cosmetic: HUD bike-computer skin) |
| `bar_tape.mid` | Thick tacky bar tape | 40 | 90 g | road, gravel | |
| `bag.top_tube` | Top-tube bag | 55 | 150 g | road, gravel, mtb | aero −1 |

#### 4J – Tests (spec §11.4: no step without tests)

- **Catalogue:** `assets/parts.toml` parses; ids unique and stable (a fixture of earlier ids must still exist); `hearts == ceil(median(sources))` for every part; base parts cost 0 and exist for every slot and bike type; no brand names (a deny-list of the researched brands and their product names checked against `name`, `id` and the model names); every model reference resolves; sources not older than 18 months.
- **Physics (`ocw_core::bike`, pure, wasm):** build mass / Crr / CdA / efficiency from parts; class steps; the build caps (best build ≤ +1.5 % on the flat, ≤ +2.5 % on 6 % against the profile; the mass floors); *Profile* and *Equal* modes ignore parts; unknown and not-owned parts count as base parts; the client's values are never used.
- **Hearts (pure ledger model):** earning from work / climbing / workouts / events / achievements (no daily cap); zero with virtual power, in god mode, while down, for bots, in captures; balance = sum of the ledger; purchase rejects too low a balance, an incompatible part and a stale price (the server's price wins); idempotent purchase (the same key twice = one purchase); refunds (full before the first ride, 50 % floor after); repair prices; the one-time import cap. Server-side tests against an in-memory store; the API tests in Phase 5.
- **Workshop model (Bevy-free, `apps/desktop/src/workshop/`):** zone and bike navigation (left / right wrap, up / down between zones), the camera state machine, the swap sequence, hotspot screen-space navigation (nearest in direction from fixed projections), picker groups and filters, preview / restore, compare, every action → expected console command line (as the menu, §3.2.1).
- **Input on every workshop screen** with keyboard, mouse, gamepad and touch (§3.2.2: the same reachability tests as the menu pages); orbit / zoom with each device.
- **Rendering:** headless screenshots (X11 + Wayland, `scripts/scenes.toml` scenes `workshop-overview`, `workshop-wall`, `workshop-stand-hotspots`, `workshop-picker`, `workshop-mirror`) and a short video of a bike swap and an orbit for the build history; all bikes with every part variant through the validator (triangle budget, nodes, `hs_*` anchors).

#### 4K – Milestones (proposal)

- **W1 – Scene:** generated workshop room, stand, wall hooks, the six starter bikes, camera states, bike choice with left / right and the swap, all four input devices; ride window.
- **W2 – Parts:** `assets/parts.toml` and sources, `hs_*` anchors and part variants in the generator (wheels, tyres, cockpits, saddles, groupset looks …), hotspots, picker, live preview, compare; the local wallet and ledger, buy / equip / unequip / sell, console commands.
- **W3 – Physics:** class steps and build caps on the server, `BikeChange` message, *Profile* race mode, the effect lines on the cards.
- **W4 – Character corner:** mirror, body / skin / hair / kit slots, apparel prices, try on the bike.
- **W5 – Backend (Phase 5):** account ledger and inventory, purchase API, crediting by game servers and from uploaded rides, the one-time import.
- **W6 – Later:** persistent damage and repairs (if Phase 12 Q6 says so), achievements' hearts (Phase 7), translations.

#### 4L – Questions (answered 2026-11-13 in the build dashboard)

- **Q1 – When changes apply: decided (2026-11-13, dashboard):** at once when leaving the workshop (in free ride and outside events; new `BikeChange` message) – the proposal; the v2.7 "from the next start" rule is replaced.
- **Q2 – Physics of parts: decided (2026-11-13, dashboard):** bounded effects (best build ≤ +1.5 % on the flat) with the new *Profile* mode as the default for races and group rides – the proposal.
- **Q3 – Earning rate and start: decided (2026-11-13, dashboard):** 1 ♥ per 10 kJ + 1 ♥ per 20 m climbing, **no daily cap**, 500 ♥ at the start. Not part of the answer, so not adopted: back-crediting saved rides (the proposed one-time credit up to 2,000 ♥ is dropped unless the user asks for it).
- **Q4 – Apparel: decided (2026-11-13, dashboard):** kit (helmet, shoes, clothing) priced in hearts like parts; **body, skin, hair and glasses always free**.
- **Q5 – Touch on the kiosk: postponed (2026-11-13, dashboard):** no touchscreen testing for now; touch is covered by synthetic-event tests until the user brings it up again.

#### 4M – Status (2026-11-14, v3.00; W3 v3.03; W4 v3.07): W1 – W4 built

- **Scene (W1):** `ocw_assets::workshop` generates the room (concrete floor, walls, ceiling, window, pegboard with tools, bench and parts shelf, the changing corner with mirror, bench and locker, the repair stand, one hook per bike plus the "+" hook with card plates; ≈ 7.7 k triangles for six bikes, < 40 k for 24) and its `Layout` (stand pose, hook poses per bike size, views). `ocw-render::workshop` draws it on render layer 7 with its own camera (the default UI camera while open; the ride's overlays are hidden), a shadow-casting spot over the stand and warm fills; the world camera renders the ride into an image at a quarter of the window size and 10 fps for the **ride window** (top right, `workshop.ride_window off`). *Workshop* in the menu runs `workshop open` (the menu closes, the ride holds autodrive as under the menu); Back / Esc leaves to the menu's top page; the rider list and kit colours moved to the mirror corner (`menu workshop-page`) until W4.
- **Model (`apps/desktop/src/workshop/`, Bevy-free):** `model` (zones wall – stand – bench – mirror with ↑ / ↓, ← / → cycling the stand bike first then the hooks with wrap-around, Confirm: the 1.2 s swap or the stand view, Back one level up), `camera` (Overview, Wall(i), Stand orbit 0.8–4 m / −10° … 70° with 0.3 s inertia, Part close-up at the left third, Bench, Mirror; 0.6 s eased moves from the shown pose, never cuts), `nav` (screen-space hotspot navigation within 60° of the direction; pointer hits with at least the 48 px touch circle; bike boxes), `input` (keyboard, mouse with hover-only-when-moved and drag ≠ click, gamepad with a once-per-push stick, touch recogniser: tap, double tap = reset, long press = compare, drag = orbit, swipe = cycle, pinch = zoom, two-finger tap = back, a third finger cancels; mouse ignored for 0.5 s after a touch; the hint bar follows the last device), `picker`, `wallet`. Every action is a console line (`bikes.select`, `workshop.buy` …, `workshop.apply` on leaving).
- **When it applies (Q1):** the swap and the parts change the garage at once (saved), the server gets `BikeChange` only when the workshop closes with a change (`Out::Leave { apply }`); outside the workshop `bikes.select` / `workshop.buy` … apply at once. The riding profile keeps the bike for ride switches; with a rider profile `riders.edit … --bike` saves it in `riders.toml`.
- **Parts (W2):** `assets/parts.toml` – the 40 entries of 4I with the 4H prices plus 36 base parts (0 ♥; a base per slot and bike type, a base frameset per profile; lights, computer and bag start as "none", so a bike has no lights until they are bought, Phase 9 Q3) – and `assets/parts-sources.toml` (every 4H reference; pairs `multiply = 2`, cockpits and the computer as sums of group medians). `ocw-assets parts validate | reprice`; the validator checks ids, `hearts = ceil(median)` in exact half-cents, free base parts for every slot / type, the brand deny-list on names, ids and models (generic words such as Time, Race, Look are allowed), model references against the generator's variants (slot and bike types), source ages (warning 12, error 18 months). The generator adds 17 `hs_<slot>` anchors to every bike (the validator requires them) and 30 variants (`bikes::build_with`): rim depths, spokes, the TT disc, tyre widths / knobs / tan walls, cable or electronic groupset looks, a power-meter spider, carbon / integrated / clip-on cockpits, thicker tape, a short-nose saddle, carbon seat post, road and two-sided pedals, carbon cages, lights (lenses emissive), an out-front computer, a top-tube bag; the stock bike equals the shipped GLB.
- **Picker and wallet:** rows *equipped*, *owned* (spare or on another bike; the base part to go back to) and *shop* (fitting parts by tier, cheapest first; a lock with the missing hearts), live preview on the stand bike, detail card (weight against the fitted part, the class steps of 4G – **cosmetic until W3**, said on the card), compare (pin A, words better / worse), dialogs with Cancel focused (buy with the balance before → after, equip, unequip, sell with its refund, paint free). The local wallet (`<data dir>/ocw/workshop.toml`, append-only `hearts-ledger.jsonl`): per rider the six starter bikes and 500 ♥, balance = sum of the ledger, purchases only when fitting and affordable, refund in full before 1 km ridden with the part, then 50 % (floor), base parts and starter bikes not sellable; broken files are kept as `.broken`.
- **Hearts:** the server counts per rider the accepted work and the climbing (`ocw_core::hearts::HeartsMeter`, at most 2 m of distance per tick so a teleport is no climb) and sends the total in `RiderUpdate.hearts`; the client books the increase into the wallet only from the local server and not in captures or with `--no-hearts` (the kiosk's show mode passes it); virtual power is flagged per sample.
- **Tests:** camera moves (eased, no cut, 0.6 s, limits, inertia), zones / cycling / swap / leaving and "applies only when leaving", hotspot navigation, pointer and touch hits, every device's mapping and the gesture recogniser, picker groups / filters / locks / dialogs → console lines / compare / preview, wallet rules and persistence, earning rules, the catalogue validator and deny-list, BikeChange and hearts round trips, the server's BikeChange rule (at once / not in a race / unknown profile = default) and hearts (none for virtual power, bots, god mode), app tests with keyboard, gamepad and touch through the real plugins. Screenshots (X11 + Wayland): `workshop-overview`, `workshop-wall`, `workshop-stand-hotspots`, `workshop-picker`; video `workshop-swap-video`.
- **W3 – Physics (v3.03):** the build physics of 4G on the server (*Implemented* there): `BikeChange` applies the parts' weights and class steps within the caps (in an event: when it is over), the Hello carries the start build, `RiderBuild` tells everyone once per change (the snapshots only a 4-byte hash), so the others see the fitted parts on your avatar (`ocw-render::riders` builds a bike per profile and variant set once). Race modes *Open* / *Profile* (default for races and group rides) / *Equal* in the event setup and state, shown on the start line and in the results; free ride and single player *Open*. The client's prediction rides the same build and mode as the server and follows the server's `RiderBuild`. The picker's detail card shows the part's speed effect on this bike ("Speed: +0.4 % flat, +0.6 % at 6 %", "capped" at the band's edge), the bike's mass and speed against the stock bike, the server's word on the ridden bike ("Riding now (server): …") and where parts count; the compare card the speed difference of the two parts on the bike; the wall cards the bike's speed line. `dev.bot_bike` (local servers) gives a bot a bike, parts (`--best`), steady power, a fixed lane and a name. Scenes: `workshop-picker-effect`, `sprint-open-video`, `sprint-open-results`, `sprint-profile-results`.
- **W4 – Character corner (v3.07):** Confirm at the mirror (or `workshop mirror`) opens the editor (*Dress* view: over the rider's shoulder into the mirror; the rider stands 1.15 m in front of it). **Reflection:** a second camera at the workshop camera's eye mirrored at the glass plane, looking straight at the glass (its picture plane is the glass's), the near plane on the glass, a `SubCameraView` of a frustum centred on the eye's foot point that is exactly the glass rectangle, rendering layer 7 at half the window height into an image that an unlit quad on the glass (layer 8, seen by the workshop camera only) shows with the matching UVs – exact for every camera position, no shader; `workshop.mirror off` (low graphics quality – there is no global quality setting yet) skips it and the editor looks straight at the rider. **Navigation:** the five hotspots on the rider (head, torso, hands, legs, feet) by screen-space direction, hover / click and tap; LB / RB (Q / E) through the slots helmet, cap, glasses, hair, hair colour, face, jersey, body, skin tone, gloves, bib shorts / tights, socks, shoes, *Try on the bike*; Confirm opens the slot's picker; the orbit turns the rider; Back steps out (picker → editor → corner → overview). **Free slots** (body build, 12 skin tones, 4 faces, 6 hair styles, 8 hair colours, 3 glasses; the height is the rider profile's) apply at once as a `character` line (stored in `riders.toml`); **kit slots** list the worn item, owned items, the base item and the shop with locks, buy / wear / take off / sell with the parts' refund rules (`workshop.buy helmet.tt.top`, `workshop.unequip helmet`), live preview on the rider, compare. *Try on the bike* puts the rider on the stand bike (the saddle at their height, the riding pose at 45 rpm, the rear wheel turning); Back stands them up. **Kit physics (3.04, 3.05):** apparel items in `assets/parts.toml` (slots jersey, shorts, socks, shoes, gloves, helmet, cap; one free base item each, fitting every bike), aero steps of 0.0008 m² (aero jersey, aero bibs, aero socks, overshoes +1, skinsuit and aero road helmet +2, TT helmet +4), race shoes efficiency +1 (0.1 %), weights counting for helmets and shoes (after the bike's mass floor), comfort items none, a skinsuit `covers` the shorts; bike and kit share the caps, now held at 150, 250 and 400 W; Open only. Scores (road bike, 250 W / 75 kg): TT helmet 0.6 / 1.8 / 3.9 W at 25 / 35 / 45 km/h, +0.3 % flat; aero road helmet and skinsuit +0.2 %; aero jersey, bibs, socks, overshoes +0.1 %; race shoes +0.2 % at 6 %; light helmet +0.1 % at 6 %; TT helmet + skinsuit +0.5 %, the best kit +0.6 % flat. **Benefit lines** (`ocw_sim::build::Benefit`) on the bike-part and kit cards and the compare card: aero items by speed with "pays off above ~30 km/h", weight items by the climb. **Others see your look:** `RiderLook` carries the character (fields 5–11), sent in the Hello and `BikeChange` and to everyone once per change in `RiderBuild.look` (9; also to joiners), never in the snapshots' looks; the kit as catalogue ids with the bike's parts (`Hello.parts`, `BikeChange.parts`, `RiderBuild.parts`); avatars wear the kit variants, skin, hair, height (scaled, saddle moved) and build. The browser's anonymous rider: `character` / `workshop` answer "log in to customize". **Prices** (ceil(median) of 3–4 EUR list prices per item, `apparel.*` in `assets/parts-sources.toml`, researched 2026-11-14, many R\* from retailers; brands only in the sources and the deny-list): thermal jersey 130, aero jersey 210, skinsuit 430, winter tights 180, aero bib shorts 250, tall socks 20, aero socks 34, carbon race shoes 400, aero overshoes 70, gel mitts 30, full-finger gloves 60, winter gloves 108, lightweight helmet 320 (dearer than the aero road helmet in the research), aero road helmet 268, TT helmet 500, cotton cap 30 ♥. Scenes: `workshop-mirror`, `workshop-helmet-picker`, `workshop-try-on`, `workshop-kit-video`.
- **Still open:** W5 (backend ledger; with accounts the server checks that the parts are owned), W6; the bench catalogue (4D: buying framesets, the "+" hook); the groupset equipping its crankset / cassette / chain; the ride's bike lights still follow the Phase 9 placeholder, not the fitted lights; the trainer's SIM parameters (Crr, CdA) still send the profile's values, not the build's; the egui console draws into the ride window while the workshop is open; touch on a real screen (Q5 postponed); from W4: a global graphics-quality setting (the mirror's reflection is its own switch); the far LOD shows the base look and the default height; the standing pose is the model's rest pose (arms a little out), no idle animation; kit colours beyond the jersey / helmet colours (shorts, shoes and gloves keep the model's colours).

**Result:** Individually customizable riders.

### Phase 5 – Multiplayer

- Accounts.
- Sessions.
- Networked server: the existing local-server logic runs remotely. (Started: QUIC transport, dev-mode TLS, `ocw-server --listen`, `ocw-desktop --server`, §6.2; tested across machines and operating systems with headless `ocw-bot` clients and the kiosk, `make test-multiplayer`, §11.4.1.)
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

**Done so far (first increment, see §7 *Implementation*):** room and token management in the game server (`VoiceTokenRequest` / `VoiceGrant`, `Welcome.voice`, `Snapshot.human_riders`), the `ocw-voice` crate (always-on join logic, mute switches, spatial attenuation and nearest-N, device fallback, mixer, controller), the LiveKit backend behind the `voice` feature, hotkeys, console commands, HUD indicator, Settings → Voice (M8), the env-gated two-participant test; the echo test (an Echo rider plays the own microphone back, delayed, from its place on the road: local loopback on solo rides, an echo participant for a LiveKit server; v2.96). Open: CI with a LiveKit service, the Windows build of the feature, accounts-based identities (Phase 5).

### Phase 7 – Production Readiness

- Performance optimization.
- Large regions.
- Weather and times of day (planned in detail as Phase 9: sky, time machine, weather machine, real weather).
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

### Phase 8 – Textures and Regional Look (planning, 2026-11-12)

**Goal:** the world looks like the real place. Meadows, forests, roads, roofs and façades in Liechtenstein look like Liechtenstein; in the Black Forest like the Black Forest. Riders and bikes get real materials instead of flat paint slots. Everything shipped must have a licence compatible with our MIT / Apache-2.0 code, and nothing may come from proprietary map imagery (§4.6, §13).

#### 8A – Licence rule for textures, imagery and models

Code stays MIT / Apache-2.0. Data and assets keep their own licence and are listed in the package / asset manifest (§8.1, §13). "Compatible" here means permissive, without share-alike, non-commercial or no-derivatives clauses:

| Licence | Shipped in packages / app? | Obligation |
|---|---|---|
| CC0 1.0, public domain, own work (MIT / Apache-2.0) | Yes | none (we still credit the source) |
| CC BY 4.0 (e.g. ESA WorldCover) | Yes | attribution on the credits screen |
| Datenlizenz Deutschland zero / by 2.0 (German state geodata) | Yes | by-2.0: source note (e.g. "© LGL, www.lgl-bw.de") |
| Swiss / Liechtenstein OGD "BY" (open use, source must be named) | Yes | source note, also downstream |
| Copernicus Sentinel data licence | Yes | "Contains modified Copernicus Sentinel data [year]" |
| ODbL (OSM) | Yes (Produced Work) | "© OpenStreetMap contributors", as today |
| CC BY-SA 4.0 (e.g. Mapillary, older EOX mosaics) | **No** in packages; reference use only (looking at it, never copying pixels) | – |
| CC BY-NC(-SA) (EOX Sentinel-2 cloudless 2018–2025) | **No** | – |
| Google / Bing / Apple / Esri / Mapbox imagery, Street View | **No** pixels in assets, and never traced into geodata (positions, outlines, road widths). Looking at photos to learn what things look like is fine (8D) | – |

#### 8B – Research results: aerial and satellite imagery

Yes, usable imagery exists for both regions, at very high resolution and under open licences (as of 2026-11-12; the exact terms must be re-checked per dataset and year before use, see Q1):

| Region | Source | Resolution | Licence (found) | Notes |
|---|---|---|---|---|
| Liechtenstein | **Orthophoto Liechtenstein** (Amt für Tiefbau und Geoinformation, geodaten.llv.li; listed on opendata.swiss for 2003 … 2025, 2025 = SwissImage) | 10–25 cm | varies by year: 2018 "Opendata BY – open use, source must be named"; older years non-commercial. **Checked 2026-11-13:** 2025 (10 cm, summer flight, published 2026-02-12) is "Opendata BY" on opendata.swiss – used (T3). The WMS (service.geo.llv.li, layer `li.atg.orthophoto2025`) says "service free for non-commercial use, datasets tagged OpenData under CC BY": we only fetch the open dataset through it | GeoTIFF download and WMS; ATG holds the distribution rights for SWISSIMAGE and swissALTI3D in Liechtenstein |
| Liechtenstein / Switzerland | **swisstopo SWISSIMAGE 10 cm** | 10 cm (plains, main valleys), 25 cm (Alps) | OGD: open use, source must be named (central attribution accepted) | also **swissALTI3D** (0.5–2 m terrain) for a better DEM than SRTM |
| Black Forest (Baden-Württemberg) | **LGL DOP20** (digital orthophotos) via opengeodata.lgl-bw.de | 20 cm | **dl-de/by-2.0**, free of charge since 2024-06-09 | also **DGM1** (1 m terrain) as open data since 2024; **LoD2 building models**: open-data status unclear (an LGL price list exists) → Q8 |
| Europe / world | **Copernicus Sentinel-2** L2A scenes (Copernicus Data Space) | 10 m | Copernicus Sentinel data licence: free, full and open, attribution | we build **our own** cloud-free summer median composite, so no third-party mosaic licence applies |
| Europe / world | EOX Sentinel-2 cloudless **2016** | 10 m | CC BY 4.0 | usable but old; 2018–2025 layers are CC BY-NC-SA → not usable |
| Europe / world | **ESA WorldCover** (land cover) | 10 m | CC BY 4.0 | already planned (§4.6) |
| Europe | **Copernicus HRL Forest Type / Tree Cover Density** (land monitoring) | 10–20 m | Copernicus land data: free and open, attribution | conifer vs. broadleaf and density → which trees grow where |

**How imagery is used (extends §4.6):**

1. **Far and mid distance:** the satellite / orthophoto colour tints the terrain (as planned in §4.6), now from the regional orthophoto where available, else Sentinel-2.
2. **Calibration (the main use):** the world builder *measures* the region instead of guessing it:
   - **Regional palettes** per land class: the median colour of meadow, farmland, forest, rock, settlement, asphalt in the orthophoto drives the tint of the tiling CC0 materials.
   - **Roof colour per building:** sample the orthophoto inside each OSM building footprint (eroded by 1 m), store one colour per building in the package (3 bytes). Red tiles in Vaduz, dark shingles on Black Forest farms come out right automatically. OSM `roof:colour` wins where it is set.
   - **Tree species and density** from Copernicus Forest Type / Tree Cover Density plus OSM `leaf_type`: spruce/fir in the Black Forest, mixed forest in the Rhine valley.
   - **Road surface and markings:** asphalt tone from the orthophoto along the road; OSM `surface`, `lanes`, `width` as today.
3. **Optional orthophoto drape near the road:** a corridor of about ±50 m around ridden roads at 0.4–0.5 m/pixel, blended with the materials at mid distance (never as the close-up texture, see §4.6). Estimate: 1 km of road ≈ 0.4 Mpx ≈ 50–100 KB as KTX2/Basis; Liechtenstein's network (≈ 600–800 km) ≈ 30–80 MB. To be measured (Q4).

**Tenerife (v2.42):** the Spanish **PNOA** orthophotos (25 cm, IGN / GRAFCAN, CC BY 4.0 "PNOA cedido por © Instituto Geográfico Nacional", WMS / download centre) cover the Canary Islands (**checked 2026-11-13:** the IGN's WMS `pnoa-ma`, layer `OI.OrthoimageCoverage`, states "CC BY 4.0 scne.es", no fees; mosaics of different flights at 25–50 cm, Sentinel-2 only at small scales – used for T3 at 1 m); GRAFCAN's own OrtoExpress services need an express authorisation and are not used. The island needs regional materials the other worlds do not: lava rock, volcanic ash / pumice, black sand, Canary pines, laurel forest, banana terraces (§4.7 "Tenerife").

#### 8C – Research results: textures and models for close-up surfaces, houses, riders and bikes

| What | Where from | Licence | Regional match |
|---|---|---|---|
| Ground (grass, forest floor, rock, gravel, snow, soil) | **ambientCG**, **Poly Haven** (also cgbookcase, ShareTextures – to verify) | CC0 | tinted with the regional palette (8B) |
| Road surfaces (asphalt types, cobbles, gravel, dirt), markings | ambientCG / Poly Haven + procedural markings | CC0 / own | OSM `surface`, colour from the orthophoto |
| Houses: roofs and façades | CC0 roof tile / plaster / timber / shingle materials + **procedural façades** (windows and doors from `building:levels`, `building` type, `roof:shape`, `roof:levels`, `building:material`, `building:colour`) | CC0 / own | **regional style presets**: Rhine valley (plaster, red tile, gable), Alpine Liechtenstein (timber upper floors, wide eaves), Black Forest farmhouse (huge hipped roof down to the ground floor, dark shingles, wood); roof colour from the orthophoto |
| Trees, bushes, grass | Poly Haven models, Quaternius / Kenney packs (CC0), or own procedural trees | CC0 / own | species and density from 8B |
| Riders (skin, kit fabric, helmets, shoes) | **generated** (Phase 4 rig and meshes), fabric normal maps from CC0 libraries, procedural jersey designs (stripes, panels, numbers) | own / CC0 | – (cosmetics, §2.5) |
| Bikes (frame paint, carbon weave, metal, rubber, saddle) | **generated** procedural materials (carbon weave, brushed aluminium, rubber) | own | – ; **no real brand names or logos** (trademarks) |
| Street furniture (signs, barriers, benches, fountains) | own low-poly models; sign faces drawn procedurally from OSM `traffic_sign` | own | country sign styles (CH/LI vs. DE) |

**Can they be generated? Yes – in three ways, in this order of preference:**

1. **Procedural (code):** noise-based materials, façades, jersey patterns, carbon weave and tree generators in the world builder / asset tool. Deterministic, tiny, MIT / Apache-2.0 by construction, testable (golden-image tests at low resolution). First choice for riders, bikes, façades and markings.
2. **Example-based from our own photos:** photos taken by us in the regions (bark, roof tiles, asphalt, walls) turned into tiling PBR materials (photogrammetry / texture synthesis). Own work, licensed like the code. Best for "looks like the real place".
3. **AI-generated textures:** possible, but the copyright of the output and the licences of models and training data are unclear and vary by model. Not for shipped assets until decided (Q3).

#### 8D – Reference photos: how things really look

Before a material, house style, tree, sign or bike part is designed, we look at **real photos of the region** to learn what it looks like: roof shapes and colours, façade materials, the asphalt, kerbs and markings, the forest, fences, signs, farm buildings, typical bikes and kit.

- **Sources:** Wikipedia / **Wikimedia Commons** (searchable by place and category, e.g. "Category:Vaduz", "Category:Black Forest houses", every file with its licence), municipal and tourism websites, general image search, and our own photos (Phase 8 Q6).
- **Looking vs. copying:** looking at any photo to understand shapes, proportions, colours and materials is fine, because facts and styles are not protected. Copying is a different matter: pixels in an asset, tracing outlines, or a recognisable reproduction of a specific photo are allowed only from files whose licence permits it under 8A (CC0, public domain, CC BY with attribution). CC BY-SA and unlicensed web images stay reference-only.
- **Reference boards:** per region a short Markdown file `art/reference/<region>.md` (in git) with links (not copies) to the photos used, what each shows (e.g. "Black Forest farmhouse: hipped roof down to the ground floor, dark wood shingles, balcony under the eaves") and its licence. Downloads for viewing go to a git-ignored cache; only permitted files may be used for textures, with a manifest entry.
- **Colour picking from photos** (e.g. the typical roof red of Vaduz) is reference use; the orthophoto sampling of 8B gives the measured values per building.
- **Agents** doing Phase 8 work start each design task by collecting 5–15 reference photos (Wikimedia Commons first), write them into the board, and compare their render with the references in the scene harness.

#### 8E – Pipeline and milestones (proposal)

- **T1 – Material library:** CC0 PBR set (ground, road, roof, façade, bark, fabric), KTX2/Basis compression, manifest entries with licence / source / author, validator extension (no unknown licence, no NC/SA/ND), credits screen entries. **Done (v2.46)** for ground, roads and houses (17 materials, §4.6 "Material library"); bark and fabric come with T4 / T6 (fabric, carbon and leather: T6, v3.12). Credits: `CREDITS.md` and the console command `materials` (a credits page in the menu is still open).
- **T2 – Terrain materials:** splat map (§4.6) → tiling materials with triplanar mapping on slopes; regional tint maps from Sentinel-2 (own composite) / orthophoto; far LOD colour. **Done (v2.46)** except the tint maps (T3; the hook is there): §4.6 "Terrain (T2)" and "Roads (T2)". Measured on marge (same build, Vaduz–Malbun km 1.6, 1280×720, `--no-vsync`, two interleaved 40 s runs each, machine shared with other builds): AMD Radeon RX Vega 250–360 fps with the library vs. 67–92 fps with `OCW_MATERIALS=off` (the flat path is not slower by its shaders: a per-frame mutable access to the road material assets was found and removed; the remaining difference is not understood yet), software Vulkan (lavapipe, the headless tests) 2.7–3.7 fps either way (no measurable cost). Within the budget.
- **T3 – Region calibration in the world builder:** orthophoto and Sentinel download per region (`fetch` scripts, licence table per source as for OSM/SRTM), palettes per land class, roof colour per building, forest type and density; package format extension (new sections, only added fields). **Done (v2.47) except forest type and density** (Copernicus HRL, open) and the orthophoto corridor (T7):
  - **Sources** (`scripts/fetch-imagery.sh REGION [PACKAGE...]`, after the package is built: `world-builder imagery-cells` lists the 1 km cells of its tiles in the source's CRS; rows appended to `data/<region>/SOURCES.md`): Liechtenstein – Orthophoto Liechtenstein 2025 (SWISSIMAGE, Opendata BY) at 0.5 m, LV95, then swisstopo SWISSIMAGE 10 cm (OGD) where the first has no data; Black Forest – LGL DOP20 (dl-de/by-2-0, "LGL-BW (2026) Datenlizenz Deutschland – Namensnennung – Version 2.0, www.lgl-bw.de") at 1 m, UTM 32; Tenerife – IGN PNOA máxima actualidad (CC BY 4.0 scne.es) at 1 m, REGCAN95 / UTM 28N. All over the providers' official WMS (one GetMap per 1 km cell, JPEG, four at a time), so only what a package needs is downloaded: Liechtenstein 563 cells (Orthophoto Liechtenstein 471 MB, SWISSIMAGE 656 MB at 0.5 m), Black Forest south 2,932 cells (DOP20 735 MB at 1 m), Tenerife 2,866 cells (PNOA 473 MB at 1 m; 21 cells answered 5xx and came on a second run). Sentinel-2: Liechtenstein 22 MB (12 scenes per season, winter 84.5 % clear, gaps filled from summer), Black Forest 98 MB (48 scenes per season over 4 MGRS tiles; winter 89.6 %), Tenerife 174 MB (48 per season; 99.5 – 99.8 %).
  - **Seasons** (`scripts/imagery.py seasonal`, GDAL): our own median composites of Copernicus Sentinel-2 L2A (Collection 1) true-colour COGs, clouds, cloud shadows, cirrus and no-data masked with the scene classification (kept: vegetation, bare, water, snow), the 12 clearest scenes per MGRS tile and season over 2023–2025: spring 1 Apr – 31 May (centre day 121), summer 15 Jun – 31 Aug (204), autumn 25 Sep – 10 Nov (291), winter 1 Dec – 28 Feb (14), at 20 m. Read from the AWS Open Data copy (Earth Search STAC, no account); the Copernicus Data Space Ecosystem has the same products but needs an account (open question for the user). Notice: "Contains modified Copernicus Sentinel data 2023-2025". No EOX mosaics (NC-SA).
  - **Builder** (`world-builder calibrate --world PKG --imagery DIR --pbf OSM`, run by the build scripts when `imagery/packages.txt` lists the package; `ocw-worldgen::imagery`, `::calibrate`): local metres → CRS by an affine map fitted per tile (exact to well under a centimetre over 1 km), orthophoto cells decoded with a small cache per thread, all cores. Measured palettes: Liechtenstein meadow #5f744d, forest #374335, scree #97958e, paved roads #969899, roofs #706c67; Black Forest meadow #5f634f, farmland #6f7856, forest #3a4138, paved roads #958e88, roofs #766c6b; Tenerife meadow #615d5a (the pumice of Las Cañadas, which OSM tags as meadow, no longer olive), rock #524b47, forest #393c3a, paved roads #606061, roofs #a38f85. Calibration time and peak memory on marge: Liechtenstein 35 s / 2.1 GB, Black Forest south 238 s / 3.9 GB, Tenerife 72 s / 2.2 GB.
  - **Package cost:** Liechtenstein +4.9 MB (+16 %), Black Forest south +26.1 MB (+10 %), Tenerife +23.1 MB (+9.7 %), almost all of it the seasonal maps; the raw imagery stays in the git-ignored `data/<region>/imagery/` (1.1 GB, 0.8 GB, 0.6 GB). The orthophoto corridor (T7, accepted in Q4) came with v3.10 (below): JPEG chunks per tile in the section `ORTC`, 51 MB for Liechtenstein.
  - **Renderer:** §4.6 "Region calibration (T3)". Comparison renders (scene harness, `look-*` scenes): Vaduz roofs, Malbun meadows, Black Forest farms and forest, Las Cañadas, and the Malbun road in summer / autumn / winter by the time machine.
  - **Tests:** palette extraction on synthetic images (interior cells, roads and houses masked, Sentinel fallback), roof sampling (erosion, courtyards, no data, small sheds, `roof:colour` precedence and parsing), seasonal weights by day of year (cyclic, sums to 1), tint-map codec, gap filling, section round trips with old readers skipping them, the imagery readers on fixtures (no network), manifest credits of the imagery, the renderer's tints, roof colours and the date-blended far map.
- **T4 – Houses:** procedural façades and roof shapes from OSM tags with regional style presets; roof colour from T3; LOD. **Done (v2.57):**
  - **OSM facts** (world builder, `ocw-worldgen::buildings`): roof shapes gabled, hipped, half-hipped, pyramidal, skillion, mansard, gambrel (saltbox and round → gabled, side-hipped → half-hipped, cone / dome / onion → pyramidal); pitched shapes need a nearly rectangular footprint (≥ 85 % of its oriented rectangle; up to 400 m² untagged, 1,500 m² for farms and churches, 3,000 m² when tagged), pyramids also a convex polygon; ridge along the long side unless `roof:orientation=across` or `roof:direction` (the slopes' compass direction; a skillion's low side faces it); roof height from `roof:height`, `roof:angle` (over the half span, a skillion's whole span), `roof:levels` (2.5 m each + 0.5 m), else 0.4 × the span (0.3 for small buildings); `height` includes the roof; `min_height` / `building:min_level`; wall and roof material and colour; stored in `BTAG` (§4.7).
  - **Regional styles** (`assets/building-styles.toml`, built into the app, `ocw-render::house_style`): a style applies to the packages whose time zone it lists, optionally inside a latitude / longitude box and above an elevation (of the matching styles the highest threshold wins; the first style is the default) – the same data-driven way as the material sets. Presets per building kind and finer use, each field looked up along the use, the kind, the default style's use, the style's `other`, the default's kind and `other`: roof shape mix for untagged pitched roofs (flat turns them flat), pitch factor, eave drop (the eaves come down, the ridge stays: the big farm roofs), overhang, parapet, wall material mix, upper floors' material with its probability (timber upper floors and gables), plaster colours, roof material mix, clay tile colours, window style, chimney probability. Shipped: **central** (Rhine valley, towns: light plaster, red-brown tiles, gable 65 % / hipped 25 % / half-hipped 10 %), **alpine** (Europe/Vaduz, Zurich, Vienna, from 750 m: gable roofs at 0.6 of the pitch with 1.2 m eaves, timber upper floors, shutters, slate / tile / metal roofs), **blackforest** (Europe/Berlin in the Black Forest box from 450 m: farms with hipped or half-hipped roofs 1.25 × as high and 2.5 m lower eaves, timber walls, dark roofs; houses with green shutters), **canary** (Atlantic/Canary: 80 % flat roofs with 0.6 m parapets, white, cream and ochre walls, some tiled roofs, stone farms). Flat roofs are concrete / gravel unless `roof:material` says otherwise; untagged blocks over 350 m² (farms excepted) get no timber walls from a preset. Every choice left to chance is a hash of the building's position: deterministic. Tagged facts win; the measured roof colour (T3 `ROOF`) wins over `roof:colour` (a quarter of the way to the default, as in T3), which wins over the preset.
  - **Meshes** (`ocw-render::houses`): per shape the roof planes and the gable walls over the rectangle (half-hips at 55 % of the height keeping the pitch, mansard breaks at 65 % over 18 % of the shorter side, gambrel at 60 % over 20 %); walls from 0.5 m below the lowest ground (or from `min_height`) to the eaves; canopies (`building=roof`, carports) as a 0.35 m slab on posts. Faces carry their material layer(s), the colour and a façade code (storey height and count, window style, door bay); every roof shape is closed with its walls (tested).
  - **Façades** (`building.wesl`): the library's building layers – clay tiles, plaster, timber and, new with T4, roof slates, box-profile metal sheet, brick, rubble stone, concrete (Poly Haven, CC0, 1024², KTX2 like T1); glass untextured – tinted to the face's colour (texture ÷ its mean), normal-mapped, fading to the flat colour between 90 and 320 m; procedural windows per bay (a wall's length is divided into whole bays) and storey from the window style (bay, size, sill, frame colour, shutters, shop fronts on the ground floor), one door on the ground floor of the longest wall, the upper floors in their own material above the first storey, anti-aliased with screen derivatives, fading out between 420 and 900 m; windows light up warm at night (sun below 2° → fully at −6°, 45 % of the windows, Phase 9 night).
  - **Levels of detail** (Bevy `VisibilityRange`, from each mesh's origin): **near** per 250 m chunk up to 330–400 m (fading): eaves and verges extended along the roof's pitch with a 0.18 m roof slab, parapets on flat roofs, chimneys, canopy posts; **mid** merged per tile up to 1.9–2.0 km: walls and roofs without overhangs, 0.15 m inside the near mesh; **far** merged per tile from 1.3 km: no windows, no buildings under 30 m², 0.4 m inside, no shadows. Each coarser level sits inside the finer one, so where two are drawn the finer hides the other: no z-fighting and no gaps although chunks and tiles have different origins. Camera-solid up to each house's top (ridge, spire or parapet).
  - **Measured** (marge, AMD Radeon RX Vega, release build, `scripts/measure-fps.sh`: headless weston with the GPU, 1280 × 720, no vsync, 40 s with 12 s warm-up, runs interleaved): Vaduz centre 311 → 262 fps (3.22 → 3.82 ms per frame, +0.60 ms), St. Märgen 309 → 265 fps (3.23 → 3.77 ms, +0.54 ms), La Orotava 325 → 269 fps (3.08 → 3.71 ms, +0.63 ms), before = the build without T4 on the same rebuilt packages, two runs each; well within the 60 fps target (§15). The cost is the façade shader and the near meshes; the far level keeps distant tiles cheap. Package cost: `BTAG` is 0.02 MB in Liechtenstein (2,105 of 17,897 buildings with facts, 660 with a roof shape; +0.06 % of the package), 0.51 MB in the Black Forest south (51,729 of 306,478; 17,039 shapes; +0.18 %) and 0.53 MB in Tenerife (63,588 of 116,479; 949 shapes; +0.2 %).
  - **Tests:** tag parsing (shapes, uses, materials, directions, roof heights by tag, angle and levels, pyramids only on convex towers, facts on placed buildings), `BTAG` round trip, sparseness, unknown fields and layouts, old readers; style file validation, style selection by zone / box / elevation, preset chains, tagged facts winning, regional defaults (Canary flat roofs, Alpine pitch and eaves, Black Forest eaves), measured roof colours first; roof geometry per shape watertight with its walls, heights, ridge direction, skillion and pyramid apex, overhangs along the pitch, mid inside near, bays, storeys and one door, parapets, courtyards, canopies, chimneys; LOD selection and coverage of the visibility ranges, the tile's three meshes. Scenes `look-houses-*` (Vaduz, Triesenberg, Malbun, St. Märgen, Hofsgrund, La Orotava, Los Gigantes).
  - **Open:** pitched roofs on non-rectangular footprints (L-shapes stay flat: a straight skeleton would do), dormers and roof windows (`roof:levels`), balconies, `building:part` (towers on churches are their own footprint only when mapped as buildings), real 3D building data (Q8).
- **T5 – Roads:** surface materials, markings, kerbs and sidewalks in towns, wear and patches. **Done (v2.60):**
  - **Data** (`ocw_world::road_dress`, package section **`RDRS`**, version byte 1, only added; written for tiles where a strip differs from its class default, so packages without it and older readers keep working): per road strip the arc length of its first point along its edge (dashes, wear and textures run on across tiles; overlapping strips at tile borders agree), the edge length, lanes forward / backward, centre line (none, dashed, solid, double, solid on the forward or the backward side), edge lines, lane lines, cycle lanes left / right (lane, advisory, exclusive), sidewalk widths left / right (dm), whether the sidewalks are asphalt (`sidewalk:*:surface=asphalt|concrete`; else pavers), kerb height (cm), the **junction zones** at both edge ends (at a vertex of three or more edges: half the widest road there + 2 m) and the transverse markings (zebra, signalled crossing, stop line, give-way line; position along the strip, which half). About 13 bytes per strip.
  - **From OSM** (`world-builder`, `ocw_worldgen::dress`): `lanes`, `lanes:forward` / `:backward` (a one-way road: all lanes one way, no centre line), `overtaking[:forward|:backward]=no`, `lane_markings=yes|no`; `cycleway`, `cycleway:left|right|both=lane` with `…:lane=advisory|exclusive` (`opposite_lane` on one-way roads); `sidewalk`, `sidewalk:left|right|both` (`yes`, `no`, `separate`) with `…:width`; `kerb`, `sidewalk:…:kerb`, `kerb:height` (raised 12 cm, lowered / rolled 4 cm, flush 0); untagged or `separate` sides of a road get the sidewalks **mapped as separate ways** (`footway=sidewalk`, not rideable) that run parallel beside them (within 6 m of the edge on at least 40 % of 8 m samples; the width reaches from the kerb to just beyond the footway's centreline, 1.2–5 m). Way nodes (the OSM reader now keeps the tags of `highway=crossing|stop|give_way|traffic_signals` and `crossing=*` nodes): zebra markings (`crossing:markings=zebra`, `crossing_ref=zebra`, `crossing=zebra|marked|uncontrolled`, `crossing:markings=yes`) are zebras – also with signals –, `crossing=traffic_signals` / `crossing:markings=lines|dashes` two dashed lines with stop lines before them in both directions, `unmarked` / `no` nothing; `highway=stop|give_way` a line on the half of the traffic meeting it (`direction`, else towards the nearer edge end; at a junction node on the minor roads only, 3 m before the junction). Crossings are painted on the roads with a carriageway through the node (a footway crossing a road paints the road). Defaults without tags: main roads (trunk … unclassified) at least 5.5 m wide get a dashed centre line, trunk / primary / secondary from 6 m edge lines; residential, service, tracks, cycleways and paths are unmarked; no paint on gravel, dirt or cobbles (`surface`, T2).
  - **Measured OSM coverage, Liechtenstein:** 6,568 rideable ways; `lanes` on 811, `sidewalk*` on about 800 (of them `separate` 284 sides), 2,163 separate sidewalk ways, 1,107 crossing nodes (455 with `crossing:markings=zebra`, 501 `=no`), 344 give-way and 8 stop nodes; cycle lanes on 77 ways (35 advisory).
  - **Layout** (`marking_lines`, Rust twin of the shader): cycle lanes (1.5 m) along the edges replace the edge lines; the centre line divides the rest in proportion to the lanes each way; lane lines between lanes of one direction; edge lines inside the edges, not in towns (a street with a sidewalk is in a town, with the short town dashes). **Regional styles** (`assets/road-styles.toml`, by the package's time zone like the material sets and house styles): Germany (default; RMS: 12 cm lines, dashes 6 m / gaps 12 m, in towns 3 m / 6 m, edge lines 15 cm, cycle lanes exclusive, zebra bars 50 cm × 4 m), Switzerland / Liechtenstein (15 cm lines, 6 m / 9 m, **yellow zebras and yellow dashed cycle lanes**), Spain / Canary Islands (10 cm centre lines, 3.5 m / 9 m, wide 15 cm edge lines close to the edge).
  - **Rendering** (`ocw-render::road_mesh`, `road.wesl`): each strip vertex carries (across, along) in metres, its layer, its part (carriageway, kerb face, sidewalk, shoulder, wall, overlay) and the marking code; the colour's alpha is the half width. The shader integrates every line's coverage over the pixel's footprint in road metres (box filter in u and over the dash pattern in v, computed from the dash phase so kilometres of arc length keep their precision): crisp lines close up, their mean grey far away, no shimmer, no extra geometry. Zebras, signalled crossings, stop and give-way lines are short overlay ribbons 1.2 cm above the surface with the same filtered coverage, blended (alpha). Raised kerbs: a 12 cm kerb face and the sidewalk surface (2 % fall towards the road) in the new road layers **sidewalk pavers** (Poly Haven *Concrete Pavers*, CC0) and **kerb concrete** (Poly Haven *Concrete Floor 03*, CC0; also the kerb stones' 15 cm top band and the walls), tapering to nothing over 2 m into each junction zone (a lowered, rounded corner), none on bridge decks; the riding surface is unchanged, riders do not collide with or ride onto kerbs (they stop 0.6 m inside the edge, §5.2.1; wild mode rides the terrain as before). Wear on asphalt, deterministic per position: two tyre tracks per lane (darker, smoother), repaired patches (8 m cells, 6 % chance per half on asphalt, 22 % on worn asphalt), transverse cracks that fade out when a pixel is wider than a few centimetres, tone variation; tracks get grass between their wheel ruts. Without the material library the same shader draws the flat colours with the markings. The route's own centre line over package roads is gone (the roads carry their markings).
  - **Road edge:** the shoulder and walls of §4.8.3 ("Road edge into the terrain").
  - **Tests:** tag parsing and defaults (lanes, overtaking, lane markings, sidewalks, kerbs, cycle lanes, crossing nodes), the marking layout per class / lanes / cycle lanes and point samples, transverse markings, code round trips, kerb taper in junction zones, patches deterministic and rare, section round trip with defaults not written and newer versions skipped; builder: separate sidewalks found on their side, crossings placed on the roads through their nodes (stop on the minor road, before the junction), dress on the strips with the package round trip; renderer: the edge blend on a flat plain (nothing above the terrain beyond the 3 cm lip, the outer shoulder under the ground), walls on deep drops and narrow verges in cuttings, kerb / sidewalk geometry beside an unchanged riding surface, marking codes and arc length on the vertices, crossing overlays (not on cobbles), the built-in styles chosen by region, the uniform.
  - **Measured** (marge, AMD Radeon RX Vega, release builds, `scripts/measure-fps.sh` (headless weston with the GPU, 1280 × 720, no vsync, 40 s with 12 s warm-up, two interleaved runs each, the rider standing): Vaduz centre 2.25 → 2.41 ms per frame (445 → 416 fps, +0.16 ms), Freiburg Werthmannstraße 2.36 → 2.59 ms (425 → 387 fps, +0.23 ms), Las Cañadas 2.48 → 2.54 ms (403 → 395 fps, +0.05 ms); before = the build without T5 on the same rebuilt packages (it skips `RDRS`). The cost is the longer road meshes (shoulders, kerbs, sidewalks: about 2.5× the vertices of a strip) and the marking loops of the road shader. Package cost of `RDRS`: Liechtenstein +0.18 MB (35.05 → 35.23 MB, +0.5 %; 10,139 strips, 2,538 with sidewalks – 2,043 sides from separate sidewalk ways –, 174 with cycle lanes, 889 crossings / stop / give-way lines), Black Forest south +2.9 MB (285.0 → 287.9 MB, +1.0 %; 166,320 strips, 15,265 with sidewalks, 1,265 with cycle lanes, 2,790 crossings), Tenerife +1.5 MB (268.2 → 269.7 MB, +0.5 %; 80,576 strips, 6,261 with sidewalks, 12,784 crossings); the dress stage of the builder takes 0.04 s / 0.7 s / 0.5 s. Comparison renders (scene harness, `road-*` scenes, before / after): the Äulestrasse and its yellow zebra in Vaduz, the Werthmannstraße in Freiburg, a country road above Triesenberg, the edge of the TF-21 in Las Cañadas (the slab edge gone).)
- **T6 – Riders and bikes:** procedural fabric / carbon / metal / rubber materials on the Phase 4 models, jersey designer (patterns, colours, number), helmet and shoe variants. **Done (v3.12):**
  - **Materials** (`ocw-render::gear`, `gear.wesl`; one `ExtendedMaterial` for every near part that needs a shader): the kind of a part comes from its paint slot and node (`gear::kind_of`): *paint* (frame, decals: a clear coat), *carbon* (bare carbon parts, rims; a frame painted **bare carbon** – `workshop.paint carbon`, the frame colour `#0C0C0E`, older apps draw it near-black – shows the weave: a 2 × 2 twill under a clear coat), *metal* (cassette, chainrings, spokes, hubs: spun, fine rings about the node's axle with anisotropic highlights stretched along them; the anodised components – stems, posts, derailleurs – fainter), *tyre* (a slick centre strip, a file tread of 4 mm diamonds on the shoulders, a whole number round the wheel, rubber grain), *grip* (bar tape), *leather* (the saddle), *knit* (jersey, socks; mesh side panels under the arms), *lycra* (shorts, skinsuit, gloves, overshoes: a faint rib and a lower roughness for the sheen), *pad* (the gel gloves' palm), *plastic* (helmet shell, shoe uppers, bottles: gloss = a clear coat at roughness 0.28, matte = 0.7 without), *plain* (skin, hair, lenses, lights). Painted, plastic and plain parts are standard materials with these values (they batch across riders); the others use the shader. **Textures:** three CC0 materials of the library (group `gear`, `GEAR_IDS`; §4.6): the jersey knit (ambientCG *Fabric 070*, sportswear, 6 cm repeat), the carbon twill (*Fabric 004*, 4 cm) and the saddle leather (*Leather 027*, 15 cm), sampled triplanar in the part's rest coordinates and tinted to its colour; without the library (older folders, `OCW_MATERIALS=off`, the browser app) the same kinds are procedural (own work). **Coordinates:** every near mesh carries its rest coordinates (`gear::gear_mesh`: the rider in its rest pose with the body region per node, bike parts in their node, e.g. the wheel about its hub), so patterns stick to the moving body; the shader maps them to the world by the screen derivatives (the rest-to-world map is affine per mesh) and bumps by Mikkelsen's surface gradient, with the slopes taken as differences one pixel over (no derivative in branches: the material is read per draw from the bindless table). **Levels of detail:** the detail fades out by the pixel's footprint (≈ 0.15–0.6 of its period), the far model (§8, past 40 m) keeps plain materials in the kit colours, its jersey in the pattern's dominant colour (`ocw_core::kit::dominant_jersey_colour`). `OCW_GEAR=off` draws the old plain materials (comparisons).
  - **Bindless:** the gear material is bindless where the platform has it (index table at binding 100, the uniforms as one storage array at 101, textures in Bevy's bindless arrays; plain bindings 50–53 for WebGL 2): with a bind group per rider and part, 30 riders cost 48 ms per frame instead of 3 (every material its own draw in every pass); sharing the parts without a design (same colour) brought 12 ms, bindless the rest.
  - **Jersey designer** (character corner, W4 slots; `apps/desktop/src/workshop/designer.rs`): two more slots, *Jersey designer* (torso) and *Kit colours* (legs) – ↑ / ↓ a field, ← / → change it on a draft shown live on the rider and in the mirror, Confirm applies it as one `kit.design` line (menu, console and scripts share it), Back discards it. Jersey: the pattern, the first, second (*auto*: white on dark, black on light – the old accent) and an optional third colour, the race number 1–999, the pattern on the helmet; kit: shorts, socks, shoes, gloves (*standard*: the model's colour; the variants of a slot take it too: tights, aero socks, race shoes, overshoes), helmet colour, helmet and shoe finish. All free (who you are never costs hearts, Phase 4 Q4). The patterns (`ocw_core::kit`, Rust twin of the shader, rest-pose metres): the chest band 0.24–0.34 m, the sash at 0.62 rad from the right shoulder to the left hip (8.4 cm), vertical stripes of 4.5 cm (every third in the third colour), chevrons pointing down the chest (7 cm bands, the middle one in the third colour), a vertical gradient (through the third colour); the third colour also as 1.4 cm pinstripes along bands and sashes; the shorts get side panels in the jersey colours, the helmet a centre stripe (or the gradient front to back) on a shell in the jersey colour. **Race numbers:** a paper bib (20 × 15 cm) with 3 × 5 pixel digits on the jersey's back, the digits alone on both hips of the shorts (read from the side), anti-aliased in the shader. The W4 band and sash nodes stay in `rider.glb` but are never shown.
  - **Data and network:** `ocw_core::KitDesign` in `RiderLook` (`design`; `Finish` gloss / matte); `KitPattern` gained stripes (3), chevrons (4), gradient (5). The design travels like the character: in the full look of `Hello`, `BikeChange`, `RiderBuild` (`RiderLook` fields 12–21, all optional; absent = the plain kit), not in the snapshots (which carry the pattern); the session reports a changed design once (`RiderBuild` to the others), a joining client gets it with the other builds. Saved in `riders.toml` (`kit_second`, `kit_third`, `race_number`, `helmet_pattern`, `shorts_color`, `socks_color`, `shoes_color`, `gloves_color`, `helmet_finish`, `shoe_finish`; default values are not written; bad values dropped with a warning). Server bots wear designs (all patterns, second and third colours, numbers 1…, patterned and matte helmets). Anonymous riders of the browser app keep the plain default kit (the mirror stays locked).
  - **Helmet and shoe variants:** W4's generated variants stay (road, light, aero road and time-trial helmets, cap; road and race shoes, overshoes); T6 adds their free colours and the gloss / matte finish.
  - **Measured** (marge, AMD Radeon RX Vega, release build, `scripts/measure-fps.sh`, 1280 × 720, no vsync, 40 s with 12 s warm-up, two interleaved runs each; Hills test route, a group ride with 30 bots, the camera 10 m in front of the group): **3.17 / 3.18 ms → 3.26 / 3.54 ms per frame** (315 → 307 / 283 fps; before = the same build with `OCW_GEAR=off`, the plain materials of W4), well within §15.
  - **Tests:** material kinds per slot and node (the saddle among the components, the skinsuit among the jersey's parts, bare carbon frames), the surfaces carry the design (pattern, colours, number, finishes, clear coat, anisotropy) and share materials when equal, per node and LOD on a spawned avatar (near parts gear or standard, far parts plain, the far jersey's dominant colour, rest coordinates on the near meshes), pattern generation (deterministic, distinct layouts and shares, spot checks per pattern, shorts and helmet), digit glyphs and the bib / hip ink, the designer's fields (step and wrap, the `kit.design` line), the look's round trip through the wire (snapshots without the design, bad values validated) and `riders.toml`, others' view of a design (client test against the server: after the hello, after a change, for a newcomer), the session's report of a design change, the library manifest (the three gear textures pinned, CC0, credited; the licence check still rejects SA / NC / unknown). Screenshots (Wayland and X11): the carbon bike on the stand, groups in different designs from the front and behind, the rider close up, the designer; a video of the designer (scenes `t6-*`).
  - **Open:** sweat / dirt, embroidered logos and sponsor-free name prints, real anisotropic tangents for brushed (not spun) metal, a sheen lobe for lycra, numbers on bikes (frame plates), patterns on sleeves beyond the gradient.
- **T7 – Optional orthophoto corridor** near ridden roads (after Q4). **Done (v3.10; on by default in every package, decided by the user 2026-11-14):**
  - **Builder** (`ocw-worldgen::corridor`, run by `world-builder calibrate` after the palette, so the build scripts make it wherever they calibrate; `--corridor 0` leaves it out, `--corridor-pixel`, `--corridor-quality`): per tile the corridor weights (`ocw_world::ortho::corridor_weights`: every road strip of the package counts – in free ride every road is ridden –, ±50 m beside its edge as decided in 8B.3 / Q4, none in forests or on buildings, §4.7), then the 125 m chunks the corridor touches, resampled bilinearly from the orthophoto cells of T3 (best source first; Liechtenstein's Orthophoto 2025, SWISSIMAGE where it has no data) at 0.5 m, or at the source's own pixel size where that is coarser (the Black Forest's DOP20 and Tenerife's PNOA were fetched at 1 m for T3: 1 m). **Shadows and cars toned down:** relative to the local mean brightness (8 m blocks blurred over 24 m) darker pixels are lifted (ratio^0.55: a cast shadow at 40 % of its surroundings comes out at 60 %; our own trees and houses cast their shadows again) and take on the local ground colour in proportion to their darkness (sky-lit shadows are blue: lifted alone they stayed blue-green patches in the first Black Forest renders), brighter ones are pulled in with their hue kept (ratio^0.7: parked cars, glare). **Roofs masked:** the photos are not true orthophotos, roofs lean beyond their footprints; the footprints grown by 2 m are filled with the ground colour around them (first renders showed red patches beside the houses of Günterstal). Pixels outside the corridor are the local ground colour too; each chunk is a baseline JPEG (`image`, 4:2:2, quality 80; 70 saves 20 %). **Format choice:** JPEG in the package, BC3 made by the client: BC1/BC3/BC7 in the package would be 4–8 bits per pixel (Liechtenstein's corridor 110–220 MB), JPEG is 1.85 bits at q80 and the client compresses only the 3 × 3 tiles around the rider.
  - **Package cost** (marge, `t7/` packages built from the main checkout's ones; before → after): Liechtenstein 35.2 → 86.4 MB (+51.2 MB: 167 tiles with imagery, 5,654 chunks, 219 Mpx, 49.8 MB images at 1.82 bits per pixel, 1.4 MB weights; q70 about 20 % smaller), Black Forest south 287.9 → 559.5 MB (+271.6 MB: 2,377 tiles, 91,602 chunks, 777 Mpx at 1 m, 2.61 bits per pixel, 18 MB weights), Tenerife 269.7 → 483.4 MB (+213.7 MB: 1,866 tiles, 67,045 chunks, 616 Mpx at 1 m, 2.58 bits per pixel, 15 MB weights). The first cut with forests at 35 % and 2 m weight cells cost +74 MB, +552 MB and +273 MB (the weights alone 4 / 73 / 42 MB). Builder time of the corridor stage 11–15 s / 40–70 s / 29–42 s on marge (all cores, depending on the other builds), peak memory of the whole `calibrate` 2.5 / 4.2 / 3.1 GB. Within Q4 for Liechtenstein (30–80 MB estimated); the large regions nearly double. **Shipped packages** (rebuilt with the build scripts on marge, 2026-11-14, corridor on by default as the user decided; with `LITR` of Phase 9 S2 too): Liechtenstein 35.2 → 86.4 MB, Tenerife 269.7 → 483.4 MB, Black Forest south 287.9 → 559.6 MB, middle 236.8 → 457.5 MB (+205 MB images, 1,902 tiles), north 216.5 → 416.3 MB (+186 MB images, 1,640 tiles); the corridor stage takes 20 s, 30 s and 60–90 s per Black Forest package.
  - **Renderer:** §4.6 "Orthophoto corridor (T7)": per tile near the rider a 2,000² (1,000² at 1 m) BC3 texture with the weight in alpha, built in the background; the photo's colour with the materials' brightness detail, blurred up close, sharp from 15–120 m, fading out 650 → 950 m. Without the material library (no BC support) no corridor. Setting `display.orthophoto` on / off (default on), console `ortho`.
  - **Measured frame time** (marge, AMD Radeon RX Vega, release build, `scripts/measure-fps.sh`: headless weston with the GPU, 1280 × 720, no vsync, 50 s with 12 s warm-up, the rider standing; the same build and packages with `ortho off` and on, interleaved, three rounds – the machine was shared, the quiet rounds count): Vaduz centre 2.44 → 2.44 ms (round 1: 2.49 → 2.51), Günterstal on Freiburg – Schauinsland 3.24 → 3.25 ms, Las Cañadas (TF-21) 2.41 → 2.45 ms (round 1: 2.38 → 2.42); round 3 ran beside other builds (±0.6 ms noise either way). The cost is two texture samples per ground fragment within 950 m and the background decoding of nine tiles; within the noise of a shared machine. Software Vulkan (the X11 smoke runs) renders it too.
  - **Comparison renders** (scene harness, `ortho-*` scenes, before = the packages of the main checkout without `ORTC`, after = with it, Wayland and X11): Vaduz centre (parking lots, lawns, the town's grey and green instead of a uniform meadow), the Malbun road above Triesenberg in the chase camera, the chalets and parking lots of Malbun, Günterstal on Freiburg – Schauinsland (meadows, gardens, the stream), the TF-21 in Las Cañadas (the reddish-brown lava and pumice instead of a uniform grey-olive).
  - **Tests:** the section round trip, a skeleton without it, old readers and newer layouts skipping it; weight encoding; the corridor geometry (full up to 30 m beside the edge, fading to 50 m, symmetric, roads of the neighbour tile reaching across the edge, both tiles agreeing on it); forests and buildings lowering the weight; the builder on a synthetic orthophoto (only the chunks near the road, colours in place with rows along +z, the coarser source's pixel size, weights, the roof masked, the package round trip, no palette → no corridor); toning (shadows lifted and taking the local colour, highlights pulled with their hue); JPEG chunks; the renderer's blend weights by view distance and corridor weight, the tint (material detail kept up close, the sharp photo far off, nothing outside the corridor), the texture (chunks and weights in place, no weight without a chunk, mips in linear light, BC3 sizes), the console command and the setting.
  - **Open:** the Black Forest and Tenerife corridors are at 1 m (the T3 imagery); 0.5 m along the roads only would need a second fetch (DOP20 is 20 cm, PNOA 25 cm at the source) and about 2–3 × the bytes. The user decided (2026-11-14) to ship the corridor **in every world package, on by default**, also for the large regions (no separate download). Streamed packages (the browser app, Phase 10 B2) leave the `ORTC` chunk out of the base (`package::STREAM_SKIPPED_SECTIONS`, `stream::plan::skipped_section`): one chunk of 50–270 MB before the first tile is too much; fetching it per tile would need the section split into per-tile chunks in the manifest (later, with a browser corridor). The corridor is kept in memory compressed with the package (51 / 272 / 214 MB); reading chunks from disk on demand would save that. Weights only follow road strips, not the routes' importance; tunnels get a corridor too (harmless, the tube is cut out).
- **Unit tests:** licence validator (rejects NC/SA/ND/unknown), palette extraction on synthetic orthophoto tiles, roof-colour sampling (footprint erosion, nodata, `roof:colour` precedence), forest-type mapping, façade generator (window counts from levels, deterministic output), package section round-trips, KTX2 encoding of a sample, golden-image tests of the generated materials at low resolution.
- **Comparison tests:** the scene harness (§11.4) renders the same viewpoints before and after, next to a photo of the real place taken by us where available (never Google / Street View), for the build history and the art project.

**Result:** recognisable regions: the red roofs of Vaduz, the dark farmhouses and spruce forests of the Black Forest, real road surfaces, riders and bikes with real materials, all under open licences with full attribution.

#### 8F – Open questions (for the user)

- **Q1 – Attribution licences: decided (2026-11-12, "use the best data you can find"):** attribution licences (CC BY 4.0, dl-de/by-2.0, Swiss / Liechtenstein OGD BY, Copernicus) are accepted; credits on the credits screen and in each package's LICENSE-DATA.md.
- **Q2 – Commercial use: decided (2026-11-13):** **no commercial release** is planned. The licence rule 8A stays as it is (permissive licences only, no NC / SA / ND), so the code and the packages stay freely reusable; a legal review is not needed for now.
- **Q3 – AI-generated textures: decided (2026-11-13):** **no** AI-generated textures in shipped assets for now; procedural, CC0 and own photos (8C). To be revisited only by an explicit user decision.
- **Q4 – Package size: decided (2026-11-13):** the orthophoto corridor along the roads (8B.3, about 30–80 MB for Liechtenstein, more for larger regions) is acceptable; it may be part of the package or an optional download, whichever the implementation finds better.
- **Q5 – Season and time of day: decided (2026-11-13):** **seasonal variants**: summer, autumn colours, winter with snow, and spring, driven by the date (Phase 9 9C), with season-specific imagery and tints where open imagery of that season exists.
- **Q6 – Own photos: postponed (2026-11-13):** not possible for now; textures come from CC0 libraries and code, reference from openly licensed web photos (8D).
- **Q7 – Mapillary / street-level imagery: decided (2026-11-13):** Mapillary images are **CC BY-SA 4.0** by default (Mapillary terms of use; some content under other terms, commercial use under extra terms). Looking at them to learn what a place looks like is fine. Copying pixels into assets would make the asset CC BY-SA, which rule 8A excludes, and automated extraction of data from them is not done (Mapillary grants derived-data permission for OSM / Wikimedia contributions, not for us). So: **reference only, by eye**, like other web photos (8D).
- **Q8 – 3D building data: postponed (2026-11-14, dashboard: "save that for later").** No enquiries to swisstopo / ATG / LGL for now; the research below stays for later. **Liechtenstein / Switzerland:** *swissBUILDINGS3D 3.0 (Beta)* by swisstopo has roof shapes and overhangs (CityGML), licence **Opendata BY** (open use, source must be named, allowed by 8A), coverage listed as Switzerland and Liechtenstein; to confirm with swisstopo (geodata@swisstopo.ch) and the Liechtenstein office ATG (info.atg@llv.li, geodaten.llv.li), which holds the distribution rights in Liechtenstein. **Baden-Württemberg:** the LGL LoD2 models (from ALKIS footprints and laser heights, no roof superstructures, no textures) do **not** appear in the LGL Open GeoData list of 2024; an LGL price list exists, so they are probably not free. To ask the LGL customer service (contact via lgl-bw.de / the Open GeoData portal) whether LoD2 is or will be open data, and under which licence. **Tenerife:** to check the Spanish cadastre (INSPIRE buildings) and GRAFCAN. Until confirmed, buildings stay OSM-based with regional style presets (T4).
- **Q9 – Better terrain: decided (2026-11-12):** use the **best open data available** for every region: elevation (swissALTI3D / Liechtenstein DTM, LGL DGM1 for Baden-Württemberg, other official open DTMs elsewhere; SRTM / Copernicus DEM only as fallback), the freshest OSM, the best open orthophotos and textures (8A licences). The same rule applies to all other images and data. **Elevation and OSM implemented (v2.33, §4.3 "High-resolution terrain models"):** LGL DGM1 and swissALTI3D in every package, the Geofabrik extracts of the day; Austria (Vorarlberg, on the pilot's eastern edge) and France still fall back to SRTM – open Austrian DTMs (e.g. the 10 m "DGM Österreich", CC BY 4.0) are the next candidates. Orthophotos and textures remain open (8B / 8E).
- **Q10 – Priority: decided (2026-11-13):** **all** of them: ground and roads, houses, riders and bikes, worked on in parallel.
- **Q11 – Brands: decided (2026-11-13):** **no** real bike or kit brands or logos, also not as cosmetics.

---

### Phase 9 – Sky, Time, Seasons and Weather (planning, 2026-11-12)

**Goal:** the world shows the **real sky, season and weather of the place the rider is riding**, at the real moment: the sun, moon and stars in the right position and moving, daylight and night, the season of the date, and today's weather there. On top, two machines let riders choose. The **time machine** sets any date and time; the **weather machine** sets any weather.

#### 9A – Principles

- **Default = real.** "Now" means the real current instant (UTC) shown at the position of the rider **in the world**: the world's local time zone and daylight saving time, the sun's position over that spot, today's weather there. Example: a rider in Tokyo riding Liechtenstein at 14:00 Tokyo time sees 06:00 in Liechtenstein, i.e. sunrise. The time of the virtual place counts, never the time where the person physically is (decided, Q1).
- **One clock and one weather per session, owned by the game server** (server-authoritative, §6.1): `WorldClock` (instant, time speed, mode real/fixed) and `WeatherState` are server state, sent in `Welcome` and in snapshots (new protobuf fields, only added). Single-player runs the same code in the local server, so solo riders are their own "host".
- **Visual vs. physical.** Light, sky, season and most weather effects are visual. Wind, road wetness and air density can also change the **physics** (§5). They are server-side and switched per session or event ("weather affects physics": on / off; off by default in races for fairness, Q4).
- **Deterministic and testable.** All astronomy, time-zone, season and weather logic is pure Rust outside Bevy, builds for wasm (§3.5), and is unit-tested against reference values. The renderer only draws the state.
- **No network in the client for weather.** The game server or backend fetches real weather (9F). The client never sends the rider's position to a weather provider (privacy, §12).

#### 9B – Day and night: sun, moon, stars (circadian)

| Element | How | Accuracy target |
|---|---|---|
| Local time | IANA time-zone database (tzdb, public domain) via a Rust crate (e.g. `jiff` / `chrono-tz`). Each world package stores the time-zone id(s) of its area (package metadata, set by the world builder from the region) | DST changes exactly as in reality |
| Sun | Solar position (NOAA / Meeus algorithms; SPA for higher precision) from instant + latitude / longitude: azimuth, elevation, sunrise / sunset, civil / nautical / astronomical twilight | < 0.05° (well below what is visible) |
| Moon | Position and phase (Meeus, truncated ELP series), illuminated fraction and orientation of the terminator, earthshine; rises and sets, moves through the night | < 0.3° |
| Stars (not yet, Q7) | Bright star catalogue (≈ 9 000 stars to magnitude 6.5, real colours from B–V index) placed on the celestial sphere, rotated by local sidereal time (GMST + longitude) around the celestial pole; twinkling near the horizon | constellations exactly where they are |
| Planets (optional) | Venus, Mars, Jupiter, Saturn (VSOP87 truncated / Meeus) | < 0.5° |
| Milky Way | procedural band from galactic coordinates (own work), not a photo (photo maps are CC BY-SA, 9H) | – |
| Sky colour and light | physically based atmosphere (Bevy's atmosphere rendering if available in the Bevy version used, else own Preetham / Hillaire model): sky colour, sun colour and intensity, aerial perspective, sunrise / sunset glow, blue hour; auto-exposure | believable, not measured |
| Mountains block the sun | the sun's horizon from the DEM (§4.3): in the Malbun valley the sun sets behind the ridge earlier than the astronomical sunset; long shadows | per tile, computed by the world builder |
| Night | moonlight (intensity from phase and elevation), starlight, light pollution near towns (fewer stars), **street lamps** from OSM (`highway=street_lamp`, roads with `lit=yes`), lit windows in houses at night, **bike lights**: front and rear lamps on all riders (cosmetic, §2.5), automatic or by setting | – |
| HUD | world local time (24 h) and an optional sun / moon icon; the ISO timestamp of photos stays separate (art project, top right) | – |

The celestial bodies move continuously with the session clock: at time speed ×1 a sunset takes as long as in reality; with the time machine (9D) it can be a time-lapse.

#### 9C – Seasons

The date drives the look of the landscape (connects to Phase 8, Q5):

- **Vegetation:** deciduous trees (by forest type, Phase 8) leaf out in spring, are green in summer, colour in autumn and are bare in winter; conifers stay green. Meadows and fields follow a crop calendar (green → yellow → harvested / ploughed). Timing shifts with latitude and **altitude** (spring arrives later at 1 600 m in Malbun than at 450 m in Vaduz): a phenology model with simple degree-day rules, tunable per region.
- **Snow cover:** snow line by date and altitude (climatology) or, with real weather, from the actual snow (9F); snow on roofs, trees, meadows; roads cleared (they stay rideable).
- **Day length and sun height** follow automatically from 9B.
- **Imagery:** Phase 8 tints come from summer imagery; seasons modify them by rules. Optional per-season tints from Sentinel-2 composites (Q5 of Phase 8). **Basis done (v2.47, T3):** every calibrated package has spring / summer / autumn / winter tint maps from our own Sentinel-2 medians, blended by the world date (`ocw_world::look::season_weights`: linear between the composites' centre days, cyclic over the year); the near materials follow the classes' seasonal colours. Still open: snow by altitude and date near the rider (the winter composite whitens the far ground where it was snowy most of the winter), deciduous trees, crop calendar.

#### 9D – Time machine

The rider (in solo) or the host / event organiser (in a shared session) sets **any date and time**:

- **Modes:** *Real* (now, default), *Fixed* (a chosen instant, e.g. 21 June 2026 21:15), *Offset* (now + n hours), plus a **time speed** (paused, ×1, ×10, ×60, ×600: a whole day in a few minutes of riding, the time-lapse sunset).
- **Presets:** sunrise, golden hour, noon, sunset, blue hour, midnight, next full moon, summer / winter solstice.
- **Range:** years 1900–2100. The astronomy is valid there; seasons and snow follow the date. "Historic weather" (9F) combines naturally: ride the Malbun climb on a past date with the weather of that day.
- **UI:** menu page *Time machine* (Navi / Ride), a world clock in the HUD with the mode, console `time` (today a dev command, §3.2.1; it becomes a user command, with permissions in shared sessions).

#### 9E – Weather simulation and weather machine

**Weather state** (server, per session; optionally per area for large worlds): cloud cover and type (cumulus, stratus, cirrus, cumulonimbus), precipitation (none, drizzle, rain, thunderstorm, snow, hail) and intensity, visibility / fog (including valley fog and low cloud on the climb), wind speed, direction and gusts, temperature, humidity, pressure, **road wetness** (builds up in rain, dries afterwards depending on sun, wind and temperature; puddles in hollows), lightning.

**Rendering and sound:**

- **Visual:** procedural / volumetric clouds and their shadows on the terrain, rain and snow particles around the camera, wet roads (darker, glossy, reflections, spray from wheels), fog, lightning, snow accumulation.
- **Movement:** wind moves trees, grass and flags.
- **Sound** (game audio, §3.3): rain, wind gusts, thunder, wet tyre hiss.

**Physics (optional per session, 9A):**

- **Wind** enters the aero drag as the air speed relative to the rider (head-, tail- and crosswind along the road's bearing). It also changes drafting: with crosswind the shelter shifts sideways (echelons).
- **Wetness** raises rolling resistance slightly.
- **Temperature, pressure and altitude** give the air density in the drag term.
- **Trainer:** the gradient sent to the trainer in SIM mode stays the road's gradient. The extra wind load is added as resistance where the trainer supports it (FTMS wind speed / Wahoo wind), else only in the simulated speed.

**Extreme weather is included, uncapped** (decided, Q9): storms, hail, heavy snow, thunderstorms, dense fog, gale-force wind, in the simulation, the presets and real weather.

**Weather machine:** the rider / host chooses

- **Mode:** *Real* (9F), *Preset* (clear, fair-weather clouds, overcast, fog, drizzle, rain, thunderstorm, snowfall, windy, storm), or *Custom* (sliders: clouds, precipitation, wind speed / direction, temperature, fog, road wetness).
- **Transitions:** smooth over a chosen time, or immediate.
- **Mixed modes:** combinations such as "real weather but no rain".

UI: menu page *Weather machine*, HUD weather icon with temperature and a wind arrow relative to the rider's heading, console `weather` (a user command since S4, the server decides who may change it; 9I-S4).

**Without real data**, the weather simulation itself evolves the weather plausibly (a simple stochastic model: fronts passing, showers, diurnal cumulus, valley fog in the morning), seeded per session, so a long ride is not static.

#### 9F – Real weather data

The backend / game server fetches weather for the **area of the world package** (a coarse grid, e.g. one point per 5–10 km and per altitude band, not per rider). It caches it (refresh every 15–60 min, interpolated in time and space) and feeds the session's weather state. Results are cached per region, so many riders cause few requests. Without network, the session falls back to the simulation (9E).

| Provider | Coverage | Licence / terms (as found 2026-11-12) | Use |
|---|---|---|---|
| **MeteoSwiss Open Data** (Switzerland and Liechtenstein, gradually since 2025-05-22) | CH / LI, stations and models (ICON-CH) | **CC BY 4.0**, "Source: MeteoSwiss"; fair-use limits on the infrastructure | Liechtenstein world |
| **DWD Open Data** (opendata.dwd.de) | Germany, ICON-D2 model, stations, radar | open data, "Source: Deutscher Wetterdienst" (exact licence to be verified) | Black Forest world |
| **MET Norway** api.met.no Locationforecast 2.0 | global point forecasts | free; requires an identifying User-Agent and caching; licence reported as CC BY 4.0 / NLOD (to be verified) | global fallback (server-side only, generic product User-Agent, never personal data) |
| **Open-Meteo** | global, forecast + historical | data CC BY 4.0, but the **free API is non-commercial only** (< 10 000 calls/day); commercial use needs a subscription; can also be self-hosted (open source) | prototyping; production only self-hosted or with a subscription (Q6) |
| **Copernicus ERA5 / ERA5-Land** (reanalysis, hourly since 1940) | global, 9–31 km | Copernicus licence: free, attribution | **historic weather** for the time machine |
| NOAA GFS | global model, GRIB | public domain | heavy raw data; server-side alternative |

Attribution of the provider used appears on the credits screen and in the weather panel ("Weather: MeteoSwiss"), as for map data (§13).

#### 9G – Architecture and milestones (proposal)

- **New crates:**
  - `ocw-sky`: time zones, ephemeris for sun, moon, stars and planets, twilight, sidereal time, season and phenology. Pure, wasm.
  - `ocw-weather`: weather state, transitions, stochastic simulation, physics coefficients. Pure, wasm. Provider clients sit behind a `WeatherProvider` trait on the server side only (no network in tests: recorded fixtures).
- **Server:** session clock and weather in `GameServer`; host / event permissions for the machines; protocol fields added to `Welcome` / `Snapshot`.
- **Renderer (`ocw-render`):**
  - sky and atmosphere, sun / moon / star rendering;
  - light colour and shadows, night lighting (street lamps, windows, bike lights);
  - clouds, precipitation, wetness and fog;
  - seasonal vegetation parameters and snow.
- **World builder:** time-zone id per package, horizon / sun-blocking per tile from the DEM, street-lamp and `lit` data, phenology parameters per region.
- **Milestones:**
  - **S1** – time & sun: world clock, real local time with DST, sun position, day / night light, HUD clock;
  - **S2** – night sky: moon (phases), stars, planets, Milky Way, night lighting (**done without stars**, Q7; 9I-S2);
  - **S3** – time machine: modes, presets, time speed, menu, console, session permissions;
  - **S4** – weather simulation and rendering: clouds, rain / snow, fog, wet roads, wind in vegetation, sound (**done**, 9I-S4);
  - **S5** – weather machine: presets, custom, transitions, menu, console (**done**, 9I-S5);
  - **S6** – physics: wind / wetness / air density (server-side, per-session switch, trainer wind resistance);
  - **S7** – real weather: providers, caching, region grid, fallback, attribution; historic weather (ERA5);
  - **S8** – seasons: phenology, snow cover, crop calendar.
- **Unit tests:**
  - **Astronomy:** sunrise / sunset / solar noon for Vaduz and Freiburg on fixed dates against NOAA reference values (± 1 min); sun azimuth / elevation against SPA reference points; moon phases against known new / full moon dates (± 1 h); a star's altitude / azimuth against reference at a given instant; sidereal time.
  - **Time:** DST transitions in Europe/Vaduz and Europe/Berlin (the skipped and the repeated hour); time-machine arithmetic (speed, pause, offset); session clock in snapshots.
  - **Weather:** state transitions (deterministic with a seed); road wetness build-up and drying; provider parsing from recorded fixtures (no network); caching and fallback; attribution.
  - **Physics:** a headwind needs more power for the same speed, a tailwind less, a crosswind none; drafting offset in crosswind; air density from temperature / altitude; switch off = identical physics to today.
  - **Seasons and protocol:** phenology dates shifting with altitude; protocol round-trips.
- **Visual tests:** the scene harness (§11.4) renders fixed viewpoints at fixed instants (sunrise / noon / sunset / full-moon midnight / winter / storm) for comparison and for the art project's build history.

**Result:** riding Vaduz–Malbun at 20:30 in June shows the real sunset behind the ridge, the first stars and the street lamps of Malbun; the time machine makes it a winter night with full moon and snow; the weather machine adds a thunderstorm or today's real weather.

#### 9I – Implemented: S1 (time and sun) and S3 (time machine), 2026-11-13

- **Crate `ocw-sky`** (pure, no Bevy, builds for wasm32, in the wasm check): time zones, sun, clock, time machine and the light model; instants are Unix milliseconds passed in (no clock reads), so everything is deterministic and unit-tested.
  - *Time zones:* `jiff` (Unlicense OR MIT) with its **bundled tzdb** (`tzdb-bundle-always`): maintained by the author of `regex`, explicit handling of DST gaps and folds, the same zone data on Linux, Windows, macOS and wasm (no system zoneinfo). Chosen over `chrono-tz` (also embedded, but the older chrono API and no first-class gap / fold result). A skipped local time (spring) maps with the offset before the change (02:30 → 03:30 CEST), a repeated one (autumn) to the earlier instant.
  - *Sun:* the NOAA solar calculator's algorithm (Meeus): azimuth and elevation with NOAA refraction; sunrise / sunset (zenith 90.833°), solar noon, civil / nautical / astronomical twilight per local date (iterated to the event). Tests: Vaduz and Freiburg on solstices, an equinox-season date and both DST days within 1 minute of the NOAA-algorithm tables (astronomical twilight on the June solstice within 3 minutes: the sun only just reaches −18°), the NREL SPA reference point within 0.05°.
  - *Time machine range:* 1900-01-01 to 2100-12-31 (UTC); a time-lapse stops at the end.
- **Zone of a place (Q1):** world packages carry the IANA zone in the new optional section `TZID` (format unchanged otherwise, older readers skip it); `world-builder package` sets the zone of the region at the origin (`--time-zone` overrides): Liechtenstein `Europe/Vaduz`, the Black Forest `Europe/Berlin`, Tenerife `Atlantic/Canary` (v2.42). Packages without it and route files fall back to the zone of their origin (`ocw_sky::zone::zone_for`: a small table of regions, else the longitude's nautical zone `Etc/GMT±N`, no DST); rides without a geographic origin (test routes) are in Vaduz. The sun is computed at the rider's GPS position.
- **Session clock (§6.1):** `WorldClock` (mode real / fixed / offset, speed, paused) is owned by `GameServer` and runs on **session time** (the server's ticks: frame captures and `--fast` servers keep sun and simulation together; the real UTC instant is read once at the session's start). World instant = anchor + elapsed × rate; changing the speed or pausing in real / offset mode makes it fixed at the current instant; a step keeps the mode (real becomes offset). Sent in `Welcome.clock` and every `Snapshot.clock` (new fields only); clients predict it (`ClockPredictor`: blended corrections, monotonic while running, jumps taken at once).
- **Permissions (Q2, recommended option):** `ClockRequest` (real, fixed instant, offset, step, speed, pause) is answered with a `DevCommandResult`. The server decides with `ClockAccess`: *anyone* on the local servers (the single player is the host), *host* by default on `ocw-server` (the rider connected longest; the next one takes over when the host leaves), *nobody* for organizer-set sessions (`--time-control`). The console command `time` is a **user** command now; the client checks too, only for a quick message. A personal visual-only override is not implemented.
- **Light model** (`ocw_sky::light`, pure, tested): the sun's colour and strength from clear-sky transmittance exp(−τ·m) per channel (Kasten–Young air mass), diffuse skylight falling ten-fold every 2.4° below the horizon, an auto-exposure that adapts to 75 % of the darkening in stops (at most 8), eased like an eye (snapping on time-machine jumps), and a **night-brightness floor** for the displayed ambient light (`display.night_brightness`, default 10 % of daylight; 0 = realistic darkness; Q3 answered provisionally). Fog and plain-sky colours by elevation (warm at sunset, blue hour, dark night).
- **Renderer (`ocw-render::sky`):** the directional sun follows the solar position (shadows with it, off once the sun is down); **Bevy 0.20's atmosphere** (Hillaire 2020: sky, sun disc, aerial perspective; it reddens the sun light itself, so the light is given above the atmosphere) is the default, `display.sky simple` draws one plain sky colour for GPUs without compute shaders; ambient light, exposure and fog colour from the model. **Bike lights:** front (white) and rear (red) emissive lamps on all riders, plus a spot light from the own front lamp onto the road; automatic from dusk to dawn (sun below 1°), `display.bike_lights auto|on|off`. Stars and moon: S2.
- **HUD:** the world clock at the end of the main HUD line (`☼ 18:42`; `◐` twilight, `●` night; then `x600`, `paused`, `fixed` or `+2 h`), not top right (the capture timestamp is there, §3.2).
- **Menu:** *Navi → Time machine* (since v2.39 *World → Time*, §3.2.1 *World menu*; six rows and Back, fits 16:9): world time (← / → one hour), day (← / → one day), speed (paused … x600), preset (sunrise, golden hour = sun 5° high in the evening, noon = solar noon, sunset, blue hour = sun 5° below the horizon, midnight = the coming local midnight, summer / winter solstice = 21 June / 21 December at the current time of day, swapped south of the equator), mode (Enter: real time), who sets the time. Console: `time [real | HH:MM | YYYY-MM-DD [HH:MM] | ±Nh/m/d | offset ±Nh | speed N | pause | play | PRESET]`.
- **Persistence:** the night brightness, sky mode and bike-light mode are settings (`settings.toml`); the time machine itself is not saved: every ride starts at the real time (9A: default = real).
- **Visual tests:** scenes `sky-sunrise`, `sky-noon`, `sky-golden-hour`, `sky-sunset`, `sky-sunset-ridge`, `sky-blue-hour`, `sky-night-lights`, `menu-time-machine` and the video `sky-sunset-timelapse` (x600 across the sunset) in `scripts/scenes.toml`.
- **Not yet:** the sun's horizon from the DEM: the sun is not hidden behind ridges, and the terrain casts no shadows (it is not a shadow caster, for cost), so a ridge does not darken the valley at sunset; trees, buildings and riders cast long shadows that follow the sun. Moon, stars and street lamps: S2.

#### 9I-S2 – Implemented: S2 (night sky without stars), 2026-11-14

Decisions Q7 (no stars for now) and Q3 (minimum brightness, lights as a workshop part) applied; everything pure in `ocw-sky` (wasm) or `ocw-world`, drawn by `ocw-render`.

- **Astronomy (`ocw_sky::astro`):** ΔT (Espenak & Meeus polynomials), Greenwich / local sidereal time (Meeus 12), nutation and obliquity (Meeus 22, main terms), ecliptic → equatorial → horizontal, the equatorial → east / north / up rotation (turns the Milky Way). Tests: Meeus examples 12.a/b, 13.a, 22.a.
- **Moon (`ocw_sky::moon`):** position from the truncated ELP-2000/82 series of Meeus ch. 47 (60 + 60 periodic terms plus the additive terms), topocentric parallax and refraction, illuminated fraction, phase angle and the bright limb's position angle (ch. 48), apparent diameter, phase names, the instant of the next new / first-quarter / full / last-quarter moon (Newton steps on the elongation) and moon rise / set (upper limb with parallax and refraction, to the minute). **Accuracy (tests):** Meeus example 47.a within 0.001° in longitude / latitude and 1 km in distance, 48.a within 0.05° (phase angle) and 0.5° (limb); the 13 full and 12 new moons of 2026 within **1.3 minutes** of the published times; at the total solar eclipse of 2026-08-12 (Burgos, 18:28 UTC) the moon stands **0.005°** from the sun, at the lunar eclipse of 2026-03-03 opposite it within 1.5°; the bright limb points at the sun projected on the sky (within 1°).
- **Planets (`ocw_sky::planets`):** Mercury, Venus, Mars, Jupiter, Saturn from Keplerian J2000 elements with linear rates (E. M. Standish, JPL, *Approximate Positions of the Planets*, table 1: published constants, licence-free), Kepler's equation, one light-time step, precession to the date and nutation; magnitudes of the *Astronomical Almanac* (Meeus 41; Saturn with the rings' mean −0.6). **Accuracy (tests):** Venus on 1992-12-20 (Meeus 33.a) **0.004°**; Jupiter–Saturn at the great conjunction of 2020-12-21 0.111° apart (true 0.10°); the oppositions of Mars (2020-10-13) and Jupiter (2026-01-10) and the inferior conjunction of Venus (2022-01-09) within **0.05°** of 180° / 0°.
- **Milky Way (`ocw_sky::milky_way`):** the IAU galactic frame (the ICRS → galactic rotation; tested on the galactic centre, the north galactic pole, Deneb) and a procedural surface brightness, **own work, no photo or catalogue**: brighter and wider towards the centre (Sagittarius bulge), clouds in Cygnus, Scutum and Carina, the Great Rift, faint at the anticentre, mottled by deterministic value noise; visible only after nautical twilight, washed out by the moon (lit share × height) and by light pollution (gone at 0.6).
- **Night light (`ocw_sky::night`):** **moonlight** for the adapted eye: a high full moon lights the scene with ~3.5 % of the displayed noon sun (the eye's 8 stops of adaptation, `light`), bluish (Purkinje), scaled by Allen's phase law (a quarter moon a tenth) and the atmosphere's extinction, plus a little moonlit skylight on top of the night floor; shadows from about 15 % of full. The **limiting magnitude** of the eye (−4.5 by day, ~0 at the sun −4°, 6.0 at astronomical night, less by up to 1.5 mag in moonlight and 3 mag in a town) fades the planets in with the twilight (Venus first, Saturn later). The **minimum night brightness** (Q3) stays the floor of the displayed ambient light (`display.night_brightness`, default 10 %; the moon's light adds to it).
- **Moon disc (`ocw_sky::moon_disc`):** drawn on the CPU into a 256² texture (no shader): the Lommel–Seeliger law of a dusty surface (a full moon evenly bright to its edge), the terminator from the phase angle and the bright limb's direction, a hand-placed sketch of the maria and two bright craters (own work), earthshine on the dark part (strongest at a thin crescent); by day the dark part is transparent and the lit part pale.
- **Renderer (`ocw-render::night_sky`, `sky`):** moon (drawn at 2× its 0.5° as the eye perceives it, reddened and dimmed near the horizon, north up so the maria stand right), planets (soft point sprites, colour per planet, brightness by magnitude) and the Milky Way band (a mesh in equatorial coordinates turned by the local sidereal time) are unlit, additive and fog-free, centred on the camera beyond the far terrain (400 km; the infinite projection keeps them, the terrain hides them). At night the scene's **one** directional light (the regression of the two shadows) is the moon: with Bevy's atmosphere its sun disc is switched off there (`SunDisk` intensity 0) and the moonlit sky scatters faintly blue.
- **Street lamps (`ocw_world::lamps`, `ocw-render::street_lamps`):** the world builder stores OSM `lit` per road strip in the optional section **`LITR`** (one byte per strip, like `RSRF`; `yes` / `24/7` / `automatic` / `sunset-…` → lit, `no` / `disused` → dark; older readers skip it, packages without it read as unknown); untagged roads are lit **in towns** (residential, commercial or industrial land use) except tracks, paths and cycleways. The client places the lamps along the strips: every 26–36 m by road class at a fixed phase of the edge's length (continuous across tiles), right of the road beyond the sidewalk (staggered on both sides from 9 m width), poles 4.5–8 m with an arm over the road, feet on the terrain (at road height on bridges and walls). Poles and heads are built with the package tiles; the heads glow from dusk (the windows' night factor); a pool of 12 point lights (no shadows, 26 m reach) follows the camera to the nearest lamps – the warm light pools on the road. `highway=street_lamp` nodes are not used yet.
- **Light pollution (`ocw_world::lamps::light_pollution`):** from the package's buildings within 12 km (each tile's count weighted 1 / (1 + (d / 1.5 km)²), 3000 weighted buildings = e⁻¹ of a dark sky): Vaduz bright, Malbun a little, the Teide plateau dark; it draws an orange **sky glow** dome above the horizon (strongest at the horizon, e⁻¹ at 12°, only at night) and dims the Milky Way and the planets.
- **Lit windows (T4):** they light up by the sun's elevation (night factor 0 at +2° … 1 at −6°), checked in the night scenes.
- **Bike lights (Q3):** the lamps on a bike show only when a part of the slot `lights` (e.g. `lights.set`) is fitted (`RiderView.parts`, the workshop W2/W3); `display.bike_lights auto` switches them on from dusk to dawn and in tunnels. The stock bike has none; bots get lights with `dev.bot_bike N --parts lights.set`.
- **Time machine:** presets *Next full moon* (`full-moon`) and *Next new moon* (`new-moon`): the local midnight nearest to the next phase (the moon high in the south, or the darkest night of the month); the World → Time page cycles through them. `time` and `sky` show the moon's phase and place (and `sky` the planets above the horizon and the light pollution). Console `camera.look AZ EL` looks at a point of the sky from just behind the rider (`camera.look off`: the chase camera again).
- **Visual tests (§11.4):** scenes `sky-full-moon-canadas`, `sky-crescent-venus`, `sky-moonless-vaduz`, `sky-milky-way-teide`, `sky-full-moon-preset` (and `sky-night-lights` with fitted lights) in `scripts/scenes.toml`, Wayland and X11.
- **Not yet:** stars and constellations (Q7: later, with a catalogue whose licence fits), `highway=street_lamp` nodes, the pilot packages rebuilt with `LITR` (until then their lamps come from the town land use), light pollution from neighbouring packages (the home package only), twinkling, zodiacal light, eclipses drawn as such (the positions are right, the sky does not darken).

#### 9I-S4 – Implemented: S4 (weather simulation and rendering), 2026-11-14

Decisions Q2 (only the host / organiser changes the weather in shared sessions) and Q9 (extreme weather, no cap) applied. The weather is pure Rust in the new crate **`ocw-weather`** (as planned in 9G: weather state, transitions, simulation and later the physics coefficients of S6 are a subject of their own; the crate has no dependency, not even on `ocw-sky`, so the server's physics can use it without the astronomy), wasm-clean (in the wasm check), drawn by `ocw-render::weather`.

- **State (`ocw_weather::state`):** `WeatherState` – cloud cover (0–1) and type (cumulus, stratus, cirrus, cumulonimbus), cloud base above the ground, precipitation (none, drizzle, rain, snow, sleet, hail) with its intensity (mm/h water equivalent), fog visibility, mean wind, peak gusts and direction (meteorological, from), temperature, humidity, pressure, lightning rate (strikes / min). `lerp` blends two states (numbers linearly, visibility in log space so fog thickens evenly, the wind the short way round, the cloud type half way; precipitation starts with its intensity rising from zero and stops after fading out). `clamped` keeps the impossible out and the extremes in (no cap, Q9: gusts to 120 m/s, 400 mm/h). `condition` / `summary` / `hud` name it ("heavy rain 11 °C").
- **Presets (`ocw_weather::presets`):** clear, fair, cloudy, overcast, fog (80 m), mist, drizzle, light-rain, rain, heavy-rain, thunderstorm (30 mm/h, gusts 22 m/s, 6 strikes / min), hail, snow, heavy-snow (5 mm/h, −5 °C), sleet, windy, storm (gale, gusts 32 m/s). The weather machine's menu (S5) will offer them.
- **Session weather (`ocw_weather::session`):** owned by the `GameServer` like the world clock (§6.1), on session time: *fixed* (set at once or as an eased transition over a chosen time) or *simulated* – a seeded Markov chain over the presets (fronts and showers: fair → cloudy → overcast → rain → …, thunderstorms and storms now and then), every step varied (cover ±0.1, wind ×0.7–1.3, direction ±30°, intensity ×0.6–1.5), lasting 8–25 min and reached over 2–6 min, around the session's temperature (cold sessions get snow or sleet instead of rain). Default: clear and fixed (real weather is S7). `ocw-server --weather PRESET`.
- **Ground (`ocw_weather::ground`):** a water balance in millimetres, integrated every tick: rain (and melt water) fills a road film of 0.6 mm (fully wet-looking at 0.12 mm), the excess partly fills the hollows (puddles full at 2 mm, at most 3 mm, draining 0.5 mm/h); evaporation by sun (cloud cover), wind, warmth and dry air (a sunny, breezy afternoon dries a damp road within the hour, a humid grey day keeps it wet); snow settles below about +1 °C (fully covering at 3 mm, about 3 cm) and melts by degree-hours (more in sun and rain). A weather set at once starts *settled* (two hours of it from dry ground: a rain scene has wet roads, heavy snow a white landscape); console overrides `wet=`, `puddles=`, `snowcover=`.
- **Wind (`ocw_weather::wind`):** the gust envelope is smooth value noise on session time and the session's seed (swells of ~15 s with gusts of ~5 and ~2 s), so the server, every client (trees, rain, sound) and later the physics (S6) see the same gust at the same moment.
- **Lightning (`ocw_weather::lightning`):** one draw per second of session time at the weather's rate (Poisson, 1 − e^(−rate/60)); distance uniform over the area of the ring 250 m – 15 km, bearing, intensity and a shape seed from the session's seed. A strike lies at a distance and bearing *from each rider* (the storm surrounds the riders alike: every client sees and hears the same strike at the same delay). The flash flickers with 1–4 return strokes 50–110 ms apart, each decaying in ~30 ms, dimmer from afar; the thunder arrives at 343 m/s (three seconds per kilometre), with a crack within 1.5 km and a rumble that lasts longer and sounds lower from farther away, inaudible beyond 20 km. The jagged channel (24 segments and 2–4 branches) is deterministic.
- **What falls (`ocw_weather::particles`):** visibility by Koschmieder (V = 3.912 / σ) with the extinctions of fog and precipitation added, σ = a · Rᵇ per type (rain 0.25 R^0.63, drizzle 1.0 R^0.6, snow 3.0 R^0.8, sleet, hail; 1/km): light rain leaves kilometres, a 25 mm/h downpour about 2 km, 100 mm/h under 1 km, 5 mm/h of snow 360 m. Fall speeds: drizzle 1.5 m/s, rain 4–9 m/s growing with the intensity, snow 1 m/s, hail 12–25 m/s. The particle count grows with the square root of the intensity within a budget (40 000 quads by default; a cloudburst of 60 mm/h fills it); per type the box around the camera, width, streak length (the fall in a 1/45 s exposure), opacity, wind drift and the flakes' tumbling, and their fade at the lens, at the box's edge and in fog.
- **Light (`ocw_weather::light`):** factors on the clear-sky light of `ocw_sky::light`: the direct sun's share (cumulus heaps leave gaps, c^1.6; haze and precipitation dim it), the sky light relative to a clear sky's (a closed stratus deck 1.25: a brighter grey; a cumulonimbus 0.35; rain a little less), shadows only above 30 % direct light, the change in stops (overcast ≈ −1.7, a thunderstorm ≈ −4.3) of which the exposure follows 70 % (the eye adapts), how grey the fog and the plain sky turn, and how dark the clouds' undersides are.
- **Protocol (§6.2, fields only added):** `Weather` (the state) and `SessionWeather` (now, a transition's target and time left, the ground's factors, revision, immediate, mode, access, gust seed, session time, the strikes of the last 3 s) in **`Welcome.weather` (17)** and **every `Snapshot.weather` (7)**; **`ClientMessage.weather` (17) = `WeatherRequest`** (target, transition, settle the ground, simulate with a seed, ground overrides, a test strike), answered with a `DevCommandResult`. The server clamps every value.
- **Permissions (Q2):** `ServerConfig.weather_access` (`ClockAccess`): anyone on the local servers (the single player is the host), the host by default on `ocw-server` (`--weather-control host|anyone|nobody`), nobody in an organizer's session; the client checks too, only for a quick message.
- **Client (`ocw_weather::follow`, `ocw_client::weather`):** the follower approaches each snapshot's weather with a time constant of 0.5 s (hiding the 10 Hz steps) and takes a change made at once (a new revision) without easing; strikes are taken once each, converted into client time (session time ↔ client time from the snapshots, smoothed) and kept a minute for their thunder. `RideView.weather` carries the state, the ground, the wind now with its gust, the strikes and the HUD text; desktop and browser app share it.
- **Renderer (`ocw-render::weather`, `sky`):**
  - *Light:* overcast is flat light – the sun dimmed to its direct share, no hard shadows (shadow maps off below 30 %), a brighter grey sky light, grey fog and plain sky, darker in storms; under broken cumulus the sun at the rider goes in and out as the heaps drift by (the cloud noise evaluated on the CPU along the ray to the sun: the clouds' shadows sweep over the rider; they are not drawn on the terrain); the exposure follows part of the darkening.
  - *Clouds:* a sky dome (150 km, sorted after the moon and planets and before everything else that is transparent) whose fragment shader meets each view ray with the cloud layer – from below its base, or from above its top on high mountains (a sea of clouds below the Teide) – and draws 4–5 octaves of value noise shaped per type (crisp cumulus heaps, a soft stratus deck, cirrus streaks along the wind at 7 km and more, towering dark cumulonimbus), self-shadowed by a tap towards the sun, with a silver lining against it, lit from inside by lightning, fading into the haze towards the horizon; under a closed deck the horizon is cloud; inside the layer (low cloud on a climb) the camera is in fog. The layer drifts with the wind aloft (1.8 × the surface wind + 2 m/s) and floats over the rider's ground (smoothed over 30 s). No textures, no compute: WebGPU-compatible.
  - *Fog:* the weather's visibility (fog plus precipitation, and the cloud itself when inside it) replaces the distance fog when it is shorter (Bevy's exponential fog from the visibility); the sky behind dense fog takes the fog's colour.
  - *Precipitation:* one mesh of seeded quads (the budget), one draw: the vertex shader places every particle fixed in the world and wraps it into the box around the camera (riding through rain streams it past), stretches drops into streaks along their motion relative to the camera (slanted by wind and speed), keeps flakes and hail round and facing the camera, and collapses the quads beyond the count; snow tumbles, hail falls fast and bright; lit by the sky light, the sun and the flashes. `weather.fx particles N` sets the budget (0 – 100 000).
  - *Surfaces:* the road, terrain, building and tree materials get the weather uniform (wetness, puddles, snow cover, sway; quantized, changed only when it shows): wet asphalt darker (×0.55) and glossy (roughness 0.2), the verges less; when the puddles fill, water stands in the hollows of a noise relief, in the worn wheel ruts and at the edges as dark mirrors; wet ground and walls a little darker; snow on the ground that faces up (not on cliffs), in patches while it is thin, on roofs, on the upper sides of the tree crowns and on the verges – the roads stay cleared (9C).
  - *Wind in the trees:* the tree material's vertex shader leans the crowns downwind and sways them (by the wind with its gusts, from a breeze to a gale bending them; the phase runs with the place, so gusts move through a forest as a wave); package trees carry their height above the foot in uv.x. The shadow pass keeps them still (cheap). There is no grass geometry to move yet.
  - *Spray:* every rider's rear wheel throws up a plume of mist puffs on a wet road, its size by wetness² × speed (from 11 to 43 km/h) and standing water.
  - *Lightning:* the flash adds up to 1800 units of sky light (three daylight ambients) and lights the clouds towards it; a strike within 9 km shows its glowing channel from the cloud base to the ground while the flash lasts.
  - *HUD:* the main line shows the weather before the clock ("heavy rain 11 °C   ☼ 15:00").
- **Sound (§3.3, `apps/desktop/src/audio/weather.rs`, all synthesised):** rain as a broad wash of filtered noise growing with the square root of the intensity plus drops ticking on the helmet, drizzle as a soft hiss, hail as hard knocks on the shell over a hollow body, the wind's gusts as rolling low-passed noise (brighter in a gale), tyres hissing on a wet road, and thunder – a crack for near strikes, a rumble rolling with random swells – sent when it has travelled to the rider (in game time, so frame captures keep the delay); all on the effects bus (`audio.effects`), stereo-decorrelated; in a tunnel the rain and the wind are silent.
- **Console:** `weather` (a user command; the server decides): `weather` shows it, `weather list` the presets, `weather PRESET [in T]`, `weather FIELD=VALUE … [in T]` (clouds, type, base, rain / drizzle / snow / sleet / hail, precip=none, fog, wind, gust, dir, temp, humidity, pressure, lightning; the ground's wet, puddles, snowcover), `weather auto [SEED]` (the simulation), `weather strike [DIST [BEARING]]` (a test strike); `weather.fx clouds|particles|spray|bolts on|off|N` switches the effects (frame-time comparisons, weak GPUs). World → Weather shows it (§3.2.1).
- **Performance (§15):** measured on marge (AMD GPU, headless weston with the GPU renderer, `scripts/measure-fps.sh`, release build, 1280×720, no vsync, the Liechtenstein package on the Vaduz–Malbun road at km 1.6 with 3 bots, 21 June 15:00, three interleaved rounds of 35 s each; the machine was shared with other builds, so single samples spread widely): **clear 149–191 fps (5.2–6.7 ms), heavy rain 136–200 fps (5.0–7.3 ms, 40 000 particles budget), dense fog 144–149 fps (6.7–7.0 ms), thunderstorm 189–290 fps (3.4–5.3 ms), heavy snow 164–387 fps (2.6–6.1 ms)** – no weather is measurably slower than clear (the dome is one transparent draw, the particles one draw; overcast and storms switch the shadow maps off, which saves more than the effects cost), all far within the 60 fps budget of §15 (16.7 ms). Software Vulkan (lavapipe, the scene harness at 2560×1440) runs the weather scenes at about the clear scenes' 39 fps.
- **Tests (§11.4):** interpolation and transitions (log-space fog, the short way round, eased, starting where it is), presets valid and extreme (Q9), the session's ground and the settled ground, lightning at its rate and deterministic per seed, the simulation repeatable and plausible (snow in cold sessions), the follower (smooth, snapping on immediate changes), gusts (bounded, smooth, seeded), visibility and particle maths, the cloud light (overcast flat without shadows, storms dark), thunder timing and character, protocol round trips and clamping, the server's permissions (host only, organizer, solo) and the weather in welcome and snapshots, the client end to end against a stepped server (a strike once, in client time), the console parser, the menu rows, the renderer's uniforms and its CPU noise twin, the sounds (rain grows, hail knocks, near thunder cracks, all end). **Visual tests:** scenes `weather-light-rain-blackforest`, `weather-thunderstorm-tenerife`, `weather-fog-vaduz`, `weather-heavy-snow-feldberg`, `weather-overcast-flat-light` and the videos with sound `weather-rain-shower-video` (a passing shower), `weather-thunderstorm-video` (two strikes and their thunder), `weather-gale-video` (trees in a gale) in `scripts/scenes.toml`, Wayland and X11.
- **Not yet:** cloud shadows drawn on the terrain (only the light at the rider changes), rain splashes on the ground, reflections in wet roads and puddles (no screen-space reflections: they show the sky light and the sun's highlight), snow on the far terrain beyond the near tiles and on the test routes' plain terrain, grass and flags in the wind, sway in the shadows, valley fog in hollows (the fog is the same everywhere), weather per area in large worlds; real weather (S7), weather physics (S6). The weather machine's menu: done with S5 (9I-S5).

#### 9I-S5 – Implemented: S5 (the weather machine), 2026-11-14

Decisions Q2 (only the host / organiser changes the weather in shared sessions), Q4 (in events the organiser decides) and Q9 (extremes included) applied. The logic is pure in `ocw-weather` (wasm) and `ocw-client` (shared by the desktop and the browser app); the desktop draws the menu.

- **Menu *World → Weather*** (`apps/desktop/src/weather.rs`, pages `Weather`, `WeatherPresets`, `WeatherCustom`, `WeatherAir`; title *Weather machine*): under the title the weather now (condition, temperature, wind, road) and who sets it ("set by you / the host / the organizer / the event's organizer"); six rows and Back (ten-foot rule):
  - *Mode* – fixed or simulated; ← / → / Enter switch (`weather auto SEED` with the machine's seed, `weather fixed`: the simulation stops, a running transition finishes). During a transition the value is "-> thunderstorm in 40 s".
  - *Real weather* – dimmed, "coming (Phase 9 S7)"; `weather real` answers the same.
  - *Presets* – the preset the weather is (or is heading to), else "custom"; opens the **preset grid**.
  - *Custom* – the weather it is heading to; opens the **custom editor**.
  - *Transition* – instant, 1 min (default), 5 min, 15 min: the time the menu's changes take (`weather.transition`; any duration from the console).
  - *Random weather* – ← / → change the seed (1–99 999), Enter applies `weather random SEED in T`.
- **Preset grid:** the 17 presets in five columns (17 cells and Back in four rows; the panel 150 vh wide so every label fits one line at 16:9), each with an **icon** and its name; the arrows move in two directions (wrapping within the row order and between the top and bottom rows), Back is the last cell (the touch user's way back besides the right click / Esc / B); the weather now is framed; under the title the focused preset's description and "Enter: in 1 min" (or who sets it). Enter applies `weather PRESET in T` (a preset set at once also settles its ground).
- **Icons (`ocw_weather::icon`):** what an icon shows follows from any weather state (`Icon::of`: the sun alone, beside a small cloud or behind a big one; a white, grey or dark cloud; drizzle, rain in three strengths, snow in two, sleet, hail; a bolt; mist or dense fog bars; wind streaks), so every preset has its own and a custom weather gets one too; `icon::rgba` draws it as anti-aliased signed-distance shapes into a 128² RGBA image (own work, no font glyphs or image assets; the browser's B6 menu can use the same).
- **Custom editor** (`ocw_weather::machine::Field`): page *Custom weather: sky* – cloud cover (tenths), cloud type (cumulus, stratus, cirrus, cumulonimbus; a cumulonimbus brings lightning, 3 strikes / min, another type takes it away), precipitation (none, drizzle, rain, snow, sleet, hail, each turned on with a typical amount: 0.3 / 4 / 1.5 / 2 / 25 mm/h, from a sky at least 80 % covered), intensity (a ladder 0.1 … 400 mm/h), fog (10 m … none), and *Custom: wind, temperature* (row *Wind, temperature*) – wind (0–60 m/s, the gusts keep their margin), gusts (at least the wind, up to 90 m/s), direction (16 compass points), temperature (−40 … +50 °C). Each ← / → sends `weather FIELD=VALUE in T` at once (from the weather a transition is heading to); the editor's steps never leave these ranges and `machine::validate` keeps the state consistent (gusts ≥ wind, no intensity without precipitation, lightning only from a cumulonimbus); **the server clamps again** (`WeatherState::clamped` on every request: the console's extremes up to the state's limits stay possible, Q9).
- **Input (§3.2.2):** keyboard, gamepad (D-pad, A, B, Start), mouse (hover, click, right click = Back) and touch (taps; the grid's Back cell): value rows draw their **‹ › arrows as their own pointer targets** (`MenuPointerEvent::Step`, `Action::Step`), so mouse and touch reach every value without ← / →, on every menu page. Rows that do nothing are dimmed (`Activate::Disabled`; `MenuView.dimmed`). Tests drive every page of the weather machine with each device alone.
- **Permissions (Q2, Q4):** the server decides as since S4 (`ServerConfig.weather_access`: anyone on the local servers, the host by default, `--weather-control anyone|nobody`), and **an event's weather locks it**: `GameServer::weather_access` answers *nobody* from the start until the event is finished or cancelled, for everyone including the single player who started it; requests are refused with "the organizer set the weather of this event (…)". The client shows who sets it and dims the rows; the console says the same.
- **Event weather:** `event race|group|tt … --weather PRESET [FIELD=VALUE…] | custom FIELD=VALUE… | random SEED | auto SEED` (`machine::parse_event_weather`; `auto` without a seed is seed 1, so every run of an event has the same fronts). Protocol (fields only added): **`EventStart.weather` (8)** and **`EventState.weather` (15)** = `EventWeather` (name, state or simulate + seed). The server puts it in force at the start (set at once with its ground, or the simulation) and names it in its answer and the event's state; the client shows "Weather: thunderstorm (set by the organizer)" on the start line and under the results. Without `--weather` the session's weather and rule stay. After the event the weather stays as it is (the rider may change it again).
- **Console:** `weather` extended – `weather PRESET FIELD=VALUE… [in T]` (a preset with fields changed), `weather random [SEED] [in T]` (`machine::random`: a preset varied like a simulation step – cover ±0.1, wind ×0.7–1.3, direction ±30°, intensity ×0.6–1.5, temperature ±3 °C, snow kept frozen and rain liquid; without a seed one from the clock), `weather fixed`, `weather real` (→ S7); new `weather.list` (presets, custom fields, transitions), `weather.status` (now, target and time left, mode and seed – `SessionWeather.sim_seed` (14) –, clouds, visibility, humidity, pressure, lightning, ground, who sets it and an event's lock; on the desktop also the machine's transition and seed), `weather.transition [T]`, `weather.seed [N]`. The command logic is one function for both apps (`ocw_client::weather::run`).
- **Browser app:** `weather` (with the S5 forms), `weather.list` and `weather.status` in its console; the menu follows with the B6 menus (Phase 10).
- **Tests (§11.4):** the editor's steps within the ranges from every preset (120 steps each way), the ends, validation and clamping (also the server's: 10⁹ m/s, NaN, −50 m fog, 10⁶ mm/h, a 10¹² s transition), editor steps as console lines that parse back to the same value, random weather repeatable and varied, event weather parsing and round trips, icons distinct per preset and showing sun / drops / bolt where they belong, transitions over session time (clear → thunderstorm in 60 s: growing cloud, the time left falling, reached at 60 s), permissions (solo, host, anyone, nobody, host leaving), an event's weather fixed until cancelled or finished, the client's console end to end against a stepped server, the rows and the status of every page, the menu's grid navigation, the ‹ › arrows, and the whole machine with the keyboard, the gamepad, the mouse and touch alone in the desktop app with a stepped local server; every page fits a 16:9 window with its text. **Visual tests:** scenes `menu-world-weather`, `menu-weather-presets`, `menu-weather-custom`, `event-weather-start` and the video `weather-clear-to-thunderstorm-video` (clear to a thunderstorm in one minute) in `scripts/scenes.toml`, Wayland and X11.
- **Not yet:** real weather and "mixed modes" such as real weather without rain (S7), road wetness as a slider (the console's `wet=` / `puddles=` / `snowcover=` remain), cloud base, humidity and pressure in the menu (console only), saving the machine's transition and seed (every ride starts with 1 min and seed 1, like the time machine), the HUD's weather icon with a wind arrow (9E), the browser app's menu (B6).

#### 9H – Open questions (for the user)

- **Q1 – Whose "now"? Decided (2026-11-12):** the time of the place **in the virtual world** where the rider is: its local time zone and DST, its sun, moon and stars. The real location of the person at home plays no role. A "same time as at home" preset is not planned.
- **Q2 – Shared sessions: decided (2026-11-14, dashboard):** in group rides and events **only the host / organiser** controls the time and weather machines (the recommendation); no personal visual-only override per rider.
- **Q3 – Night riding: decided (2026-11-14, dashboard):** night keeps a **minimum brightness** (no full darkness – training comfort comes first). **Bike lights are not automatic:** they are a part the rider buys with hearts and fits in the workshop (§14 Phase 4, 4B slot `lights`, catalogue entry `lights.set`, 80 ♥); without lights fitted the bike has none, and the minimum brightness still lets the rider see the road. Until the workshop exists, the lights the renderer draws today (§3.3 / Phase 9 S1) stay on as a placeholder. **Done with S2 (2026-11-14):** the lamps follow the fitted `lights` part (9I-S2).
- **Q4 – Weather and physics: decided (2026-11-12):** yes, wind, wet roads and air density change the physics **realistically**. On in solo and free ride; in races and events the **organiser** decides (off unless enabled).
- **Q5 – Trainer wind resistance: decided (2026-11-14, dashboard):** the wind load is **sent to trainers that support it** (FTMS indoor bike simulation wind speed, Wahoo wind); trainers without wind support keep gradient-only resistance with the wind folded into the simulated grade.
- **Q6 – Weather provider: decided (2026-11-14, dashboard):** **MeteoSwiss + DWD** (regional, open data) with **MET Norway** as the global fallback, **plus Open-Meteo** as a further provider. The provider is **selectable in the menu** (World → Weather → Data source: *Automatic* (regional first, then MET Norway) / MeteoSwiss / DWD / MET Norway / Open-Meteo; a provider that does not cover the place falls back to Automatic with a note). Open-Meteo's free API is non-commercial, which fits the decision of no commercial release (Phase 8 Q2); its attribution (CC BY 4.0) is shown like the others. In shared sessions the host / organiser's choice applies (Q2).
- **Q7 – Star catalogue licence: decided (2026-11-14): no stars for now.** S2 draws the night sky without stars (moon, planets, procedural Milky Way, 9I-S2); stars and constellations come later, once a catalogue with a fitting licence is chosen (the research and the proposal below stay for that). Background: the Yale Bright Star Catalogue's licence is not clearly documented (Debian ships it in *non-free*), and the HYG database is CC BY-SA 4.0. Acceptable alternatives: ESA Hipparcos / Gaia-derived lists (terms to check), or a catalogue we derive ourselves from public-domain data. Decide after checking; the code doesn't depend on the choice.
  - **Research (2026-11-14), from the publishers' own pages and the VizieR ReadMe files:**

    | Source | Licence (primary source) | 8A | Coverage / fields |
    |---|---|---|---|
    | Hipparcos, Tycho (ESA 1997, VizieR I/239) | **CC BY-NC 3.0 IGO**, "Credit: ESA" (ESA Hipparcos catalogues page; ReadMe "License: CC-BY-NC-3.0 IGO") | no (NC) | 118 218 stars, V, B–V, proper motion |
    | Hipparcos new reduction (van Leeuwen 2007, I/311) | no licence line; ESA mission data, so treat as ESA terms | no (assume NC) | astrometry only, no B–V |
    | Tycho-2 (Høg et al. 2000, I/259) | **CC BY-NC 3.0 IGO** (ReadMe "License:" line) | no (NC) | 2.5 M stars, BT/VT |
    | Gaia DR3 (ESA, I/355) | **CC BY-NC 3.0 IGO** (ESA Gaia "License" page; it is *not* CC BY-SA 3.0 IGO or CC BY 4.0); commercial use only via the ESA archive T&C | no (NC) | 1.8 G sources; saturates on the brightest stars (G < 3), so unsuitable alone anyway |
    | Yale Bright Star Catalogue 5th ed. (Hoffleit & Warren 1991, V/50, HEASARC `bsc5p`) | **no licence**; HEASARC: provided "solely for the purposes of scientific research", it does not own the copyright, Yale University Observatory and / or the authors may still hold it; not a US-government work (Yale); Debian: *non-free* | unclear – avoid | 9 110 objects (9 096 stars) to V 6.5, J2000, V, B–V, proper motion, HR / Bayer / Flamsteed |
    | HYG v4.x / AT-HYG v3.x (astronexus, Codeberg) | **CC BY-SA 4.0** (LICENSE files; older versions CC BY-SA 2.5) | no (SA) | 119 614 / 2.5 M stars |
    | Stellarium star catalogues and sky cultures | stars from Hipparcos / Tycho-2 / Gaia (NC chain); sky cultures `modern`, `modern_iau`, `modern_rey` CC BY-SA 4.0, `modern_st` CC BY-SA 2.0, `modern_sternenkarten` CC BY-ND 4.0, `modern_hlad` GPL-2.0 | no | – |
    | Celestia (CelestiaContent) | `stars.dat` CC BY-NC 3.0 IGO (© ESA), `starnames.dat` GPL-2.0-or-later | no | – |
    | d3-celestial (Olaf Frohn) | repository **BSD 3-Clause**; stars from XHIP (Hipparcos-based, so the NC chain), constellation lines own work after the IAU constellation charts | lines: yes; stars: no | 88 constellations (lines, names, boundaries) |
    | SKY2000 Master Catalog v5 (Myers et al., NASA GSFC Flight Dynamics Division 2006, VizieR V/145) | **no licence or restriction stated**; NASA product (US-government work, 17 U.S.C. §105, so public domain in the US – but some authors were contractors and NASA states no explicit PD notice) | probably yes, **to confirm** | 299 460 stars to V ≈ 8, J2000, proper motion, V, **B–V**, HD / SAO ids |
    | IAU WGSN star names (IAU-CSN, exopla.net) | names are scientific standards, "free to use … not subject to copyright"; the WGSN texts are CC BY (some CC0) | yes | ≈ 650 proper names with HIP / HR ids |
    | CDS / VizieR | no blanket licence: free access "essentially for scientific use"; the catalogue's own ReadMe licence applies; requested acknowledgement "This research has made use of the VizieR catalogue access tool, CDS, Strasbourg, France (DOI 10.26093/cds/vizier)" | depends on the catalogue | – |

    Note: positions and magnitudes are facts, and the EU database right of Hipparcos (1997) has expired, but rule 8A follows the licence the publisher states, so the NC catalogues stay out.

  - **Proposal (not decided):** (a) **stars: SKY2000 v5**, filtered by the world builder / a build script to V ≤ 6.5 (≈ 9 000 stars; RA / Dec J2000, proper motion, V, B–V), after a short confirmation request to NASA GSFC / HEASARC that it is a public-domain US-government work; fallback: our own list from **Wikidata** (CC0; RA / Dec, apparent magnitude, B–V colour index; coverage not yet measured). (b) **Names: IAU WGSN** list, plus Bayer / Flamsteed letters from the catalogue's ids. (c) **Constellation lines: d3-celestial** `constellations.lines.json` (BSD 3-Clause: keep the copyright notice and licence text in the asset manifest), snapped to our star positions. Attribution lines: *"Star data: SKY2000 Master Catalog, Version 5 (Myers et al., NASA Goddard Space Flight Center, 2006), via VizieR V/145, CDS, Strasbourg, France"* · *"Star names: IAU Working Group on Star Names (WGSN)"* · *"Constellation lines: d3-celestial, © 2015 Olaf Frohn, BSD 3-Clause, after the IAU constellation charts"*. **Avoid:** Hipparcos, Tycho, Tycho-2, Gaia (NC), HYG / AT-HYG and Stellarium sky cultures (SA / ND / GPL), Celestia data (NC / GPL), d3-celestial's star files (Hipparcos-derived), BSC5 (unclear copyright).
- **Q8 – Historic weather: decided (2026-11-14, dashboard: keep it only if the data fits our licence) – kept, the data fits.** ERA5 (ECMWF / Copernicus C3S reanalysis, hourly from 1940) is published under **CC BY 4.0** for most datasets (e.g. ERA5 hourly single levels, ERA5-Land) and otherwise under the *Licence to use Copernicus products* – both are allowed by rule 8A (attribution: "Contains modified Copernicus Climate Change Service information [year]"; the exact licence is checked per dataset on its CDS page before use). Sources: the Copernicus Climate Data Store (free account needed for downloads, on our infrastructure) or Open-Meteo's historical API (ERA5-based, CC BY 4.0 data; its free API is non-commercial, which fits the decision of no commercial release); for the regions, MeteoSwiss and DWD historic station data (open data) refine it. If a dataset turns out to have an incompatible licence, that source is dropped, not the feature.
- **Q9 – Extreme weather: decided (2026-11-12):** realistic, **no cap**. Storms, hail, heavy snow, thunderstorms with lightning, dense fog and gale-force wind are all part of the simulation, the weather machine and real weather. The only limit is the physics switch (Q4).
- **Q10 – Priority: decided (2026-11-14):** after S1 and S3 (done), the order is **S2 → S4 → S5 → S7 → S8 → S6** (S2 done, 9I-S2; S4 done, 9I-S4; S5 done, 9I-S5).

---

### Phase 10 – Browser App: riding in the browser (planning, 2026-11-13)

**Goal:** ride Open Cycling World in a web browser, with no installation: the same worlds, physics, multiplayer, voice and riders as the desktop app. It complements the **web companion** (Phase 5, account / history / routes, no riding); it does not replace the desktop app. §3.5 has kept this possible from Phase 1: the core crates build for wasm, devices sit behind `DeviceBackend`, the protocol is transport-independent and packages can be streamed.

#### 10A – Browser platform facts (as found 2026-11-13; re-check before starting)

| Capability | Needed for | Support |
|---|---|---|
| **WebGPU** | 3D rendering (Bevy / wgpu) | Chrome / Edge since 113 (desktop), Chrome Android since 121 (Android 12+); **Safari 26** (macOS, iOS, iPadOS 26; on iOS all browsers use WebKit); Firefox 141 on Windows, 145 on Apple-silicon macOS, Linux / Android in progress. Fallback: WebGL2 (wgpu supports it, fewer features: no compute, weaker shadows) |
| **WebTransport** (HTTP/3, QUIC datagrams + streams) | the game connection (§6.2) | Baseline in 2026: Chrome / Edge since 2022, Firefox since 2023, Safari 26.4 |
| **Web Bluetooth** | trainer, power meter, heart rate (FTMS etc.) | **Chromium only** (Chrome / Edge / Opera on Windows, macOS, Linux, ChromeOS, Android). **Not** Safari / iOS, not Firefox (only experimental extensions) |
| WebRTC | voice (LiveKit `livekit-client`, §7 platform plan) | all browsers |
| Web Audio, AudioWorklet | game audio (§3.3) | all browsers |
| Gamepad API | gamepads (§3.2.1) | all browsers |
| OPFS / IndexedDB, Storage API | world package cache (hundreds of MB) | all browsers; quota per origin, ask for persistent storage |
| Canvas capture, MediaRecorder, WebCodecs | screenshots and videos (art project rules: ISO timestamp, JPEG q90, audio) | all current browsers (WebCodecs: Safari 26 for video) |
| Wasm threads (SharedArrayBuffer) | parallel systems, decoding | needs cross-origin isolation (COOP / COEP headers) |
| Wasm memory | the loaded world | 4 GB address space (wasm32); practical limit lower, so streaming is required |

**Consequence:** full riding with a trainer works in Chromium browsers on Windows, macOS, Linux, ChromeOS and Android. In Safari / iOS / Firefox the 3D world, multiplayer and voice work, but **no direct Bluetooth**: riding there needs one of the bridges in 10D or virtual power.

#### 10B – Architecture

- **New app `apps/web`:** the same Bevy client as the desktop, built for `wasm32-unknown-unknown` with the WebGPU backend (WebGL2 fallback), via `wasm-bindgen` / `trunk` (or `wasm-pack`) and served as static files. Platform differences sit behind the existing traits. There is **one renderer**: `ocw-render` stays shared. Desktop-only parts (btleplug, cpal native, ffmpeg, file dialogs, the kiosk) are cfg-gated and replaced by web backends.
- **Client core:** `ClientCore`, prediction, `SnapshotBuffer`, workouts and the console are already Bevy-free and wasm-safe. The local server (single player) runs **in the browser too**, as the same `GameServer` driven by the frame loop or a Web Worker instead of a thread.
- **Game connection:** a **WebTransport endpoint** on the game server next to native QUIC. The protobuf messages are identical: snapshots go as datagrams, reliable messages as streams. On the server: `web-transport-quinn` on our quinn endpoint (chosen in B3 over `wtransport` and `h3-webtransport`, §6.2). TLS needs a real certificate (no self-signed pinning in browsers), or `serverCertificateHashes` for development (≤ 14-day ECDSA certificates, rotated).
- **World data:** packages are **streamed by tile over HTTP** (range requests on the package file, or per-tile chunks: §3.5 / §4.7) from our CDN. They are verified (signatures, chunk hashes, §4.7) and cached in OPFS. Only nearby tiles are in memory, which the desktop needs anyway for larger worlds (Black Forest measurements, spec 2.27).
- **Audio:** game audio via cpal's Web Audio backend or our mixer in an AudioWorklet. Voice via `livekit-client` (JS) behind the `VoiceBackend` trait (§7).
- **Devices:** `WebBluetoothBackend` implements `DeviceBackend` on top of the pure `ocw-devices` protocol state machine (`gatt`, `ble::protocol`). Only the `GattLink` is new (navigator.bluetooth via `web-sys`). The user grants device access per page (browser chooser).
- **Captures:** screenshots from the canvas as JPEG q90 with EXIF date and time written by our own encoder step. Videos via MediaRecorder / WebCodecs with game audio mixed in. The ISO timestamp is in the HUD top right (art project §2.1).
- **Settings and rides:** settings in browser storage, with account sync via the backend (Phase 5). FIT export as a download; Strava upload through the backend (§2.8).
- **Input:** the input model of §3.2.2 as on the desktop; touch is the main input on phones and tablets (the touch overlay in a ride, touch-mode hit targets everywhere), pointer events via `bevy_picking` as on the desktop.
- **Hosting:** static app + package CDN + game server endpoints, behind the same domain as the web companion. Single sign-on with the account (OIDC, §9). COOP / COEP headers for wasm threads. A strict CSP.

#### 10C – Performance budget

- **Download:** initial app (wasm + assets) ≤ 30 MB compressed, cached; world tiles on demand. First ride within 30 s on a 50 Mbit/s line.
- **Frame rate:** 60 fps at 1080p on a mid-range laptop GPU in Chrome; 30 fps minimum on WebGL2 / phones (lower quality preset).
- **Memory:** ≤ 1.5 GB tab memory with a streamed world.

#### 10D – Riding without Web Bluetooth (Safari / iOS / Firefox)

1. **Virtual power:** keyboard / gamepad / on-screen power (as in the desktop's virtual trainer). Always available, also for spectators and tours.
2. **Phone or desktop as a sensor bridge:** the native app (desktop, or a future mobile app) connects to the trainer over BLE and relays the sensor samples through the backend to the browser session (both logged in to the same account). The server already takes timestamped `SensorSample`s (§6.1), so the bridge is a thin relay. Trainer control (ERG / SIM) goes back the same way.
3. **Third-party iOS browsers with Bluetooth** (e.g. Bluefy): possible, but not supported officially.

#### 10E – Milestones (proposal)

- **B1 – Wasm build:** `apps/web` compiles. Bevy runs on WebGPU with a test route and the local server in the browser; virtual power. CI builds it.
- **B2 – Worlds over HTTP:** tile streaming, verification, OPFS cache, loading screen, memory budget.
- **B3 – Multiplayer:** WebTransport endpoint on the server, browser client joins the same sessions as desktop riders.
- **B4 – Devices:** `WebBluetoothBackend` (FTMS, Cycling Power, CSC, Heart Rate, Wahoo), ERG / SIM control.
- **B5 – Audio and voice:** Web Audio game audio, LiveKit voice, mute hotkeys.
- **B6 – UI parity:** menus, settings, HUD, console permissions, gamepad, captures with timestamps, FIT download.
- **B7 – Sensor bridge** (10D.2) and WebGL2 fallback quality presets.
- **B8 – Hosting and release:** domain, CDN, certificates, CSP / COOP / COEP, account sign-on, browser test matrix.

**Unit tests:**

- WebTransport framing and server endpoint loopback tests (native, like the QUIC tests);
- tile range-request planner and cache eviction;
- package verification on streamed chunks;
- the Web Bluetooth `GattLink` against the existing protocol mocks;
- bridge relay ordering and timestamps;
- EXIF / JPEG writer;
- MediaRecorder / audio mix glue kept thin, with pure logic tested natively.

**Browser tests:** headless Chromium (`wasm-bindgen-test`, Playwright) for start-up, a ride on the test route and a screenshot. A smoke matrix of Chrome / Edge / Firefox / Safari on the build machines (the Mac for Safari, Windows for Edge).

#### 10F – Open questions (for the user)

- **Q1 – Target browsers: decided (2026-11-14, dashboard: "that's fine"):** Chromium-based browsers (desktop + Android) for riding with a trainer (Web Bluetooth); Safari / iOS / Firefox ride with virtual power and the sensor bridge.
- **Q2 – Sensor bridge: decided (2026-11-14, dashboard):** **yes, build it** (10D.2) – the way to ride with a trainer on iPad / iPhone (and in browsers without Web Bluetooth) before a native iOS app exists.
- **Q3 – Priority vs. other phases: decided (2026-11-14, dashboard):** **start now** – the browser app is built in parallel with Phases 8 / 9, beginning with B1 (wasm build on WebGPU with the test route, the local server in the browser and virtual power).
- **Q4 – Hosting: decided (2026-11-14, dashboard):** **own servers for now** (no CDN provider); world packages are served from our machines with HTTP caching and range requests, a CDN can come later if traffic needs it.
- **Q5 – Anonymous access: decided (2026-11-14, dashboard):** **anonymous riding is allowed**, but with a plain default ("boring") bike and character and **no customization**: the workshop and the character editor are locked (shown with a "log in to customize" hint), no hearts are earned or spent, nothing is saved across visits. Only logged-in users customize their bike and character (and keep hearts, parts and kit, §14 Phase 4).
- **Q6 – Same app or a lighter "web edition"? Decided (2026-11-14, dashboard): full parity** – the browser app is the same app with the same worlds and features as the desktop (quality presets and streaming adapt to the device, not a reduced edition).


#### 10G – Status: B1, B2, B3 and B4 built (2026-11-14)

**B1 – Wasm build: done.** `apps/web` (crate `ocw-web`, a `cdylib`) is the same client as the desktop app, built for `wasm32-unknown-unknown` and run by the browser on **WebGPU** (Bevy's `webgpu` + `web` features, only for the wasm target):

- **One client, shared crates (Q6: full parity).** The desktop's Bevy-free client logic moved into the new crate **`ocw-client`**: `ClientCore` (prediction, reconciliation, interpolation, the ride view), events and ghosts, god-mode state, the pure key logic (`input`: power ramps, `KeyThrottle`, lean and line steering, junction keys), the GPS position and the sky (`sky`: the zone of the place, the HUD clock, `sky_view`). `apps/desktop` uses it unchanged in behaviour; the renderer `ocw-render` is shared as before. Desktop-only parts stay in `apps/desktop` for now and move into shared crates behind web backends milestone by milestone (devices B4, audio and voice B5, menus / settings / captures B6), until both apps are thin entry points over the same plugins.
- **Local server in the browser.** The same `GameServer`, driven by the frame loop: `ocw_server::local::FrameServer::advance(frame time)` runs the 50 Hz ticks that fit (counted in whole µs, no drift) and drops a backlog beyond 0.25 s like the server thread (`MAX_BACKLOG_S`, e.g. a tab in the background). Unit tests: tick counts over uneven frames, tick-for-tick equality with `SteppedServer`, the backlog rule, the same snapshot rate as the threaded `LocalServer` over the same wall time.
- **Virtual power.** `ocw_devices::FrameVirtualTrainer`: the virtual trainer's logic (`virtual_power`, shared with the threaded `VirtualTrainer`) reporting at 4 Hz from the frame time; 200 W at the start, the desktop's keys (↑ adds power, ↓ stops, Page Up / Down ±10 W, A autodrive, W wild, ← / → line or lean) and the console (`sim.power`, `teleport`, `net.stats`, every `ocw-render` command).
- **The test route** (Hills, or `?route=loop`), 5 bots, the sky and lights from the server's world clock (`ocw-sky`, Vaduz time), the chase camera, the HUD with the ISO timestamp, the console. Page options in the query string (`route`, `bots`, `power`, `start`, `name`).
- **wasm-safety fixes:** `std::time::SystemTime` panics on wasm32-unknown-unknown, so the server's clock, the voice clock and the HUD timestamp use `web-time` there, and chrono's `wasmbind` gives the local UTC offset; `ocw-devices` builds without btleplug / tokio on wasm32; the server's `net`, `worlds` and `LocalServer` thread are native only. No getrandom backend was needed.
- **Tooling:** `make deps-web` (wasm-bindgen-cli of exactly the `wasm-bindgen` version in `Cargo.lock`, 0.2.130, via `cargo install --locked`, no sudo), `make web` (`scripts/build-web.sh`: the `web` profile = release, `opt-level = "s"`, thin LTO; wasm-bindgen `--target web` without the name section; `wasm-opt` when installed; output and `sizes.txt` in `target/web/`), `make web-serve` (`scripts/web-serve.py`: COOP / COEP headers, wasm MIME type), `make test-web` (`scripts/test-web.py`: headless Chromium on WebGPU over the DevTools protocol, console log, timings, a screenshot in `test-output/web/`). `make check` builds `ocw-web` for wasm32 (`test-web-build`); `test-wasm` also checks `ocw-devices` and the `ocw-server` lib; CI has a `web` job that runs `make web` and keeps `target/web/` as an artifact.

**Measured (marge, 2026-11-14):**

| | |
|---|---|
| `ocw_web_bg.wasm` | 63.8 MiB raw, 14.3 MiB gzip -9, 9.1 MiB brotli -q 11 (budget 10C: ≤ 30 MB compressed); the name section (another ~87 MB) is stripped; no wasm-opt yet. Code ~53 MB, data ~13 MB (embedded models, fonts, styles) |
| `ocw_web.js` | 180 KiB (wasm-bindgen glue) |
| Build | `make web` on marge: 4–9 min (the `web` profile's thin-LTO link of Bevy for wasm32; 2 min when only our crates changed) |
| Load (local server, headless Chromium 156) | served from localhost (no compression): HTML 6 ms, the .wasm downloaded at 0.2 s, compiled and started at 0.73 s, the rider joined the local server at 1.04 s after navigation; the first headless start of Chromium on marge waited 25 s for the keyring over D-Bus until `--password-store=basic` |
| Running | WebGPU on the AMD Radeon RX Vega (Dawn on Vulkan, headless `--enable-unsafe-webgpu`); the local server ticks at 50 Hz (100 ticks per 2 s in the log), the rider rides at 200 W; screenshot with HUD and ISO timestamp |

**Not yet / notes for later milestones:** WebGL2 fallback (Bevy selects WebGPU or WebGL2 at compile time: a second build, B7); multiplayer (B3), Web Bluetooth (B4), audio (B5; Bevy's audio plugin is off), menus, settings, gamepad parity and captures with EXIF (B6; the test screenshot comes from the DevTools protocol, without EXIF); the material library (flat land-use colours, as on the desktop without it); `wasm-opt` and a CDN with brotli (B8). The canvas fills the page; the arrows ride instead of scrolling it.

**B2 – Worlds over HTTP: done (v2.99).** `?world=liechtenstein&route=vaduz-malbun` streams a world package from our own server (Q4) into the browser app; the ride starts when the first tiles are there, more come while riding.

- **Shared, pure and tested natively:** `ocw_worldstore::stream` builds for wasm32 (the crate's file system, keys and `ureq` parts are native only; `test-wasm` checks it). `accept_manifest` (size, parse, signature chain under the unsigned policy) is the same check `--fetch-world` now uses; the desktop's download is otherwise unchanged. `plan`: the base chunks (`header`, `sections`, every `section …`), the `tile X,Z` chunks around a position (nearest first), merged into `Range` requests (gap ≤ 64 kB, ≤ 4 MB each). `load::load_chunks`: cache first, then the ranges; every chunk is checked against its SHA-256 in the manifest before use; a mismatch is fetched once more on its own, then the load fails with "chunk … does not match its SHA-256 … the copy on the server is damaged" (nothing damaged is cached). `cache::ChunkCache` over a `ChunkStore` (async trait: OPFS in the browser, `MemoryStore` in the tests): files `<id>@<version>@<chunk>`, an index with sizes and last use, LRU eviction by a size budget (default 1 GiB), re-verification on read (a damaged cached file is removed and refetched), store errors (quota, no OPFS) never stop the ride. `resident::ResidentTiles`: tiles within the fetch radius (3) are fetched, beyond the keep radius (5) dropped, over the memory budget (96 MB of decoded tiles) the farthest beyond the fetch radius first. `driver::StreamDriver`: the sequence as async steps (`latest` → manifest → base → tile batches) sharing `StreamShared` (progress, base, requests, arrived tiles) with the app. `progress::LoadProgress`: the loading screen as text.
- **Signatures in the browser:** ed25519 and SHA-256 (`ocw_world::manifest`) build for wasm, so the full signature chain is checked as on the desktop. The page trusts the production roots (none yet); on development pages only (localhost) also `trust=dev`, `trust-root=HEX` and `allow-unsigned=1` (chunks still verified). `make web-worlds` (`scripts/publish-web-worlds.sh`) signs a copy of the pilot package with a development root kept in the git-ignored `target/web-keys/` and publishes it to `target/web/worlds/` (`world-builder publish`, which now also writes `<id>/latest`); `make web-serve` serves it with single byte ranges (206, Content-Range, 416) and prints the links.
- **The app (`apps/web`):** `world.rs` (Bevy glue, tested natively with the driver stepped over an in-memory server: load, ride the package route, teleport → tiles come and go, second load from the cache); `browser.rs` (wasm only, thin): `fetch()` with `Range`, `OpfsStore` (`navigator.storage.getDirectory()`, folder `ocw-worlds`, `persist()` asked), the storage estimate, the driver in a `spawn_local` loop, the loading screen in the page's `#ocw-status`. The session waits for the base and first tiles (`WebSession::ride` on `Course::RouteRide` / `FreeRide` over the package's road graph); `WorldSet::update_tiles` hands changed tiles to the renderer without re-meshing the near tiles (`tiles_version` rebuilds the far terrain). Page options: `world` (id, `ID@VERSION`, manifest URL), `route` (package route, by name or slug), `free=1`, `tiles`, `keep`, `memory-mb`, `cache-mb`, `cache=0`; console `world.info` and `time …` (the time machine, as on the desktop); `window.ocw.console(line)` and `window.ocw.wasmMemory()` for tests. Fix on the way: after a jump the renderer waited for the urgent tiles' tasks with `block_on`, which never returns on wasm (one thread): there they are built in place.
- **Test (`make test-web-world`, `scripts/test-web.py --world`):** headless Chromium on marge streams Liechtenstein (signed by the dev root) from the dev server, rides Vaduz – Malbun at noon (`time noon`), teleports to km 6, screenshots (loading screen, ride, after the teleport), then loads the page again in the same profile: from OPFS, no package bytes over the network, faster.

**Measured (marge, 2026-11-14, Chromium on WebGPU, localhost):**

| | first load (network) | second load (OPFS cache) |
|---|---|---|
| world ready after navigation | 2.74 s (2.49 s in an earlier run) | 2.16 s (1.60 s) |
| in the app (stream start → ride) | 1.88 s: manifest 0.01 s, base 1.1 s (11.2 MB in 15 chunks: mostly decoding), 49 first tiles | 1.50 s |
| package bytes over HTTP | 14.2 MB in 34 requests (manifest and merged ranges) before the start; 15.9 MB served with the teleport (45 % of the 35.2 MB package) | 0 (only `latest` and the manifest, 105 kB) |
| from the cache | 0 | 14.2 MB in 64 chunks; OPFS holds 15.7 MB in 94 files (quota 10.7 GB, not persisted in headless) |
| resident tiles | 49 of 510 at the start (3.1 MB decoded), peak 4.2 MB; 7 dropped after the teleport | 49 (3.1 MB) |
| wasm memory | peak 652–759 MB (it only grows; the renderer's meshes and assets, not the tiles); JS heap 28 MB | 700 MB |
| `ocw_web_bg.wasm` | 71.7 MiB raw, 15.8 MiB gzip, 10.0 MiB brotli (B1: 63.8 / 14.3 / 9.1; collisions and Avian came in between) | |

**Open (B2):** the wasm memory (0.65–0.76 GB) is within 10C's 1.5 GB but far above the tiles' few MB: what the renderer keeps per tile (meshes, colour maps, collider data) should be measured and trimmed before larger worlds; the base (11 MB) is fetched whole before the ride – the seasonal tint maps (`SEAS`, 4.8 MB) and the land use could come per tile with a package format that splits them; range requests run one after another (parallel fetches would help on real networks); neighbour packages (`WorldSet` with several members) stream only the home package so far; no Service Worker for the app itself; signatures by production roots (and their distribution) come with B8.

**B3 – Multiplayer in the browser: done (v3.02).** `?server=https://HOST:47801&cert-hash=HEX&name=Ada` joins a remote `ocw-server` session over WebTransport; desktop (`ocw-desktop --server`), `ocw-bot` and browser riders ride together and see each other.

- **Server:** the WebTransport endpoint of §6.2 (`--webtransport`, `web-transport-quinn` on our quinn endpoint, UDP 47801, path `/ocw/1`; the 13-day ECDSA certificate by hash, kept in `--tls-dir`, rotated two days before the end; `--web-info-file`; per-endpoint counters in `--stats-file`). The startup prints the page options.
- **One client loop:** the browser's `WebSession` runs the shared `ClientCore` (prediction, reconciliation with the snapshot age from the RTT, interpolation of the others) on either the local `FrameServer` or a `RemoteLink`: the wasm-safe `ocw_server::wire::ClientLink` state machine plus a `Transport` that only moves bytes – in the browser `webtransport.rs` (own `wasm-bindgen` bindings for `WebTransport`, its bidirectional stream and datagrams: web-sys has them only behind `web_sys_unstable_apis`), in the tests a fake over an in-memory `GameServer` (datagrams, stream-only, reordering, refusal). Messages are time-stamped when the transport gets them. The HUD shows "Connecting to the server …" / "Disconnected: …" as on the desktop; `net.stats` shows the link, the RTT (`getStats().smoothedRtt`, else Hello → Welcome), counters, the snapshot timing and the riders seen; `window.ocw.netStats()` gives the same as JSON. Dev commands only on the local server.
- **Anonymous rider (Q5):** browser riders send `RiderLook::anonymous()` (grey frame, light grey kit, white helmet, plain) on the standard road bike; no customization until accounts (Phase 5) – only the name (`name=`).
- **Snapshot timing:** `ocw_server::timing::SnapshotTiming` (interval mean / jitter / percentiles, age = arrival wall clock − the snapshot's server instant `world_unix_ms − offset_ms`, meaningful on one machine), used by the browser and `ocw-bot` (its report's `timing`), so both are measured alike. `ocw-bot --server https://…` rides over WebTransport natively.
- **Tests:** `wire` (frames from any chunking, the state machine: queueing, samples on the stream until answered, datagrams only when allowed and small enough, broken frames, the first close reason), `timing`, the certificate (13-day validity in the DER, ECDSA P-256, rotation, kept in the directory, damaged / too-long files replaced), WebTransport URLs, `services/server/tests/webtransport_loopback.rs` (a WebTransport client joins and rides with samples as datagrams; QUIC and WebTransport riders in one session see each other's power; stream-only fallback; a wrong hash and the native certificate are refused; a shutdown reaches the client; rotation while running: the old session goes on, the old hash is refused, the new one joins), the browser session and plugin over the fake transport, the page options. **End to end:** `make test-web-multiplayer` (`scripts/test-web-multiplayer.py`): on marge `ocw-server --listen --webtransport` (Hills, no server bots), two `ocw-bot` riders over QUIC and one over WebTransport (all following the browser rider), headless Chromium on WebGPU joins with the cert hash, sets noon with the time machine (`--time-control anyone`), rides 30 s; checks: joined over WebTransport, ≥ 2 other riders seen, the server applied its samples (`last_input_seq`, the rider at 200 W in the server log, WebTransport datagrams in the statistics), every bot saw the browser rider; screenshot. `--desktop` adds `ocw-desktop --server` (headless X11) to the session.

**Measured (marge, 2026-11-14, everything on one machine, 30 s; `test-output/web/web-mp-20261114-061908.*`):** all checks passed with 2 QUIC bots, 1 WebTransport bot, the desktop client and the browser in one session (5 riders); the browser joined 2.4 s after navigation and saw all 4 others; the server applied its samples up to #121 (4 Hz for 30 s), the WebTransport endpoint received 263 messages from the browser and the WebTransport bot, 255 of them as datagrams.

| snapshots at | interval mean | jitter (σ) | p95 / p99 / max | age mean / p95 / max |
|---|---|---|---|---|
| browser (WebTransport, Chromium 156) | 100.0 ms | 16.2 ms | 115 / 126 / 226 ms | −5.9 / 8.5 / 239 ms |
| `ocw-bot` over WebTransport | 100.0 ms | 3.7 ms | 104 / 117 / 126 ms | −6.4 / 0.0 / 20 ms |
| `ocw-bot` over QUIC (×2) | 100.0 ms | 3.0–3.6 ms | 104 / 109–119 / 120 ms | −7.6 / −3.5 / 12 ms |

The ages carry a common offset of about −8 ms (the server's tick instant runs slightly ahead of the wall clock), so the one-way delay is a few ms everywhere. WebTransport itself adds nothing measurable (the WebTransport bot equals the QUIC ones); the browser's extra jitter (≈ one 60 Hz frame) comes from its main thread: the datagram reader's promises resolve between the frames Bevy renders, so arrivals are frame-quantised, and a long frame (start-up, shader compilation) delays a few snapshots (max 226 ms). The interpolation delay (§6.3, 100–150 ms) covers it. A repeat after rebasing on 3.00 (`web-mp-20261114-063344`): browser jitter 12.9 ms (max 133 ms), the bots 2.8–8.4 ms – the bots' jitter depends on the load of the shared CPU (the desktop client renders in software on it); without the desktop client it was 0.01 ms.

**Open (B3):** reading the WebTransport streams in a Web Worker (off the render thread) would remove the frame quantisation; production certificates and no `cert-hash` (B8); a browser rider rides only test routes and packages the server has too (the Welcome check as on the desktop); reconnect after a drop needs a page reload (as on the desktop, §6.3 session hold not built); Safari / Firefox not tested yet (B8 browser matrix); voice in the browser (B5).

**B4 – Devices in the browser: done (v3.09).** The page's **Devices** panel (bottom right; a page overlay until the menus come with B6) has **+ Trainer**, **+ Power meter** and **+ Heart rate**; each opens Chromium's Bluetooth chooser, and the device then runs **the same code as on the desktop**.

- **One protocol, two links.** `ocw-devices`' `ble` module now builds for wasm32: `protocol` (the pure state machine), `runner::run` and the new `runner::supervise` (connect → run → `Disconnected` → wait 5 s taking commands → connect again; the latest SIM / ERG target is applied on every new connection) no longer need Tokio: a `Timer` gives them time (`TokioTimer` natively, `JsTimer` = `setTimeout` in the browser; `select` of `futures`, `web_time::Instant`). The desktop's btleplug backend (`ble::btle`) uses `supervise` too. `DeviceEvent::Connected { kind }` reports what a device is once it is subscribed ("smart trainer (FTMS)", "smart trainer (Wahoo control)", "power meter", "heart-rate sensor", …); the desktop prints it.
- **The Web Bluetooth `GattLink`** (`ble::web`, pure, tested natively): the boundary `WebGatt` is what the adapter needs from a browser `BluetoothDevice` (connect, connected, discover the permitted services' characteristics with their properties, read, write with / without response, `startNotifications`, the value and `gattserverdisconnected` events, disconnect). `WebGattLink` maps the UUID strings to OCW's numbers (SIG numbers; Wahoo's 128-bit control point in any case), writes with response where the characteristic allows it, and turns the events into the runner's notification stream, which **ends with the disconnect event** (the runner sees a drop at once, not at its 1 s connection check). `run_device` = `supervise` over one device, `gatt.disconnect()` at shutdown. `DeviceKind` (trainer, power meter, heart rate) builds the `requestDevice` options: one filter per service (trainer: FTMS or Cycling Power, and `namePrefix: "KICKR"`; power meter: Cycling Power or CSC; heart rate), and every fitness service plus Wahoo's as `optionalServices` (a site may open only the services it asked for). `Remembered`: the devices this origin used (kind, browser id, name; `localStorage`, newest first, at most 8).
- **The browser side** (wasm32 only, thin): own `wasm-bindgen` bindings for `navigator.bluetooth`, `BluetoothDevice`, its GATT server, services and characteristics (web-sys has them only behind `web_sys_unstable_apis`, a cfg for the whole build; every call is structural, so a test double works as well), `JsGatt` (the `WebGatt`), `WebBluetoothBackend` (a `DeviceBackend`: the JS device waits in a thread-local slot, the backend itself is `Send`; `start` spawns `run_device` on the page's event loop). The chooser must open **in the user gesture**: the panel's click handler calls `window.ocw.connectDevice(kind)`, which calls `requestDevice` synchronously; the console's `devices.connect trainer|power|hr` works too (the Enter key is the gesture). At the start the app asks `getAvailability()` and `getDevices()` (Chromium's permission persistence): permitted devices this origin remembers reconnect **without the chooser**.
- **Shared trainer control.** `TrainerControl` (SIM / ERG, §5.3) and the workout session moved from the desktop into `ocw-client` (the desktop keeps the files, menu and commands); `TrainerControl::follow` is the per-frame step both apps run (god-mode flying, down after a crash, bump pulses, then SIM / ERG), and **SIM now follows the build in force** – `set_profile(ClientCore::effective_profile())`, i.e. the profile with the server's `RiderBuild` effect: the trainer gets the build's Crr and CdA (0.5 · ρ · CdA) and the rider + bike mass, and a change is sent at the next update interval (`SimController::set_profile`; desktop and browser).
- **The browser app** (`apps/web/src/devices.rs`, pure, tested natively): `WebDevices` holds the devices (state connecting / connected / reconnecting, last power and heart rate), the trainer control and the workout. Power comes from the **virtual trainer until a trainer or power meter is connected**, then only from the devices (a heart-rate strap alone keeps the virtual power; while a device reconnects the rider gets no power, as on the desktop; forgetting the last one brings the virtual trainer back); device samples are not virtual (hearts, §14 Phase 4 4E). Every SIM / ERG command goes to every device and to the virtual trainer (ERG: it reports the target). HUD status, e.g. `TICKR 131 bpm | KICKR 234 W | trainer SIM +2.1 % (difficulty 50 %)`; `Virtual trainer 200 W (no Web Bluetooth here)`. Console as on the desktop: `devices.list`, `devices.connect`, `devices.forget <name|id|all>`, `trainer.mode sim|erg|off`, `trainer.difficulty`, `erg.target`, `workout start --example …|stop|skip|pause|resume|status|list` (the bundled examples; the panel shows in the HUD); page option `ftp=`. `window.ocw.devices()` gives the state as JSON (the panel and the tests read it).
- **Without Web Bluetooth** (Firefox, Safari, iOS; Q1): the panel's buttons are disabled with the message "This browser has no Web Bluetooth (Firefox, Safari, iOS): riding with virtual power …"; the console says it once; the HUD says "(no Web Bluetooth here)". No adapter (`getAvailability()` false): "No Bluetooth adapter (or Bluetooth is off)".
- **Tests (no hardware):** `ble::web` against a fake browser device (`FakeDevice`: an FTMS trainer with Device Information, an older KICKR with the upper-case Wahoo UUID, a strap): discovery and write types, notifications ending with the disconnect, **the whole trainer flow through the shared state machine** (connect → subscribe → power events → SIM 3 % / 5 % writes → ERG 200 W → drop → reconnect → control, start and the ERG target again → shutdown disconnects), the KICKR with Wahoo control and a failed reconnect attempt, the strap; `supervise` with a mock (failed attempt, drop, a target set while away applied on reconnect); `SimController` / `TrainerControl` with a new build; `WebDevices` (virtual until a power device, states and messages, commands to every device, a late device gets the latest target, ERG and workouts, the no-Bluetooth wording); the plugin's commands headless.
- **Browser tests on marge (headless Chromium 156, WebGPU):**
  - `make test-web-ble` – a **fake `navigator.bluetooth`** (`scripts/web-ble-fake.js`, injected with `Page.addScriptToEvaluateOnNewDocument`; an FTMS trainer at 321 W / 88 rpm that holds ERG targets like an ideal trainer, a strap at 142 bpm; `getDevices` from the session) and real mouse clicks (`Input.dispatchMouseEvent`: `navigator.userActivation` is active in `requestDevice`). 18 checks passed (`test-output/web/web-ble-fake-20261114-092226*`): virtual 120 W first; + Trainer → the rider rides on 321 W from the device; the filters / optional services; + Heart rate; 12 s on the Hills: control, start, **29 SIM writes** with the hills' gradients (3.4 … 2.6 %, then down to −0.8 %); `erg.target 180` → the write `05 b4 00` and the rider at 180 W; a drop (out of range 3 s) → "reconnecting", connected again after the 5 s pause with control, start and ERG 180 re-sent; SIM again; a **reload** → both devices back without a chooser (0 `requestDevice` calls); `#nobt` (no `navigator.bluetooth`) → virtual power, disabled buttons and the message.
  - `make test-web-ble-emulation` – **Chromium's own Web Bluetooth stack** with a simulated peripheral: DevTools `BluetoothEmulation` (on the **browser** target in Chromium 156, not the page) – `simulatePreconnectedPeripheral`, `addService` / `addCharacteristic` and the CCCD descriptor 0x2902 (without it `startNotifications` fails with "GATT Error: Not supported"), the GATT and characteristic operations answered from the test – and the chooser answered with `DeviceAccess.selectPrompt` (**one answer per prompt**: Chromium fires `deviceRequestPrompted` twice and a second `selectPrompt` fails a CHECK, the browser exits). A KICKR-like device (Cycling Power + Wahoo's control point): chooser, connection, discovery ("smart trainer (Wahoo control)"), both subscriptions and the Wahoo writes (unlock, SIM mode, then a grade every 1.5 s) arrive. The emulation cannot send notifications, so no power arrives and no write is answered (the protocol's write timeout releases the next one); power and answers are the fake's part.
  - `make test-web-ble-real` – **the real Wahoo KICKR from the browser:** headless Chromium on marge reaches BlueZ over D-Bus; Web Bluetooth on Linux needs `--enable-experimental-web-platform-features` (without it there is no `navigator.bluetooth`; `getDevices` also with `--enable-features=WebBluetoothNewPermissionsBackend`). The chooser's list comes over `DeviceAccess` (14 devices around marge); the test picks "Wahoo KICKR A208" and rides: connected as **"smart trainer (Wahoo control)"** (this KICKR has Cycling Power + Wahoo's control point, no FTMS), 21 power samples in 20 s (0 W: nobody pedalled), 70 SIM commands with the hills' gradients, ERG (`erg.target 100` sent as 60 W: the ERG controller's low-cadence catch-up, 0.6 × target, with nobody pedalling), back to SIM; no rejected request (`test-output/web/web-ble-real-20261114-092815*`). Connecting took 21 s (BlueZ's first GATT discovery of the KICKR); the first control writes after connecting were partly unanswered within 1.5 s and one failed ("GATT operation failed for unknown reason"), later ones were answered.

**Open (B4):** a rider on the KICKR should try SIM and ERG in the browser (power and the feel of the gradients were not checked with pedalling); the panel is a page overlay until the menus (B6); the console's chooser needs the key press that ran it (works in Chromium; a menu button is the safer gesture); Chromium on Linux needs the experimental flag for Web Bluetooth (Windows, macOS, ChromeOS, Android have it by default); the KICKR's slow first connection on BlueZ; the sensor bridge for browsers without Web Bluetooth (B7); a workouts folder in the browser (files: B6).


---

### Phase 11 – VR mode (planning, 2026-11-13)

**Goal:** ride Open Cycling World in a VR headset: Oculus / Meta Rift and Quest, Valve Index, HTC Vive, Pico, Varjo and other vendors. The rider sits on the trainer, sees the world in stereo with head tracking and the own bike and hands in front of them, and keeps every feature: multiplayer, voice (spatial), workouts, the time and weather machines.

#### 11A – Platform facts (as found 2026-11-13; re-check before starting)

- **OpenXR** (Khronos) is the one cross-vendor API. Every current PC-VR runtime implements it: Meta's Quest Link / Oculus PC app (Rift, Rift S, Quest via Link / Air Link), SteamVR (Index, Vive, Pico, Quest via Steam Link, many others), Varjo, Pimax. On Linux: **Monado** (open source), SteamVR, and **WiVRn / ALVR** for streaming to Quest / Pico. Coding against OpenXR alone covers all vendors. No vendor SDK (the old Oculus / LibOVR SDK is deprecated).
- **Oculus Rift / Rift S:** discontinued hardware, still usable on Windows through the Meta PC app's OpenXR runtime. Never on Linux or macOS natively, though some of these headsets run through community drivers on Linux with limited support.
- **Windows Mixed Reality** (HP Reverb, Samsung Odyssey…) was **removed in Windows 11 24H2** ([Road to VR](https://roadtovr.com/windows-11-drops-wmr-support-24h2/), [heise](https://heise.de/-9979921)). Only older Windows versions or community drivers on Linux / SteamVR can still use them; not a target.
- **macOS:** no mainstream OpenXR runtime for PC VR, so **no VR on macOS**. Apple Vision Pro uses visionOS (no OpenXR); a native visionOS app is out of scope (like iOS, §1.3).
- **Standalone headsets** (Quest, Pico): option 1 is **PC VR streaming** (Link / Air Link, Steam Link, Virtual Desktop, WiVRn, ALVR), so no extra build. Option 2 is a native **Android OpenXR** build (Quest / Pico store): planned in §14 Phase 14.
- **Browser:** **WebXR** (Quest Browser, Chrome with a PC headset) fits the browser app (Phase 10) later.
- **Bevy integration:** the community crates `bevy_mod_xr` / `bevy_mod_openxr` (successor of the unmaintained `bevy_oxr`). The latest found is 0.4 for **Bevy 0.17** ([docs.rs](https://docs.rs/crate/bevy_mod_openxr/latest)), while OCW is on Bevy 0.20, so we need to port / fork it or write our own thin integration (the `openxr` crate + wgpu Vulkan / D3D12 interop, a stereo camera pair rendering into the OpenXR swapchain). To be decided by a spike (11D, X1).

#### 11B – Design

- **One app, a VR mode:** `ocw-desktop --vr` (and a menu switch) starts an OpenXR session. Without a runtime or headset the app runs as today, with a clear message. The **monitor** keeps a **spectator / mirror view**: chase camera or the headset view, for streaming, for the kiosk, and for the art project's captures (with the ISO timestamp). Captures come from the mirror, never from inside the headset.
- **Engine independence:** XR lives behind a `ocw-render` plugin; the sim, server, protocol and devices don't change. Head pose and controllers are only view inputs. The server stays authoritative (§6.1).
- **Camera:** first-person from the rider's head on the bike (rider height from the profile, §2.6). Head tracking is added on top of the seated riding pose: the head can look around and lean, but the bike's path stays physics-driven. Recentre with a button or after a long head offset.
- **Own body:** the own bike, handlebar, arms, hands on the bars and legs pedalling at the real cadence, seen from the first-person view (the Phase 4 rig with IK). Optional: hands from controller or hand tracking if they leave the bars.
- **HUD in VR:** world-space panels instead of a flat HUD:
  - a **bike computer** on the stem (power, speed, cadence, HR, gradient, distance);
  - a **head-locked minimum** (critical warnings only);
  - **panels** for menus, workouts, the event leaderboard and junction arrows at road level ahead;
  - the ISO timestamp on the mirror view.
- **Input in VR:** hands are on the handlebar, so the main inputs are the **trainer / sensors** (power, cadence) and **steering by leaning the head** (optional, mapped to the existing lean steering, §5.2.1), handlebar buttons (BLE remotes, later), gamepad, voice commands (later), and controller pointer for menus (gaze + click as a fallback); the controller ray is one more pointer of the input model (§3.2.2), so menus and the workshop scene need no VR-specific pages.
- **Comfort** (motion sickness is the main risk; on a trainer pedalling helps, since the vection matches effort):
  - stable horizon (no camera roll with the bike's lean by default);
  - no head bob;
  - smooth, rate-limited acceleration of the view;
  - a **comfort vignette** in sharp turns, hard braking and steep descents (strength setting);
  - always-visible cockpit (bike and bars) as a static reference;
  - snap or smooth turning only in menus;
  - a "seated / standing" calibration;
  - fade to black on teleports, ride switches and U-turns.
- **Performance:** stereo at the headset's native rate (72–120 Hz) costs roughly 2–3× the monitor budget.
  - **Rendering:** single-pass stereo / multiview where wgpu supports it, fixed foveated rendering (and eye-tracked where the runtime offers it), MSAA instead of TAA (less smear in VR), and a **VR quality preset** (shorter view distance, fewer trees and shadows, lower LOD bias).
  - **Frame budget:** stay under budget; runtime reprojection (ASW / motion smoothing) is a safety net, not the plan.
  - **Targets:** 90 Hz on a GTX 1070 / RX 5700-class GPU with the VR preset, 72 Hz minimum.
- **Audio:** game audio and **voice spatialised** with the head pose (the spatial audio of §7 already works with positions); the headset's audio device as the default output in VR.
- **Multiplayer:** other riders see a VR rider normally; optionally their head turns follow the real head (cosmetic, sent as a small orientation in the protocol, only added fields).

#### 11C – Supported devices (target matrix)

| Headset | Runtime | OS | Status |
|---|---|---|---|
| Meta Quest 2 / 3 / 3S / Pro (Link / Air Link) | Meta PC app (OpenXR) | Windows | target |
| Meta Quest (Steam Link / Virtual Desktop) | SteamVR / VD | Windows | target |
| Meta Quest (WiVRn / ALVR) | WiVRn / ALVR (OpenXR) | Linux | target |
| Oculus Rift (CV1, first generation) | Meta PC app (OpenXR) | Windows | **primary development headset** (v2.48) |
| Oculus Rift S | Meta PC app (OpenXR) | Windows | supported (legacy hardware) |
| Valve Index, HTC Vive / Vive Pro, Bigscreen Beyond, Pimax | SteamVR | Windows, Linux | target |
| Pico 4 (streaming) | Pico Connect / SteamVR / ALVR | Windows, Linux | target |
| Varjo | Varjo Base (OpenXR) | Windows | best effort |
| WMR headsets | – | – | not supported (removed in Windows 11 24H2) |
| Apple Vision Pro | visionOS | – | out of scope |
| Standalone Quest / Pico (native Android build) | OpenXR on Android | Android | Phase 14 |
| WebXR | browser | – | Phase 14 (in the Phase 10 browser app) |

#### 11D – Milestones (proposal)

- **X1 – Spike:** an OpenXR session from Bevy 0.20, comparing a port of `bevy_mod_openxr` with our own thin integration. A stereo view of the test route with head tracking on the **Oculus Rift CV1** via the Meta runtime (Windows) first, then SteamVR / Monado (Linux) as a second runtime check. Measure the frame time. Decide on the integration (spec entry).
- **X2 – Riding in VR:** `--vr`, first-person camera on the bike, own body with IK, recentre, mirror view, VR quality preset, captures from the mirror.
- **X3 – VR HUD and menus:** bike computer on the stem, world-space menus with controller / gaze input, junction arrows in the world.
- **X4 – Comfort:** vignette, horizon lock, fades, calibration, settings page *VR comfort*.
- **X5 – Audio and voice in VR:** head-relative spatial audio, the headset's audio device, voice positions.
- **X6 – Head-lean steering** (optional) and handlebar remotes.
- **X7 – Device matrix tests** (11C) and the performance tuning to the targets.
- **X8 – Native standalone build** (Android OpenXR for Quest / Pico) and **WebXR** in the browser app: planned in its own phase, §14 Phase 14 (milestones V1–V9).

**Unit tests** (no headset needed):

- head-pose composition (bike pose + seated head + tracking offset, recentre);
- the comfort vignette curve from turn rate / acceleration;
- horizon lock;
- world-space panel layout and hit testing;
- IK hand / handlebar placement;
- the head-lean → steering mapping;
- quality-preset selection.

**Integration tests:**

- the OpenXR loader against **Monado's headless / simulated device** on Linux in CI (session start, frame loop, swapchain), so VR code is exercised without hardware;
- screenshots of the mirror view with the scene harness.

#### 11E – Open questions (for the user)

- **Q1 – decided (v2.48):** an **Oculus Rift CV1** (first generation) is the development and test headset, on Windows through the Meta PC app's OpenXR runtime (the CV1 has no usable Linux runtime, so VR testing runs on Win11Builder or another Windows PC with the Rift; Monado covers CI). Development stays OpenXR-only, so other headsets need no vendor code. CV1 specifics to design for: 2160×1200 total (1080×1200 per eye), 90 Hz, 3-DoF+ constellation tracking with external sensors (USB 3.0), Oculus Touch controllers; frame budget 11.1 ms, asynchronous spacewarp as fallback at 45 Hz.
- **Q2 – Linux VR: decided (2026-11-14, dashboard):** **target it** – SteamVR, Monado and WiVRn on Linux (Valve Index natively, Quest via WiVRn), alongside the Rift CV1 on Windows (Q1). The Rift CV1 itself has no usable Linux runtime, so Linux VR is tested with Monado's simulated device in CI and on a supported headset when one is available.
- **Q3 – Integration: decided (2026-11-14, dashboard): both, compared** – the X1 spike builds **both** variants, a port / fork of `bevy_mod_openxr` to Bevy 0.20 and our own thin OpenXR layer (the `openxr` crate + wgpu Vulkan / D3D12 interop), and tests them side by side (and different versions of the crates) on the Rift CV1 (Windows) and Monado / SteamVR (Linux): frame time, latency, stability, code size, maintenance. The comparison goes into the spec; the winner is the integration from X2 on.
- **Q4 – Head-lean steering: decided (2026-11-14, dashboard):** **no head-lean steering** – steering in VR stays on the keys, the gamepad and autodrive, as on the desktop (a first answer "wanted" was replaced a few seconds later by this one).
- **Q5 – Native standalone (Quest / Pico store) and WebXR: decided (2026-11-14, dashboard):** **yes** – planned as its own phase (§14 Phase 14 – Standalone VR), not only as the future item X8.
- **Q6 – Priority: decided (2026-11-14, dashboard):** **at the end** – VR (Phase 11, and the standalone Phase 14) comes after the other phases.

### Phase 12 – Collisions, crashes and damage (planning, 2026-11-13; K1, K2 done, v2.98 – see 12I)

**Goal:** riders and world objects become solid. Riders who touch each other or an object are slowed down (soft bump) or crash and fall (hard hit). Autodrive avoids collisions by itself; with manual steering the rider has to avoid them. An optional damage model adds injuries to the rider and damage to the bike, shown by a damage indicator. Both are **switchable globally** (on / off).

#### 12A – Switches (decided by the user, 2026-11-13)

- **`ride.collisions` (on | off):** off = today's behaviour: riders pass through each other and objects (only the road edge of §5.2.1 holds them). On = the collision model below. Proposal for the default: **on** (Q1).
- **`ride.damage` (on | off):** only has an effect with collisions on. Off = bumps and crashes slow riders down and make them fall, but nothing accumulates. On = every hit adds to the rider's injury and the bike's damage (12E). Proposal for the default: **off** (Q1).
- Both are settings in the registry (§3.2.1: section `[ride]`, scope *live*, console `set ride.collisions on`, Settings → Ride) and server session parameters. **The server decides** (server-authoritative, §6.1): on the local server they follow the rider's settings; in a shared session the host sets them (like the world switch, `ServerConfig.collision_access`: host / anyone / nobody); in **organiser-set events** (races, group rides on a remote server) the organiser sets them per event, like the weather (Phase 9 Q4), and riders cannot change them. Sent in `Welcome` and `EventState` (additive fields, §6.2), shown in the event's info ("collisions on, damage off").
- **Exempt from collisions in every mode:** the time-trial ghost (a replay, §2.2), riders in the countdown formation (they stand still; collisions start with GO), spectators / camera, and riders whose client is reconnecting (they ride as ghosts until the next snapshot is acknowledged, so a lag spike cannot cause a crash).

#### 12B – Collision model (server, `ocw-sim::collide`, pure, deterministic, wasm)

- **Bodies.** A rider + bike is a footprint in the road's frame (§5.2: distance along the path `s`, lateral offset `d`): a rounded box **1.75 m long** (wheelbase + wheels, from the bike's physics profile, §2.5) and **0.5 m wide** (handlebar width), oriented by the rider's heading relative to the road. Inside the footprint the two **wheels** are marked: the front wheel (front 0.35 m) and the rear wheel (rear 0.35 m), because touching wheels is the classic way bunches crash.
- **Riders against riders.** Riders on the same path segment are checked in the path frame (exact and cheap: a sort by `s`, only neighbours within 3 m are tested; O(n log n) for a bunch of 100). At junctions and on different edges of the free-ride graph (§2.2.1) the check uses world positions (x, z) projected onto the road plane, in a 4 m spatial hash. Contacts are resolved at the 50 Hz tick, after the physics step and before the broadcast.
- **Riders against the world.** Static obstacles come from the world package: buildings (`BLDG` footprints), trees (trunk circles, 0.3–0.6 m), walls, fences, bollards, posts and barriers (OSM `barrier=*` nodes / ways, new additive section `OBST` in the package, written by the world builder with a 2 m corridor rule), tunnel walls and bridge railings (from the tunnel / bridge geometry, §4.8). On the road only a few of these are reachable (posts, barriers, bollards on paths); in **wild mode** (§5.2.1) every object is solid. The road edge keeps its scrape (§5.2.1); hitting it hard (steering into it at more than 30° to the road) counts as a hit on a wall.
- **Impact strength.** For each contact the server computes the **closing speed** `v_c` along the contact normal (the relative velocity of the two bodies; for a static object the rider's own velocity), the **contact type** (side by side, wheel to wheel, front wheel into a rear wheel, head-on, object) and the impact energy `E = ½ · m_red · v_c²` (`m_red` = reduced mass of rider + bike; for a static object the rider's mass).
- **Soft bump** (closing speed below the crash threshold): the two riders are pushed apart (lateral offsets separated to touching, no overlap), both lose speed by a share of the closing speed (an inelastic exchange of momentum: the faster rider slows down, the slower one is pushed a little), and the bikes **wobble** (a short lean oscillation of ±3–6°, 0.6 s). A touch from the side while riding parallel is the gentlest case (shoulder rub). The trainer feels it in SIM mode as a short resistance pulse (the speed loss as a brief grade increase; ERG unchanged).
- **Hard hit = crash** when one of these holds (tunable, measured by tests):
  - `v_c` ≥ **2.5 m/s** (9 km/h) rider against rider, or ≥ **3.5 m/s** (12.6 km/h) against a solid object;
  - **front wheel into another rider's rear wheel** with a lateral overlap (wheel touch) at `v_c` ≥ 1.0 m/s, or at any speed while the rider leans more than 10° into the other rider (hooking a wheel);
  - head-on (opposite directions, e.g. after a U-turn) at any speed above 1.5 m/s;
  - a rider ridden into from behind at speed (the rider behind crashes, the one in front gets a soft bump unless the energy passes the threshold for them too).
  Both riders in a hard rider-rider contact can crash; which one falls is decided by the contact type (the rider whose front wheel was hit falls; the rider hit in the rear wheel usually stays up), **deterministically** (no random numbers on the server, or a seed from the tick and the rider ids), so replays and the client's prediction agree.
- **Chain crashes:** a fallen rider and bike lie on the road as an obstacle (footprint of the fallen bike, 1.8 m × 1 m, for the time they are down), so riders behind must steer around it or crash too – a pile-up in a bunch is possible, as on the road.

#### 12C – Crash sequence

- **States** (part of the rider state in snapshots, additive field `RiderUpdate.crash`): `Riding → Falling (0.8 s) → Down (2–8 s, by impact energy) → Remount (1.5 s) → Riding` from standstill (0 km/h; the rider accelerates again with the own power). Ride time continues (the ride never pauses, §3.2.1); in races the clock runs, no neutralisation unless the organiser sets one.
- **Physics while down:** the rider's speed goes to 0 within the fall (sliding friction), power has no effect, autodrive's line is reset to the current lateral position. **Trainer:** SIM mode gets a flat road and the lowest resistance while the rider is down (so the legs keep spinning easily), ERG mode is held (the workout continues: crash or not, the plan is the plan; a workout may also pause its step, Q4).
- **Rendering** (`ocw-render`): a **ragdoll** with the Avian physics engine (Q5, decided 2026-11-14; built in K2, v2.98, 12I) – no scripted fall first: rider and bike fall from the server's crash with its push, the bike slides and its wheels spin, the rider lies apart. The bike lies apart from the rider. Remount: the rider picks up the bike and clips in. The chase camera shakes briefly and holds a wider orbit; in **VR** (Phase 11) the view never rolls or tumbles – a short fade to dark and back (comfort rule).
- **Sound** (procedural, §3.3): a bump (body thud + tyre squeak), a crash (impact, metal scrape of the bike sliding, a fading spin of the free wheel), the remount clicks. Other riders' crashes are placed in space like their bike sounds.
- **HUD:** "Crash!" with the time down counting; for others a crash marker over the fallen rider and a line in the event's feed (races: "R. Rider crashed at km 12.4").

#### 12D – Avoidance: autodrive avoids, manual riders must avoid

- **Autodrive (and every bot) avoids collisions by itself** (`ocw-sim::avoid`, the same code on server and client): every tick it looks ahead `max(4 s × speed, 15 m)` along the path for riders and obstacles in its lane. For each it computes the time to collision from the speed difference. When it falls below 3 s, autodrive **moves its line** to the free side (overtaking: the left side in right-hand traffic countries, the right side in left-hand traffic ones – from the package's region, §4.7; on narrow roads either side, keeping 0.6 m clear of the edge) with a smooth lane change (lateral speed ≤ 1 m/s, so it never cuts across another rider), and moves back once past. When both sides are blocked, autodrive **holds back**: it settles behind the rider in front at a 0.5 m wheel gap (drafting, §5.2), braking virtually (the speed above the one in front is shed, like coasting with brakes – the rider's power still counts, so a strong rider sits on the wheel until there is a gap). Junction exits and U-turns respect the same look-ahead. Autodrive never causes a collision by its own move; it can still be hit by a manually steered rider.
- **Manual steering (autodrive off) and wild mode:** **no help** – the rider steers, and collisions happen as the model says. Two optional aids (settings, off in organiser races unless allowed): a **proximity cue** (a soft bell / tyre-hum sound and a HUD arrow when someone is within 1 m beside or 2 m in front) and a **"last moment" warning** (an amber flash 1 s before a predicted hit). Neither steers for the rider.
- **Prediction (§6.1):** the client predicts its own rider's collisions with the last known positions of the others (interpolated, §6.5) and with static objects exactly; contacts with other riders are confirmed by the server, so a predicted bump the server did not see is reconciled away (small correction), and a crash is only shown when the server says so (no false falls from lag). Crash state, damage and the "who hit whom" record come only from the server.

#### 12E – Damage model (with `ride.damage on`)

- **Two meters, 0–100 %:** **rider condition** (injury) and **bike condition** (damage), both start at 100 % for each ride (no injuries carried over between rides; a persistent, "career" mode is Q6). Every hit takes off points by its impact energy and type:
  - soft bump: 0–2 % rider, 0–2 % bike (a shoulder rub costs nothing, a wheel touch a little);
  - crash: 5–25 % rider and 5–30 % bike, by energy and type (head-on and objects hit hardest);
  - riding through a fallen bike, a kerb (hard edge hit) or a pothole later (Phase 8 surfaces): bike only.
- **Injury levels** (rider condition) and their effects – the real rider's power is never touched; the effect is on the virtual rider:
  - 100–70 % *bruised*: no effect;
  - 70–40 % *hurt*: the virtual drivetrain loses 3–8 % of the power (the avatar rides stiffly), longer time down after a crash;
  - 40–15 % *injured*: 10–20 % loss, the avatar rides with a visible limp / bandage, every further crash takes longer;
  - below 15 % *badly injured*: the rider must stop – a **medical stop** of 60 s, then rides on at most 50 % effective power, or abandons (DNF in races; organiser option "DNF when badly injured").
- **Bike damage** is per part, each with its own visible and physical effect, chosen by the impact (front / rear / side):
  - **wheels** (true): a buckled wheel adds rolling resistance (+10–40 %), a visible wobble and a rubbing tick in the sound;
  - **tyres:** a crash can cause a **puncture** (by energy, deterministic): the rider must stop for a 30 s repair, or ride on slowly (flat: rolling resistance ×4, speed capped at 15 km/h);
  - **drivetrain** (derailleur, chain): efficiency loss 2–10 %, chain noise; a broken chain = stop for 45 s;
  - **handlebar / brakes:** steering a little less precise (manual lean rate −20 %), autodrive unaffected;
  - **frame / fork:** only from very hard hits; below 10 % bike condition the bike is **broken**: the rider waits for a spare bike (60 s, from the team car / neutral service in races) or the ride ends there.
  - Damage shows on the bike model: scratches and scuffs (decals), a bent wheel, a dropped chain, a torn jersey and dust on the rider.
- **Parts and hearts (v2.53, §14 Phase 4, 4E):** the damaged parts are the rider's own workshop parts (a damaged deep carbon wheel looks like one); with damage reset per ride (Q6 proposal) repairs are free and no hearts are lost; with a persistent mode a repair costs `ceil(price paid × (1 − condition) × 0.3)` hearts, and a broken part is replaced by the slot's free base part until repaired. A part's tier never changes its damage resistance (no paid durability).
- **Recovery:** riding on does not heal; a **stop** (speed 0 for 30 s, or a "service" point in races) restores bike parts that the organiser allows; the rider recovers 1 % per minute of easy riding (below 50 % FTP). Restart, a new ride or a world switch reset both meters.
- **Server and protocol:** damage is part of the server's rider state (`RiderUpdate.damage`: rider %, bike %, parts as a bit field, injury level), sent at 2 Hz and on every change; the event results show crashes and the final condition. FIT files get a lap / event marker per crash (developer field for the condition).

#### 12F – Damage indicator (HUD)

- A compact widget beside the bike computer: **two icons with bars** – a rider silhouette (condition %) and a bike silhouette (condition %) – each with its level word ("bruised", "hurt", "injured"; "scratched", "damaged", "broken"). Colours from the status palette (good / warning / serious / critical: green, amber, orange, red) and always an icon + word, never colour alone (accessibility). Parts with damage light up on the bike icon (wheel, tyre, chain, bars, frame), with a short label ("rear wheel buckled", "puncture: stop to repair").
- **On a hit:** the affected icon flashes and the bar drops with an animation, the loss is shown for 2 s ("−12 % bike").
- **Others:** in group rides and races a small damage dot next to riders in the standings / the rider list (amber / red), so the field sees who is hurt.
- Hidden when `ride.damage` is off; with collisions off nothing collision-related is shown. Settings → HUD can hide the widget (the effects stay).
- In VR (Phase 11) the indicator is part of the stem-mounted bike computer.

#### 12G – Bots, events and other modes

- **Bots** always use autodrive avoidance, so a field of bots never crashes by itself; they can be crashed by a human rider (and take damage). An option for **"chaotic" bots** that sometimes make mistakes (late reactions, wheel touches) for realism is Q3.
- **Group rides:** collisions are a session setting (12A); proposal: group rides default to **soft bumps only** (crash threshold raised, no chain crashes), so a social ride is not ruined (Q2).
- **Races:** organiser decides collisions, damage, neutral service (spare wheels / bikes and their delay), DNF rules and whether the proximity aids are allowed.
- **Time trial:** no other riders, so only world objects (and the road edge) count.
- **Workouts (ERG):** the workout continues; Q4 whether to pause the step while down.
- **Kiosk / show mode:** collisions off (the show rider rides through the bots, as now).
- **Strava / FIT uploads:** crashes are only markers; no injury data leaves the device unless the rider wants it.

#### 12H – Tests (spec §11.4: no step without tests)

- `ocw-sim::collide` unit tests: footprint overlap in the path frame and in world space; closing speed and contact types (side, wheel to wheel, front-to-rear, head-on, object); soft bump below the threshold (separation without overlap, speed exchange conserving momentum within a tolerance), crash at and above it; the deterministic choice of who falls; a fallen bike as an obstacle; collisions off = no contact ever; ghosts, formation and reconnecting riders exempt.
- `ocw-sim::avoid`: a field of 50 autodrive bots at mixed powers on a loop for 30 simulated minutes has **zero contacts**; an autodrive rider overtakes a slower one on the correct side by the region's traffic side; a blocked autodrive rider settles at the wheel gap; a manual rider steered into another crashes as the model says.
- Server: the crash state machine (durations by energy), trainer commands while down (SIM flat, ERG held), snapshot fields, event feed lines and results; 100-rider bunch performance: collisions within 0.3 ms per tick (§15).
- Damage: points per hit type, injury levels and their effects on the virtual power, punctures and repairs, recovery, reset on restart; damage off = meters never change.
- Client: prediction of bumps against interpolated riders reconciles without visible snaps; a crash appears only from the server; HUD widget levels, colours + words, and the hit animation (headless screenshots, X11 + Wayland); a scene in `scripts/scenes.toml` with a staged crash (console `dev.crash` / `dev.bump` on the local server, `Dev` permission) for the build history.
- World builder: `OBST` section written from OSM barriers, posts and bollards near roads; package round trip; lint finds obstacles on the road centreline (a data error: a gate across a through road is noted, not made solid, Q7).

#### 12I – Milestones (proposal)

- **K1 – Rider collisions, soft bumps:** footprints, path-frame and world-space checks, separation and speed exchange, wobble, `ride.collisions` setting and session parameter, sounds; autodrive avoidance (overtake / hold back) for riders and bots.
- **K2 – Crashes:** thresholds, contact types, crash states and fall animation, fallen bikes as obstacles, trainer behaviour while down, HUD crash display, event feed.
- **K3 – World objects:** `OBST` section in the world builder, buildings / trees / walls / barriers in wild mode, tunnel walls, bridge railings, hard road-edge hits.
- **K4 – Damage:** `ride.damage`, rider and bike meters, injury levels, part damage, punctures and repairs, recovery, protocol fields, FIT markers.
- **K5 – Damage indicator and visible damage:** HUD widget, standings dots, bike damage on the model, rider dust / bandage.
- **K6 – Events and options:** organiser settings, neutral service, DNF rules, proximity aids, group-ride soft mode, VR comfort fade.
- **K7:** persistent damage and injuries across rides with repairs in the workshop (decided 2026-11-14, Q6); ragdoll falls moved into K2 and chaotic bots into K6.

**Status (v2.98): K1 and K2 done.** As built:

- **Model (`ocw-sim::collide`, pure, deterministic, wasm):** footprints 1.75 × 0.5 m (fallen 1.8 × 1 m), wheel zones the front / rear 0.35 m; boxes by separating axes (the corners are not rounded). One route: the path frame (x = `s`, wrapping on a loop, y = `d`), a sort by `s` and neighbours within 3 m; free ride: world (x, z) in a 4 m spatial hash (junctions, different edges), corrections taken back into each rider's own (Δs, Δd). Contact kinds side, wheel to wheel (only at an angle: riding parallel a front wheel beside the rear wheel ahead is a wheel touch), wheel touch (its closing speed is the relative speed of the wheels), rear-end, head-on, object (a fallen bike). Soft bumps: separation to touching, an almost inelastic normal impulse (restitution 0.1) plus friction 0.3 along the contact (momentum kept, tested), a heading into the other rider halved, a wobble of 3–6° over 0.6 s (sent as `RiderUpdate.bump`, drawn by the client). Crashes as 12B; a hard side hit (2.5–3.5 m/s) brings down the rider who was hit (less sideways speed over the road towards the other; a tie by a coin from the tick and both ids), above 3.5 m/s both; the rider in front of a rear-end falls above 4 m/s. **Two autodrive riders never bring each other down** (only bumps): a crash needs a manually steered rider, a chaotic bot's mistake or a fallen bike – so a field of bots never crashes by itself (12G). Time down `2 + 6 · min(E / 1280 J, 1)` s.
- **Session / server:** the bodies are taken after the physics step, before the broadcast; crashed riders slide to a stop in the 0.8 s fall (with a sideways slide from the push), lie, remount and ride on from standstill; power has no effect, autodrive's line is reset to where they lie; event formations, god mode and the Echo rider are exempt; a client rider is a ghost for its first 2 s after joining and then until clear of everyone (a reconnect is a new join today; a transport-level "reconnecting" state is future work). The model in force: the organiser's rule for the event (`EventStart.collisions`), else off with `ride.collisions` off, else group rides soft (Q2), everything else full. `ServerConfig.collisions` / `chaotic_bots` / `collision_access` (local servers: on, the single player; `ocw-server --collisions on|off --chaotic-bots --collision-control host|anyone|nobody`); `RulesRequest` from the host changes them live (they reach everyone in the next snapshot and survive a world switch). `ServerConfig::default()` keeps collisions off (tests, embeddings).
- **Avoidance (`ocw-sim::avoid`, the same code on server and client):** look-ahead `max(4 s × v, 15 m)`, a move of the line when the time to collision is under 3 s or the gap under 1.5 m: to the region's overtaking side (`RoadNetwork::keep_left`: left-hand traffic passes on the right; routes without a package: right-hand traffic), on roads under 4.5 m either side; a line is only taken when it is free from just behind (riders closing from behind over the next 2 s) to the look-ahead and nobody rides beside in between; back to the own line once it is free; blocked: the virtual brake holds `v_front + 1.5/s · (gap − 0.5 m)`. In free ride only riders on the own path (same direction) are avoided. Tested: 50 bots at 110–340 W on the test loop for 30 simulated minutes – **zero contacts** with the field mixing; passing sides by region; the wheel gap held (±0.2 m).
- **Chaotic bots (Q3, a first model for K6):** with `ride.chaotic_bots` on every bot draws a mistake every 20 s (8 %, deterministic): 3 s of late reaction (no avoidance) drifting 0.45 m towards the rider ahead; a mistaking bot counts as manual for the crash rules. Off by default.
- **Client:** crashes only from the server (`RiderUpdate.crash` with phase, elapsed, time down, kind, who, the push and the speed); while down the prediction takes the server's state; bumps are reconciled by the usual blending (tested: no sideways step over 6 cm per tick, no snap); the own autodrive avoids with the others' last known positions; HUD line "Crash! (wheel touch) - back on the bike in 3.2 s", feed lines ("Bot 4 crashed at km 1.2", in an event's panel), a red marker over other fallen riders; the proximity cue (`ride.proximity_cue`, off) as a bell and a HUD text arrow ("<< rider close on the left"). Trainer: SIM flat while down, a soft bump a 0.5 s gradient pulse (0.6 × wobble rad), ERG unchanged and the step keeps running (`ERG_STEP_RUNS_WHILE_DOWN`, the Q4 proposal; Q4 stays open). Sounds (procedural): a bump (thud + tyre squeak, the own and within 15 m), a crash (impact, scrape, fading free wheel; within 40 m, not yet placed in space), the remount clicks, the proximity bell.
- **Ragdoll (Q5) – the K2 spike, 2026-11-14:** Avian 0.8 (`avian3d`, MIT OR Apache-2.0) against Rapier 0.36 (`rapier3d`, Apache-2.0; `bevy_rapier3d` 0.37), both for Bevy 0.20, measured on marge in a shared target directory over a cached Bevy: native dev build +123 s Avian vs +90 s Rapier, wasm32 +112 s vs +75 s – both build for the browser; a 10-capsule chain on spherical joints settled in both (0.29 ms per Bevy frame with Avian incl. the schedule, 0.04 ms per Rapier step); both pull `parry3d` (Apache-2.0). **Chosen: Avian** – the engine itself is MIT OR Apache-2.0 like our code, it is ECS-native for our Bevy (bodies and joints are components of the avatar's own segment entities, transforms sync and interpolate by themselves, no glue layer), it builds for wasm32 and its joints have swing / twist limits; Rapier compiles a little faster and steps faster, which does not matter for a few ragdolls; determinism is not needed (visual only). Built (`ocw-render::ragdoll`, also in the browser app): on the server's crash the avatar's 11 body segments (pelvis, torso, head, thighs, shins, upper arms, forearms; capsules to the next joint) become rigid bodies on spherical joints with swing / twist limits, the bike's frame a box and the wheels cylinders on hinges; they start with the rider's speed and the contact's push, tip towards its side, the bike slides away; a ground plane in the road's plane under the rider; on *Remount* the pieces ease (1.3 s) onto a fresh avatar in the riding pose, which takes over. Tested headless (spawn, settle on the road within 0.75 m, slid along, remount, cleanup).
- **Not yet (open):** the chase camera's shake and wider orbit and the VR fade (12C), the "last moment" warning and a proximity arrow drawn as a graphic (12D), spatial crash sounds, the client predicting bumps itself (the server's are blended in), obstacles from the world (K3), damage (K4–K5), organiser settings and neutral service on remote servers (K6), the kiosk's show mode turning collisions off (it rides with the default, on).
- **Performance (§15):** the collisions of 100 riders take 0.027 ms per tick in a bunch and 0.078 ms in a full crush (~180 contacts; release on marge, `cargo test --release -p ocw-sim a_hundred_riders`); the whole server tick with 100 bots and everything on: 322 µs mean (216 µs with collisions off; `ocw-server --bots 100 --fast`).

#### 12J – Open questions (for the user)

- **Q1 – Defaults: decided (2026-11-14, dashboard):** **collisions on, damage off** by default (the proposal).
- **Q2 – Group rides: decided (2026-11-14, dashboard):** group rides default to **soft bumps only** (no crashes); **races use the full model** (organisers can still change it per event, 12A).
- **Q3 – Bots' mistakes: decided (2026-11-14, dashboard):** **chaotic bots, as a setting** – bots sometimes make mistakes (late reactions, wheel touches) that can cause crashes; a session / event setting (`ride.chaotic_bots`, off by default; organisers decide in events). With it off, bots ride perfectly with autodrive avoidance (12D).
- **Q4 – Workouts:** keep the ERG step running while the rider is down (proposal), or pause the step until the rider rides again? *(As built in K2, v2.98: the proposal – the step keeps running – behind one constant, `ERG_STEP_RUNS_WHILE_DOWN`.)*
- **Q5 – Falls: decided (2026-11-14, dashboard):** **straight to a ragdoll with a physics engine** (no scripted fall first). The engine (Avian or Rapier – both MIT / Apache-2.0) is chosen by a short spike in K2; it runs only on the client for the visual fall of the rider and the bike (the crash itself, its timing and the time down stay server-authoritative, 12B / 12C), so riders see plausible but not identical falls. *Engine (K2 spike, v2.98): **Avian** (12I).*
- **Q6 – Persistence: decided (2026-11-14, dashboard):** injuries and bike damage **carry over between rides** – a career / repair-shop mode: bike damage stays until it is repaired in the workshop, paid in hearts (§14 Phase 4, 4E: `ceil(price × (1 − condition) × 0.3)` ♥), and injuries heal over real time and easy riding (12E recovery). With `ride.damage` off nothing accumulates; anonymous riders (Phase 10 Q5) never keep damage. This also unblocks Phase 4 W6.
- **Q7 – Gates and barriers on roads: decided (2026-11-14, dashboard):** gates, bollards and barriers on roads are **not shown by default** (and never block the routed road); a setting (`display.road_barriers`, Settings → Display) turns them on. When shown they are drawn but not solid on the routed road; off the road (wild mode) they are solid like other obstacles (12B).

### Phase 13 – God mode: flying through the world as a unicorn (planned 2026-11-13; G1 – G4 done, v2.61 – see 13I)

**Goal:** a **god mode** in which the rider leaves the road and flies freely and fast through the world, e.g. to inspect the map, look at a region before riding it, or check a world package. While in god mode the rider turns into a **unicorn**. Toggled by a hotkey; controlled like a flight mode: **+ / −** for altitude, the **mouse** to look around, the **cursor keys** to move in the horizontal plane.

#### 13A – Entering and leaving (decided by the user, 2026-11-13: hotkey, unicorn, fast flight)

- **Hotkey G** (free today; rebindable later with the other keys, §3.2.1 Controls) toggles god mode; console `god [on|off]`; gamepad: Select / Back + North (a chord, so it is not pressed by accident). The menu gets *Ride → God mode* with the same switch.
- **Entering:** a short transformation (0.6 s: a sparkle burst, the rider and bike fade out while the unicorn fades in and rises 3 m), the camera switches from the chase camera to the flight camera behind the unicorn, the mouse pointer is captured (hidden, relative motion).
- **Leaving (G again) – lands where the unicorn is (decided, Q2):** the unicorn glides down (at most 2 s) and turns back into the rider and bike **on the nearest road below it**, standing still: in free ride (§2.2.1) the nearest road of the graph (`--start`-like placement, the path starts there); on a fixed route the nearest point of the route (the rider continues along the route from there). The landing spot is marked under the unicorn while flying (a glowing ring on the road), so the rider sees where they will land. **Shift + G** instead flies back to where the rider left the road (a quick, eased flight, at most 2 s however far) and rides on from there. Esc in god mode first releases the pointer (a second Esc opens the menu, §3.2.1).
- **Who may use it:** single player on the local server: always. Shared sessions: the host decides (`ServerConfig.god_access`: anyone / host / nobody, like the world switch and the time machine, sent in `Welcome`); in a running **event** (race, group ride, time trial: from countdown to the rider's finish) god mode is **not available** – before and after it is. The kiosk / show mode never enters it by itself.

#### 13B – Flight controls

- **Touch** (where the device has one, §3.2.2; as built, v2.61): one-finger drag = look (as the mouse), two-finger drag = move in the plane (the fingers' centroid is a stick: full at 90 px from where they came down), spreading / pinching the two fingers = speed scale (as the wheel), on-screen **+ / −** buttons at the right edge (≥ 72 px, shown for 8 s after a touch) = altitude; a third finger cancels. (The proposed ‹ › ˄ ˅ pad was dropped: two fingers do it without covering the view.)
- **Flight model – like a plane (decided by the user, 2026-11-13, v2.67; replaces the strafing "drone" model of v2.51):** the unicorn always flies forward along its heading at a cruise speed (never hovers in flight; ↓ slows it to a slow glide, the minimum speed). **← / →** bank and turn: the unicorn rolls into the turn and turns like a plane in a coordinated turn (turn rate from the bank angle and the speed, `g · tan(bank) / v`), the camera banks with it; letting go levels out smoothly. **↑ / ↓** are the throttle: faster / slower along the heading, within the speed law below (Shift boost, Ctrl precise, wheel scale).
- **Look:** the mouse looks around the unicorn freely (yaw unlimited, pitch −89° … +89°; sensitivity and invert Y from Settings → Controls) **without changing the flight direction**; the view recentres behind the unicorn after a moment without mouse movement, or at once on a key. The direction of flight is the unicorn's heading, set by banking.
- **Keys only on the cursor block:** W / A / S / D are taken by riding (A autodrive, W wild mode; Q3).
- **Altitude (decided by the user, 2026-11-13): + makes the unicorn go up in the air, − makes it go down.** Held = continuous; the nose pitches up / down a little while climbing / descending and levels off when the key is let go; climb rate = half the speed. The keys are matched by the **character they type** (logical key: `+`, `-`, and `=` as + on US layouts), not by their position, so the main-keyboard + / − work on every layout (on a German keyboard + and − sit where a US keyboard has `]` and `/`; matching US positions made them dead there – bug found 2026-11-13); the numeric keypad + / − work too. The altitude stays at least **2 m above the terrain, water, buildings and trees** (also during a banked turn) and at most **8,000 m above sea level**.
- **Speed – much faster than riding:** the base speed grows with the height above ground, so low flight is controllable and high flight covers the map: `v = clamp(20 m/s + 0.5 × height above ground, 20, 400 m/s)` (72 km/h close to the ground, ~250 km/h at 100 m, ~790 km/h at 400 m, 1,440 km/h at 760 m and higher; with the boost and the wheel together at most **1,600 m/s** – 400 m/s × the boost; the wheel cannot go beyond it, so the tile streaming and the server's check can follow, v2.58). **Shift** = ×4 (boost), **Ctrl** = ×0.25 (precise); the **mouse wheel** scales the base speed (×0.25 … ×8, shown in the HUD). Climb rate = half the horizontal speed. Movements are smoothed (0.15 s acceleration, 0.25 s stop), so the flight is fluid at any speed.
- **Gamepad:** left stick ← / → = bank, ↑ / ↓ = throttle; right stick = look around; right / left trigger = climb / descend; A (South) = boost. **Touch:** one finger looks around, the +/− buttons climb and descend, bank and throttle by dragging on the left half (or on-screen buttons).
- **Map limits:** the unicorn can fly over the whole world set (§4.7: the home package and its neighbours), softly stopped 200 m outside the outermost package's bounds.

#### 13C – The unicorn

- **Model:** our own low-poly unicorn (made for OCW, licensed like the code – no third-party asset, §13; no brand characters): white body, a spiral golden horn, a rainbow mane and tail (the eight colours of the categorical palette, animated by the wind of the flight) and **large feathered wings like a pegasus** (decided, Q5): they flap with the climb and the acceleration (beat rate up to ~2 Hz), spread wide in gliding and turns (banking with the turn rate), and fold along the body while hovering and when landing. It **gallops in the air**: a gallop cycle whose rate follows the speed, legs tucked in at high speed, a gentle bob while hovering. A faint **rainbow trail** behind it (fading after 2 s; off with Settings → Display → Effects).
- **Rider on it?** The rider is not shown; the unicorn *is* the rider. **No saddle cloth (decided by the user, 2026-11-13: the pink/yellow square on its back is removed)**; the rider's colour shows only in the mane's first band, plus the name tag above it as for riders, so in multiplayer each unicorn stays recognisable.
- **Wings – a bird's wingbeat (decided by the user, 2026-11-14):** the large feathered pegasus wings beat **continuously while flying**, clearly visible from the chase camera: about 1.2–2 Hz, slower and deeper when slow or climbing, faster and shallower when fast, with short glides (wings spread) between bursts of flapping like a large bird. **Organic motion:** each wing has a shoulder and a wrist joint with a phase lag, so the outer wing and the primaries trail the inner wing; the downstroke is faster and stronger than the upstroke; on the upstroke the wing folds slightly and sweeps back, on the downstroke it is fully spread with the primaries fanned out; the body rises a little on each downstroke, the head and neck counter-move, the rainbow tail reacts. In a banked turn the inner wing folds slightly and the outer one stretches; landing ends with big braking flaps, then the wings fold. Each unicorn has its own phase (from the rider id), so several never flap in sync. Transitions are eased; the near and far models flap, the impostor beyond 500 m stays still.
- **Sound** (procedural, §3.3): a chime on transformation, wind rising with the speed (the existing wind voice), soft wing beats when the wings are open, a gallop of hooves on air (muffled) while slow.
- **LOD:** a 3k-triangle model near, 600 far, an impostor beyond 500 m – like the riders (§8).

#### 13D – Camera and inspection aids

- **Camera:** behind and slightly above the unicorn (8 m, looking along the view); **C** toggles a first-person view from the unicorn's head (the horn in view) – the pure "fly-through" for inspecting. Field of view widens with speed (+10° at 400 m/s). The camera never goes under the terrain.
- **Inspection HUD** (the bike computer is hidden; a compact flight panel instead): altitude above sea level and above ground, speed, base-speed factor, heading (compass), position (package metres x / z, latitude / longitude, tile id), the local time of the place (Phase 9). A crosshair in the middle: what it points at is named below it – the road (name, OSM way id, class, surface, width, bridge / tunnel, gradient), the area (land use, `LUSE`), building or water, and the elevation there. **I** toggles the panel; **P** puts a pin (a marker kept for the ride, `god.pins` lists them with coordinates, `--start X,Z` ready to copy).
- **World streaming:** the tile loader looks ahead along the flight direction with a radius that grows with altitude and speed (§4.7: neighbours load within 3 km of their region, extended to 3 km + 10 s of flight); at high altitude only the far terrain LOD and the colour map are drawn; trees and buildings appear below 1,500 m above ground. Frame time budget as in riding (§15); the tile loads keep their own thread.

#### 13E – Simulation and multiplayer (server-authoritative, §6.1)

- **Server state:** the rider enters the state `God` (additive field in `RiderUpdate`, §6.2). In this state the rider is **taken out of the physics**: no speed along the path, no drafting, no collisions (Phase 12), not in standings; the rider's road position is frozen where god mode began; on leaving, the server places the rider at the landing spot (after the plausibility check) or restores the old position (Shift + G). The unicorn's flight is **client-driven**: the client sends its 3D position and view at 10 Hz (`GodPose`), the server checks plausibility (speed ≤ the limits above, inside the world set) and broadcasts it to the others, who see the unicorn interpolated (§6.5). The server stays the truth for who is in god mode and where the rider re-enters.
- **Trainer:** SIM mode is set to a flat road (lowest resistance), ERG keeps its target (a workout continues; Q4). Power and cadence keep being shown, but they do not move the unicorn.
- **Ride recording:** ride time continues (the ride never pauses, §3.2.1); **distance and elevation are not credited** while flying, the FIT file gets a marker "god mode" at both ends (Strava sees a pause in movement, not a 1,000 km/h segment). Achievements and leaderboards (Phase 7) ignore god-mode time, and **no hearts** are earned while flying (§14 Phase 4, 4E).
- **Others (decided, Q1: visible):** riders see the unicorn flying over them with the rider's name and colours; no sound beyond 100 m. A host can see who is in god mode in the rider list.

#### 13F – Tests (spec §11.4)

- Pure flight model (`ocw-sim::god` or `apps/desktop/src/god.rs`, Bevy-free, wasm): speed law, boost / precise factors, wheel scale, acceleration and stopping, terrain clearance (never below 2 m over a test height field, also over a ridge at full speed), ceiling, world-set limits, moving relative to the yaw only (pitch does not dive), +/− climb rate.
- Input mapping: G, Shift + G, +/− (main and keypad), cursor keys, mouse motion, gamepad mapping; keys of riding (A, W, U, Q / E) do nothing in god mode.
- Server: `God` state excludes the rider from physics, drafting, collisions and standings; road position frozen and restored; plausibility check rejects too-fast poses; refused in running events and when `god_access` forbids it; distance not credited; FIT markers.
- Client: transformation and return, camera never under ground, crosshair inspection names the road under it (a test package); headless screenshots on X11 and Wayland of the unicorn in flight (scene in `scripts/scenes.toml`, also a short video of a flight over Liechtenstein for the build history).

#### 13G – Milestones (proposal)

- **G1 – Flight:** `god` command and G hotkey, flight model and controls, flight camera, pointer capture, terrain clearance, return to the road; the rider's model as a placeholder.
- **G2 – The unicorn:** model, gallop and wing animation, rainbow mane and trail, transformation effect, sounds.
- **G3 – Inspection:** flight panel, crosshair inspection, pins, first-person view, look-ahead tile streaming.
- **G4 – Multiplayer:** `God` state and `GodPose` on the server, access rules, others see the unicorn, recording rules.
- **G5 (later):** god mode in VR (Phase 11: head look instead of the mouse, comfort vignette at speed), a guided camera path for flyover videos (art project), god mode in the browser app (Phase 10).

#### 13H – Open questions (for the user)

- **Q1 – Visible to others: decided (2026-11-13):** yes, other riders see the unicorn with name and colours.
- **Q2 – Leaving: decided (2026-11-13):** land where the unicorn is (nearest road or route point below it); Shift + G flies back to where the rider left.
- **Q3 – Keys: decided (2026-11-14, dashboard):** **no WASD for flying** – the keyboard uses the cursor keys and + / − (as built, `WASD_FLIES = false`), and the **gamepad** is the other way to fly (left stick bank / throttle, triggers climb / descend, right stick look, 13B).
- **Q4 – Trainer while flying: decided (2026-11-14, dashboard):** **flat road** (lowest resistance; ERG keeps its target) – as built.
- **Q5 – Wings: decided (2026-11-13):** a classic unicorn with large feathered wings like a pegasus.
- **Q6 – Name in the UI: decided (2026-11-14, dashboard):** **"Unicorn mode"** in the menu, HUD, settings and messages (World → Unicorn mode); the console commands stay `god`, `god.*`.

#### 13I – Status (v2.58: G1 and G2 done, G4 in part; v2.61: G3 done, the gaps closed)

- **Built (G1):** `ocw_sim::god` (pure, wasm): the flight model `Flight` (speed law, factors, wheel scale, smoothing, the yaw-relative horizontal movement, the climb rate, the clearance sampled every metre along the step *and* the next step, so the straight line between two frames clears a ridge at full speed too, ceiling, soft stop), the glides of leaving, the landing spot on a polyline / a route of the road graph, the server's pose check, the wing beat and gallop rates. Client: `apps/desktop/src/god/` – `state.rs` (Bevy-free state machine in `ClientCore`: riding → asking → flying → leaving; poses at 10 Hz; the others' unicorns interpolated between snapshots), `input.rs` (pure mapping of G, Shift + G, the cursor keys, + / − on the main keyboard and the keypad, Shift / Ctrl, the wheel, mouse look with Settings → Controls → *invert Y*, the gamepad's sticks, triggers, South boost and the Select chords), `mod.rs` (Bevy glue, pointer capture, console `god`, `god.pose`, `god.cruise`, `god.view`, `god.trail`, `god.invert`). The riding keys (A, W, U, J, Q / E, the arrows' steering and power) and the chase camera's mouse orbit, wheel zoom and C reset are off while flying; Esc frees the pointer first, a click captures it again.
- **Built (G2):** `ocw_render::unicorn` – the model made in code (no asset file), its pure animation and the flight effects (sparkle burst in and out, the landing ring, the rainbow trail, name tags); procedural sounds (chime, wing beats on the downstrokes, the wind of the flight instead of tyres and drivetrain).
- **Built (G4):** the protocol (additive: `ClientMessage.god` / `.god_pose`, `ServerMessage.god`, `RiderUpdate.mode` / `.god`, `Welcome.god_access`), the session's `God` state (`Session::enter_god`, `set_god_pose`, `leave_god`), access rules (`ServerConfig.god_access`: anyone on the local servers, the host by default on `ocw-server`, `--god-control nobody` for organizers), others see the unicorn with name and colour; ride recording credits no distance while flying (nor the step to the landing spot), the ride time goes on; the trainer simulates a flat road in SIM (ERG unchanged). v2.61: the HUD's rider count says how many fly ("riders: 4 (1 flying)") and the console `ride.riders` lists the riders and who of them flies (there is no rider-list page yet; the menu's *Riders* page holds the local profiles); the FIT file gets an `event` message `user_marker` / `marker` at both ends of a flight (`data` 1 = god mode began, 2 = ended; `ocw_fit::Marker`), written in time among the records.
- **Built (G3, v2.61):**
  - `ocw_world::inspect` (pure, wasm): `raycast` (marches in steps of half the height above the surface, 0.5 … 20 m, so nothing 20 m wide is missed; bisection to the hit; 30 km at most) and `inspect`, which reads the package that draws the ground there: the road (a tile strip within half its width + 1 m, then the nearest graph edge, preferring the strip's class, for name, OSM way, class, width, bridge / tunnel, the profile's gradient signed by the looking direction, and the strip's `RSRF` surface), the building whose footprint covers the point (`BTAG` use, levels, roof shape; its height from the foot to the ridge), the `LUSE` water level and area, the elevation, the tile, latitude / longitude; `solid_top` / `water_level`. Tested against a test package (road, building, area, water, outside the world, a wall at any position).
  - Client (`apps/desktop/src/god/inspect.rs`): the crosshair's ray from the flight camera's eye along the view (first person: from the head) over terrain, buildings and water, 5 times a second; the panel (pure `panel_lines`: altitude ASL and above ground, speed, speed factor, heading, position x / z, latitude / longitude, tile and package, the local time with the UTC offset from the world clock, the pin count) – the big flight line keeps altitude, speed, factor and heading too; **I** toggles panel and crosshair (on by default; console `god.inspect [on|off]`, without an argument it prints what the crosshair names); **P** / `god.pin` puts a pin where the crosshair points (below the unicorn when it points at the sky), labelled with the first line of the inspection; `god.pins [list|clear]` lists them (`pin 1: x 1234 z -568, 455 m  47.16° N 9.54° E  Landstrasse · …  --start 1234,-568`). Pins are kept for the session (also after landing) and drawn as 60 m magenta beams with a "Pin N" tag (`ocw_render::inspect_hud`, which also draws the panel top right, the crosshair and its label).
  - First-person view (C, `god.view first`): checked – the eye at the head, the horn in view, never under the ground; the crosshair's ray starts there.
  - Look-ahead streaming (`ocw_render::flight_world`): the ring around the unicorn grows from 2 tiles by one per 600 m above the ground up to 3 (7 × 7 tiles); a 3 × 3 ring around the point 3 s ahead along the flight (at most 3 km); tiles are dropped only outside both (one ring of hysteresis); neighbour packages load within 3 km + 10 s of the horizontal speed (`WorldSet::plan_residency_reach`). Trees and houses (`TileDetail`) are hidden above 1,500 m over the ground; the terrain, its colour map and the far terrain stay.
  - Tree crowns in the clearance: each streamed tile's tree sites become a crown index (`TreeCrowns`: radius and tip of the drawn cone) in `FlightObstacles`, a layer per tile that only the flight uses – the camera still sees through trees (they fade). Route mode (no package) keeps its trees out of the clearance.
  - The unicorn's LOD (`unicorn::Lod`): the near model (≈ 2.9k triangles) to 80 m, a far model (≈ 670: the same parts on the same joints with fewer sides, four broad feathers per wing, no eyes) to 500 m, cross-faded over ± 10 m, then an impostor – a camera-facing card with a drawn unicorn (64 × 48 px, made in code), flipped to the flight direction and growing with the distance (≥ 2.6 m, 0.6 % of the distance) so a far rider stays visible.
  - Muffled hooves: below 12 m/s four soft, low-passed hoof beats per gallop stride (`god::hoof_beat_hz`, `Cue::Hoof`).
  - Touch (13B, as built): `apps/desktop/src/god/touch.rs` (pure mapping; the buttons' layout is shared with the renderer, `inspect_hud::touch_buttons`); a touch also suppresses the mouse look of that frame (Windows' emulated mouse). Not yet tried on a real touchscreen (none on the build machines).
- **Decisions while building:** the top speed is capped at 1,600 m/s (above); the take-off pose is 3 m above the rider, looking along the riding direction; the gamepad flies back with Select + West; the menu entry is *World → God mode* (the Ride page already has the seven items of the ten-foot rule); the trail switch is the setting `display.unicorn_trail` (console `god.trail`; the Display page is full too); the transformation hides the rider half-way through (0.3 s) instead of cross-fading the materials; open questions Q3, Q4 and Q6 are implemented as proposed behind constants (`god::input::WASD_FLIES`, `trainer::FLAT_WHILE_FLYING`, `god::MENU_NAME`). v2.61: the inspection is on by default; at high altitude the near tiles still stream (only trees and houses go), since the far terrain's 100 m mesh alone looks coarse under the unicorn.
- **As built (v2.82, to the 2.67 definition):**
  - Flight (`ocw_sim::god`, pure, wasm): `FlightModel` (`Plane` default, `Hover` = the v2.58 model behind the setting `controls.god_flight = plane|hover`, console `god.flight`); `Flight` keeps the throttle (15 … 100 % of the speed law's cap, 50 % at take-off, 60 % per second while ↑ / ↓ is held, never under 3 m/s), the bank (up to 55° at full ← / →, time constant 0.35 s; letting go levels out), the coordinated turn `coordinated_turn_rate` = 40 m/s² × tan(bank) / v (v taken as ≥ 10 m/s, at most 1.2 rad/s; ≈ 45 s for a full circle at 400 m/s – slow down for a tight turn) and its inverse `bank_for_turn`, the nose (follows the climb, ≤ 20°, the view takes half), the look-around (`look`, eased back after 1.5 s with 0.4 s, `recentre_look` on **L** or the **middle mouse button**), `view_yaw` / `view_pitch` / `camera_roll` (half the bank). The pose's yaw is the heading; the body's bank on every client is the coordinated one from the turn rate (`god::bank`).
  - Keys: `god::input::LayoutKeys` reads each `KeyboardInput`'s logical key – `+` / `=` climb, `-` / `_` descend – and keeps the role of that physical key for the session (Shift + the German + types `*` and still climbs once the key was pressed alone); the keypad + / − always. The other god-mode and riding keys are letters, arrows and function keys (positional, as before); the console's `` ` `` is the key below Esc (`^` on a German keyboard).
  - Gamepad: left stick x bank, y throttle; triggers climb / descend; right stick looks (eases back too). Touch (`god/touch.rs`): a finger on the left half is the stick (bank and throttle from where it came down, full at 90 px), on the right half it looks; two fingers on one half are the stick with the pinch for the speed scale; the + / − buttons as before.
  - Camera: `FlightView.roll` rolls the flight camera with half the bank (`camera::flight_up`); `camera.orbit` (console) also orbits the flight camera around the unicorn (`camera::flight_orbit_eye`; scenes and videos, e.g. the wings from the side); the crosshair's ray follows the view.
  - The unicorn (`ocw_render::unicorn`): no saddle cloth in any LOD; each wing is a shoulder (`Wing`), a wrist (`WingTip`, 0.78 m out) and three groups of primaries (`Primary`) on both the near and the far model (the impostor stays still); the pure wingbeat: `stroke` (a smoothstep downstroke over 40 % of the beat, the upstroke over 60 %), `wing_angles` (the arm 0.7 × depth, the hand 0.9 × depth 10 % of a beat later, the upstroke sweeping the arm back 0.22 rad and the hand 0.55 rad and closing the primaries, the inner wing tucked in a bank, reaching forward when braking), `god::wing_beat_hz` (1.3 Hz at 10 m/s … 2.1 Hz at 160 m/s, slower climbing, faster speeding up; none hovering or gliding down) and `god::wing_beat_depth` (deeper slow and climbing); flap-gliding while cruising level (3 … 5 beats, then 0.6 … 1.4 s with the wings spread; climbing, speeding up and landing flap on); the body rises 7 cm on the downstroke, the neck and head counter-move, the tail answers; landing: 0.9 s of deep braking flaps, then the wings fold. The phase and the rhythm come from the rider id (the same on every client, different riders out of step). The wing sound still beats at `wing_beat_hz` (also during the short glides).
  - Tests: the coordinated turn (rate against bank and speed, its inverse, in flight, levelling out), the throttle, climb / descend with the nose and the level-off, clearance in a banked descending turn over a ridge, the look-around, the model switch, + / − on US and German layouts (pure and through the Bevy system), gamepad and touch mapping, the camera's roll and orbit, the LOD paints (no saddle cloth); the wingbeat's asymmetry, the trailing hand, the fan, depth and rate by speed, the glides, determinism per rider, joint limits, the bob, the tucked inner wing, the braking flaps. Video: scene `god-plane-video` (a banked turn and a climb over Vaduz, then the wings from the side).
- **Not yet:** G5 (VR, a guided camera path, the browser app); the rider list as a menu page (the console and the HUD's count show who flies); the touch controls on real hardware.

### Phase 14 – Standalone VR: Quest, Pico and WebXR (planning, 2026-11-14)

**Goal:** ride Open Cycling World **in a standalone headset without a PC**: a native Android OpenXR app for Meta Quest 2 / 3 / 3S / Pro and Pico 4 / 4 Ultra, and **WebXR** in the browser app (Phase 10) for headsets with a WebXR browser. The rider sits on the trainer, the headset talks to the trainer, power meter and heart rate strap itself (or through a bridge), and every feature of Phases 10 / 11 is there: the same worlds, multiplayer, voice, workouts, the time and weather machines. **Decided (2026-11-14, Phase 11 Q5):** a standalone version and WebXR are wanted, as their own phase. Everything else below is a **proposal**.

It builds on Phase 11 (the VR mode of the desktop app: OpenXR, the first-person rig, the VR HUD, comfort, no head-lean steering, Phase 11 Q4) and Phase 10 (the browser app: full parity, WebGPU, WebTransport, Web Bluetooth, the sensor bridge, own hosting). Phase 14 adds the **Android target**, the **mobile-GPU performance tier** and the **standalone distribution**; it adds no new game rules.

#### 14A – Platform facts (as found 2026-11-14; re-check before starting)

**Headsets** (all Android-based, OpenXR, Vulkan on Qualcomm Adreno):

| Headset | SoC / RAM | Display rates | Notes |
|---|---|---|---|
| Meta Quest 2 | XR2 Gen 1, 6 GB | 72 / 90 / 120 Hz | grey passthrough; the weakest target (Meta's guidance: < 100 draw calls, < 750k triangles per frame) |
| Meta Quest 3 | XR2 Gen 2, 8 GB | 72 / 90 / 120 Hz | colour passthrough, depth sensor; GPU ≈ 2.5 × Quest 2 (< 200 draw calls, < 1.5M triangles) |
| Meta Quest 3S | XR2 Gen 2, 8 GB | 72 / 90 / 120 Hz | Quest 3 performance with lower-resolution Fresnel lenses; the cheapest current Quest |
| Meta Quest Pro | XR2+ Gen 1, 12 GB | 72 / 90 Hz | eye tracking (eye-tracked foveation); discontinued |
| Pico 4 | XR2 Gen 1, 8 GB | 72 / 90 Hz | colour passthrough (low resolution) |
| Pico 4 Ultra | XR2 Gen 2, 12 GB | 72 / 90 Hz | colour passthrough, depth; PICO OS 5.14 (OS 6 in developer early access) |

- **OpenXR is the only API on Quest:** Meta ended VrApi / the Oculus Mobile SDK on 31 Aug 2022; new Quest apps must use OpenXR ([Meta: OpenXR vs VrApi](https://developers.meta.com/vr/documentation/native/android/os-openxr-vrapi/), [Meta blog: all in on OpenXR](https://developers.meta.com/horizon/blog/oculus-all-in-on-openxr-deprecates-proprietary-apis/)). Quest uses the **standard Khronos OpenXR loader for Android** (`org.khronos.openxr:openxr_loader_for_android` ≥ 1.0.34, Horizon OS ≥ v62; no proprietary loader), launched by an activity with the intent category `org.khronos.openxr.intent.category.IMMERSIVE_HMD`, initialised via `XR_KHR_loader_init_android` / `XR_KHR_android_create_instance` ([Meta: Mobile OpenXR](https://developers.meta.com/horizon/documentation/native/android/mobile-openxr), updated 2025-01-23). **Pico** is OpenXR 1.1 compliant and takes the official Khronos SDK and loader since PICO OS 5.9 ([PICO developer news](https://developer.picoxr.com/news/oru5hv2u); [PICO 4 Ultra](https://developer.picoxr.com/pico4-ultra/)). So **one APK** with the Khronos loader can run on both, with vendor extensions (`XR_FB_*`, `XR_META_*`, `XR_BD_*` / `XR_PICO_*`) probed at runtime – the same approach as Godot's universal OpenXR APK ([UploadVR](https://www.uploadvr.com/godot-now-supports-more-xr-features-builds-a-universal-openxr-apk/)).
- **Rendering features:** all Quests support **multiview** on Vulkan (Vulkan 1.1 core; [Meta: multiview](https://developers.meta.com/horizon/documentation/unity/enable-multiview/)); fixed foveated rendering (`XR_FB_foveation`, Vulkan fragment density map; eye-tracked only on Quest Pro); refresh rate chosen per app (`XR_FB_display_refresh_rate`); the frame budget is 13.9 ms at 72 Hz, 11.1 ms at 90 Hz, 8.3 ms at 120 Hz, and **72 Hz is the store minimum** (VRC) ([Meta: device optimisation comparison](https://developers.meta.com/horizon/resources/device-optimization-comparison/)). CPU / GPU clock levels are set by the runtime with thermal throttling on long sessions – a cycling session of 60–90 min with the rider's body heat around the headset is the worst case; to measure (OVR Metrics Tool).
- **Rust / Bevy on Android:** Bevy runs on Android with `GameActivity` (feature `android-game-activity`) or `NativeActivity`; the native library is built with **`cargo-ndk`** (4.1.2) into a Gradle project; `cargo-apk` is deprecated and does not support `GameActivity`; `xbuild` (0.2.0, 2022) is barely maintained ([Bevy mobile example](https://github.com/bevyengine/bevy/tree/main/examples/mobile), [cargo-ndk](https://github.com/bbqsrc/cargo-ndk)). **`bevy_mod_openxr` 0.6.0 (2026-06-20) targets Bevy 0.19** with wgpu 29, openxr 0.21 and Android dependencies (`android-activity`, `ndk-context`) – newer than the 0.4 / Bevy 0.17 noted in 11A (crates.io, checked 2026-11-14); the starter template [`bevy_xr_app`](https://github.com/kcking/bevy_xr_app) builds for Quest with the Khronos loader. OCW is on Bevy 0.20.1 / wgpu 30, so the Phase 11 X1 choice (port of `bevy_mod_openxr` vs. our own layer) decides the Android path too. wgpu has no OpenXR or multiview-for-XR API of its own; the integrations create the Vulkan device through OpenXR (`XR_KHR_vulkan_enable2`) and wrap it with wgpu-hal.
- **APK and data size:** the store APK may be ≤ 1 GB, plus expansion / asset files of ≤ 4 GB each (≤ 2 GB recommended), uploaded with `ovr-platform-util` and delivered at install or as DLC ([Meta: publish APK](https://developers.meta.com/horizon/resources/publish-apk/), [asset files](https://developers.meta.com/horizon/documentation/unity/ps-assets/)). Our world packages (30–300 MB each, §4.7) are better **downloaded by the app itself** from our servers (Phase 10 Q4) into app-specific storage, verified and cached like on the desktop – no store asset files, no store update for a new world. Storage: Quest 3S from 128 GB, Quest 2 from 64 / 128 GB, shared with every other app.

**Bluetooth on the headset** (crucial for riding standalone):

- Horizon OS and PICO OS are Android, so the **Android BLE central APIs** (`BluetoothLeScanner`, `BluetoothGatt`) are there. **Direct BLE to FTMS trainers works on Quest in practice:** the fitness app HOLOFIT connects FTMS bikes, ellipticals and rowers and BLE cadence sensors directly to standalone Quest 2 / 3 / 3S / Pro, while its cadence sensor does *not* work with PC-tethered headsets ([HOLOFIT FAQ](https://www.holodia.com/?p=5719), [HOLOFIT compatibility](https://www.holodia.com/?p=4379)). Quest also pairs BLE heart rate straps at system level (Polar H10, Garmin HRM-Dual; Jan 2023, [Meta newsroom](https://about.fb.com/news/2023/01/track-fitness-stats-on-quest-heart-rate-monitoring-android-integration)) – that path feeds Meta's own overlay and Health Connect, not third-party apps.
- **Policy:** on the Horizon Store, `BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT` and `BLUETOOTH_ADVERTISE` are "permissions review required": **permitted only for interoperability with a third-party Bluetooth device that can't be otherwise paired through the Meta Quest app Bluetooth interface** ([Meta: permissions review](https://developers.meta.com/horizon/resources/permissions-review-required/), updated 2025-11-26). Trainers and power meters are such devices, so a store app must justify the permissions in its submission; location permissions for scanning were rejected in 2021 ([Meta forum, 2021–22](https://communityforums.atmeta.com/discussions/dev-quest/bluetooth-scan-and-permissions-ble-and-the-companiondevicemanager/871648)). The user must allow "Nearby devices" (Settings → Privacy) for the app.
- **Known problems (forum reports, 2021–2023, unverified on current OS):** scans through `CompanionDeviceManager` failed silently until Horizon OS v37; auto-reconnect after a drop failing on Quest 2; Bluetooth access reset to off by an OS update. So the **reconnect logic and a visible device state** (already in `ble::runner`) matter more than on the desktop; the headset's BLE radio is shared with the controllers, so the link budget to a trainer 1 m away should be fine but is to measure.
- **Pico:** no source found on third-party BLE GATT access from PICO OS apps; it is Android 10/12-based, so the same APIs are expected, but **to verify on a device** before Pico is promised for riding with a trainer.
- **btleplug** has an Android backend (JNI to the Android BLE APIs, with its `droidplug` Java part), so the existing `BtleplugBackend` may work on Android with a JNI bootstrap; otherwise a thin `AndroidBleBackend` implements `GattLink` (§3.4) over JNI. Either way the pure `ocw-devices` protocol state machine (`gatt`, `ble::protocol`) is unchanged.

**WebXR in headset browsers:**

- **Meta Quest Browser** (Chromium-based): WebXR `immersive-vr` and `immersive-ar` (passthrough, planes, anchors), hand tracking on by default since 15.1, **WebGPU** since 32.0 (March 2024), experimental foveation for WebGPU in WebXR in 150.1 ([Wikipedia: Meta Quest Browser](https://en.wikipedia.org/wiki/Meta_Quest_Browser), [Meta: browser 150.1](https://developers.meta.com/vr/downloads/package/browser/150.1)); `camera-access` refused in `immersive-ar` (Horizon OS v85, forum).
- **WebXR + WebGPU binding** (`XRGPUBinding`, WebGPU-backed projection layers): an **Immersive Web Editor's Draft (15 June 2026)**, "under development" ([W3C draft](https://immersive-web.github.io/WebXR-WebGPU-Binding/)); in Chrome behind flags ("WebXR/WebGPU Bindings", "WebXR Projection Layers"), no origin trial found; early-2026 reports that WebGPU-WebXR samples did not run on the Quest Browser ([Meta forum](https://communityforums.atmeta.com/discussions/WebXRDevelopment/webxr-webgpu-bindings-for-dynamic-gaussian-splatting/1255357), [X3D list, Feb 2026](https://web3d.org/pipermail/x3d-ecosystem_web3d.org/2026-February/000799.html)). **Consequence: WebXR today means WebGL2** (`XRWebGLLayer`, `OCULUS_multiview` / `OVR_multiview2`), i.e. the browser app's WebGL2 fallback (Phase 10) becomes the WebXR renderer until the binding ships; WebGPU in WebXR is a later switch.
- **Web Bluetooth in the Quest Browser: not available.** The API answers as if the headset had no Bluetooth (Meta forum, 20 Sep 2024, no answer from Meta; [thread](https://communityforums.atmeta.com/discussions/dev-quest/support-of-web-bluetooth-ble-in-quest-browser/1242380)); a sideloaded plain Chromium has working Web Bluetooth but poor WebXR (no Google services; [PiEEG tutorial](https://www.pieeg.com/tutorials/ble-quest-chromium-pieeg-xr), which also notes BLE is suspended when the window loses focus). No official Meta statement found. **So WebXR riding with a trainer needs the sensor bridge (Phase 10 Q2, 10D.2)** – the trainer stays connected to a phone, PC or the native app, and the samples reach the session through the backend.
- **Pico Browser:** WebXR VR works (older reports of entering WebXR failing, 2023); Wolvic (Igalia, open source) runs on Pico 4 with passthrough. No Web Bluetooth found. **Desktop Chrome / Edge with a PC headset:** WebXR via the OpenXR runtime (and Web Bluetooth works there, Phase 10A) – but that case is better served by the native desktop VR mode (Phase 11).
- **WebTransport** in the Quest Browser: expected from its Chromium base (Phase 10A), to verify.

**Distribution:**

- **Meta Horizon Store:** App Lab was merged into the store in August 2024 (complete 23 Aug 2024); former App Lab titles became regular or "Early Access" listings, still under the Virtual Reality Checks (VRC), content guidelines, data use and app policies ([UploadVR](https://www.uploadvr.com/quest-app-lab-merged-into-meta-horizon-store/), [Road to VR](https://roadtovr.com/meta-elevates-indie-app-lab-main-store/)). Free apps are allowed; Meta keeps 30 % of sales only. A developer account is free; publishing needs an organisation, developer verification and a review. Requirements relevant for us: a **live public privacy policy URL** explaining what is collected, how it is used and how to request deletion (VRC.Quest.Privacy.1–4; [Meta](https://developers.meta.com/horizon/resources/vrc-quest-privacy-1/)), a **Data Use Checkup** only if Platform SDK features are used (we use none: own accounts via OIDC, §9; [Meta](https://developers.meta.com/horizon/resources/publish-data-use/)), the Bluetooth permission justification (above), 72 Hz minimum, store assets (screenshots, trailer, ratings).
- **Sideloading:** developer mode on the headset (free developer account), `adb install` or SideQuest; no review. Suits the build machines, testers and early users.
- **Pico Store:** an agreement and a test before launch; PICO announced self-publishing and paid distribution for individual developers; "Pico Lab" (free apps only, lighter review) existed at the Pico 4 launch ([UploadVR: Pico Lab](https://www.uploadvr.com/pico-lab/), [PICO FAQ](https://sdk.picovr.com/docs/FAQ/chapter_twentyeight.html)); details to re-check in the PICO developer console.
- **Licences:** MIT / Apache-2.0 code is fine for both stores (§13); the store terms bind only our listing. The OSM attribution and ODbL offer (§13) appear in the app's credits and the store description. The Khronos loader is Apache-2.0. No commercial release is planned (Phase 8 Q2), so the app is **free**, with no in-app purchases (hearts are earned by riding only, Phase 4).

#### 14B – Architecture (proposal)

- **One codebase, a third target:** the same Bevy app and `ocw-render` as the desktop (Phase 11 VR mode) and the browser (Phase 10), built for `aarch64-linux-android` as **`apps/android`** – a thin crate (`cdylib` with the `android_main` entry, the Gradle project with the manifest, the Khronos loader AAR, icons) around the shared app library. The app library is split out of `apps/desktop` so both binaries (and `apps/web`) share it; desktop-only parts (the kiosk, ffmpeg captures, file dialogs, the window manager integration) are cfg-gated as for the web. The standalone app **always runs in VR** (no flat mode on the headset; a 2D panel app on Horizon OS is a possible later variant).
- **XR layer:** the Phase 11 integration chosen in X1 (Q3: both variants compared) gets an Android path: the loader init (`XR_KHR_loader_init_android`), Vulkan through `XR_KHR_vulkan_enable2`, multiview swapchains, `XR_FB_foveation`, `XR_FB_display_refresh_rate`, passthrough. X1's comparison should include a Quest build early, so the winner is not one that cannot run on Android.
- **Engine independence (§3.5, §10):** unchanged – Android only touches the app and the render plugin; `ocw-core`, `ocw-sim`, `ocw-protocol`, `ocw-world`, `ocw-sky` stay platform-free (they already build for wasm, so Android is no harder). The **server stays authoritative** (§6.1): single player runs the in-process `LocalServer` thread on the headset, as on the desktop.
- **WebXR:** the browser app (Phase 10, `apps/web`) gets an **XR mode** behind the same render plugin: `navigator.xr.requestSession('immersive-vr' | 'immersive-ar')` via `web-sys`, the WebGL2 path with `XRWebGLLayer` and multiview first, `XRGPUBinding` when it ships. Bevy has no WebXR backend; this is our own thin layer (to check whether `bevy_mod_xr`'s planned WebXR support has landed by then). The XR-specific logic (head pose composition, VR HUD layout, comfort) is the pure code of Phase 11, shared by OpenXR and WebXR.
- **World data on the device:** the app downloads world packages over HTTPS from our servers (Phase 10 Q4), verifies them (signatures, chunk hashes, §4.7) and keeps them in app storage (`Context.getExternalFilesDir` / `filesDir`), with a storage page in World (size per world, delete, "keep for offline"). Tile streaming from the file (§3.5, §4.5) keeps memory bounded. **Mobile world builds:** the world builder can emit a **mobile LOD set** per package (coarser far meshes, fewer trees and house details, compressed textures in ASTC / ETC2 instead of BC) as an optional package section or a sibling package – only added sections, older readers skip them (§4.7).
- **Multiplayer:** native **QUIC** (quinn, rustls with *ring*) runs on Android unchanged (§6.2); WebXR uses the **WebTransport** endpoint of Phase 10 (B3). Same sessions as desktop and browser riders.
- **Voice:** LiveKit has an Android SDK and the Rust SDK builds for Android (`ocw-voice` with `livekit`, §7); the headset microphone and spatial audio from the head pose (Phase 11 X5). WebXR: `livekit-client` (JS) as in the browser app.
- **Devices:** a BLE backend for Android behind `DeviceBackend` (§3.4) – btleplug's Android backend if it works with our runtime, else an `AndroidBleBackend` (`GattLink` over JNI) – with the existing pure protocol (FTMS, Cycling Power, CSC, Heart Rate, Wahoo control), ERG / SIM and wind (Phase 9 Q5). Runtime permissions (`BLUETOOTH_SCAN` with `neverForLocation`, `BLUETOOTH_CONNECT`), a device page usable in VR, and a robust reconnect (14A). **Fallback:** the **sensor bridge** (Phase 10 Q2) – the trainer is connected to the desktop app, the browser app on a phone or a future phone app, and the samples reach the headset's session through the backend. ANT+ stays deferred.
- **Accounts and settings:** OIDC sign-in (§9) through a device-code flow or a code shown in VR and confirmed on the phone / web companion (no typing passwords in VR); settings sync as for the browser app. Anonymous riding follows Phase 10 Q5 (plain bike, no customisation) if the user wants that for standalone too.
- **Captures (art project):** screenshots and videos from the left eye or a spectator view rendered at a lower rate, with the ISO timestamp burnt in (art project §2.1); `adb` pulls them into `test-output/` on the build machines. Meta's system casting / recording also works for demos.

#### 14C – Rendering and performance budget (proposal)

- **Targets:** **72 Hz minimum on Quest 2 / Pico 4** with the *Standalone low* preset, **90 Hz on Quest 3 / 3S / Pico 4 Ultra** with *Standalone*, 120 Hz optional; never under budget for more than a frame (reprojection – Meta's Application SpaceWarp – as a safety net, not the plan). Per-frame budgets from Meta's guidance: ≤ 100 draw calls and ≤ 750k triangles on Quest 2, ≤ 200 / ≤ 1.5M on Quest 3 (14A); the tile and tree merging of §3.1.1 / §4.5 already works this way.
- **A standalone quality tier** (new presets beside *VR* of Phase 11): view distance 2–4 km near terrain and 20–40 km far terrain (instead of 120 km), trees as merged clusters and impostor cards beyond 150 m, houses as the mid / far LOD early (Phase 8 T4 levels), no real-time shadows on Quest 2 (one cascade on Quest 3), **baked lighting where sensible** (ambient occlusion and sky light baked into terrain and building vertex colours by the world builder's mobile LOD set; the sun direction from the world clock still lights them in a cheap forward pass), the sky and atmosphere (Phase 9) as a precomputed lookup, weather particles reduced, MSAA 4× (tile-based GPUs resolve it nearly free), no post-processing beyond tone mapping, ASTC textures.
- **Multiview** single-pass stereo and **fixed foveated rendering** (high level on Quest 2) from the start; dynamic resolution as a second lever; GPU-driven culling where it helps the CPU (draw calls are the Quest 2 bottleneck).
- **CPU:** the 50 Hz physics of the local server is cheap; the world streaming and mesh building run on worker threads with a lower priority so the render thread keeps its core; no shader compilation during a ride (pipelines warmed at load).
- **Thermals and battery:** measure a 60 min ride (OVR Metrics Tool: CPU / GPU level, temperature, battery); the target is a full 60 min ride on battery on Quest 3 with *Standalone*, and an "endurance" preset that drops to 72 Hz and lowers the resolution when the runtime reports throttling. A headset fan accessory or a charge cable is advised for longer rides (documentation, not code).
- **Memory:** ≤ 3 GB on Quest 2 (6 GB shared with the OS), ≤ 4 GB on Quest 3 / Pico.
- **App size:** APK ≤ 200 MB (the app, shaders, sounds, default bikes and characters); no world inside – the first start offers the test route (in the APK, a few MB) and downloads a world.

#### 14D – Devices and input (proposal)

- **Sensors** as in 14B: direct BLE on the headset (trainer, power meter, cadence, heart rate) as the main way, the sensor bridge as the fallback, virtual power for spectators and tests.
- **Controllers for menus:** the controller ray is a pointer of the input model (§3.2.2) on the world-space panels (Phase 11 X3); the thumbsticks and buttons map to the gamepad actions (steering keys, U-turn, junction choice, menu), so riding needs no new bindings – steering stays on keys / gamepad / autodrive (Phase 11 Q4: no head-lean). A controller can rest in a handlebar mount; a BLE gamepad or handlebar remote works as on the desktop.
- **Hand tracking** for menus (pinch to select, `XR_EXT_hand_tracking`; WebXR hand input), useful because hands are on the bars and controllers get put down; never needed to ride.
- **Passthrough / mixed reality** (comfort and safety option): Quest 3 / 3S colour passthrough and Pico 4 / 4 Ultra passthrough through OpenXR (`XR_FB_passthrough` on Quest; the Pico equivalent to check) and WebXR `immersive-ar`. Modes: *off* (full VR), a **passthrough window around the handlebar and the floor** (see the trainer, the bottle, the towel and the real hands), **full passthrough while the menu is open or when stopped**, and a "look down to see the room" fade. On Quest 2 the grey passthrough is only a safety view. A guardian / boundary for a stationary seated user (Meta's stationary boundary) is enough; the app declares a seated, stationary experience.
- **Comfort** (the Phase 11 rules, which suit standalone riding): fixed horizon (no camera roll), no artificial rotation of the view outside the road's own turns, the comfort vignette on sharp turns and descents, fades on teleports / ride switches / U-turns, a visible cockpit; **seated** riding with a height calibration; recentre on a button.
- **Text input:** the system keyboard of Horizon OS / PICO OS for names and the console; the console's Dev commands stay limited to the local server as everywhere.

#### 14E – Distribution (proposal)

- **Stages:** (1) **sideloaded APKs** for our own tests and testers (developer mode, `adb`, a download page on our servers, release notes); (2) the **Meta Horizon Store** as a free app, first as an Early Access listing; (3) the **Pico Store** once Pico riding is verified (14A).
- **What the stores need from us:** an organisation and verified developer account, a privacy policy on our own website (what the server stores: account, rides, chat / voice routing, no sale of data; how to delete; §12), the Bluetooth permission justification, the content rating questionnaire, store art and a trailer (from the art project's videos), the OSM / data credits (§13).
- **Updates:** store updates for the app; worlds come from our servers (14B), so new worlds need no store review.
- **WebXR** needs no store: the browser app's URL (Phase 10) works in the Quest and Pico browsers; a Horizon Store listing of the web app (a PWA wrapper) is possible later.

#### 14F – Tests (spec §11.4, proposal)

- **Unit tests (no headset):** the Android BLE `GattLink` against the protocol mocks (as for Web Bluetooth); reconnect and permission-denied state handling; the standalone preset selection per device (from the OpenXR system name and the GPU); the world storage manager (download, verify, quota, eviction, resume); the mobile LOD set writer and reader in the world builder (byte-identical rebuilds, old readers skip it); the passthrough mode state machine; the device-code sign-in flow; the WebXR session state machine (pure, wasm-tested).
- **Build checks:** `cargo ndk -t arm64-v8a build` for the app in CI (on marge; Linux only), the APK assembled by Gradle, `cargo check` for `aarch64-linux-android` of every crate the app uses; the wasm check (§3.5) stays green.
- **Integration:** the OpenXR frame loop against Monado's simulated device (Phase 11 X1, native), the Android build on an emulator for start-up only (no XR); on a real headset over `adb` (when one is connected to a build machine): start, enter a world, a 60 s ride with the virtual trainer, a screenshot and the OVR Metrics frame-time log into `test-output/`.
- **Hardware in the loop:** the headset connected to the Wahoo KICKR next to marge (§3.4) – scan, connect, ERG / SIM, reconnect after a trainer power cycle.
- **WebXR:** headless Chromium with the WebXR emulation (Immersive Web Emulator / `navigator.xr` mock) for session start and a frame; the Quest Browser by hand.
- **Performance gates:** frame time on Quest 2 and Quest 3 per release on the standard scenes (`scripts/scenes.toml`), with the 72 / 90 Hz budgets as pass / fail.

#### 14G – Milestones (proposal)

- **V1 – Android build:** the shared app library split out of `apps/desktop`; `apps/android` builds with `cargo-ndk` + Gradle; a flat Bevy scene starts on a Quest (sideloaded). CI builds the APK.
- **V2 – OpenXR on Quest:** the X1 integration (Phase 11) on Android with the Khronos loader: stereo test route with head tracking, multiview, foveation, refresh rate; frame time measured on Quest 2 / 3.
- **V3 – Standalone tier:** presets, the mobile LOD set from the world builder, ASTC textures, baked lighting, world download and storage on the device; Liechtenstein at 72 Hz on Quest 2 and 90 Hz on Quest 3.
- **V4 – Sensors on the headset:** the Android BLE backend with FTMS, Cycling Power, CSC, Heart Rate and Wahoo control, ERG / SIM, permissions, reconnect; hardware-in-the-loop with the KICKR; the sensor bridge as the fallback.
- **V5 – Riding parity:** VR HUD, menus with controllers and hands, comfort, passthrough modes, multiplayer over QUIC, voice (LiveKit), sign-in, captures.
- **V6 – Pico:** the same APK on Pico 4 / 4 Ultra with vendor extensions probed; BLE verified; Pico Store submission.
- **V7 – Store release:** privacy policy, permission justification, store assets; Meta Horizon Store Early Access, then a full listing.
- **V8 – WebXR:** the browser app's XR mode (WebGL2 + multiview first), controllers and hands, passthrough via `immersive-ar`, the sensor bridge for trainers; WebGPU via `XRGPUBinding` when browsers ship it.
- **V9 – Thermals and endurance:** the 60 min ride measurements, the endurance preset, battery and fan guidance.

#### 14H – Open questions (for the user)

- **Q1 – First headset:** **Quest 3 / 3S** first (proposal: the most common current standalone headset, colour passthrough, 2.5 × the Quest 2 GPU), with Quest 2 as the minimum and Pico after it? A development headset would have to be bought (the Rift CV1 cannot run standalone apps).
- **Q2 – Native or WebXR first:** native Android OpenXR first (proposal: full performance, direct BLE works on Quest, WebGPU-in-WebXR is not shipped), or WebXR first (no store, reuses the browser app, but WebGL2 only and trainers only through the sensor bridge)?
- **Q3 – Distribution:** sideloading for testers first and then a free Meta Horizon Store listing (proposal), or sideloading only (no store review, no organisation account, no privacy-policy obligations towards Meta)? Pico Store later or not at all?
- **Q4 – BLE fallback:** if direct BLE on a headset turns out limited (Pico unverified; Quest reconnects reported flaky), is the **sensor bridge** through a PC or phone acceptable as the official way – and should that include a small **phone relay app** (Android first; iOS would be the first native iOS code, §1.3), or only the desktop and the browser app as bridges (Phase 10 Q2)?
- **Q5 – Priority:** after Phase 11 X1 – X2 (proposal: the VR mode on the desktop first, then V1 – V2 while the browser app reaches B3), or later after the browser app (Phase 10) is done?


---

## 15. Quality Goals

| Topic | Target |
|---|---|
| Frame rate | 60 FPS on suitable desktop hardware (measured on the wall clock: FPS counter, `--log-fps`, §3.2) |
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
