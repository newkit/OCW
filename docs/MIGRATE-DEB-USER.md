# Moving the OCW coordinator from `nils@deb` to `claude@deb`

Decided by the user on 2026-11-15:

- **New checkouts:** `/home/claude/src/OCW` and, next to it, the art project's `/home/claude/src/OCW-ART` ([newkit/OCW-ART](https://github.com/newkit/OCW-ART)), fresh `git clone`s.
- **Commit author:** unchanged. The commits keep the user's git name and e-mail, plus the Claude co-author lines.
- **Transcripts:** the old Claude Code transcripts are copied, so the art project keeps its prompt and token history.
- **One by one:** first OCW (move 1), then OCW-ART (move 2).

Who runs a step: **[user]** means you (root, or your browser for GitHub). **[claude]** means the Claude Code session that runs as `claude@deb`. **[nils]** means the current session.

The build machines are not affected. They know only their own users, `remote.sh` keeps their checkouts at the same paths, and the SSH keys for them travel with `config/ssh/`.

**Disk:** the copies go to `/home/claude`'s own disk. OCW's state is about 8 GB (`test-output/` 4 GB, `target/web/` 2.2 GB, transcripts 1.4 GB) and the art archive about 7 GB.

**Order:** OCW-ART reads OCW from `../OCW` and writes its collector status into OCW's `test-output/build-log/DAY/art.json`, so it can move only after OCW. Between the two moves the art collector is paused. It collects yesterday and today on every run, so do move 2 on the same or the next day. For days that were missed, run `./collect.sh DAY...` in OCW-ART afterwards.

## 0. Before both moves

1. **GitHub access for `claude` [user].** As `claude`, choose one of these:

   - **HTTPS:** `gh auth login` (GitHub.com, HTTPS, log in with a browser and the one-time code), then `gh auth setup-git`.
   - **SSH:** `ssh-keygen -t ed25519 -C "claude@deb OCW"`, then add `~/.ssh/id_ed25519.pub` to your GitHub account (Settings → SSH keys), plus `gh auth login` for releases and workflows.

   Check: `gh auth status` and `git ls-remote git@github.com:newkit/OCW.git HEAD` (or the HTTPS URL).

2. **Git identity [claude].**

   ```bash
   git config --global user.name  "<your name as in nils' git config>"
   git config --global user.email "<your e-mail as in nils' git config>"
   ```

   Copy them from `git config user.name` / `user.email` in the old checkout; they are not written here on purpose.

3. **Local tools.** Only on deb; builds stay on the build machines.

   - Already installed: `python3` (3.11+ at `/usr/bin/python3`), `ffmpeg`, `rsync`, `gh`.
   - Rust (`rustup`) is needed only for tools that `remote.sh` does not build remotely. Today nothing on deb needs a local cargo build.

## Move 1: OCW

1. **[nils] Push.** `main` in `/home/nils/data/git/OCW` must have no unpushed commits (`git status -sb` does not say "ahead"). Agent work lives on pushed branches.

2. **[claude] Clone or update.**

   ```bash
   mkdir -p /home/claude/src
   git clone git@github.com:newkit/OCW.git /home/claude/src/OCW   # or: git -C /home/claude/src/OCW pull
   ```

3. **[user, root] Dry run.**

   ```bash
   sudo /home/nils/data/git/OCW/scripts/migrate-user.sh ocw
   ```

   It shows what it would copy:

   - **`config/`:** `machines.toml`, `build-machines.toml`, `ssh/` and `dashboard-token`. Secrets stay mode 600 and are never printed.
   - **`test-output/`, `target/web/`** and the dev world key.
   - **The Claude Code project folder:** transcripts and memory, into `claude`'s `~/.claude/projects/` under its old name (`-home-nils-data-git-OCW`). The memory also goes into the new checkout's folder, `-home-claude-src-OCW`. The art collector finds both histories through `~/.claude/projects/*OCW*/`.

4. **Cutover**, when no agent, remote run, render or deploy is active:

   1. **[nils]** Stop the dashboard and pause the art collector:

      ```bash
      systemctl --user disable --now ocw-build-dashboard.service ocw-art-collect.timer
      ```

   2. **[user]** Copy and switch on linger. Re-running only copies what changed.

      ```bash
      sudo /home/nils/data/git/OCW/scripts/migrate-user.sh ocw --apply --linger
      ```

   3. **[claude]** Check and start, in `/home/claude/src/OCW`:

      ```bash
      scripts/remote.sh marge enabled && scripts/remote.sh marge run 'cargo --version'
      scripts/remote.sh macos run 'cargo --version'
      gh auth status && git push --dry-run origin main
      make dashboard-install
      curl -s localhost:8765/api/status | head -c 200
      ```

   4. **[claude]** Re-arm the dashboard answer watch (`tail -n 0 -F test-output/answers/inbox.log`) and check `scripts/remote.sh leela enabled`. Then deploy once with `scripts/deploy-test.sh marge`, so the kiosks get a build from the new checkout.

   5. **[claude]** Re-create or resume open agent work from its pushed branches; worktrees are not copied.

   From here on the `nils@deb` session does no more OCW work. The dashboard's art line stays "LATE" until move 2.

## Move 2: OCW-ART

1. **[nils] Push.** `/home/nils/data/git/OCW-ART` has no unpushed commits, and no render or publish is running.

2. **[claude] Clone** next to OCW. OCW-ART then finds `../OCW` by itself and needs no `ocw-art.toml`.

   ```bash
   git clone git@github.com:newkit/OCW-ART.git /home/claude/src/OCW-ART
   ```

3. **[user, root] Copy the art archive's media**: dry run, then the copy. The media go into `/home/claude/src/OCW-ART/build-history/` from `/home/nils/data/git/OCW-ART/build-history/` (or from OCW's old `art/build-history/` if they are still there; OCW-ART's ART-PROJECT.md §5.2). The clone already has the tracked text files.

   ```bash
   sudo /home/nils/data/git/OCW/scripts/migrate-user.sh art
   sudo /home/nils/data/git/OCW/scripts/migrate-user.sh art --apply
   ```

4. **[nils]** Remove the old collector units: `make -C /home/nils/data/git/OCW-ART uninstall-timer`.

5. **[claude]** Check, install the collector and catch up, in `/home/claude/src/OCW-ART`:

   ```bash
   make ocw-repo                 # /home/claude/src/OCW
   make check && make test
   make install-timer
   ./collect.sh                  # yesterday and today; add DAYs missed since move 1
   make timer-status             # ExecStart: /home/claude/src/OCW-ART/collect.sh
   ```

   The dashboard's art line then shows the new run.

## Afterwards

- **Old checkouts:** `/home/nils/data/git/OCW` and `/home/nils/data/git/OCW-ART` stay as read-only backups until you delete them.
- **The `nils@deb` session:** it stops working on OCW and OCW-ART.
- **Other jobs:** the unrelated `nils` crontab entry (COVID-19) stays with `nils`.
- **Memory:** the `claude` session's memory starts from the copied memory files (move 1).
