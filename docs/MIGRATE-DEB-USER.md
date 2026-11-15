# Moving the OCW coordinator from `nils@deb` to `claude@deb`

Decided by the user on 2026-11-15:

- **New checkout:** `/home/claude/OCW`, a fresh `git clone`.
- **Commit author:** unchanged. The commits keep the user's git name and e-mail, plus the Claude co-author lines.
- **Transcripts:** the old Claude Code transcripts are copied, so the art project keeps its prompt and token history.

Who runs a step: **[user]** means you (root, or your browser for GitHub). **[claude]** means the Claude Code session that runs as `claude@deb`. **[nils]** means the current session.

The build machines are not affected. They know only their own users, `remote.sh` keeps their checkouts at the same paths, and the SSH keys for them travel with `config/ssh/`.

## 1. GitHub access for `claude` [user]

As `claude`, choose one of these:

- **HTTPS:** `gh auth login` (GitHub.com, HTTPS, log in with a browser and the one-time code), then `gh auth setup-git`.
- **SSH:** `ssh-keygen -t ed25519 -C "claude@deb OCW"`, then add `~/.ssh/id_ed25519.pub` to your GitHub account (Settings → SSH keys), plus `gh auth login` for releases and workflows.

Check: `gh auth status` and `git ls-remote git@github.com:newkit/OCW.git HEAD` (or the HTTPS URL).

## 2. Git identity [claude]

```bash
git config --global user.name  "<your name as in nils' git config>"
git config --global user.email "<your e-mail as in nils' git config>"
```

Copy them from `git config user.name` / `user.email` in the old checkout; they are not written here on purpose.

## 3. Clone [claude]

```bash
git clone git@github.com:newkit/OCW.git /home/claude/OCW   # or the HTTPS URL
cd /home/claude/OCW
```

Local tools on deb only. Builds stay on the build machines.

- Already installed: `python3` (3.11+ at `/usr/bin/python3`), `ffmpeg`, `rsync`, `gh`.
- Rust (`rustup`) is needed only for the tools that `remote.sh` does not build remotely. Today nothing on deb needs a local cargo build.

## 4. Copy the git-ignored state [user, root]

```bash
sudo /home/nils/data/git/OCW/scripts/migrate-user.sh            # dry run first
sudo /home/nils/data/git/OCW/scripts/migrate-user.sh --apply    # copy
```

It copies:

- **`config/`:** `machines.toml`, `build-machines.toml`, `ssh/` and `dashboard-token`. Secrets stay mode 600.
- **`test-output/`, `art/build-history/`, `target/web/`** and the dev world key.
- **The Claude Code project folder:** transcripts and memory, into `claude`'s `~/.claude/projects/` under its old name. The art collector finds both histories through `~/.claude/projects/*OCW*/`.

Re-running only copies what changed.

## 5. Check [claude]

```bash
scripts/remote.sh marge enabled && scripts/remote.sh marge run 'cargo --version'
scripts/remote.sh macos run 'cargo --version'
gh auth status && git push --dry-run origin main
make art-check && make test-art
python3 tools/build-dashboard/dashboard.py --once --no-record | head
```

## 6. Cut over (when no agent and no remote run is active) [nils] → [user] → [claude]

1. **[nils]** Let the running agents finish, or commit their work on their branches and push the branches. Then stop and disable the user units:

   ```bash
   systemctl --user disable --now ocw-art-collect.timer ocw-build-dashboard.service
   ```

2. **[user]** Run the final copy and switch on linger:

   ```bash
   sudo .../scripts/migrate-user.sh --apply --linger
   ```

3. **[claude]** Install the units from the new checkout:

   ```bash
   make dashboard-install && make art-install-timer
   systemctl --user list-timers | grep ocw-art
   curl -s localhost:8765/api/status | head -c 200
   ```

4. **[claude]** Re-arm the dashboard answer watch (`tail -n 0 -F test-output/answers/inbox.log`), check `scripts/remote.sh leela enabled`, then deploy once with `scripts/deploy-test.sh marge` so the kiosks get a build from the new checkout.

5. **[claude]** Re-create or resume open agent work from its pushed branches; worktrees are not copied.

## 7. Afterwards

- **The old checkout:** `/home/nils/data/git/OCW` stays as a read-only backup until you delete it.
- **This session:** the `nils@deb` session stops working on OCW.
- **Other jobs:** the unrelated `nils` crontab entry (COVID-19) stays with `nils`.
- **Memory:** the `claude` session's memory starts from the copied memory files (step 4).
