# OCW build diary – day 1, 2026-11-11

155 commits (35 merges), +66673 / −3436 lines, 160 images, 49 videos, 0 sounds.

## Commits

| Time | Commit | |
|---|---|---|
| 00:38 | `836996e` | Add specification, proposals and CLAUDE.md |
| 00:56 | `2e90754` | Add junction steering, platform rollout order and web companion |
| 00:59 | `bd6f171` | Merge proposals into specification v1.1 |
| 08:15 | `8c831c5` | Add testing strategy to specification v1.2 (spec §11.4) |
| 08:15 | `c051398` | Phase 1 scaffolding: workspace, local client/server, test route, console |
| 08:30 | `05be99c` | Complete Phase 1 unit tests and add Makefile (spec §11.4, §14) |
| 08:51 | `8238b65` | Add Wayland smoke test and headless video recording on X11 and Wayland |
| 09:16 | `f6b4f30` | Road-conformed terrain for the test loop (spec §4.3) |
| 09:22 | `ae2f5d3` | Smooth route heading and frame-exact video capture |
| 09:27 | `c42d787` | Add machine configuration template for remote build/test machines |
| 09:31 | `8881015` | Archive test screenshots and videos in test-output/ |
| 09:41 | `8cbd005` | Phase 2: BLE fitness profile parsing and FTMS control point (spec §3.4, §5.3) |
| 09:43 | `12e4501` | Phase 2: SIM and ERG trainer control logic (spec §5.3) |
| 10:01 | `f0ff0a5` | Phase 2: Bluetooth LE backend with btleplug (spec §3.4) |
| 10:25 | `f2dec04` | Remote build helper and complete Linux package list |
| 10:27 | `b7d1c87` | Split make deps per platform: deps-linux, deps-windows, deps-macos |
| 10:40 | `219f3ab` | Graphics engine spike, iteration 1 (spec §3.1.1) |
| 10:48 | `4981909` | remote.sh: per-branch remote checkout, config from the main worktree |
| 11:38 | `3c00e08` | Spike iteration 2: splat terrain material, video capture, winding road |
| 11:39 | `38b1557` | Spec v1.3: graphics engine spike results and decisions (§3.1.1) |
| 11:39 | `e1b1b77` | WIP Phase 2: device integration, trainer control loop, drafting |
| 11:50 | `88c4dd2` | Phase 2: BLE hardware-in-the-loop tool (scan, services, monitor, ride) |
| 12:07 | `7fe1b93` | Phase 2: Wahoo trainer control for KICKRs without FTMS; one control write at a time |
| 12:18 | `99b09aa` | drive.sh: GPU weston renderer on machines with a GPU render node |
| 13:25 | `5a5f9c9` | Desktop: hilly test route, pedalling legs, mouse orbit camera, steadier shadows |
| 13:53 | `8e30d4c` | HUD: road gradient in the main line |
| 14:01 | `cea04e3` (merge) | Merge phase2-devices: Bluetooth trainers, trainer control, drafting |
| 14:11 | `0b099c9` | Own rider in #FF1472 with a cycling cap; smooth rendering between physics steps |
| 14:15 | `12e57a7` | Steer the rider with the arrow keys (spec §6.1) |
| 14:23 | `85a7b28` | Kiosk for test machines: --monitor, deploy script, auto-restart watchdog |
| 14:38 | `41c0192` | Phase 3, step 1: OSM import and road graph for the pilot region (spec §4.4) |
| 14:38 | `340d000` | Cargo.lock with the dependencies resolved on marge; remote.sh copies it back after each run |
| 14:45 | `0824035` | Phase 3, step 2: elevation model and road profiles (spec §4.3) |
| 15:02 | `5b26629` | Phase 3 preview: ride a real route (Vaduz - Malbun) with its terrain |
| 15:02 | `e6c0916` | deploy-test.sh: also install the built route files |
| 15:09 | `1ab675c` | Up accelerates, down stops; fix the start arch bar |
| 15:13 | `8d4dd3f` | Up adds 25 W per press; F11 cycles screens |
| 15:17 | `3dadedd` | Phase 3, step 3: world package with road-conformed terrain, streamed in the client |
| 15:20 | `ffde42a` | Up raises the rider's power with any device; camera stays above the ground |
| 15:34 | `b6ebd62` | World package: road strips keep the profile height on winding roads; terrain cut under roads |
| 16:18 | `7ed34e6` | Elevation filter along whole roads; junctions inside bridges follow the bridge |
| 16:21 | `0baa61d` | Riders lean into curves; bots ride at different strengths |
| 16:22 | `2d38015` | run-ocw skill: X11 modes fail under Xvfb on GPU machines; use Wayland |
| 16:34 | `e71a454` | Lean steering with the arrow keys: the rider tilts and rides a curve |
| 16:38 | `02e4f69` | Bots climb at least 10 km/h instead of getting stuck |
| 16:40 | `c35cb9d` | Random start point (on by default) for everyone on the route |
| 16:48 | `f4b27dd` | Autodrive, manual lean steering, road edge, wild mode, gamepad; surfaces |
| 17:05 | `96a9178` | Camera follows the rider off the road without clipping; bike sits on both wheels |
| 17:13 | `afb52f3` | App logo: a unicorn riding a bike over a rainbow; window icon, desktop entry, AppImage |
| 17:25 | `b5b37cc` | scripts/setup-windows-buildserver.ps1: OpenSSH server and a local build user |
| 17:36 | `4f2353b` | Local projection moves to ocw-world::geo; routes know their geographic origin |
| 17:36 | `5849145` | ocw-fit: FIT activity writer (spec §2.8, §14 Phase 2) |
| 17:36 | `2c1aef8` | Ride recording, FIT save and Strava upload / sync (spec §2.8, v1.6) |
| 17:39 | `070f184` (merge) | Merge ride recording, FIT export and Strava upload/sync |
| 17:46 | `7fd46d3` | remote.sh: sync to macOS, whose openrsync lacks --delete-missing-args |
| 17:47 | `15d89d0` (merge) | Merge remote.sh support for macOS build machines |
| 17:58 | `2a88575` | CI: apt step fails fast and retries when the Ubuntu archive is unreachable |
| 18:03 | `ffa9b12` | Add artwork/build-history: screenshots and videos from development |
| 18:13 | `310655b` | README: controls and how to connect a gamepad; remote.sh caps build jobs for branch checkouts |
| 18:15 | `ee28953` | Add release/0.1.0-15d89d0: Linux x86_64 and macOS arm64 builds |
| 18:16 | `add4c06` | Desktop: gate the Linux desktop integration so macOS/Windows clippy passes |
| 18:16 | `7928bfd` (merge) | Merge build history (screenshots, videos) and release builds for Linux and macOS |
| 18:16 | `7d6b8f8` (merge) | Merge CI apt retry/timeout and the macOS/Windows clippy fix |
| 18:18 | `aebd0c2` | README: riding modes, trainer and devices, rides and Strava, all command-line options, console commands and tools |
| 18:30 | `7e78642` | World package format 2 with the road graph; one-way roads |
| 18:30 | `a83cffa` | Free-ride paths through the road graph: Track, RoadNetwork, RidePath |
| 18:30 | `117ae0d` | Free ride in the simulation, protocol and server |
| 18:30 | `6bf6b4e` | Free ride in the desktop client: junction overlay, keys, --free-ride |
| 18:30 | `8e915d8` | Spec 1.6: free ride, junction keys, U-turn loop, package format 2 |
| 18:30 | `23c1880` | Steering: stop scraping along the road edge, let the lean keys cross upright (spec §5.2.1, v1.7) |
| 18:33 | `a1edc38` (merge) | Merge free ride on the road graph: junctions, U-turns, dead ends (spec 1.7) |
| 18:34 | `d3da4de` (merge) | Merge steering fixes: edge scraping only when pushing into it, lean key pauses at upright then continues (spec 1.8) |
| 18:35 | `473c91c` | Spec 1.9: catch-up of all implemented decisions; README: free ride |
| 18:50 | `63910f2` | R restarts everything, C resets the camera, V records a video; gamepad up/down |
| 19:18 | `3680f76` | Spec: in-game menu plan (§3.2.1): menu tree, input and pause, settings file, phases; audio output device (§3.3) |
| 19:18 | `6b0321b` (merge) | Merge in-game menu plan (spec §3.2.1, §3.3, §14) |
| 19:18 | `2c5f020` | Spec 2.1: version entry for the in-game menu plan |
| 19:25 | `532c2d3` | Spec 2.2: menu decisions (no pause, Esc never quits, rider profiles with weight) |
| 19:56 | `3793cc8` | Wild mode on switches autodrive off (spec §5.2.1) |
| 20:13 | `028c1fb` | release: macOS app (DMG) built from 532c2d3 |
| 20:16 | `539c33f` | HUD: red "REC m:ss" while recording a video, "Video saved: <file>" after V stops it |
| 20:20 | `edc1758` | Gamepad triggers calibrated to their resting value: a connected pad no longer brakes the rider |
| 20:27 | `3b1d68d` | ocw-world: GPX import, map matching, graph routes, ROUT section, border links |
| 20:27 | `fae1547` | Route rides on the road graph, rejoin, package check in Hello/Welcome |
| 20:27 | `61ffde3` | Spec and README: routes on the road graph, GPX import, U-turn width, package check, borders |
| 20:37 | `31fa7d1` | macOS app bundle, DMG and zip (spec §11.3) |
| 20:38 | `46edf03` (merge) | Merge macOS app bundle packaging (scripts/build-macos-app.sh, make macos-app, CI job); spec 2.3 |
| 20:49 | `0780fe6` | Second Linux build machine (leela, Fedora): make deps for dnf; remote.sh build slots |
| 20:51 | `b1adff0` | make deps-linux: per-distribution targets with detection (Debian/Ubuntu, Fedora, Arch) |
| 20:55 | `687aaf9` | make deps-linux-fedora: install ffmpeg without conflicting with RPM Fusion |
| 21:05 | `afedca1` (merge) | Merge main into routes branch |
| 21:05 | `8f83ade` | World package: land-use (LUSE) and buildings (BLDG) sections (spec §4.6, §4.7) |
| 21:05 | `7652c30` | worldgen: OSM land use, water surfaces and buildings (spec §4.6) |
| 21:05 | `3ae8f49` | Render land use: colour maps, flat water, trees by land use, buildings |
| 21:05 | `fa8b807` | world-builder: package writes land use and buildings, check reports them; spec and README |
| 21:05 | `e6a5141` (merge) | Merge main into land use and buildings |
| 21:09 | `acfb76e` | README: build server setup (Linux, macOS, Windows incl. the PowerShell execution policy) |
| 21:31 | `bd3784c` | remote.sh: Windows build machines (tar sync, PowerShell run, fetch) |
| 21:32 | `4091b74` | Devices: a missing --devices-file falls back to the default file; log it |
| 21:45 | `b7ca637` | World package manifests, Ed25519 signatures, cache and download (spec §4.7, §12) |
| 21:45 | `7fb3cf6` | world-builder: keys, sign, verify, publish, diff; OSM XML fixture reader |
| 21:45 | `65c4dcf` | Desktop and server: --world PATH/ID with verification, --fetch-world, worlds.* console commands |
| 21:45 | `79e352f` | End-to-end world pipeline test from checked-in OSM fixtures (spec §14 Phase 3) |
| 21:45 | `1fef50e` | Spec §4.7/§12, README: package signing, trust policy, cache, download |
| 21:45 | `5e5ff14` (merge) | Merge Phase 3 B: GPX import and map matching, routes on the road graph, route arrows with leave/rejoin, U-turn width, package check, border matching |
| 21:48 | `600b44d` (merge) | Merge Phase 3 C: signed, versioned, cached world packages; end-to-end test (spec 2.4, 2.5) |
| 21:51 | `5c65626` | README: Windows build server toolchain (VS Build Tools as admin, rustup as the build user) |
| 21:53 | `a6a3c2c` | tools/build-dashboard: web page with build server load and tasks |
| 21:53 | `5fd5c71` (merge) | Merge build server dashboard (tools/build-dashboard) |
| 22:08 | `e0b496f` | make deps-macos: print how to install Homebrew and ffmpeg when brew is missing |
| 22:08 | `407a769` | Video recording: find ffmpeg outside PATH (app started from Finder) |
| 22:15 | `c3e4624` | make deps-macos: find brew in /opt/homebrew or /usr/local when not on PATH |
| 22:15 | `22ca43c` | Land use: clippy fixes, connected thin streams, greener settlements, tile meshing benchmark |
| 22:20 | `98779cf` (merge) | Merge main (routes ROUT, signed packages and manifest) into land use and buildings |
| 22:20 | `52ec918` (merge) | Merge Phase 3 A: land use, water and buildings from OSM (spec 2.6) |
| 22:25 | `6f43702` | WIP gps |
| 22:25 | `00ca28f` (merge) | Merge main |
| 22:32 | `b17141a` | make deps-macos: skip brew when ffmpeg is installed; a failed brew install only warns |
| 22:42 | `df28850` (merge) | Merge main |
| 22:42 | `aeb4f18` | WIP build id |
| 22:42 | `78c6341` (merge) | Merge main |
| 22:42 | `f4b1ffc` | release: macOS app (DMG) built from b17141a |
| 22:43 | `2e55b3f` (merge) | Merge macOS: ffmpeg found from Finder, deps-macos with Homebrew, DMG 0.1.0-b17141a |
| 22:43 | `746df95` | WIP smooth video |
| 22:44 | `98279b6` (merge) | Merge main into video HUD branch |
| 22:46 | `aebde24` | CLAUDE.md: the Mac build machine takes platform-independent checks too |
| 22:48 | `6c61188` | Kiosk: default arguments file; deploy-test builds the pilot world package |
| 22:48 | `9ba6223` (merge) | Merge kiosk args file and world package deploy (ocw-desktop.args, deploy-test.sh) |
| 22:54 | `086a253` | Build dashboard: task markers, Windows CPU load and build slots |
| 22:58 | `e75af8c` | HUD: rider GPS position bottom-left, rider.gps; wrap long HUD lines to the window (spec §3.2) |
| 22:59 | `1d312d1` (merge) | Merge branch 'worktree-agent-ac86d7155a2f5ab78' into worktree-agent-a92c0d2a727e217b2 |
| 23:02 | `da1e26d` | make deps-appimage: appimagetool to ~/.local/bin; FUSE 2 in the Linux deps |
| 23:05 | `faf8420` | Build id in the HUD's lower right corner, --version and startup log |
| 23:05 | `3c1df06` (merge) | Merge HUD: rider GPS position bottom-left, rider.gps; wrap long HU |
| 23:05 | `1177963` (merge) | Merge wild mode turns autodrive off; appimagetool in the Linux deps |
| 23:06 | `adab164` (merge) | Merge build id in the HUD corner, --version, OCW_BUILD_ID through remote.sh |
| 23:26 | `d7eaf99` | Build slots per machine, any-slot waiting, macOS slots; dashboard queue |
| 23:28 | `fc136b7` (merge) | Merge build slots per machine (2 Linux, 1 Windows, 1 macOS), any-slot waiting; dashboard queue view |
| 23:29 | `624bad2` | WIP in-game menu M1 |
| 23:29 | `24ced5d` (merge) | Merge main |
| 23:29 | `9f5e8e7` | ocw-worldstore: resolve package files next to a Windows manifest path |
| 23:29 | `ab4dee4` (merge) | Merge branch 'main' into worktree-agent-a29fa2740ef9e3048 |
| 23:32 | `e8bf0c0` | Video: per-tick screenshots in flight, never block the game (in progress) |
| 23:32 | `5a525d3` (merge) | Merge main (remote.sh slots) into video branch |
| 23:34 | `857c40a` | remote.sh: Windows exit code for failed cmdlets; per-machine build jobs |
| 23:41 | `3c766c0` | Camera: solid vs see-through objects; trees fade instead of moving the camera (spec §3.2) |
| 23:42 | `196b5c0` (merge) | Merge camera classes: trees see-through (dithered fade), buildings as solid camera obstacles |
| 23:44 | `56d61dc` | Video: smooth recordings – a screenshot per 1/25 s tick, placed at its request time, never blocking the game |
| 23:45 | `f67085e` (merge) | Merge video REC indicator in the HUD and smooth live recording (fixed-rate capture) |
| 23:45 | `6fd9acc` | More bots: 10 by default, start formation in rows, 12 bot strengths (spec §6.1) |
| 23:46 | `a1df35e` (merge) | Merge 10 bots by default, start formation in rows behind the start, 12 bot strengths |
| 23:48 | `00b8c84` | README: Windows build machine notes (build times, memory, jobs, PowerShell) |
| 23:51 | `827792a` (merge) | Merge Windows build: worldstore paths with backslashes, remote.sh Windows exit codes, per-machine jobs key |
| 23:53 | `fe99d35` | In-game menu M1: input contexts, menu shell, autodrive while open (spec §3.2.1) |
| 23:54 | `c863f97` (merge) | Merge main |

