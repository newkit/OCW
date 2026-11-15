# OCW build diary – day 4, 2026-11-14

155 commits (47 merges), +151071 / −14750 lines, 535 images, 39 videos, 2 sounds.

## Commits

| Time | Commit | |
|---|---|---|
| 00:01 | `427d41a` | Spec 2.67: god mode refined – plane-like flight, + up / − down by typed character, no saddle cloth, bird-like wingbeat |
| 00:02 | `6f7268e` | Spec: 2.67 row at the top of the newest rows |
| 00:02 | `f8392c9` (merge) | Merge main (spec 2.67: refined god mode) |
| 00:03 | `27b9fd3` | CLAUDE.md: Win11Builder resumed for Windows-related builds |
| 00:09 | `03eaed0` | Spec 2.68: Phase 4 decisions from the build dashboard – changes on leaving the workshop, bounded parts with Profile mode, hearts without daily cap, kit costs hearts (glasses free), touch testing postponed |
| 00:10 | `2d9dbf9` | CLAUDE.md: read and watch the dashboard's answer inbox, record answers in the spec |
| 00:10 | `2d35dc4` | build-dashboard: enable/disable build machines and set their build slots (spec 2.66) |
| 00:11 | `18500b8` | Unicorn: a bird's wingbeat; touch steers on the left half; camera.orbit in flight (spec 2.68) |
| 00:11 | `fc8c397` (merge) | Merge build dashboard machine controls: enable/disable machines for building, slots per machine; remote.sh refuses disabled machines (deploys still allowed) (spec 2.66) |
| 00:13 | `8e3ba43` | Spec 2.69: dashboard answers – time/weather control host-only in shared sessions; 3D building data postponed |
| 00:14 | `888a33a` | Spec 2.70: night keeps a minimum brightness; bike lights are bought and fitted in the workshop |
| 00:14 | `1efbad1` | Spec 2.71: wind load sent to trainers that support it |
| 00:15 | `1c97a5e` | Spec 2.72: weather providers MeteoSwiss + DWD, MET Norway fallback, Open-Meteo; selectable in the menu |
| 00:18 | `c5bd97d` | Spec 2.73: historic weather kept – ERA5 under CC BY 4.0 / Copernicus licence fits rule 8A |
| 00:18 | `919f7cb` | Spec: 2.73 row in place |
| 00:19 | `cd44fdc` | Spec 2.74: browser targets – Chromium for trainer rides, others with virtual power and the bridge |
| 00:19 | `e68d49c` | Spec 2.75: build the sensor bridge for browser rides |
| 00:20 | `ff26081` | Spec 2.76: browser app starts now (B1 first) |
| 00:21 | `03650f5` | Spec 2.77: browser app hosted on own servers for now |
| 00:22 | `2e100e4` | Spec 2.78: anonymous riding allowed with a plain bike and character; customization for logged-in users only |
| 00:22 | `5c520a8` | Spec 2.79: browser app with full parity |
| 00:22 | `bb98a02` | Spec 2.80: Linux VR is a target |
| 00:23 | `8de74cc` | Spec 2.81: VR spike builds and compares both OpenXR integrations |
| 00:24 | `b40b1a9` (merge) | Merge god mode refined: plane-like flight, + / − by typed character, no saddle cloth, bird-like wingbeat (spec 2.67) |
| 00:24 | `fd87152` | Spec 2.83: VR steering stays on keys, gamepad and autodrive (no head-lean) |
| 00:25 | `0fcf78f` | Spec 2.84: standalone VR (Quest / Pico) and WebXR wanted – new phase |
| 00:25 | `1adf7a8` | Spec 2.85: VR at the end of the phase order |
| 00:25 | `1120509` | Spec 2.86: collisions on, damage off by default |
| 00:26 | `01f90ad` | Spec 2.87: star catalogue licence research for Phase 9 Q7 (proposal, still open) |
| 00:27 | `c58c411` | Spec 2.88: group rides soft bumps only, races full collision model |
| 00:28 | `20fb1cb` | Spec 2.89: chaotic bots as a setting |
| 00:29 | `1bef6f6` | Spec 2.90: falls as ragdolls with a physics engine |
| 00:29 | `44c0f4a` | Spec 2.91: Phase 14 planned – standalone VR (Quest / Pico native OpenXR) and WebXR |
| 00:29 | `74f3837` | Spec 2.91: persistent damage with repairs in the workshop |
| 00:30 | `5237cd8` | Spec: persistence decision renumbered to 2.92 (2.91 is Phase 14) |
| 00:31 | `19a2d24` | Spec 2.93: road gates and barriers hidden by default, a setting shows them |
| 00:31 | `f1eb921` | remote.sh: Windows sync sends the whole tree to a new checkout |
| 00:32 | `5329910` | Spec 2.94: Windows tree sync sends the whole tree to a new checkout |
| 00:47 | `78acea9` | Unicorn mode: the UI name of god mode (spec 2.94, Phase 13 Q6); trainer flat while flying (Q4) |
| 00:48 | `94e5590` | Release 0.2.1 |
| 01:04 | `5db3ec2` | remote.sh: a new Windows checkout gets every file (empty manifest sent none) |
| 01:32 | `c1ad780` | Spec 2.80: Phase 10 B1 – the browser app on WebGPU with the local server in the frame loop |
| 01:34 | `0421d31` | Voice echo test: an Echo rider plays your microphone back, delayed, when you are near it (spec 2.95, §7 Echo bot) |
| 01:42 | `b74653a` (merge) | Merge Windows sync fix: new checkouts get the whole tree, manifest bound to its checkout, safety net for missing files (spec 2.95) |
| 01:42 | `1a347ba` | Spec: header 2.95 |
| 01:43 | `55429b2` (merge) | Merge voice echo bot: Echo rider, delayed spatial playback of your voice, Settings → Audio → Voice, voice.echo console, LiveKit echo participant (spec 2.96) |
| 01:53 | `0b2af39` | Phase 8 T7: orthophoto corridor along the roads (spec 2.65) |
| 01:58 | `8df2552` (merge) | Merge origin/main (voice echo bot, spec 2.96) into the browser app B1; B1 is spec 2.97 |
| 01:58 | `b7f3681` | Cargo.lock: ocw-client and ocw-web at the workspace version 0.2.1 |
| 01:58 | `483417b` (merge) | Merge Phase 10 B1: the browser app – shared ocw-client crate, apps/web on WebGPU, frame-driven local server, make web / web-serve / test-web (spec 2.97) |
| 02:27 | `391be91` | Phase 8 T7: roofs masked, sky-lit shadows take the local ground colour; final sizes and frame times (spec 2.65) |
| 03:06 | `e626c0a` | Phase 12 K1 + K2: rider collisions, soft bumps, crashes and ragdoll falls with Avian (spec 2.98) |
| 03:24 | `fe448c1` (merge) | Merge Phase 12 K1+K2: rider collisions, soft bumps, crashes with Avian ragdoll falls, autodrive avoidance (spec 2.98) |
| 03:36 | `d78f76d` | Kiosk show mode without collisions (spec 12G) |
| 04:05 | `55fb43f` | ocw-assets: generated workshop room and layout (spec 14 Phase 4, 4A) |
| 04:07 | `abd5201` | Parts catalogue: assets/parts.toml, price sources, validator and reprice (spec 14 Phase 4, 4H-4J) |
| 04:16 | `3b5b438` | ocw-assets: workshop hotspot anchors and part variants in the bike generator (spec 14 Phase 4, 4B/4I, W2) |
| 04:58 | `95366fe` | Phase 4 W1 + W2: the workshop scene, parts and hearts (spec 2.99, 14 Phase 4 4M) |
| 05:03 | `31dd314` | Phase 10 B2: worlds over HTTP in the browser app – streamed by chunk, verified, cached in OPFS, resident tiles within a memory budget (spec 3.00) |
| 05:04 | `ebd9507` (merge) | Merge Phase 10 B2: worlds over HTTP in the browser – range-request streaming, SHA-256 + signature verification, OPFS cache, loading screen, memory budget (spec 2.99) |
| 05:04 | `fa2723f` | Spec: B2 is version 2.99 |
| 05:08 | `3d07c8c` | Workshop scenes: later shots for slow software rendering (X11) |
| 05:24 | `4766822` (merge) | Merge Phase 4 W1+W2: the 3D workshop – bikes on the stand and the wall, part hotspots and picker, 76-part catalogue in hearts, local wallet, server-credited hearts, BikeChange (spec 3.00) |
| 06:21 | `b24cf1d` | Phase 10 B3: multiplayer in the browser – WebTransport endpoint next to native QUIC, browser riders in the same sessions (spec 3.01) |
| 06:39 | `05b8bc8` | Spec 3.01: the build dashboard defined as a whole (§11.2.1) |
| 06:39 | `c64b855` | Spec §11.2.1: wording |
| 06:40 | `e55d1a8` (merge) | Merge Phase 10 B3: browser multiplayer over WebTransport (web-transport-quinn on our quinn endpoint), anonymous browser riders (spec 3.02) |
| 06:46 | `0e575d1` | Phase 4 W3: build physics on the server – class steps and build caps, BikeChange applies parts, Profile / Open / Equal race modes, effect lines on the cards, others see your parts (spec 3.03) |
| 06:46 | `5f80016` | Spec §2.6 / 4C: character editor wording follows the Phase 4 Q4 and Phase 10 Q5 decisions |
| 06:47 | `235a725` (merge) | Merge Phase 4 W3: parts physics within the caps, Profile / Equal / Open race modes, others see fitted parts (spec 3.03) |
| 06:52 | `739bf9f` | Spec 3.04: kit gives a slight benefit with a benefit score per item (TT helmet faster), within the build caps |
| 06:59 | `fc990eb` | Spec 3.05: aero effects depend on speed – investigated thresholds and magnitudes, scores by speed, caps at three reference powers |
| 07:06 | `17b34c9` | Spec 3.06: no WASD in Unicorn mode, the gamepad instead |
| 07:14 | `f1f6dab` | Spec: version table in one numerical order, newest first |
| 07:58 | `ba70dad` (merge) | Merge origin/main into the T7 orthophoto corridor branch (spec 3.07) |
| 08:22 | `9535449` | Phase 4 W4: the character corner – mirror editor with a planar reflection, free character, kit with hearts and small speed-dependent benefits, try on the bike (spec 3.07) |
| 08:26 | `d3f5168` (merge) | Merge Phase 4 W4: the character corner – mirror editor, free character, kit for hearts with speed-dependent benefits, try on the bike (spec 3.07) |
| 09:08 | `356cdf1` | Phase 9 S2: the night sky without stars – moon, planets, Milky Way, street lamps, light pollution (spec 3.08) |
| 09:14 | `6f4ba07` | Spec: the shipped packages with the orthophoto corridor (sizes after the rebuild on marge) |
| 09:14 | `996ea07` (merge) | Merge origin/main (W4, spec 3.07): the T7 row becomes 3.08 |
| 09:23 | `44c4fd8` (merge) | Merge Phase 9 S2: the night sky without stars – moon (ELP), planets, Milky Way, moonlight, street lamps (LITR), light pollution, bike lights follow the fitted part (spec 3.08) |
| 09:28 | `4c83f3d` (merge) | Merge local main (Phase 9 S2, LITR, street lamps, spec 3.08): T7 becomes 3.09; ORTC and LITR both optional, streaming skips ORTC and keeps LITR |
| 09:28 | `0dedf22` | T7 tests: road strips with the LITR field |
| 09:44 | `fce62bc` | Phase 10 B4: devices in the browser – Web Bluetooth with the shared protocol, SIM / ERG, the real KICKR ridden from headless Chromium (spec 3.09) |
| 09:47 | `a7d82cb` (merge) | Merge Phase 10 B4: devices in the browser – Web Bluetooth GattLink on the shared BLE protocol, reconnecting supervisor, SIM from the build in force (spec 3.09) |
| 09:47 | `43e797f` (merge) | Merge local main (Phase 10 B4, spec 3.09): the T7 row becomes 3.10 |
| 10:03 | `947ea5d` | Spec: shipped package sizes with ORTC and LITR |
| 10:26 | `2625e71` (merge) | Merge Phase 8 T7: orthophoto corridor (ORTC), on by default in every world package; packages rebuilt with ORTC and LITR (spec 3.10) |
| 10:58 | `6472c43` | Phase 9 S4: weather simulation and rendering – clouds, rain / snow / hail, fog, wet roads, snow cover, wind in the trees, lightning and thunder (spec 3.11) |
| 10:59 | `f53b3b0` (merge) | Merge main (Phase 8 T7, Phase 10 B4, spec 3.10) into Phase 9 S4: the S4 row becomes 3.11 |
| 11:10 | `85e4ed0` (merge) | Merge Phase 9 S4: weather simulation and rendering – ocw-weather crate, session weather on the server, clouds, rain/snow/hail, fog, wet roads, snow cover, wind, lightning and thunder, weather sounds (spec 3.11) |
| 11:36 | `77631a2` | Phase 8 T6: riders and bikes – real materials (bindless gear shader, CC0 knit / carbon / leather), jersey designer with patterns, colours, race numbers, kit colours and finishes (spec 3.12) |
| 11:38 | `9c12aac` (merge) | Merge Phase 8 T6: rider and bike materials (carbon, spun metal, tyres, fabrics), jersey designer and kit colours (spec 3.12) |
| 12:07 | `5b0617e` | Phase 9 S5: the weather machine – World -> Weather with mode, presets as an icon grid, custom editor, transitions, random weather; event weather set by the organiser; console weather.list / weather.status (spec 3.13) |
| 12:25 | `1249f33` | Phase 12 K3: world objects – OBST section, wild-mode collisions with trees, buildings, walls and barriers, railings, tunnel walls, hard road-edge hits (spec 3.13) |
| 12:28 | `0b17e07` (merge) | Merge Phase 9 S5: the weather machine – World → Weather with presets grid, custom editor, transitions, random weather; event weather set by organisers (spec 3.13) |
| 12:29 | `83bcc3c` | Spec: remove conflict markers left by the T6 and S5 merges (header, version table, T6/T7 status) |
| 12:29 | `ef7c7ea` | CLAUDE.md: check for leftover conflict markers before committing a merge |
| 12:34 | `1d8fadb` (merge) | Merge Phase 12 K3: world objects – OBST section, wild-mode collisions with trees, fences, walls, railings and tunnel walls, road barriers hidden by default (spec 3.14) |
| 13:06 | `d0b6b2f` | Open items of Phase 4 W2 / 4M, Phase 8 T5 / T6, Phase 9 9I-S2: groupset fitting, console over the workshop, near-black carbon, brushed metal, hip number plates, warm lit windows, kerbs in wild mode, round sidewalk corners (spec 3.15) |
| 13:17 | `0d62f45` (merge) | Merge polish bundle: groupset fits its crankset/cassette/chain, console over the workshop, near-black carbon, hip number plates, brushed vs spun metal, warm varied lit windows, kerbs in wild mode, round sidewalk corners (spec 3.15) |
| 13:53 | `eb94eb4` | Phase 9 S7: real weather – MeteoSwiss, DWD, MET Norway and Open-Meteo via the game server (region grid, cache, fallback, offline), ERA5 history ("ride a day"), World -> Weather -> Real weather with data source and attribution, HUD credit; deterministic a_finished_event_unlocks_the_weather (spec 3.16) |
| 13:55 | `c699d17` (merge) | Merge Phase 9 S7: real weather – MeteoSwiss, DWD, MET Norway, Open-Meteo via the game server, ERA5 history (ride a day), Real weather page with attribution (spec 3.16) |
| 14:29 | `5f3769c` | Phase 12 K4 + K5: damage, the damage indicator and visible damage; Phase 4 W6: repairs in the workshop (spec 3.17) |
| 14:32 | `2551b8f` | Phase 12 K4 / W6: the bike change carries the garage's rider condition too, so a staged or kept injury is not overwritten by the ride's (spec 3.17) |
| 14:51 | `39fbdf2` (merge) | Merge main (3.16, S7 real weather) into Phase 12 K4 + K5 / Phase 4 W6 |
| 14:51 | `e107920` (merge) | Merge Phase 12 K4 + K5 and Phase 4 W6 repairs: damage model, damage indicator and visible damage, persistent part conditions, repairs in hearts; crash notice on its own box (spec 3.17) |
| 15:21 | `1299cd6` | Phase 9 S8: seasons – degree-day phenology by date and altitude, crop calendar, climatological snow line (spec 3.17) |
| 15:22 | `68340e6` (merge) | Merge Phase 9 S8: seasons – phenology by degree days and altitude, deciduous trees and larches through the year, crop calendar, climatological snow line, real snow depth (spec 3.18) |
| 15:22 | `e9ffc2f` | WIP Phase 9 S6: weather physics |
| 15:24 | `e164d9e` (merge) | Merge main into the S6 branch |
| 15:44 | `d567596` | Phase 9 S6: weather physics – wind, air density and wet roads in the server's physics and the client's prediction; per-session switch and the organiser's event rule; trainer wind (FTMS, Wahoo, folded gradient for inclination-only trainers, virtual trainer); HUD wind indicator (spec 3.19) |
| 15:44 | `baa2ba7` (merge) | Merge Phase 9 S6: weather physics – headwind along the direction of travel, air density, wet-road rolling resistance and grip, crosswind drafting, tunnel shelter, trainer wind (FTMS / Wahoo) and folded gradient, HUD wind (spec 3.19) |
| 16:35 | `c49800e` | Phase 8 T3 complete: forest type and density from the Copernicus HRL (spec 3.20) |
| 16:36 | `eb17602` (merge) | Merge Phase 8 T3: forest type and density from the Copernicus HRL (Dominant Leaf Type, Tree Cover Density 2018) – section FRST, measured leaf type, density-driven tree placement and far canopy (spec 3.20) |
| 16:56 | `108e041` | Phase 10 B5: audio and voice in the browser – shared ocw-audio mixer through Web Audio, LiveKit's JS SDK behind the shared voice controller (spec 3.21) |
| 17:09 | `f8698e5` (merge) | Merge main (3.20) into Phase 10 B5: weather.physics moved into the shared registry (ocw_client::settings), spec 3.21 on top of 3.20 |
| 17:10 | `284441f` (merge) | Merge Phase 10 B5: audio and voice in the browser – shared ocw-audio mixer, Web Audio scheduling with the autoplay gate, LiveKit JS voice behind ocw_voice::web, shared settings registry, Sound & voice panel (spec 3.21) |
| 17:10 | `1a1a5d8` | CLAUDE.md: code map for ocw-audio and ocw_client audio / voice / settings (B5) |
| 17:49 | `9d87780` | Build cleanup on the Mac: no zsh globs in remote snippets |
| 18:32 | `b516e1f` | Phase 12 / 13 leftovers: wet roads in crashes, the camera's shake, the aids as graphics, guided camera paths, the rider list page (spec 3.22) |
| 18:34 | `a044af8` | Phase 4 leftovers: the bench catalogue, fitted lights in the ride, idle pose, kit colours, trainer SIM tests (spec 3.23) |
| 18:34 | `8ff4f87` (merge) | Merge main into the Phase 4 leftovers branch |
| 18:35 | `74151f9` (merge) | Merge Phase 4 leftovers: bench catalogue (4D) with the + hook, fitted lights drive the ride's bike lights, trainer SIM per race mode test, idle pose at the mirror, kit colours for shorts / socks / shoes / gloves (spec 3.23) |
| 18:43 | `bcf6345` (merge) | Merge main (3.21, B5 browser audio/voice) into the Phase 12 / 13 leftovers (3.22) |
| 18:43 | `d8ecdb0` | Phase 10 B5: the real-LiveKit browser test passes – the second rider's page in its own window (a hidden tab gets no animation frames, so its game loop never joined), checks that both pages are visible and joined; empty favicon (no 404); spec B5 note updated |
| 18:46 | `abe1abd` (merge) | Merge Phase 12 / 13 leftovers: wet-road grip in crashes and skids (Slide), chase camera shake and wider orbit, last-moment warning and proximity arrow as graphics, guided camera paths (G5), rider list page; deterministic frame-server test (spec 3.22) |
| 18:46 | `500e293` (merge) | Merge B5 follow-up: real LiveKit browser test passes (second rider in its own window, visibility and join checks), favicon 404 fixed |
| 20:11 | `97b00c3` | Phase 10 B6: UI parity – one menu, settings, time and weather machines, events and collisions for desktop and browser (new crate ocw-ui, ocw_client::menu), gamepad via the Gamepad API, timestamped screenshot / video / FIT downloads (spec 3.24) |
| 20:11 | `b3cdc56` (merge) | Merge Phase 10 B6: UI parity in the browser – shared menus, settings, HUD, time and weather machines in ocw-client / the new ocw-ui crate, platform filter, gamepad, timestamped captures (JPEG, WebM) and FIT download (spec 3.24) |
| 20:18 | `bfddece` | Spec 3.25: test pictures stand alone – sent and archived one by one, joined only to show a comparison (§11.4, ART-PROJECT §2) |
| 20:23 | `a979344` | Phase 10 B6 follow-up: the browser test checks that the Devices panel steps aside while the menu is open and comes back with its chooser buttons |
| 20:27 | `ee2aa6b` (merge) | Merge B6 follow-up: browser test checks that the Devices panel hides while the menu is open and returns after |
| 21:29 | `106cbd9` | Phase 8 T4 open items: roofs over polygons, dormers, balconies, building:part (spec 3.26) |
| 21:30 | `f250b61` (merge) | Merge Phase 8 T4 open items: straight-skeleton roofs on any footprint, dormers and roof windows, style-driven balconies, building:part (BPRT), min_height fix (spec 3.26) |
| 22:05 | `ce6a4ca` | Phase 10 B7: WebGL 2 fallback and quality presets (spec 3.27) |
| 22:06 | `e57f2e2` (merge) | Merge Phase 10 B7 (WebGL 2 part): a second web build for WebGL 2 chosen at page load, adapter-checked workarounds (simple sky, texture-array fallback, abrupt LOD ranges, one cascade), quality presets Low / Medium / High / Auto as display.quality for both apps, gpu and quality commands (spec 3.27) |
| 22:18 | `1d57b7a` | Docs catch-up (spec 3.28): CLAUDE.md project status and code map (forest, roof skeleton, seasons, quality, WebGL 2), machine notes (Mac zsh globs, livekit-server on marge, package rollout); spec phase headings 8 / 9 / 10 with their status, janitor /bin/sh note |
| 22:26 | `99dd6f9` | Spec 3.29: VR development headset is the Oculus Quest (first generation) – focus on it, other models postponed; Quest 1 facts (Link / Air Link / ALVR, SD835 72 Hz, sideload only); new Phase 11 Q7 (which PC drives it) |
| 22:27 | `b6e5662` | Spec 3.30: Phase 14 Q4 (BLE fallback) postponed (dashboard) |
| 22:29 | `c2157f9` | Spec 3.31: VR starts with Oculus Link (Quest 1 as a viewing device of the desktop app); standalone app by sideloading only |
| 22:36 | `0f8be5d` | README: step-by-step guide to the browser app (prerequisites, build, serve, page options, worlds, multiplayer, devices, sound and voice, menus, graphics, tests, limitations); weather physics, the bench catalogue, fitted lights, kit colours, quality presets, riding aids, the rider list, T4 houses, T3 forest type |
| 22:37 | `2250a8d` (merge) | Merge README: step-by-step browser app guide (setup, build, serve, solo, worlds, multiplayer, devices, sound and voice, captures, graphics, tests, limits) and today's missing features |
| 22:54 | `03659d6` | README: full browser URLs for the test routes and for a streamed world (example trust-root key, a one-liner printing your own), other worlds and route names |
| 23:13 | `d50652f` | Art: day 4 script – The weather, the seasons and the 404 |
| 23:20 | `27ec9bc` | Art: images 40 % longer, shots in capture order (images and videos mixed) in all day scripts; day 4 gets the early-morning chapters (collisions, workshop, trees) and more clips |
| 23:21 | `c3f069a` | Art: hourly collector (systemd user timer), archive to art/build-history/ with masked text in git (spec 3.34, ART-PROJECT 1.5) |
| 23:23 | `a7faff9` | Docs: build infrastructure chapter in the README, spec 3.36 (§11.2 overview, §11.2.1 dashboard install and page, §11.3 Windows zip) |
| 23:24 | `f813e5a` | README build infrastructure: root only for linger, one serving dashboard instance |
| 23:28 | `5bf6b52` (merge) | Merge build infrastructure docs: README Build infrastructure chapter (remote builds, dashboard, cleanup, kiosks, profiling, harness, releases, world builds), spec §11.2 / §11.2.1 / §11.3 (spec 3.32); make help lists targets with digits |
| 23:29 | `5dd86ca` | Untrack compiled Python files (__pycache__ is already git-ignored; running the dashboard rewrote them) |
| 23:52 | `c03317c` | Spec 3.33: sign-in with Google and Strava, brokered by our own backend (§9.1) |
| 23:53 | `0d64c56` | Spec 3.34: VR starts now (Phase 11 Q6), X1 first |
| 23:56 | `4aececa` (merge) | Merge art collector: hourly systemd user timer (art-collect), archive moved to art/build-history with masked text tracked in git, lock, status file and dashboard panel, ART-PROJECT v1.5 (spec 3.35) |
| 23:57 | `8db85b1` | Art: the build history's masked text files in git (index, SHA256SUMS, journals and token counts of days 1–4); archive moved to art/build-history |

