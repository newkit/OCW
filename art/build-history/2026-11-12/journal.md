# OCW build diary – day 2, 2026-11-12

113 commits (27 merges), +51385 / −3001 lines, 383 images, 63 videos, 2 sounds, 726 tests.

## Commits

| Time | Commit | |
|---|---|---|
| 00:01 | `a07845c` (merge) | Merge in-game menu M1: input contexts, Esc/Start menu, gamepad navigation, ride continues on autodrive |
| 00:07 | `b898618` | Collect agent screenshots and videos into the test-output build history |
| 00:10 | `7fb7796` | Menu M2: settings registry, settings.toml, set/get, rider profiles (spec §3.2.1) |
| 00:13 | `69bf15b` | Structured workouts with ERG control (spec §2.8, §5.3) |
| 00:17 | `3207b93` (merge) | Merge main (menu M1); hook the Settings and Workshop pages into the menu |
| 00:25 | `73d60f7` | Gradient feel: average gradient ahead, mini elevation profile, camera tilt (spec §3.2) |
| 00:26 | `2230599` (merge) | Merge main into the gradient-feel branch (camera: menu input context) |
| 00:30 | `da8c939` | QUIC transport groundwork: ocw-server --listen, ocw-desktop --server (spec §6.2) |
| 00:32 | `b9dcb21` (merge) | Merge gradient feel: 100 m average gradient, mini elevation profile, camera tilt with the road |
| 00:33 | `093696f` | collect-test-output: index rows for fetched archive files without their index row |
| 00:34 | `669b0d9` (merge) | Merge main into workouts (menu plugin, README video text) |
| 00:35 | `0aea363` (merge) | Merge structured workouts: TOML format, .erg/.mrc import, ERG control, HUD workout panel |
| 00:36 | `b3fcd3d` (merge) | Merge branch 'main' into worktree-agent-aba411746b9949897 |
| 00:39 | `902a48a` (merge) | Merge QUIC transport groundwork: ocw-server --listen, ocw-desktop --server (spec §6.2) |
| 00:43 | `df75cfe` | Menu pages: Workshop note as one paragraph, shorter fullscreen label |
| 00:55 | `29c321a` (merge) | Merge main (QUIC transport, elevation profile, camera tilt) |
| 00:55 | `7a8004e` (merge) | Merge menu M2: settings registry and settings.toml, set/get, rider profiles (weight to server physics and trainer), Settings and Workshop pages |
| 00:56 | `7a2d61e` | Example rider names: neutral names in docs and tests |
| 00:57 | `c7e7f4a` | Example rider names: lower-case variants too |
| 00:57 | `919d706` | Dashboard mask test: neutral example home directory |
| 00:58 | `cab06dd` | Bundle Fira Mono as the UI font: non-ASCII street names, degree sign |
| 00:58 | `0826aa5` (merge) | Merge branch 'main' into worktree-agent-af4905a2faee382c1 |
| 00:59 | `204a46b` | Neighbour packages at runtime: free ride crosses package borders (spec §2.2.2) |
| 01:21 | `3a48c4f` | Seamless borders: --bbox parts take profiles and terrain from the roads around them; strips overlap the cut |
| 01:21 | `85e24a6` (merge) | Merge branch 'main' into worktree-agent-a06e0b830e326ed78 |
| 01:27 | `007ba83` | Curated physics profiles, bike and look per rider, procedural bike types, Workshop Bike page (spec §2.5) |
| 01:27 | `9691ddd` (merge) | Merge branch 'main' into worktree-agent-a7db2db773f1023e3 |
| 01:36 | `d44f9da` (merge) | Merge Phase 4 groundwork: curated physics profiles, bike and look per rider, procedural bike types, Workshop Bike page |
| 01:42 | `26c0f96` (merge) | Merge neighbour packages at runtime: REGN region, WorldSet, seamless borders, free ride across package borders |
| 01:56 | `7973c4b` (merge) | Merge bundled Fira Mono UI font: non-ASCII street names, degree sign (OFL 1.1) |
| 01:59 | `4931aa9` | Local events: race, group ride, time trial vs ghost (spec §2.3.1) |
| 02:00 | `2c3af89` (merge) | Merge main into the local-events branch |
| 02:11 | `7b22782` | Events after the merge: ServerConfig in the new tests, menu test on the Navi placeholder |
| 02:17 | `d8ad641` (merge) | Merge local events: race, group ride, time trial vs own ghost (spec §2.3.1) |
| 02:17 | `a1e1cdf` | WIP: in-app ride switch, Ride/Navi lists, menu paging (M3) |
| 02:18 | `25d69c7` | remote.sh: OCW_REMOTE_JOBS also applies to main checkouts |
| 02:18 | `8ec78d5` (merge) | Merge main (local events) into M3: Ride -> Change ride sub page |
| 02:20 | `6675050` | M3 docs: README and spec (in-app ride switch, Navi lists, GPX import) |
| 02:28 | `53b8264` | M3: fix tests after the events merge; --at runs on app time (a ride switch does not reset it) |
| 02:31 | `4292c75` | M3: shorter list rows (two lines at most), Workshop paging test |
| 02:32 | `73a9a27` | Free ride: no back-and-forth after a U-turn on a one-way road (spec §2.2.1, §2.2.2) |
| 02:32 | `4a17191` | M3: GPX import test on a small road network |
| 02:33 | `f7d2620` (merge) | Merge branch 'main' into worktree-agent-ae002a0855f7ede36 |
| 02:33 | `1a78d83` | README: --at counts app time |
| 02:35 | `b65bc83` (merge) | Merge menu M3: in-app ride switch, Change ride and Navi pages, GPX import, list paging |
| 02:39 | `d6cf57b` (merge) | Merge branch 'main' into worktree-agent-a1fda7ac7113f788d |
| 02:41 | `160d04f` | One-way U-turn test: ServerConfig field from main |
| 02:41 | `d6f00c3` (merge) | Merge U-turn fix on one-way roads (no double U-turn, no roundabout circling) |
| 02:55 | `25eecf7` | Tunnels: tube and tunnel camera where the road runs under the terrain |
| 03:15 | `396f438` | Steep ramps: riders crawl instead of stalling; grade limit across junctions; corner heights by arc length |
| 03:18 | `97a5805` (merge) | Merge steep-ramp fixes: start from standstill on any grade, grade limit across junctions, corner heights by distance |
| 03:21 | `a6ddb66` | Tunnel camera: under the hillside at the entry portal |
| 03:21 | `0f25268` (merge) | Merge branch 'main' into worktree-agent-af2d574dade06721e |
| 03:23 | `d0a9576` (merge) | Merge tunnels: tube and tunnel camera where the road runs under the terrain (Stegertunnel) |
| 03:51 | `173d2c9` | Ghost: no shadows (spec §2.3.1) |
| 03:51 | `bb43e69` | FIT laps, event start confirmation, Workouts menu page, FTP from the rider profile |
| 03:52 | `2dc1892` (merge) | Merge events/workouts polish: FIT laps, ghost without shadows, event start confirmation, Workouts menu page, FTP from the rider profile |
| 03:52 | `8613029` | Spec 2.8: Appendix B – autonomous development 2026-11-11/12, additions and assumptions |
| 04:31 | `c5255a1` | Final platform runs on 8613029 (Linux, macOS, Windows: 726 tests each); macOS DMG in release/; drive.sh prefers OCW_BUILD_ID |
| 06:44 | `fe00c32` | Release 0.1.0-c5255a1 for Linux (AppImage, binaries) and Windows (zip) |
| 07:05 | `ec53449` | Spec 2.9: route editor postponed; Phase 4 models/audio and Phase 6 voice start |
| 07:13 | `c669b94` | Spec 2.10: voice chat always on like a call; mic and speaker mute hotkeys (M / Shift+M, Select / R3) |
| 07:39 | `dcb2e8a` | Releases on GitHub Releases; binaries removed from history (commit-map.txt), spec 2.11 |
| 07:42 | `e461ad7` | Game audio and Settings -> Audio (M5, spec §3.3, §3.2.1) |
| 07:46 | `cb197e6` | make deps-linux-debian: clang 21 from apt.llvm.org (deps-llvm-debian) and libglib2.0-dev for the voice feature |
| 07:53 | `a57b86e` | Phase 4: generated glTF bikes and rider, asset validator, riding animation |
| 07:55 | `4cbf320` | Spec 2.12: voice platform plan – LiveKit on every platform (desktop Rust SDK, browser livekit-client via wasm, mobile SDKs) |
| 07:57 | `50d898d` | Audio page: short output labels (ten-foot layout), one-paragraph note; ride make-up gain |
| 08:13 | `66969b5` | deps-linux: GLib/GIO headers for webrtc-sys on Fedora (glib2-devel) and Arch (glib2) too; README: CXX=clang++-21 |
| 08:26 | `67bdf2d` | Phase 4: Bevy glTF loader integration test, TT riders stay on the extensions on mild climbs, clippy fixes, spec status |
| 08:36 | `9b1cd14` | Build-history media out of git: zips on the GitHub release build-history; spec 2.13 |
| 08:38 | `1fd04f8` | Spec 2.14: Phase 4 models and animation |
| 08:45 | `e952dac` | Remove artwork/build-history; build history lives only on the GitHub release build-history (spec 2.15) |
| 08:52 | `b9775e1` | Phase 6 voice chat, first increment: rooms and tokens, ocw-voice, LiveKit backend, M8 |
| 08:54 | `4b8329b` | Spec 2.16: game audio (M5) |
| 09:00 | `c71e9c3` | Multiplayer test across machines: ocw-bot client, server stats and route files, test-* make targets |
| 09:06 | `dc21e39` | Scene-rendering harness: scripts/scenes.toml, render-scenes.sh, make test-scenes |
| 09:10 | `e7af0be` | Spec 2.17: voice chat first increment; Strava account page moved to Navi; CLAUDE.md wasm check includes ocw-voice |
| 09:23 | `a56b615` | scenes.toml: Vaduz scenes tuned (front orbit for the buildings, raised chase camera without bots for the video) |
| 09:24 | `f1540e3` | Menu: group the audio and voice device lists in write_view (clippy too_many_arguments after merging M5 and M8); Cargo.lock |
| 09:29 | `d91b6f4` | Spec 2.18: scene-rendering harness |
| 10:13 | `2e2fd4b` | Fix six bugs: event bot looks, menu overflow, teleport past the route end, ground after a teleport, tunnel portal wall, --world alone |
| 10:13 | `192d202` | scenes.toml: route-end-teleport description (13.3 km route) |
| 10:14 | `c0216f7` | Spec 2.19: six bug fixes |
| 10:17 | `03f1ead` | Multiplayer test: camera client for a locked kiosk screen, caffeinate on macOS, lost-close and suspend handling in the verdict |
| 10:18 | `4aec9de` | ocw-bot: verdict test for a timeout long before the server's end |
| 10:23 | `aa65b76` | test-multiplayer: redact every machine's host and the server log's peer addresses (IPv6 too) |
| 10:26 | `ea84a09` | Spec 2.20: multiplayer test across machines, test-* make targets |
| 10:26 | `690170e` | Server Cargo.toml: one serde_json entry after merging voice and stats |
| 10:28 | `5da9fa3` | rustfmt: module order in the server crate |
| 10:58 | `34148f3` | Audio: freehub ticks only while coasting, drivetrain whirs (spec §3.3) |
| 10:59 | `22fbc7d` | Spec 2.21: freehub ticks only while coasting |
| 11:31 | `9198206` | Real render rate in the HUD, smooth 25 fps multiplayer videos |
| 11:41 | `fbd2fce` | Spec 2.22: render rate, FPS counter, smooth multiplayer videos |
| 12:23 | `ed021e1` | README: logo and two pictures of riders on the Malbun road (images on the build-history release); readme-* scenes |
| 16:29 | `1daacc7` | Packaging scripts: OCW_FEATURES for extra cargo features (voice in release builds) |
| 18:34 | `765002f` | Release builds with voice on Windows and macOS |
| 18:36 | `9573ffe` | build-appimage.sh: create the output directory |
| 19:28 | `72238db` | Voice: dropping the LiveKit backend waits for its threads |
| 19:36 | `f055270` | Spec 2.23: release builds with voice on all platforms; release notes 0.1.0-72238db |
| 22:00 | `d95dd30` | Art project: the OCW Build Diary (art/ART-PROJECT.md) |
| 22:03 | `5a0bf61` | art/diary.py publish: upload SHA256SUMS of all zips and videos |
| 22:15 | `96f3de0` | Art project 1.1: ISO 8601 date and time in every photo and video |
| 22:23 | `f03a863` | Spec 2.24: Phase 8 – textures and regional look (planning) |
| 22:26 | `9e65a55` | CLAUDE.md: use the Mac and Windows build machines for dev tasks too |
| 22:39 | `a19dffd` | Spec 2.25: Phase 9 – sky, time, seasons and weather (planning) |
| 22:40 | `8de0d28` | World builder: gutters are not water, bike-open pedestrian streets are paved |
| 22:40 | `0561281` | world-builder: new manifest after in-place rewrites; route --bbox/--origin; areas |
| 22:40 | `42f5083` | Second world: the southern Black Forest (spec 2.27) |
| 22:40 | `8e7a70f` | Spec 2.26: Phase 8D – reference photos of how things really look |
| 22:48 | `a07acce` | Timestamped captures: ISO 8601 HUD stamp, JPEG/EXIF screenshots, 1440p test media, game sound in videos |
| 22:56 | `c35a760` | Spec 2.28: Phase 9 decisions – virtual-world local time, extreme weather uncapped |
| 22:58 | `fd8e665` | Spec 2.29: decisions – realistic weather physics (organiser in races), best open data per region, attribution licences accepted |

