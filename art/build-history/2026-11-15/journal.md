# OCW build diary – day 5, 2026-11-15

1 commits (0 merges), +3 / −1 lines, 31 images, 3 videos, 0 sounds.

## Commits

| Time | Commit | |
|---|---|---|
| 00:00 | `9ea2fd0` | Spec 3.36: public preview of the browser app on GitHub Pages (B8a, stage 1) |

## Builds and build servers

16 build/test runs on the build machines, 2.7 h in total, 3 failed; 365 server samples; cargo compiled 4 crates in 0.0 h, 2.0 h waiting for build slots.

| Machine | Runs | Hours | Cargo h | Crates | Waited h | Failed | Online |
|---|---|---|---|---|---|---|---|
| Win11Builder | 0 | 0.0 | 0.0 | 0 | 0.0 | 0 | 100% |
| leela | 0 | 0.0 | 0.0 | 0 | 0.0 | 0 | 2% |
| macos | 11 | 0.7 | 0.0 | 4 | 0.4 | 2 | 100% |
| marge | 5 | 2.0 | 0.0 | 0 | 1.6 | 1 | 100% |

Failed runs:

- 00:09 macos: `cargo fmt -p ocw-xr -p ocw-assets && cargo test -p ocw-xr 2>&1 | tail -40 && car` (exit 129, log `build-log/000938-macos-3079424.log`)
- 00:16 macos: `cd vendor && (time CARGO_TARGET_DIR=../target cargo check --workspace --examples` (exit 1, log `build-log/001625-macos-3091403.log`)
- 00:13 marge: `make test-web-readme README_FLAGS="--keep --reuse-target"` (exit 2, log `build-log/001311-marge-3085504.log`)

Longest runs:

- 32 min on marge: `scripts/render-scenes.sh --out test-output/scenes/2026-11-15-000040 --only god-u`
- 30 min on marge: `cd tools/build-dashboard && python3 -m unittest -q`
- 26 min on marge: `make test-web-readme README_FLAGS="--keep --reuse-target"`
- 18 min on marge: `scripts/render-scenes.sh --out test-output/scenes/helmets-before2-x11 --only hel`
- 16 min on marge: `scripts/render-scenes.sh --out test-output/scenes/helmets-before2-wayland --only`

## Token usage

259.2 M tokens in 911 model messages (output 841 k, cache reads 254.2 M, cache writes 4.2 M, uncached input 2 k). Full breakdown: [tokens.json](tokens.json).

| Feature | Tokens | Output | Messages |
|---|---|---|---|
| Planning & spec | 117.5 M | 440 k | 333 |
| Other | 101.1 M | 359 k | 472 |
| Riders & bikes | 20.2 M | 17 k | 40 |
| Art project | 13.1 M | 18 k | 50 |
| Worlds & terrain | 4.1 M | 5 k | 6 |
| UI & menus | 3.1 M | 2 k | 10 |

| Task | Feature | Tokens | Output |
|---|---|---|---|
| agent: Phase 5: Google + Strava sign-in | Planning & spec | 46.8 M | 253 k |
| agent: VR X1: OpenXR viewing mode | Planning & spec | 44.7 M | 169 k |
| agent: B8a: GitHub Pages preview | Other | 28.9 M | 108 k |
| agent: Cooler helmets | Other | 23.0 M | 27 k |
| agent: Animations and K6 organiser rules | Riders & bikes | 20.2 M | 17 k |
| agent: Phase 7: training history, rebinding | Planning & spec | 17.3 M | 11 k |
| agent: Port bevy_mod_openxr to Bevy 0.20 | Other | 14.8 M | 24 k |
| agent: Realistic vegetation and crops | Other | 13.1 M | 97 k |
| agent: Art: hourly collector, move history | Other | 11.5 M | 63 k |
| agent: Research vegetation and data sources | Other | 9.9 M | 41 k |
| agent: Web mode: fix 404, pinned browser, README test | Art project | 9.8 M | 12 k |
| agent: God mode in the browser | Planning & spec | 6.7 M | 4 k |
| coordination: make the trees and vegetation more realistic. get information about the (big) pl… | Worlds & terrain | 4.1 M | 5 k |
| coordination: okay, do the changes and implement the small preview hosting on github. add do r… | Art project | 3.3 M | 6 k |
| agent: Console autocomplete, GPS in HUD | UI & menus | 3.1 M | 2 k |
| coordination: the music of day 4 is cool, but a bit monoton. make it a bit more varied. also m… | Planning & spec | 2.0 M | 2 k |

## Prompts

2 prompts (full text: [prompts.md](prompts.md)).