## Builds and build servers

680 build/test runs on the build machines, 58.7 h in total, 23 failed; 1439 server samples; cargo compiled 6709 crates in 7.3 h, 9.9 h waiting for build slots.

| Machine | Runs | Hours | Cargo h | Crates | Waited h | Failed | Online |
|---|---|---|---|---|---|---|---|
| Win11Builder | 23 | 3.4 | 1.3 | 1259 | 0.2 | 4 | 99% |
| leela | 21 | 0.6 | 0.0 | 0 | 0.0 | 1 | 71% |
| macos | 126 | 6.4 | 0.2 | 752 | 1.1 | 1 | 100% |
| marge | 510 | 48.3 | 5.7 | 4698 | 8.5 | 17 | 100% |

Failed runs:

- 00:11 Win11Builder: `cargo clippy -p ocw-sim -p ocw-desktop --all-targets -- -D warnings 2>&1 | Selec` (exit 1, log `build-log/001116-Win11Builder-1164637.log`)
- 00:32 marge: `cargo fmt --all --check && cargo clippy -p ocw-desktop -p ocw-render --all-targe` (exit 1, log `build-log/003205-marge-1202102.log`)
- 00:39 marge: `cargo fmt --all --check && cargo clippy -p ocw-desktop -p ocw-render --all-targe` (exit 1, log `build-log/003921-marge-1209958.log`)
- 00:49 Win11Builder: `cargo build --release -p ocw-desktop -p ocw-server -p world-builder --features o` (exit 1, log `build-log/004952-Win11Builder-1222421.log`)
- 01:03 Win11Builder: `cargo check -p ocw-voice --features livekit --all-targets 2>&1 | Select-Object -` (exit 1, log `build-log/010308-Win11Builder-1236574.log`)
- 01:04 marge: `cargo build -q --release -p ocw-desktop 2>&1 | tail -3; export OCW_PROFILE=relea` (exit 1, log `build-log/010432-marge-1239978.log`)
- 01:48 leela: `   set -e   D=/var/tmp/ocw   put() { install -m $1 "$2" "$D/.$3.new" && mv -f "$` (exit 255, log `build-log/014845-leela-1285578.log`)
- 02:51 Win11Builder: `cargo clippy -p ocw-sim -p ocw-desktop --all-targets -- -D warnings 2>&1 | Selec` (exit 101, log `build-log/025106-Win11Builder-1348258.log`)
- 04:04 macos: `cargo test -p ocw-assets tmp_print -- --nocapture 2>&1 | grep -E "TRIS|OV "; car` (exit 1, log `build-log/040420-macos-1417260.log`)
- 04:42 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-14-044244 --only 'work` (exit 1, log `build-log/044244-marge-1458350.log`)
- 07:22 marge: `cargo run -q -p ocw-assets -- generate && cargo run -q -p ocw-assets -- parts va` (exit 101, log `build-log/072230-marge-1627745.log`)
- 07:23 marge: `cargo run -q -p ocw-assets -- generate && cargo run -q -p ocw-assets -- parts va` (exit 101, log `build-log/072309-marge-1628710.log`)
- 07:25 marge: `cargo run -q -p ocw-assets -- generate >/dev/null && git status --short assets/m` (exit 128, log `build-log/072510-marge-1631378.log`)
- 12:06 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-14-120606 --only 'k3-t` (exit 1, log `build-log/120606-marge-1965620.log`)
- 12:34 marge: `   set -e   D=/var/tmp/ocw   put() { install -m $1 "$2" "$D/.$3.new" && mv -f "$` (exit 101, log `build-log/123427-marge-2001161.log`)
- 12:38 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-14-123846 --only 'fix-` (exit 1, log `build-log/123846-marge-2006844.log`)
- 13:39 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-14-133926 --only menu-` (exit 1, log `build-log/133926-marge-2083358.log`)
- 15:35 marge: `df -h ~ | tail -1; pwd; ls ..; ls ../OCW/data ../OCW/data/pilot | head -50; pyth` (exit 127, log `build-log/153538-marge-2246417.log`)
- 17:59 marge: `cp apps/web/web/index.html target/web/ 2>/dev/null; ls target/web/ocw_web_bg.was` (exit 2, log `build-log/175958-marge-2470111.log`)
- 21:12 marge: `set -o pipefail; echo "== fmt"; cargo fmt --all --check 2>&1 | head -60; echo "f` (exit 101, log `build-log/211251-marge-2753485.log`)
- 21:21 marge: `rm -rf test-output/scenes/t4b-wayland test-output/scenes/t4b-x11 test-output/sce` (exit 1, log `build-log/212119-marge-2763843.log`)
- 23:13 marge: `make test-web-readme` (exit 2, log `build-log/231321-marge-2966239.log`)
- 23:34 marge: `make test-web-readme README_FLAGS=--keep` (exit 2, log `build-log/233427-marge-3010982.log`)

