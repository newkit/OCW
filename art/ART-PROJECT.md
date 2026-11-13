# OCW Build Diary – Art Project Specification

Version 1.4 – 13 November 2026

| Version | Change |
|---|---|
| 1.0 | The brief, the diary format, the media archive, the tool (`art/diary.py`), day scripts for 11 and 12 November 2026 |
| 1.1 | Every photo and video carries its ISO 8601 date and time, in the file and in the picture (§2.1) |
| 1.2 | The build process itself is documented: every build/test run on the build machines and the build servers' state (§2.2) |
| 1.3 | The user's prompts are part of the archive and the videos (§2.3) |
| 1.4 | Token usage of the build: in total, per task and per feature (§2.4) |

## 1. The brief

The art project documents the **entire build process of Open Cycling World**:
every step, with images, videos, text and audio. Every commit, merge and test
is part of the story. From this material a video is made for **every build day**,
and the days are joined into a growing **diary**: each new day is added after the
day before, so the diary video tells the whole story from the first commit to today.
Each day's video is also kept on its own.

Tone and style:

- **Cool and stylish**: a clear visual identity (the app's colours and fonts),
  confident typography, motion that serves the story.
- **Funny**: dry, affectionate humour about the build, the bugs, the riders and the
  agents who never sleep. It never mocks real people, and it doesn't invent facts:
  every joke is about something that really happened that day (commits, tests,
  pictures).
- **Appealing to the viewer**: short (about 1.5–3 minutes per day), fast
  but readable, music and sound, a story arc per day.
- **Time-lapse elements**: the whole day is compressed into a time-lapse
  (every picture in order, the clock running, the commits scrolling by),
  and video shots can be sped up (with a "TIME-LAPSE ×N" badge).

The source text of this brief (the user's own words, 12 November 2026):

> I want to create an Art Project on the entire Build Process of this OCW App.
> Therefore it is super important to document every step we do with images,
> videos, text and audio. Every merge, commit and test is interesting as well.
> […] create a video which documents the evolution of the world, the riders,
> the images, the landscapes and such. […] for every build day, adding each new
> day to the previous video from the day before, so that it becomes like a diary.
> Also keep the per day video separately as well. Use a cool and stylish way of
> story telling and make it funny too. Make it appealing to the viewer. Use some
> time lapse elements.

## 2. What is documented (the archive)

**Everything** made while building OCW is kept; nothing is curated away. The
selection happens only in the video.

| Kind | Source | Kept as |
|---|---|---|
| Screenshots, contact sheets | test runs (`test-output/`, agent worktrees), scene harness | `YYYY-MM-DD/HHMMSS-label.png` |
| Videos | `--capture` recordings, scenes, multiplayer kiosk/camera | `YYYY-MM-DD/HHMMSS-label.mp4` |
| Audio | game-audio demos (`.wav`) | `YYYY-MM-DD/HHMMSS-label.wav` |
| Text | test verdicts and summaries (`.txt`) | `YYYY-MM-DD/HHMMSS-label.txt` |
| Logs | app/server/bot logs, run args, route files | `YYYY-MM-DD/logs/…` |
| Commits, merges, spec versions, tests | git history | `YYYY-MM-DD/journal.md` + `journal.json` |

- **Where:** `artwork/build-history/YYYY-MM-DD/`, which is git-ignored. Media never
  go into git (spec §11.3; the history was rewritten on 2026-11-12 to remove them).
- **Published:** one zip per day, `ocw-build-history-YYYY-MM-DD.zip`, on the GitHub
  release [`build-history`](https://github.com/newkit/OCW/releases/tag/build-history),
  next to the videos. `art/diary.py fetch` restores a day on another machine.
- **Time** of a file: the `HHMMSS-` prefix (local time of the run), else the time
  in its folder name, else its modification time. Commits use local time as well.
- Identical files (same SHA-256) are kept once.

### 2.1 Date and time on every photo and video

Every photo and video carries its **ISO 8601 date and time** (local time with
UTC offset, e.g. `2026-11-12T22:14:05+01:00`) twice:

- **In the picture:** the app draws it in the in-game HUD's **top-right corner**
  on every screenshot and every video frame, also when the HUD is otherwise
  off (spec §3.2). In frame-exact `--capture` videos it is the capture start
  plus the video time.
- **In the file:** PNG `tEXt` chunk `Creation Time` (plus `Software`); MP4
  `creation_time` (UTC, the start of the recording). The diary videos carry
  their render time.
- **Older material** (made before this rule): `art/diary.py stamp` (run by
  `gather`) writes the capture time from the file name into the archive copy's
  metadata; the picture itself is left as it was taken.

### 2.2 The build process: runs, build times, errors, build servers

The build infrastructure is part of the story, so it is recorded continuously,
in `test-output/build-log/YYYY-MM-DD/` of the main checkout (git-ignored;
gathered into the day folder's `logs/build-log/`):

| File | Written by | Content |
|---|---|---|
| `runs.jsonl` | `scripts/remote.sh run` (every build / test run on a build machine, by people and agents) | one JSON line per run: start (ISO 8601), seconds (including any wait for a build slot), machine, OS, checkout, branch, command, exit code, log file |
| `HHMMSS-MACHINE-PID.log` | `scripts/remote.sh run` | the run's full output (stdout + stderr): build times, warnings, test results, **errors** |
| `servers.jsonl` | the build dashboard (`make dashboard-install`; every 60 s) | the build servers' state: online / offline, CPU, memory, disk, running and queued tasks, build slots |

- **Privacy:** host names, addresses, user names and key paths are replaced by
  the machine's name before anything is written (the dashboard masks too).
- **Journal:** `art/diary.py journal` adds "Builds and build servers" to the day's
  `journal.md`: runs, build hours, failures with their commands and logs, the
  longest runs, and each server's online share. The day video's end card shows
  BUILD RUNS and BUILD HOURS.
- `OCW_BUILD_LOG=0` turns the run log off for a single command.

### 2.3 The prompts

The user's prompts to Claude Code (the conversation that builds OCW) are part
of the story.

- **Source:** the Claude Code session transcripts of the project
  (`~/.claude/projects/*OCW*/*.jsonl`, `OCW_TRANSCRIPTS` overrides the root): the
  user's own messages, including those sent while Claude was working; not the
  agents' messages, tool output or notifications; slash commands as `/name args`.
- **Kept as:** `DAY/prompts.jsonl` (time, ISO 8601, text) and `DAY/prompts.md`;
  the journal lists them. `art/diary.py prompts DAY` (run by `journal`).
- **Masked** before they are written: every build machine's host and user name,
  e-mail addresses and the home directory. The day zips are public.
- **In the video:** during the time-lapse each prompt pops up as a chat bubble
  ("YOU · 08:58") when its time comes (at most one new bubble per 1.2 s; the
  newest wins); a chapter can quote the prompt that started it (`prompt = "…"`
  in the day script, shown as "YOU ASKED"); the end card counts PROMPTS.

### 2.4 Token usage

How much model work went into the build is part of the record too.

- **Source:** every assistant message in the project's Claude Code
  transcripts: the main session and every agent
  (`subagents/agent-*.jsonl` with its `meta.json`). Each message is counted
  once (de-duplicated by message id), on the local day of its timestamp.
- **Counted:** uncached input, cache writes, cache reads and output tokens,
  their sum ("tokens"), and the number of model messages; per model too.
- **Per task:** an agent's task description (e.g. "agent: Fix wheels sinking
  into road"); the main session's work under the user prompt it answered
  ("coordination: <prompt>", masked like the prompts, §2.3).
- **Per feature:** tasks are grouped by keywords (`FEATURES` in `art/diary.py`:
  art project, materials & look, menus, sky & time, audio, multiplayer,
  riders, worlds & terrain, build & infra, planning; the first match wins).
- **Kept as:** `DAY/tokens.json`; the journal has a "Token usage" section
  (per feature, the 40 biggest tasks); the day video's end card shows
  MILLION TOKENS. `art/diary.py tokens DAY` (run by `journal`).
- Most of the total is cache reads (the conversation re-read from the cache
  on every step); output tokens are about 0.2 % of it.

## 3. The day video

About 1.5–3 minutes, 1920×1080, 30 fps, H.264 + AAC, built from these parts,
joined with short animated transitions (pixelize, slides, wipes …):

1. **Title** (≈6 s): the day's hero picture with a slow zoom; "OCW BUILD DIARY",
   **DAY N**, the date, the tagline typed out, the day's title. A bicycle bell rings.
2. **The day in N seconds** (≈10–26 s, time-lapse): every picture of the day in
   time order (no review sheets or before/after crops). A big clock shows the time
   of each picture; counters for commits, merges and pictures run up; on the
   right the commit log scrolls, the newest commit highlighted in pink.
3. **Chapters** (the story, ≈6–10 s each): a time stamp (gold) and a chapter
   title (violet lower third) slide in, then a caption is typed out. Shots are
   stills with a slow Ken-Burns zoom, video clips (optionally sped up, with a
   time-lapse badge) or before/after **split** shots. A yellow **sticker note**
   pops in with a joke or a pointer. A chapter can play a sound file of that day
   (e.g. the game's own audio) while the music ducks.
4. **End card** (≈9 s): a mosaic of the day's pictures, the numbers of the day
   counting up (commits, merges, lines written, pictures and videos, tests
   passing), a closing line and a teaser for the next day.

**Diary video** (`ocw-build-diary.mp4`): a short opener ("The Build Diary",
the logo, a bell), then every day video in order. Rebuilding it after a new
day is the same as appending the new day to the previous diary.

### 3.1 Visual identity

- Colours from the logo (`artwork/logos/ocw.svg`): rider pink `#FF1472`, deep
  violet `#3A2F55`, gold `#C98A00`; sticker yellow `#FFE45C`.
- Type: **Lato** (Black, Heavy, Semibold, Medium Italic) for titles and captions,
  **Fira Mono** (the app's UI font, `crates/ocw-render/assets/fonts/`) for clocks,
  counters, the commit log and badges.
- Pictures that are not 16:9 sit on a blurred, darkened copy of themselves (no
  black bars). Time-lapse and end card get a light vignette.

### 3.2 Sound

- **Music**: synthesised by the tool itself (no samples, no licences):
  a chiptune-pop band (kick, clap, hats, square bass, pulse arpeggio, soft
  pad) over a vi–IV–I–V progression. Sections follow the video: *intro*
  (no drums) under the title, *drive* (four-on-the-floor) under the
  time-lapse, *groove* under the chapters, *outro* under the end card.
  Tempo and key per day (`bpm`, `key`).
- **Effects**, also synthesised: bicycle bell (title), whooshes (transitions),
  pops (sticker notes), ticks (counters), a ding (end card).
- **Game audio** from the day's archive (e.g. the freehub demo) can be mixed in
  per chapter.

## 4. The day script (`art/days/YYYY-MM-DD.toml`)

The story is written per day. It can be changed at any time and the video
rendered again.

```toml
title = "A one-line title of the day"
tagline = "In which … (typed out on the title)"
hero = "HHMMSS-label.png"          # title background
bpm = 108                          # music tempo (optional)
key = "C"                          # music key (optional)
tests = 903                        # tests passing at the end of the day (optional;
                                   # else the highest "N tests" in a commit message)
outro = "Closing line on the end card."
teaser = "NEXT TIME: …"            # optional
title_seconds = 6.5                # optional
end_seconds = 9                    # optional

[[chapter]]
time = "15:02"                     # when it happened (shown in gold)
title = "Hello, Liechtenstein"
caption = "The funny caption, typed out."
prompt = "the user's prompt that started it"   # optional, shown as a chat bubble
sound = "HHMMSS-audio.wav"         # optional: the day's sound file, music ducks
sound_gain = 1.0                   # optional
shots = [
  { file = "HHMMSS-x.png", seconds = 3.0, note = "sticker text" },     # still: Ken Burns
  { file = "HHMMSS-y.mp4", seconds = 4.0, start = 2.0, speed = 2 },    # clip, 2× time-lapse
  { split = ["before.png", "after.png"], seconds = 3.5 },              # before | after
  { card = true, seconds = 2.0 },                                      # plain violet card
]
```

`file` is a name in the day folder, or a path in the repository (e.g.
`artwork/logos/ocw-512.png`), or `../YYYY-MM-DD/name` for another day.

Writing guidelines:

- 6–12 chapters in time order, each one thing that changed what you see or
  how it works. Use the journal (`journal.md`) and the contact sheets.
- Captions ≤ 120 characters, notes ≤ 40. One joke per chapter is plenty.
- Prefer shots that show change: first appearances, before/after, failures that
  look funny (a rider inside a tree is content).
- Facts must be true: times, numbers and names come from the journal.

## 5. Workflow

Needs Python 3.11+ (`/usr/bin/python3` on Debian 13; the pyenv default 3.8 is too
old), ffmpeg with libass, `zip`, and `gh` (logged in) for fetching/publishing.
The tool runs on the dev machine (no Rust build involved).

```bash
make art-gather DAY=2026-11-13    # collect everything of the day, stamp times, journal
make art-sheets DAY=2026-11-13    # contact sheets: artwork/build-history/.work/DAY/sheets/
# write art/days/2026-11-13.toml (story, shots, jokes)
make art-render DAY=2026-11-13    # → artwork/build-history/diary/ocw-diary-day-NN-DAY.mp4
make art-diary                    # → artwork/build-history/diary/ocw-build-diary.mp4
make art-publish DAY=2026-11-13   # day zip + all videos → release build-history
```

or directly `art/diary.py gather|journal|sheet|render|diary|publish|all [DAY…]`.

**At the end of every build day** (or when asked): gather, journal, write the
day script, render the day, rebuild the diary, publish. During the day, keep
every screenshot, video and sound in `test-output/` (the test harnesses already
do), so the gather step finds them.

## 6. Future elements (ideas, not built yet)

- A narrator voice (local TTS, e.g. Piper) reading the captions.
- Per-commit "diff art": lines added/removed as a growing skyline.
- A map of Liechtenstein (and later worlds) lighting up the roads ridden so far.
- Test results as a heartbeat line (passes per run over the day).
- Chapter music stingers per theme (night shift, release day).
- An auto-drafted day script from the journal, for a human to polish.
