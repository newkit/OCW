# Moving the build coordinator from the Debian dev machine to marge

Status: **prepared, waiting for the user's okay** (2026-11-13). The coordinator is
the machine where the Claude Code session runs: it holds the git checkout that is
pushed to GitHub, `config/machines.toml` + `config/ssh/`, the agent worktrees, the
build history and the GitHub CLI login. Today that is the Debian dev machine.

## Findings on marge (2026-11-13)

| Item | State |
|---|---|
| Claude Code | installed (`/usr/bin/claude` 2.1.311; the dev machine has 2.1.323), **not logged in** (no `~/.claude`) |
| git, gh, python3 3.13, ffmpeg 7.1, zip, rsync, tmux, jq, clang-21, rustup toolchain | present (`art/diary.py` works with the system python) |
| GitHub | `gh` **not logged in**; no SSH key for GitHub |
| `~/src/OCW` | the `remote.sh` sync target (overwritten on every run) with a **stale `.git` from before the history rewrite** (HEAD `f0ff0a5`); not usable as the coordinator checkout |
| SSH to other build machines | macOS **reachable**; Windows **not reachable from marge** (no ping, port closed; reachable from the dev machine, so probably the Windows firewall / network); leela unreachable (as from the dev machine) |
| Keys for the other machines | none on marge (`~/.ssh` has only `authorized_keys`) |
| Disk | 85 GB free of 492 GB; `~/src/OCW/target` 108 GB, old agent worktrees 68 GB (can be cleaned) |
| Memory / CPU | 31 GB, 12 cores, shared with the kiosk |

## Steps for the user (before the move)

1. **Log in Claude Code on marge** as user `claude`: ssh in, run `claude`, then `/login`. Update first: `claude update`.
2. **GitHub on marge:** `gh auth login` (HTTPS, `repo` scope), then `gh auth setup-git` so `git push` and `gh release upload` work.
3. **Windows firewall:** allow SSH (port 22) from marge's address on the Windows build machine.
4. **Secrets are copied, never committed** (user, 2026-11-13): `config/machines.toml` and `config/ssh/` (keys for macOS and Windows, `known_hosts`) are copied with `scp`/`rsync` over SSH straight into marge's `~/coord/OCW/config/` (directory 700, files 600). They are git-ignored there as here: verified that `git check-ignore` covers both and that no commit has ever contained them. After copying, check with `git status --ignored` that they show only as ignored.

## Steps for Claude (after the okay)

1. Let the running agents finish and merge everything to `main`. Their worktrees live on the dev machine.
2. **Fresh clone** on marge to `~/coord/OCW`, separate from `~/src/OCW`, which `remote.sh` keeps overwriting.
3. **Copy** to marge:
   - `config/machines.toml` and `config/ssh/` by `scp`/`rsync`, never through git (step 4 above);
   - Claude memory: `~/.claude/projects/-home-nils-data-git-OCW/memory/` → `~/.claude/projects/-home-claude-coord-OCW/memory/`;
   - `artwork/build-history/` (1.3 GB);
   - `test-output/` (0.6 GB);
   - `data/pilot` and `data/blackforest` (or fetch them again).
4. **`remote.sh`:** when the target machine is the one it runs on (marge), run in place instead of ssh + rsync to itself. Machine entry `local = true` in `machines.toml`.
5. **CLAUDE.md:** marge is now the coordinator and local build machine. The "never build locally" rule then means "never on the dev machine". The kiosk shares marge, so heavy builds keep the slot limits, and long runs use `nice`.
6. **Clean up** old target dirs on marge (`cargo clean` in `~/src/OCW-worktree-agent-*`, about 68 GB).
7. **Verify:**
   - `make check`;
   - `scripts/remote.sh macos run 'cargo test -p ocw-sim'` and the same for Windows;
   - `gh release list`;
   - `art/diary.py fetch`;
   - `make test-multiplayer` (server and kiosk are now local);
   - `scripts/deploy-test.sh marge`.
8. **Run the session in tmux** on marge (survives disconnects). The user reaches it via ssh or Remote Control.

## Risks

- **Load:** builds and the kiosk share marge's GPU, CPU and 31 GB of memory. Slot limits stay at 2; videos and scenes are rendered while no Bevy build runs.
- **One failure point:** if marge goes down, coordination stops too. The dev machine keeps a clone as a fallback.
- **Losing work in flight:** don't move while agents are running.