Longest runs:

- 51 min on marge: `set -o pipefail; for s in liechtenstein tenerife blackforest; do echo "== $s"; s`
- 45 min on marge: `cargo fmt --all --check >/dev/null 2>&1 && echo FMT_OK || { echo FMT_BAD; cargo `
- 39 min on marge: `set -e; df -h ~ | tail -1; scripts/build-liechtenstein.sh; scripts/build-tenerif`
- 37 min on marge: `make web 2>&1 | grep -v "^\s*Compiling\|extra-filename" | tail -25; make test-we`
- 36 min on marge: `make web && python3 scripts/web_browser.py version && make test-web; echo "TESTW`

## Token usage

3.18 B tokens in 9,044 model messages (output 8.1 M, cache reads 3.14 B, cache writes 30.2 M, uncached input 19 k). Full breakdown: [tokens.json](tokens.json).

| Feature | Tokens | Output | Messages |
|---|---|---|---|
| Planning & spec | 1.38 B | 3.4 M | 3,743 |
| Sky, time & weather | 487.7 M | 1.4 M | 1,365 |
| Other | 378.9 M | 972 k | 1,063 |
| Worlds & terrain | 193.3 M | 484 k | 531 |
| Audio & voice | 171.7 M | 407 k | 474 |
| UI & menus | 144.6 M | 287 k | 424 |
| Materials & look | 118.9 M | 282 k | 279 |
| Riders & bikes | 93.9 M | 230 k | 285 |
| Build, release & infra | 83.2 M | 225 k | 401 |
| Multiplayer & server | 64.5 M | 198 k | 207 |
| Art project | 64.0 M | 178 k | 272 |

| Task | Feature | Tokens | Output |
|---|---|---|---|
| agent: Phase 8 T7: orthophoto corridor | Planning & spec | 150.0 M | 226 k |
| agent: Phase 12 K4+K5: damage | Planning & spec | 137.0 M | 288 k |
| agent: Polish: small follow-ups bundle | Other | 133.5 M | 298 k |
| agent: Phase 4 W4: character corner | Planning & spec | 125.1 M | 286 k |
| agent: Phase 9 S4: weather rendering | Sky, time & weather | 121.2 M | 306 k |
| agent: Phase 8 T6: rider/bike materials | Materials & look | 118.9 M | 282 k |
| agent: Phase 12 K3: world objects | Worlds & terrain | 118.4 M | 264 k |
| agent: Phase 10 B6: browser UI parity | Planning & spec | 107.7 M | 239 k |
| agent: Phase 12/13 leftovers bundle | Planning & spec | 105.8 M | 229 k |
| coordination: check for unimplemented features | Other | 101.6 M | 58 k |
| agent: Phase 9 S7: real weather | Sky, time & weather | 97.7 M | 262 k |
| agent: Phase 12 K1+K2: collisions | Planning & spec | 93.8 M | 278 k |
| agent: Phase 4 W1+W2: workshop | Planning & spec | 90.2 M | 217 k |
| agent: Phase 8 T4 leftovers: houses | Planning & spec | 86.5 M | 276 k |
| agent: Phase 10 B5: browser audio+voice | Audio & voice | 85.5 M | 233 k |
| agent: Phase 4 workshop leftovers | Planning & spec | 81.2 M | 182 k |
| agent: Phase 7: training history, rebinding | Planning & spec | 79.5 M | 248 k |
| agent: Phase 10 B2: worlds over HTTP | Worlds & terrain | 74.9 M | 220 k |
| agent: Phase 9 S6: weather physics | Sky, time & weather | 74.1 M | 189 k |
| agent: Phase 9 S2: night sky w/o stars | Sky, time & weather | 71.6 M | 224 k |
| agent: Animations and K6 organiser rules | Riders & bikes | 66.0 M | 192 k |
| agent: Phase 10 B7: WebGL2 fallback | Planning & spec | 64.9 M | 170 k |
| agent: Phase 9 S5: weather machine | Sky, time & weather | 62.2 M | 200 k |
| coordination: add the voice echo bot to the settings menu | UI & menus | 62.1 M | 43 k |
| agent: Phase 10 B3: browser multiplayer | Multiplayer & server | 62.0 M | 195 k |
| agent: Phase 9 S8: seasons | Sky, time & weather | 60.9 M | 211 k |
| agent: Phase 4 W3: parts physics | Planning & spec | 56.5 M | 153 k |
| agent: Phase 10 B1: wasm browser build | Build, release & infra | 51.4 M | 134 k |
| agent: Phase 10 B4: Web Bluetooth | Planning & spec | 51.2 M | 197 k |
| agent: Phase 8 T3: forest type | Planning & spec | 46.3 M | 136 k |
| coordination: to test the voice feature, enable an echo bot that when near, records your audio… | Audio & voice | 45.3 M | 45 k |
| agent: Voice echo bot | Audio & voice | 40.9 M | 129 k |
| agent: Console autocomplete, GPS in HUD | UI & menus | 30.5 M | 113 k |
| agent: God mode: plane-like controls | Planning & spec | 22.6 M | 58 k |
| agent: God mode in the browser | Planning & spec | 20.7 M | 96 k |
| agent: Cooler helmets | Other | 20.6 M | 159 k |
| coordination: spec change: do not join two or more images, rather than that, send me all pictu… | Art project | 20.5 M | 23 k |
| coordination: (session start) | Other | 19.5 M | 51 k |
| coordination: tell me, how are the 3d models implemented for drivers, bikes etc implemented? a… | Riders & bikes | 17.3 M | 21 k |
| agent: README: web guide and doc gaps | Art project | 16.7 M | 52 k |

## Prompts

54 prompts (full text: [prompts.md](prompts.md)).

| Time | Prompt |
|---|---|
| 00:00 | god mode: have the unicorn move more its wings when flying. make the movement more organic, like a bird. |
| 00:01 | add the refined definition of god mode to specification. |
| 00:02 | continue the stopped build tasks on w11 builder. |
| 00:09 | check if the reporting back of answers to the user chatpot from the build-dashboard works. |
| 00:11 | to test the voice feature, enable an echo bot that when near, records your audio and plays it back to you a couple of ms later |
| 00:33 | build a new release and release all platforms. |
| 00:39 | add the voice echo bot to the settings menu |
| 06:38 | isthe build dashboard definition in the specification? |
| 06:41 | should the open question section in build dashboard renew automatically or do we need a background job? |
| 06:46 | give me the curren specification of the charackter editor? |
| 06:51 | change the spec: remove 'Looks only: kit never changes the physics; a time-trial helmet is just a look.' and change it to: The accessory das |
| 06:58 | fix spec that aero bike and aero components have an effect only at speeds higher speeds. investigate from which speed and how much. i also a |
| 07:07 | update the kiosk app on leela with the latest version |
| 07:14 | fix the numerical order in the spec summary table |
| 07:21 | how to start the kiosk on leela? |
| 07:22 | rebuilt and execute the kiosk on leela |
| 07:48 | check for unimplemented features |
| 17:42 | yes, install livekit-server on marge |
| 17:43 | why is macos disk full and with what? |
| 17:59 | tell me, how are the 3d models implemented for drivers, bikes etc implemented? are the saves as files somehiw pr are they in code? |
| 20:18 | spec change: do not join two or more images, rather than that, send me all picture stand alone (and save them alone for the art project). on |
| 22:17 | are there thing no yet added to documentation and specfile? update |
| 22:17 | add instructions on how to setup and run the web mode |
| 22:24 | VR: correction, I have an Oculus Quest version 1. focus on this model for VR development. postpone the other models. |
| 22:28 | VR: start with oculus link to add the VR headset just as viewing device of the desktop app. do the standalone VR app via sideloading only fo |
| 22:30 | which phase are we in now and whats open to develop? |
| 22:31 | which signin provider could you include easily? |
| 22:34 | compare keycloak and zitadel |
| 22:37 | what about kanidm? |
| 22:39 | where would be a good place to add a job for the art project that runs at the end of the day and collects all the data? who would ensure the |
| 22:41 | what about authentik as authentication provider? |
| 22:44 | yes, enable the art job collector on deb as systemd job. also joint the directories art and build-history, put build-history under art. keep |
| 22:48 | stop the background task job for the art collector |
| 22:52 | if the systemd art collector would run periodically. e.g. every hour, would it also work? |
| 22:54 | update the readme with a full example of the browser URL to do a both the test hills but also a map and not only the shortend URL: http://12 |
| 22:57 | for me the web mode does not work. no matter what URL I use, I always get   Error response  Error code: 404  Message: File not found.  Error |
| 22:59 | have you created the daily art project videos? |
| 23:02 | start over with the art collector. make it a systemd timer job that runs every hour and collects the data. |
| 23:06 | document the build dashboard and other build tools in the readme and in the spec file. |
| 23:06 | document the art collector in the art spec file. add also the files to git repo. |
| 23:08 | can the "make web-serve" webserver listen on 0.0.0.0? |
| 23:09 | authenication: would is be for now possible to just implement oauth via google and strava for now? |
| 23:09 | can you start with "scripts/web-serve.py --bind 0.0.0.0 --port 8080" on marge? |
| 23:19 | cool video, make the shown images a bit longer. and mix images and videos (chronologically). rebuild the day 4 video. create systemd timer j |
| 23:20 | make the helmets look cooler. |
| 23:30 | why does unicorn/god mode not work in browser mode? |
| 23:34 | add autocompletion to the ingame console. check and restore the GPS location printing in hud. |
| 23:45 | explain why the command "! sudo loginctl enable-linger $USER" what it does |
| 23:46 | this should be run on which machine? |
| 23:51 | implement the google and strava authentication as suggested. |
| 23:52 | start with VR for oculust rift. |
| 23:54 | plan for previewing the web version on the internet. what would be required for hosting a dev build? could it be hostet on github? |
| 23:55 | i do have  a quest 1. it should work via oculus link but also standalone. |
| 23:59 | okay, do the changes and implement the small preview hosting on github. add do readme after headline as preview. yes, record this in spec an |

## Media