## Token usage

970.8 M tokens in 3,497 model messages (output 3.4 M, cache reads 952.3 M, cache writes 15.2 M, uncached input 8 k). Full breakdown: [tokens.json](tokens.json).

| Feature | Tokens | Output | Messages |
|---|---|---|---|
| Other | 231.7 M | 744 k | 832 |
| UI & menus | 183.1 M | 716 k | 897 |
| Build, release & infra | 156.9 M | 581 k | 663 |
| Worlds & terrain | 137.3 M | 505 k | 370 |
| Planning & spec | 118.9 M | 492 k | 345 |
| Art project | 45.7 M | 85 k | 124 |
| Materials & look | 42.9 M | 92 k | 88 |
| Multiplayer & server | 35.1 M | 126 k | 154 |
| Riders & bikes | 19.3 M | 33 k | 24 |

| Task | Feature | Tokens | Output |
|---|---|---|---|
| agent: Phase 3: junction steering, free ride | Planning & spec | 66.2 M | 244 k |
| agent: Phase 3 B: GPX, routes, free-ride gaps | Worlds & terrain | 63.4 M | 225 k |
| coordination: background task: update the build dashboard to also show queued tasks. is the nu… | UI & menus | 30.5 M | 39 k |
| agent: Phase 3 A: land use and buildings | Build, release & infra | 30.3 M | 155 k |
| coordination: Okay, start implementing phase 3. Use a small test area from the world where you… | Worlds & terrain | 29.3 M | 80 k |
| agent: Phase 3 C: signed packages, cache, e2e test | Worlds & terrain | 22.9 M | 150 k |
| agent: HUD recording indicator for V | UI & menus | 22.5 M | 86 k |
| agent: In-game menu M1 | UI & menus | 19.1 M | 86 k |
| coordination: okay, when moving the kickr the cyclists moves. to test the resistance i need mo… | Materials & look | 17.7 M | 24 k |
| coordination: show me a video of the ride through the new terrain | Worlds & terrain | 16.4 M | 35 k |
| coordination: earlier the road did have white line markings that gone missing in the spike vid… | Materials & look | 16.0 M | 24 k |
| coordination: to make the testing more interesting, add a random startpoint option which is tr… | Other | 15.7 M | 40 k |
| agent: Menu M2 settings + rider profiles | UI & menus | 15.5 M | 96 k |
| agent: Camera-ignored objects; fade trees | Other | 15.2 M | 71 k |
| coordination: dont run the dashboard on leela. run it on deb, the main build coordinator | UI & menus | 15.0 M | 11 k |
| coordination: start phase 1 scaffolding | Planning & spec | 13.9 M | 91 k |
| coordination: i've installed the depencencies on marge, continue | Other | 12.3 M | 26 k |
| coordination: start with phase 2 while i setup machines. | Build, release & infra | 12.1 M | 67 k |
| coordination: also add control of the cyclist with cursor keys | Other | 11.9 M | 19 k |
| coordination: no, its fine implement the old path as alternative. | Other | 11.6 M | 30 k |
| coordination: breaking with key down works, key up accelerates only very little. add a hotkey … | Other | 11.1 M | 20 k |
| coordination: yeah, there is an artifact in the road, investigate, screenshot and fix | UI & menus | 10.9 M | 33 k |
| agent: Dashboard: Windows tasks and CPU | UI & menus | 10.4 M | 92 k |
| coordination: mac os is in, make a parallel task to build and test there. | Build, release & infra | 10.4 M | 25 k |
| coordination: update documentation README with all the functions/modes/internal shell commands | Art project | 10.2 M | 12 k |
| coordination: the ip is Win11Builder | Other | 9.5 M | 10 k |
| coordination: while the spike test is running, can you continue with phase 2 implementation? d… | Planning & spec | 9.3 M | 34 k |
| agent: Per-machine build slots and queue view | Build, release & infra | 9.3 M | 62 k |
| coordination: you can outsource some smaller build tasks to macos, too | Build, release & infra | 9.2 M | 4 k |
| agent: FIT export and Strava upload/sync | Other | 9.1 M | 93 k |
| coordination: when the camera is moved by mouse and comes to close to road, it's possible to w… | Riders & bikes | 8.5 M | 10 k |
| agent: Windows toolchain and first build | Build, release & infra | 8.4 M | 38 k |
| coordination: which port does the server use? add discription to readme on how to host and run… | Art project | 8.4 M | 14 k |
| coordination: start resuming the task why the kicker does not work anymore on marge now | Other | 8.0 M | 7 k |
| coordination: whats eating up all the memory on marge? the system in now unresponsive | Other | 8.0 M | 7 k |
| coordination: make sure all the changes/additions make there way in a new version of the SPECI… | Build, release & infra | 7.9 M | 13 k |
| agent: More bots with start formation | Multiplayer & server | 7.5 M | 36 k |
| coordination: bg task: add version id into lower right corner of HUD. | UI & menus | 7.4 M | 5 k |
| agent: Workouts: TOML, .erg/.mrc, ERG | Other | 7.4 M | 81 k |
| coordination: try now to connect with leela | Other | 7.4 M | 7 k |

