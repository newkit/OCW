# Machine configuration

Describes the remote machines used to build, test and benchmark OCW on real
hardware (Windows and macOS ports, spec §11.1; graphics spike on a real GPU,
spec §3.1.1).

| File | In git | Contents |
|---|---|---|
| `machines.example.toml` | yes | Documented template with placeholder values |
| `machines.toml` | **no** | Your machines (copy of the template, filled in) |
| `ssh/` | **no** | Private keys and `known_hosts` for these machines |

Everything in `config/` except this README and `*.example.*` files is ignored by
git (see `.gitignore`), so the real configuration and keys cannot be committed by
accident. Check with:

```bash
git check-ignore -v config/machines.toml config/ssh/ocw_linux_ed25519
```

## Setup

1. Copy the template and fill in your machines:

   ```bash
   cp config/machines.example.toml config/machines.toml
   ```

2. Create one key pair per machine (no passphrase, so automated runs work; the
   key only grants access to the dedicated `ocw` account):

   ```bash
   mkdir -p config/ssh && chmod 700 config/ssh
   ssh-keygen -t ed25519 -N "" -C "ocw-linux-gpu" -f config/ssh/ocw_linux_ed25519
   ```

3. Install the public key (`*.pub`) on the machine, in
   `~/.ssh/authorized_keys` of the `ocw` user (Windows:
   `C:\Users\ocw\.ssh\authorized_keys`, or `administrators_authorized_keys`
   for admin accounts).

4. Record the host key once, so later connections are verified:

   ```bash
   ssh-keyscan -p 22 linux-gpu.example.lan >> config/ssh/known_hosts
   ```

5. Test the login:

   ```bash
   ssh -i config/ssh/ocw_linux_ed25519 -o UserKnownHostsFile=config/ssh/known_hosts \
       -o IdentitiesOnly=yes ocw@linux-gpu.example.lan 'uname -a'
   ```

## Building and testing remotely

```bash
scripts/remote.sh marge run 'cargo test --workspace'   # copy working tree, run there
scripts/remote.sh marge ssh 'vulkaninfo --summary'      # plain ssh, no copy
scripts/remote.sh marge fetch test-output ./from-marge  # copy results back
```

`python3 tools/build-dashboard/dashboard.py` shows the load of these
machines and the tasks running or waiting on them (read-only, see
`tools/build-dashboard/README.md`).

`run` copies tracked and untracked files (never `config/` or git-ignored
files such as `target/`) to `repo_path` and runs the command with
`~/.cargo/env` loaded.

## Test app on a machine's screen (kiosk)

A test machine can show the current build permanently on one of its
screens, restarting it whenever a new build is deployed:

```bash
scripts/deploy-test.sh marge        # build there, install to /var/tmp/ocw (atomic)
make deploy-test MACHINE=marge      # the same
```

Once, as the user of the machine's desktop session (in a terminal on that
desktop, not over SSH):

```bash
/var/tmp/ocw/ocw-kiosk.sh install --monitor 2560x1600   # or an index (0, 1) or a name (HDMI)
/var/tmp/ocw/ocw-kiosk.sh status                        # build, autostart, recent log
/var/tmp/ocw/ocw-kiosk.sh uninstall
```

`install` adds an XDG autostart entry and starts the kiosk now. The kiosk
restarts the app on every deployment and after crashes; logs are in
`~/.local/state/ocw/` (`kiosk.log`, `app.log`). The detected monitors are
listed as `monitor` lines in `app.log`. `scripts/test-kiosk.sh` tests the
kiosk logic with a fake app.

## Per-OS prerequisites on the machines

- **Linux:** run `make deps-linux` in a checkout (detects Debian/Ubuntu, Fedora or Arch; packages via
  sudo, incl. FUSE 2 for AppImages, then rustup with the pinned toolchain and
  appimagetool in `~/.local/bin` for `scripts/build-appimage.sh`; that part
  alone, without sudo: `make deps-appimage`). An admin can install only the
  packages for another user with `sudo make -C <checkout> deps-linux-packages`. A user without a desktop
  session needs `sudo usermod -aG render,video <user>` for GPU access.
  `vulkaninfo --summary` must show the real GPU, not llvmpipe. Benchmarks run
  on a headless weston, so no desktop session is needed.
- **Windows:** OpenSSH Server and a local build user (not an administrator):
  `scripts/setup-windows-buildserver.ps1 -PublicKeyFile <key>.pub` in PowerShell
  as Administrator (after `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass`
  to allow scripts in that window) sets both up (any Windows language; user `claude` by
  default), plus `make deps-windows` (winget: Git, Visual
  Studio Build Tools with the C++ workload, rustup; needs GNU make, e.g. from
  MSYS2 or `winget install ezwinports.make`). GUI runs need a logged-in user;
  they are started as a scheduled task in that session (programs started over
  SSH cannot open windows).
- **macOS:** Remote Login (System Settings → General → Sharing), then
  `make deps-macos` (Xcode Command Line Tools, rustup, ffmpeg via Homebrew). GUI runs need a logged-in user. Bluetooth access asks for
  permission once on screen.
