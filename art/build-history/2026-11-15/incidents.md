# Incidents – 2026-11-15

Root-cause notes by the coordinator (art/ART-PROJECT.md §2.6). The collected
entries (journal, build log) are in `logs/build-log/incidents.jsonl` and in
the journal's "Incidents" section.

## 00:39 – deb ran out of memory: the dashboard and the coordinator session were killed

- **What happened:** the OOM killer stopped `ocw-build-dashboard.service`
  (00:39:18) and the coordinator's Claude Code session, its tmux scope
  (00:39:47), on deb. systemd restarted the dashboard 11 s later.
- **Cause:** two QEMU VMs ("windows", "macos") held about 8 GB of deb's 15 GB
  permanently. On top of that ran about 10 parallel agents in one Claude Code
  process (6.8 GB peak), the re-renders of the diary videos (ffmpeg) and the
  art collector. Swap was full (4 of 4 GB).
- **Effect:** the session crashed and 8 running agents were interrupted.
  Their worktrees were intact, and they were resumed at about 06:15. The
  day-4 video rendering at that moment was left truncated and was rendered
  again.
- **Fix:** the user stopped the VMs, and the dashboard was restarted. From now
  on at most about 5 agents run at a time, and renders don't run in parallel
  with many agents. Renders on deb run one day at a time.
- **Follow-up:** the dashboard unit got `Restart=on-failure` and
  `MemoryHigh=768M` / `MemoryMax=1G` (`tools/build-dashboard/ocw-build-dashboard.service`).
  It takes effect after `make dashboard-install`.

## 01:05 – the art collector failed every hour: journals written without the users masked

- **What happened:** `ocw-art-collect.service` failed every hour from 01:05 to
  06:05. Its mask check found 16 unmasked build-machine user names in the
  tracked journals and token files of days 1–4.
- **Cause:** the music agent re-rendered days 1–4 from its agent worktree
  against the main archive (00:24–00:39), and `render` rewrites the day's
  journal. `diary.py` read the machine list from the worktree's
  `config/machines.toml`, which a worktree doesn't have, so the users and
  hosts were left out of the mask. Only the home directory, e-mail and IP
  patterns applied.
- **Effect:** the tracked text files in the main checkout held the build user
  name as a word (not committed). The re-rendered day videos of that run
  could show it in prompt bubbles; they were published at about 06:15 and
  replaced at 06:43.
  No hosts and no IP addresses were affected.
- **Fix:** `check-masked --fix` with the main checkout's tool re-masked the
  files, and the check passes again. `diary.py` now finds `config/machines.toml`
  in the main checkout too, and refuses to mask without a machine list.
  Days 1–4 and the diary were rendered again and re-published.
- **Follow-up:** a regression test (`MachinesConfTest`). The collector's mask
  check did its job; the incident log made the hourly failures visible.