## Prompts

144 prompts (full text: [prompts.md](prompts.md)).

| Time | Prompt |
|---|---|
| 00:21 | analyse the specification for the OCW app as follows. translate to english and save it as specification.   <pasted_content id="4528"> Open C |
| 00:21 | write the specification in markup language. |
| 00:26 | Save ANT+ development for later. Suggest proposals for the other open items. |
| 00:29 | Gpx import for tracks. Strave upload for finished rides. Add a in-- |
| 00:31 | gGpx import for tracks. Strave upload for finished rides. Add a ingame console to enter commands. investigate software options for 3d graphi |
| 00:33 | continue |
| 00:38 | commit to main branch. |
| 00:45 | add usage of keys (left/right) and onscreen overlay arrows for ingame turn left right into roads. default is to right straight on / follow r |
| 00:47 | platform implementation schedule: first linux, then windows, then mac os. |
| 00:50 | Where would a web app running in any browser fit in here? is connecting to wahoo kicker there possible or would that require a native app on |
| 00:54 | yes, the web companion, but keep 2. full web client and native mobile app for future development. |
| 00:56 | commit. then merge proposals into specification. |
| 01:03 | start phase 1 scaffolding |
| 08:15 | commit to main |
| 08:17 | did you add all unit tests for phase 1? |
| 08:19 | yes, add those testss. add a makefile for installing dependencies, test and building. |
| 08:30 | yes |
| 08:31 | If I would just add or give you SSH connections to a Windows and a macOS machine, could you test and develop also on that platforms? And add |
| 08:37 | yes do the wayland test as well |
| 08:43 | to keep me in the loop, send me screenshots and short videos of the test output you create. save this for this project |
| 08:47 | Yes, at Wayland recording as well. |
| 08:52 | commit the changes. Did you notice that the trees are flying up in the air? The road goes up, but the rest of the terrain just stays flat. S |
| 08:58 | yes, do that now before phase 3. |
| 09:13 | which phase are we in? |
| 09:15 | commit. investigate graphic spikes. i also notices the the video does not look smooth, the roade makes a slight curve and i notice a small g |
| 09:20 | <e-mail>:newkit/OCW.git |
| 09:21 | here is an upstream github repository: add your changes there. |
| 09:26 | yes, i have those machines. create a config example that expects details for the build/test machines in a folder config/ but make sure that  |
| 09:30 | okay, will do that. in the mean time, implement saving of the screenshots and test videos to disk as well. |
| 09:37 | start with phase 2 while i setup machines. |
| 10:02 | i added a linux machine with gpu and bluetooth capabilites to config/machine.toml. This machine should also see a wahoo KIcKR via bluetooth. |
| 10:11 | move all building to machine marge. |
| 10:24 | make sure to add this to the make deps section for linux in makefile |
| 10:26 | split make deps into specific make deps-linux, windows, macos per platform. |
| 10:29 | i've installed the depencencies on marge, continue |
| 10:48 | while the spike test is running, can you continue with phase 2 implementation? develop in a seperate branch to not interfere with spike test |
| 11:17 | does the spike test have to be so long (running)? I think we can get enough insights from a shorter running test. |
| 11:18 | do you also record a video of the spike test? |
| 11:28 | also share the video of the spike test here |
| 11:32 | earlier the road did have white line markings that gone missing in the spike video. add them again. also change to road to have more curves  |
| 11:59 | done? |
| 12:01 | no, its fine implement the old path as alternative. |
| 12:12 | is the desktop app build on marge and ready to test? |
| 12:13 | and rename the desktop app to owc-desktop |
| 12:22 | leave it |
| 12:39 | What is ERG? |
| 13:14 | okay, when moving the kickr the cyclists moves. to test the resistance i need more different and changing steep profiles. also the legs of t |
| 13:52 | print the percentage of steepness in hud |
| 14:01 | alright, pretty cool, it works. merge and commit. |
| 14:04 | make the own rider character in color #FF1472 and give it one of the unique cyclist caps. i noticed some bouncing of the rider and the trees |
| 14:06 | also add control of the cyclist with cursor keys |
| 14:19 | have you build a test app? how to make the testapp startup on marge ob a specific scrren and always restart when its changed? create a scrip |
| 14:29 | what w What will be implemented in phase 3? |
| 14:29 | Okay, start implementing phase 3. Use a small test area from the world where you start testing. |
| 14:46 | And what's still open from phase two? I thought phase two is done. |
| 14:55 | show me a video of the ride through the new terrain |
| 15:06 | keys: pressing up should accellerate the rider, pressing down should stop. |
| 15:08 | the start portal ingame is broken, the top bar is 90 degrees wron, check and fix. |
| 15:08 | continue with phase 3 |
| 15:11 | breaking with key down works, key up accelerates only very little. add a hotkey that enables switching monitor. |
| 15:17 | when the camera is moved by mouse and comes to close to road, it's possible to watch through the road (clippimg problem), fix that. pressing |
| 15:24 | yeah, there is an artifact in the road, investigate, screenshot and fix |
| 16:08 | continue |
| 16:09 | implement a filter for the road data, there are some jumps and unsteady section. |
| 16:16 | make some of the other rides slower. add tilting / leaning inwards the driver when going in curves so it looks more realistic. |
| 16:27 | when pressing cursor keys left/right, implement a more realistic turn behavior: tilt the driver and ride into a curve. you need to press the |
| 16:33 | implement an autodrive mode, where the driver tries to stay on the middle of the road. when not in autodrive, the user has to actively steer |
| 16:35 | make  the steering controllable by a gamepad as well. |
| 16:37 | i see two riders get stuck now, fix that. |
| 16:38 | to make the testing more interesting, add a random startpoint option which is true for now. |
| 16:52 | okay commit and push |
| 16:52 | what are next development tasks |
| 16:53 | in wild mode, fix the following camera and fix clipping |
| 16:54 | mac os is in, make a parallel task to build and test there. |
| 17:03 | have the rider follow the surface. make the rider tiltable so that e.g. if the rider rides up a steep bump, first the front wheel goes up, a |
| 17:08 | generate an appimage logo showing a  unicorn riding on a bike over a rainbow. create a folder artwork and put it there uner logos. use this  |
| 17:10 | implement the strava upload and sync function in a background task. |
| 17:17 | how to enable ssh on the windows build machine? |
| 17:18 | and how to add a pure local build user? |
| 17:21 | make the user creation suitable for a german windows. call the user user |
| 17:24 | put those two steps into a script: setup-windows-buildserver.ps1 |
| 17:30 | continue implementing phase 3 in background |
| 17:48 | a github runner jpb failed. check and fix in a background task: https://github.com/newkit/OCW/actions/runs/43039804090/job/130588241350#anno |
| 17:54 | in which phase does the multiplayer architecture come? |
| 17:55 | whats in phase 4? |
| 17:58 | background task: include a folder artwork/build-history that contains all the screenshots and videos created during development. add that to |
| 18:09 | background task: there is something odd when steering very much, it all of a sudden stops and then dows not allow to go back/other direction |
| 18:11 | check on the background jobs, the seem to block / wait on eachother. how to add a gamepad? describe in README. |
| 18:17 | update documentation README with all the functions/modes/internal shell commands |
| 18:32 | make sure all the changes/additions make there way in a new version of the SPECIFICATIONS.md |
| 18:44 | gamepad: left / right works, up/down for accellerate/break does not. make the R key reset the entire scene, like when the programm has start |
| 18:45 | whats the status on macos? |
| 18:45 | which port does the server use? add discription to readme on how to host and run. |
| 18:48 | make the C button the camera reset button |
| 18:51 | build the desktop app on macos |
| 18:54 | what are open tasks, what do you suggest to do next? |
| 19:02 | continue in a background task with: Phase 3 (OpenStreetMap world, in progress):      Land use from OSM: forests, meadows, fields, built-up a |
| 19:03 | another bg task: turning on wild mode should turn off autodrive. |
| 19:03 | add appimagetool to the build deps |
| 19:06 | bg: have the video recording start with V key, print recording in HUD and have it stop when pressing V again. |
| 19:07 | where are the videos saved? |
| 19:08 | why do the videos look choppy? fix that |
| 19:14 | bg task: do planning for a in game menu. there should be (at least): settings (controls, trainer, strava, audio: mix abd also output). works |
| 19:15 | bg tasks: make more bots |
| 19:25 | Pause on the trainer: do not change anything  Should the ride timer stop during the pause (proposed)? -> No, the real riding time has to con |
| 19:32 | how to unzip the xz file on macos? |
| 19:36 | bg task: add to mac os build workflow packaging the app into an application so it can run from gui |
| 19:42 | bg task: add version id into lower right corner of HUD. |
| 19:45 | bg task: define objects that do not change camera. trees are such objects. |
| 19:50 | bg task: print gps coordinates of rider in HUD |
| 20:12 | where is the appbundle on mac os? add it to the release folder |
| 20:16 | the kickr input somehow got lost. i tested the latest linux version and turning the kickr does not have aby effect. fix that |
| 20:24 | no, kickr does not work |
| 20:33 | whats eating up all the memory on marge? the system in now unresponsive |
| 20:44 | i added another linux build machine: leela. use that as well. |
| 20:47 | try now to connect with leela |
| 20:50 | the make deps does not work on fedora, split make deps-linux into arch specific functions and add detection logic |
| 20:53 | on fedora when make deps-linux roblem: problem with installed package   - installed package ffmpeg-libs-8.1.3-1.fc44.x86_64 conflicts with l |
| 20:57 | okay, try a quick smok smoke test build on Lila. |
| 21:04 | yes |
| 21:09 | whats the command to allow powershell script execution? add this and all buildserver setup instructtions to readme. |
| 21:18 | check the windows build server |
| 21:19 | which ip are you using? you should be using <ip> |
| 21:19 | the ip is Win11Builder |
| 21:28 | marge should be back online, |
| 21:29 | start resuming the task why the kicker does not work anymore on marge now |
| 21:36 | bg task: as a little side project: create a webapp that show the usage of the buildserver, the tasks running/queued on them. |
| 21:40 | make the buildserver dashboard listen on all adresses 0.0.0.0. make it use ssh to gather all the information from the other machines configu |
| 21:43 | dont run the dashboard on leela. run it on deb, the main build coordinator |
| 21:49 | whats the issue on windows? |
| 21:51 | add that manual setup to the readme in the windows buildserver seutp section |
| 22:03 | windows toolchain installed. continue. |
| 22:04 | whats the status on macos? is there an appbundle? |
| 22:06 | bg task: add the ffmpeg to the deps on macos. then build and put the bundle to release/ |
| 22:06 | bg task: continue with windows setup and first build |
| 22:15 | ffmpeg and brew is installed on mac os |
| 22:16 | i will turn the padel now on the kickr, look for it on marge |
| 22:21 | okay. kickr works now. I noticed that the inclination reported by HUD and the inclinication indicated by the terrain do not mach so well in  |
| 22:25 | background task: include marge to build pool. HUD fix: break line if its out of the visible area. Where is the GPS position in HUD i asked f |
| 22:27 | bacgrount task: update the kiosk on marge with the latest build including the world and terrain updates. |
| 22:32 | bacground task: the reporting in build dashboard for windows is poor, what would it take to display bacground task and cpu load? |
| 22:41 | background task: start implementing and testing open tasks and phases on linux, use both build servers. where unsure, take the best guess bu |
| 22:46 | you can outsource some smaller build tasks to macos, too |
| 22:49 | background task: update the build dashboard to also show queued tasks. is the number of build slots correct? i thout 1 mac 2x2 linux and 1 w |