## Token usage

872.6 M tokens in 3,799 model messages (output 2.8 M, cache reads 856.9 M, cache writes 13.0 M, uncached input 8 k). Full breakdown: [tokens.json](tokens.json).

| Feature | Tokens | Output | Messages |
|---|---|---|---|
| Other | 184.7 M | 617 k | 904 |
| Audio & voice | 125.3 M | 410 k | 476 |
| Worlds & terrain | 117.1 M | 327 k | 463 |
| UI & menus | 113.2 M | 370 k | 535 |
| Multiplayer & server | 110.1 M | 328 k | 459 |
| Art project | 88.4 M | 207 k | 314 |
| Riders & bikes | 68.1 M | 312 k | 281 |
| Materials & look | 32.3 M | 124 k | 170 |
| Build, release & infra | 21.0 M | 78 k | 166 |
| Sky, time & weather | 10.4 M | 27 k | 23 |
| Planning & spec | 2.0 M | 4 k | 8 |

| Task | Feature | Tokens | Output |
|---|---|---|---|
| agent: Phase 6 voice chat start | Audio & voice | 84.5 M | 213 k |
| agent: Multiplayer test + make test- prefix | Multiplayer & server | 66.7 M | 189 k |
| agent: Phase 3: neighbour packages at runtime | Worlds & terrain | 61.2 M | 181 k |
| agent: Local events: group ride/race/TT | Other | 54.1 M | 166 k |
| agent: Phase 4 bike/character models | Riders & bikes | 46.3 M | 206 k |
| coordination: keep adding created images or videos to the build-history | Art project | 45.8 M | 88 k |
| agent: Menu M3: in-app ride switch + Navi | UI & menus | 37.6 M | 132 k |
| agent: Build Black Forest world | Worlds & terrain | 37.6 M | 102 k |
| agent: Fix six rendering/gameplay bugs | Other | 36.5 M | 134 k |
| agent: Game audio M5 | Audio & voice | 34.9 M | 138 k |
| agent: ISO timestamps in captures | UI & menus | 30.2 M | 123 k |
| agent: Multiplayer frame rate + FPS counter | Multiplayer & server | 25.8 M | 93 k |
| agent: Camera/rider in tunnels (km 8.8) | Materials & look | 25.1 M | 104 k |
| agent: Menu M2 settings + rider profiles | UI & menus | 22.9 M | 47 k |
| agent: Phase 4 groundwork: profiles, bikes | Riders & bikes | 21.7 M | 106 k |
| coordination: i ran the chown command as you suggested. remove the artwork/build-history entir… | Art project | 21.4 M | 35 k |
| agent: Fix U-turn oscillation in free ride | Other | 19.5 M | 64 k |
| agent: Scene render test harness | Other | 17.0 M | 69 k |
| agent: Fix stall on steep ramp spike | Other | 16.9 M | 83 k |
| agent: Events/workouts polish: FIT laps, ghost | Other | 16.1 M | 64 k |
| coordination: BG Task: Create a new world: The Blackforest. | Worlds & terrain | 14.7 M | 26 k |
| coordination: are there any open build items / issues? | Build, release & infra | 13.7 M | 50 k |
| agent: Phase 5 groundwork: QUIC transport | Multiplayer & server | 13.1 M | 36 k |
| coordination: yes, fix the bugs above. | Other | 12.9 M | 15 k |
| agent: Gradient feel: HUD + camera pitch | UI & menus | 9.4 M | 16 k |
| agent: Workouts: TOML, .erg/.mrc, ERG | Other | 7.8 M | 12 k |
| coordination: I want to creat an Art Project on the entire Build Process of this OCW App. Ther… | Art project | 7.4 M | 53 k |
| coordination: Put a couple of photos showing multiple risers and mountains on the projects git… | Art project | 7.4 M | 10 k |
| agent: HUD/menu font with full Latin glyphs | UI & menus | 6.8 M | 32 k |
| coordination: time question: i mean the time of where the rider is in the virtual world, not t… | Sky, time & weather | 6.0 M | 7 k |
| coordination: regarding the build-history, yes, keep that out of the github history as well. p… | Art project | 5.7 M | 18 k |
| coordination: bg task: fix the performance issue in multiplayer. the videos show a video with … | Multiplayer & server | 4.4 M | 10 k |
| agent: Freehub click only when coasting | Audio & voice | 3.7 M | 39 k |
| coordination: Planning: Think about where to get textures from. The have to be license free or… | Materials & look | 3.6 M | 14 k |
| coordination: why is the exo/sentinel license problematic? another suggestion for textures and… | Materials & look | 3.6 M | 5 k |
| coordination: Important Addon: for every photo and video you take, we need to embedd the ISO D… | UI & menus | 3.4 M | 11 k |
| coordination: gh is logged in on deb machine. release the world and the binaries via gh now. a… | Worlds & terrain | 2.8 M | 14 k |
| coordination: Add planning for season and weather simulation for a later phase. also circadian… | Sky, time & weather | 2.6 M | 13 k |
| coordination: you can use the mac and windows build machines too. they are a bit slower so mig… | Build, release & infra | 2.1 M | 2 k |
| coordination: Your question: Weather physics: should wind and wet roads change the physics, an… | Sky, time & weather | 1.9 M | 7 k |

## Prompts

41 prompts (full text: [prompts.md](prompts.md)).