| Time | File |
|---|---|
| 00:12:41 | [001241-video-wayland-god-plane-video.jpg](001241-video-wayland-god-plane-video.jpg) |
| 00:12:41 | [001241-video-wayland-god-plane-video.mp4](001241-video-wayland-god-plane-video.mp4) |
| 01:02:04 | [010204-echo-echo-demo.wav](010204-echo-echo-demo.wav) |
| 01:08:55 | [010855-agent-web-web.jpg](010855-agent-web-web.jpg) |
| 01:20:42 | [012042-agent-web-web.jpg](012042-agent-web-web.jpg) |
| 01:22:34 | [012234-agent-scenes-menu-audio-voice.jpg](012234-agent-scenes-menu-audio-voice.jpg) |
| 01:22:46 | [012246-agent-web-web.jpg](012246-agent-web-web.jpg) |
| 01:26:45 | [012645-agent-scenes-menu-audio-voice.jpg](012645-agent-scenes-menu-audio-voice.jpg) |
| 01:27:03 | [012703-agent-web-web.jpg](012703-agent-web-web.jpg) |
| 01:57:44 | [015744-agent-web-web.jpg](015744-agent-web-web.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-after-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-after-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-after-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-after-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-before-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-before-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-before-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-malbun-before-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-after-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-after-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-after-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-after-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-before-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-before-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-before-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-schauinsland-before-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-after-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-after-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-after-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-after-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-before-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-before-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-before-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-tf21-before-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-after-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-after-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-after-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-after-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-before-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-before-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-before-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-triesenberg-chase-before-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-after-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-after-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-after-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-after-x11.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-before-wayland.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-before-wayland.jpg) |
| 02:27:31 | [022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-before-x11.jpg](022731-agent-scenes-2026-11-14-t7-orthophoto-ortho-vaduz-before-x11.jpg) |
| 02:54:10 | [025410-video-wayland-collision-bump-video.jpg](025410-video-wayland-collision-bump-video.jpg) |
| 02:54:10 | [025410-video-wayland-collision-bump-video.mp4](025410-video-wayland-collision-bump-video.mp4) |
| 02:57:40 | [025740-agent-build-history-2026-11-14-collision-crash-video.mp4](025740-agent-build-history-2026-11-14-collision-crash-video.mp4) |
| 02:57:40 | [025740-video-wayland-collision-crash-video.jpg](025740-video-wayland-collision-crash-video.jpg) |
| 02:57:40 | [025740-video-wayland-collision-crash-video.mp4](025740-video-wayland-collision-crash-video.mp4) |
| 02:57:54 | [025754-shot-wayland-collision-crash.jpg](025754-shot-wayland-collision-crash.jpg) |
| 04:35:25 | [043525-agent-web-web-world.jpg](043525-agent-web-web-world.jpg) |
| 04:38:10 | [043810-shot-wayland-workshop-overview.jpg](043810-shot-wayland-workshop-overview.jpg) |
| 04:38:19 | [043819-shot-wayland-workshop-stand-hotspots.jpg](043819-shot-wayland-workshop-stand-hotspots.jpg) |
| 04:38:28 | [043828-shot-wayland-workshop-picker.jpg](043828-shot-wayland-workshop-picker.jpg) |
| 04:40:28 | [044028-agent-web-web-world-cached-loading.jpg](044028-agent-web-web-world-cached-loading.jpg) |
| 04:40:28 | [044028-agent-web-web-world-cached.jpg](044028-agent-web-web-world-cached.jpg) |
| 04:40:28 | [044028-agent-web-web-world-first-loading.jpg](044028-agent-web-web-world-first-loading.jpg) |
| 04:40:28 | [044028-agent-web-web-world-teleport.jpg](044028-agent-web-web-world-teleport.jpg) |
| 04:40:28 | [044028-agent-web-web-world.jpg](044028-agent-web-web-world.jpg) |
| 04:48:25 | [044825-agent-web-web-world-cached-loading.jpg](044825-agent-web-web-world-cached-loading.jpg) |
| 04:48:25 | [044825-agent-web-web-world-cached.jpg](044825-agent-web-web-world-cached.jpg) |
| 04:48:25 | [044825-agent-web-web-world-first-loading.jpg](044825-agent-web-web-world-first-loading.jpg) |
| 04:48:25 | [044825-agent-web-web-world-teleport.jpg](044825-agent-web-web-world-teleport.jpg) |
| 04:48:25 | [044825-agent-web-web-world.jpg](044825-agent-web-web-world.jpg) |
| 04:49:04 | [044904-agent-web-web.jpg](044904-agent-web-web.jpg) |
| 04:49:36 | [044936-shot-wayland-workshop-overview.jpg](044936-shot-wayland-workshop-overview.jpg) |
| 04:49:44 | [044944-shot-wayland-workshop-wall.jpg](044944-shot-wayland-workshop-wall.jpg) |
| 04:49:52 | [044952-shot-wayland-workshop-stand-hotspots.jpg](044952-shot-wayland-workshop-stand-hotspots.jpg) |
| 04:50:01 | [045001-shot-wayland-workshop-picker.jpg](045001-shot-wayland-workshop-picker.jpg) |
| 04:50:43 | [045043-video-wayland-workshop-swap-video.jpg](045043-video-wayland-workshop-swap-video.jpg) |
| 04:50:43 | [045043-video-wayland-workshop-swap-video.mp4](045043-video-wayland-workshop-swap-video.mp4) |
| 04:58:43 | [045843-shot-wayland-workshop-overview.jpg](045843-shot-wayland-workshop-overview.jpg) |
| 04:58:51 | [045851-shot-wayland-workshop-wall.jpg](045851-shot-wayland-workshop-wall.jpg) |
| 04:58:59 | [045859-shot-wayland-workshop-stand-hotspots.jpg](045859-shot-wayland-workshop-stand-hotspots.jpg) |
| 04:59:09 | [045909-shot-wayland-workshop-picker.jpg](045909-shot-wayland-workshop-picker.jpg) |
| 05:00:00 | [050000-video-wayland-workshop-swap-video.jpg](050000-video-wayland-workshop-swap-video.jpg) |
| 05:00:00 | [050000-video-wayland-workshop-swap-video.mp4](050000-video-wayland-workshop-swap-video.mp4) |
| 05:00:30 | [050030-shot-workshop-overview.jpg](050030-shot-workshop-overview.jpg) |
| 05:00:55 | [050055-shot-workshop-wall.jpg](050055-shot-workshop-wall.jpg) |
| 05:01:23 | [050123-shot-workshop-stand-hotspots.jpg](050123-shot-workshop-stand-hotspots.jpg) |
| 05:01:47 | [050147-shot-workshop-picker.jpg](050147-shot-workshop-picker.jpg) |
| 05:05:52 | [050552-shot-workshop-wall.jpg](050552-shot-workshop-wall.jpg) |
| 05:06:16 | [050616-shot-workshop-stand-hotspots.jpg](050616-shot-workshop-stand-hotspots.jpg) |
| 05:06:42 | [050642-shot-workshop-picker.jpg](050642-shot-workshop-picker.jpg) |
| 05:57:29 | [055729-agent-web-web-mp.jpg](055729-agent-web-web-mp.jpg) |
| 06:06:28 | [060628-agent-web-web-mp-desktop.jpg](060628-agent-web-web-mp-desktop.jpg) |
| 06:06:28 | [060628-agent-web-web-mp.jpg](060628-agent-web-web-mp.jpg) |
| 06:07:27 | [060727-agent-web-web.jpg](060727-agent-web-web.jpg) |
| 06:12:35 | [061235-agent-web-web-world-cached-loading.jpg](061235-agent-web-web-world-cached-loading.jpg) |
| 06:12:35 | [061235-agent-web-web-world-cached.jpg](061235-agent-web-web-world-cached.jpg) |
| 06:12:35 | [061235-agent-web-web-world-first-loading.jpg](061235-agent-web-web-world-first-loading.jpg) |
| 06:12:35 | [061235-agent-web-web-world-teleport.jpg](061235-agent-web-web-world-teleport.jpg) |
| 06:12:35 | [061235-agent-web-web-world.jpg](061235-agent-web-web-world.jpg) |
| 06:19:08 | [061908-agent-web-web-mp-desktop.jpg](061908-agent-web-web-mp-desktop.jpg) |
| 06:19:08 | [061908-agent-web-web-mp.jpg](061908-agent-web-web-mp.jpg) |
| 06:19:42 | [061942-agent-web-web.jpg](061942-agent-web-web.jpg) |
| 06:32:47 | [063247-shot-wayland-workshop-picker-effect.jpg](063247-shot-wayland-workshop-picker-effect.jpg) |
| 06:33:44 | [063344-agent-web-web-mp-desktop.jpg](063344-agent-web-web-mp-desktop.jpg) |
| 06:33:44 | [063344-agent-web-web-mp.jpg](063344-agent-web-web-mp.jpg) |
| 06:34:19 | [063419-agent-web-web.jpg](063419-agent-web-web.jpg) |
| 06:34:36 | [063436-video-wayland-sprint-open-video.jpg](063436-video-wayland-sprint-open-video.jpg) |
| 06:34:36 | [063436-video-wayland-sprint-open-video.mp4](063436-video-wayland-sprint-open-video.mp4) |
| 06:36:03 | [063603-shot-wayland-sprint-open-results.jpg](063603-shot-wayland-sprint-open-results.jpg) |
| 06:37:30 | [063730-shot-wayland-sprint-profile-results.jpg](063730-shot-wayland-sprint-profile-results.jpg) |
| 06:38:43 | [063843-agent-web-web-world-cached-loading.jpg](063843-agent-web-web-world-cached-loading.jpg) |
| 06:38:43 | [063843-agent-web-web-world-cached.jpg](063843-agent-web-web-world-cached.jpg) |
| 06:38:43 | [063843-agent-web-web-world-first-loading.jpg](063843-agent-web-web-world-first-loading.jpg) |
| 06:38:43 | [063843-agent-web-web-world-teleport.jpg](063843-agent-web-web-world-teleport.jpg) |
| 06:38:43 | [063843-agent-web-web-world.jpg](063843-agent-web-web-world.jpg) |
| 07:51:19 | [075119-shot-wayland-workshop-mirror.jpg](075119-shot-wayland-workshop-mirror.jpg) |
| 07:51:43 | [075143-shot-wayland-workshop-helmet-picker.jpg](075143-shot-wayland-workshop-helmet-picker.jpg) |
| 07:52:06 | [075206-shot-wayland-workshop-try-on.jpg](075206-shot-wayland-workshop-try-on.jpg) |
| 08:07:08 | [080708-shot-wayland-workshop-mirror.jpg](080708-shot-wayland-workshop-mirror.jpg) |
| 08:08:59 | [080859-shot-wayland-workshop-helmet-picker.jpg](080859-shot-wayland-workshop-helmet-picker.jpg) |
| 08:14:14 | [081414-shot-wayland-workshop-try-on.jpg](081414-shot-wayland-workshop-try-on.jpg) |
| 08:15:59 | [081559-video-wayland-workshop-kit-video.jpg](081559-video-wayland-workshop-kit-video.jpg) |
| 08:15:59 | [081559-video-wayland-workshop-kit-video.mp4](081559-video-wayland-workshop-kit-video.mp4) |
| 08:16:41 | [081641-shot-workshop-mirror.jpg](081641-shot-workshop-mirror.jpg) |
| 08:17:17 | [081717-shot-workshop-helmet-picker.jpg](081717-shot-workshop-helmet-picker.jpg) |
| 08:17:56 | [081756-shot-workshop-try-on.jpg](081756-shot-workshop-try-on.jpg) |
| 08:23:07 | [082307-shot-workshop-helmet-picker.jpg](082307-shot-workshop-helmet-picker.jpg) |
| 08:24:07 | [082407-shot-workshop-try-on.jpg](082407-shot-workshop-try-on.jpg) |
| 08:26:06 | [082606-video-wayland-workshop-kit-video.jpg](082606-video-wayland-workshop-kit-video.jpg) |
| 08:26:06 | [082606-video-wayland-workshop-kit-video.mp4](082606-video-wayland-workshop-kit-video.mp4) |
| 08:35:01 | [083501-shot-wayland-sky-night-lights.jpg](083501-shot-wayland-sky-night-lights.jpg) |
| 08:35:14 | [083514-shot-wayland-sky-full-moon-canadas.jpg](083514-shot-wayland-sky-full-moon-canadas.jpg) |
| 08:35:26 | [083526-shot-wayland-sky-crescent-venus.jpg](083526-shot-wayland-sky-crescent-venus.jpg) |
| 08:35:38 | [083538-shot-wayland-sky-moonless-vaduz.jpg](083538-shot-wayland-sky-moonless-vaduz.jpg) |
| 08:35:51 | [083551-shot-wayland-sky-milky-way-teide.jpg](083551-shot-wayland-sky-milky-way-teide.jpg) |
| 08:36:02 | [083602-shot-wayland-sky-full-moon-preset.jpg](083602-shot-wayland-sky-full-moon-preset.jpg) |
| 08:39:34 | [083934-shot-wayland-sky-night-lights.jpg](083934-shot-wayland-sky-night-lights.jpg) |
| 08:39:48 | [083948-shot-wayland-sky-full-moon-canadas.jpg](083948-shot-wayland-sky-full-moon-canadas.jpg) |
| 08:40:00 | [084000-shot-wayland-sky-crescent-venus.jpg](084000-shot-wayland-sky-crescent-venus.jpg) |
| 08:40:12 | [084012-shot-wayland-sky-moonless-vaduz.jpg](084012-shot-wayland-sky-moonless-vaduz.jpg) |
| 08:40:26 | [084026-shot-wayland-sky-milky-way-teide.jpg](084026-shot-wayland-sky-milky-way-teide.jpg) |
| 08:40:37 | [084037-shot-wayland-sky-full-moon-preset.jpg](084037-shot-wayland-sky-full-moon-preset.jpg) |
| 08:50:33 | [085033-shot-wayland-sky-night-lights.jpg](085033-shot-wayland-sky-night-lights.jpg) |
| 08:50:48 | [085048-shot-wayland-sky-full-moon-canadas.jpg](085048-shot-wayland-sky-full-moon-canadas.jpg) |
| 08:51:00 | [085100-shot-wayland-sky-crescent-venus.jpg](085100-shot-wayland-sky-crescent-venus.jpg) |
| 08:51:13 | [085113-shot-wayland-sky-moonless-vaduz.jpg](085113-shot-wayland-sky-moonless-vaduz.jpg) |
| 08:51:27 | [085127-shot-wayland-sky-milky-way-teide.jpg](085127-shot-wayland-sky-milky-way-teide.jpg) |
| 08:51:39 | [085139-shot-wayland-sky-full-moon-preset.jpg](085139-shot-wayland-sky-full-moon-preset.jpg) |
| 09:00:19 | [090019-shot-sky-night-lights.jpg](090019-shot-sky-night-lights.jpg) |
| 09:00:40 | [090040-shot-sky-full-moon-canadas.jpg](090040-shot-sky-full-moon-canadas.jpg) |
| 09:00:59 | [090059-shot-sky-crescent-venus.jpg](090059-shot-sky-crescent-venus.jpg) |
| 09:01:23 | [090123-shot-sky-moonless-vaduz.jpg](090123-shot-sky-moonless-vaduz.jpg) |
| 09:01:37 | [090137-shot-sky-milky-way-teide.jpg](090137-shot-sky-milky-way-teide.jpg) |
| 09:02:00 | [090200-shot-sky-full-moon-preset.jpg](090200-shot-sky-full-moon-preset.jpg) |
| 09:13:43 | [091343-agent-scenes-t7-main-packages-ortho-vaduz.jpg](091343-agent-scenes-t7-main-packages-ortho-vaduz.jpg) |
| 09:13:59 | [091359-agent-scenes-t7-main-packages-ortho-tf21.jpg](091359-agent-scenes-t7-main-packages-ortho-tf21.jpg) |
| 09:14:29 | [091429-shot-sky-night-lights.jpg](091429-shot-sky-night-lights.jpg) |
| 09:15:08 | [091508-shot-sky-full-moon-canadas.jpg](091508-shot-sky-full-moon-canadas.jpg) |
| 09:15:44 | [091544-shot-sky-crescent-venus.jpg](091544-shot-sky-crescent-venus.jpg) |
| 09:16:36 | [091636-shot-sky-moonless-vaduz.jpg](091636-shot-sky-moonless-vaduz.jpg) |
| 09:16:58 | [091658-shot-sky-milky-way-teide.jpg](091658-shot-sky-milky-way-teide.jpg) |
| 09:17:39 | [091739-shot-sky-full-moon-preset.jpg](091739-shot-sky-full-moon-preset.jpg) |
| 09:21:06 | [092106-agent-web-web-ble-fake-erg.jpg](092106-agent-web-web-ble-fake-erg.jpg) |
| 09:21:06 | [092106-agent-web-web-ble-fake-nobt.jpg](092106-agent-web-web-ble-fake-nobt.jpg) |
| 09:21:06 | [092106-agent-web-web-ble-fake-reload.jpg](092106-agent-web-web-ble-fake-reload.jpg) |
| 09:21:06 | [092106-agent-web-web-ble-fake-sim.jpg](092106-agent-web-web-ble-fake-sim.jpg) |
| 09:21:06 | [092106-agent-web-web-ble-fake.jpg](092106-agent-web-web-ble-fake.jpg) |
| 09:22:26 | [092226-agent-web-web-ble-fake-erg.jpg](092226-agent-web-web-ble-fake-erg.jpg) |
| 09:22:26 | [092226-agent-web-web-ble-fake-nobt.jpg](092226-agent-web-web-ble-fake-nobt.jpg) |
| 09:22:26 | [092226-agent-web-web-ble-fake-reload.jpg](092226-agent-web-web-ble-fake-reload.jpg) |
| 09:22:26 | [092226-agent-web-web-ble-fake-sim.jpg](092226-agent-web-web-ble-fake-sim.jpg) |
| 09:22:26 | [092226-agent-web-web-ble-fake.jpg](092226-agent-web-web-ble-fake.jpg) |
| 09:27:01 | [092701-agent-web-web-ble-emulation-emulation.jpg](092701-agent-web-web-ble-emulation-emulation.jpg) |
| 09:27:53 | [092753-agent-web-web-ble-emulation-emulation.jpg](092753-agent-web-web-ble-emulation-emulation.jpg) |
| 09:28:15 | [092815-agent-web-web-ble-real-real-erg.jpg](092815-agent-web-web-ble-real-real-erg.jpg) |
| 09:28:15 | [092815-agent-web-web-ble-real-real-sim.jpg](092815-agent-web-web-ble-real-real-sim.jpg) |
| 09:44:07 | [094407-agent-web-web.jpg](094407-agent-web-web.jpg) |
| 10:18:08 | [101808-agent-scenes-s4-wl-1-weather-light-rain-blackforest.jpg](101808-agent-scenes-s4-wl-1-weather-light-rain-blackforest.jpg) |
| 10:18:22 | [101822-agent-scenes-s4-wl-1-weather-thunderstorm-tenerife.jpg](101822-agent-scenes-s4-wl-1-weather-thunderstorm-tenerife.jpg) |
| 10:18:35 | [101835-agent-scenes-s4-wl-1-weather-fog-vaduz.jpg](101835-agent-scenes-s4-wl-1-weather-fog-vaduz.jpg) |
| 10:20:08 | [102008-agent-scenes-s4-wl-1-weather-heavy-snow-feldberg.jpg](102008-agent-scenes-s4-wl-1-weather-heavy-snow-feldberg.jpg) |
| 10:20:19 | [102019-agent-scenes-s4-wl-1-weather-overcast-flat-light.jpg](102019-agent-scenes-s4-wl-1-weather-overcast-flat-light.jpg) |
| 10:22:53 | [102253-agent-scenes-s4-wl-2-weather-light-rain-blackforest.jpg](102253-agent-scenes-s4-wl-2-weather-light-rain-blackforest.jpg) |
| 10:23:08 | [102308-agent-scenes-s4-wl-2-weather-thunderstorm-tenerife.jpg](102308-agent-scenes-s4-wl-2-weather-thunderstorm-tenerife.jpg) |
| 10:23:33 | [102333-agent-scenes-s4-wl-2-weather-thunderstorm-video.jpg](102333-agent-scenes-s4-wl-2-weather-thunderstorm-video.jpg) |
| 10:24:27 | [102427-agent-scenes-s4-wl-2-weather-thunderstorm-video.mp4](102427-agent-scenes-s4-wl-2-weather-thunderstorm-video.mp4) |
| 10:24:40 | [102440-agent-scenes-s4-wl-2-weather-fog-vaduz.jpg](102440-agent-scenes-s4-wl-2-weather-fog-vaduz.jpg) |
| 10:24:54 | [102454-agent-scenes-s4-wl-2-weather-heavy-snow-feldberg.jpg](102454-agent-scenes-s4-wl-2-weather-heavy-snow-feldberg.jpg) |
| 10:25:05 | [102505-agent-scenes-s4-wl-2-weather-overcast-flat-light.jpg](102505-agent-scenes-s4-wl-2-weather-overcast-flat-light.jpg) |
| 10:25:29 | [102529-agent-scenes-s4-wl-2-weather-rain-shower-video.jpg](102529-agent-scenes-s4-wl-2-weather-rain-shower-video.jpg) |
| 10:26:27 | [102627-agent-scenes-s4-wl-2-weather-rain-shower-video.mp4](102627-agent-scenes-s4-wl-2-weather-rain-shower-video.mp4) |
| 10:26:49 | [102649-agent-scenes-s4-wl-2-weather-gale-video.jpg](102649-agent-scenes-s4-wl-2-weather-gale-video.jpg) |
| 10:27:14 | [102714-agent-scenes-s4-wl-2-weather-gale-video.mp4](102714-agent-scenes-s4-wl-2-weather-gale-video.mp4) |
| 10:27:39 | [102739-agent-scenes-s4-x11-1-weather-light-rain-blackforest.jpg](102739-agent-scenes-s4-x11-1-weather-light-rain-blackforest.jpg) |
| 10:28:10 | [102810-agent-scenes-s4-x11-1-weather-thunderstorm-tenerife.jpg](102810-agent-scenes-s4-x11-1-weather-thunderstorm-tenerife.jpg) |
| 10:28:36 | [102836-agent-scenes-s4-x11-1-weather-fog-vaduz.jpg](102836-agent-scenes-s4-x11-1-weather-fog-vaduz.jpg) |
| 10:29:13 | [102913-agent-scenes-s4-x11-1-weather-heavy-snow-feldberg.jpg](102913-agent-scenes-s4-x11-1-weather-heavy-snow-feldberg.jpg) |
| 10:29:46 | [102946-agent-scenes-s4-x11-1-weather-overcast-flat-light.jpg](102946-agent-scenes-s4-x11-1-weather-overcast-flat-light.jpg) |
| 10:46:32 | [104632-shot-wayland-t6-carbon-stand.jpg](104632-shot-wayland-t6-carbon-stand.jpg) |
| 10:46:41 | [104641-shot-wayland-t6-group-front.jpg](104641-shot-wayland-t6-group-front.jpg) |
| 10:46:51 | [104651-agent-scenes-s4-wl-3-weather-thunderstorm-tenerife.jpg](104651-agent-scenes-s4-wl-3-weather-thunderstorm-tenerife.jpg) |
| 10:47:08 | [104708-agent-scenes-s4-wl-3-weather-thunderstorm-video.jpg](104708-agent-scenes-s4-wl-3-weather-thunderstorm-video.jpg) |
| 10:47:22 | [104722-shot-wayland-t6-jersey-designer.jpg](104722-shot-wayland-t6-jersey-designer.jpg) |
| 10:47:30 | [104730-agent-scenes-s4-wl-3-weather-thunderstorm-video.mp4](104730-agent-scenes-s4-wl-3-weather-thunderstorm-video.mp4) |
| 10:47:50 | [104750-agent-scenes-s4-wl-3-weather-rain-shower-video.jpg](104750-agent-scenes-s4-wl-3-weather-rain-shower-video.jpg) |
| 10:48:21 | [104821-agent-scenes-s4-wl-3-weather-rain-shower-video.mp4](104821-agent-scenes-s4-wl-3-weather-rain-shower-video.mp4) |
| 10:49:05 | [104905-shot-wayland-t6-group-back.jpg](104905-shot-wayland-t6-group-back.jpg) |
| 10:49:15 | [104915-shot-wayland-t6-rider-close.jpg](104915-shot-wayland-t6-rider-close.jpg) |
| 10:49:32 | [104932-shot-t6-carbon-stand.jpg](104932-shot-t6-carbon-stand.jpg) |
| 10:49:52 | [104952-shot-t6-group-front.jpg](104952-shot-t6-group-front.jpg) |
| 10:50:11 | [105011-video-wayland-t6-designer-video.jpg](105011-video-wayland-t6-designer-video.jpg) |
| 10:50:11 | [105011-video-wayland-t6-designer-video.mp4](105011-video-wayland-t6-designer-video.mp4) |
| 10:50:16 | [105016-shot-t6-group-back.jpg](105016-shot-t6-group-back.jpg) |
| 10:50:36 | [105036-shot-t6-rider-close.jpg](105036-shot-t6-rider-close.jpg) |
| 10:51:22 | [105122-shot-t6-jersey-designer.jpg](105122-shot-t6-jersey-designer.jpg) |
| 10:51:54 | [105154-shot-wayland-t6-group-back.jpg](105154-shot-wayland-t6-group-back.jpg) |
| 10:52:04 | [105204-shot-wayland-t6-rider-close.jpg](105204-shot-wayland-t6-rider-close.jpg) |
| 11:09:13 | [110913-agent-scenes-s4-merged-wl-weather-light-rain-blackforest.jpg](110913-agent-scenes-s4-merged-wl-weather-light-rain-blackforest.jpg) |
| 11:09:28 | [110928-agent-scenes-s4-merged-wl-weather-heavy-snow-feldberg.jpg](110928-agent-scenes-s4-merged-wl-weather-heavy-snow-feldberg.jpg) |
| 11:09:45 | [110945-agent-scenes-s4-merged-x11-weather-heavy-snow-feldberg.jpg](110945-agent-scenes-s4-merged-x11-weather-heavy-snow-feldberg.jpg) |
| 11:10:52 | [111052-shot-wayland-t6-carbon-stand.jpg](111052-shot-wayland-t6-carbon-stand.jpg) |
| 11:11:02 | [111102-shot-wayland-t6-rider-close.jpg](111102-shot-wayland-t6-rider-close.jpg) |
| 11:16:11 | [111611-shot-wayland-t6-carbon-stand.jpg](111611-shot-wayland-t6-carbon-stand.jpg) |
| 11:16:20 | [111620-shot-wayland-t6-group-front.jpg](111620-shot-wayland-t6-group-front.jpg) |
| 11:16:29 | [111629-shot-wayland-t6-group-back.jpg](111629-shot-wayland-t6-group-back.jpg) |
| 11:16:38 | [111638-shot-wayland-t6-rider-close.jpg](111638-shot-wayland-t6-rider-close.jpg) |
| 11:17:19 | [111719-shot-wayland-t6-jersey-designer.jpg](111719-shot-wayland-t6-jersey-designer.jpg) |
| 11:18:10 | [111810-video-wayland-t6-designer-video.jpg](111810-video-wayland-t6-designer-video.jpg) |
| 11:18:10 | [111810-video-wayland-t6-designer-video.mp4](111810-video-wayland-t6-designer-video.mp4) |
| 11:18:51 | [111851-shot-t6-carbon-stand.jpg](111851-shot-t6-carbon-stand.jpg) |
| 11:19:18 | [111918-shot-t6-group-front.jpg](111918-shot-t6-group-front.jpg) |
| 11:19:43 | [111943-shot-t6-group-back.jpg](111943-shot-t6-group-back.jpg) |
| 11:20:15 | [112015-shot-t6-rider-close.jpg](112015-shot-t6-rider-close.jpg) |
| 11:21:08 | [112108-shot-t6-jersey-designer.jpg](112108-shot-t6-jersey-designer.jpg) |
| 11:48:57 | [114857-shot-wayland-k3-road-bollards.jpg](114857-shot-wayland-k3-road-bollards.jpg) |
| 11:49:12 | [114912-shot-wayland-k3-road-bollards-hidden.jpg](114912-shot-wayland-k3-road-bollards-hidden.jpg) |
| 11:50:52 | [115052-video-wayland-k3-tree-crash-video.jpg](115052-video-wayland-k3-tree-crash-video.jpg) |
| 11:50:52 | [115052-video-wayland-k3-tree-crash-video.mp4](115052-video-wayland-k3-tree-crash-video.mp4) |
| 11:51:15 | [115115-shot-wayland-k3-tree-crash.jpg](115115-shot-wayland-k3-tree-crash.jpg) |
| 11:51:28 | [115128-shot-wayland-k3-bridge-railing.jpg](115128-shot-wayland-k3-bridge-railing.jpg) |
| 11:54:19 | [115419-shot-wayland-k3-road-bollards.jpg](115419-shot-wayland-k3-road-bollards.jpg) |
| 11:55:21 | [115521-video-wayland-k3-tree-crash-video.jpg](115521-video-wayland-k3-tree-crash-video.jpg) |
| 11:55:21 | [115521-video-wayland-k3-tree-crash-video.mp4](115521-video-wayland-k3-tree-crash-video.mp4) |
| 11:55:42 | [115542-shot-wayland-k3-tree-crash.jpg](115542-shot-wayland-k3-tree-crash.jpg) |
| 11:57:12 | [115712-video-wayland-k3-tree-crash-video.jpg](115712-video-wayland-k3-tree-crash-video.jpg) |
| 11:57:12 | [115712-video-wayland-k3-tree-crash-video.mp4](115712-video-wayland-k3-tree-crash-video.mp4) |
| 11:57:33 | [115733-shot-wayland-k3-tree-crash.jpg](115733-shot-wayland-k3-tree-crash.jpg) |
| 11:57:37 | [115737-shot-wayland-menu-world-weather.jpg](115737-shot-wayland-menu-world-weather.jpg) |
| 11:57:54 | [115754-shot-wayland-menu-weather-presets.jpg](115754-shot-wayland-menu-weather-presets.jpg) |
| 11:59:00 | [115900-shot-wayland-k3t.jpg](115900-shot-wayland-k3t.jpg) |
| 11:59:55 | [115955-shot-wayland-k3t.jpg](115955-shot-wayland-k3t.jpg) |
| 12:00:46 | [120046-shot-wayland-k3t.jpg](120046-shot-wayland-k3t.jpg) |
| 12:01:22 | [120122-shot-wayland-k3-road-bollards.jpg](120122-shot-wayland-k3-road-bollards.jpg) |
| 12:01:37 | [120137-shot-wayland-k3-road-bollards-hidden.jpg](120137-shot-wayland-k3-road-bollards-hidden.jpg) |
| 12:02:08 | [120208-shot-wayland-menu-weather-custom.jpg](120208-shot-wayland-menu-weather-custom.jpg) |
| 12:02:15 | [120215-shot-wayland-event-weather-start.jpg](120215-shot-wayland-event-weather-start.jpg) |
| 12:02:33 | [120233-shot-k3-road-bollards.jpg](120233-shot-k3-road-bollards.jpg) |
| 12:02:48 | [120248-video-wayland-k3-tree-crash-video.jpg](120248-video-wayland-k3-tree-crash-video.jpg) |
| 12:02:48 | [120248-video-wayland-k3-tree-crash-video.mp4](120248-video-wayland-k3-tree-crash-video.mp4) |
| 12:03:13 | [120313-shot-wayland-k3-tree-crash.jpg](120313-shot-wayland-k3-tree-crash.jpg) |
| 12:03:17 | [120317-shot-k3-road-bollards-hidden.jpg](120317-shot-k3-road-bollards-hidden.jpg) |
| 12:03:58 | [120358-shot-wayland-k3-bridge-railing.jpg](120358-shot-wayland-k3-bridge-railing.jpg) |
| 12:06:29 | [120629-agent-build-history-2026-11-14-menu-world-weather.jpg](120629-agent-build-history-2026-11-14-menu-world-weather.jpg) |
| 12:06:29 | [120629-shot-wayland-menu-world-weather.jpg](120629-shot-wayland-menu-world-weather.jpg) |
| 12:06:41 | [120641-shot-wayland-menu-weather-presets.jpg](120641-shot-wayland-menu-weather-presets.jpg) |
| 12:06:53 | [120653-agent-build-history-2026-11-14-menu-weather-custom.jpg](120653-agent-build-history-2026-11-14-menu-weather-custom.jpg) |
| 12:06:53 | [120653-shot-wayland-menu-weather-custom.jpg](120653-shot-wayland-menu-weather-custom.jpg) |
| 12:07:01 | [120701-shot-wayland-event-weather-start.jpg](120701-shot-wayland-event-weather-start.jpg) |
| 12:10:02 | [121002-video-wayland-k3-tree-crash-video.jpg](121002-video-wayland-k3-tree-crash-video.jpg) |
| 12:10:02 | [121002-video-wayland-k3-tree-crash-video.mp4](121002-video-wayland-k3-tree-crash-video.mp4) |
| 12:10:21 | [121021-shot-wayland-k3-tree-crash.jpg](121021-shot-wayland-k3-tree-crash.jpg) |
| 12:11:05 | [121105-shot-wayland-k3t.jpg](121105-shot-wayland-k3t.jpg) |
| 12:11:48 | [121148-shot-wayland-k3-tree-crash.jpg](121148-shot-wayland-k3-tree-crash.jpg) |
| 12:12:49 | [121249-shot-k3-road-bollards.jpg](121249-shot-k3-road-bollards.jpg) |
| 12:13:39 | [121339-shot-k3-road-bollards-hidden.jpg](121339-shot-k3-road-bollards-hidden.jpg) |
| 12:13:56 | [121356-agent-build-history-2026-11-14-weather-clear-to-thunderstorm-video.mp4](121356-agent-build-history-2026-11-14-weather-clear-to-thunderstorm-video.mp4) |
| 12:13:58 | [121358-video-wayland-weather-clear-to-thunderstorm-video.jpg](121358-video-wayland-weather-clear-to-thunderstorm-video.jpg) |
| 12:13:58 | [121358-video-wayland-weather-clear-to-thunderstorm-video.mp4](121358-video-wayland-weather-clear-to-thunderstorm-video.mp4) |
| 12:23:30 | [122330-shot-menu-world-weather.jpg](122330-shot-menu-world-weather.jpg) |
| 12:24:18 | [122418-shot-menu-weather-presets.jpg](122418-shot-menu-weather-presets.jpg) |
| 12:25:05 | [122505-shot-menu-weather-custom.jpg](122505-shot-menu-weather-custom.jpg) |
| 12:25:34 | [122534-shot-event-weather-start.jpg](122534-shot-event-weather-start.jpg) |
| 12:25:43 | [122543-video-k3-tree-crash-video.jpg](122543-video-k3-tree-crash-video.jpg) |
| 12:25:43 | [122543-video-k3-tree-crash-video.mp4](122543-video-k3-tree-crash-video.mp4) |
| 12:26:24 | [122624-shot-menu-world-weather.jpg](122624-shot-menu-world-weather.jpg) |
| 12:26:53 | [122653-shot-menu-weather-presets.jpg](122653-shot-menu-weather-presets.jpg) |
| 12:27:17 | [122717-shot-k3-tree-crash.jpg](122717-shot-k3-tree-crash.jpg) |
| 12:27:29 | [122729-shot-menu-weather-custom.jpg](122729-shot-menu-weather-custom.jpg) |
| 12:27:52 | [122752-shot-event-weather-start.jpg](122752-shot-event-weather-start.jpg) |
| 12:27:55 | [122755-shot-k3-bridge-railing.jpg](122755-shot-k3-bridge-railing.jpg) |
| 12:33:33 | [123333-shot-wayland-sky-moonless-vaduz.jpg](123333-shot-wayland-sky-moonless-vaduz.jpg) |
| 12:33:59 | [123359-shot-wayland-fix-groupset-picker.jpg](123359-shot-wayland-fix-groupset-picker.jpg) |
| 12:34:25 | [123425-shot-wayland-fix-groupset-crankset.jpg](123425-shot-wayland-fix-groupset-crankset.jpg) |
| 12:34:39 | [123439-shot-wayland-fix-console-workshop.jpg](123439-shot-wayland-fix-console-workshop.jpg) |
| 12:35:11 | [123511-shot-wayland-fix-carbon-stand.jpg](123511-shot-wayland-fix-carbon-stand.jpg) |
| 12:35:43 | [123543-shot-wayland-fix-mtb-fork.jpg](123543-shot-wayland-fix-mtb-fork.jpg) |
| 12:36:15 | [123615-shot-wayland-fix-mtb-fork-front.jpg](123615-shot-wayland-fix-mtb-fork-front.jpg) |
| 12:36:26 | [123626-shot-wayland-fix-hip-number.jpg](123626-shot-wayland-fix-hip-number.jpg) |
| 12:36:43 | [123643-shot-wayland-fix-wild-kerb-vaduz.jpg](123643-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 12:36:54 | [123654-shot-wayland-fix-sidewalk-corners-vaduz.jpg](123654-shot-wayland-fix-sidewalk-corners-vaduz.jpg) |
| 12:37:05 | [123705-shot-wayland-fix-sidewalk-corners-aeule.jpg](123705-shot-wayland-fix-sidewalk-corners-aeule.jpg) |
| 12:39:51 | [123951-shot-wayland-sky-moonless-vaduz.jpg](123951-shot-wayland-sky-moonless-vaduz.jpg) |
| 12:40:27 | [124027-shot-wayland-fix-hip-number.jpg](124027-shot-wayland-fix-hip-number.jpg) |
| 12:40:50 | [124050-shot-wayland-fix-wild-kerb-vaduz.jpg](124050-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 12:41:01 | [124101-shot-wayland-fix-sidewalk-corners-vaduz.jpg](124101-shot-wayland-fix-sidewalk-corners-vaduz.jpg) |
| 12:41:11 | [124111-shot-wayland-fix-sidewalk-corners-aeule.jpg](124111-shot-wayland-fix-sidewalk-corners-aeule.jpg) |
| 12:43:59 | [124359-shot-wayland-fix-groupset-picker.jpg](124359-shot-wayland-fix-groupset-picker.jpg) |
| 12:44:24 | [124424-shot-wayland-fix-groupset-crankset.jpg](124424-shot-wayland-fix-groupset-crankset.jpg) |
| 12:44:38 | [124438-shot-wayland-fix-console-workshop.jpg](124438-shot-wayland-fix-console-workshop.jpg) |
| 12:45:09 | [124509-shot-wayland-fix-carbon-stand.jpg](124509-shot-wayland-fix-carbon-stand.jpg) |
| 12:45:40 | [124540-shot-wayland-fix-mtb-fork.jpg](124540-shot-wayland-fix-mtb-fork.jpg) |
| 12:46:12 | [124612-shot-wayland-fix-mtb-fork-front.jpg](124612-shot-wayland-fix-mtb-fork-front.jpg) |
| 12:49:38 | [124938-shot-wayland-fix-console-workshop.jpg](124938-shot-wayland-fix-console-workshop.jpg) |
| 12:50:10 | [125010-shot-wayland-fix-carbon-stand.jpg](125010-shot-wayland-fix-carbon-stand.jpg) |
| 12:50:32 | [125032-shot-wayland-fix-wild-kerb-vaduz.jpg](125032-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 12:50:42 | [125042-shot-wayland-fix-sidewalk-corners-vaduz.jpg](125042-shot-wayland-fix-sidewalk-corners-vaduz.jpg) |
| 12:50:53 | [125053-shot-wayland-fix-sidewalk-corners-aeule.jpg](125053-shot-wayland-fix-sidewalk-corners-aeule.jpg) |
| 12:54:40 | [125440-shot-wayland-fix-wild-kerb-vaduz.jpg](125440-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 12:54:50 | [125450-shot-wayland-fix-sidewalk-corners-vaduz.jpg](125450-shot-wayland-fix-sidewalk-corners-vaduz.jpg) |
| 12:55:01 | [125501-shot-wayland-fix-sidewalk-corners-aeule.jpg](125501-shot-wayland-fix-sidewalk-corners-aeule.jpg) |
| 12:56:28 | [125628-shot-wayland-fix-wild-kerb-vaduz.jpg](125628-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 12:56:38 | [125638-shot-wayland-fix-sidewalk-corners-vaduz.jpg](125638-shot-wayland-fix-sidewalk-corners-vaduz.jpg) |
| 12:59:17 | [125917-shot-wayland-fix-wild-kerb-vaduz.jpg](125917-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 13:01:03 | [130103-shot-wayland-fix-wild-kerb-vaduz-top.jpg](130103-shot-wayland-fix-wild-kerb-vaduz-top.jpg) |
| 13:05:01 | [130501-shot-wayland-fix-wild-kerb-vaduz.jpg](130501-shot-wayland-fix-wild-kerb-vaduz.jpg) |
| 13:05:19 | [130519-shot-wayland-fix-wild-kerb-vaduz-top.jpg](130519-shot-wayland-fix-wild-kerb-vaduz-top.jpg) |
| 13:05:55 | [130555-agent-scenes-before-fix-wild-kerb-vaduz.jpg](130555-agent-scenes-before-fix-wild-kerb-vaduz.jpg) |
| 13:42:01 | [134201-shot-wayland-menu-weather-real.jpg](134201-shot-wayland-menu-weather-real.jpg) |
| 13:42:23 | [134223-shot-wayland-weather-real-vaduz.jpg](134223-shot-wayland-weather-real-vaduz.jpg) |
| 13:47:32 | [134732-shot-wayland-weather-real-lothar-blackforest.jpg](134732-shot-wayland-weather-real-lothar-blackforest.jpg) |
| 13:48:36 | [134836-shot-menu-weather-real.jpg](134836-shot-menu-weather-real.jpg) |
| 13:49:27 | [134927-shot-weather-real-vaduz.jpg](134927-shot-weather-real-vaduz.jpg) |
| 13:50:12 | [135012-shot-weather-real-lothar-blackforest.jpg](135012-shot-weather-real-lothar-blackforest.jpg) |
| 14:07:55 | [140755-shot-wayland-k4-damage-crash.jpg](140755-shot-wayland-k4-damage-crash.jpg) |
| 14:08:33 | [140833-video-wayland-k4-damage-crash-video.jpg](140833-video-wayland-k4-damage-crash-video.jpg) |
| 14:08:33 | [140833-video-wayland-k4-damage-crash-video.mp4](140833-video-wayland-k4-damage-crash-video.mp4) |
| 14:08:42 | [140842-shot-wayland-k4-buckled-wheel.jpg](140842-shot-wayland-k4-buckled-wheel.jpg) |
| 14:08:52 | [140852-shot-wayland-k5-visible-damage.jpg](140852-shot-wayland-k5-visible-damage.jpg) |
| 14:09:16 | [140916-shot-wayland-k4-workshop-repair.jpg](140916-shot-wayland-k4-workshop-repair.jpg) |
| 14:09:37 | [140937-shot-wayland-k4-workshop-repair-card.jpg](140937-shot-wayland-k4-workshop-repair-card.jpg) |
| 14:09:58 | [140958-shot-wayland-k4-workshop-injury.jpg](140958-shot-wayland-k4-workshop-injury.jpg) |
| 14:16:19 | [141619-shot-wayland-k4-damage-crash.jpg](141619-shot-wayland-k4-damage-crash.jpg) |
| 14:17:10 | [141710-video-wayland-k4-damage-crash-video.jpg](141710-video-wayland-k4-damage-crash-video.jpg) |
| 14:17:10 | [141710-video-wayland-k4-damage-crash-video.mp4](141710-video-wayland-k4-damage-crash-video.mp4) |
| 14:20:54 | [142054-shot-wayland-k4-buckled-wheel.jpg](142054-shot-wayland-k4-buckled-wheel.jpg) |
| 14:21:12 | [142112-shot-wayland-k5-visible-damage.jpg](142112-shot-wayland-k5-visible-damage.jpg) |
| 14:21:35 | [142135-shot-wayland-k4-workshop-repair.jpg](142135-shot-wayland-k4-workshop-repair.jpg) |
| 14:21:57 | [142157-shot-wayland-k4-workshop-repair-card.jpg](142157-shot-wayland-k4-workshop-repair-card.jpg) |
| 14:22:18 | [142218-shot-wayland-k4-workshop-injury.jpg](142218-shot-wayland-k4-workshop-injury.jpg) |
| 14:23:03 | [142303-shot-k4-damage-crash.jpg](142303-shot-k4-damage-crash.jpg) |
| 14:25:56 | [142556-shot-k4-buckled-wheel.jpg](142556-shot-k4-buckled-wheel.jpg) |
| 14:27:04 | [142704-shot-k5-visible-damage.jpg](142704-shot-k5-visible-damage.jpg) |
| 14:28:02 | [142802-shot-k4-workshop-repair.jpg](142802-shot-k4-workshop-repair.jpg) |
| 14:28:50 | [142850-shot-k4-workshop-repair-card.jpg](142850-shot-k4-workshop-repair-card.jpg) |
| 14:29:41 | [142941-shot-k4-workshop-injury.jpg](142941-shot-k4-workshop-injury.jpg) |
| 14:31:43 | [143143-shot-wayland-k4-workshop-injury.jpg](143143-shot-wayland-k4-workshop-injury.jpg) |
| 14:32:28 | [143228-shot-k4-workshop-injury.jpg](143228-shot-k4-workshop-injury.jpg) |
| 14:36:46 | [143646-shot-wayland-k4-damage-crash.jpg](143646-shot-wayland-k4-damage-crash.jpg) |
| 14:39:33 | [143933-shot-k4-damage-crash.jpg](143933-shot-k4-damage-crash.jpg) |
| 14:44:14 | [144414-shot-wayland-season-vaduz-winter.jpg](144414-shot-wayland-season-vaduz-winter.jpg) |
| 14:44:26 | [144426-shot-wayland-season-vaduz-spring.jpg](144426-shot-wayland-season-vaduz-spring.jpg) |
| 14:44:39 | [144439-shot-wayland-season-vaduz-summer.jpg](144439-shot-wayland-season-vaduz-summer.jpg) |
| 14:44:51 | [144451-shot-wayland-season-vaduz-autumn.jpg](144451-shot-wayland-season-vaduz-autumn.jpg) |
| 14:45:03 | [144503-shot-wayland-season-fields-winter.jpg](144503-shot-wayland-season-fields-winter.jpg) |
| 14:45:14 | [144514-shot-wayland-season-fields-spring.jpg](144514-shot-wayland-season-fields-spring.jpg) |
| 14:45:26 | [144526-shot-wayland-season-fields-summer.jpg](144526-shot-wayland-season-fields-summer.jpg) |
| 14:46:09 | [144609-shot-wayland-season-fields-autumn.jpg](144609-shot-wayland-season-fields-autumn.jpg) |
| 14:47:59 | [144759-shot-wayland-season-malbun-spring.jpg](144759-shot-wayland-season-malbun-spring.jpg) |
| 14:50:10 | [145010-shot-k4-damage-crash.jpg](145010-shot-k4-damage-crash.jpg) |
| 14:50:53 | [145053-shot-k4-damage-crash.jpg](145053-shot-k4-damage-crash.jpg) |
| 14:52:49 | [145249-shot-wayland-season-malbun-winter.jpg](145249-shot-wayland-season-malbun-winter.jpg) |
| 14:58:00 | [145800-shot-wayland-season-vaduz-winter.jpg](145800-shot-wayland-season-vaduz-winter.jpg) |
| 14:58:12 | [145812-shot-wayland-season-vaduz-spring.jpg](145812-shot-wayland-season-vaduz-spring.jpg) |
| 14:58:24 | [145824-shot-wayland-season-vaduz-summer.jpg](145824-shot-wayland-season-vaduz-summer.jpg) |
| 14:58:35 | [145835-shot-wayland-season-vaduz-autumn.jpg](145835-shot-wayland-season-vaduz-autumn.jpg) |
| 14:58:47 | [145847-shot-wayland-season-fields-winter.jpg](145847-shot-wayland-season-fields-winter.jpg) |
| 14:58:59 | [145859-shot-wayland-season-fields-spring.jpg](145859-shot-wayland-season-fields-spring.jpg) |
| 14:59:10 | [145910-shot-wayland-season-fields-summer.jpg](145910-shot-wayland-season-fields-summer.jpg) |
| 14:59:22 | [145922-shot-wayland-season-fields-autumn.jpg](145922-shot-wayland-season-fields-autumn.jpg) |
| 14:59:34 | [145934-shot-wayland-season-malbun-spring.jpg](145934-shot-wayland-season-malbun-spring.jpg) |
| 14:59:45 | [145945-shot-wayland-season-malbun-winter.jpg](145945-shot-wayland-season-malbun-winter.jpg) |
| 15:02:15 | [150215-shot-wayland-season-vaduz-winter.jpg](150215-shot-wayland-season-vaduz-winter.jpg) |
| 15:02:27 | [150227-shot-wayland-season-vaduz-spring.jpg](150227-shot-wayland-season-vaduz-spring.jpg) |
| 15:02:38 | [150238-shot-wayland-season-vaduz-summer.jpg](150238-shot-wayland-season-vaduz-summer.jpg) |
| 15:02:50 | [150250-shot-wayland-season-vaduz-autumn.jpg](150250-shot-wayland-season-vaduz-autumn.jpg) |
| 15:03:02 | [150302-shot-wayland-season-fields-winter.jpg](150302-shot-wayland-season-fields-winter.jpg) |
| 15:03:13 | [150313-shot-wayland-season-fields-spring.jpg](150313-shot-wayland-season-fields-spring.jpg) |
| 15:03:25 | [150325-shot-wayland-season-fields-summer.jpg](150325-shot-wayland-season-fields-summer.jpg) |
| 15:03:37 | [150337-shot-wayland-season-fields-autumn.jpg](150337-shot-wayland-season-fields-autumn.jpg) |
| 15:03:48 | [150348-shot-wayland-season-malbun-spring.jpg](150348-shot-wayland-season-malbun-spring.jpg) |
| 15:04:00 | [150400-shot-wayland-season-malbun-winter.jpg](150400-shot-wayland-season-malbun-winter.jpg) |
| 15:05:28 | [150528-shot-season-vaduz-winter.jpg](150528-shot-season-vaduz-winter.jpg) |
| 15:05:53 | [150553-shot-season-vaduz-spring.jpg](150553-shot-season-vaduz-spring.jpg) |
| 15:06:19 | [150619-shot-season-vaduz-summer.jpg](150619-shot-season-vaduz-summer.jpg) |
| 15:06:46 | [150646-shot-season-vaduz-autumn.jpg](150646-shot-season-vaduz-autumn.jpg) |
| 15:07:02 | [150702-shot-season-fields-winter.jpg](150702-shot-season-fields-winter.jpg) |
| 15:07:18 | [150718-shot-season-fields-spring.jpg](150718-shot-season-fields-spring.jpg) |
| 15:07:34 | [150734-shot-season-fields-summer.jpg](150734-shot-season-fields-summer.jpg) |
| 15:07:50 | [150750-shot-season-fields-autumn.jpg](150750-shot-season-fields-autumn.jpg) |
| 15:08:18 | [150818-shot-season-malbun-spring.jpg](150818-shot-season-malbun-spring.jpg) |
| 15:08:45 | [150845-shot-season-malbun-winter.jpg](150845-shot-season-malbun-winter.jpg) |
| 15:11:58 | [151158-shot-season-vaduz-winter.jpg](151158-shot-season-vaduz-winter.jpg) |
| 15:12:24 | [151224-shot-season-vaduz-spring.jpg](151224-shot-season-vaduz-spring.jpg) |
| 15:12:50 | [151250-shot-season-vaduz-summer.jpg](151250-shot-season-vaduz-summer.jpg) |
| 15:13:17 | [151317-shot-season-vaduz-autumn.jpg](151317-shot-season-vaduz-autumn.jpg) |
| 15:13:39 | [151339-shot-season-fields-winter.jpg](151339-shot-season-fields-winter.jpg) |
| 15:14:01 | [151401-shot-season-fields-spring.jpg](151401-shot-season-fields-spring.jpg) |
| 15:14:23 | [151423-shot-season-fields-summer.jpg](151423-shot-season-fields-summer.jpg) |
| 15:14:45 | [151445-shot-season-fields-autumn.jpg](151445-shot-season-fields-autumn.jpg) |
| 15:15:12 | [151512-shot-season-malbun-spring.jpg](151512-shot-season-malbun-spring.jpg) |
| 15:15:39 | [151539-shot-season-malbun-winter.jpg](151539-shot-season-malbun-winter.jpg) |
| 15:42:02 | [154202-shot-wayland-weather-physics-headwind-vaduz.jpg](154202-shot-wayland-weather-physics-headwind-vaduz.jpg) |
| 15:42:28 | [154228-shot-wayland-weather-physics-tailwind-vaduz.jpg](154228-shot-wayland-weather-physics-tailwind-vaduz.jpg) |
| 15:43:03 | [154303-shot-weather-physics-headwind-vaduz.jpg](154303-shot-weather-physics-headwind-vaduz.jpg) |
| 15:43:35 | [154335-shot-weather-physics-tailwind-vaduz.jpg](154335-shot-weather-physics-tailwind-vaduz.jpg) |
| 16:23:33 | [162333-shot-wayland-forest-vaduz-autumn.jpg](162333-shot-wayland-forest-vaduz-autumn.jpg) |
| 16:23:51 | [162351-shot-wayland-forest-vaduz-winter.jpg](162351-shot-wayland-forest-vaduz-winter.jpg) |
| 16:24:10 | [162410-shot-wayland-forest-bf-autumn.jpg](162410-shot-wayland-forest-bf-autumn.jpg) |
| 16:24:30 | [162430-shot-wayland-forest-tf-pines.jpg](162430-shot-wayland-forest-tf-pines.jpg) |
| 16:26:43 | [162643-shot-forest-vaduz-autumn.jpg](162643-shot-forest-vaduz-autumn.jpg) |
| 16:27:17 | [162717-shot-forest-vaduz-winter.jpg](162717-shot-forest-vaduz-winter.jpg) |
| 16:27:43 | [162743-shot-forest-bf-autumn.jpg](162743-shot-forest-bf-autumn.jpg) |
| 16:28:13 | [162813-shot-forest-tf-pines.jpg](162813-shot-forest-tf-pines.jpg) |
| 16:28:33 | [162833-shot-wayland-forest-vaduz-autumn.jpg](162833-shot-wayland-forest-vaduz-autumn.jpg) |
| 16:28:51 | [162851-shot-wayland-forest-vaduz-winter.jpg](162851-shot-wayland-forest-vaduz-winter.jpg) |
| 16:30:25 | [163025-agent-web-web-audio-audio-locked.jpg](163025-agent-web-web-audio-audio-locked.jpg) |
| 16:30:25 | [163025-agent-web-web-audio-audio-sound.jpg](163025-agent-web-web-audio-audio-sound.jpg) |
| 16:30:48 | [163048-agent-web-web-audio-voice-echo.jpg](163048-agent-web-web-audio-voice-echo.jpg) |
| 16:30:48 | [163048-agent-web-web-audio-voice-voice-muted.jpg](163048-agent-web-web-audio-voice-voice-muted.jpg) |
| 16:30:48 | [163048-agent-web-web-audio-voice-voice.jpg](163048-agent-web-web-audio-voice-voice.jpg) |
| 16:31:45 | [163145-agent-web-web-audio-voice-echo.jpg](163145-agent-web-web-audio-voice-echo.jpg) |
| 16:31:45 | [163145-agent-web-web-audio-voice-voice-muted.jpg](163145-agent-web-web-audio-voice-voice-muted.jpg) |
| 16:31:45 | [163145-agent-web-web-audio-voice-voice.jpg](163145-agent-web-web-audio-voice-voice.jpg) |
| 16:32:42 | [163242-shot-wayland-forest-bf-autumn.jpg](163242-shot-wayland-forest-bf-autumn.jpg) |
| 16:32:56 | [163256-agent-web-web-audio-voice-echo.jpg](163256-agent-web-web-audio-voice-echo.jpg) |
| 16:32:56 | [163256-agent-web-web-audio-voice-voice-muted.jpg](163256-agent-web-web-audio-voice-voice-muted.jpg) |
| 16:32:56 | [163256-agent-web-web-audio-voice-voice.jpg](163256-agent-web-web-audio-voice-voice.jpg) |
| 16:33:18 | [163318-shot-forest-bf-autumn.jpg](163318-shot-forest-bf-autumn.jpg) |
| 16:34:45 | [163445-shot-forest-vaduz-autumn.jpg](163445-shot-forest-vaduz-autumn.jpg) |
| 16:35:19 | [163519-shot-forest-vaduz-winter.jpg](163519-shot-forest-vaduz-winter.jpg) |
| 16:55:18 | [165518-agent-web-web-audio-audio-locked.jpg](165518-agent-web-web-audio-audio-locked.jpg) |
| 16:55:18 | [165518-agent-web-web-audio-audio-sound.jpg](165518-agent-web-web-audio-audio-sound.jpg) |
| 16:55:18 | [165518-agent-web-web-audio-voice-echo.jpg](165518-agent-web-web-audio-voice-echo.jpg) |
| 16:55:18 | [165518-agent-web-web-audio-voice-voice-muted.jpg](165518-agent-web-web-audio-voice-voice-muted.jpg) |
| 16:55:18 | [165518-agent-web-web-audio-voice-voice.jpg](165518-agent-web-web-audio-voice-voice.jpg) |
| 17:51:02 | [175102-video-wayland-k12-wet-slide-video.jpg](175102-video-wayland-k12-wet-slide-video.jpg) |
| 17:51:02 | [175102-video-wayland-k12-wet-slide-video.mp4](175102-video-wayland-k12-wet-slide-video.mp4) |
| 17:51:14 | [175114-shot-wayland-k12-wet-slide-down.jpg](175114-shot-wayland-k12-wet-slide-down.jpg) |
| 17:51:23 | [175123-shot-wayland-k12d-warning-arrow.jpg](175123-shot-wayland-k12d-warning-arrow.jpg) |
| 17:51:35 | [175135-shot-wayland-k13-rider-list.jpg](175135-shot-wayland-k13-rider-list.jpg) |
| 17:52:58 | [175258-video-wayland-g5-flyover-vaduz-video.jpg](175258-video-wayland-g5-flyover-vaduz-video.jpg) |
| 17:52:58 | [175258-video-wayland-g5-flyover-vaduz-video.mp4](175258-video-wayland-g5-flyover-vaduz-video.mp4) |
| 18:01:46 | [180146-shot-wayland-workshop-bench-catalogue.jpg](180146-shot-wayland-workshop-bench-catalogue.jpg) |
| 18:07:08 | [180708-shot-wayland-workshop-bench-plus-bought.jpg](180708-shot-wayland-workshop-bench-plus-bought.jpg) |
| 18:07:22 | [180722-shot-wayland-lights-night-fitted.jpg](180722-shot-wayland-lights-night-fitted.jpg) |
| 18:07:36 | [180736-shot-wayland-lights-night-none.jpg](180736-shot-wayland-lights-night-none.jpg) |
| 18:07:54 | [180754-video-wayland-k12-wet-slide-video.jpg](180754-video-wayland-k12-wet-slide-video.jpg) |
| 18:07:54 | [180754-video-wayland-k12-wet-slide-video.mp4](180754-video-wayland-k12-wet-slide-video.mp4) |
| 18:08:01 | [180801-shot-wayland-workshop-mirror-idle-kit-colours.jpg](180801-shot-wayland-workshop-mirror-idle-kit-colours.jpg) |
| 18:08:13 | [180813-shot-wayland-k12-wet-slide-down.jpg](180813-shot-wayland-k12-wet-slide-down.jpg) |
| 18:08:21 | [180821-shot-wayland-k12d-warning-arrow.jpg](180821-shot-wayland-k12d-warning-arrow.jpg) |
| 18:08:27 | [180827-shot-wayland-workshop-gloves-colour.jpg](180827-shot-wayland-workshop-gloves-colour.jpg) |
| 18:08:33 | [180833-shot-wayland-k13-rider-list.jpg](180833-shot-wayland-k13-rider-list.jpg) |
| 18:09:59 | [180959-video-wayland-workshop-bench-video.jpg](180959-video-wayland-workshop-bench-video.jpg) |
| 18:09:59 | [180959-video-wayland-workshop-bench-video.mp4](180959-video-wayland-workshop-bench-video.mp4) |
| 18:10:03 | [181003-video-wayland-g5-flyover-vaduz-video.jpg](181003-video-wayland-g5-flyover-vaduz-video.jpg) |
| 18:10:03 | [181003-video-wayland-g5-flyover-vaduz-video.mp4](181003-video-wayland-g5-flyover-vaduz-video.mp4) |
| 18:17:34 | [181734-shot-workshop-bench-catalogue.jpg](181734-shot-workshop-bench-catalogue.jpg) |
| 18:17:36 | [181736-video-k12-wet-slide-video.jpg](181736-video-k12-wet-slide-video.jpg) |
| 18:17:36 | [181736-video-k12-wet-slide-video.mp4](181736-video-k12-wet-slide-video.mp4) |
| 18:18:00 | [181800-shot-k12-wet-slide-down.jpg](181800-shot-k12-wet-slide-down.jpg) |
| 18:18:18 | [181818-shot-workshop-bench-plus-bought.jpg](181818-shot-workshop-bench-plus-bought.jpg) |
| 18:18:29 | [181829-shot-k12d-warning-arrow.jpg](181829-shot-k12d-warning-arrow.jpg) |
| 18:19:05 | [181905-shot-k13-rider-list.jpg](181905-shot-k13-rider-list.jpg) |
| 18:19:06 | [181906-shot-lights-night-fitted.jpg](181906-shot-lights-night-fitted.jpg) |
| 18:20:03 | [182003-shot-lights-night-none.jpg](182003-shot-lights-night-none.jpg) |
| 18:21:04 | [182104-shot-workshop-mirror-idle-kit-colours.jpg](182104-shot-workshop-mirror-idle-kit-colours.jpg) |
| 18:22:02 | [182202-shot-workshop-gloves-colour.jpg](182202-shot-workshop-gloves-colour.jpg) |
| 18:30:30 | [183030-video-g5-flyover-vaduz-video.jpg](183030-video-g5-flyover-vaduz-video.jpg) |
| 18:30:30 | [183030-video-g5-flyover-vaduz-video.mp4](183030-video-g5-flyover-vaduz-video.mp4) |
| 18:31:48 | [183148-shot-workshop-bench-plus-bought.jpg](183148-shot-workshop-bench-plus-bought.jpg) |
| 18:32:23 | [183223-shot-k12-wet-slide-down.jpg](183223-shot-k12-wet-slide-down.jpg) |
| 18:32:48 | [183248-shot-k12d-warning-arrow.jpg](183248-shot-k12d-warning-arrow.jpg) |
| 18:33:02 | [183302-shot-workshop-bench-plus-bought.jpg](183302-shot-workshop-bench-plus-bought.jpg) |
| 18:33:14 | [183314-shot-k13-rider-list.jpg](183314-shot-k13-rider-list.jpg) |
| 18:33:55 | [183355-shot-wayland-workshop-bench-plus-bought.jpg](183355-shot-wayland-workshop-bench-plus-bought.jpg) |
| 18:34:41 | [183441-shot-workshop-bench-plus-bought.jpg](183441-shot-workshop-bench-plus-bought.jpg) |
| 18:42:03 | [184203-agent-web-web-audio-livekit-livekit-ada.jpg](184203-agent-web-web-audio-livekit-livekit-ada.jpg) |
| 18:42:03 | [184203-agent-web-web-audio-livekit-livekit-bea.jpg](184203-agent-web-web-audio-livekit-livekit-bea.jpg) |
| 19:44:20 | [194420-agent-web-web-downloads-ocw-2026-11-14-19-45-01.jpg](194420-agent-web-web-downloads-ocw-2026-11-14-19-45-01.jpg) |
| 19:44:20 | [194420-agent-web-web-downloads-ocw-2026-11-14-19-45-05.webm](194420-agent-web-web-downloads-ocw-2026-11-14-19-45-05.webm) |
| 19:44:20 | [194420-agent-web-web-gamepad.jpg](194420-agent-web-web-gamepad.jpg) |
| 19:44:20 | [194420-agent-web-web-hud.jpg](194420-agent-web-web-hud.jpg) |
| 19:44:20 | [194420-agent-web-web-menu.jpg](194420-agent-web-web-menu.jpg) |
| 19:44:20 | [194420-agent-web-web-riders.jpg](194420-agent-web-web-riders.jpg) |
| 19:44:20 | [194420-agent-web-web-settings.jpg](194420-agent-web-web-settings.jpg) |
| 19:44:20 | [194420-agent-web-web-weather.jpg](194420-agent-web-web-weather.jpg) |
| 19:44:20 | [194420-agent-web-web.jpg](194420-agent-web-web.jpg) |
| 20:03:12 | [200312-agent-web-web-downloads-ocw-2026-11-14-20-03-53.jpg](200312-agent-web-web-downloads-ocw-2026-11-14-20-03-53.jpg) |
| 20:03:12 | [200312-agent-web-web-downloads-ocw-2026-11-14-20-03-56.webm](200312-agent-web-web-downloads-ocw-2026-11-14-20-03-56.webm) |
| 20:03:12 | [200312-agent-web-web-gamepad.jpg](200312-agent-web-web-gamepad.jpg) |
| 20:03:12 | [200312-agent-web-web-hud.jpg](200312-agent-web-web-hud.jpg) |
| 20:03:12 | [200312-agent-web-web-menu.jpg](200312-agent-web-web-menu.jpg) |
| 20:03:12 | [200312-agent-web-web-riders.jpg](200312-agent-web-web-riders.jpg) |
| 20:03:12 | [200312-agent-web-web-settings.jpg](200312-agent-web-web-settings.jpg) |
| 20:03:12 | [200312-agent-web-web-weather.jpg](200312-agent-web-web-weather.jpg) |
| 20:03:12 | [200312-agent-web-web.jpg](200312-agent-web-web.jpg) |
| 20:10:30 | [201030-agent-web-web-audio-audio-locked.jpg](201030-agent-web-web-audio-audio-locked.jpg) |
| 20:10:30 | [201030-agent-web-web-audio-audio-sound.jpg](201030-agent-web-web-audio-audio-sound.jpg) |
| 20:10:30 | [201030-web-web-audio-audio.mic.wav](201030-web-web-audio-audio.mic.wav) |
| 20:22:23 | [202223-agent-web-web-downloads-ocw-2026-11-14-20-23-06.jpg](202223-agent-web-web-downloads-ocw-2026-11-14-20-23-06.jpg) |
| 20:22:23 | [202223-agent-web-web-downloads-ocw-2026-11-14-20-23-10.webm](202223-agent-web-web-downloads-ocw-2026-11-14-20-23-10.webm) |
| 20:22:23 | [202223-agent-web-web-gamepad.jpg](202223-agent-web-web-gamepad.jpg) |
| 20:22:23 | [202223-agent-web-web-hud.jpg](202223-agent-web-web-hud.jpg) |
| 20:22:23 | [202223-agent-web-web-menu.jpg](202223-agent-web-web-menu.jpg) |
| 20:22:23 | [202223-agent-web-web-riders.jpg](202223-agent-web-web-riders.jpg) |
| 20:22:23 | [202223-agent-web-web-settings.jpg](202223-agent-web-web-settings.jpg) |
| 20:22:23 | [202223-agent-web-web-weather.jpg](202223-agent-web-web-weather.jpg) |
| 20:22:23 | [202223-agent-web-web.jpg](202223-agent-web-web.jpg) |
| 20:52:39 | [205239-agent-web-web-webgl2.jpg](205239-agent-web-web-webgl2.jpg) |
| 21:11:48 | [211148-agent-web-web-webgl2.jpg](211148-agent-web-web-webgl2.jpg) |
| 21:21:32 | [212132-agent-scenes-t4b-wayland-look-houses-vaduz.jpg](212132-agent-scenes-t4b-wayland-look-houses-vaduz.jpg) |
| 21:21:43 | [212143-agent-scenes-t4b-wayland-look-houses-triesenberg.jpg](212143-agent-scenes-t4b-wayland-look-houses-triesenberg.jpg) |
| 21:21:55 | [212155-agent-scenes-t4b-wayland-look-houses-malbun.jpg](212155-agent-scenes-t4b-wayland-look-houses-malbun.jpg) |
| 21:22:11 | [212211-agent-scenes-t4b-wayland-look-houses-bf-stmaergen.jpg](212211-agent-scenes-t4b-wayland-look-houses-bf-stmaergen.jpg) |
| 21:22:27 | [212227-agent-scenes-t4b-wayland-look-houses-bf-hofsgrund.jpg](212227-agent-scenes-t4b-wayland-look-houses-bf-hofsgrund.jpg) |
| 21:22:43 | [212243-agent-scenes-t4b-wayland-look-houses-tf-orotava.jpg](212243-agent-scenes-t4b-wayland-look-houses-tf-orotava.jpg) |
| 21:22:57 | [212257-agent-scenes-t4b-wayland-look-houses-tf-gigantes.jpg](212257-agent-scenes-t4b-wayland-look-houses-tf-gigantes.jpg) |
| 21:23:09 | [212309-agent-scenes-t4b-wayland-look-houses-vaduz-polygons.jpg](212309-agent-scenes-t4b-wayland-look-houses-vaduz-polygons.jpg) |
| 21:23:20 | [212320-agent-scenes-t4b-wayland-look-houses-vaduz-cathedral.jpg](212320-agent-scenes-t4b-wayland-look-houses-vaduz-cathedral.jpg) |
| 21:23:32 | [212332-agent-scenes-t4b-wayland-look-houses-triesen-church.jpg](212332-agent-scenes-t4b-wayland-look-houses-triesen-church.jpg) |
| 21:23:43 | [212343-agent-scenes-t4b-wayland-look-houses-triesenberg-balconies.jpg](212343-agent-scenes-t4b-wayland-look-houses-triesenberg-balconies.jpg) |
| 21:23:55 | [212355-agent-scenes-t4b-wayland-look-houses-malbun-dormers.jpg](212355-agent-scenes-t4b-wayland-look-houses-malbun-dormers.jpg) |
| 21:24:10 | [212410-agent-scenes-t4b-wayland-look-houses-bf-farms-close.jpg](212410-agent-scenes-t4b-wayland-look-houses-bf-farms-close.jpg) |
| 21:24:40 | [212440-agent-scenes-t4b-x11-look-houses-vaduz.jpg](212440-agent-scenes-t4b-x11-look-houses-vaduz.jpg) |
| 21:25:10 | [212510-agent-scenes-t4b-x11-look-houses-triesenberg.jpg](212510-agent-scenes-t4b-x11-look-houses-triesenberg.jpg) |
| 21:25:30 | [212530-agent-scenes-t4b-x11-look-houses-malbun.jpg](212530-agent-scenes-t4b-x11-look-houses-malbun.jpg) |
| 21:25:54 | [212554-agent-scenes-t4b-x11-look-houses-bf-stmaergen.jpg](212554-agent-scenes-t4b-x11-look-houses-bf-stmaergen.jpg) |
| 21:26:19 | [212619-agent-scenes-t4b-x11-look-houses-bf-hofsgrund.jpg](212619-agent-scenes-t4b-x11-look-houses-bf-hofsgrund.jpg) |
| 21:26:41 | [212641-agent-scenes-t4b-x11-look-houses-tf-orotava.jpg](212641-agent-scenes-t4b-x11-look-houses-tf-orotava.jpg) |
| 21:27:04 | [212704-agent-scenes-t4b-x11-look-houses-tf-gigantes.jpg](212704-agent-scenes-t4b-x11-look-houses-tf-gigantes.jpg) |
| 21:27:23 | [212723-agent-scenes-t4b-x11-look-houses-vaduz-polygons.jpg](212723-agent-scenes-t4b-x11-look-houses-vaduz-polygons.jpg) |
| 21:27:41 | [212741-agent-scenes-t4b-x11-look-houses-vaduz-cathedral.jpg](212741-agent-scenes-t4b-x11-look-houses-vaduz-cathedral.jpg) |
| 21:28:08 | [212808-agent-scenes-t4b-x11-look-houses-triesen-church.jpg](212808-agent-scenes-t4b-x11-look-houses-triesen-church.jpg) |
| 21:28:32 | [212832-agent-scenes-t4b-x11-look-houses-triesenberg-balconies.jpg](212832-agent-scenes-t4b-x11-look-houses-triesenberg-balconies.jpg) |
| 21:28:50 | [212850-agent-scenes-t4b-x11-look-houses-malbun-dormers.jpg](212850-agent-scenes-t4b-x11-look-houses-malbun-dormers.jpg) |
| 21:29:16 | [212916-agent-scenes-t4b-x11-look-houses-bf-farms-close.jpg](212916-agent-scenes-t4b-x11-look-houses-bf-farms-close.jpg) |
| 21:38:13 | [213813-agent-web-web.jpg](213813-agent-web-web.jpg) |
| 21:47:42 | [214742-agent-web-web-webgl2-menu.jpg](214742-agent-web-web-webgl2-menu.jpg) |
| 21:47:42 | [214742-agent-web-web-webgl2.jpg](214742-agent-web-web-webgl2.jpg) |
| 21:48:38 | [214838-agent-web-web-compare-webgl2-graphics.jpg](214838-agent-web-web-compare-webgl2-graphics.jpg) |
| 21:48:38 | [214838-agent-web-web-compare-webgl2-low.jpg](214838-agent-web-web-compare-webgl2-low.jpg) |
| 21:48:38 | [214838-agent-web-web-compare-webgpu-high.jpg](214838-agent-web-web-compare-webgpu-high.jpg) |
| 21:48:38 | [214838-agent-web-web-compare.jpg](214838-agent-web-web-compare.jpg) |
| 22:03:37 | [220337-agent-web-web-compare-webgl2-graphics.jpg](220337-agent-web-web-compare-webgl2-graphics.jpg) |
| 22:03:37 | [220337-agent-web-web-compare-webgl2-low.jpg](220337-agent-web-web-compare-webgl2-low.jpg) |
| 22:03:37 | [220337-agent-web-web-compare-webgpu-high.jpg](220337-agent-web-web-compare-webgpu-high.jpg) |
| 22:03:37 | [220337-agent-web-web-compare.jpg](220337-agent-web-web-compare.jpg) |
| 23:45:50 | [234550-shot-wayland-helmet-mirror-road.jpg](234550-shot-wayland-helmet-mirror-road.jpg) |
| 23:45:59 | [234559-shot-helmet-mirror-road.jpg](234559-shot-helmet-mirror-road.jpg) |
| 23:46:17 | [234617-shot-wayland-helmet-mirror-road-side.jpg](234617-shot-wayland-helmet-mirror-road-side.jpg) |
| 23:46:33 | [234633-scenes-a26-cornering.jpg](234633-scenes-a26-cornering.jpg) |
| 23:46:33 | [234633-scenes-a26-foot-down-video.jpg](234633-scenes-a26-foot-down-video.jpg) |
| 23:46:33 | [234633-scenes-a26-foot-down-video.mp4](234633-scenes-a26-foot-down-video.mp4) |
| 23:46:33 | [234633-scenes-a26-foot-down.jpg](234633-scenes-a26-foot-down.jpg) |
| 23:46:33 | [234633-scenes-a26-uturn-look.jpg](234633-scenes-a26-uturn-look.jpg) |
| 23:46:33 | [234633-scenes-a26-uturn-loop.jpg](234633-scenes-a26-uturn-loop.jpg) |
| 23:46:33 | [234633-scenes-a26-uturn-video.jpg](234633-scenes-a26-uturn-video.jpg) |
| 23:46:33 | [234633-scenes-a26-uturn-video.mp4](234633-scenes-a26-uturn-video.mp4) |
| 23:46:33 | [234633-scenes-a26-victory-video.jpg](234633-scenes-a26-victory-video.jpg) |
| 23:46:33 | [234633-scenes-a26-victory-video.mp4](234633-scenes-a26-victory-video.mp4) |
| 23:46:33 | [234633-scenes-a26-victory.jpg](234633-scenes-a26-victory.jpg) |
| 23:46:33 | [234633-scenes-k6-event-rules.jpg](234633-scenes-k6-event-rules.jpg) |
| 23:46:33 | [234633-scenes-k6-organiser-rules-menu.jpg](234633-scenes-k6-organiser-rules-menu.jpg) |
| 23:46:33 | [234633-scenes-k6-results-rules.jpg](234633-scenes-k6-results-rules.jpg) |
| 23:46:33 | [234633-shot-helmet-mirror-road-side.jpg](234633-shot-helmet-mirror-road-side.jpg) |
| 23:46:43 | [234643-shot-wayland-helmet-mirror-light.jpg](234643-shot-wayland-helmet-mirror-light.jpg) |
| 23:47:10 | [234710-shot-helmet-mirror-light.jpg](234710-shot-helmet-mirror-light.jpg) |
| 23:47:10 | [234710-shot-wayland-helmet-mirror-light-side.jpg](234710-shot-wayland-helmet-mirror-light-side.jpg) |
| 23:47:36 | [234736-shot-wayland-helmet-mirror-aero-road.jpg](234736-shot-wayland-helmet-mirror-aero-road.jpg) |
| 23:47:49 | [234749-shot-helmet-mirror-light-side.jpg](234749-shot-helmet-mirror-light-side.jpg) |
| 23:48:03 | [234803-shot-wayland-helmet-mirror-aero-road-side.jpg](234803-shot-wayland-helmet-mirror-aero-road-side.jpg) |
| 23:48:26 | [234826-shot-helmet-mirror-aero-road.jpg](234826-shot-helmet-mirror-aero-road.jpg) |
| 23:48:29 | [234829-shot-wayland-helmet-mirror-tt.jpg](234829-shot-wayland-helmet-mirror-tt.jpg) |
| 23:48:57 | [234857-shot-wayland-helmet-mirror-tt-side.jpg](234857-shot-wayland-helmet-mirror-tt-side.jpg) |
| 23:49:06 | [234906-shot-helmet-mirror-aero-road-side.jpg](234906-shot-helmet-mirror-aero-road-side.jpg) |
| 23:49:25 | [234925-shot-wayland-helmet-mirror-cap.jpg](234925-shot-wayland-helmet-mirror-cap.jpg) |
| 23:49:35 | [234935-shot-wayland-helmet-road-behind.jpg](234935-shot-wayland-helmet-road-behind.jpg) |
| 23:49:44 | [234944-shot-helmet-mirror-tt.jpg](234944-shot-helmet-mirror-tt.jpg) |
| 23:49:46 | [234946-shot-wayland-helmet-tt-position.jpg](234946-shot-wayland-helmet-tt-position.jpg) |
| 23:50:02 | [235002-shot-wayland-helmet-group-chase.jpg](235002-shot-wayland-helmet-group-chase.jpg) |
| 23:50:23 | [235023-shot-helmet-mirror-tt-side.jpg](235023-shot-helmet-mirror-tt-side.jpg) |
| 23:51:12 | [235112-shot-helmet-mirror-cap.jpg](235112-shot-helmet-mirror-cap.jpg) |
| 23:51:49 | [235149-shot-helmet-road-behind.jpg](235149-shot-helmet-road-behind.jpg) |
| 23:52:24 | [235224-shot-helmet-tt-position.jpg](235224-shot-helmet-tt-position.jpg) |
| 23:52:54 | [235254-shot-helmet-group-chase.jpg](235254-shot-helmet-group-chase.jpg) |
