# Build history

Screenshots and videos made while developing Open Cycling World, kept as a
visual record of how the client evolved. They come from headless test runs
(`.claude/skills/run-ocw/drive.sh`, `make smoke`, `make video`/`video-wayland`,
`ocw-desktop --capture`) on the local machine and on the Linux build machine
with a real GPU, plus contact sheets and crops made from them for review.

146 files (108 PNG, 38 MP4), 102.0 MB in total.

- Files are grouped by day (`YYYY-MM-DD/`) and named `HHMMSS-<label>.<ext>`
  (local time the file was created; for archived test runs the start time of
  the run). A PNG and MP4 with the same name prefix belong to the same run
  (the PNG is the last frame).
- Duplicates (identical content) were removed; files identical to the logo
  renders in `artwork/logos/` are not repeated here.
- "Args" are the `ocw-desktop` arguments recorded in the run archive
  (`test-output/index.md`); runs without them were made ad hoc. Paths under
  `data/pilot/` refer to git-ignored, OSM/DEM-derived data (route and world
  package for Liechtenstein).
- Map renders contain OpenStreetMap-derived data:
  © OpenStreetMap contributors (ODbL).
- New archived runs land in the git-ignored `test-output/`; copy the ones
  worth keeping here and add a row below.
- **The media are not in git** (decided 2026-11-12; the history was rewritten
  to remove them, see `release/commit-map.txt`): the day folders
  `YYYY-MM-DD/` are git-ignored and published as one zip per day,
  `ocw-build-history-YYYY-MM-DD.zip`, on the GitHub release
  [`build-history`](https://github.com/newkit/OCW/releases/tag/build-history).
  This index stays in git; unzip a day's file here to make its links work.

| Date | File | What it shows |
|---|---|---|
| 2026-11-11 01:28:29 | [012829-shot1.png](2026-11-11/012829-shot1.png) | First headless screenshot of the test route: own rider, bots, trees, HUD |
| 2026-11-11 01:29:20 | [012920-shot2.png](2026-11-11/012920-shot2.png) | Test route with the console overlay open (command list) |
| 2026-11-11 01:29:28 | [012928-shot3.png](2026-11-11/012928-shot3.png) | Test route, chase camera with bots |
| 2026-11-11 01:34:41 | [013441-drv.png](2026-11-11/013441-drv.png) | Screenshot taken with the run-ocw drive.sh script |
| 2026-11-11 08:24:27 | [082427-after.png](2026-11-11/082427-after.png) | Test route after a rendering change, single rider |
| 2026-11-11 08:25:07 | [082507-make.png](2026-11-11/082507-make.png) | Screenshot from a Makefile run (make smoke) |
| 2026-11-11 08:39:57 | [083957-wl.png](2026-11-11/083957-wl.png) | Smoke screenshot on headless Wayland (weston) |
| 2026-11-11 08:40:04 | [084004-x11.png](2026-11-11/084004-x11.png) | Smoke screenshot on X11 (Xvfb) |
| 2026-11-11 08:43:46 | [084346-smoke-wayland.png](2026-11-11/084346-smoke-wayland.png) | make smoke: Wayland screenshot |
| 2026-11-11 08:43:46 | [084346-smoke-x11.png](2026-11-11/084346-smoke-x11.png) | make smoke: X11 screenshot |
| 2026-11-11 08:45:02 | [084502-ride.mp4](2026-11-11/084502-ride.mp4) | Short ride recording on the test route |
| 2026-11-11 08:45:07 | [084507-ride.png](2026-11-11/084507-ride.png) | Short ride recording on the test route |
| 2026-11-11 08:45:08 | [084508-ride-sheet.png](2026-11-11/084508-ride-sheet.png) | Contact sheet of the ride recording |
| 2026-11-11 08:47:37 | [084737-cage.mp4](2026-11-11/084737-cage.mp4) | Ride recorded inside the cage compositor (Wayland) |
| 2026-11-11 08:47:40 | [084740-cage.png](2026-11-11/084740-cage.png) | Ride recorded inside the cage compositor (Wayland) |
| 2026-11-11 08:47:45 | [084745-cage-sheet.png](2026-11-11/084745-cage-sheet.png) | Contact sheet of the cage recording |
| 2026-11-11 08:48:29 | [084829-cage-fs.mp4](2026-11-11/084829-cage-fs.mp4) | Fullscreen ride recorded inside cage |
| 2026-11-11 08:48:29 | [084829-cage-fs.png](2026-11-11/084829-cage-fs.png) | Fullscreen ride recorded inside cage |
| 2026-11-11 08:48:30 | [084830-cage-fs-frame.png](2026-11-11/084830-cage-fs-frame.png) | Single frame of the fullscreen cage recording |
| 2026-11-11 08:49:28 | [084928-ride-x11.mp4](2026-11-11/084928-ride-x11.mp4) | Ride recording on X11 |
| 2026-11-11 08:49:33 | [084933-ride-x11.png](2026-11-11/084933-ride-x11.png) | Ride recording on X11 |
| 2026-11-11 08:49:51 | [084951-ride-wayland.mp4](2026-11-11/084951-ride-wayland.mp4) | Ride recording on Wayland |
| 2026-11-11 08:49:56 | [084956-ride-wayland.png](2026-11-11/084956-ride-wayland.png) | Ride recording on Wayland |
| 2026-11-11 08:49:58 | [084958-ride-wayland-sheet.png](2026-11-11/084958-ride-wayland-sheet.png) | Contact sheet of the Wayland ride recording |
| 2026-11-11 09:04:13 | [090413-base-fps.png](2026-11-11/090413-base-fps.png) | Baseline view with FPS counter before the terrain work |
| 2026-11-11 09:06:09 | [090609-terrain-top.png](2026-11-11/090609-terrain-top.png) | Road-conformed terrain (spec 4.3), top-down view |
| 2026-11-11 09:09:13 | [090913-terrain-side.png](2026-11-11/090913-terrain-side.png) | Road-conformed terrain, side view |
| 2026-11-11 09:09:23 | [090923-terrain-chase.png](2026-11-11/090923-terrain-chase.png) | Road-conformed terrain, chase camera |
| 2026-11-11 09:09:34 | [090934-terrain-before-after.png](2026-11-11/090934-terrain-before-after.png) | Terrain before/after comparison |
| 2026-11-11 09:09:45 | [090945-terrain-ride.mp4](2026-11-11/090945-terrain-ride.mp4) | Ride over the road-conformed terrain |
| 2026-11-11 09:09:50 | [090950-terrain-ride.png](2026-11-11/090950-terrain-ride.png) | Ride over the road-conformed terrain |
| 2026-11-11 09:20:04 | [092004-cap-hud.png](2026-11-11/092004-cap-hud.png) | HUD crops of consecutive captured frames (speed/time advance per frame) |
| 2026-11-11 09:20:04 | [092004-cap.mp4](2026-11-11/092004-cap.mp4) | Fixed-step capture (--capture) test |
| 2026-11-11 09:21:05 | [092105-smooth-x11.png](2026-11-11/092105-smooth-x11.png) | Smooth fixed-step video (make video), X11 |
| 2026-11-11 09:21:06 | [092106-smooth-x11.mp4](2026-11-11/092106-smooth-x11.mp4) | Smooth fixed-step video (make video), X11 |
| 2026-11-11 09:21:57 | [092157-smooth-wayland.mp4](2026-11-11/092157-smooth-wayland.mp4) | Smooth fixed-step video (make video-wayland), Wayland |
| 2026-11-11 09:21:57 | [092157-smooth-wayland.png](2026-11-11/092157-smooth-wayland.png) | Smooth fixed-step video (make video-wayland), Wayland |
| 2026-11-11 09:22:03 | [092203-wl-shot.png](2026-11-11/092203-wl-shot.png) | Wayland screenshot of the test route with bots |
| 2026-11-11 09:22:09 | [092209-smooth-sheet.png](2026-11-11/092209-smooth-sheet.png) | Contact sheet of the smooth capture videos |
| 2026-11-11 09:30:51 | [093051-a.png](2026-11-11/093051-a.png) | Test route screenshot (drive.sh archive test) (shot, x11, commit c42d787-dirty; args `--cmd teleport 1.2`) |
| 2026-11-11 09:31:08 | [093108-b.mp4](2026-11-11/093108-b.mp4) | Side camera capture (drive.sh archive test) (video-wayland, wayland, commit c42d787-dirty; args `--capture-seconds 2 --cmd camera.mode side`) |
| 2026-11-11 09:31:08 | [093108-b.png](2026-11-11/093108-b.png) | Side camera capture (drive.sh archive test) (video-wayland, wayland, commit c42d787-dirty; args `--capture-seconds 2 --cmd camera.mode side`) |
| 2026-11-11 09:31:14 | [093114-c.png](2026-11-11/093114-c.png) | Test route screenshot (drive.sh archive test) |
| 2026-11-11 10:39:08 | [103908-trial.png](2026-11-11/103908-trial.png) | Trees/terrain spike, 1 km trial run (spike, wayland, commit f0ff0a5-dirty; args `--km 1`) |
| 2026-11-11 10:40:21 | [104021-check.png](2026-11-11/104021-check.png) | Trees/terrain spike, check run at km 6 (spike, wayland, commit f0ff0a5-dirty; args `--km 0.3 --start-km 6`) |
| 2026-11-11 10:55:42 | [105542-bench-instanced-40.png](2026-11-11/105542-bench-instanced-40.png) | Tree benchmark: instanced trees, 40 km/h (spike, wayland, commit f0ff0a5-dirty; args `--km 10 --speed-kmh 40 --tree-mode instanced --start-km 2`) |
| 2026-11-11 11:10:43 | [111043-bench-merged-40.png](2026-11-11/111043-bench-merged-40.png) | Tree benchmark: merged tree meshes, 40 km/h (spike, wayland, commit f0ff0a5-dirty; args `--km 10 --speed-kmh 40 --tree-mode merged --start-km 2`) |
| 2026-11-11 11:14:45 | [111445-bench-instanced-150.png](2026-11-11/111445-bench-instanced-150.png) | Tree benchmark: instanced trees, 150 km/h (spike, wayland, commit f0ff0a5-dirty; args `--km 10 --speed-kmh 150 --tree-mode instanced --start-km 2`) |
| 2026-11-11 11:18:46 | [111846-bench-merged-150.png](2026-11-11/111846-bench-merged-150.png) | Tree benchmark: merged tree meshes, 150 km/h (spike, wayland, commit f0ff0a5-dirty; args `--km 10 --speed-kmh 150 --tree-mode merged --start-km 2`) |
| 2026-11-11 11:25:06 | [112506-spike-splat.mp4](2026-11-11/112506-spike-splat.mp4) | Spike video: splat-textured terrain, merged trees (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --tree-mode merged --start-km 6`) |
| 2026-11-11 11:25:06 | [112506-spike-splat.png](2026-11-11/112506-spike-splat.png) | Spike video: splat-textured terrain, merged trees (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --tree-mode merged --start-km 6`) |
| 2026-11-11 11:25:13 | [112513-splat-sheet.png](2026-11-11/112513-splat-sheet.png) | Contact sheet of the spike-splat video |
| 2026-11-11 11:28:46 | [112846-short-merged-vertex.png](2026-11-11/112846-short-merged-vertex.png) | Spike: merged trees, vertex-coloured terrain (spike, wayland, commit f0ff0a5-dirty; args `--tree-mode merged --terrain vertex --start-km 2`) |
| 2026-11-11 11:31:48 | [113148-short-merged-splat.png](2026-11-11/113148-short-merged-splat.png) | Spike: merged trees, splat terrain (spike, wayland, commit f0ff0a5-dirty; args `--tree-mode merged --terrain splat --start-km 2`) |
| 2026-11-11 11:34:49 | [113449-short-instanced-splat.png](2026-11-11/113449-short-instanced-splat.png) | Spike: instanced trees, splat terrain (spike, wayland, commit f0ff0a5-dirty; args `--tree-mode instanced --terrain splat --start-km 2`) |
| 2026-11-11 11:36:50 | [113650-stress-merged-splat.png](2026-11-11/113650-stress-merged-splat.png) | Spike stress run at 150 km/h (spike, wayland, commit f0ff0a5-dirty; args `--tree-mode merged --terrain splat --km 5 --speed-kmh 150 --start-km 2`) |
| 2026-11-11 11:37:24 | [113724-spike-winding.mp4](2026-11-11/113724-spike-winding.mp4) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:37:24 | [113724-spike-winding.png](2026-11-11/113724-spike-winding.png) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:37:56 | [113756-spike-winding.mp4](2026-11-11/113756-spike-winding.mp4) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:37:56 | [113756-spike-winding.png](2026-11-11/113756-spike-winding.png) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:37:56 | [113756-winding-first.png](2026-11-11/113756-winding-first.png) | First frame of the spike-winding video (sky only, scene not yet loaded) |
| 2026-11-11 11:38:31 | [113831-spike-winding.mp4](2026-11-11/113831-spike-winding.mp4) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:38:31 | [113831-spike-winding.png](2026-11-11/113831-spike-winding.png) | Spike video on a winding road section (spike-video, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --tree-mode merged --start-km 9.3`) |
| 2026-11-11 11:38:33 | [113833-winding-sheet.png](2026-11-11/113833-winding-sheet.png) | Contact sheet of the spike-winding video |
| 2026-11-11 12:17:47 | [121747-kickr.png](2026-11-11/121747-kickr.png) | devices.list with a real Wahoo KICKR (shot-wayland, wayland, commit nogit; args `--devices-file /tmp/ocw/kickr/devices.toml --cmd devices.list`) |
| 2026-11-11 13:24:16 | [132416-hills-chase.mp4](2026-11-11/132416-hills-chase.mp4) | Hilly test route, chase camera (video-wayland, wayland, commit nogit; args `--capture-seconds 8 --virtual --power 330 --cmd teleport 0.55`) |
| 2026-11-11 13:24:16 | [132416-hills-chase.png](2026-11-11/132416-hills-chase.png) | Hilly test route, chase camera (video-wayland, wayland, commit nogit; args `--capture-seconds 8 --virtual --power 330 --cmd teleport 0.55`) |
| 2026-11-11 13:24:25 | [132425-hills-orbit.mp4](2026-11-11/132425-hills-orbit.mp4) | Hilly test route, orbit camera (video-wayland, wayland, commit nogit; args `--capture-seconds 8 --virtual --power 330 --cmd teleport 0.55 --cmd camera.orbit 70 15 8`) |
| 2026-11-11 13:24:25 | [132425-hills-orbit.png](2026-11-11/132425-hills-orbit.png) | Hilly test route, orbit camera (video-wayland, wayland, commit nogit; args `--capture-seconds 8 --virtual --power 330 --cmd teleport 0.55 --cmd camera.orbit 70 15 8`) |
| 2026-11-11 13:24:32 | [132432-hills-sheet.png](2026-11-11/132432-hills-sheet.png) | Contact sheet of the hills videos |
| 2026-11-11 13:24:32 | [132432-legs.png](2026-11-11/132432-legs.png) | Crops of the rider pedalling animation (legs) |
| 2026-11-11 13:25:04 | [132504-hills-climb.mp4](2026-11-11/132504-hills-climb.mp4) | Climb with trainer difficulty 100 (video-wayland, wayland, commit nogit; args `--capture-seconds 10 --virtual --power 330 --cmd teleport 3.70 --cmd trainer.difficulty 100`) |
| 2026-11-11 13:25:04 | [132504-hills-climb.png](2026-11-11/132504-hills-climb.png) | Climb with trainer difficulty 100 (video-wayland, wayland, commit nogit; args `--capture-seconds 10 --virtual --power 330 --cmd teleport 3.70 --cmd trainer.difficulty 100`) |
| 2026-11-11 13:25:05 | [132505-climb.png](2026-11-11/132505-climb.png) | Crop from the hills-climb video |
| 2026-11-11 13:52:57 | [135257-hud.png](2026-11-11/135257-hud.png) | HUD on a climb (shot-wayland, wayland, commit nogit; args `--virtual --power 330 --cmd teleport 3.75`) |
| 2026-11-11 14:07:19 | [140719-cap.png](2026-11-11/140719-cap.png) | Fixed-step capture (--capture) test (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 200 --cmd camera.orbit 150 12 3`) |
| 2026-11-11 14:14:59 | [141459-steer-sheet.png](2026-11-11/141459-steer-sheet.png) | Contact sheet of the steering video |
| 2026-11-11 14:14:59 | [141459-steer.mp4](2026-11-11/141459-steer.mp4) | Steering via rider.steer, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 6 --virtual --power 220 --capture-warmup 0.3 --cmd camera.orbit 0 55 14 --cmd rider.steer 2.5`) |
| 2026-11-11 14:14:59 | [141459-steer.png](2026-11-11/141459-steer.png) | Steering via rider.steer, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 6 --virtual --power 220 --capture-warmup 0.3 --cmd camera.orbit 0 55 14 --cmd rider.steer 2.5`) |
| 2026-11-11 14:15:10 | [141510-steer-trimmed.mp4](2026-11-11/141510-steer-trimmed.mp4) | Steering video, trimmed |
| 2026-11-11 14:22:50 | [142250-mon.png](2026-11-11/142250-mon.png) | Window placed on monitor 0 (--monitor) (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --monitor 0`) |
| 2026-11-11 14:38:03 | [143803-graph.png](2026-11-11/143803-graph.png) | Liechtenstein road graph plotted from world-builder output |
| 2026-11-11 14:43:32 | [144332-grade-small.png](2026-11-11/144332-grade-small.png) | Road graph coloured by gradient (smaller render) |
| 2026-11-11 14:43:32 | [144332-grade.png](2026-11-11/144332-grade.png) | Liechtenstein road graph coloured by gradient |
| 2026-11-11 14:59:20 | [145920-lie1.png](2026-11-11/145920-lie1.png) | Liechtenstein pilot route (Vaduz-Malbun) (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 280 --route-file data/pilot/vaduz-malbun.route --cmd teleport 3.2`) |
| 2026-11-11 14:59:45 | [145945-lie-a.mp4](2026-11-11/145945-lie-a.mp4) | Liechtenstein route, km 3.2 (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 3.2`) |
| 2026-11-11 14:59:45 | [145945-lie-a.png](2026-11-11/145945-lie-a.png) | Liechtenstein route, km 3.2 (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 3.2`) |
| 2026-11-11 14:59:54 | [145954-lie-b.mp4](2026-11-11/145954-lie-b.mp4) | Liechtenstein route, km 5.6, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 5.6 --cmd camera.orbit 165 22 28`) |
| 2026-11-11 14:59:54 | [145954-lie-b.png](2026-11-11/145954-lie-b.png) | Liechtenstein route, km 5.6, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 5.6 --cmd camera.orbit 165 22 28`) |
| 2026-11-11 15:02:09 | [150209-lie-b.mp4](2026-11-11/150209-lie-b.mp4) | Liechtenstein route, km 5.6, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 5.6 --cmd camera.orbit 165 22 28`) |
| 2026-11-11 15:02:09 | [150209-lie-b.png](2026-11-11/150209-lie-b.png) | Liechtenstein route, km 5.6, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 5.6 --cmd camera.orbit 165 22 28`) |
| 2026-11-11 15:02:18 | [150218-lie-c.mp4](2026-11-11/150218-lie-c.mp4) | Liechtenstein route, km 11.5, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 11.5 --cmd camera.orbit 30 18 14`) |
| 2026-11-11 15:02:18 | [150218-lie-c.png](2026-11-11/150218-lie-c.png) | Liechtenstein route, km 11.5, orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 8 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --cmd teleport 11.5 --cmd camera.orbit 30 18 14`) |
| 2026-11-11 15:02:18 | [150218-liechtenstein.mp4](2026-11-11/150218-liechtenstein.mp4) | Liechtenstein pilot route (Vaduz-Malbun) video |
| 2026-11-11 15:02:19 | [150219-lie-sheet.png](2026-11-11/150219-lie-sheet.png) | Contact sheet of the Liechtenstein route videos |
| 2026-11-11 15:09:23 | [150923-arch.png](2026-11-11/150923-arch.png) | Orbit camera close-up of the rider (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --cmd camera.orbit 25 10 14`) |
| 2026-11-11 15:16:11 | [151611-pkg1.png](2026-11-11/151611-pkg1.png) | Liechtenstein world package (.ocwpkg), km 3.2 (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 280 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.2`) |
| 2026-11-11 15:16:24 | [151624-pkg2.png](2026-11-11/151624-pkg2.png) | Liechtenstein world package, high orbit camera (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 280 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 1.2 --cmd camera.orbit 200 45 75`) |
| 2026-11-11 15:16:25 | [151625-pkg-sheet.png](2026-11-11/151625-pkg-sheet.png) | Contact sheet of the world-package videos |
| 2026-11-11 15:16:49 | [151649-pk-a.mp4](2026-11-11/151649-pk-a.mp4) | World package ride video, km 3.2 (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 7 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.2`) |
| 2026-11-11 15:16:49 | [151649-pk-a.png](2026-11-11/151649-pk-a.png) | World package ride video, km 3.2 (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 7 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.2`) |
| 2026-11-11 15:16:57 | [151657-package.mp4](2026-11-11/151657-package.mp4) | Ride with the Liechtenstein world package (.ocwpkg) |
| 2026-11-11 15:16:57 | [151657-pk-b.mp4](2026-11-11/151657-pk-b.mp4) | World package ride video, high orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 7 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 1.2 --cmd camera.orbit 200 40 60`) |
| 2026-11-11 15:16:57 | [151657-pk-b.png](2026-11-11/151657-pk-b.png) | World package ride video, high orbit camera (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 7 --virtual --power 300 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 1.2 --cmd camera.orbit 200 40 60`) |
| 2026-11-11 15:19:46 | [151946-cam.png](2026-11-11/151946-cam.png) | Low camera near the road (terrain clipping check) (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 250 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.25 --cmd camera.orbit 270 -3 25`) |
| 2026-11-11 15:24:47 | [152447-art-top.png](2026-11-11/152447-art-top.png) | Terrain artefact investigation, top view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.25 --cmd camera.orbit 270 80 22`) |
| 2026-11-11 15:24:54 | [152454-art-side.png](2026-11-11/152454-art-side.png) | Terrain artefact investigation, side view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.25 --cmd camera.orbit 270 6 14`) |
| 2026-11-11 15:24:55 | [152455-art-before.png](2026-11-11/152455-art-before.png) | Terrain artefact near km 3.25, crops before the fix |
| 2026-11-11 15:25:18 | [152518-art-a.png](2026-11-11/152518-art-a.png) | Terrain artefact, low view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.28 --cmd camera.orbit 270 -3 25`) |
| 2026-11-11 15:25:25 | [152525-art-b.png](2026-11-11/152525-art-b.png) | Terrain artefact, high view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.28 --cmd camera.orbit 300 35 18`) |
| 2026-11-11 15:25:26 | [152526-art-before2.png](2026-11-11/152526-art-before2.png) | Terrain artefact, further crops before the fix |
| 2026-11-11 15:33:44 | [153344-fix-a.png](2026-11-11/153344-fix-a.png) | Terrain artefact after the fix, low view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.28 --cmd camera.orbit 270 -3 25`) |
| 2026-11-11 15:33:52 | [153352-art-after.png](2026-11-11/153352-art-after.png) | Terrain artefact crops after the fix |
| 2026-11-11 15:33:52 | [153352-art-before-after.png](2026-11-11/153352-art-before-after.png) | Terrain artefact before/after comparison |
| 2026-11-11 15:33:52 | [153352-fix-b.png](2026-11-11/153352-fix-b.png) | Terrain artefact after the fix, high view (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 3.28 --cmd camera.orbit 300 35 18`) |
| 2026-11-11 16:17:47 | [161747-gradient-before-after.png](2026-11-11/161747-gradient-before-after.png) | Gradient display before/after comparison |
| 2026-11-11 16:20:44 | [162044-lean.mp4](2026-11-11/162044-lean.mp4) | Leaning in curves with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --virtual --power 420 --bots 5 --cmd camera.orbit 200 12 9`) |
| 2026-11-11 16:20:44 | [162044-lean.png](2026-11-11/162044-lean.png) | Leaning in curves with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --virtual --power 420 --bots 5 --cmd camera.orbit 200 12 9`) |
| 2026-11-11 16:21:21 | [162121-lean-sheet.png](2026-11-11/162121-lean-sheet.png) | Contact sheet of the lean video |
| 2026-11-11 16:21:21 | [162121-lean.mp4](2026-11-11/162121-lean.mp4) | Leaning in curves with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --virtual --power 600 --bots 5 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 9.52 --cmd camera.orbit 180 8 7`) |
| 2026-11-11 16:21:21 | [162121-lean.png](2026-11-11/162121-lean.png) | Leaning in curves with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 10 --virtual --power 600 --bots 5 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 9.52 --cmd camera.orbit 180 8 7`) |
| 2026-11-11 16:34:09 | [163409-steer-sheet.png](2026-11-11/163409-steer-sheet.png) | Contact sheet of the steering video |
| 2026-11-11 16:34:09 | [163409-steer.mp4](2026-11-11/163409-steer.mp4) | Steering by leaning (rider.lean) with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 11 --virtual --power 380 --bots 3 --at 4=rider.lean 14 --at 5.6=rider.lean 0 --at 8=rider.lean -14 --at 10=rider.lean 0 --cmd camera.orbit 0 14 11`) |
| 2026-11-11 16:34:09 | [163409-steer.png](2026-11-11/163409-steer.png) | Steering by leaning (rider.lean) with bots (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 11 --virtual --power 380 --bots 3 --at 4=rider.lean 14 --at 5.6=rider.lean 0 --at 8=rider.lean -14 --at 10=rider.lean 0 --cmd camera.orbit 0 14 11`) |
| 2026-11-11 16:40:09 | [164009-rs.png](2026-11-11/164009-rs.png) | Random start position (--random-start) (shot-wayland, wayland, commit f0ff0a5-dirty; args `--virtual --power 250 --random-start true`) |
| 2026-11-11 16:47:37 | [164737-modes.mp4](2026-11-11/164737-modes.mp4) | Riding modes: autodrive off/on, wild mode with lean (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 18 --virtual --power 400 --bots 2 --cmd teleport 1.40 --cmd camera.orbit 0 22 14 --at 6=rider.autodrive off --at 11=rider.autodrive on --at 15=rider.wild on --at 15=rider.lean 14 --at 17.5=rider.lean 0`) |
| 2026-11-11 16:47:37 | [164737-modes.png](2026-11-11/164737-modes.png) | Riding modes: autodrive off/on, wild mode with lean (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 18 --virtual --power 400 --bots 2 --cmd teleport 1.40 --cmd camera.orbit 0 22 14 --at 6=rider.autodrive off --at 11=rider.autodrive on --at 15=rider.wild on --at 15=rider.lean 14 --at 17.5=rider.lean 0`) |
| 2026-11-11 16:47:38 | [164738-modes-sheet.png](2026-11-11/164738-modes-sheet.png) | Contact sheet of the riding-modes video |
| 2026-11-11 16:56:22 | [165622-wildcam.mp4](2026-11-11/165622-wildcam.mp4) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 16:56:22 | [165622-wildcam.png](2026-11-11/165622-wildcam.png) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 16:58:52 | [165852-wildcam-before-sheet.png](2026-11-11/165852-wildcam-before-sheet.png) | Contact sheet of the wild-mode camera before the fix |
| 2026-11-11 16:59:19 | [165919-wildcam.mp4](2026-11-11/165919-wildcam.mp4) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 16:59:19 | [165919-wildcam.png](2026-11-11/165919-wildcam.png) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 16:59:20 | [165920-wildcam-sheet.png](2026-11-11/165920-wildcam-sheet.png) | Contact sheet of the wild-mode camera after the fix |
| 2026-11-11 16:59:56 | [165956-dbg.mp4](2026-11-11/165956-dbg.mp4) | Wild mode camera, debug run (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 16:59:56 | [165956-dbg.png](2026-11-11/165956-dbg.png) | Wild mode camera, debug run (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:00:45 | [170045-wildcam.mp4](2026-11-11/170045-wildcam.mp4) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:00:45 | [170045-wildcam.png](2026-11-11/170045-wildcam.png) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:01:42 | [170142-wildcam.mp4](2026-11-11/170142-wildcam.mp4) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:01:42 | [170142-wildcam.png](2026-11-11/170142-wildcam.png) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:03:25 | [170325-wildcam-end.png](2026-11-11/170325-wildcam-end.png) | Last frame of the wild-mode camera video |
| 2026-11-11 17:03:25 | [170325-wildcam.mp4](2026-11-11/170325-wildcam.mp4) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:03:25 | [170325-wildcam.png](2026-11-11/170325-wildcam.png) | Wild mode (free steering) camera on the Liechtenstein route (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 14 --virtual --power 450 --bots 1 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 6.2 --at 4=rider.wild on --at 4=rider.lean 12 --at 6=rider.lean 0 --at 10=rider.lean -16 --at 12.5=rider.lean 0`) |
| 2026-11-11 17:04:48 | [170448-bump.mp4](2026-11-11/170448-bump.mp4) | Wild mode: riding off the road over terrain (bump) (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 12 --virtual --power 350 --bots 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 2.3 --cmd camera.orbit 270 4 5 --at 3.5=rider.wild on --at 3.5=rider.lean 10 --at 5=rider.lean 0`) |
| 2026-11-11 17:04:48 | [170448-bump.png](2026-11-11/170448-bump.png) | Wild mode: riding off the road over terrain (bump) (video-wayland, wayland, commit f0ff0a5-dirty; args `--capture-seconds 12 --virtual --power 350 --bots 0 --route-file data/pilot/vaduz-malbun.route --world data/pilot/liechtenstein.ocwpkg --cmd teleport 2.3 --cmd camera.orbit 270 4 5 --at 3.5=rider.wild on --at 3.5=rider.lean 10 --at 5=rider.lean 0`) |
| 2026-11-11 17:04:49 | [170449-bump-sheet.png](2026-11-11/170449-bump-sheet.png) | Contact sheet of the bump video |

## 2026-11-12

The scene renders up to 09:23 were made before the Phase 4 models were merged (earlier procedural riders); the renders from 10:16 on are from main c0216f7 with the new models and the six bug fixes.

| Date | File | What it shows |
|---|---|---|
| 2026-11-12 07:54:20 | [075420-audio-ride-sounds.wav](2026-11-12/075420-audio-ride-sounds.wav) | Game audio (M5): 20 s scripted demo ride with all synthesised sounds – wind, tyres by surface, freewheel ticks, chain, menu, countdown, finish and workout cues (48 kHz stereo WAV) |
| 2026-11-12 07:56:54 | [075654-audio-settings-page.png](2026-11-12/075654-audio-settings-page.png) | In-game menu: Settings -> Audio page (master, effects, music, voice volumes, mute, output device) |
| 2026-11-12 08:06:42 | [080642-models-side.png](2026-11-12/080642-models-side.png) | Phase 4 models: close side view, seated climbing on the tops (generated glTF bike and rider) |
| 2026-11-12 08:06:53 | [080653-models-group.png](2026-11-12/080653-models-group.png) | Phase 4 models: group from the front with different bikes, kits and helmets |
| 2026-11-12 08:07:39 | [080739-models-sprint.png](2026-11-12/080739-models-sprint.png) | Phase 4 models: sprint at 1100 W, out of the saddle |
| 2026-11-12 08:07:52 | [080752-models-freewheel.png](2026-11-12/080752-models-freewheel.png) | Phase 4 models: freewheeling with level cranks; aero bike with deep rims behind |
| 2026-11-12 08:08:05 | [080805-models-bikes.png](2026-11-12/080805-models-bikes.png) | Phase 4 models: group from above, far riders on the far model (LOD at 40 m) |
| 2026-11-12 08:08:11 | [080811-models-ride.mp4](2026-11-12/080811-models-ride.mp4) | Phase 4 models: 4 s pedalling close-up (wheels, cranks and legs animated) |
| 2026-11-12 08:25:51 | [082551-models-mtb.png](2026-11-12/082551-models-mtb.png) | Phase 4 models: mountain bike close-up (suspension fork, flat bar, knobby tyres) |
| 2026-11-12 08:36:49 | [083649-models-tt.png](2026-11-12/083649-models-tt.png) | Phase 4 models: time-trial rider on the extensions |
| 2026-11-12 08:44:11 | [084411-voice-hud.png](2026-11-12/084411-voice-hud.png) | Voice chat (Phase 6): HUD indicator bottom left – microphone muted (M), speaker on, "voice failed, retrying" without a LiveKit server (voice branch, before the Phase 4 models) |
| 2026-11-12 09:07:05 | [090705-testroute-hills.png](2026-11-12/090705-testroute-hills.png) | Built-in Hills test route: own rider and bots in the chase camera, HUD and elevation profile (scene `testroute-hills`) |
| 2026-11-12 09:07:12 | [090712-console-overlay.png](2026-11-12/090712-console-overlay.png) | Console overlay over the Hills test route: the command list from `help` (scene `console-overlay`) |
| 2026-11-12 09:07:27 | [090727-workout-erg.png](2026-11-12/090727-workout-erg.png) | Bundled sweet-spot workout in ERG mode on the Loop test route: workout HUD panel with target, step, time left and zone-coloured profile (scene `workout-erg`) |
| 2026-11-12 09:08:19 | [090819-race-results.png](2026-11-12/090819-race-results.png) | Local race over 0.4 km on the Loop test route at 700 W: provisional results panel after your finish (scene `race-results`) |
| 2026-11-12 09:09:16 | [090916-tt-ghost.png](2026-11-12/090916-tt-ghost.png) | Time trial on the Loop test route, second attempt: your translucent ghost from the first run rides beside you, ghost gap in the event HUD (scene `tt-ghost`) |
| 2026-11-12 09:09:21 | [090921-menu-main.png](2026-11-12/090921-menu-main.png) | In-game menu (Esc), top page over the Hills test route (scene `menu-main`) |
| 2026-11-12 09:09:28 | [090928-menu-settings.png](2026-11-12/090928-menu-settings.png) | In-game menu: Settings page (Riders, Controls, Trainer, Display, Strava) (scene `menu-settings`) |
| 2026-11-12 09:09:36 | [090936-menu-settings-display.png](2026-11-12/090936-menu-settings-display.png) | In-game menu: Settings -> Display page with its values (HUD, units, FPS, camera, fullscreen) (scene `menu-settings-display`) |
| 2026-11-12 09:09:43 | [090943-menu-workshop-bike.png](2026-11-12/090943-menu-workshop-bike.png) | In-game menu: Workshop -> Bike page of a rider profile (gravel bike, colours, kit) with the rider behind it (scene `menu-workshop-bike`) |
| 2026-11-12 09:09:50 | [090950-menu-workouts.png](2026-11-12/090950-menu-workouts.png) | In-game menu: Ride -> Workouts list (bundled examples) (scene `menu-workouts`) |
| 2026-11-12 09:09:56 | [090956-menu-ride.png](2026-11-12/090956-menu-ride.png) | In-game menu: Ride page (local race, group ride, time trial, change ride, workouts) (scene `menu-ride`) |
| 2026-11-12 09:10:03 | [091003-route-start-formation.png](2026-11-12/091003-route-start-formation.png) | Vaduz-Malbun route in Vaduz: start formation of a local race with 8 bots during the countdown, seen from the front (scene `route-start-formation`) |
| 2026-11-12 09:10:12 | [091012-race-start-video.mp4](2026-11-12/091012-race-start-video.mp4) | Video: countdown (3-2-1, GO!) and start of a local race with 8 bots in Vaduz, orbiting camera in front of the field (scene `race-start-video`) |
| 2026-11-12 09:10:29 | [091029-camera-chase.png](2026-11-12/091029-camera-chase.png) | Chase camera on the Vaduz-Malbun route leaving Vaduz: bots passing the own rider between the houses (scene `camera-chase`) |
| 2026-11-12 09:10:47 | [091047-camera-side.png](2026-11-12/091047-camera-side.png) | Side camera on the Vaduz-Malbun route in Vaduz: rider and bots from the side of the road (scene `camera-side`) |
| 2026-11-12 09:11:04 | [091104-camera-top.png](2026-11-12/091104-camera-top.png) | Top camera on the Vaduz-Malbun route in Vaduz: looking down on the riders, the road and the houses (scene `camera-top`) |
| 2026-11-12 09:11:16 | [091116-camera-close.png](2026-11-12/091116-camera-close.png) | Close orbit camera (3 m, front-left) on the own rider: bike, kit and helmet in detail (scene `camera-close`) |
| 2026-11-12 09:11:29 | [091129-orbit-video.mp4](2026-11-12/091129-orbit-video.mp4) | Video: the chase camera orbits once around the rider climbing out of Vaduz (camera.orbit stepped by --at) (scene `orbit-video`) |
| 2026-11-12 09:11:39 | [091139-malbun-climb.png](2026-11-12/091139-malbun-climb.png) | Steep climb on the Malbun road (km 10, about 11 %): gradient HUD with the 100 m average and the gradient-coloured elevation profile (scene `malbun-climb`) |
| 2026-11-12 09:11:50 | [091150-malbun-climb-video.mp4](2026-11-12/091150-malbun-climb-video.mp4) | Video: climbing the Malbun road (km 10) in the side camera (scene `malbun-climb-video`) |
| 2026-11-12 09:12:01 | [091201-malbun-arrival.png](2026-11-12/091201-malbun-arrival.png) | Arrival in Malbun (km 13.9, 1600 m): chalets and the mountains around the valley head (scene `malbun-arrival`) |
| 2026-11-12 09:12:10 | [091210-tunnel-entry.png](2026-11-12/091210-tunnel-entry.png) | Steg tunnel portal on the Vaduz-Malbun route (km 8.77, 1265 m): the road disappears into the hillside (scene `tunnel-entry`) |
| 2026-11-12 09:12:20 | [091220-tunnel-inside.png](2026-11-12/091220-tunnel-inside.png) | Inside the Steg tunnel (km 8.9): tunnel tube and the close tunnel camera, exit light ahead (scene `tunnel-inside`) |
| 2026-11-12 09:12:37 | [091237-tunnel-video.mp4](2026-11-12/091237-tunnel-video.mp4) | Video: riding into the Steg tunnel at km 8.75 at 800 W (15 %), the camera blends into the tunnel camera (scene `tunnel-video`) |
| 2026-11-12 09:13:03 | [091303-freeride-junction.png](2026-11-12/091303-freeride-junction.png) | Free ride junction HUD in Vaduz: exits at their real angles with street name and distance, the chosen (left) exit locked in (scene `freeride-junction`) |
| 2026-11-12 09:13:12 | [091312-rhine-water.png](2026-11-12/091312-rhine-water.png) | The Rhine at Vaduz from above: water surface, the dam track, meadows and land use (scene `rhine-water`) |
| 2026-11-12 09:13:25 | [091325-far-view.png](2026-11-12/091325-far-view.png) | Long-distance view from the hairpins above Vaduz (km 3, 725 m) over the Rhine valley: far terrain tiles, forests, Vaduz and the mountains beyond (scene `far-view`) |
| 2026-11-12 09:13:31 | [091331-menu-navi.png](2026-11-12/091331-menu-navi.png) | In-game menu: Navi page (routes, saved rides, GPX import, Strava) (scene `menu-navi`) |
| 2026-11-12 09:23:01 | [092301-vaduz-buildings.png](2026-11-12/092301-vaduz-buildings.png) | Free ride in Vaduz centre: extruded OSM buildings with flat and gable roofs along the street, orbit camera from the front (scene `vaduz-buildings`) |
| 2026-11-12 09:23:10 | [092310-vaduz-video.mp4](2026-11-12/092310-vaduz-video.mp4) | Video: free ride through Vaduz centre among the buildings (raised chase camera), junction HUD counting down (scene `vaduz-video`) |
| 2026-11-12 10:17:10 | [101710-testroute-hills.png](2026-11-12/101710-testroute-hills.png) | Built-in Hills test route: own rider and bots in the chase camera, HUD and elevation profile (scene `testroute-hills`) |
| 2026-11-12 10:17:17 | [101717-console-overlay.png](2026-11-12/101717-console-overlay.png) | Console overlay over the Hills test route: the command list from `help` (scene `console-overlay`) |
| 2026-11-12 10:17:33 | [101733-workout-erg.png](2026-11-12/101733-workout-erg.png) | Bundled sweet-spot workout in ERG mode on the Loop test route: workout HUD panel with target, step, time left and zone-coloured profile (scene `workout-erg`) |
| 2026-11-12 10:18:24 | [101824-race-results.png](2026-11-12/101824-race-results.png) | Local race over 0.4 km on the Loop test route at 700 W: provisional results panel after your finish (scene `race-results`) |
| 2026-11-12 10:19:21 | [101921-tt-ghost.png](2026-11-12/101921-tt-ghost.png) | Time trial on the Loop test route, second attempt: your translucent ghost from the first run rides beside you, ghost gap in the event HUD (scene `tt-ghost`) |
| 2026-11-12 10:19:26 | [101926-menu-main.png](2026-11-12/101926-menu-main.png) | In-game menu (Esc), top page over the Hills test route (scene `menu-main`) |
| 2026-11-12 10:19:33 | [101933-menu-settings.png](2026-11-12/101933-menu-settings.png) | In-game menu: Settings page (Riders, Controls, Trainer, Display, Strava) (scene `menu-settings`) |
| 2026-11-12 10:19:41 | [101941-menu-settings-display.png](2026-11-12/101941-menu-settings-display.png) | In-game menu: Settings -> Display page with its values (HUD, units, FPS, camera, fullscreen) (scene `menu-settings-display`) |
| 2026-11-12 10:19:48 | [101948-menu-workshop-bike.png](2026-11-12/101948-menu-workshop-bike.png) | In-game menu: Workshop -> Bike page of a rider profile (gravel bike, colours, kit) with the rider behind it (scene `menu-workshop-bike`) |
| 2026-11-12 10:19:55 | [101955-menu-workouts.png](2026-11-12/101955-menu-workouts.png) | In-game menu: Ride -> Workouts list (bundled examples) (scene `menu-workouts`) |
| 2026-11-12 10:20:01 | [102001-menu-ride.png](2026-11-12/102001-menu-ride.png) | In-game menu: Ride page (local race, group ride, time trial, change ride, workouts) (scene `menu-ride`) |
| 2026-11-12 10:20:08 | [102008-route-start-formation.png](2026-11-12/102008-route-start-formation.png) | Vaduz-Malbun route in Vaduz: start formation of a local race with 8 bots during the countdown, seen from the front (scene `route-start-formation`) |
| 2026-11-12 10:20:18 | [102018-race-start-video.mp4](2026-11-12/102018-race-start-video.mp4) | Video: countdown (3-2-1, GO!) and start of a local race with 8 bots in Vaduz, orbiting camera in front of the field (scene `race-start-video`) |
| 2026-11-12 10:20:35 | [102035-camera-chase.png](2026-11-12/102035-camera-chase.png) | Chase camera on the Vaduz-Malbun route leaving Vaduz: bots passing the own rider between the houses (scene `camera-chase`) |
| 2026-11-12 10:20:52 | [102052-camera-side.png](2026-11-12/102052-camera-side.png) | Side camera on the Vaduz-Malbun route in Vaduz: rider and bots from the side of the road (scene `camera-side`) |
| 2026-11-12 10:21:10 | [102110-camera-top.png](2026-11-12/102110-camera-top.png) | Top camera on the Vaduz-Malbun route in Vaduz: looking down on the riders, the road and the houses (scene `camera-top`) |
| 2026-11-12 10:21:18 | [102118-camera-close.png](2026-11-12/102118-camera-close.png) | Close orbit camera (3 m, front-left) on the own rider: bike, kit and helmet in detail (scene `camera-close`) |
| 2026-11-12 10:21:27 | [102127-orbit-video.mp4](2026-11-12/102127-orbit-video.mp4) | Video: the chase camera orbits once around the rider climbing out of Vaduz (camera.orbit stepped by --at) (scene `orbit-video`) |
| 2026-11-12 10:21:38 | [102138-malbun-climb.png](2026-11-12/102138-malbun-climb.png) | Steep climb on the Malbun road (km 10, about 11 %): gradient HUD with the 100 m average and the gradient-coloured elevation profile (scene `malbun-climb`) |
| 2026-11-12 10:21:46 | [102146-malbun-climb-video.mp4](2026-11-12/102146-malbun-climb-video.mp4) | Video: climbing the Malbun road (km 10) in the side camera (scene `malbun-climb-video`) |
| 2026-11-12 10:21:56 | [102156-malbun-arrival.png](2026-11-12/102156-malbun-arrival.png) | Arrival in Malbun (km 13.9, 1600 m): chalets and the mountains around the valley head (scene `malbun-arrival`) |
| 2026-11-12 10:22:05 | [102205-tunnel-entry.png](2026-11-12/102205-tunnel-entry.png) | Steg tunnel portal on the Vaduz-Malbun route (km 8.77, 1265 m): the road disappears into the hillside (scene `tunnel-entry`) |
| 2026-11-12 10:22:14 | [102214-tunnel-inside.png](2026-11-12/102214-tunnel-inside.png) | Inside the Steg tunnel (km 8.9): tunnel tube and the close tunnel camera, exit light ahead (scene `tunnel-inside`) |
| 2026-11-12 10:22:26 | [102226-tunnel-video.mp4](2026-11-12/102226-tunnel-video.mp4) | Video: riding into the Steg tunnel at km 8.75 at 800 W (15 %), the camera blends into the tunnel camera (scene `tunnel-video`) |
| 2026-11-12 10:22:34 | [102234-vaduz-buildings.png](2026-11-12/102234-vaduz-buildings.png) | Free ride in Vaduz centre: extruded OSM buildings with flat and gable roofs along the street, orbit camera from the front (scene `vaduz-buildings`) |
| 2026-11-12 10:22:42 | [102242-vaduz-video.mp4](2026-11-12/102242-vaduz-video.mp4) | Video: free ride through Vaduz centre among the buildings (raised chase camera), junction HUD counting down (scene `vaduz-video`) |
| 2026-11-12 10:22:49 | [102249-freeride-junction.png](2026-11-12/102249-freeride-junction.png) | Free ride junction HUD in Vaduz: exits at their real angles with street name and distance, the chosen (left) exit locked in (scene `freeride-junction`) |
| 2026-11-12 10:22:57 | [102257-rhine-water.png](2026-11-12/102257-rhine-water.png) | The Rhine at Vaduz from above: water surface, the dam track, meadows and land use (scene `rhine-water`) |
| 2026-11-12 10:23:11 | [102311-far-view.png](2026-11-12/102311-far-view.png) | Long-distance view from the hairpins above Vaduz (km 3, 725 m) over the Rhine valley: far terrain tiles, forests, Vaduz and the mountains beyond (scene `far-view`) |
| 2026-11-12 10:23:17 | [102317-menu-navi.png](2026-11-12/102317-menu-navi.png) | In-game menu: Navi page (routes, saved rides, GPX import, Strava) (scene `menu-navi`) |
| 2026-11-12 10:23:24 | [102324-event-bots.png](2026-11-12/102324-event-bots.png) | Local group ride with 6 bots on the Hills test route, orbit camera from the front: every event bot on its own bike and kit (scene `event-bots`) |
| 2026-11-12 10:23:30 | [102330-teleport-video.mp4](2026-11-12/102330-teleport-video.mp4) | Video: teleport from Vaduz to km 5 of the Vaduz-Malbun route at 4 s; the near terrain and road tiles are there in the first frame after the jump (scene `teleport-video`) |
| 2026-11-12 10:23:39 | [102339-route-end-teleport.png](2026-11-12/102339-route-end-teleport.png) | teleport 15 on the 13.3 km Vaduz-Malbun route: moved to its end in Malbun (speed 0), the console reports it (scene `route-end-teleport`) |
| 2026-11-12 10:23:49 | [102349-world-only.png](2026-11-12/102349-world-only.png) | --world without a route option: the package's first route (else free ride) instead of the built-in test route in the hillside (scene `world-only`) |
| 2026-11-12 10:22:26 | [102226-multiplayer-still-30.png](2026-11-12/102226-multiplayer-still-30.png) | make test-multiplayer: camera client on marge, 30 s into the run – 8 riders (Linux, macOS, Windows bots and the kiosk client) on marge's server over QUIC, Vaduz–Malbun |
| 2026-11-12 10:22:26 | [102226-multiplayer-still-45.png](2026-11-12/102226-multiplayer-still-45.png) | make test-multiplayer: camera client, 45 s – the remote riders climbing out of Vaduz, HUD "riders: 8" |
| 2026-11-12 10:22:26 | [102226-multiplayer-camera.mp4](2026-11-12/102226-multiplayer-camera.mp4) | make test-multiplayer: camera client video (5 fps, before the frame-rate fix) |
| 2026-11-12 10:22:26 | [102226-multiplayer-verdict.txt](2026-11-12/102226-multiplayer-verdict.txt) | make test-multiplayer: verdict, RESULT: PASS (13 checks) – joins, snapshots, visibility, prediction, round trips, server tick timing |
| 2026-11-12 11:40:00 | [114000-multiplayer-kiosk-kiosk.mp4](2026-11-12/114000-multiplayer-kiosk-kiosk.mp4) | make test-multiplayer after the frame-rate fix: kiosk screen recording on marge (real GPU, 2560x1600 scaled, 25 fps real time, FPS badge 60 fps), 8 riders over QUIC |
| 2026-11-12 11:40:00 | [114000-multiplayer-kiosk-still-25.png](2026-11-12/114000-multiplayer-kiosk-still-25.png) | Kiosk still at 25 s: remote riders climbing out of Vaduz, FPS badge "60 fps max 24 ms", REC indicator |
| 2026-11-12 11:40:00 | [114000-multiplayer-camera-camera.mp4](2026-11-12/114000-multiplayer-camera-camera.mp4) | Camera client (headless, release build) 25 fps real-time video, ~40 fps render rate (headless weston vsync) |
| 2026-11-12 11:40:00 | [114000-multiplayer-camera-still-50.png](2026-11-12/114000-multiplayer-camera-still-50.png) | Camera client still at 50 s |
| 2026-11-12 11:40:00 | [114000-multiplayer-verdict.txt](2026-11-12/114000-multiplayer-verdict.txt) | Verdict of this run: RESULT: PASS (13 checks) |
| 2026-11-12 11:40:00 | [114000-multiplayer-summary.txt](2026-11-12/114000-multiplayer-summary.txt) | Run summary: machines, videos (size, fps, length), camera render rate |