| Time | Prompt |
|---|---|
| 00:06 | keep adding created images or videos to the build-history |
| 06:30 | build linux and windows release |
| 06:52 | Why aren't we releasing the Liechtenstein world file? I mean, is there any license restriction, or is it just due to the big size? |
| 06:54 | Yeah, better release it with GitHub release, and also release the binaries with GitHub release. |
| 06:58 | What's the development status? |
| 07:00 | What do you mean by phase 5 backend? And which exactly steps are you referring to? Finishing phase 3. What about phase 4? And are there any  |
| 07:04 | postpone the root editor for now. Start with phase for and models and audio, and start also with the live communication feature. |
| 07:12 | Let's refine the specification for the voice chat feature. So it should be always on, similar to video conferences and calls, Teams or Googl |
| 07:33 | better switch to clang 21, let's install from debian backports |
| 07:35 | gh is logged in on deb machine. release the world and the binaries via gh now. also remove the binaries from the git history, |
| 07:42 | add installing the required clang version (here 21) via llvm debian repo into make deps-linux-debian |
| 07:49 | why can'T you build the voice option with rust? |
| 07:52 | okay. what are the options for voice when thinking about ios/android and pure browser option? |
| 07:54 | yes, stay with option 1 and continue as proposed |
| 08:10 | i need the make deps-llvm-debian command to install clang on the system. |
| 08:12 | can you verify we need libglib2.0-dev? if yes, add it better to the make deps-linux section. update that on marge. |
| 08:14 | done |
| 08:25 | bg task: build a test harness that renders various scenes. take photos and videos, add them to the artwork/build-history |
| 08:31 | bg task: make a multiplayer test where marge runs the server and GUI in kiosk mode. run headless bots on the available build machines and ha |
| 08:34 | regarding the build-history, yes, keep that out of the github history as well. package it as zip file and put it next to the releases. open  |
| 08:44 | i ran the chown command as you suggested. remove the artwork/build-history entirely |
| 09:33 | yes, fix the bugs above. |
| 10:26 | bg task: note that the typical clicking sound comes from the freehup and is only audible when the rider is not pedalling. fix that. |
| 10:28 | bg task: fix the performance issue in multiplayer. the videos show a video with a low frame rate (is the fps counter still visible? couldnt  |
| 12:19 | Put a couple of photos showing multiple risers and mountains on the projects github start page. Add the logo there too. |
| 16:28 | check if all build artifacts are uptodate and release all platforms in a new release. |
| 16:28 | did the multiplayer test finish sucessfully? |
| 16:29 | are there any open build items / issues? |
| 21:29 | I want to creat an Art Project on the entire Build Process of this OCW App. Therefore it is super important to document every step we do wit |
| 21:40 | BG Task: Create a new world: The Blackforest. |
| 22:13 | Important Addon: for every photo and video you take, we need to embedd the ISO Date and timestamp into bothe the file and the image itself.  |
| 22:17 | increase the quality to 90% of the screenshots and videos taken. use jpg rather than png and a higher resulution. make sure the exportet and |
| 22:21 | Planning: Think about where to get textures from. The have to be license free or with an open license like MIT/Apache 2.0 compatible. Are th |
| 22:26 | you can use the mac and windows build machines too. they are a bit slower so might be better suited for smaller dev tasks but should be used |
| 22:37 | Add planning for season and weather simulation for a later phase. also circadian (day/night/position of the sun and moon and stars, movement |
| 22:40 | why is the exo/sentinel license problematic? another suggestion for textures and design of objects: look for photos on the internet e.g wiki |
| 22:46 | time question: i mean the time of where the rider is in the virtual world, not the time the physical human is. weather should be realistic a |
| 22:57 | earlier you asked if to use more precise hight data: yes, use the best data you can find. same for the OSM, texture and other images. |
| 22:57 | create some pictures and videos (with timestamps as defined earlier) to show the region. Show a couple of well known places like mummelsee,  |
| 22:57 | Your question: Weather physics: should wind and wet roads change the physics, and in races too? I propose on in solo and free ride, off in r |
| 22:59 | add planning for the browser based app |

## Media

| Time | File |
|---|---|
| 00:17:24 | [001724-shot-wayland-gf-hills.png](001724-shot-wayland-gf-hills.png) |
| 00:17:42 | [001742-shot-wayland-gf-hills-notilt.png](001742-shot-wayland-gf-hills-notilt.png) |
| 00:19:00 | [001900-shot-wayland-gf-pilot-1.5.png](001900-shot-wayland-gf-pilot-1.5.png) |
| 00:19:19 | [001919-shot-wayland-gf-pilot-4.0.png](001919-shot-wayland-gf-pilot-4.0.png) |
| 00:19:38 | [001938-shot-wayland-gf-pilot-7.0.png](001938-shot-wayland-gf-pilot-7.0.png) |
| 00:20:11 | [002011-shot-wayland-ocw-workout-agent.png](002011-shot-wayland-ocw-workout-agent.png) |
| 00:20:30 | [002030-shot-wayland-gf-steep-1663,3336.png](002030-shot-wayland-gf-steep-1663,3336.png) |
| 00:20:49 | [002049-shot-wayland-gf-steep-983,4503.png](002049-shot-wayland-gf-steep-983,4503.png) |
| 00:21:08 | [002108-shot-wayland-gf-steep378,6005.png](002108-shot-wayland-gf-steep378,6005.png) |
| 00:21:48 | [002148-video-wayland-ocw-workout-agent.mp4](002148-video-wayland-ocw-workout-agent.mp4) |
| 00:21:48 | [002148-video-wayland-ocw-workout-agent.png](002148-video-wayland-ocw-workout-agent.png) |
| 00:30:48 | [003048-video-wayland-gf-hills.mp4](003048-video-wayland-gf-hills.mp4) |
| 00:30:48 | [003048-video-wayland-gf-hills.png](003048-video-wayland-gf-hills.png) |
| 00:33:32 | [003332-agent-remote-server.png](003332-agent-remote-server.png) |
| 00:34:46 | [003446-shot-wayland-ocw-m2-workshop.png](003446-shot-wayland-ocw-m2-workshop.png) |
| 00:35:01 | [003501-shot-wayland-ocw-m2-display.png](003501-shot-wayland-ocw-m2-display.png) |
| 00:37:03 | [003703-shot-wayland-ocw-main-workout.png](003703-shot-wayland-ocw-main-workout.png) |
| 00:57:13 | [005713-shot-wayland-ocw-font-hud.png](005713-shot-wayland-ocw-font-hud.png) |
| 00:57:24 | [005724-shot-wayland-ocw-font-menu.png](005724-shot-wayland-ocw-font-menu.png) |
| 00:58:26 | [005826-shot-wayland-ocw-font-staedtle.png](005826-shot-wayland-ocw-font-staedtle.png) |
| 01:05:31 | [010531-shot-wayland-a06-border-top.png](010531-shot-wayland-a06-border-top.png) |
| 01:07:58 | [010758-shot-wayland-a06-dir.png](010758-shot-wayland-a06-dir.png) |
| 01:09:12 | [010912-shot-wayland-a06-p-2009.png](010912-shot-wayland-a06-p-2009.png) |
| 01:09:20 | [010920-shot-wayland-a06-p-2214.png](010920-shot-wayland-a06-p-2214.png) |
| 01:09:27 | [010927-shot-wayland-a06-p-2728.png](010927-shot-wayland-a06-p-2728.png) |
| 01:09:34 | [010934-shot-wayland-a06-p-2780.png](010934-shot-wayland-a06-p-2780.png) |
| 01:09:42 | [010942-shot-wayland-a06-p-2905.png](010942-shot-wayland-a06-p-2905.png) |
| 01:09:49 | [010949-shot-wayland-a06-p-3023.png](010949-shot-wayland-a06-p-3023.png) |
| 01:10:12 | [011012-video-wayland-a06-border.mp4](011012-video-wayland-a06-border.mp4) |
| 01:10:12 | [011012-video-wayland-a06-border.png](011012-video-wayland-a06-border.png) |
| 01:12:38 | [011238-shot-wayland-a06-cmp.png](011238-shot-wayland-a06-cmp.png) |
| 01:12:59 | [011259-shot-wayland-a06-cmp.png](011259-shot-wayland-a06-cmp.png) |
| 01:13:16 | [011316-shot-wayland-a06-n-2905.png](011316-shot-wayland-a06-n-2905.png) |
| 01:13:25 | [011325-shot-wayland-a06-n-3023.png](011325-shot-wayland-a06-n-3023.png) |
| 01:13:35 | [011335-shot-wayland-a06-n-2728.png](011335-shot-wayland-a06-n-2728.png) |
| 01:13:44 | [011344-shot-wayland-a06-n-2471.png](011344-shot-wayland-a06-n-2471.png) |
| 01:13:53 | [011353-shot-wayland-a06-n-2009.png](011353-shot-wayland-a06-n-2009.png) |
| 01:14:03 | [011403-shot-wayland-a06-n-3160.png](011403-shot-wayland-a06-n-3160.png) |
| 01:14:24 | [011424-video-wayland-a06-landstrasse.mp4](011424-video-wayland-a06-landstrasse.mp4) |
| 01:14:24 | [011424-video-wayland-a06-landstrasse.png](011424-video-wayland-a06-landstrasse.png) |
| 01:14:35 | [011435-shot-wayland-a06-landstrasse-side.png](011435-shot-wayland-a06-landstrasse-side.png) |
| 01:15:01 | [011501-frame-landstrasse-11.png](011501-frame-landstrasse-11.png) |
| 01:15:01 | [011501-frame-landstrasse-3.png](011501-frame-landstrasse-3.png) |
| 01:15:01 | [011501-frame-landstrasse-7.png](011501-frame-landstrasse-7.png) |
| 01:20:18 | [012018-video-wayland-a06-landstrasse2.mp4](012018-video-wayland-a06-landstrasse2.mp4) |
| 01:20:18 | [012018-video-wayland-a06-landstrasse2.png](012018-video-wayland-a06-landstrasse2.png) |
| 01:20:28 | [012028-shot-wayland-a06-seam-top.png](012028-shot-wayland-a06-seam-top.png) |
| 01:20:37 | [012037-frame2-landstrasse-6.png](012037-frame2-landstrasse-6.png) |
| 01:20:37 | [012037-frame2-landstrasse-7.png](012037-frame2-landstrasse-7.png) |
| 01:20:37 | [012037-frame2-landstrasse-8.png](012037-frame2-landstrasse-8.png) |
| 01:35:15 | [013515-shot-wayland-ocw-bikes-side.png](013515-shot-wayland-ocw-bikes-side.png) |
| 01:35:23 | [013523-shot-wayland-ocw-bikes-chase.png](013523-shot-wayland-ocw-bikes-chase.png) |
| 01:35:28 | [013528-shot-wayland-ocw-workshop-bike.png](013528-shot-wayland-ocw-workshop-bike.png) |
| 01:36:09 | [013609-shot-wayland-ocw-bike-local-tt.png](013609-shot-wayland-ocw-bike-local-tt.png) |
| 01:40:47 | [014047-shot-wayland-ev-countdown.png](014047-shot-wayland-ev-countdown.png) |
| 01:41:44 | [014144-shot-wayland-ev-countdown.png](014144-shot-wayland-ev-countdown.png) |
| 01:41:44 | [014144-video-wayland-a06-border-final.mp4](014144-video-wayland-a06-border-final.mp4) |
| 01:41:44 | [014144-video-wayland-a06-border-final.png](014144-video-wayland-a06-border-final.png) |
| 01:41:51 | [014151-frame3-final-7.png](014151-frame3-final-7.png) |
| 01:42:15 | [014215-shot-wayland-ev-race.png](014215-shot-wayland-ev-race.png) |
| 01:43:12 | [014312-shot-wayland-ev-results.png](014312-shot-wayland-ev-results.png) |
| 01:44:33 | [014433-shot-wayland-ev-tt-ghost.png](014433-shot-wayland-ev-tt-ghost.png) |
| 01:44:45 | [014445-video-wayland-ev-go.mp4](014445-video-wayland-ev-go.mp4) |
| 01:44:45 | [014445-video-wayland-ev-go.png](014445-video-wayland-ev-go.png) |
| 01:55:46 | [015546-shot-wayland-ev-results.png](015546-shot-wayland-ev-results.png) |
| 01:56:16 | [015616-shot-wayland-ev-tt-ghost.png](015616-shot-wayland-ev-tt-ghost.png) |
| 01:56:27 | [015627-video-wayland-ev-tt-ghost.mp4](015627-video-wayland-ev-tt-ghost.mp4) |
| 01:56:27 | [015627-video-wayland-ev-tt-ghost.png](015627-video-wayland-ev-tt-ghost.png) |
| 02:15:27 | [021527-shot-wayland-ev-race.png](021527-shot-wayland-ev-race.png) |
| 02:16:23 | [021623-shot-wayland-ev-results.png](021623-shot-wayland-ev-results.png) |
| 02:16:34 | [021634-video-wayland-ev-tt-ghost.mp4](021634-video-wayland-ev-tt-ghost.mp4) |
| 02:16:34 | [021634-video-wayland-ev-tt-ghost.png](021634-video-wayland-ev-tt-ghost.png) |
| 02:29:19 | [022919-shot-wayland-m3-ride-pages.png](022919-shot-wayland-m3-ride-pages.png) |
| 02:29:42 | [022942-shot-wayland-m3-navi-routes.png](022942-shot-wayland-m3-navi-routes.png) |
| 02:31:38 | [023138-video-wayland-m3-switch.mp4](023138-video-wayland-m3-switch.mp4) |
| 02:31:38 | [023138-video-wayland-m3-switch.png](023138-video-wayland-m3-switch.png) |
| 02:31:43 | [023143-shot-wayland-m3-change-ride.png](023143-shot-wayland-m3-change-ride.png) |
| 02:36:08 | [023608-video-wayland-ocw-uturn-ffj.mp4](023608-video-wayland-ocw-uturn-ffj.mp4) |
| 02:36:08 | [023608-video-wayland-ocw-uturn-ffj.png](023608-video-wayland-ocw-uturn-ffj.png) |
| 02:50:51 | [025051-agent-baseline-af2d-base-88.png](025051-agent-baseline-af2d-base-88.png) |
| 02:51:26 | [025126-shot-wayland-af2d-8.6.png](025126-shot-wayland-af2d-8.6.png) |
| 02:51:35 | [025135-shot-wayland-af2d-8.95.png](025135-shot-wayland-af2d-8.95.png) |
| 02:51:44 | [025144-shot-wayland-af2d-9.3.png](025144-shot-wayland-af2d-9.3.png) |
| 02:52:57 | [025257-shot-wayland-af2d-8.95.png](025257-shot-wayland-af2d-8.95.png) |
| 02:54:10 | [025410-shot-wayland-af2d-tunnel-km8.0.png](025410-shot-wayland-af2d-tunnel-km8.0.png) |
| 02:54:17 | [025417-shot-wayland-af2d-tunnel-km8.75.png](025417-shot-wayland-af2d-tunnel-km8.75.png) |
| 02:54:25 | [025425-shot-wayland-af2d-tunnel-km8.8.png](025425-shot-wayland-af2d-tunnel-km8.8.png) |
| 02:54:32 | [025432-shot-wayland-af2d-tunnel-km9.2.png](025432-shot-wayland-af2d-tunnel-km9.2.png) |
| 02:54:39 | [025439-shot-wayland-af2d-tunnel-km9.5.png](025439-shot-wayland-af2d-tunnel-km9.5.png) |
| 02:54:47 | [025447-shot-wayland-af2d-tunnel-side.png](025447-shot-wayland-af2d-tunnel-side.png) |
| 02:54:57 | [025457-video-wayland-af2d-tunnel-entry.mp4](025457-video-wayland-af2d-tunnel-entry.mp4) |
| 02:54:57 | [025457-video-wayland-af2d-tunnel-entry.png](025457-video-wayland-af2d-tunnel-entry.png) |
| 02:55:08 | [025508-video-wayland-af2d-tunnel-exit.mp4](025508-video-wayland-af2d-tunnel-exit.mp4) |
| 02:55:08 | [025508-video-wayland-af2d-tunnel-exit.png](025508-video-wayland-af2d-tunnel-exit.png) |
| 02:55:57 | [025557-video-wayland-af2d-tunnel-entry2.mp4](025557-video-wayland-af2d-tunnel-entry2.mp4) |
| 02:55:57 | [025557-video-wayland-af2d-tunnel-entry2.png](025557-video-wayland-af2d-tunnel-entry2.png) |
| 03:16:46 | [031646-shot-wayland-corner.png](031646-shot-wayland-corner.png) |
| 03:16:51 | [031651-video-wayland-af2d-tunnel-entry3.mp4](031651-video-wayland-af2d-tunnel-entry3.mp4) |
| 03:16:51 | [031651-video-wayland-af2d-tunnel-entry3.png](031651-video-wayland-af2d-tunnel-entry3.png) |
| 03:18:01 | [031801-shot-wayland-climb.png](031801-shot-wayland-climb.png) |
| 03:18:21 | [031821-video-wayland-af2d-tunnel-entry4.mp4](031821-video-wayland-af2d-tunnel-entry4.mp4) |
| 03:19:51 | [031951-video-wayland-af2d-tunnel-entry5.mp4](031951-video-wayland-af2d-tunnel-entry5.mp4) |
| 03:20:51 | [032051-video-wayland-af2d-tunnel-entry.mp4](032051-video-wayland-af2d-tunnel-entry.mp4) |
| 03:20:51 | [032051-video-wayland-af2d-tunnel-entry.png](032051-video-wayland-af2d-tunnel-entry.png) |
| 03:21:01 | [032101-video-wayland-af2d-tunnel-exit.mp4](032101-video-wayland-af2d-tunnel-exit.mp4) |
| 03:21:01 | [032101-video-wayland-af2d-tunnel-exit.png](032101-video-wayland-af2d-tunnel-exit.png) |
| 03:21:09 | [032109-shot-wayland-af2d-tunnel-km8.8.png](032109-shot-wayland-af2d-tunnel-km8.8.png) |
| 03:21:16 | [032116-shot-wayland-af2d-tunnel-km9.2.png](032116-shot-wayland-af2d-tunnel-km9.2.png) |
| 03:44:43 | [034443-shot-wayland-ocw-workouts.png](034443-shot-wayland-ocw-workouts.png) |
| 03:45:06 | [034506-shot-wayland-ocw-tt-ghost.png](034506-shot-wayland-ocw-tt-ghost.png) |
| 03:45:56 | [034556-shot-wayland-ocw-tt-ghost.png](034556-shot-wayland-ocw-tt-ghost.png) |
| 03:46:32 | [034632-shot-wayland-ocw-tt-ghost.png](034632-shot-wayland-ocw-tt-ghost.png) |
| 03:56:25 | [035625-shot-wayland-ocw-final.png](035625-shot-wayland-ocw-final.png) |
| 07:54:20 | [075420-audio-ride-sounds.wav](075420-audio-ride-sounds.wav) |
| 07:56:54 | [075654-audio-settings-page.png](075654-audio-settings-page.png) |
| 08:06:42 | [080642-models-side.png](080642-models-side.png) |
| 08:06:53 | [080653-models-group.png](080653-models-group.png) |
| 08:07:39 | [080739-models-sprint.png](080739-models-sprint.png) |
| 08:07:52 | [080752-models-freewheel.png](080752-models-freewheel.png) |
| 08:08:05 | [080805-models-bikes.png](080805-models-bikes.png) |
| 08:08:11 | [080811-models-ride.mp4](080811-models-ride.mp4) |
| 08:08:11 | [080811-video-wayland-m-ride.png](080811-video-wayland-m-ride.png) |
| 08:25:39 | [082539-shot-wayland-m-tt.png](082539-shot-wayland-m-tt.png) |
| 08:25:51 | [082551-models-mtb.png](082551-models-mtb.png) |
| 08:36:49 | [083649-models-tt.png](083649-models-tt.png) |
| 08:36:59 | [083659-shot-wayland-m-smoke.png](083659-shot-wayland-m-smoke.png) |
| 08:42:21 | [084221-shot-wayland-x-route.png](084221-shot-wayland-x-route.png) |
| 08:42:31 | [084231-shot-wayland-x-free.png](084231-shot-wayland-x-free.png) |
| 08:43:16 | [084316-shot-wayland-x-route2.png](084316-shot-wayland-x-route2.png) |
| 08:43:22 | [084322-shot-wayland-x-freeA.png](084322-shot-wayland-x-freeA.png) |
| 08:43:29 | [084329-shot-wayland-x-freeB.png](084329-shot-wayland-x-freeB.png) |
| 08:43:36 | [084336-shot-wayland-x-menu.png](084336-shot-wayland-x-menu.png) |
| 08:44:11 | [084411-voice-hud.png](084411-voice-hud.png) |
| 08:44:24 | [084424-shot-wayland-x-t8.4.png](084424-shot-wayland-x-t8.4.png) |
| 08:44:30 | [084430-shot-wayland-x-t8.5.png](084430-shot-wayland-x-t8.5.png) |
| 08:44:36 | [084436-shot-wayland-x-t8.6.png](084436-shot-wayland-x-t8.6.png) |
| 08:44:43 | [084443-shot-wayland-x-t8.7.png](084443-shot-wayland-x-t8.7.png) |
| 08:44:49 | [084449-shot-wayland-x-t9.1.png](084449-shot-wayland-x-t9.1.png) |
| 08:44:56 | [084456-shot-wayland-x-t9.3.png](084456-shot-wayland-x-t9.3.png) |
| 08:45:02 | [084502-shot-wayland-x-t9.5.png](084502-shot-wayland-x-t9.5.png) |
| 08:45:10 | [084510-shot-wayland-x-rhine.png](084510-shot-wayland-x-rhine.png) |
| 08:46:03 | [084603-shot-wayland-x-race.png](084603-shot-wayland-x-race.png) |
| 08:52:40 | [085240-shot-wayland-testroute-hills.png](085240-shot-wayland-testroute-hills.png) |
| 08:52:48 | [085248-shot-wayland-console-overlay.png](085248-shot-wayland-console-overlay.png) |
| 08:53:04 | [085304-shot-wayland-workout-erg.png](085304-shot-wayland-workout-erg.png) |
| 08:53:56 | [085356-shot-wayland-race-results.png](085356-shot-wayland-race-results.png) |
| 08:54:53 | [085453-shot-wayland-tt-ghost.png](085453-shot-wayland-tt-ghost.png) |
| 08:54:59 | [085459-shot-wayland-menu-main.png](085459-shot-wayland-menu-main.png) |
| 08:55:06 | [085506-shot-wayland-menu-settings.png](085506-shot-wayland-menu-settings.png) |
| 08:55:15 | [085515-shot-wayland-menu-settings-display.png](085515-shot-wayland-menu-settings-display.png) |
| 08:55:22 | [085522-shot-wayland-menu-workshop-bike.png](085522-shot-wayland-menu-workshop-bike.png) |
| 08:55:29 | [085529-shot-wayland-menu-workouts.png](085529-shot-wayland-menu-workouts.png) |
| 08:55:35 | [085535-shot-wayland-menu-ride.png](085535-shot-wayland-menu-ride.png) |
| 08:55:42 | [085542-shot-wayland-route-start-formation.png](085542-shot-wayland-route-start-formation.png) |
| 08:55:56 | [085556-video-wayland-race-start-video.mp4](085556-video-wayland-race-start-video.mp4) |
| 08:55:56 | [085556-video-wayland-race-start-video.png](085556-video-wayland-race-start-video.png) |
| 08:56:07 | [085607-shot-wayland-camera-chase.png](085607-shot-wayland-camera-chase.png) |
| 08:56:18 | [085618-shot-wayland-camera-side.png](085618-shot-wayland-camera-side.png) |
| 08:56:29 | [085629-shot-wayland-camera-top.png](085629-shot-wayland-camera-top.png) |
| 08:56:38 | [085638-shot-wayland-camera-close.png](085638-shot-wayland-camera-close.png) |
| 08:56:54 | [085654-video-wayland-orbit-video.mp4](085654-video-wayland-orbit-video.mp4) |
| 08:56:54 | [085654-video-wayland-orbit-video.png](085654-video-wayland-orbit-video.png) |
| 08:57:05 | [085705-shot-wayland-malbun-climb.png](085705-shot-wayland-malbun-climb.png) |
| 08:57:18 | [085718-video-wayland-malbun-climb-video.mp4](085718-video-wayland-malbun-climb-video.mp4) |
| 08:57:18 | [085718-video-wayland-malbun-climb-video.png](085718-video-wayland-malbun-climb-video.png) |
| 08:57:29 | [085729-shot-wayland-malbun-arrival.png](085729-shot-wayland-malbun-arrival.png) |
| 08:57:39 | [085739-shot-wayland-tunnel-entry.png](085739-shot-wayland-tunnel-entry.png) |
| 08:57:49 | [085749-shot-wayland-tunnel-inside.png](085749-shot-wayland-tunnel-inside.png) |
| 08:57:59 | [085759-video-wayland-tunnel-video.mp4](085759-video-wayland-tunnel-video.mp4) |
| 08:57:59 | [085759-video-wayland-tunnel-video.png](085759-video-wayland-tunnel-video.png) |
| 08:58:08 | [085808-shot-wayland-vaduz-buildings.png](085808-shot-wayland-vaduz-buildings.png) |
| 08:58:18 | [085818-video-wayland-vaduz-video.mp4](085818-video-wayland-vaduz-video.mp4) |
| 08:58:18 | [085818-video-wayland-vaduz-video.png](085818-video-wayland-vaduz-video.png) |
| 08:58:19 | [085819-shot-wayland-ocw-p4.png](085819-shot-wayland-ocw-p4.png) |
| 08:58:25 | [085825-shot-wayland-freeride-junction.png](085825-shot-wayland-freeride-junction.png) |
| 08:58:34 | [085834-shot-wayland-rhine-water.png](085834-shot-wayland-rhine-water.png) |
| 08:58:47 | [085847-shot-wayland-far-view.png](085847-shot-wayland-far-view.png) |
| 08:58:55 | [085855-shot-wayland-menu-navi-routes.png](085855-shot-wayland-menu-navi-routes.png) |
| 09:01:26 | [090126-shot-wayland-ocw-main.png](090126-shot-wayland-ocw-main.png) |
| 09:01:34 | [090134-shot-wayland-route-start-formation.png](090134-shot-wayland-route-start-formation.png) |
| 09:01:43 | [090143-video-wayland-race-start-video.mp4](090143-video-wayland-race-start-video.mp4) |
| 09:01:43 | [090143-video-wayland-race-start-video.png](090143-video-wayland-race-start-video.png) |
| 09:02:00 | [090200-shot-wayland-camera-chase.png](090200-shot-wayland-camera-chase.png) |
| 09:02:11 | [090211-shot-wayland-ocw-a.png](090211-shot-wayland-ocw-a.png) |
| 09:02:18 | [090218-shot-wayland-camera-side.png](090218-shot-wayland-camera-side.png) |
| 09:02:26 | [090226-shot-wayland-ocw-b.png](090226-shot-wayland-ocw-b.png) |
| 09:02:35 | [090235-shot-wayland-camera-top.png](090235-shot-wayland-camera-top.png) |
| 09:02:44 | [090244-shot-wayland-camera-close.png](090244-shot-wayland-camera-close.png) |
| 09:02:54 | [090254-shot-wayland-malbun-climb.png](090254-shot-wayland-malbun-climb.png) |
| 09:03:04 | [090304-video-wayland-malbun-climb-video.mp4](090304-video-wayland-malbun-climb-video.mp4) |
| 09:03:04 | [090304-video-wayland-malbun-climb-video.png](090304-video-wayland-malbun-climb-video.png) |
| 09:03:13 | [090313-shot-wayland-tunnel-entry.png](090313-shot-wayland-tunnel-entry.png) |
| 09:03:23 | [090323-shot-wayland-tunnel-inside.png](090323-shot-wayland-tunnel-inside.png) |
| 09:03:38 | [090338-video-wayland-tunnel-video.mp4](090338-video-wayland-tunnel-video.mp4) |
| 09:03:38 | [090338-video-wayland-tunnel-video.png](090338-video-wayland-tunnel-video.png) |
| 09:03:46 | [090346-shot-wayland-vaduz-buildings.png](090346-shot-wayland-vaduz-buildings.png) |
| 09:03:55 | [090355-shot-wayland-rhine-water.png](090355-shot-wayland-rhine-water.png) |
| 09:04:09 | [090409-shot-wayland-far-view.png](090409-shot-wayland-far-view.png) |
| 09:04:16 | [090416-shot-wayland-menu-navi.png](090416-shot-wayland-menu-navi.png) |
| 09:04:30 | [090430-shot-wayland-far-view-b.png](090430-shot-wayland-far-view-b.png) |
| 09:04:44 | [090444-shot-wayland-far-view-c.png](090444-shot-wayland-far-view-c.png) |
| 09:04:54 | [090454-shot-wayland-tunnel-entry-b.png](090454-shot-wayland-tunnel-entry-b.png) |
| 09:05:47 | [090547-shot-wayland-tunnel-entry.png](090547-shot-wayland-tunnel-entry.png) |
| 09:05:59 | [090559-video-wayland-tunnel-video.mp4](090559-video-wayland-tunnel-video.mp4) |
| 09:05:59 | [090559-video-wayland-tunnel-video.png](090559-video-wayland-tunnel-video.png) |
| 09:06:07 | [090607-shot-wayland-vaduz-buildings.png](090607-shot-wayland-vaduz-buildings.png) |
| 09:06:20 | [090620-shot-wayland-far-view.png](090620-shot-wayland-far-view.png) |
| 09:07:05 | [090705-testroute-hills.png](090705-testroute-hills.png) |
| 09:07:12 | [090712-console-overlay.png](090712-console-overlay.png) |
| 09:07:13 | [090713-shot-wayland-console-overlay.png](090713-shot-wayland-console-overlay.png) |
| 09:07:27 | [090727-workout-erg.png](090727-workout-erg.png) |
| 09:07:28 | [090728-shot-wayland-workout-erg.png](090728-shot-wayland-workout-erg.png) |
| 09:08:19 | [090819-race-results.png](090819-race-results.png) |
| 09:08:19 | [090819-shot-wayland-race-results.png](090819-shot-wayland-race-results.png) |
| 09:09:16 | [090916-shot-wayland-tt-ghost.png](090916-shot-wayland-tt-ghost.png) |
| 09:09:16 | [090916-tt-ghost.png](090916-tt-ghost.png) |
| 09:09:21 | [090921-menu-main.png](090921-menu-main.png) |
| 09:09:28 | [090928-menu-settings.png](090928-menu-settings.png) |
| 09:09:36 | [090936-menu-settings-display.png](090936-menu-settings-display.png) |
| 09:09:43 | [090943-menu-workshop-bike.png](090943-menu-workshop-bike.png) |
| 09:09:50 | [090950-menu-workouts.png](090950-menu-workouts.png) |
| 09:09:56 | [090956-menu-ride.png](090956-menu-ride.png) |
| 09:10:03 | [091003-route-start-formation.png](091003-route-start-formation.png) |
| 09:10:03 | [091003-shot-wayland-route-start-formation.png](091003-shot-wayland-route-start-formation.png) |
| 09:10:12 | [091012-race-start-video.mp4](091012-race-start-video.mp4) |
| 09:10:12 | [091012-video-wayland-race-start-video.png](091012-video-wayland-race-start-video.png) |
| 09:10:29 | [091029-camera-chase.png](091029-camera-chase.png) |
| 09:10:30 | [091030-shot-wayland-camera-chase.png](091030-shot-wayland-camera-chase.png) |
| 09:10:47 | [091047-camera-side.png](091047-camera-side.png) |
| 09:10:47 | [091047-shot-wayland-camera-side.png](091047-shot-wayland-camera-side.png) |
| 09:11:04 | [091104-camera-top.png](091104-camera-top.png) |
| 09:11:04 | [091104-shot-wayland-camera-top.png](091104-shot-wayland-camera-top.png) |
| 09:11:16 | [091116-camera-close.png](091116-camera-close.png) |
| 09:11:16 | [091116-shot-wayland-camera-close.png](091116-shot-wayland-camera-close.png) |
| 09:11:29 | [091129-orbit-video.mp4](091129-orbit-video.mp4) |
| 09:11:29 | [091129-video-wayland-orbit-video.png](091129-video-wayland-orbit-video.png) |
| 09:11:39 | [091139-malbun-climb.png](091139-malbun-climb.png) |
| 09:11:40 | [091140-shot-wayland-malbun-climb.png](091140-shot-wayland-malbun-climb.png) |
| 09:11:50 | [091150-malbun-climb-video.mp4](091150-malbun-climb-video.mp4) |
| 09:11:50 | [091150-video-wayland-malbun-climb-video.png](091150-video-wayland-malbun-climb-video.png) |
| 09:12:01 | [091201-malbun-arrival.png](091201-malbun-arrival.png) |
| 09:12:01 | [091201-shot-wayland-malbun-arrival.png](091201-shot-wayland-malbun-arrival.png) |
| 09:12:10 | [091210-shot-wayland-tunnel-entry.png](091210-shot-wayland-tunnel-entry.png) |
| 09:12:10 | [091210-tunnel-entry.png](091210-tunnel-entry.png) |
| 09:12:20 | [091220-shot-wayland-tunnel-inside.png](091220-shot-wayland-tunnel-inside.png) |
| 09:12:20 | [091220-tunnel-inside.png](091220-tunnel-inside.png) |
| 09:12:37 | [091237-tunnel-video.mp4](091237-tunnel-video.mp4) |
| 09:12:37 | [091237-video-wayland-tunnel-video.png](091237-video-wayland-tunnel-video.png) |
| 09:12:45 | [091245-shot-wayland-vaduz-buildings.png](091245-shot-wayland-vaduz-buildings.png) |
| 09:12:55 | [091255-video-wayland-vaduz-video.mp4](091255-video-wayland-vaduz-video.mp4) |
| 09:12:55 | [091255-video-wayland-vaduz-video.png](091255-video-wayland-vaduz-video.png) |
| 09:13:03 | [091303-freeride-junction.png](091303-freeride-junction.png) |
| 09:13:03 | [091303-shot-wayland-freeride-junction.png](091303-shot-wayland-freeride-junction.png) |
| 09:13:12 | [091312-rhine-water.png](091312-rhine-water.png) |
| 09:13:12 | [091312-shot-wayland-rhine-water.png](091312-shot-wayland-rhine-water.png) |
| 09:13:25 | [091325-far-view.png](091325-far-view.png) |
| 09:13:25 | [091325-shot-wayland-far-view.png](091325-shot-wayland-far-view.png) |
| 09:13:31 | [091331-menu-navi.png](091331-menu-navi.png) |
| 09:22:06 | [092206-shot-wayland-ocw-main.png](092206-shot-wayland-ocw-main.png) |
| 09:22:23 | [092223-shot-wayland-vaduz-buildings.png](092223-shot-wayland-vaduz-buildings.png) |
| 09:23:01 | [092301-vaduz-buildings.png](092301-vaduz-buildings.png) |
| 09:23:02 | [092302-shot-wayland-vaduz-buildings.png](092302-shot-wayland-vaduz-buildings.png) |
| 09:23:10 | [092310-vaduz-video.mp4](092310-vaduz-video.mp4) |
| 09:23:10 | [092310-video-wayland-vaduz-video.png](092310-video-wayland-vaduz-video.png) |
| 09:50:35 | [095035-shot-wayland-menu-ride.png](095035-shot-wayland-menu-ride.png) |
| 09:50:46 | [095046-video-wayland-tunnel-video.mp4](095046-video-wayland-tunnel-video.mp4) |
| 09:50:46 | [095046-video-wayland-tunnel-video.png](095046-video-wayland-tunnel-video.png) |
| 09:50:53 | [095053-shot-wayland-event-bots.png](095053-shot-wayland-event-bots.png) |
| 09:50:59 | [095059-video-wayland-teleport-video.mp4](095059-video-wayland-teleport-video.mp4) |
| 09:50:59 | [095059-video-wayland-teleport-video.png](095059-video-wayland-teleport-video.png) |
| 09:51:08 | [095108-shot-wayland-route-end-teleport.png](095108-shot-wayland-route-end-teleport.png) |
| 09:51:18 | [095118-shot-wayland-world-only.png](095118-shot-wayland-world-only.png) |
| 10:08:37 | [100837-shot-wayland-menu-main.png](100837-shot-wayland-menu-main.png) |
| 10:08:44 | [100844-shot-wayland-menu-settings.png](100844-shot-wayland-menu-settings.png) |
| 10:08:50 | [100850-shot-wayland-menu-ride.png](100850-shot-wayland-menu-ride.png) |
| 10:08:59 | [100859-shot-wayland-tunnel-entry.png](100859-shot-wayland-tunnel-entry.png) |
| 10:09:08 | [100908-shot-wayland-tunnel-inside.png](100908-shot-wayland-tunnel-inside.png) |
| 10:09:20 | [100920-video-wayland-tunnel-video.mp4](100920-video-wayland-tunnel-video.mp4) |
| 10:09:27 | [100927-shot-wayland-event-bots.png](100927-shot-wayland-event-bots.png) |
| 10:09:33 | [100933-video-wayland-teleport-video.png](100933-video-wayland-teleport-video.png) |
| 10:09:42 | [100942-shot-wayland-route-end-teleport.png](100942-shot-wayland-route-end-teleport.png) |
| 10:09:51 | [100951-shot-wayland-world-only.png](100951-shot-wayland-world-only.png) |
| 10:10:28 | [101028-agent-tele-after-t005.png](101028-agent-tele-after-t005.png) |
| 10:10:28 | [101028-agent-tele-after-t020.png](101028-agent-tele-after-t020.png) |
| 10:10:29 | [101029-agent-tele-after-t050.png](101029-agent-tele-after-t050.png) |
| 10:10:32 | [101032-agent-tele-after-end.png](101032-agent-tele-after-end.png) |
| 10:11:18 | [101118-agent-tele-before-t005.png](101118-agent-tele-before-t005.png) |
| 10:11:18 | [101118-agent-tele-before-t020.png](101118-agent-tele-before-t020.png) |
| 10:11:18 | [101118-agent-tele-before-t050.png](101118-agent-tele-before-t050.png) |
| 10:11:22 | [101122-agent-tele-before-end.png](101122-agent-tele-before-end.png) |
| 10:12:04 | [101204-agent-tele-before2-t000.png](101204-agent-tele-before2-t000.png) |
| 10:12:04 | [101204-agent-tele-before2-t010.png](101204-agent-tele-before2-t010.png) |
| 10:12:04 | [101204-agent-tele-before2-t030.png](101204-agent-tele-before2-t030.png) |
| 10:12:07 | [101207-agent-tele-before2-end.png](101207-agent-tele-before2-end.png) |
| 10:12:11 | [101211-multiplayer-camera-camera.mp4](101211-multiplayer-camera-camera.mp4) |
| 10:12:11 | [101211-multiplayer-camera-final.png](101211-multiplayer-camera-final.png) |
| 10:12:11 | [101211-multiplayer-camera-still-30.png](101211-multiplayer-camera-still-30.png) |
| 10:12:11 | [101211-multiplayer-camera-still-45.png](101211-multiplayer-camera-still-45.png) |
| 10:12:11 | [101211-multiplayer-camera-still-60.png](101211-multiplayer-camera-still-60.png) |
| 10:16:42 | [101642-multiplayer-camera-camera.mp4](101642-multiplayer-camera-camera.mp4) |
| 10:16:42 | [101642-multiplayer-camera-still-30.png](101642-multiplayer-camera-still-30.png) |
| 10:16:42 | [101642-multiplayer-camera-still-45.png](101642-multiplayer-camera-still-45.png) |
| 10:16:42 | [101642-multiplayer-camera-still-60.png](101642-multiplayer-camera-still-60.png) |
| 10:17:10 | [101710-testroute-hills.png](101710-testroute-hills.png) |
| 10:17:17 | [101717-console-overlay.png](101717-console-overlay.png) |
| 10:17:18 | [101718-shot-wayland-console-overlay.png](101718-shot-wayland-console-overlay.png) |
| 10:17:33 | [101733-shot-wayland-workout-erg.png](101733-shot-wayland-workout-erg.png) |
| 10:17:33 | [101733-workout-erg.png](101733-workout-erg.png) |
| 10:18:24 | [101824-race-results.png](101824-race-results.png) |
| 10:18:24 | [101824-shot-wayland-race-results.png](101824-shot-wayland-race-results.png) |
| 10:19:21 | [101921-shot-wayland-tt-ghost.png](101921-shot-wayland-tt-ghost.png) |
| 10:19:21 | [101921-tt-ghost.png](101921-tt-ghost.png) |
| 10:19:26 | [101926-menu-main.png](101926-menu-main.png) |
| 10:19:33 | [101933-menu-settings.png](101933-menu-settings.png) |
| 10:19:41 | [101941-menu-settings-display.png](101941-menu-settings-display.png) |
| 10:19:48 | [101948-menu-workshop-bike.png](101948-menu-workshop-bike.png) |
| 10:19:55 | [101955-menu-workouts.png](101955-menu-workouts.png) |
| 10:20:01 | [102001-menu-ride.png](102001-menu-ride.png) |
| 10:20:08 | [102008-route-start-formation.png](102008-route-start-formation.png) |
| 10:20:09 | [102009-shot-wayland-route-start-formation.png](102009-shot-wayland-route-start-formation.png) |
| 10:20:18 | [102018-race-start-video.mp4](102018-race-start-video.mp4) |
| 10:20:18 | [102018-video-wayland-race-start-video.png](102018-video-wayland-race-start-video.png) |
| 10:20:35 | [102035-camera-chase.png](102035-camera-chase.png) |
| 10:20:35 | [102035-shot-wayland-camera-chase.png](102035-shot-wayland-camera-chase.png) |
| 10:20:52 | [102052-camera-side.png](102052-camera-side.png) |
| 10:20:52 | [102052-shot-wayland-camera-side.png](102052-shot-wayland-camera-side.png) |
| 10:21:10 | [102110-camera-top.png](102110-camera-top.png) |
| 10:21:10 | [102110-shot-wayland-camera-top.png](102110-shot-wayland-camera-top.png) |
| 10:21:18 | [102118-camera-close.png](102118-camera-close.png) |
| 10:21:18 | [102118-shot-wayland-camera-close.png](102118-shot-wayland-camera-close.png) |
| 10:21:27 | [102127-orbit-video.mp4](102127-orbit-video.mp4) |
| 10:21:27 | [102127-video-wayland-orbit-video.png](102127-video-wayland-orbit-video.png) |
| 10:21:38 | [102138-malbun-climb.png](102138-malbun-climb.png) |
| 10:21:38 | [102138-shot-wayland-malbun-climb.png](102138-shot-wayland-malbun-climb.png) |
| 10:21:46 | [102146-malbun-climb-video.mp4](102146-malbun-climb-video.mp4) |
| 10:21:46 | [102146-video-wayland-malbun-climb-video.png](102146-video-wayland-malbun-climb-video.png) |
| 10:21:56 | [102156-malbun-arrival.png](102156-malbun-arrival.png) |
| 10:21:56 | [102156-shot-wayland-malbun-arrival.png](102156-shot-wayland-malbun-arrival.png) |
| 10:22:05 | [102205-shot-wayland-tunnel-entry.png](102205-shot-wayland-tunnel-entry.png) |
| 10:22:05 | [102205-tunnel-entry.png](102205-tunnel-entry.png) |
| 10:22:14 | [102214-tunnel-inside.png](102214-tunnel-inside.png) |
| 10:22:15 | [102215-shot-wayland-tunnel-inside.png](102215-shot-wayland-tunnel-inside.png) |
| 10:22:26 | [102226-multiplayer-camera-still-60.png](102226-multiplayer-camera-still-60.png) |
| 10:22:26 | [102226-multiplayer-camera.mp4](102226-multiplayer-camera.mp4) |
| 10:22:26 | [102226-multiplayer-still-30.png](102226-multiplayer-still-30.png) |
| 10:22:26 | [102226-multiplayer-still-45.png](102226-multiplayer-still-45.png) |
| 10:22:26 | [102226-tunnel-video.mp4](102226-tunnel-video.mp4) |
| 10:22:26 | [102226-video-wayland-tunnel-video.png](102226-video-wayland-tunnel-video.png) |
| 10:22:34 | [102234-shot-wayland-vaduz-buildings.png](102234-shot-wayland-vaduz-buildings.png) |
| 10:22:34 | [102234-vaduz-buildings.png](102234-vaduz-buildings.png) |
| 10:22:42 | [102242-vaduz-video.mp4](102242-vaduz-video.mp4) |
| 10:22:42 | [102242-video-wayland-vaduz-video.png](102242-video-wayland-vaduz-video.png) |
| 10:22:49 | [102249-freeride-junction.png](102249-freeride-junction.png) |
| 10:22:49 | [102249-shot-wayland-freeride-junction.png](102249-shot-wayland-freeride-junction.png) |
| 10:22:57 | [102257-rhine-water.png](102257-rhine-water.png) |
| 10:22:58 | [102258-shot-wayland-rhine-water.png](102258-shot-wayland-rhine-water.png) |
| 10:23:11 | [102311-far-view.png](102311-far-view.png) |
| 10:23:11 | [102311-shot-wayland-far-view.png](102311-shot-wayland-far-view.png) |
| 10:23:17 | [102317-menu-navi.png](102317-menu-navi.png) |
| 10:23:24 | [102324-event-bots.png](102324-event-bots.png) |
| 10:23:24 | [102324-shot-wayland-event-bots.png](102324-shot-wayland-event-bots.png) |
| 10:23:30 | [102330-teleport-video.mp4](102330-teleport-video.mp4) |
| 10:23:30 | [102330-video-wayland-teleport-video.png](102330-video-wayland-teleport-video.png) |
| 10:23:39 | [102339-route-end-teleport.png](102339-route-end-teleport.png) |
| 10:23:39 | [102339-shot-wayland-route-end-teleport.png](102339-shot-wayland-route-end-teleport.png) |
| 10:23:49 | [102349-shot-wayland-world-only.png](102349-shot-wayland-world-only.png) |
| 10:23:49 | [102349-world-only.png](102349-world-only.png) |
| 10:48:59 | [104859-audio-demo-freehub.wav](104859-audio-demo-freehub.wav) |
| 11:40:00 | [114000-multiplayer-camera-camera.mp4](114000-multiplayer-camera-camera.mp4) |
| 11:40:00 | [114000-multiplayer-camera-still-30.png](114000-multiplayer-camera-still-30.png) |
| 11:40:00 | [114000-multiplayer-camera-still-50.png](114000-multiplayer-camera-still-50.png) |
| 11:40:00 | [114000-multiplayer-camera-still-65.png](114000-multiplayer-camera-still-65.png) |
| 11:40:00 | [114000-multiplayer-kiosk-kiosk.mp4](114000-multiplayer-kiosk-kiosk.mp4) |
| 11:40:00 | [114000-multiplayer-kiosk-still-12.png](114000-multiplayer-kiosk-still-12.png) |
| 11:40:00 | [114000-multiplayer-kiosk-still-25.png](114000-multiplayer-kiosk-still-25.png) |
| 11:40:00 | [114000-multiplayer-kiosk-still-40.png](114000-multiplayer-kiosk-still-40.png) |
| 12:21:49 | [122149-scenes-readme1-readme-climb-behind.png](122149-scenes-readme1-readme-climb-behind.png) |
| 12:22:02 | [122202-scenes-readme1-readme-climb-front.png](122202-scenes-readme1-readme-climb-front.png) |
| 12:22:16 | [122216-scenes-readme1-readme-malbun-high.png](122216-scenes-readme1-readme-malbun-high.png) |
| 12:22:29 | [122229-scenes-readme1-readme-hairpins.png](122229-scenes-readme1-readme-hairpins.png) |
| 19:40:52 | [194052-multiplayer-kiosk-kiosk.mp4](194052-multiplayer-kiosk-kiosk.mp4) |
| 19:40:52 | [194052-multiplayer-kiosk-still-12.png](194052-multiplayer-kiosk-still-12.png) |
| 19:40:52 | [194052-multiplayer-kiosk-still-25.png](194052-multiplayer-kiosk-still-25.png) |
| 19:40:52 | [194052-multiplayer-kiosk-still-40.png](194052-multiplayer-kiosk-still-40.png) |
| 22:04:40 | [220440-shot-wayland-bf-freiburg-freeride.png](220440-shot-wayland-bf-freiburg-freeride.png) |
| 22:04:49 | [220449-video-wayland-bf-freiburg-video.mp4](220449-video-wayland-bf-freiburg-video.mp4) |
| 22:04:49 | [220449-video-wayland-bf-freiburg-video.png](220449-video-wayland-bf-freiburg-video.png) |
| 22:05:02 | [220502-shot-wayland-bf-schauinsland-climb.png](220502-shot-wayland-bf-schauinsland-climb.png) |
| 22:05:11 | [220511-video-wayland-bf-schauinsland-climb-video.mp4](220511-video-wayland-bf-schauinsland-climb-video.mp4) |
| 22:05:11 | [220511-video-wayland-bf-schauinsland-climb-video.png](220511-video-wayland-bf-schauinsland-climb-video.png) |
| 22:05:26 | [220526-shot-wayland-bf-schauinsland-group.png](220526-shot-wayland-bf-schauinsland-group.png) |
| 22:05:41 | [220541-shot-wayland-bf-schauinsland-far-view.png](220541-shot-wayland-bf-schauinsland-far-view.png) |
| 22:05:53 | [220553-shot-wayland-bf-feldberg.png](220553-shot-wayland-bf-feldberg.png) |
| 22:06:06 | [220606-shot-wayland-bf-titisee-water.png](220606-shot-wayland-bf-titisee-water.png) |
| 22:06:15 | [220615-video-wayland-bf-titisee-video.mp4](220615-video-wayland-bf-titisee-video.mp4) |
| 22:06:15 | [220615-video-wayland-bf-titisee-video.png](220615-video-wayland-bf-titisee-video.png) |
| 22:06:28 | [220628-shot-wayland-bf-world-only.png](220628-shot-wayland-bf-world-only.png) |
| 22:16:24 | [221624-shot-wayland-bf-freiburg-freeride.png](221624-shot-wayland-bf-freiburg-freeride.png) |
| 22:16:33 | [221633-video-wayland-bf-freiburg-video.mp4](221633-video-wayland-bf-freiburg-video.mp4) |
| 22:16:33 | [221633-video-wayland-bf-freiburg-video.png](221633-video-wayland-bf-freiburg-video.png) |
| 22:16:46 | [221646-shot-wayland-bf-schauinsland-climb.png](221646-shot-wayland-bf-schauinsland-climb.png) |
| 22:16:55 | [221655-video-wayland-bf-schauinsland-climb-video.mp4](221655-video-wayland-bf-schauinsland-climb-video.mp4) |
| 22:16:55 | [221655-video-wayland-bf-schauinsland-climb-video.png](221655-video-wayland-bf-schauinsland-climb-video.png) |
| 22:17:10 | [221710-shot-wayland-bf-schauinsland-group.png](221710-shot-wayland-bf-schauinsland-group.png) |
| 22:17:25 | [221725-shot-wayland-bf-schauinsland-far-view.png](221725-shot-wayland-bf-schauinsland-far-view.png) |
| 22:17:38 | [221738-shot-wayland-bf-feldberg.png](221738-shot-wayland-bf-feldberg.png) |
| 22:17:51 | [221751-shot-wayland-bf-titisee-water.png](221751-shot-wayland-bf-titisee-water.png) |
| 22:18:00 | [221800-video-wayland-bf-titisee-video.mp4](221800-video-wayland-bf-titisee-video.mp4) |
| 22:18:00 | [221800-video-wayland-bf-titisee-video.png](221800-video-wayland-bf-titisee-video.png) |
| 22:18:13 | [221813-shot-wayland-bf-world-only.png](221813-shot-wayland-bf-world-only.png) |
| 22:19:02 | [221902-scenes-bf-try-titisee-60.png](221902-scenes-bf-try-titisee-60.png) |
| 22:19:13 | [221913-scenes-bf-try-titisee-120.png](221913-scenes-bf-try-titisee-120.png) |
| 22:19:25 | [221925-scenes-bf-try-titisee-240.png](221925-scenes-bf-try-titisee-240.png) |
| 22:19:37 | [221937-scenes-bf-try-titisee-300.png](221937-scenes-bf-try-titisee-300.png) |
| 22:19:49 | [221949-scenes-bf-try-freiburg-high.png](221949-scenes-bf-try-freiburg-high.png) |
| 22:20:01 | [222001-scenes-bf-try-freiburg-high2.png](222001-scenes-bf-try-freiburg-high2.png) |
| 22:21:11 | [222111-scenes-bf-try2-titisee-0.png](222111-scenes-bf-try2-titisee-0.png) |
| 22:21:23 | [222123-scenes-bf-try2-titisee-90.png](222123-scenes-bf-try2-titisee-90.png) |
| 22:21:35 | [222135-scenes-bf-try2-titisee-180.png](222135-scenes-bf-try2-titisee-180.png) |
| 22:21:47 | [222147-scenes-bf-try2-titisee-270.png](222147-scenes-bf-try2-titisee-270.png) |
| 22:22:17 | [222217-scenes-bf-try3-titisee-top.png](222217-scenes-bf-try3-titisee-top.png) |
| 22:22:29 | [222229-scenes-bf-try3-titisee-p70.png](222229-scenes-bf-try3-titisee-p70.png) |
| 22:22:41 | [222241-scenes-bf-try3-titisee-w.png](222241-scenes-bf-try3-titisee-w.png) |
| 22:22:52 | [222252-scenes-bf-try3-titisee-v2.png](222252-scenes-bf-try3-titisee-v2.png) |
| 22:29:50 | [222950-shot-wayland-bf-freiburg-freeride.png](222950-shot-wayland-bf-freiburg-freeride.png) |
| 22:29:59 | [222959-video-wayland-bf-freiburg-video.mp4](222959-video-wayland-bf-freiburg-video.mp4) |
| 22:30:12 | [223012-shot-wayland-bf-schauinsland-climb.png](223012-shot-wayland-bf-schauinsland-climb.png) |
| 22:30:22 | [223022-video-wayland-bf-schauinsland-climb-video.mp4](223022-video-wayland-bf-schauinsland-climb-video.mp4) |
| 22:30:22 | [223022-video-wayland-bf-schauinsland-climb-video.png](223022-video-wayland-bf-schauinsland-climb-video.png) |
| 22:30:36 | [223036-shot-wayland-bf-schauinsland-group.png](223036-shot-wayland-bf-schauinsland-group.png) |
| 22:30:52 | [223052-shot-wayland-bf-schauinsland-far-view.png](223052-shot-wayland-bf-schauinsland-far-view.png) |
| 22:31:04 | [223104-shot-wayland-bf-feldberg.png](223104-shot-wayland-bf-feldberg.png) |
| 22:31:17 | [223117-shot-wayland-bf-titisee-water.png](223117-shot-wayland-bf-titisee-water.png) |
| 22:31:27 | [223127-video-wayland-bf-titisee-video.mp4](223127-video-wayland-bf-titisee-video.mp4) |
| 22:31:27 | [223127-video-wayland-bf-titisee-video.png](223127-video-wayland-bf-titisee-video.png) |
| 22:31:40 | [223140-shot-wayland-bf-world-only.png](223140-shot-wayland-bf-world-only.png) |
| 22:52:50 | [225250-shot-wayland-shot.jpg](225250-shot-wayland-shot.jpg) |
| 22:54:53 | [225453-shot-wayland-hudoff.jpg](225453-shot-wayland-hudoff.jpg) |
| 22:55:03 | [225503-shot-wayland-shot.png](225503-shot-wayland-shot.png) |
| 22:55:12 | [225512-shot-wayland-shot720.jpg](225512-shot-wayland-shot720.jpg) |
| 22:55:28 | [225528-video-wayland-cap.jpg](225528-video-wayland-cap.jpg) |
| 22:55:28 | [225528-video-wayland-cap.mp4](225528-video-wayland-cap.mp4) |
| 22:55:36 | [225536-video-wayland-cap720.jpg](225536-video-wayland-cap720.jpg) |
| 22:55:36 | [225536-video-wayland-cap720.mp4](225536-video-wayland-cap720.mp4) |
| 22:55:47 | [225547-ts-verify-live-2026-11-12-21-55-40.mp4](225547-ts-verify-live-2026-11-12-21-55-40.mp4) |
| 22:55:53 | [225553-shot-wayland-live.jpg](225553-shot-wayland-live.jpg) |
| 22:56:04 | [225604-ts-verify-live-dev-2026-11-12-21-55-57.mp4](225604-ts-verify-live-dev-2026-11-12-21-55-57.mp4) |
| 22:56:10 | [225610-shot-wayland-live-dev.jpg](225610-shot-wayland-live-dev.jpg) |
