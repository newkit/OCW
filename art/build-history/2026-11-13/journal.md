# OCW build diary – day 3, 2026-11-13

143 commits (43 merges), +61670 / −3101 lines, 447 images, 28 videos, 0 sounds.

## Commits

| Time | Commit | |
|---|---|---|
| 05:42 | `f9ece33` (merge) | Merge origin/main; spec row of the timestamped captures renumbered to 2.30 |
| 05:42 | `7c5d706` | Spec 2.30: Phase 10 – browser app for riding (planning) |
| 05:58 | `27ae1a8` | docs/COORDINATOR-MOVE.md: plan for moving the build coordinator to marge (waiting for okay) |
| 06:00 | `134332f` | docs/COORDINATOR-MOVE.md: secrets are copied over SSH, never committed |
| 06:02 | `36aa6ab` | ocw-sky: time zones, world clock, sun position and light (spec §14 Phase 9 S1/S3) |
| 06:07 | `dee8a56` (merge) | Merge timestamped captures: ISO 8601 HUD stamp, JPEG/EXIF, 1440p, game sound in videos (spec 2.31) |
| 06:11 | `05cb864` | Build dashboard as a systemd user service (make dashboard-install / -uninstall) |
| 06:18 | `2bbab2f` | World builder: high-resolution DTMs, merged extracts, chains of neighbour packages |
| 06:19 | `f69ec68` | World clock and time machine: server clock, sky rendering, HUD, menu, console (spec §14 Phase 9 S1/S3) |
| 06:19 | `3ba0548` (merge) | Merge remote-tracking branch 'origin/main' into worktree-agent-a6af0af8d605a3782 |
| 06:20 | `4de0b1f` (merge) | Merge branch 'main' into worktree-agent-a6af0af8d605a3782 |
| 06:27 | `cbe077c` | Spec 2.32: Phase 9 S1/S3 implemented (9I); README, wasm check with ocw-sky, sky scenes |
| 06:33 | `6da4fc6` | Phase 8 T1/T2: material library and terrain / road materials (spec 2.32) |
| 06:33 | `fb2b635` (merge) | Merge remote-tracking branch 'origin/main' into worktree-agent-a52ee1f13a9414383 |
| 06:34 | `f9d8d75` (merge) | Merge main (timestamped captures, spec 2.31) into the Phase 8 T1/T2 branch |
| 06:37 | `4f8bdd9` | Build log for the art project: every remote.sh run and the build servers' state |
| 06:39 | `0d1819a` | Spec 2.32: Phase 11 – VR mode (planning) |
| 06:41 | `6e665dc` | Light model: optical depths of Bevy's earth atmosphere (consistent exposure at sunset); client clock end-to-end test |
| 06:42 | `9272b3d` | dem-import pads partial DTM tiles to whole kilometres; README: worlds and data commands |
| 06:47 | `1c6c63d` | Art project 1.3: the user's prompts in the archive and the videos |
| 06:47 | `75de9c0` | README: logo inline at the left of the heading, at the heading's height |
| 06:49 | `23477f6` (merge) | Merge origin/main (timestamped captures, spec 2.32) into the high-resolution terrain branch |
| 06:53 | `d53bd4f` | Time machine page: time and date on their own rows (one line each at 16:9); scene and spec notes on horizon blocking |
| 07:24 | `bf5a044` (merge) | Merge Phase 9 S1+S3: world clock, sun, day/night light, bike lights, time machine (spec 2.33) |
| 07:32 | `b3e1975` | Spec 2.33/2.34: high-resolution terrain and the Black Forest in three packages; fast border matching |
| 07:46 | `ede0074` | Terrain materials: rock from 75 % slope, forests hold on steep ground, forest floor with moss and canopy shade; rustfmt |
| 07:49 | `baad6c6` | Other riders' bike sounds: spatial, per-rider character, group layer (spec 2.33) |
| 07:49 | `301aba1` | Spec 2.33: others' level and crowd limit as implemented |
| 07:49 | `9c6cdb4` | docs/COORDINATOR-MOVE.md: move postponed until further notice |
| 08:05 | `d7c9fca` | deploy-test.sh installs all world packages side by side; OCW_KIOSK_WORLD/ROUTE pick the kiosk's world |
| 08:09 | `08f72b9` | Kiosk: start on the virtual trainer at 500 W, legs pedalling (kiosk.args) |
| 08:10 | `a423260` | Spec 2.32: measured frame rates, forest floor and canopy shade; test for the shaded forest colour |
| 08:11 | `dff173f` | World cleaning and bridges (spec §4.8, v2.32) |
| 08:24 | `afb0cb1` | Build dashboard: the prompt each job belongs to |
| 08:35 | `14d0250` (merge) | Merge other riders' bike sounds: spatial placement, nearest-8 voices + group layer, others bus (spec 2.34) |
| 08:35 | `8e09d70` (merge) | Merge high-resolution terrain (DGM1, swissALTI3D) and the middle/north Black Forest packages, routes, showcase, deploy support (spec 2.35, 2.36) |
| 08:36 | `4631ea2` | Kiosk show mode (virtual trainer at 500 W) for every kiosk world: scripts/kiosk-show.args |
| 08:36 | `fbdcde2` | Build dashboard: match shortened agent ids in job labels to their prompt |
| 08:51 | `f0041d3` | World cleaning: retaining-wall road edges, rim water levels, covered bridges |
| 09:18 | `c728e8c` | World cleaning: covered=yes only on bridge ways, safe junction-mouth easing |
| 10:32 | `bf68716` | Rider shadows: only one LOD model casts (fixes "two shadows of the rider"); spec 2.34 |
| 10:32 | `b21dd57` | World menu: change world, about this world, time, weather (spec 2.34) |
| 10:34 | `6923df8` (merge) | Merge origin/main into the rider-shadow fix branch |
| 10:37 | `5f5030f` | Tests: time_zone in the neighbour test packages (merge of Phase 9 and the Black Forest packages) |
| 10:38 | `8f1754f` (merge) | Merge main (high-resolution terrain, Black Forest packages) into world cleaning |
| 10:40 | `3607784` (merge) | Merge origin/main (bike sounds, high-resolution terrain, Black Forest, compile fix); rider-shadow spec row is now 2.37 |
| 10:43 | `ddc79c5` (merge) | Merge fix: one shadow per rider (only one LOD model casts) |
| 10:43 | `5f78247` | deploy-test.sh: keep the kiosk's current world unless OCW_KIOSK_WORLD says otherwise |
| 10:45 | `18579c3` (merge) | Merge main (Black Forest in three packages, other riders' sounds) into the World menu; World menu is spec 2.37, the three Black Forest packages list as one world with parts |
| 11:12 | `c46037b` | Releases increment the minor version: make release-check / release-prepare / release-publish / version-bump (spec 2.38) |
| 11:27 | `7aa1c51` (merge) | Merge origin/main (c46037b) into world cleaning; spec row becomes 2.39 |
| 11:28 | `b5ea991` (merge) | Merge origin/main (release versions, rider shadows); World menu is spec 2.39 |
| 11:28 | `20c0651` | Spec §4.8.7: world cleaning results with the high-resolution DEMs |
| 11:30 | `28ce17e` | Art project 1.4: token usage in total, per task and per feature |
| 11:37 | `33d9489` | Kiosk fixes: no marking shadows, no sun in tunnels, route files fitted onto package roads (spec 2.39) |
| 11:41 | `65b4e10` (merge) | Merge World menu: change world in the app, world info, time and weather pages (spec 2.39) |
| 11:50 | `ba0270b` | ocw-worldstore test: drop the time_zone field the merge duplicated |
| 11:59 | `6bacb87` | Tenerife world: island package, MDT05 terrain, sea from the coastline, routes, showcase (spec 4.7) |
| 11:59 | `e58e76a` (merge) | Merge remote-tracking branch 'origin/main' into worktree-agent-ae14e7e218fa01d74 |
| 12:02 | `e167977` (merge) | Merge world cleaning (lint, clean passes, terrain settle, water levels) and bridges (BRDG) (spec 2.40) |
| 12:02 | `b41ca82` | Spec: world cleaning note is v2.40 |
| 12:12 | `45892db` | Tests: bridges in the World menu's test packages (merge of World menu and world cleaning) |
| 12:12 | `fdeab4e` (merge) | Merge tunnel/route fix: route heights fitted onto the package roads, no sun in tunnels, flat overlays cast no shadows (spec 2.41) |
| 12:22 | `6b29709` | Wheels on the road: riders stand on the drawn surface, wheels rest as circles (spec 2.39) |
| 12:23 | `6247b35` (merge) | Merge remote-tracking branch 'origin/main' into worktree-agent-ae14e7e218fa01d74 |
| 12:24 | `f8df640` (merge) | Merge origin/main (fdeab4e): one road lift, routes fitted onto the strips to the millimetre (spec 2.42) |
| 12:24 | `bae102c` (merge) | Merge origin/main (spec 2.33-2.41: sky, high-res terrain, cleaning, bridges, tunnel fix) into Phase 8 T1/T2 |
| 12:26 | `b57f03c` | Release 0.2.0 |
| 12:32 | `c5649c7` | RoadStrip literals of the merged tests get their surface |
| 12:37 | `aa23dea` | Tenerife: rebuilt with the world cleaning, lint numbers, sea kept at 0 m by the water levelling (spec 2.42) |
| 12:38 | `f6e50cb` (merge) | Merge the Tenerife world: island package (MDT05), sea from the coastline, tiles near roads, routes, showcase (spec 2.42) |
| 12:38 | `57d3956` | Spec 2.43: Phase 8 decisions – no AI-generated textures (Q3), no real brands (Q11) |
| 12:43 | `1633efc` | Spec 2.44: Phase 8 decisions Q2/Q4/Q5/Q6/Q7/Q10 and the 3D building data research (Q8) |
| 12:46 | `9bc2502` | Spec 2.42: bridges and the material library |
| 12:49 | `4dedc0f` | Spec 2.45: keep MIT / Apache-2.0 (licence change declined for now) |
| 12:51 | `fc1c355` (merge) | Merge origin/main (spec 2.42-2.45: Tenerife, Phase 8 decisions, licences) into Phase 8 T1/T2; spec row 2.46 |
| 12:55 | `adc66a4` | README: Worlds section with Liechtenstein, Black Forest and Tenerife highlights (images on the build-history release) |
| 12:58 | `42f2e1c` | Wheel contact on the real roads: fitted route file test, Vaduz scenes, spec 2.42 residuals |
| 12:59 | `0ae3d50` | Regional material sets; volcanic ground for Tenerife (spec 2.46) |
| 13:12 | `bd5b4f0` (merge) | Merge Phase 8 T1/T2: CC0 material library, textured terrain and roads, volcanic set for Tenerife (spec 2.46) |
| 13:12 | `612a871` | Phase 8 T3: region look sections (PALT, ROOF, SEAS), imagery readers, calibration, fetch scripts |
| 13:13 | `57fe8b6` (merge) | Merge origin/main (Phase 8 T1/T2 materials, road surfaces, spec 2.46) into T3 |
| 13:26 | `ef8a0d8` (merge) | Merge wheels on the road: riders stand on the drawn surface, wheels rest as circles, routes fitted to 1 mm (spec 2.47) |
| 14:04 | `149a9a0` | Phase 8 T3: renderer wiring – measured palette and season tint the ground, satellite far colours by the world date, sampled roof colours |
| 14:09 | `2597a11` | Spec 2.48: Phase 11 develops on the Oculus Rift CV1, OpenXR-only for all other headsets |
| 14:24 | `c60a5c8` | wheel_contact tests: RoadStrip surface field after the materials merge |
| 14:29 | `3ae9ee5` (merge) | Merge branch 'main' into worktree-agent-ad17650ac131c55d2 |
| 14:33 | `c3f9272` | Releases increment the patch version (0.2.0 -> 0.2.1), spec 2.49 |
| 14:52 | `ddd7edd` | Phase 8 T3: seasonal near tint mostly brightness (regression: winter meadows went pink); scenes |
| 14:53 | `f866af7` | Spec 2.49: Phase 8 T3 region calibration – sources, look sections, renderer tints, measured palettes and package cost |
| 15:01 | `f5a7104` | Spec 2.50: Phase 12 planned – collisions, crashes and damage |
| 15:09 | `275aff1` | Spec 2.51: Phase 13 planned – god mode, flying through the world as a unicorn |
| 15:09 | `a0b1126` | Spec 2.51: wording of the god-mode movement keys |
| 15:10 | `af223cd` | rustfmt |
| 15:13 | `c231c9f` | Spec 2.52: god mode decisions – visible unicorn, land where you are, pegasus wings |
| 15:21 | `2c46c57` | Build and world-build profiling, faster world builds, build caching across checkouts (spec 2.49, WIP) |
| 15:21 | `a4e07fc` | Clippy: calibration and imagery readers |
| 15:30 | `d399c07` | README: iconic Teide rendering above the Worlds table; scene showcase-tf-teide-iconic |
| 15:35 | `b0eaa81` | CLAUDE.md: keep building features from the spec when all build jobs are done; ping the user with questions; project status updated |
| 15:40 | `37b98ca` | Build dashboard: describe what each job is trying to achieve instead of the prompt |
| 15:40 | `c256c08` (merge) | Merge build dashboard: describe what each job is for instead of the prompt |
| 15:48 | `9ec93da` | Spec 2.53, 2.54: Phase 4 workshop scene, hearts and part catalogue; input model for the whole UI |
| 15:48 | `bc67a63` (merge) | Merge Phase 4 workshop, hearts economy and part prices, whole-UI input model (spec 2.53, 2.54) |
| 16:02 | `dee6740` | README: remove the 'Teide' hero image – the summit is missing from the Tenerife package (tiles near roads only); the cone shown was a lower hill |
| 16:25 | `bada30a` (merge) | Merge Phase 8 T3: region calibration – orthophoto palettes, roof colours, seasonal Sentinel-2 maps (spec 2.55) |
| 16:44 | `d554051` | World build: junction-grade sweeps skip settled edges (byte-identical); seeding and voice build-dir measured and made opt-in; spec 2.49 numbers |
| 17:04 | `34cf16b` | Tenerife: the Teide's summit is in the package and drawn at distance (spec 2.55) |
| 18:21 | `752432c` | Spec 2.49: Windows build profile (full debug 3,673 s -> 2,564 s, edits 158 s -> 35 s, target 18 -> 5.6 GB); wording of the caching results |
| 18:21 | `2629b79` (merge) | Merge branch 'main' into worktree-agent-a232dd34cb256c06e |
| 18:22 | `fb814f0` | Far terrain: the region look's class colours at the world date, like the streamed tiles (Phase 8 T3) |
| 18:23 | `8a7aeec` (merge) | Merge main (bada30a: wheels, materials, T3 region calibration, dashboard; spec 2.55) into the build-profiling branch; spec row 2.49 -> 2.59, calibrate in the stage table |
| 18:26 | `4b19939` | Scenes: look-houses-* villages per region (spec §14 Phase 8 T4) |
| 18:31 | `502adbb` | far_terrain tests: the package's look (Phase 8 T3 field) |
| 18:41 | `ba8a3c6` (merge) | Merge build and world-build profiling: sccache on all machines, leaner dev profile, parallel world builder (byte-identical packages), remote-gc (spec 2.59) |
| 18:51 | `d93ec01` | Far terrain: forests tinted by their tree crowns (no trees far away: they read as bare brown ground) |
| 18:51 | `b15c1aa` (merge) | Merge Teide summit: land tiles in island packages, far terrain to 120 km with region colours, sea plane (spec 2.56) |
| 19:09 | `fadc897` | Phase 8 T4: houses – roof shapes and OSM facts (BTAG), regional styles, procedural façades, LOD |
| 19:39 | `e5c9cd3` | God mode G1/G2 (spec 2.58, §14 Phase 13): flight, the unicorn, server God state |
| 19:52 | `7bc4f63` | God mode: scenes at 14:00, no junction overlay while flying, a lighter, narrower rainbow trail |
| 20:16 | `84a626a` (merge) | Merge Phase 13 G1+G2 (+G4): god mode – flight, the pegasus-winged unicorn, server God state and GodPose (spec 2.58) |
| 20:21 | `6fb88e4` | T4: flat roofs concrete, no timber on blocks, WGSL reserved word, clippy; spec 2.57 with measurements |
| 20:28 | `715bcdb` (merge) | Merge Phase 8 T4: houses – OSM roof shapes and tags (BTAG), regional house styles, façade shader, three LODs (spec 2.57) |
| 20:32 | `b5c01ad` | imagery.py: rebuild a seasonal Sentinel-2 composite when the grid changed (more packages) instead of reusing the stale one |
| 20:49 | `03c3b31` | X11 smoke runs: lavapipe under Xvfb; restart on a presenting adapter instead of Bevy's present-mode panic |
| 20:51 | `1739cd1` (merge) | Merge X11 fix: llvmpipe for Xvfb runs (RADV cannot present without DRI3), surface guard restarts on a presentable adapter instead of panicking |
| 21:35 | `7f0923d` | README: the Teide hero image again – now the real summit (land tiles, far terrain), with the region colours |
| 21:41 | `d635380` | Phase 13 G3: god mode inspection – panel, crosshair, pins, look-ahead streaming; G1/G2/G4 gaps closed (spec 2.61) |
| 22:09 | `97ba13a` | Phase 8 T5: roads – markings, kerbs, sidewalks, crossings, wear, road edge into the terrain (spec 2.60) |
| 22:12 | `2b1dc40` (merge) | Merge Phase 13 G3: god-mode inspection – crosshair picking, flight panel, pins, look-ahead streaming, unicorn LOD, touch flight, FIT markers (spec 2.61) |
| 22:19 | `bc6364f` (merge) | Merge Phase 8 T5: roads – markings (RDRS), kerbs and sidewalks, crossings, wear, verge instead of the floating road slab (spec 2.60) |
| 22:19 | `73a2043` | inspect tests: RoadStrip dress field after the roads merge |
| 22:42 | `b213d6b` | Build dashboard: open questions from the spec, disk watcher and janitor; remote.sh snapshot at invocation and content-based sync |
| 22:45 | `fd14580` | Spec 2.63 (§11.2), README, dashboard README, CLAUDE.md: open questions panel, disk watcher and janitor, cleanup on merge, remote.sh snapshot and mtime fixes, Windows toolchain move |
| 23:07 | `cb81951` | CLAUDE.md: build machine roles – Win11Builder only for Windows-related builds, leela back with its own kiosk |
| 23:18 | `86c9a51` | Build dashboard: answer the open questions on the page (token-protected), answers.jsonl + inbox.log for the coordinator, GET /api/answers |
| 23:23 | `176498a` | Art project: build day 3 (2026-11-13) – day script, two-row end card, masked public zips |
| 23:26 | `387ac0e` | test-remote: regression for remote files touched into the future, then newer content with an older local mtime (must rebuild); spec note |
| 23:32 | `608d0c1` | CLAUDE.md: no new builds on Win11Builder until further notice; build on marge |
| 23:34 | `8743105` | remote.sh: Windows slot-time handshake opt-in (OCW_WIN_HANDSHAKE=1; stuck in its first real runs), default syncs the snapshot at once; tests: Windows wrapper assembly, simulated tar -m extraction |
| 23:36 | `0760eff` (merge) | Merge janitor and open questions: answerable questions panel in the dashboard, disk watcher, remote-gc --branch, remote.sh snapshot at invocation and content-based mtimes (spec 2.63) |
| 23:37 | `8c85763` | remote.sh: absolute config dir – the snapshot sync runs inside the snapshot directory, so the relative config/ssh paths of the main checkout broke every run |
| 23:51 | `dc86fe9` | God mode flies like a plane; + / − by what the key types; no saddle cloth (spec 2.64) |
| 23:54 | `02364cc` | CLAUDE.md: leela excluded from building until further notice (kiosk only) |

## Builds and build servers

373 build/test runs on the build machines, 71.5 h in total, 18 failed; 978 server samples; cargo compiled 1471 crates in 0.6 h, 7.0 h waiting for build slots.

| Machine | Runs | Hours | Cargo h | Crates | Waited h | Failed | Online |
|---|---|---|---|---|---|---|---|
| Win11Builder | 25 | 14.1 | 0.4 | 470 | 1.8 | 5 | 74% |
| leela | 0 | 0.0 | 0.0 | 0 | 0.0 | 0 | 0% |
| macos | 95 | 15.1 | 0.0 | 4 | 2.2 | 7 | 100% |
| marge | 253 | 42.3 | 0.1 | 997 | 3.0 | 6 | 100% |

Failed runs:

- 06:34 macos: `echo will-fail >&2; exit 3` (exit 3, log `build-log/063406-macos-68601.log`)
- 06:49 macos: `cargo run -q -p ocw-assets -- generate && cargo test -q -p ocw-assets 2>&1 | tai` (exit 128, log `build-log/064915-macos-89185.log`)
- 09:06 macos: `cargo test -p ocw-render 2>&1 | grep -E "^test result|FAILED|panicked"` (exit 1, log `build-log/090608-macos-251497.log`)
- 10:33 macos: `cargo run -q -p ocw-assets -- generate && cargo test -q -p ocw-assets > /tmp/ocw` (exit 1, log `build-log/103338-macos-330826.log`)
- 10:35 macos: `cargo fmt --all -- --check -l 2>/dev/null | sed "s|.*agent-a7e58741f5bce8d7d/||"` (exit 2, log `build-log/103507-macos-333140.log`)
- 10:45 marge: `scripts/test-release-version.sh` (exit 1, log `build-log/104554-marge-346313.log`)
- 11:29 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-13-112923 --only 'whee` (exit 1, log `build-log/112923-marge-388164.log`)
- 12:02 Win11Builder: `cmd /c "cargo test -p ocw-world -p ocw-worldgen -p ocw-worldstore > wt.log 2>&1"` (exit 101, log `build-log/120214-Win11Builder-426589.log`)
- 13:45 marge: `scripts/render-scenes.sh --out test-output/scenes/t3-li-before --only look-vaduz` (exit 1, log `build-log/134512-marge-530517.log`)
- 15:07 macos: `~/.local/bin/sccache --zero-stats >/dev/null; scripts/build-profile.sh --label m` (exit 143, log `build-log/150700-macos-608046.log`)
- 16:39 macos: `cargo clippy -q -p ocw-worldgen -p world-builder --all-targets -- -D warnings 2>` (exit 1, log `build-log/163938-macos-696644.log`)
- 14:23 Win11Builder: `powershell -ExecutionPolicy Bypass -File scripts/build-profile.ps1 -Label win-fu` (exit 101, log `build-log/142304-Win11Builder-564844.log`)
- 18:48 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-13-184856 --world /hom` (exit 1, log `build-log/184856-marge-814338.log`)
- 19:49 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-13-194921 --only 'god-` (exit 1, log `build-log/194921-marge-871012.log`)
- 19:51 marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-13-195141 --only god-u` (exit 1, log `build-log/195141-marge-873278.log`)
- 19:39 Win11Builder: `cargo test -p ocw-sim -p ocw-protocol -p ocw-server -p ocw-world -p ocw-render -` (exit 255, log `build-log/193958-Win11Builder-862230.log`)
- 20:26 Win11Builder: `cargo clippy --workspace --all-targets -- -D warnings; if ($LASTEXITCODE -eq 0) ` (exit 101, log `build-log/202657-Win11Builder-904535.log`)
- 21:06 Win11Builder: `cargo clippy --workspace --all-targets -- -D warnings; if ($LASTEXITCODE -eq 0) ` (exit 101, log `build-log/210615-Win11Builder-944908.log`)

Longest runs:

- 197 min on Win11Builder: `powershell -ExecutionPolicy Bypass -File scripts/build-profile.ps1 -Label win-fu`
- 126 min on Win11Builder: `cargo clippy -p ocw-render -p ocw-world -p ocw-worldgen --all-targets -- -D warn`
- 67 min on Win11Builder: `cargo clippy -p ocw-render -p ocw-world -p ocw-desktop --all-targets -- -D warni`
- 61 min on Win11Builder: `cargo test -q -p ocw-world -p ocw-assets 2>&1 | Select-String "test result|FAILE`
- 60 min on marge: `set -e; D=data/blackforest; scripts/fetch-imagery.sh blackforest $D/blackforest.`

## Token usage

1.97 B tokens in 5,945 model messages (output 4.5 M, cache reads 1.94 B, cache writes 29.8 M, uncached input 12 k). Full breakdown: [tokens.json](tokens.json).

| Feature | Tokens | Output | Messages |
|---|---|---|---|
| Worlds & terrain | 547.9 M | 1.1 M | 1,449 |
| Materials & look | 403.2 M | 770 k | 1,228 |
| Planning & spec | 381.3 M | 1.3 M | 1,281 |
| Sky, time & weather | 176.9 M | 445 k | 515 |
| UI & menus | 170.9 M | 387 k | 496 |
| Other | 106.0 M | 250 k | 427 |
| Multiplayer & server | 57.6 M | 67 k | 81 |
| Build, release & infra | 55.7 M | 87 k | 168 |
| Art project | 37.3 M | 77 k | 140 |
| Audio & voice | 24.6 M | 102 k | 120 |
| Riders & bikes | 10.6 M | 46 k | 40 |

| Task | Feature | Tokens | Output |
|---|---|---|---|
| agent: Phase 8 T1+T2: materials and terrain | Materials & look | 193.8 M | 324 k |
| agent: Profile and speed up builds + world builds | Worlds & terrain | 175.3 M | 292 k |
| agent: Clean world creation + bridges | Worlds & terrain | 135.1 M | 309 k |
| agent: Northern Black Forest + best terrain | Worlds & terrain | 119.1 M | 249 k |
| agent: Phase 8 T3: regional calibration + seasons | Sky, time & weather | 106.7 M | 247 k |
| agent: World menu: select world, time, weather info | UI & menus | 97.7 M | 204 k |
| agent: Fix wheels sinking into road | Materials & look | 94.6 M | 183 k |
| agent: Phase 8 T4: houses | Planning & spec | 91.9 M | 250 k |
| agent: Phase 13 G1+G2: god mode | Planning & spec | 88.5 M | 235 k |
| agent: Create the Tenerife world | Worlds & terrain | 87.4 M | 178 k |
| agent: Phase 9 S1+S3: sun, day/night, time machine | Sky, time & weather | 70.2 M | 198 k |
| agent: Phase 8 T5: roads | Planning & spec | 69.2 M | 249 k |
| agent: Fix tunnel artifacts, rider sinking, marking shadows | Materials & look | 58.6 M | 133 k |
| agent: Janitor + open questions panel | Planning & spec | 51.9 M | 200 k |
| agent: Phase 13 G3 + god-mode gaps | Planning & spec | 47.2 M | 152 k |
| agent: Fix missing Teide summit tiles | Other | 45.6 M | 119 k |
| coordination: use w11buildserver and macos buildserver, too | Multiplayer & server | 33.1 M | 24 k |
| coordination: on macos | Build, release & infra | 31.4 M | 37 k |
| coordination: add some profiling in for the building and see if the build process could be acc… | Worlds & terrain | 25.8 M | 14 k |
| agent: God mode: plane-like controls | Planning & spec | 21.7 M | 87 k |
| coordination: hence stick to texture that are available under the current license. continue in… | Materials & look | 20.5 M | 16 k |
| agent: Audible sounds of other riders | Audio & voice | 20.2 M | 89 k |
| agent: Fix double rider shadow | Materials & look | 19.5 M | 66 k |
| coordination: build dashboard: add the prompt (short version) that belongs to the job to the s… | UI & menus | 16.8 M | 19 k |
| coordination: add all the open build questions into the build dashboard. Yes implement the jan… | UI & menus | 11.8 M | 14 k |
| agent: Art project: day 3 diary | Art project | 11.2 M | 46 k |
| coordination: github readme: make the OCW logo in the size of the Open Cycling World heading a… | Art project | 9.6 M | 7 k |
| agent: Dashboard: describe build goal | UI & menus | 9.1 M | 69 k |
| coordination: check mac build server for build artifacts that can be deleted | Multiplayer & server | 9.0 M | 17 k |
| coordination: i still do not see the teide in the images. do a carefull planning of where the … | UI & menus | 8.8 M | 20 k |
| agent: Fix X11 rendering on marge | Other | 8.3 M | 47 k |
| agent: ISO timestamps in captures | UI & menus | 7.9 M | 12 k |
| coordination: i had to reinitialize the drive B: on w11builder. setup for building there and d… | Multiplayer & server | 7.1 M | 6 k |
| coordination: remove the pink/yellow square on the back of the unicorn. | Other | 6.9 M | 6 k |
| agent: Price research: contact and frame parts | Other | 6.8 M | 38 k |
| coordination: I hit my usage limit while you were working, but it has reset now. Please contin… | Other | 6.3 M | 4 k |
| coordination: when running in kiosk mode, always start with 500W. have the rider also move it'… | UI & menus | 5.7 M | 5 k |
| agent: Price research: drivetrain, wheels, electronics | Materials & look | 5.6 M | 32 k |
| coordination: whats the hotkey of god mode? | Other | 5.6 M | 5 k |
| coordination: also document the build status, the processes, buildtime, errors, status of buil… | Art project | 5.4 M | 8 k |

## Prompts

94 prompts (full text: [prompts.md](prompts.md)).

| Time | Prompt |
|---|---|
| 05:41 | where did you sent me the mention image? i dont see it here |
| 05:41 | Start implementing the next phases. Give me a quick summary of the open questions. |
| 05:56 | can you prepare moving from deb to marge as the build coordinator? check if user cli is availble for you on marge. wait with the actual move |
| 06:00 | Make sure you don't add config and ssh keys to git. copy them. |
| 06:03 | done, i logged in user and gh on marge |
| 06:04 | done, gh is logged in as user now |
| 06:10 | can you startub the build dashboard on deb? create systemctl startup scripts for it and enable them |
| 06:19 | also document the build status, the processes, buildtime, errors, status of build servers for the art project. |
| 06:25 | add planning for a VR mode where oculus rift and other vendor/models are supported. |
| 06:31 | bg task: plan for a cleaner wold creation. right now, there are often jumps in the road or rivers or other objects comeing from OSM data. do |
| 06:33 | bg task: there seems to be a small deviation: the wheels of the bike always seen to be like 1cm inside of the road. fix that. |
| 06:36 | adding to the cleaning world planning task: i've seen sometimes the road being like 1m above the ground. I see there two reasons for that. e |
| 06:42 | audio: make the noise from other rides also audible. art-project: include the prompts i do also into the logfile and later on in the video. |
| 06:46 | github readme: make the OCW logo in the size of the Open Cycling World heading and make it aligned in the heading left side. |
| 07:44 | when the black forest world is ready, make it also used in the kiosk mode. add a world entry to the game menu where a user can select the wo |
| 07:49 | postpone moving the coordinator until further notice. i noticed that there is a bug with the shadow since the sun is there ingame. i some ti |
| 08:09 | when running in kiosk mode, always start with 500W. have the rider also move it's legs eventhough it's fake |
| 08:15 | i dont know what happend but some change in the near past has intruduced weird graphic errors in the tunnel and the road before. in the tunn |
| 08:21 | build dashboard: add the prompt (short version) that belongs to the job to the status as well. restart dashboard after implementing this. |
| 08:49 | create a world for tenariffa |
| 10:31 | I hit my usage limit while you were working, but it has reset now. Please continue from where you left off. |
| 10:39 | i had to reinitialize the drive B: on w11builder. setup for building there and do a quick check. |
| 10:43 | implement a logic that everytime we release on Github, the minor version number is incremented. |
| 10:50 | instead of seperate bump and release scripts, make this part of the Makefile |
| 10:52 | /login |
| 10:54 | /login |
| 10:57 | /login |
| 10:58 | /login |
| 11:04 | check mac build server for build artifacts that can be deleted |
| 11:15 | what are the six finished copies on mac? |
| 11:15 | /login |
| 11:16 | what are the six finished copies on mac? |
| 11:26 | can you continue the OCW session on deb? |
| 11:27 | yes, delete the six on the mac |
| 11:28 | for the art project, keep track of the token usage, in total and per task and feature. |
| 11:29 | use w11buildserver and macos buildserver, too |
| 12:26 | release the new updated world files. illustrate the world in GH Readme with a couple of screenshots of the highlights |
| 12:30 | are there open question regarding the textures? |
| 12:38 | both questions: yes |
| 12:42 | * Q2 – Commercial release planned? This decides how strict we are with grey-area sources, and when a lawyer reviews the licences. -> NO * Q4 |
| 12:43 | are there issues on macos or why is nothing building there? |
| 12:47 | what would change if we switch our license from MIT/Apache to CC BY-SA? |
| 12:49 | avoid changing the license for now |
| 12:50 | hence stick to texture that are available under the current license. continue integrating texture feature |
| 13:05 | add some profiling in for the building and see if the build process could be accelerated. also for building the world. |
| 13:53 | investigate the context what could be compacted and what should be kept |
| 13:54 | /compact |
| 13:59 | planning: regarding my VR question earlier: I have an oculus rift, first generation. hence focus on this for development. use the open frame |
| 14:19 | Try again |
| 14:21 | put also jobs on w11builder |
| 14:23 | what was the answer to my build performance question? can you propose a solution that efficiently uses caching between different build agent |
| 14:28 | yes, implement the build caching as suggested. |
| 14:30 | version bump: instead of increasing the middle number, e.g. 0.1.0 -> 0.2.0 increase only the last number, e.g. 0.1.0 -> 0.1.1 |
| 14:32 | create an iconic rendering for the readme world section of the volcan teide on tenerife. there is a road around the caldera. make an image t |
| 14:49 | how many tokens have been used up for this OCW so far? |
| 14:51 | explain: why have you queued this job "/.local/bin/sccache --zero-stats >/dev/null; scripts/build-profile.sh --label macos-sccache-warm --co |
| 14:58 | planning (add as new phase to spec): design a crash and collision model with world objects and other riders. make it globally configurable ( |
| 15:00 | in the build dashboard: rather than showing the prompt describe what the actual build is trying to archive. |
| 15:08 | planning (and add to spec): implement a god mode, that allows flying through the world (e.g. to inspect the map). the rider should convert i |
| 15:10 | in which build phase are we in? what would come next? |
| 15:12 | Regarding your god mode questions:   * Should other riders see your unicorn? -> yes * Should leaving always bring you back to your spot on t |
| 15:29 | planning for bike and character editor of phase 4: refine specifications. When selecting this menu point, the screen should change to a rend |
| 15:29 | does windows also have sccache? |
| 15:34 | whenever all build jobs are done, start implementing features from specification that are not yet implemented and where you have all informa |
| 15:35 | how to start manually on leela the app with black forest world? |
| 15:36 | the tener |
| 15:42 | i still do not see the teide in the images. do a carefull planning of where the riders have to be and move the camera so that it sees the ti |
| 18:19 | is it easy to move the two waiting build jobs to marge (fromw11builder) |
| 18:20 | Okay. And why aren't there any build jobs on Marge? Is everything implemented, or what's the reason? |
| 18:25 | Would this really build artifacts that are used for caching, or is it really build data we don't need? |
| 18:27 | Yes, delete the unnecessary build files, and think about a mechanism how to clean up the builds from unnecessary data, and also how to make  |
| 18:28 | situation. Text space. Tag builder. |
| 18:30 | on macos |
| 22:08 | add all the open build questions into the build dashboard. Yes implement the janitor as sugested. |
| 22:48 | buildserver leela is back online. build a kiosmode for leela like you did for marge. |
| 22:50 | yes, the key is correct user@leela:~$ ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub 256 SHA256:AHd2iyMgxxfIwz4xo6g4BdyJRNl73ulHT2C0dV9DIv |
| 22:51 | show tenrife teide route on leela |
| 22:54 | where is the artwork for the art project? |
| 22:55 | check connectivity with leela |
| 22:55 | but where are the files, show me the location and name of the files |
| 23:01 | where is the token usage logged? |
| 23:03 | restart the build dashboard. whats the command you use? |
| 23:06 | rewrite user.md the buildserver section: W11Builder should only be used for Windows related builds. |
| 23:13 | whats the status of adding the open question to the build dashboard? make it interactive, so that i can answer them there. |
| 23:15 | whats the hotkey of god mode? |
| 23:30 | god mode: make the control like flying a plane. plus key should make you go up in the air, minus key makes you go down |
| 23:32 | stop queueing new builds on w11builder. use marge for building. |
| 23:34 | remove the pink/yellow square on the back of the unicorn. |
| 23:44 | answering some questions: Phase 4 – Avatar and Bicycle Editor · 5 open · §14 4L Q1When changes apply at once when leaving the workshop (in f |
| 23:45 | where to get that config/token |
| 23:52 | continue paused (bg) tasks on marge |
| 23:54 | exclude leela from builting. |
| 23:55 | build-dashboard: make a button to enable/disable machines for building. make the amount of slots configurable. |
| 23:56 | make all the routes available on leela |

## Media

| Time | File |
|---|---|
| 06:33:56 | [063356-scenes-t2-before-malbun-arrival.png](063356-scenes-t2-before-malbun-arrival.png) |
| 06:34:05 | [063405-scenes-t2-before-rhine-water.png](063405-scenes-t2-before-rhine-water.png) |
| 06:34:19 | [063419-scenes-t2-before-far-view.png](063419-scenes-t2-before-far-view.png) |
| 06:34:30 | [063430-scenes-t2-before-mat-meadow-road.png](063430-scenes-t2-before-mat-meadow-road.png) |
| 06:34:41 | [063441-scenes-t2-before-mat-forest.png](063441-scenes-t2-before-mat-forest.png) |
| 06:34:52 | [063452-scenes-t2-before-mat-cliff.png](063452-scenes-t2-before-mat-cliff.png) |
| 06:35:03 | [063503-scenes-t2-before-mat-malbun.png](063503-scenes-t2-before-mat-malbun.png) |
| 06:35:13 | [063513-scenes-t2-before-mat-rhine-gravel.png](063513-scenes-t2-before-mat-rhine-gravel.png) |
| 06:35:23 | [063523-scenes-t2-before-mat-video.png](063523-scenes-t2-before-mat-video.png) |
| 06:35:25 | [063525-scenes-t2-before-mat-video.mp4](063525-scenes-t2-before-mat-video.mp4) |
| 06:35:38 | [063538-scenes-t2-before-bf-mat-forest.png](063538-scenes-t2-before-bf-mat-forest.png) |
| 06:35:52 | [063552-scenes-t2-before-bf-mat-meadow.png](063552-scenes-t2-before-bf-mat-meadow.png) |
| 06:36:16 | [063616-scenes-before-sheet.png](063616-scenes-before-sheet.png) |
| 06:37:39 | [063739-shot-wayland-sky-noon.jpg](063739-shot-wayland-sky-noon.jpg) |
| 06:37:50 | [063750-shot-wayland-sky-sunset.jpg](063750-shot-wayland-sky-sunset.jpg) |
| 06:38:01 | [063801-shot-wayland-sky-night-lights.jpg](063801-shot-wayland-sky-night-lights.jpg) |
| 06:42:03 | [064203-shot-wayland-sky-sunrise.jpg](064203-shot-wayland-sky-sunrise.jpg) |
| 06:42:08 | [064208-agent-scenes-t2-try1-mat-meadow-road.jpg](064208-agent-scenes-t2-try1-mat-meadow-road.jpg) |
| 06:42:14 | [064214-shot-wayland-sky-noon.jpg](064214-shot-wayland-sky-noon.jpg) |
| 06:42:24 | [064224-shot-wayland-sky-golden-hour.jpg](064224-shot-wayland-sky-golden-hour.jpg) |
| 06:42:35 | [064235-shot-wayland-sky-sunset.jpg](064235-shot-wayland-sky-sunset.jpg) |
| 06:42:46 | [064246-shot-wayland-sky-sunset-ridge.jpg](064246-shot-wayland-sky-sunset-ridge.jpg) |
| 06:42:56 | [064256-shot-wayland-sky-blue-hour.jpg](064256-shot-wayland-sky-blue-hour.jpg) |
| 06:43:08 | [064308-shot-wayland-sky-night-lights.jpg](064308-shot-wayland-sky-night-lights.jpg) |
| 06:43:16 | [064316-shot-wayland-menu-time-machine.jpg](064316-shot-wayland-menu-time-machine.jpg) |
| 06:43:48 | [064348-video-wayland-sky-sunset-timelapse.jpg](064348-video-wayland-sky-sunset-timelapse.jpg) |
| 06:43:48 | [064348-video-wayland-sky-sunset-timelapse.mp4](064348-video-wayland-sky-sunset-timelapse.mp4) |
| 06:53:49 | [065349-agent-fps-amd-off.png](065349-agent-fps-amd-off.png) |
| 06:54:31 | [065431-agent-fps-amd-on.png](065431-agent-fps-amd-on.png) |
| 06:55:13 | [065513-agent-fps-lvp-off.png](065513-agent-fps-lvp-off.png) |
| 06:55:56 | [065556-agent-fps-lvp-on.png](065556-agent-fps-lvp-on.png) |
| 07:08:52 | [070852-shot-wayland-menu-time-machine.jpg](070852-shot-wayland-menu-time-machine.jpg) |
| 07:13:35 | [071335-agent-fps-amd-off-1.png](071335-agent-fps-amd-off-1.png) |
| 07:14:16 | [071416-agent-fps-amd-on-1.png](071416-agent-fps-amd-on-1.png) |
| 07:14:21 | [071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-flat-contact.jpg](071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-flat-contact.jpg) |
| 07:14:21 | [071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-mtb-steep-contact.jpg](071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-mtb-steep-contact.jpg) |
| 07:14:21 | [071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-steep-contact.jpg](071421-agent-scenes-wheel-contact-vaduz-before-wheel-contact-steep-contact.jpg) |
| 07:14:57 | [071457-agent-fps-amd-off-2.png](071457-agent-fps-amd-off-2.png) |
| 07:15:39 | [071539-agent-fps-amd-on-2.png](071539-agent-fps-amd-on-2.png) |
| 07:15:52 | [071552-agent-fps-cmp.png](071552-agent-fps-cmp.png) |
| 07:18:45 | [071845-agent-scenes-t2-after-malbun-arrival.jpg](071845-agent-scenes-t2-after-malbun-arrival.jpg) |
| 07:18:53 | [071853-agent-scenes-t2-after-rhine-water.jpg](071853-agent-scenes-t2-after-rhine-water.jpg) |
| 07:19:06 | [071906-agent-scenes-t2-after-far-view.jpg](071906-agent-scenes-t2-after-far-view.jpg) |
| 07:19:17 | [071917-agent-scenes-t2-after-mat-meadow-road.jpg](071917-agent-scenes-t2-after-mat-meadow-road.jpg) |
| 07:19:27 | [071927-agent-scenes-t2-after-mat-forest.jpg](071927-agent-scenes-t2-after-mat-forest.jpg) |
| 07:19:37 | [071937-agent-scenes-t2-after-mat-cliff.jpg](071937-agent-scenes-t2-after-mat-cliff.jpg) |
| 07:19:48 | [071948-agent-scenes-t2-after-mat-malbun.jpg](071948-agent-scenes-t2-after-mat-malbun.jpg) |
| 07:19:57 | [071957-agent-scenes-t2-after-mat-rhine-gravel.jpg](071957-agent-scenes-t2-after-mat-rhine-gravel.jpg) |
| 07:20:06 | [072006-agent-scenes-t2-after-mat-video.jpg](072006-agent-scenes-t2-after-mat-video.jpg) |
| 07:20:16 | [072016-agent-scenes-t2-after-mat-video.mp4](072016-agent-scenes-t2-after-mat-video.mp4) |
| 07:20:28 | [072028-agent-scenes-t2-after-bf-mat-forest.jpg](072028-agent-scenes-t2-after-bf-mat-forest.jpg) |
| 07:20:41 | [072041-agent-scenes-t2-after-bf-mat-meadow.jpg](072041-agent-scenes-t2-after-bf-mat-meadow.jpg) |
| 07:20:56 | [072056-agent-scenes-after-sheet.png](072056-agent-scenes-after-sheet.png) |
| 07:24:14 | [072414-agent-scenes-t2-after2-malbun-arrival.jpg](072414-agent-scenes-t2-after2-malbun-arrival.jpg) |
| 07:24:23 | [072423-agent-scenes-t2-after2-rhine-water.jpg](072423-agent-scenes-t2-after2-rhine-water.jpg) |
| 07:24:36 | [072436-agent-scenes-t2-after2-far-view.jpg](072436-agent-scenes-t2-after2-far-view.jpg) |
| 07:24:47 | [072447-agent-scenes-t2-after2-mat-meadow-road.jpg](072447-agent-scenes-t2-after2-mat-meadow-road.jpg) |
| 07:24:57 | [072457-agent-scenes-t2-after2-mat-forest.jpg](072457-agent-scenes-t2-after2-mat-forest.jpg) |
| 07:25:07 | [072507-agent-scenes-t2-after2-mat-cliff.jpg](072507-agent-scenes-t2-after2-mat-cliff.jpg) |
| 07:25:18 | [072518-agent-scenes-t2-after2-mat-malbun.jpg](072518-agent-scenes-t2-after2-mat-malbun.jpg) |
| 07:25:27 | [072527-agent-scenes-t2-after2-mat-rhine-gravel.jpg](072527-agent-scenes-t2-after2-mat-rhine-gravel.jpg) |
| 07:25:36 | [072536-agent-scenes-t2-after2-mat-video.jpg](072536-agent-scenes-t2-after2-mat-video.jpg) |
| 07:25:49 | [072549-agent-scenes-t2-after2-mat-video.mp4](072549-agent-scenes-t2-after2-mat-video.mp4) |
| 07:26:01 | [072601-agent-scenes-t2-after2-bf-mat-forest.jpg](072601-agent-scenes-t2-after2-bf-mat-forest.jpg) |
| 07:26:14 | [072614-agent-scenes-t2-after2-bf-mat-meadow.jpg](072614-agent-scenes-t2-after2-bf-mat-meadow.jpg) |
| 07:28:38 | [072838-agent-scenes-after2-sheet.png](072838-agent-scenes-after2-sheet.png) |
| 07:33:23 | [073323-shot-wayland-wheel-contact-flat.jpg](073323-shot-wayland-wheel-contact-flat.jpg) |
| 07:33:31 | [073331-shot-wayland-wheel-contact-steep.jpg](073331-shot-wayland-wheel-contact-steep.jpg) |
| 07:33:39 | [073339-shot-wayland-wheel-contact-lean.jpg](073339-shot-wayland-wheel-contact-lean.jpg) |
| 07:33:48 | [073348-shot-wayland-wheel-contact-mtb-steep.jpg](073348-shot-wayland-wheel-contact-mtb-steep.jpg) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-freiburg-muensterplatz-dgm1.jpg](073622-agent-scenes-terrain-before-after-2026-11-13-freiburg-muensterplatz-dgm1.jpg) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-freiburg-muensterplatz-srtm.jpg](073622-agent-scenes-terrain-before-after-2026-11-13-freiburg-muensterplatz-srtm.jpg) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-srtm.png](073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-srtm.png) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-swissalti3d-5m-grid.png](073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-swissalti3d-5m-grid.png) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-swissalti3d.png](073622-agent-scenes-terrain-before-after-2026-11-13-malbun-road-km11-swissalti3d.png) |
| 07:36:22 | [073622-agent-scenes-terrain-before-after-2026-11-13-mummelsee-service-road-dgm1.jpg](073622-agent-scenes-terrain-before-after-2026-11-13-mummelsee-service-road-dgm1.jpg) |
| 07:47:34 | [074734-shot-wayland-showcase-mummelsee.jpg](074734-shot-wayland-showcase-mummelsee.jpg) |
| 07:47:56 | [074756-video-wayland-showcase-mummelsee-video.jpg](074756-video-wayland-showcase-mummelsee-video.jpg) |
| 07:47:56 | [074756-video-wayland-showcase-mummelsee-video.mp4](074756-video-wayland-showcase-mummelsee-video.mp4) |
| 07:48:10 | [074810-shot-wayland-showcase-hornisgrinde.jpg](074810-shot-wayland-showcase-hornisgrinde.jpg) |
| 07:48:25 | [074825-shot-wayland-showcase-hochstrasse.jpg](074825-shot-wayland-showcase-hochstrasse.jpg) |
| 07:48:47 | [074847-video-wayland-showcase-hochstrasse-video.jpg](074847-video-wayland-showcase-hochstrasse-video.jpg) |
| 07:48:47 | [074847-video-wayland-showcase-hochstrasse-video.mp4](074847-video-wayland-showcase-hochstrasse-video.mp4) |
| 07:49:02 | [074902-shot-wayland-showcase-badenbaden.jpg](074902-shot-wayland-showcase-badenbaden.jpg) |
| 07:49:24 | [074924-video-wayland-showcase-badenbaden-video.jpg](074924-video-wayland-showcase-badenbaden-video.jpg) |
| 07:49:24 | [074924-video-wayland-showcase-badenbaden-video.mp4](074924-video-wayland-showcase-badenbaden-video.mp4) |
| 07:49:39 | [074939-shot-wayland-showcase-sasbachwalden.jpg](074939-shot-wayland-showcase-sasbachwalden.jpg) |
| 07:49:54 | [074954-shot-wayland-showcase-feldberg.jpg](074954-shot-wayland-showcase-feldberg.jpg) |
| 07:50:19 | [075019-video-wayland-showcase-feldberg-video.jpg](075019-video-wayland-showcase-feldberg-video.jpg) |
| 07:50:19 | [075019-video-wayland-showcase-feldberg-video.mp4](075019-video-wayland-showcase-feldberg-video.mp4) |
| 07:50:34 | [075034-shot-wayland-showcase-titisee.jpg](075034-shot-wayland-showcase-titisee.jpg) |
| 07:50:58 | [075058-video-wayland-showcase-titisee-video.jpg](075058-video-wayland-showcase-titisee-video.jpg) |
| 07:50:58 | [075058-video-wayland-showcase-titisee-video.mp4](075058-video-wayland-showcase-titisee-video.mp4) |
| 07:51:13 | [075113-shot-wayland-showcase-schauinsland.jpg](075113-shot-wayland-showcase-schauinsland.jpg) |
| 07:51:28 | [075128-shot-wayland-showcase-freiburg.jpg](075128-shot-wayland-showcase-freiburg.jpg) |
| 07:51:44 | [075144-shot-wayland-showcase-schluchsee.jpg](075144-shot-wayland-showcase-schluchsee.jpg) |
| 07:51:59 | [075159-shot-wayland-showcase-belchen.jpg](075159-shot-wayland-showcase-belchen.jpg) |
| 07:52:12 | [075212-shot-wayland-showcase-malbun-hairpins.jpg](075212-shot-wayland-showcase-malbun-hairpins.jpg) |
| 07:52:27 | [075227-shot-wayland-showcase-feldberg-pass.jpg](075227-shot-wayland-showcase-feldberg-pass.jpg) |
| 07:52:42 | [075242-shot-wayland-showcase-schauinsland.jpg](075242-shot-wayland-showcase-schauinsland.jpg) |
| 07:52:56 | [075256-shot-wayland-showcase-malbun-hairpins.jpg](075256-shot-wayland-showcase-malbun-hairpins.jpg) |
| 07:53:11 | [075311-shot-wayland-showcase-feldberg-pass.jpg](075311-shot-wayland-showcase-feldberg-pass.jpg) |
| 08:02:37 | [080237-fps-amd-off-1.png](080237-fps-amd-off-1.png) |
| 08:03:18 | [080318-fps-amd-on-1.png](080318-fps-amd-on-1.png) |
| 08:04:00 | [080400-fps-amd-off-2.png](080400-fps-amd-off-2.png) |
| 08:04:41 | [080441-fps-amd-on-2.png](080441-fps-amd-on-2.png) |
| 08:05:23 | [080523-agent-fps-lvp-off-1.png](080523-agent-fps-lvp-off-1.png) |
| 08:06:08 | [080608-agent-fps-lvp-on-1.png](080608-agent-fps-lvp-on-1.png) |
| 08:06:53 | [080653-agent-fps-lvp-off-2.png](080653-agent-fps-lvp-off-2.png) |
| 08:07:38 | [080738-agent-fps-lvp-on-2.png](080738-agent-fps-lvp-on-2.png) |
| 08:07:55 | [080755-agent-scenes-t2-final-malbun-arrival.jpg](080755-agent-scenes-t2-final-malbun-arrival.jpg) |
| 08:08:04 | [080804-agent-scenes-t2-final-rhine-water.jpg](080804-agent-scenes-t2-final-rhine-water.jpg) |
| 08:08:17 | [080817-agent-scenes-t2-final-far-view.jpg](080817-agent-scenes-t2-final-far-view.jpg) |
| 08:08:21 | [080821-shot-wayland-before-side-0800.png](080821-shot-wayland-before-side-0800.png) |
| 08:08:28 | [080828-agent-scenes-t2-final-mat-meadow-road.jpg](080828-agent-scenes-t2-final-mat-meadow-road.jpg) |
| 08:08:32 | [080832-shot-wayland-before-chase-0800.png](080832-shot-wayland-before-chase-0800.png) |
| 08:08:38 | [080838-agent-scenes-t2-final-mat-forest.jpg](080838-agent-scenes-t2-final-mat-forest.jpg) |
| 08:08:44 | [080844-shot-wayland-before-side-1200.png](080844-shot-wayland-before-side-1200.png) |
| 08:08:48 | [080848-agent-scenes-t2-final-mat-cliff.jpg](080848-agent-scenes-t2-final-mat-cliff.jpg) |
| 08:08:55 | [080855-shot-wayland-before-chase-1200.png](080855-shot-wayland-before-chase-1200.png) |
| 08:08:59 | [080859-agent-scenes-t2-final-mat-malbun.jpg](080859-agent-scenes-t2-final-mat-malbun.jpg) |
| 08:09:06 | [080906-shot-wayland-before-side-1730.png](080906-shot-wayland-before-side-1730.png) |
| 08:09:08 | [080908-agent-scenes-t2-final-mat-rhine-gravel.jpg](080908-agent-scenes-t2-final-mat-rhine-gravel.jpg) |
| 08:09:17 | [080917-agent-scenes-t2-final-mat-video.jpg](080917-agent-scenes-t2-final-mat-video.jpg) |
| 08:09:18 | [080918-shot-wayland-before-chase-1730.png](080918-shot-wayland-before-chase-1730.png) |
| 08:09:28 | [080928-agent-scenes-t2-final-mat-video.mp4](080928-agent-scenes-t2-final-mat-video.mp4) |
| 08:09:29 | [080929-shot-wayland-before-side-1930.png](080929-shot-wayland-before-side-1930.png) |
| 08:09:41 | [080941-agent-scenes-t2-final-bf-mat-forest.jpg](080941-agent-scenes-t2-final-bf-mat-forest.jpg) |
| 08:09:41 | [080941-shot-wayland-before-chase-1930.png](080941-shot-wayland-before-chase-1930.png) |
| 08:09:52 | [080952-shot-wayland-before-side-2045.png](080952-shot-wayland-before-side-2045.png) |
| 08:09:54 | [080954-agent-scenes-t2-final-bf-mat-meadow.jpg](080954-agent-scenes-t2-final-bf-mat-meadow.jpg) |
| 08:10:03 | [081003-shot-wayland-before-chase-2045.png](081003-shot-wayland-before-chase-2045.png) |
| 08:10:13 | [081013-agent-scenes-final-sheet.png](081013-agent-scenes-final-sheet.png) |
| 08:10:24 | [081024-agent-scenes-t2-compare-far-view.jpg](081024-agent-scenes-t2-compare-far-view.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-mat-cliff.jpg](081024-agent-scenes-t2-compare-mat-cliff.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-mat-forest.jpg](081024-agent-scenes-t2-compare-mat-forest.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-mat-malbun.jpg](081024-agent-scenes-t2-compare-mat-malbun.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-mat-meadow-road.jpg](081024-agent-scenes-t2-compare-mat-meadow-road.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-mat-rhine-gravel.jpg](081024-agent-scenes-t2-compare-mat-rhine-gravel.jpg) |
| 08:10:24 | [081024-agent-scenes-t2-compare-rhine-water.jpg](081024-agent-scenes-t2-compare-rhine-water.jpg) |
| 08:10:25 | [081025-agent-scenes-t2-compare-bf-mat-forest.jpg](081025-agent-scenes-t2-compare-bf-mat-forest.jpg) |
| 08:10:25 | [081025-agent-scenes-t2-compare-bf-mat-meadow.jpg](081025-agent-scenes-t2-compare-bf-mat-meadow.jpg) |
| 08:10:25 | [081025-agent-scenes-t2-compare-malbun-arrival.jpg](081025-agent-scenes-t2-compare-malbun-arrival.jpg) |
| 08:18:39 | [081839-shot-wayland-diag-top-0800.png](081839-shot-wayland-diag-top-0800.png) |
| 08:19:25 | [081925-agent-shadow-before-crop-chase-0800.png](081925-agent-shadow-before-crop-chase-0800.png) |
| 08:19:25 | [081925-agent-shadow-before-crop-side-0800.png](081925-agent-shadow-before-crop-side-0800.png) |
| 08:21:58 | [082158-video-wayland-ocw-others-spectrogram.png](082158-video-wayland-ocw-others-spectrogram.png) |
| 08:21:58 | [082158-video-wayland-ocw-others.jpg](082158-video-wayland-ocw-others.jpg) |
| 08:21:58 | [082158-video-wayland-ocw-others.mp4](082158-video-wayland-ocw-others.mp4) |
| 08:23:17 | [082317-shot-wayland-before-orbit-0930-30.png](082317-shot-wayland-before-orbit-0930-30.png) |
| 08:23:28 | [082328-shot-wayland-before-orbit-0930-40.png](082328-shot-wayland-before-orbit-0930-40.png) |
| 08:23:40 | [082340-shot-wayland-before-orbit-1200-30.png](082340-shot-wayland-before-orbit-1200-30.png) |
| 08:23:51 | [082351-shot-wayland-before-orbit-1200-40.png](082351-shot-wayland-before-orbit-1200-40.png) |
| 08:24:00 | [082400-agent-shadow-lod-crop-before-orbit-0930-30.png](082400-agent-shadow-lod-crop-before-orbit-0930-30.png) |
| 08:24:00 | [082400-agent-shadow-lod-crop-before-orbit-0930-40.png](082400-agent-shadow-lod-crop-before-orbit-0930-40.png) |
| 08:24:00 | [082400-agent-shadow-lod-crop-before-orbit-1200-30.png](082400-agent-shadow-lod-crop-before-orbit-1200-30.png) |
| 08:24:00 | [082400-agent-shadow-lod-crop-before-orbit-1200-40.png](082400-agent-shadow-lod-crop-before-orbit-1200-40.png) |
| 08:24:15 | [082415-agent-shadow-lod-zoom-1200-40.png](082415-agent-shadow-lod-zoom-1200-40.png) |
| 08:24:16 | [082416-agent-shadow-lod-zoom-1200-30.png](082416-agent-shadow-lod-zoom-1200-30.png) |
| 08:31:01 | [083101-shot-wayland-kiosk-check.jpg](083101-shot-wayland-kiosk-check.jpg) |
| 08:46:22 | [084622-agent-tunnel-repro-main-08f72b9-k-before-0820.jpg](084622-agent-tunnel-repro-main-08f72b9-k-before-0820.jpg) |
| 08:46:33 | [084633-agent-tunnel-repro-main-08f72b9-k-entry-0820.jpg](084633-agent-tunnel-repro-main-08f72b9-k-entry-0820.jpg) |
| 08:46:44 | [084644-agent-tunnel-repro-main-08f72b9-k-inside-0820.jpg](084644-agent-tunnel-repro-main-08f72b9-k-inside-0820.jpg) |
| 08:46:54 | [084654-agent-tunnel-repro-main-08f72b9-k-before-noon.jpg](084654-agent-tunnel-repro-main-08f72b9-k-before-noon.jpg) |
| 08:47:05 | [084705-agent-tunnel-repro-main-08f72b9-k-entry-noon.jpg](084705-agent-tunnel-repro-main-08f72b9-k-entry-noon.jpg) |
| 08:47:15 | [084715-agent-tunnel-repro-main-08f72b9-k-inside-noon.jpg](084715-agent-tunnel-repro-main-08f72b9-k-inside-noon.jpg) |
| 08:47:26 | [084726-agent-tunnel-repro-main-08f72b9-k-inside-night.jpg](084726-agent-tunnel-repro-main-08f72b9-k-inside-night.jpg) |
| 08:47:36 | [084736-agent-tunnel-repro-main-08f72b9-r-before-0820.jpg](084736-agent-tunnel-repro-main-08f72b9-r-before-0820.jpg) |
| 08:47:46 | [084746-agent-tunnel-repro-main-08f72b9-r-entry-0820.jpg](084746-agent-tunnel-repro-main-08f72b9-r-entry-0820.jpg) |
| 08:49:17 | [084917-agent-tunnel-repro-main-08f72b9-r-8.3-0820.jpg](084917-agent-tunnel-repro-main-08f72b9-r-8.3-0820.jpg) |
| 08:49:28 | [084928-agent-tunnel-repro-main-08f72b9-r-8.5-0820.jpg](084928-agent-tunnel-repro-main-08f72b9-r-8.5-0820.jpg) |
| 08:49:38 | [084938-agent-tunnel-repro-main-08f72b9-r-inside-0820.jpg](084938-agent-tunnel-repro-main-08f72b9-r-inside-0820.jpg) |
| 08:49:49 | [084949-agent-tunnel-repro-main-08f72b9-r-9.2-0820.jpg](084949-agent-tunnel-repro-main-08f72b9-r-9.2-0820.jpg) |
| 08:49:59 | [084959-agent-tunnel-repro-main-08f72b9-r-before-noon.jpg](084959-agent-tunnel-repro-main-08f72b9-r-before-noon.jpg) |
| 08:50:09 | [085009-agent-tunnel-repro-main-08f72b9-r-before-night.jpg](085009-agent-tunnel-repro-main-08f72b9-r-before-night.jpg) |
| 08:50:20 | [085020-agent-tunnel-repro-main-08f72b9-r-5.0-0820.jpg](085020-agent-tunnel-repro-main-08f72b9-r-5.0-0820.jpg) |
| 08:50:30 | [085030-agent-tunnel-repro-main-08f72b9-r-5.0-1600.jpg](085030-agent-tunnel-repro-main-08f72b9-r-5.0-1600.jpg) |
| 08:50:41 | [085041-agent-tunnel-repro-v7pkg-08f72b9-r-before-0820.jpg](085041-agent-tunnel-repro-v7pkg-08f72b9-r-before-0820.jpg) |
| 08:50:51 | [085051-agent-tunnel-repro-v7pkg-08f72b9-r-entry-0820.jpg](085051-agent-tunnel-repro-v7pkg-08f72b9-r-entry-0820.jpg) |
| 08:51:01 | [085101-agent-tunnel-repro-v7pkg-08f72b9-r-inside-0820.jpg](085101-agent-tunnel-repro-v7pkg-08f72b9-r-inside-0820.jpg) |
| 08:51:12 | [085112-agent-tunnel-repro-v7pkg-08f72b9-r-before-noon.jpg](085112-agent-tunnel-repro-v7pkg-08f72b9-r-before-noon.jpg) |
| 08:51:22 | [085122-agent-tunnel-repro-v7pkg-08f72b9-r-inside-night.jpg](085122-agent-tunnel-repro-v7pkg-08f72b9-r-inside-night.jpg) |
| 08:51:32 | [085132-agent-tunnel-repro-v7pkg-08f72b9-r-5.0-0820.jpg](085132-agent-tunnel-repro-v7pkg-08f72b9-r-5.0-0820.jpg) |
| 08:51:43 | [085143-agent-tunnel-repro-v7pkg-08f72b9-r-5.0-1600.jpg](085143-agent-tunnel-repro-v7pkg-08f72b9-r-5.0-1600.jpg) |
| 09:03:34 | [090334-shot-wayland-+0100-before-orbit-sprint-1200-30.png](090334-shot-wayland-+0100-before-orbit-sprint-1200-30.png) |
| 09:04:35 | [090435-shot-wayland-+0100-before-orbit-sprint-1200-40.png](090435-shot-wayland-+0100-before-orbit-sprint-1200-40.png) |
| 09:04:46 | [090446-shot-wayland-+0100-before-orbit-sprint-1200-42.png](090446-shot-wayland-+0100-before-orbit-sprint-1200-42.png) |
| 09:04:58 | [090458-shot-wayland-+0100-after-orbit-sprint-1200-30.png](090458-shot-wayland-+0100-after-orbit-sprint-1200-30.png) |
| 09:05:09 | [090509-shot-wayland-+0100-after-orbit-sprint-1200-40.png](090509-shot-wayland-+0100-after-orbit-sprint-1200-40.png) |
| 09:05:20 | [090520-shot-wayland-+0100-after-orbit-sprint-1200-42.png](090520-shot-wayland-+0100-after-orbit-sprint-1200-42.png) |
| 09:05:52 | [090552-agent-shadow-ab-zoom-before-40.png](090552-agent-shadow-ab-zoom-before-40.png) |
| 09:05:53 | [090553-agent-shadow-ab-zoom-after-40.png](090553-agent-shadow-ab-zoom-after-40.png) |
| 09:17:25 | [091725-shot-wayland-clean-li-climb.jpg](091725-shot-wayland-clean-li-climb.jpg) |
| 09:17:37 | [091737-shot-wayland-clean-li-rhine-bridge.jpg](091737-shot-wayland-clean-li-rhine-bridge.jpg) |
| 09:17:48 | [091748-shot-wayland-clean-li-samina-bridge.jpg](091748-shot-wayland-clean-li-samina-bridge.jpg) |
| 09:18:00 | [091800-shot-wayland-clean-li-malbun.jpg](091800-shot-wayland-clean-li-malbun.jpg) |
| 09:18:26 | [091826-video-wayland-clean-li-malbun-video.jpg](091826-video-wayland-clean-li-malbun-video.jpg) |
| 09:18:26 | [091826-video-wayland-clean-li-malbun-video.mp4](091826-video-wayland-clean-li-malbun-video.mp4) |
| 09:18:40 | [091840-shot-wayland-clean-bf-dreisam.jpg](091840-shot-wayland-clean-bf-dreisam.jpg) |
| 09:18:54 | [091854-shot-wayland-clean-bf-schauinsland.jpg](091854-shot-wayland-clean-bf-schauinsland.jpg) |
| 09:19:16 | [091916-video-wayland-clean-bf-schauinsland-video.jpg](091916-video-wayland-clean-bf-schauinsland-video.jpg) |
| 09:19:16 | [091916-video-wayland-clean-bf-schauinsland-video.mp4](091916-video-wayland-clean-bf-schauinsland-video.mp4) |
| 09:19:27 | [091927-shot-wayland-clean-li-climb.jpg](091927-shot-wayland-clean-li-climb.jpg) |
| 09:19:34 | [091934-agent-tunnel-repro-kiosk-08f72b9-r-before-0820.jpg](091934-agent-tunnel-repro-kiosk-08f72b9-r-before-0820.jpg) |
| 09:19:38 | [091938-shot-wayland-clean-li-rhine-bridge.jpg](091938-shot-wayland-clean-li-rhine-bridge.jpg) |
| 09:19:45 | [091945-agent-tunnel-repro-kiosk-08f72b9-r-8.5-0820.jpg](091945-agent-tunnel-repro-kiosk-08f72b9-r-8.5-0820.jpg) |
| 09:19:49 | [091949-shot-wayland-clean-li-samina-bridge.jpg](091949-shot-wayland-clean-li-samina-bridge.jpg) |
| 09:19:55 | [091955-agent-tunnel-repro-kiosk-08f72b9-r-inside-0820.jpg](091955-agent-tunnel-repro-kiosk-08f72b9-r-inside-0820.jpg) |
| 09:20:01 | [092001-shot-wayland-clean-li-malbun.jpg](092001-shot-wayland-clean-li-malbun.jpg) |
| 09:20:06 | [092006-agent-tunnel-repro-kiosk-08f72b9-r-5.0-0820.jpg](092006-agent-tunnel-repro-kiosk-08f72b9-r-5.0-0820.jpg) |
| 09:20:16 | [092016-agent-tunnel-repro-kiosk-08f72b9-r-before-noon.jpg](092016-agent-tunnel-repro-kiosk-08f72b9-r-before-noon.jpg) |
| 09:20:17 | [092017-video-wayland-clean-li-malbun-video.jpg](092017-video-wayland-clean-li-malbun-video.jpg) |
| 09:20:17 | [092017-video-wayland-clean-li-malbun-video.mp4](092017-video-wayland-clean-li-malbun-video.mp4) |
| 09:20:27 | [092027-agent-tunnel-repro-kiosk-08f72b9-r-inside-night.jpg](092027-agent-tunnel-repro-kiosk-08f72b9-r-inside-night.jpg) |
| 09:20:30 | [092030-shot-wayland-clean-bf-dreisam.jpg](092030-shot-wayland-clean-bf-dreisam.jpg) |
| 09:20:41 | [092041-agent-tunnel-repro-kiosk-08f72b9-tunnel-video-0820.jpg](092041-agent-tunnel-repro-kiosk-08f72b9-tunnel-video-0820.jpg) |
| 09:20:43 | [092043-shot-wayland-clean-bf-schauinsland.jpg](092043-shot-wayland-clean-bf-schauinsland.jpg) |
| 09:20:50 | [092050-agent-tunnel-repro-kiosk-08f72b9-tunnel-video-0820.mp4](092050-agent-tunnel-repro-kiosk-08f72b9-tunnel-video-0820.mp4) |
| 09:21:04 | [092104-video-wayland-clean-bf-schauinsland-video.jpg](092104-video-wayland-clean-bf-schauinsland-video.jpg) |
| 09:21:04 | [092104-video-wayland-clean-bf-schauinsland-video.mp4](092104-video-wayland-clean-bf-schauinsland-video.mp4) |
| 09:31:21 | [093121-agent-tunnel-repro-png-fbd2fce-r-before-0820.png](093121-agent-tunnel-repro-png-fbd2fce-r-before-0820.png) |
| 09:31:32 | [093132-agent-tunnel-repro-png-fbd2fce-r-8.5-0820.png](093132-agent-tunnel-repro-png-fbd2fce-r-8.5-0820.png) |
| 09:31:42 | [093142-agent-tunnel-repro-png-fbd2fce-r-5.0-0820.png](093142-agent-tunnel-repro-png-fbd2fce-r-5.0-0820.png) |
| 10:43:46 | [104346-shot-wayland-wheel-contact-flat.jpg](104346-shot-wayland-wheel-contact-flat.jpg) |
| 10:43:55 | [104355-shot-wayland-wheel-contact-steep.jpg](104355-shot-wayland-wheel-contact-steep.jpg) |
| 10:44:03 | [104403-shot-wayland-wheel-contact-lean.jpg](104403-shot-wayland-wheel-contact-lean.jpg) |
| 10:44:11 | [104411-shot-wayland-wheel-contact-mtb-steep.jpg](104411-shot-wayland-wheel-contact-mtb-steep.jpg) |
| 10:45:33 | [104533-agent-scenes-wheel-contact-hills-before-wheel-contact-flat-contact.jpg](104533-agent-scenes-wheel-contact-hills-before-wheel-contact-flat-contact.jpg) |
| 10:45:33 | [104533-agent-scenes-wheel-contact-hills-before-wheel-contact-mtb-steep-contact.jpg](104533-agent-scenes-wheel-contact-hills-before-wheel-contact-mtb-steep-contact.jpg) |
| 10:45:33 | [104533-agent-scenes-wheel-contact-hills-before-wheel-contact-steep-contact.jpg](104533-agent-scenes-wheel-contact-hills-before-wheel-contact-steep-contact.jpg) |
| 10:46:47 | [104647-shot-wayland-wheel-contact-flat.jpg](104647-shot-wayland-wheel-contact-flat.jpg) |
| 10:46:55 | [104655-shot-wayland-wheel-contact-steep.jpg](104655-shot-wayland-wheel-contact-steep.jpg) |
| 10:47:03 | [104703-shot-wayland-wheel-contact-lean.jpg](104703-shot-wayland-wheel-contact-lean.jpg) |
| 10:47:12 | [104712-shot-wayland-wheel-contact-mtb-steep.jpg](104712-shot-wayland-wheel-contact-mtb-steep.jpg) |
| 11:28:59 | [112859-shot-wayland-clean-li-climb.jpg](112859-shot-wayland-clean-li-climb.jpg) |
| 11:29:11 | [112911-shot-wayland-clean-li-rhine-bridge.jpg](112911-shot-wayland-clean-li-rhine-bridge.jpg) |
| 11:29:22 | [112922-shot-wayland-clean-li-samina-bridge.jpg](112922-shot-wayland-clean-li-samina-bridge.jpg) |
| 11:29:33 | [112933-shot-wayland-clean-li-malbun.jpg](112933-shot-wayland-clean-li-malbun.jpg) |
| 11:29:48 | [112948-video-wayland-clean-li-malbun-video.jpg](112948-video-wayland-clean-li-malbun-video.jpg) |
| 11:29:48 | [112948-video-wayland-clean-li-malbun-video.mp4](112948-video-wayland-clean-li-malbun-video.mp4) |
| 11:30:01 | [113001-shot-wayland-clean-bf-dreisam.jpg](113001-shot-wayland-clean-bf-dreisam.jpg) |
| 11:30:14 | [113014-shot-wayland-clean-bf-schauinsland.jpg](113014-shot-wayland-clean-bf-schauinsland.jpg) |
| 11:30:30 | [113030-video-wayland-clean-bf-schauinsland-video.jpg](113030-video-wayland-clean-bf-schauinsland-video.jpg) |
| 11:30:30 | [113030-video-wayland-clean-bf-schauinsland-video.mp4](113030-video-wayland-clean-bf-schauinsland-video.mp4) |
| 11:30:40 | [113040-shot-wayland-clean-li-climb.jpg](113040-shot-wayland-clean-li-climb.jpg) |
| 11:30:51 | [113051-shot-wayland-clean-li-rhine-bridge.jpg](113051-shot-wayland-clean-li-rhine-bridge.jpg) |
| 11:31:03 | [113103-shot-wayland-clean-li-samina-bridge.jpg](113103-shot-wayland-clean-li-samina-bridge.jpg) |
| 11:31:14 | [113114-shot-wayland-clean-li-malbun.jpg](113114-shot-wayland-clean-li-malbun.jpg) |
| 11:31:29 | [113129-video-wayland-clean-li-malbun-video.jpg](113129-video-wayland-clean-li-malbun-video.jpg) |
| 11:31:29 | [113129-video-wayland-clean-li-malbun-video.mp4](113129-video-wayland-clean-li-malbun-video.mp4) |
| 11:31:37 | [113137-shot-wayland-menu-time-machine.jpg](113137-shot-wayland-menu-time-machine.jpg) |
| 11:31:42 | [113142-shot-wayland-clean-bf-dreisam.jpg](113142-shot-wayland-clean-bf-dreisam.jpg) |
| 11:31:44 | [113144-shot-wayland-menu-world.jpg](113144-shot-wayland-menu-world.jpg) |
| 11:31:53 | [113153-shot-wayland-menu-world-change.jpg](113153-shot-wayland-menu-world-change.jpg) |
| 11:31:55 | [113155-shot-wayland-clean-bf-schauinsland.jpg](113155-shot-wayland-clean-bf-schauinsland.jpg) |
| 11:32:01 | [113201-shot-wayland-menu-world-info.jpg](113201-shot-wayland-menu-world-info.jpg) |
| 11:32:10 | [113210-video-wayland-clean-bf-schauinsland-video.jpg](113210-video-wayland-clean-bf-schauinsland-video.jpg) |
| 11:32:10 | [113210-video-wayland-clean-bf-schauinsland-video.mp4](113210-video-wayland-clean-bf-schauinsland-video.mp4) |
| 11:32:11 | [113211-shot-wayland-menu-world-sun.jpg](113211-shot-wayland-menu-world-sun.jpg) |
| 11:32:22 | [113222-shot-wayland-menu-world-source.jpg](113222-shot-wayland-menu-world-source.jpg) |
| 11:32:31 | [113231-shot-wayland-menu-world-weather.jpg](113231-shot-wayland-menu-world-weather.jpg) |
| 11:33:19 | [113319-video-wayland-world-switch-blackforest.jpg](113319-video-wayland-world-switch-blackforest.jpg) |
| 11:33:19 | [113319-video-wayland-world-switch-blackforest.mp4](113319-video-wayland-world-switch-blackforest.mp4) |
| 11:34:34 | [113434-agent-tunnel-repro-after-kiosk-r-before-0820.jpg](113434-agent-tunnel-repro-after-kiosk-r-before-0820.jpg) |
| 11:34:45 | [113445-agent-tunnel-repro-after-kiosk-r-8.5-0820.jpg](113445-agent-tunnel-repro-after-kiosk-r-8.5-0820.jpg) |
| 11:34:55 | [113455-agent-tunnel-repro-after-kiosk-r-entry-0820.jpg](113455-agent-tunnel-repro-after-kiosk-r-entry-0820.jpg) |
| 11:35:05 | [113505-agent-tunnel-repro-after-kiosk-r-inside-0820.jpg](113505-agent-tunnel-repro-after-kiosk-r-inside-0820.jpg) |
| 11:35:16 | [113516-agent-tunnel-repro-after-kiosk-r-5.0-0820.jpg](113516-agent-tunnel-repro-after-kiosk-r-5.0-0820.jpg) |
| 11:35:26 | [113526-agent-tunnel-repro-after-kiosk-r-before-noon.jpg](113526-agent-tunnel-repro-after-kiosk-r-before-noon.jpg) |
| 11:35:37 | [113537-agent-tunnel-repro-after-kiosk-r-inside-night.jpg](113537-agent-tunnel-repro-after-kiosk-r-inside-night.jpg) |
| 11:35:47 | [113547-agent-tunnel-repro-after-kiosk-r-2.0-0820.jpg](113547-agent-tunnel-repro-after-kiosk-r-2.0-0820.jpg) |
| 11:36:03 | [113603-agent-tunnel-repro-after-kiosk-tunnel-video-0820.jpg](113603-agent-tunnel-repro-after-kiosk-tunnel-video-0820.jpg) |
| 11:36:16 | [113616-agent-tunnel-repro-after-kiosk-tunnel-video-0820.mp4](113616-agent-tunnel-repro-after-kiosk-tunnel-video-0820.mp4) |
| 11:39:43 | [113943-agent-tunnel-repro-after-kiosk-tunnel-through-0820.jpg](113943-agent-tunnel-repro-after-kiosk-tunnel-through-0820.jpg) |
| 11:39:53 | [113953-agent-tunnel-repro-after-kiosk-tunnel-through-0820.mp4](113953-agent-tunnel-repro-after-kiosk-tunnel-through-0820.mp4) |
| 11:53:10 | [115310-agent-scenes-tenerife-1-showcase-tf-teide.jpg](115310-agent-scenes-tenerife-1-showcase-tf-teide.jpg) |
| 11:53:22 | [115322-agent-scenes-tenerife-1-showcase-tf-teide-video.jpg](115322-agent-scenes-tenerife-1-showcase-tf-teide-video.jpg) |
| 11:53:32 | [115332-agent-scenes-tenerife-1-showcase-tf-teide-video.mp4](115332-agent-scenes-tenerife-1-showcase-tf-teide-video.mp4) |
| 11:53:47 | [115347-agent-scenes-tenerife-1-showcase-tf-cablecar.jpg](115347-agent-scenes-tenerife-1-showcase-tf-cablecar.jpg) |
| 11:54:01 | [115401-agent-scenes-tenerife-1-showcase-tf-teide-sunrise.jpg](115401-agent-scenes-tenerife-1-showcase-tf-teide-sunrise.jpg) |
| 11:54:16 | [115416-agent-scenes-tenerife-1-showcase-tf-teide-sunset.jpg](115416-agent-scenes-tenerife-1-showcase-tf-teide-sunset.jpg) |
| 11:54:30 | [115430-agent-scenes-tenerife-1-showcase-tf-masca.jpg](115430-agent-scenes-tenerife-1-showcase-tf-masca.jpg) |
| 11:54:43 | [115443-agent-scenes-tenerife-1-showcase-tf-masca-video.jpg](115443-agent-scenes-tenerife-1-showcase-tf-masca-video.jpg) |
| 11:54:54 | [115454-agent-scenes-tenerife-1-showcase-tf-masca-video.mp4](115454-agent-scenes-tenerife-1-showcase-tf-masca-video.mp4) |
| 11:55:08 | [115508-agent-scenes-tenerife-1-showcase-tf-gigantes.jpg](115508-agent-scenes-tenerife-1-showcase-tf-gigantes.jpg) |
| 11:55:23 | [115523-agent-scenes-tenerife-1-showcase-tf-santacruz.jpg](115523-agent-scenes-tenerife-1-showcase-tf-santacruz.jpg) |
| 11:55:37 | [115537-agent-scenes-tenerife-1-showcase-tf-orotava.jpg](115537-agent-scenes-tenerife-1-showcase-tf-orotava.jpg) |
| 11:55:52 | [115552-agent-scenes-tenerife-1-showcase-tf-anaga.jpg](115552-agent-scenes-tenerife-1-showcase-tf-anaga.jpg) |
| 11:57:41 | [115741-agent-scenes-wheel-contact-hills-after-wheel-contact-flat-contact.jpg](115741-agent-scenes-wheel-contact-hills-after-wheel-contact-flat-contact.jpg) |
| 11:57:41 | [115741-agent-scenes-wheel-contact-hills-after-wheel-contact-mtb-steep-contact.jpg](115741-agent-scenes-wheel-contact-hills-after-wheel-contact-mtb-steep-contact.jpg) |
| 11:57:41 | [115741-agent-scenes-wheel-contact-hills-after-wheel-contact-steep-contact.jpg](115741-agent-scenes-wheel-contact-hills-after-wheel-contact-steep-contact.jpg) |
| 11:58:29 | [115829-shot-wayland-wheel-contact-flat.jpg](115829-shot-wayland-wheel-contact-flat.jpg) |
| 11:58:37 | [115837-shot-wayland-wheel-contact-steep.jpg](115837-shot-wayland-wheel-contact-steep.jpg) |
| 11:58:45 | [115845-shot-wayland-wheel-contact-lean.jpg](115845-shot-wayland-wheel-contact-lean.jpg) |
| 11:58:53 | [115853-shot-wayland-wheel-contact-mtb-steep.jpg](115853-shot-wayland-wheel-contact-mtb-steep.jpg) |
| 12:24:57 | [122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-flat-contact.jpg](122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-flat-contact.jpg) |
| 12:24:57 | [122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-mtb-steep-contact.jpg](122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-mtb-steep-contact.jpg) |
| 12:24:57 | [122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-steep-contact.jpg](122457-agent-scenes-wheel-contact-vaduz2-before-wheel-contact-vaduz-steep-contact.jpg) |
| 12:31:33 | [123133-shot-wayland-wheel-contact-vaduz-flat.jpg](123133-shot-wayland-wheel-contact-vaduz-flat.jpg) |
| 12:31:42 | [123142-shot-wayland-wheel-contact-vaduz-steep.jpg](123142-shot-wayland-wheel-contact-vaduz-steep.jpg) |
| 12:31:50 | [123150-shot-wayland-wheel-contact-vaduz-mtb-steep.jpg](123150-shot-wayland-wheel-contact-vaduz-mtb-steep.jpg) |
| 12:32:49 | [123249-agent-scenes-tenerife-2-showcase-tf-teide-sunset.jpg](123249-agent-scenes-tenerife-2-showcase-tf-teide-sunset.jpg) |
| 12:33:04 | [123304-agent-scenes-tenerife-2-showcase-tf-masca.jpg](123304-agent-scenes-tenerife-2-showcase-tf-masca.jpg) |
| 12:33:18 | [123318-agent-scenes-tenerife-2-showcase-tf-gigantes.jpg](123318-agent-scenes-tenerife-2-showcase-tf-gigantes.jpg) |
| 12:33:34 | [123334-agent-scenes-tenerife-2-showcase-tf-santacruz.jpg](123334-agent-scenes-tenerife-2-showcase-tf-santacruz.jpg) |
| 12:40:50 | [124050-agent-scenes-t2-merge-before-far-view.jpg](124050-agent-scenes-t2-merge-before-far-view.jpg) |
| 12:41:00 | [124100-agent-scenes-t2-merge-before-mat-meadow-road.jpg](124100-agent-scenes-t2-merge-before-mat-meadow-road.jpg) |
| 12:41:11 | [124111-agent-scenes-t2-merge-before-mat-forest.jpg](124111-agent-scenes-t2-merge-before-mat-forest.jpg) |
| 12:41:20 | [124120-agent-scenes-t2-merge-before-mat-rhine-gravel.jpg](124120-agent-scenes-t2-merge-before-mat-rhine-gravel.jpg) |
| 12:41:33 | [124133-agent-scenes-t2-merge-before-bf-mat-forest.jpg](124133-agent-scenes-t2-merge-before-bf-mat-forest.jpg) |
| 12:41:47 | [124147-agent-scenes-t2-merge-after-far-view.jpg](124147-agent-scenes-t2-merge-after-far-view.jpg) |
| 12:41:58 | [124158-agent-scenes-t2-merge-after-mat-meadow-road.jpg](124158-agent-scenes-t2-merge-after-mat-meadow-road.jpg) |
| 12:42:08 | [124208-agent-scenes-t2-merge-after-mat-forest.jpg](124208-agent-scenes-t2-merge-after-mat-forest.jpg) |
| 12:42:18 | [124218-agent-scenes-t2-merge-after-mat-rhine-gravel.jpg](124218-agent-scenes-t2-merge-after-mat-rhine-gravel.jpg) |
| 12:42:31 | [124231-agent-scenes-t2-merge-after-bf-mat-forest.jpg](124231-agent-scenes-t2-merge-after-bf-mat-forest.jpg) |
| 12:42:50 | [124250-agent-scenes-t2-merge-compare-far-view.jpg](124250-agent-scenes-t2-merge-compare-far-view.jpg) |
| 12:42:50 | [124250-agent-scenes-t2-merge-compare-mat-forest.jpg](124250-agent-scenes-t2-merge-compare-mat-forest.jpg) |
| 12:42:50 | [124250-agent-scenes-t2-merge-compare-mat-meadow-road.jpg](124250-agent-scenes-t2-merge-compare-mat-meadow-road.jpg) |
| 12:42:50 | [124250-agent-scenes-t2-merge-compare-mat-rhine-gravel.jpg](124250-agent-scenes-t2-merge-compare-mat-rhine-gravel.jpg) |
| 12:42:51 | [124251-agent-scenes-t2-merge-compare-bf-mat-forest.jpg](124251-agent-scenes-t2-merge-compare-bf-mat-forest.jpg) |
| 12:42:51 | [124251-agent-scenes-t2-merge-compare-sheet.jpg](124251-agent-scenes-t2-merge-compare-sheet.jpg) |
| 12:46:23 | [124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-flat-contact.jpg](124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-flat-contact.jpg) |
| 12:46:23 | [124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-mtb-steep-contact.jpg](124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-mtb-steep-contact.jpg) |
| 12:46:23 | [124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-steep-contact.jpg](124623-agent-scenes-wheel-contact-vaduz2-after-wheel-contact-vaduz-steep-contact.jpg) |
| 12:47:04 | [124704-shot-wayland-wheel-contact-vaduz-flat.jpg](124704-shot-wayland-wheel-contact-vaduz-flat.jpg) |
| 12:47:13 | [124713-shot-wayland-wheel-contact-vaduz-steep.jpg](124713-shot-wayland-wheel-contact-vaduz-steep.jpg) |
| 12:47:21 | [124721-shot-wayland-wheel-contact-vaduz-mtb-steep.jpg](124721-shot-wayland-wheel-contact-vaduz-mtb-steep.jpg) |
| 12:52:43 | [125243-shot-wayland-readme-climb-behind.jpg](125243-shot-wayland-readme-climb-behind.jpg) |
| 12:52:59 | [125259-shot-wayland-bf-schauinsland-group.jpg](125259-shot-wayland-bf-schauinsland-group.jpg) |
| 12:53:12 | [125312-shot-wayland-clean-li-rhine-bridge.jpg](125312-shot-wayland-clean-li-rhine-bridge.jpg) |
| 12:53:22 | [125322-shot-wayland-sky-golden-hour.jpg](125322-shot-wayland-sky-golden-hour.jpg) |
| 12:53:37 | [125337-shot-wayland-showcase-mummelsee.jpg](125337-shot-wayland-showcase-mummelsee.jpg) |
| 12:53:52 | [125352-shot-wayland-showcase-hochstrasse.jpg](125352-shot-wayland-showcase-hochstrasse.jpg) |
| 12:54:07 | [125407-shot-wayland-showcase-tf-teide-sunset.jpg](125407-shot-wayland-showcase-tf-teide-sunset.jpg) |
| 12:54:22 | [125422-shot-wayland-showcase-tf-masca.jpg](125422-shot-wayland-showcase-tf-masca.jpg) |
| 12:54:37 | [125437-shot-wayland-showcase-tf-gigantes.jpg](125437-shot-wayland-showcase-tf-gigantes.jpg) |
| 12:58:00 | [125800-agent-scenes-tf-before-showcase-tf-teide.jpg](125800-agent-scenes-tf-before-showcase-tf-teide.jpg) |
| 12:58:15 | [125815-agent-scenes-tf-before-showcase-tf-cablecar.jpg](125815-agent-scenes-tf-before-showcase-tf-cablecar.jpg) |
| 12:58:29 | [125829-agent-scenes-tf-before-showcase-tf-gigantes.jpg](125829-agent-scenes-tf-before-showcase-tf-gigantes.jpg) |
| 12:58:44 | [125844-agent-scenes-tf-after-showcase-tf-teide.jpg](125844-agent-scenes-tf-after-showcase-tf-teide.jpg) |
| 12:58:58 | [125858-agent-scenes-tf-after-showcase-tf-cablecar.jpg](125858-agent-scenes-tf-after-showcase-tf-cablecar.jpg) |
| 12:59:12 | [125912-agent-scenes-tf-after-showcase-tf-gigantes.jpg](125912-agent-scenes-tf-after-showcase-tf-gigantes.jpg) |
| 12:59:30 | [125930-agent-scenes-tf-compare-sheet.jpg](125930-agent-scenes-tf-compare-sheet.jpg) |
| 12:59:30 | [125930-agent-scenes-tf-compare-showcase-tf-cablecar.jpg](125930-agent-scenes-tf-compare-showcase-tf-cablecar.jpg) |
| 12:59:30 | [125930-agent-scenes-tf-compare-showcase-tf-gigantes.jpg](125930-agent-scenes-tf-compare-showcase-tf-gigantes.jpg) |
| 12:59:30 | [125930-agent-scenes-tf-compare-showcase-tf-teide.jpg](125930-agent-scenes-tf-compare-showcase-tf-teide.jpg) |
| 14:31:30 | [143130-shot-wayland-look-vaduz-roofs.jpg](143130-shot-wayland-look-vaduz-roofs.jpg) |
| 14:31:33 | [143133-shot-wayland-look-bf-farms.jpg](143133-shot-wayland-look-bf-farms.jpg) |
| 14:31:41 | [143141-shot-wayland-look-malbun-meadows.jpg](143141-shot-wayland-look-malbun-meadows.jpg) |
| 14:31:47 | [143147-shot-wayland-look-bf-forest.jpg](143147-shot-wayland-look-bf-forest.jpg) |
| 14:31:52 | [143152-shot-wayland-look-season-summer.jpg](143152-shot-wayland-look-season-summer.jpg) |
| 14:32:53 | [143253-shot-wayland-look-season-autumn.jpg](143253-shot-wayland-look-season-autumn.jpg) |
| 14:33:05 | [143305-shot-wayland-look-season-winter.jpg](143305-shot-wayland-look-season-winter.jpg) |
| 14:33:32 | [143332-shot-wayland-look-tf-canadas.jpg](143332-shot-wayland-look-tf-canadas.jpg) |
| 14:50:33 | [145033-shot-wayland-look-vaduz-roofs.jpg](145033-shot-wayland-look-vaduz-roofs.jpg) |
| 14:50:43 | [145043-shot-wayland-look-malbun-meadows.jpg](145043-shot-wayland-look-malbun-meadows.jpg) |
| 14:50:55 | [145055-shot-wayland-look-season-summer.jpg](145055-shot-wayland-look-season-summer.jpg) |
| 14:51:10 | [145110-shot-wayland-look-bf-farms.jpg](145110-shot-wayland-look-bf-farms.jpg) |
| 14:51:10 | [145110-shot-wayland-look-tf-canadas.jpg](145110-shot-wayland-look-tf-canadas.jpg) |
| 14:51:25 | [145125-shot-wayland-look-bf-forest.jpg](145125-shot-wayland-look-bf-forest.jpg) |
| 14:58:21 | [145821-shot-wayland-teide-roques-y0.jpg](145821-shot-wayland-teide-roques-y0.jpg) |
| 14:58:38 | [145838-shot-wayland-teide-roques-y180.jpg](145838-shot-wayland-teide-roques-y180.jpg) |
| 14:58:54 | [145854-shot-wayland-teide-minas-y0.jpg](145854-shot-wayland-teide-minas-y0.jpg) |
| 14:59:11 | [145911-shot-wayland-teide-minas-y180.jpg](145911-shot-wayland-teide-minas-y180.jpg) |
| 14:59:27 | [145927-shot-wayland-teide-portillo-y0.jpg](145927-shot-wayland-teide-portillo-y0.jpg) |
| 14:59:43 | [145943-shot-wayland-teide-portillo-y180.jpg](145943-shot-wayland-teide-portillo-y180.jpg) |
| 15:00:32 | [150032-shot-wayland-look-season-summer.jpg](150032-shot-wayland-look-season-summer.jpg) |
| 15:00:45 | [150045-shot-wayland-look-season-autumn.jpg](150045-shot-wayland-look-season-autumn.jpg) |
| 15:00:57 | [150057-shot-wayland-look-season-winter.jpg](150057-shot-wayland-look-season-winter.jpg) |
| 15:14:37 | [151437-shot-wayland-teide2-a.jpg](151437-shot-wayland-teide2-a.jpg) |
| 15:14:53 | [151453-shot-wayland-teide2-b.jpg](151453-shot-wayland-teide2-b.jpg) |
| 15:15:08 | [151508-shot-wayland-teide2-c.jpg](151508-shot-wayland-teide2-c.jpg) |
| 15:15:23 | [151523-shot-wayland-teide2-d.jpg](151523-shot-wayland-teide2-d.jpg) |
| 15:15:38 | [151538-shot-wayland-teide2-e.jpg](151538-shot-wayland-teide2-e.jpg) |
| 15:15:53 | [151553-shot-wayland-teide2-f.jpg](151553-shot-wayland-teide2-f.jpg) |
| 15:21:13 | [152113-shot-wayland-teide3-p1.jpg](152113-shot-wayland-teide3-p1.jpg) |
| 15:21:28 | [152128-shot-wayland-teide3-p2.jpg](152128-shot-wayland-teide3-p2.jpg) |
| 15:21:44 | [152144-shot-wayland-teide3-m1.jpg](152144-shot-wayland-teide3-m1.jpg) |
| 15:22:01 | [152201-shot-wayland-teide3-m2.jpg](152201-shot-wayland-teide3-m2.jpg) |
| 15:22:17 | [152217-shot-wayland-teide3-m3.jpg](152217-shot-wayland-teide3-m3.jpg) |
| 15:22:33 | [152233-shot-wayland-teide3-r1.jpg](152233-shot-wayland-teide3-r1.jpg) |
| 15:27:43 | [152743-shot-wayland-teide4-a.jpg](152743-shot-wayland-teide4-a.jpg) |
| 15:27:59 | [152759-shot-wayland-teide4-b.jpg](152759-shot-wayland-teide4-b.jpg) |
| 15:28:15 | [152815-shot-wayland-teide4-c.jpg](152815-shot-wayland-teide4-c.jpg) |
| 15:28:32 | [152832-shot-wayland-teide4-d.jpg](152832-shot-wayland-teide4-d.jpg) |
| 15:28:48 | [152848-shot-wayland-teide4-e.jpg](152848-shot-wayland-teide4-e.jpg) |
| 15:29:04 | [152904-shot-wayland-teide4-f.jpg](152904-shot-wayland-teide4-f.jpg) |
| 16:00:06 | [160006-shot-wayland-teide5-a.jpg](160006-shot-wayland-teide5-a.jpg) |
| 16:00:21 | [160021-shot-wayland-teide5-b.jpg](160021-shot-wayland-teide5-b.jpg) |
| 16:00:38 | [160038-shot-wayland-teide5-c.jpg](160038-shot-wayland-teide5-c.jpg) |
| 16:00:54 | [160054-shot-wayland-teide5-d.jpg](160054-shot-wayland-teide5-d.jpg) |
| 16:01:10 | [160110-shot-wayland-teide5-e.jpg](160110-shot-wayland-teide5-e.jpg) |
| 16:01:26 | [160126-shot-wayland-teide5-f.jpg](160126-shot-wayland-teide5-f.jpg) |
| 17:03:22 | [170322-shot-wayland-showcase-tf-teide-iconic.jpg](170322-shot-wayland-showcase-tf-teide-iconic.jpg) |
| 17:03:37 | [170337-shot-wayland-showcase-tf-teide-km18.jpg](170337-shot-wayland-showcase-tf-teide-km18.jpg) |
| 18:39:48 | [183948-agent-scenes-before-t4-2026-11-13T1830-look-houses-vaduz.jpg](183948-agent-scenes-before-t4-2026-11-13T1830-look-houses-vaduz.jpg) |
| 18:40:00 | [184000-agent-scenes-before-t4-2026-11-13T1830-look-houses-triesenberg.jpg](184000-agent-scenes-before-t4-2026-11-13T1830-look-houses-triesenberg.jpg) |
| 18:40:10 | [184010-agent-scenes-before-t4-2026-11-13T1830-look-houses-malbun.jpg](184010-agent-scenes-before-t4-2026-11-13T1830-look-houses-malbun.jpg) |
| 18:40:23 | [184023-agent-scenes-before-t4-2026-11-13T1830-look-houses-bf-stmaergen.jpg](184023-agent-scenes-before-t4-2026-11-13T1830-look-houses-bf-stmaergen.jpg) |
| 18:40:38 | [184038-agent-scenes-before-t4-2026-11-13T1830-look-houses-bf-hofsgrund.jpg](184038-agent-scenes-before-t4-2026-11-13T1830-look-houses-bf-hofsgrund.jpg) |
| 18:40:50 | [184050-agent-scenes-before-t4-2026-11-13T1830-look-houses-tf-orotava.jpg](184050-agent-scenes-before-t4-2026-11-13T1830-look-houses-tf-orotava.jpg) |
| 18:41:03 | [184103-agent-scenes-before-t4-2026-11-13T1830-look-houses-tf-gigantes.jpg](184103-agent-scenes-before-t4-2026-11-13T1830-look-houses-tf-gigantes.jpg) |
| 18:42:22 | [184222-shot-wayland-showcase-mummelsee.jpg](184222-shot-wayland-showcase-mummelsee.jpg) |
| 18:42:37 | [184237-shot-wayland-showcase-hornisgrinde.jpg](184237-shot-wayland-showcase-hornisgrinde.jpg) |
| 18:42:53 | [184253-shot-wayland-showcase-feldberg.jpg](184253-shot-wayland-showcase-feldberg.jpg) |
| 18:43:09 | [184309-shot-wayland-showcase-belchen.jpg](184309-shot-wayland-showcase-belchen.jpg) |
| 18:43:23 | [184323-shot-wayland-showcase-malbun-hairpins.jpg](184323-shot-wayland-showcase-malbun-hairpins.jpg) |
| 18:43:37 | [184337-shot-wayland-showcase-tf-teide-iconic.jpg](184337-shot-wayland-showcase-tf-teide-iconic.jpg) |
| 18:44:29 | [184429-shot-wayland-showcase-mummelsee.jpg](184429-shot-wayland-showcase-mummelsee.jpg) |
| 18:44:44 | [184444-shot-wayland-showcase-hornisgrinde.jpg](184444-shot-wayland-showcase-hornisgrinde.jpg) |
| 18:44:59 | [184459-shot-wayland-showcase-feldberg.jpg](184459-shot-wayland-showcase-feldberg.jpg) |
| 18:45:15 | [184515-shot-wayland-showcase-belchen.jpg](184515-shot-wayland-showcase-belchen.jpg) |
| 18:45:28 | [184528-shot-wayland-showcase-malbun-hairpins.jpg](184528-shot-wayland-showcase-malbun-hairpins.jpg) |
| 18:48:56 | [184856-agent-scenes-showcase-mummelsee.jpg](184856-agent-scenes-showcase-mummelsee.jpg) |
| 18:49:49 | [184949-shot-wayland-showcase-hornisgrinde.jpg](184949-shot-wayland-showcase-hornisgrinde.jpg) |
| 18:50:05 | [185005-shot-wayland-showcase-feldberg.jpg](185005-shot-wayland-showcase-feldberg.jpg) |
| 18:50:20 | [185020-shot-wayland-showcase-belchen.jpg](185020-shot-wayland-showcase-belchen.jpg) |
| 18:50:34 | [185034-shot-wayland-showcase-malbun-hairpins.jpg](185034-shot-wayland-showcase-malbun-hairpins.jpg) |
| 19:40:32 | [194032-agent-scenes-after-t4-1-look-houses-vaduz.jpg](194032-agent-scenes-after-t4-1-look-houses-vaduz.jpg) |
| 19:40:42 | [194042-agent-scenes-after-t4-1-look-houses-triesenberg.jpg](194042-agent-scenes-after-t4-1-look-houses-triesenberg.jpg) |
| 19:40:53 | [194053-agent-scenes-after-t4-1-look-houses-malbun.jpg](194053-agent-scenes-after-t4-1-look-houses-malbun.jpg) |
| 19:41:07 | [194107-agent-scenes-after-t4-1-look-houses-bf-stmaergen.jpg](194107-agent-scenes-after-t4-1-look-houses-bf-stmaergen.jpg) |
| 19:41:20 | [194120-agent-scenes-after-t4-1-look-houses-bf-hofsgrund.jpg](194120-agent-scenes-after-t4-1-look-houses-bf-hofsgrund.jpg) |
| 19:41:33 | [194133-agent-scenes-after-t4-1-look-houses-tf-orotava.jpg](194133-agent-scenes-after-t4-1-look-houses-tf-orotava.jpg) |
| 19:41:46 | [194146-agent-scenes-after-t4-1-look-houses-tf-gigantes.jpg](194146-agent-scenes-after-t4-1-look-houses-tf-gigantes.jpg) |
| 19:43:28 | [194328-shot-wayland-god-unicorn-flight.jpg](194328-shot-wayland-god-unicorn-flight.jpg) |
| 19:43:38 | [194338-shot-wayland-god-unicorn-side.jpg](194338-shot-wayland-god-unicorn-side.jpg) |
| 19:44:27 | [194427-video-wayland-god-flight-video.jpg](194427-video-wayland-god-flight-video.jpg) |
| 19:44:27 | [194427-video-wayland-god-flight-video.mp4](194427-video-wayland-god-flight-video.mp4) |
| 19:47:58 | [194758-agent-scenes-after-t4-2-look-houses-triesenberg.jpg](194758-agent-scenes-after-t4-2-look-houses-triesenberg.jpg) |
| 19:48:15 | [194815-shot-wayland-god-unicorn-flight.jpg](194815-shot-wayland-god-unicorn-flight.jpg) |
| 19:48:25 | [194825-shot-wayland-god-unicorn-side.jpg](194825-shot-wayland-god-unicorn-side.jpg) |
| 19:49:20 | [194920-agent-build-history-2026-11-13-god-flight-video.mp4](194920-agent-build-history-2026-11-13-god-flight-video.mp4) |
| 19:49:20 | [194920-video-wayland-god-flight-video.jpg](194920-video-wayland-god-flight-video.jpg) |
| 19:49:20 | [194920-video-wayland-god-flight-video.mp4](194920-video-wayland-god-flight-video.mp4) |
| 19:50:08 | [195008-agent-scenes-after-t4-3-look-houses-vaduz.jpg](195008-agent-scenes-after-t4-3-look-houses-vaduz.jpg) |
| 19:50:19 | [195019-agent-scenes-after-t4-3-look-houses-triesenberg.jpg](195019-agent-scenes-after-t4-3-look-houses-triesenberg.jpg) |
| 19:50:29 | [195029-agent-scenes-after-t4-3-look-houses-malbun.jpg](195029-agent-scenes-after-t4-3-look-houses-malbun.jpg) |
| 19:50:42 | [195042-agent-scenes-after-t4-3-look-houses-bf-stmaergen.jpg](195042-agent-scenes-after-t4-3-look-houses-bf-stmaergen.jpg) |
| 19:50:56 | [195056-agent-scenes-after-t4-3-look-houses-bf-hofsgrund.jpg](195056-agent-scenes-after-t4-3-look-houses-bf-hofsgrund.jpg) |
| 19:51:09 | [195109-agent-scenes-after-t4-3-look-houses-tf-orotava.jpg](195109-agent-scenes-after-t4-3-look-houses-tf-orotava.jpg) |
| 19:51:21 | [195121-agent-scenes-after-t4-3-look-houses-tf-gigantes.jpg](195121-agent-scenes-after-t4-3-look-houses-tf-gigantes.jpg) |
| 20:26:01 | [202601-agent-scenes-after-t4-final-look-houses-vaduz.jpg](202601-agent-scenes-after-t4-final-look-houses-vaduz.jpg) |
| 20:26:12 | [202612-agent-scenes-after-t4-final-look-houses-triesenberg.jpg](202612-agent-scenes-after-t4-final-look-houses-triesenberg.jpg) |
| 20:26:22 | [202622-agent-scenes-after-t4-final-look-houses-malbun.jpg](202622-agent-scenes-after-t4-final-look-houses-malbun.jpg) |
| 20:26:36 | [202636-agent-scenes-after-t4-final-look-houses-bf-stmaergen.jpg](202636-agent-scenes-after-t4-final-look-houses-bf-stmaergen.jpg) |
| 20:26:49 | [202649-agent-scenes-after-t4-final-look-houses-bf-hofsgrund.jpg](202649-agent-scenes-after-t4-final-look-houses-bf-hofsgrund.jpg) |
| 20:27:02 | [202702-agent-scenes-after-t4-final-look-houses-tf-orotava.jpg](202702-agent-scenes-after-t4-final-look-houses-tf-orotava.jpg) |
| 20:27:15 | [202715-agent-scenes-after-t4-final-look-houses-tf-gigantes.jpg](202715-agent-scenes-after-t4-final-look-houses-tf-gigantes.jpg) |
| 20:47:55 | [204755-shot-ocw-a3e1-x11.jpg](204755-shot-ocw-a3e1-x11.jpg) |
| 20:48:04 | [204804-shot-wayland-ocw-a3e1-wl.jpg](204804-shot-wayland-ocw-a3e1-wl.jpg) |
| 20:49:13 | [204913-shot-showcase-malbun-hairpins.jpg](204913-shot-showcase-malbun-hairpins.jpg) |
| 20:56:50 | [205650-shot-ocw.jpg](205650-shot-ocw.jpg) |
| 20:56:59 | [205659-shot-wayland-ocw-wayland.jpg](205659-shot-wayland-ocw-wayland.jpg) |
| 21:22:51 | [212251-shot-wayland-god-unicorn-flight.jpg](212251-shot-wayland-god-unicorn-flight.jpg) |
| 21:23:02 | [212302-shot-wayland-god-inspect-vaduz.jpg](212302-shot-wayland-god-inspect-vaduz.jpg) |
| 21:23:15 | [212315-shot-wayland-god-inspect-tenerife.jpg](212315-shot-wayland-god-inspect-tenerife.jpg) |
| 21:23:16 | [212316-agent-scenes-t5-before-road-vaduz-aeule.jpg](212316-agent-scenes-t5-before-road-vaduz-aeule.jpg) |
| 21:23:27 | [212327-agent-scenes-t5-before-road-vaduz-zebra.jpg](212327-agent-scenes-t5-before-road-vaduz-zebra.jpg) |
| 21:23:41 | [212341-agent-scenes-t5-before-road-freiburg-centre.jpg](212341-agent-scenes-t5-before-road-freiburg-centre.jpg) |
| 21:24:08 | [212408-agent-scenes-t5-before-road-canadas-edge.jpg](212408-agent-scenes-t5-before-road-canadas-edge.jpg) |
| 21:32:37 | [213237-shot-wayland-god-inspect-vaduz.jpg](213237-shot-wayland-god-inspect-vaduz.jpg) |
| 21:32:51 | [213251-shot-wayland-god-inspect-tenerife.jpg](213251-shot-wayland-god-inspect-tenerife.jpg) |
| 21:35:21 | [213521-shot-wayland-showcase-tf-teide-iconic.jpg](213521-shot-wayland-showcase-tf-teide-iconic.jpg) |
| 21:35:59 | [213559-shot-wayland-god-inspect-vaduz.jpg](213559-shot-wayland-god-inspect-vaduz.jpg) |
| 21:36:13 | [213613-shot-wayland-god-inspect-tenerife.jpg](213613-shot-wayland-god-inspect-tenerife.jpg) |
| 21:36:30 | [213630-agent-scenes-t5-after-li-road-vaduz-aeule.jpg](213630-agent-scenes-t5-after-li-road-vaduz-aeule.jpg) |
| 21:36:41 | [213641-agent-scenes-t5-after-li-road-vaduz-zebra.jpg](213641-agent-scenes-t5-after-li-road-vaduz-zebra.jpg) |
| 21:40:38 | [214038-agent-scenes-t5-after-tf-road-canadas-edge.jpg](214038-agent-scenes-t5-after-tf-road-canadas-edge.jpg) |
| 21:58:06 | [215806-agent-scenes-t5-before-road-country.jpg](215806-agent-scenes-t5-before-road-country.jpg) |
| 21:58:45 | [215845-agent-scenes-t5-after-road-vaduz-aeule.jpg](215845-agent-scenes-t5-after-road-vaduz-aeule.jpg) |
| 21:58:56 | [215856-agent-scenes-t5-after-road-vaduz-zebra.jpg](215856-agent-scenes-t5-after-road-vaduz-zebra.jpg) |
| 21:59:09 | [215909-agent-scenes-t5-after-road-freiburg-centre.jpg](215909-agent-scenes-t5-after-road-freiburg-centre.jpg) |
| 21:59:20 | [215920-agent-scenes-t5-after-road-country.jpg](215920-agent-scenes-t5-after-road-country.jpg) |
| 21:59:33 | [215933-agent-scenes-t5-after-road-canadas-edge.jpg](215933-agent-scenes-t5-after-road-canadas-edge.jpg) |