## Media

| Time | File |
|---|---|
| 01:28:29 | [012829-shot1.png](012829-shot1.png) |
| 01:29:20 | [012920-shot2.png](012920-shot2.png) |
| 01:29:28 | [012928-shot3.png](012928-shot3.png) |
| 01:34:41 | [013441-drv.png](013441-drv.png) |
| 08:24:27 | [082427-after.png](082427-after.png) |
| 08:25:07 | [082507-make.png](082507-make.png) |
| 08:39:57 | [083957-wl.png](083957-wl.png) |
| 08:40:04 | [084004-x11.png](084004-x11.png) |
| 08:43:46 | [084346-smoke-wayland.png](084346-smoke-wayland.png) |
| 08:43:46 | [084346-smoke-x11.png](084346-smoke-x11.png) |
| 08:45:02 | [084502-ride.mp4](084502-ride.mp4) |
| 08:45:07 | [084507-ride.png](084507-ride.png) |
| 08:45:08 | [084508-ride-sheet.png](084508-ride-sheet.png) |
| 08:47:37 | [084737-cage.mp4](084737-cage.mp4) |
| 08:47:40 | [084740-cage.png](084740-cage.png) |
| 08:47:45 | [084745-cage-sheet.png](084745-cage-sheet.png) |
| 08:48:29 | [084829-cage-fs.mp4](084829-cage-fs.mp4) |
| 08:48:29 | [084829-cage-fs.png](084829-cage-fs.png) |
| 08:48:30 | [084830-cage-fs-frame.png](084830-cage-fs-frame.png) |
| 08:49:28 | [084928-ride-x11.mp4](084928-ride-x11.mp4) |
| 08:49:33 | [084933-ride-x11.png](084933-ride-x11.png) |
| 08:49:51 | [084951-ride-wayland.mp4](084951-ride-wayland.mp4) |
| 08:49:56 | [084956-ride-wayland.png](084956-ride-wayland.png) |
| 08:49:58 | [084958-ride-wayland-sheet.png](084958-ride-wayland-sheet.png) |
| 09:04:13 | [090413-base-fps.png](090413-base-fps.png) |
| 09:06:09 | [090609-terrain-top.png](090609-terrain-top.png) |
| 09:09:13 | [090913-terrain-side.png](090913-terrain-side.png) |
| 09:09:23 | [090923-terrain-chase.png](090923-terrain-chase.png) |
| 09:09:34 | [090934-terrain-before-after.png](090934-terrain-before-after.png) |
| 09:09:45 | [090945-terrain-ride.mp4](090945-terrain-ride.mp4) |
| 09:09:50 | [090950-terrain-ride.png](090950-terrain-ride.png) |
| 09:20:04 | [092004-cap-hud.png](092004-cap-hud.png) |
| 09:20:04 | [092004-cap.mp4](092004-cap.mp4) |
| 09:21:05 | [092105-smooth-x11.png](092105-smooth-x11.png) |
| 09:21:06 | [092106-smooth-x11.mp4](092106-smooth-x11.mp4) |
| 09:21:57 | [092157-smooth-wayland.mp4](092157-smooth-wayland.mp4) |
| 09:21:57 | [092157-smooth-wayland.png](092157-smooth-wayland.png) |
| 09:22:03 | [092203-wl-shot.png](092203-wl-shot.png) |
| 09:22:09 | [092209-smooth-sheet.png](092209-smooth-sheet.png) |
| 09:30:51 | [093051-a.png](093051-a.png) |
| 09:31:08 | [093108-b.mp4](093108-b.mp4) |
| 09:31:08 | [093108-b.png](093108-b.png) |
| 09:31:14 | [093114-c.png](093114-c.png) |
| 10:39:08 | [103908-trial.png](103908-trial.png) |
| 10:40:21 | [104021-check.png](104021-check.png) |
| 10:55:42 | [105542-bench-instanced-40.png](105542-bench-instanced-40.png) |
| 11:10:43 | [111043-bench-merged-40.png](111043-bench-merged-40.png) |
| 11:14:45 | [111445-bench-instanced-150.png](111445-bench-instanced-150.png) |
| 11:18:46 | [111846-bench-merged-150.png](111846-bench-merged-150.png) |
| 11:25:06 | [112506-spike-splat.mp4](112506-spike-splat.mp4) |
| 11:25:06 | [112506-spike-splat.png](112506-spike-splat.png) |
| 11:25:13 | [112513-splat-sheet.png](112513-splat-sheet.png) |
| 11:28:46 | [112846-short-merged-vertex.png](112846-short-merged-vertex.png) |
| 11:31:48 | [113148-short-merged-splat.png](113148-short-merged-splat.png) |
| 11:34:49 | [113449-short-instanced-splat.png](113449-short-instanced-splat.png) |
| 11:36:50 | [113650-stress-merged-splat.png](113650-stress-merged-splat.png) |
| 11:37:24 | [113724-spike-winding.mp4](113724-spike-winding.mp4) |
| 11:37:24 | [113724-spike-winding.png](113724-spike-winding.png) |
| 11:37:56 | [113756-spike-winding.mp4](113756-spike-winding.mp4) |
| 11:37:56 | [113756-spike-winding.png](113756-spike-winding.png) |
| 11:37:56 | [113756-winding-first.png](113756-winding-first.png) |
| 11:38:31 | [113831-spike-winding.mp4](113831-spike-winding.mp4) |
| 11:38:31 | [113831-spike-winding.png](113831-spike-winding.png) |
| 11:38:33 | [113833-winding-sheet.png](113833-winding-sheet.png) |
| 12:17:47 | [121747-kickr.png](121747-kickr.png) |
| 13:24:16 | [132416-hills-chase.mp4](132416-hills-chase.mp4) |
| 13:24:16 | [132416-hills-chase.png](132416-hills-chase.png) |
| 13:24:25 | [132425-hills-orbit.mp4](132425-hills-orbit.mp4) |
| 13:24:25 | [132425-hills-orbit.png](132425-hills-orbit.png) |
| 13:24:32 | [132432-hills-sheet.png](132432-hills-sheet.png) |
| 13:24:32 | [132432-legs.png](132432-legs.png) |
| 13:25:04 | [132504-hills-climb.mp4](132504-hills-climb.mp4) |
| 13:25:04 | [132504-hills-climb.png](132504-hills-climb.png) |
| 13:25:05 | [132505-climb.png](132505-climb.png) |
| 13:52:57 | [135257-hud.png](135257-hud.png) |
| 14:07:19 | [140719-cap.png](140719-cap.png) |
| 14:14:59 | [141459-steer-sheet.png](141459-steer-sheet.png) |
| 14:14:59 | [141459-steer.mp4](141459-steer.mp4) |
| 14:14:59 | [141459-steer.png](141459-steer.png) |
| 14:15:10 | [141510-steer-trimmed.mp4](141510-steer-trimmed.mp4) |
| 14:22:50 | [142250-mon.png](142250-mon.png) |
| 14:38:03 | [143803-graph.png](143803-graph.png) |
| 14:43:32 | [144332-grade-small.png](144332-grade-small.png) |
| 14:43:32 | [144332-grade.png](144332-grade.png) |
| 14:59:20 | [145920-lie1.png](145920-lie1.png) |
| 14:59:45 | [145945-lie-a.mp4](145945-lie-a.mp4) |
| 14:59:45 | [145945-lie-a.png](145945-lie-a.png) |
| 14:59:54 | [145954-lie-b.mp4](145954-lie-b.mp4) |
| 14:59:54 | [145954-lie-b.png](145954-lie-b.png) |
| 15:02:09 | [150209-lie-b.mp4](150209-lie-b.mp4) |
| 15:02:09 | [150209-lie-b.png](150209-lie-b.png) |
| 15:02:18 | [150218-lie-c.mp4](150218-lie-c.mp4) |
| 15:02:18 | [150218-lie-c.png](150218-lie-c.png) |
| 15:02:18 | [150218-liechtenstein.mp4](150218-liechtenstein.mp4) |
| 15:02:19 | [150219-lie-sheet.png](150219-lie-sheet.png) |
| 15:09:23 | [150923-arch.png](150923-arch.png) |
| 15:16:11 | [151611-pkg1.png](151611-pkg1.png) |
| 15:16:24 | [151624-pkg2.png](151624-pkg2.png) |
| 15:16:25 | [151625-pkg-sheet.png](151625-pkg-sheet.png) |
| 15:16:49 | [151649-pk-a.mp4](151649-pk-a.mp4) |
| 15:16:49 | [151649-pk-a.png](151649-pk-a.png) |
| 15:16:57 | [151657-package.mp4](151657-package.mp4) |
| 15:16:57 | [151657-pk-b.mp4](151657-pk-b.mp4) |
| 15:16:57 | [151657-pk-b.png](151657-pk-b.png) |
| 15:19:46 | [151946-cam.png](151946-cam.png) |
| 15:24:47 | [152447-art-top.png](152447-art-top.png) |
| 15:24:54 | [152454-art-side.png](152454-art-side.png) |
| 15:24:55 | [152455-art-before.png](152455-art-before.png) |
| 15:25:18 | [152518-art-a.png](152518-art-a.png) |
| 15:25:25 | [152525-art-b.png](152525-art-b.png) |
| 15:25:26 | [152526-art-before2.png](152526-art-before2.png) |
| 15:33:44 | [153344-fix-a.png](153344-fix-a.png) |
| 15:33:52 | [153352-art-after.png](153352-art-after.png) |
| 15:33:52 | [153352-art-before-after.png](153352-art-before-after.png) |
| 15:33:52 | [153352-fix-b.png](153352-fix-b.png) |
| 16:17:47 | [161747-gradient-before-after.png](161747-gradient-before-after.png) |
| 16:20:44 | [162044-lean.mp4](162044-lean.mp4) |
| 16:20:44 | [162044-lean.png](162044-lean.png) |
| 16:21:21 | [162121-lean-sheet.png](162121-lean-sheet.png) |
| 16:21:21 | [162121-lean.mp4](162121-lean.mp4) |
| 16:21:21 | [162121-lean.png](162121-lean.png) |
| 16:34:09 | [163409-steer-sheet.png](163409-steer-sheet.png) |
| 16:34:09 | [163409-steer.mp4](163409-steer.mp4) |
| 16:34:09 | [163409-steer.png](163409-steer.png) |
| 16:40:09 | [164009-rs.png](164009-rs.png) |
| 16:47:37 | [164737-modes.mp4](164737-modes.mp4) |
| 16:47:37 | [164737-modes.png](164737-modes.png) |
| 16:47:38 | [164738-modes-sheet.png](164738-modes-sheet.png) |
| 16:56:22 | [165622-wildcam.mp4](165622-wildcam.mp4) |
| 16:56:22 | [165622-wildcam.png](165622-wildcam.png) |
| 16:58:52 | [165852-wildcam-before-sheet.png](165852-wildcam-before-sheet.png) |
| 16:59:19 | [165919-wildcam.mp4](165919-wildcam.mp4) |
| 16:59:19 | [165919-wildcam.png](165919-wildcam.png) |
| 16:59:20 | [165920-wildcam-sheet.png](165920-wildcam-sheet.png) |
| 16:59:56 | [165956-dbg.mp4](165956-dbg.mp4) |
| 16:59:56 | [165956-dbg.png](165956-dbg.png) |
| 17:00:45 | [170045-wildcam.mp4](170045-wildcam.mp4) |
| 17:00:45 | [170045-wildcam.png](170045-wildcam.png) |
| 17:01:42 | [170142-wildcam.mp4](170142-wildcam.mp4) |
| 17:01:42 | [170142-wildcam.png](170142-wildcam.png) |
| 17:03:25 | [170325-wildcam-end.png](170325-wildcam-end.png) |
| 17:03:25 | [170325-wildcam.mp4](170325-wildcam.mp4) |
| 17:03:25 | [170325-wildcam.png](170325-wildcam.png) |
| 17:04:48 | [170448-bump.mp4](170448-bump.mp4) |
| 17:04:48 | [170448-bump.png](170448-bump.png) |
| 17:04:49 | [170449-bump-sheet.png](170449-bump-sheet.png) |
| 18:19:20 | [181920-agent-freeride-shot-wayland-ocw-free.png](181920-agent-freeride-shot-wayland-ocw-free.png) |
| 18:20:34 | [182034-agent-freeride-video-wayland-freeride.mp4](182034-agent-freeride-video-wayland-freeride.mp4) |
| 18:20:34 | [182034-agent-freeride-video-wayland-freeride.png](182034-agent-freeride-video-wayland-freeride.png) |
| 18:21:29 | [182129-agent-freeride-video-wayland-freeride.mp4](182129-agent-freeride-video-wayland-freeride.mp4) |
| 18:21:29 | [182129-agent-freeride-video-wayland-freeride.png](182129-agent-freeride-video-wayland-freeride.png) |
| 18:23:25 | [182325-agent-freeride-shot-wayland-smoke-wl.png](182325-agent-freeride-shot-wayland-smoke-wl.png) |
| 18:24:07 | [182407-agent-freeride-video-wayland-freeride.mp4](182407-agent-freeride-video-wayland-freeride.mp4) |
| 18:24:07 | [182407-agent-freeride-video-wayland-freeride.png](182407-agent-freeride-video-wayland-freeride.png) |
| 18:26:04 | [182604-agent-freeride-shot-wayland-smoke-wl.png](182604-agent-freeride-shot-wayland-smoke-wl.png) |
| 18:26:51 | [182651-agent-freeride-video-wayland-freeride.mp4](182651-agent-freeride-video-wayland-freeride.mp4) |
| 18:26:51 | [182651-agent-freeride-video-wayland-freeride.png](182651-agent-freeride-video-wayland-freeride.png) |
| 18:28:49 | [182849-agent-freeride-shot-wayland-smoke-wl.png](182849-agent-freeride-shot-wayland-smoke-wl.png) |
| 18:29:36 | [182936-agent-freeride-video-wayland-freeride.mp4](182936-agent-freeride-video-wayland-freeride.mp4) |
| 18:29:36 | [182936-agent-freeride-video-wayland-freeride.png](182936-agent-freeride-video-wayland-freeride.png) |
| 18:48:33 | [184833-shot-wayland-shot.png](184833-shot-wayland-shot.png) |
| 21:04:32 | [210432-shot-wayland-ocw-leela-wl.png](210432-shot-wayland-ocw-leela-wl.png) |
| 21:30:19 | [213019-shot-wayland-ocw-route.png](213019-shot-wayland-ocw-route.png) |
| 21:31:50 | [213150-agent-routes-f06.png](213150-agent-routes-f06.png) |
| 21:31:50 | [213150-agent-routes-f14.png](213150-agent-routes-f14.png) |
| 21:31:50 | [213150-agent-routes-f30.png](213150-agent-routes-f30.png) |
| 21:32:05 | [213205-video-wayland-ocw-route-leave.mp4](213205-video-wayland-ocw-route-leave.mp4) |
| 21:32:05 | [213205-video-wayland-ocw-route-leave.png](213205-video-wayland-ocw-route-leave.png) |
| 22:15:44 | [221544-agent-landuse-lu-steg.png](221544-agent-landuse-lu-steg.png) |
| 22:15:44 | [221544-agent-landuse-lu-vaduz2.png](221544-agent-landuse-lu-vaduz2.png) |
| 22:15:44 | [221544-agent-landuse-rhine-chase.png](221544-agent-landuse-rhine-chase.png) |
| 22:15:44 | [221544-agent-landuse-rhine-top.png](221544-agent-landuse-rhine-top.png) |
| 22:15:44 | [221544-agent-landuse-ride.mp4](221544-agent-landuse-ride.mp4) |
| 22:15:44 | [221544-agent-landuse-steg-stream.png](221544-agent-landuse-steg-stream.png) |
| 22:21:13 | [222113-agent-landuse-merged-steg.png](222113-agent-landuse-merged-steg.png) |
| 22:39:03 | [223903-kiosk-world-args-file.png](223903-kiosk-world-args-file.png) |
| 22:39:22 | [223922-kiosk-world-args-file-vaduz.png](223922-kiosk-world-args-file-vaduz.png) |
| 22:49:09 | [224909-agent-gps-1280x720.png](224909-agent-gps-1280x720.png) |
| 22:49:09 | [224909-agent-gps-2560x1600.png](224909-agent-gps-2560x1600.png) |
| 22:49:09 | [224909-agent-loop-800x600.png](224909-agent-loop-800x600.png) |
| 22:59:05 | [225905-agent-gps-800x600.png](225905-agent-gps-800x600.png) |
| 23:04:44 | [230444-agent-version-hud.png](230444-agent-version-hud.png) |
| 23:21:35 | [232135-agent-rel-after-end.png](232135-agent-rel-after-end.png) |
| 23:22:51 | [232251-agent-rec-leela.png](232251-agent-rec-leela.png) |
| 23:30:31 | [233031-agent-rec-marge.png](233031-agent-rec-marge.png) |
| 23:34:03 | [233403-agent-bots-formation.png](233403-agent-bots-formation.png) |
| 23:37:00 | [233700-agent-rec.png](233700-agent-rec.png) |
| 23:38:44 | [233844-agent-see-through-ocw-a13997-trees-sheet.png](233844-agent-see-through-ocw-a13997-trees-sheet.png) |
| 23:38:44 | [233844-agent-see-through-ocw-a13997-trees.mp4](233844-agent-see-through-ocw-a13997-trees.mp4) |
| 23:38:44 | [233844-agent-see-through-ocw-a13997-trees.png](233844-agent-see-through-ocw-a13997-trees.png) |
| 23:39:23 | [233923-agent-see-through-scan-sheet.png](233923-agent-see-through-scan-sheet.png) |
| 23:39:52 | [233952-agent-see-through-ocw-a13997-forest.mp4](233952-agent-see-through-ocw-a13997-forest.mp4) |
| 23:39:52 | [233952-agent-see-through-see-through-forest-sheet.png](233952-agent-see-through-see-through-forest-sheet.png) |
| 23:40:05 | [234005-agent-see-through-ocw-a13997-km9.mp4](234005-agent-see-through-ocw-a13997-km9.mp4) |
| 23:40:05 | [234005-agent-see-through-see-through-km9-sheet.png](234005-agent-see-through-see-through-km9-sheet.png) |
| 23:40:40 | [234040-agent-see-through-ocw-a13997-houses.mp4](234040-agent-see-through-ocw-a13997-houses.mp4) |
| 23:40:40 | [234040-agent-see-through-see-through-houses-sheet.png](234040-agent-see-through-see-through-houses-sheet.png) |
| 23:40:49 | [234049-agent-see-through-smoke-wayland.png](234049-agent-see-through-smoke-wayland.png) |
| 23:49:31 | [234931-shot-wayland-menu-1280.png](234931-shot-wayland-menu-1280.png) |
| 23:49:48 | [234948-shot-wayland-menu-800.png](234948-shot-wayland-menu-800.png) |
| 23:50:06 | [235006-shot-wayland-menu-quit.png](235006-shot-wayland-menu-quit.png) |
| 23:51:30 | [235130-shot-wayland-menu-1280.png](235130-shot-wayland-menu-1280.png) |
| 23:51:47 | [235147-shot-wayland-menu-800.png](235147-shot-wayland-menu-800.png) |
| 23:52:03 | [235203-shot-wayland-menu-quit.png](235203-shot-wayland-menu-quit.png) |
| 23:52:19 | [235219-shot-wayland-menu-ride.png](235219-shot-wayland-menu-ride.png) |
| 23:53:31 | [235331-shot-wayland-m.png](235331-shot-wayland-m.png) |
| 23:53:47 | [235347-shot-wayland-m.png](235347-shot-wayland-m.png) |
| 23:54:03 | [235403-shot-wayland-m.png](235403-shot-wayland-m.png) |
| 23:56:25 | [235625-shot-wayland-m.png](235625-shot-wayland-m.png) |