| Time | Prompt |
|---|---|
| 00:09 | the music of day 4 is cool, but a bit monoton. make it a bit more varied. also make different music for different days. record in spec |
| 00:17 | make the trees and vegetation more realistic. get information about the (big) plants and crops that are in the area. analyse them and add th |

## Media

| Time | File |
|---|---|
| 00:00:04 | [000004-shot-wayland-k6-organiser-rules-menu.jpg](000004-shot-wayland-k6-organiser-rules-menu.jpg) |
| 00:00:55 | [000055-shot-wayland-k6-results-rules.jpg](000055-shot-wayland-k6-results-rules.jpg) |
| 00:01:01 | [000101-shot-wayland-a26-foot-down.jpg](000101-shot-wayland-a26-foot-down.jpg) |
| 00:01:25 | [000125-video-wayland-a26-foot-down-video.jpg](000125-video-wayland-a26-foot-down-video.jpg) |
| 00:01:25 | [000125-video-wayland-a26-foot-down-video.mp4](000125-video-wayland-a26-foot-down-video.mp4) |
| 00:01:37 | [000137-shot-wayland-a26-cornering.jpg](000137-shot-wayland-a26-cornering.jpg) |
| 00:03:30 | [000330-video-wayland-a26-uturn-video.jpg](000330-video-wayland-a26-uturn-video.jpg) |
| 00:03:30 | [000330-video-wayland-a26-uturn-video.mp4](000330-video-wayland-a26-uturn-video.mp4) |
| 00:08:18 | [000818-shot-wayland-a26-uturn-look.jpg](000818-shot-wayland-a26-uturn-look.jpg) |
| 00:12:21 | [001221-shot-wayland-a26-uturn-loop.jpg](001221-shot-wayland-a26-uturn-loop.jpg) |
| 00:13:57 | [001357-video-wayland-a26-victory-video.jpg](001357-video-wayland-a26-victory-video.jpg) |
| 00:13:57 | [001357-video-wayland-a26-victory-video.mp4](001357-video-wayland-a26-victory-video.mp4) |
| 00:14:36 | [001436-shot-wayland-a26-victory.jpg](001436-shot-wayland-a26-victory.jpg) |
| 00:16:12 | [001612-shot-wayland-helmet-road-behind.jpg](001612-shot-wayland-helmet-road-behind.jpg) |
| 00:16:23 | [001623-shot-wayland-helmet-tt-position.jpg](001623-shot-wayland-helmet-tt-position.jpg) |
| 00:16:34 | [001634-shot-wayland-helmet-group-chase.jpg](001634-shot-wayland-helmet-group-chase.jpg) |
| 00:17:13 | [001713-shot-helmet-road-behind.jpg](001713-shot-helmet-road-behind.jpg) |
| 00:17:37 | [001737-frames-v10.jpg](001737-frames-v10.jpg) |
| 00:17:37 | [001737-frames-v12.jpg](001737-frames-v12.jpg) |
| 00:17:37 | [001737-frames-v13.5.jpg](001737-frames-v13.5.jpg) |
| 00:17:38 | [001738-frames-v15.jpg](001738-frames-v15.jpg) |
| 00:17:47 | [001747-shot-helmet-tt-position.jpg](001747-shot-helmet-tt-position.jpg) |
| 00:17:56 | [001756-frames-f1.jpg](001756-frames-f1.jpg) |
| 00:17:57 | [001757-frames-f4.jpg](001757-frames-f4.jpg) |
| 00:17:57 | [001757-frames-f6.5.jpg](001757-frames-f6.5.jpg) |
| 00:17:58 | [001758-frames-u3.jpg](001758-frames-u3.jpg) |
| 00:18:00 | [001800-frames-u6.jpg](001800-frames-u6.jpg) |
| 00:18:01 | [001801-frames-u9.jpg](001801-frames-u9.jpg) |
| 00:18:02 | [001802-frames-u12.jpg](001802-frames-u12.jpg) |
| 00:18:03 | [001803-frames-u15.jpg](001803-frames-u15.jpg) |
| 00:18:18 | [001818-shot-helmet-group-chase.jpg](001818-shot-helmet-group-chase.jpg) |
| 00:27:16 | [002716-shot-wayland-god-unicorn-flight.jpg](002716-shot-wayland-god-unicorn-flight.jpg) |
| 00:32:00 | [003200-shot-wayland-god-inspect-vaduz.jpg](003200-shot-wayland-god-inspect-vaduz.jpg) |
| 00:32:11 | [003211-shot-wayland-god-menu-world.jpg](003211-shot-wayland-god-menu-world.jpg) |
