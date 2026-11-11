# Open Cycling World

A cross-platform indoor cycling simulation in Rust: real bicycle hardware, real
routes from OpenStreetMap, rendered locally with Bevy, with server-authoritative
multiplayer.

**Status:** Phase 3 (OpenStreetMap world; pilot region Liechtenstein). Phases 1
(technical foundation) and 2 (trainers and sensors) are done. The requirements
and architecture are in [SPECIFICATION.md](SPECIFICATION.md).

## Build and run (Linux)

```bash
make deps     # this platform: deps-linux (detects Debian/Ubuntu, Fedora, Arch) | deps-windows | deps-macos
make check    # fmt, clippy, tests, wasm build – what CI runs
make run      # desktop client with a virtual trainer and 4 test riders
make help     # all targets (build, release, test, server, screenshot, ...)
```

The Rust version is pinned in `rust-toolchain.toml`; Debian's system Rust is too
old for Bevy. `make` finds the rustup toolchain in `~/.cargo/bin` by itself; for
plain `cargo`, add `. "$HOME/.cargo/env"` to your shell profile.

### macOS app

On a Mac, `make macos-app` (`scripts/build-macos-app.sh`) builds
`target/macos/Open Cycling World.app` and packs it as a DMG and a zip
(`OpenCyclingWorld-<version>-<commit>-macos-arm64.dmg`; CI builds them on every
push to `main`, workflow artifact `macos-app`). Open the DMG and drag the app to
Applications; it starts the Hills test route with the virtual trainer, or
your remembered Bluetooth devices (macOS asks for Bluetooth access on first
use). The log is in `~/Library/Logs/OCW/ocw-desktop.log` (Console.app → Log
Reports); `autoexec.cfg` is not read when started from Finder (the working
directory is `/`). The app is only ad-hoc signed, not notarised, so Gatekeeper
blocks the first start: right-click the app → Open, or System Settings →
Privacy & Security → Open Anyway, or remove the quarantine flag with
`xattr -dr com.apple.quarantine "/Applications/Open Cycling World.app"`.
Video recording (key V) needs ffmpeg: install Homebrew, then `brew install
ffmpeg` (`make deps-macos` prints both commands); the app also finds it in
`/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin` when started from Finder.

## Controls

| Action | Keyboard | Gamepad |
|---|---|---|
| Faster: +25 W per press, more while held | ↑ | D-pad up, left stick up or right trigger (RT / R2) |
| Stop: no power, brake | ↓ | D-pad down, left stick down or left trigger (LT / L2) |
| Steer (autodrive: move the line; manual: lean into a curve) | ← / → | left stick (left/right) |
| Autodrive on/off (on at the start) | A | South button (A / ✕) |
| Wild mode on/off (leave the road) | W | West button (X / □) |
| Free ride: choose the exit at the next junction | ← / → (autodrive, junction ahead) or Q / E | D-pad left / right |
| Free ride: U-turn | U | East button (B / ○) |
| Route ride: back to the route after leaving it | J | North button (Y / △) |
| Power ±10 W | PgUp / PgDn | |
| Camera: orbit / zoom | mouse drag / wheel | |
| Camera reset | C | |
| Restart everything (as at program start; the ride is saved first) | R | |
| Record a video (start / stop) | V | |
| Next screen | F11 | |
| Console (`help` lists the commands) | `` ` `` or F12 | |

In manual steering, the keys ramp the lean and it stays when released (press
the other way to straighten up); the stick sets the lean directly, centred is
upright. The road's bends have to be steered, see spec §5.2.1.

## Connecting a gamepad

Any controller that the operating system sees as a gamepad works (Xbox,
PlayStation, Switch Pro, 8BitDo, generic USB pads); the app reads it through
Bevy's gamepad support (gilrs) and picks it up while running, no restart needed.
More than one pad: the stick with the largest deflection steers, any pad's
buttons count.

**Linux**

1. Connect it:
   - USB: plug it in.
   - Bluetooth: put the pad in pairing mode, then
     ```bash
     bluetoothctl
     scan on            # wait for the pad's address (XX:XX:…)
     pair XX:XX:XX:XX:XX:XX
     trust XX:XX:XX:XX:XX:XX
     connect XX:XX:XX:XX:XX:XX
     scan off
     ```
     (or use the desktop's Bluetooth settings). PlayStation pads work with the
     kernel's `hid-playstation` driver; Xbox pads over Bluetooth may need the
     `xpadneo` driver.
2. Check that Linux sees it: `ls /dev/input/by-id/ | grep -i joystick`, or
   `evtest` (pick the pad, move the stick).
3. Permissions: the app reads `/dev/input/event*`. On a normal desktop login
   this is allowed for the logged-in user (systemd-logind `uaccess`). For other
   sessions (SSH, kiosk users, some window managers) add the user to the
   `input` group and log in again: `sudo usermod -aG input $USER`. The log line
   `gilrs … Failed to open "/dev/input/eventN"` means this permission is missing
   (harmless when no pad is used).
4. Unusual pads with wrong buttons: give an SDL mapping (generate one with
   SDL's `controllermap` tool or [SDL_GameControllerDB](https://github.com/mdqinc/SDL_GameControllerDB))
   in the environment: `SDL_GAMECONTROLLERCONFIG="<mapping line>" ocw-desktop`.
5. Steam: with Steam running and "Steam Input" enabled for the pad, Steam may
   take it over; turn Steam Input off for that controller, or close Steam.

**Windows:** connect it (USB, Bluetooth in *Settings → Bluetooth & devices*, or
the Xbox wireless adapter); it appears in *Game controllers* (`joy.cpl`).

**macOS:** pair it in *System Settings → Bluetooth* (or plug it in); it appears
under *Game Controllers* in System Settings.

Then start the app: the left stick steers, the triggers ride and stop (see
[Controls](#controls)).

## Riding modes

**Steering** (spec §5.2.1; HUD shows the mode and the surface)

- **Autodrive** (default, key A): the rider keeps to the middle of the road by
  itself and leans through the bends; ← / → (or the stick) move its line.
- **Manual** (autodrive off): ← / → lean the rider into a curve; the lean stays
  until you straighten up with the other key. Nobody steers the road's bends
  for you: miss one and the rider drifts to the edge, where it scrapes along
  and slows down.
- **Wild** (key W): not bound to the road; ride onto the meadows, up to 60 m
  either side. The camera follows over terrain and trees.

**Free ride** (spec §2.2.1, §2.2.2): ride anywhere on the world package's road
network instead of a fixed route:

```bash
ocw-desktop --world data/pilot/liechtenstein.ocwpkg --free-ride [--start X,Z]
```

The package must contain the road graph (format 2, from `world-builder
package`). Before each junction an overlay shows the exits (arrows at their
real angles, street name, distance; dead end / unpaved / steep markers). Pick
one with ← / → (with autodrive, while a junction is ahead) or Q / E; the choice
turns green when it locks (20 m before the junction, earlier at speed).
Without a choice the rider takes the default exit (straightest, same road,
bigger road). At dead ends and the region border the rider turns round; U
turns round any time (in a loop as wide as the road, steered to the middle
first). Sharp turns slow the rider down automatically.

**Route rides on the road graph** (spec §2.7, §2.2.1): ride a route of the
package, or import a GPX file and ride it:

```bash
# Import a GPX track into the package (matched onto its roads; problems are reported)
world-builder gpx --world data/pilot/liechtenstein.ocwpkg --gpx ride.gpx --name "Rheintal"
ocw-desktop --world data/pilot/liechtenstein.ocwpkg --route Rheintal
# Or match a GPX file at the start, without storing it
ocw-desktop --world data/pilot/liechtenstein.ocwpkg --route-gpx ride.gpx
```

The junction overlay shows the route's exit in blue (it is the default).
Choosing another exit (← / →, Q / E) shows "[leaves the route]"; once it locks
you are off the route and ride free. J (or `ride.rejoin`) adds the shortest
way back to the route; riding onto a later part of the route in its direction
rejoins it too. Bots stay on the route. The HUD shows the route's progress
(and lap, for loops); after the finish of an open route you ride on freely.

**Surfaces:** asphalt, gravel (tracks and paths; the Hills loop has a gravel
climb at 1.5–2.1 km) and grass beside the road roll increasingly harder
(rolling resistance ×1, ×2.5, ×6); a smart trainer in SIM mode feels it too.

**Routes and worlds**

- Built-in test routes: `--route hills` (default; steep, changing gradients) or
  `--route loop` (gentle).
- Real roads: `--route-file data/pilot/vaduz-malbun.route` (from `world-builder
  route`), optionally with `--world data/pilot/liechtenstein.ocwpkg` (terrain,
  all roads of the region, streamed in 1 km tiles). The data comes
  from `scripts/fetch-pilot.sh` and `world-builder` (© OpenStreetMap
  contributors, ODbL).
- Packages from `world-builder package` also carry the OSM land use and the
  buildings: the ground is coloured by land use (forest floor, meadows,
  fields, settlements, rock, scree, glaciers), lakes and rivers are flat water
  surfaces, trees stand in the forests (a few on meadows, in gardens and
  parks, never on roads, water or in buildings) and every building is
  extruded from its OSM footprint with its height (`height`,
  `building:levels`, or a default by type) and a flat or gable roof.
  `world-builder check` lists what a package contains; older packages
  without land use still load (plain grass and placeholder tree patches).
- Default arguments: `ocw-desktop.args` beside the executable (or
  `$OCW_ARGS_FILE`) is read before the command line, which wins
  (`--no-args-file` ignores it; see config/README.md, kiosk).

**World packages: signatures and cache** (spec §4.7, §12)

Every package has a manifest next to it (`liechtenstein.ocwpkg.manifest.json`:
id, version, attributions, SHA-256 of the file and of each tile) and is
verified on every load. Damaged or tampered packages are always refused.
Packages not signed by a trusted key load from a **path** with a warning
(local builds), but from the **cache** only with `--allow-unsigned`;
`--require-signed` refuses them everywhere. Debug builds trust the development
root keys (`crates/ocw-worldstore/keys/dev-roots.json`), release builds only
with `--trust-dev-root`; more roots with `--trust-root FILE` or as
`~/.config/ocw/trusted-roots/*.pub.json`.

```bash
ocw-desktop --fetch-world https://HOST/worlds/europe/liechtenstein/3/manifest.json
ocw-desktop --fetch-world /media/usb/worlds/europe/liechtenstein/3/manifest.json
ocw-desktop --list-worlds
ocw-desktop --world europe/liechtenstein --free-ride     # newest version; or ID@VERSION
```

Downloads go to the cache (`~/.local/share/ocw/worlds/<id>/<version>/`,
`$OCW_WORLD_CACHE` or `--world-cache DIR`); the signature is checked before the
large download, interrupted downloads resume (HTTP range requests), and a
package is installed only when all its files match the manifest.
- Everyone (bots too) starts at a random point of the route
  (`--random-start false` starts at 0). Bots ride at 85–250 W and push harder
  uphill.

**Trainer and sensors**

- Without a remembered device the app rides with a **virtual trainer**
  (`--power`, PgUp/PgDn, `sim.power`). Bluetooth trainers and sensors (FTMS,
  Cycling Power, Cadence, Heart Rate, Wahoo trainers): `devices.scan`,
  `devices.connect N`; connected devices are remembered
  (`~/.config/ocw/devices.toml`) and reconnected at the next start
  (`--virtual` ignores them).
- Trainer control: **SIM** (default; the trainer simulates the road's
  gradient, scaled by `trainer.difficulty`, default 50 %), **ERG** (holds a
  power: `erg.target 200`), or `trainer.mode off`.
- ↑ adds power on top of any device (25 W per press), ↓ stops (power 0 and
  brake).

**Videos:** V starts and stops recording the window as an MP4 (H.264,
25 fps) into `~/Videos/OCW/` (or `~/.local/share/ocw/videos/`); slow frames
are repeated so the video keeps real time. Needs `ffmpeg` (`make deps`
installs it).

**Rides and Strava** (spec §2.8)

- Every ride is recorded and saved as a FIT file when the app closes (rides of
  at least a minute) or with `ride.save`: `~/.local/share/ocw/rides/`
  (`--record false` turns it off).
- Strava (optional; interim direct upload until the backend exists): create a
  Strava API app at <https://www.strava.com/settings/api> with the callback
  domain `localhost`, set `OCW_STRAVA_CLIENT_ID` / `OCW_STRAVA_CLIENT_SECRET`
  (or `client_id` / `client_secret` in `~/.config/ocw/strava.toml`), then
  `strava.connect` once. Saved rides upload in the background; failed uploads
  are retried.

## Command-line options (`ocw-desktop`)

| Option | Meaning |
|---|---|
| `--name NAME` | Display name of your rider (default `Rider`) |
| `--power W` | Initial power of the virtual trainer (default 200) |
| `--bots N` | Number of test riders (default 4) |
| `--route hills\|loop` | Built-in test route (default `hills`) |
| `--route NAME\|FILE` (with `--world`) | Ride a route of the package by name, or an `ocw-graph-route` file, on the road graph |
| `--route-gpx PATH` | Ride a GPX track or route, matched onto the `--world` package's roads at the start |
| `--route-file PATH` | Ride a route file from `world-builder route` (interim format) |
| `--world PATH\|ID` | World package: a file from `world-builder package`, or a cache id (`europe/liechtenstein`, `ID@VERSION`); with `--route-file`, `--route`, `--route-gpx` or `--free-ride` |
| `--fetch-world URL` | Download (or install from a directory) a world package into the cache: URL or path of its `manifest.json`; exits unless `--world` is given |
| `--list-worlds` | List the world packages in the cache and exit |
| `--world-cache DIR` | World package cache (default `~/.local/share/ocw/worlds`, or `$OCW_WORLD_CACHE`) |
| `--allow-unsigned` / `--require-signed` | Accept untrusted packages from the cache too / refuse them also from paths |
| `--trust-root FILE`, `--trust-dev-root` | Also trust this root public key / the development roots (debug builds always do) |
| `--free-ride` | Free ride on the package's road graph (needs `--world`) |
| `--start X,Z` | Free ride start: the road nearest local X,Z metres (default: random) |
| `--random-start BOOL` | Start at a random point (default on, off for screenshot/capture runs) |
| `--virtual` | Use the virtual trainer even if Bluetooth devices are remembered |
| `--devices-file PATH` | Remembered devices (default `~/.config/ocw/devices.toml`) |
| `--record BOOL` | Record the ride as a FIT file (default on, off for screenshot/capture runs) |
| `--window WxH` | Window size (default 1280x720) |
| `--fullscreen` | Borderless fullscreen on the current monitor |
| `--monitor SPEC` | Fullscreen on a monitor: index, resolution (`2560x1600`) or name part (`HDMI`) |
| `--cmd COMMAND` | Run a console command at startup (repeatable) |
| `--exec FILE` | Run the console commands in a file at startup (also `autoexec.cfg` if present) |
| `--at SECONDS=COMMAND` | Run a console command after SECONDS of riding (scripted rides) |
| `--screenshot PATH`, `--screenshot-delay S` | Save a screenshot after S seconds (default 5), then exit |
| `--capture DIR`, `--capture-fps N`, `--capture-seconds S`, `--capture-warmup S` | Save every frame with a fixed game-time step (smooth videos), then exit |

## Console commands

Open the console with `` ` `` or F12; `help` lists the commands, `help NAME`
shows one. *Dev* commands need the in-process local server (always the case
for now).

| Command | What it does |
|---|---|
| **General** | |
| `help [COMMAND]` | List commands, or show help for one |
| `console [open\|close\|toggle]` | Open, close or toggle the console |
| `clear` | Clear the console output |
| `echo TEXT` | Print text |
| `exec FILE` | Run the commands in a file, one per line |
| `fps [on\|off]` | Show or hide the FPS counter |
| `screenshot [PATH]` | Save a screenshot of the window (default `screenshot.png`) |
| `reset` | Start everything again as at program start (key R); the ride is saved first |
| `video [start\|stop\|toggle]` | Record the window as an MP4 (key V; needs `ffmpeg`) |
| `quit` | Exit the app |
| **Rider** | |
| `rider.autodrive on\|off` | Autodrive (key A) |
| `rider.wild on\|off` | Wild mode (key W) |
| `rider.lean DEGREES` | Lean into a curve (positive = right, 0 = upright; manual steering) |
| `rider.steer METRES` | Autodrive line: lateral position on the road (positive = right) |
| `rider.state` | Print your rider's state |
| `teleport KM` | *Dev:* move your rider to a distance on the route (free ride: along the path, taking default exits) |
| `teleport.xz X Z` | *Dev:* free ride: move to the road nearest local x / z metres |
| `ride.turn left\|right\|straight\|default\|+\|-\|N` | Free ride: choose the exit at the next junction |
| `ride.uturn` | Free ride: turn round (key U) |
| `ride.junction` | Free ride: show the next junction's exits (and the route's) |
| `ride.rejoin` | Route ride: back to the route after leaving it (key J) |
| `ride.route` | Route ride: name, progress and whether you are on the route |
| `net.stats` | Connection statistics |
| `worlds.info` | The loaded world package: id, version, content hash, trust, credits |
| `worlds.list` | World packages in the local cache, with their trust state |
| `worlds.verify [PATH\|ID]` | Verify a world package's hashes and signature (default: the loaded one) |
| **Camera** | |
| `camera.mode chase\|side\|top` | Camera perspective |
| `camera.orbit YAW PITCH [DISTANCE]` | Orbit the chase camera (degrees; 0 = behind, 180 = in front; metres) |
| **Trainer and devices** | |
| `trainer.mode sim\|erg\|off` | Trainer control |
| `trainer.difficulty PERCENT` | Scale the road gradient in SIM mode |
| `erg.target WATTS` | ERG mode with this target power |
| `sim.power WATTS` | *Dev:* the virtual trainer's power |
| `devices.scan` | Scan for Bluetooth trainers and sensors (in the background) |
| `devices.connect N\|ID` | Connect a device from the last scan, and remember it |
| `devices.list` | Running and remembered devices |
| `devices.forget ID\|all` | Forget a remembered device (from the next start) |
| `devices.virtual` | Start an additional virtual trainer |
| **Rides and Strava** | |
| `ride.status` | The current recording |
| `ride.save` | End the ride, save it as FIT (and upload if Strava is linked); a new recording starts |
| `strava.connect` | Link your Strava account (opens the browser) |
| `strava.status` | Strava link and upload state |
| `strava.sync [--retry]` | Upload every saved ride not on Strava yet (`--retry`: also failed ones) |
| `strava.disconnect` | Unlink Strava (deletes the stored tokens) |

## Running the server

**There is no network server yet, so there is no port to open.** Today every
ride runs on the *local server*: the same game-server logic (physics for all
riders, bots, junction decisions) runs inside the desktop app on its own
thread and talks to the client over in-process channels (spec §6.1). Hosting
a session for other people comes with Phase 5 (multiplayer): a QUIC (UDP)
game server with Quinn, plus an HTTPS API, accounts and a database; its port
and deployment (Docker) will be documented then.

What exists already is the **headless server**: the same server logic as a
standalone program, without graphics or network, for tests and simulations:

```bash
cargo run --release -p ocw-server -- --bots 5 --seconds 60          # test loop, real time
cargo run --release -p ocw-server -- --bots 20 --seconds 600 --fast # as fast as possible
cargo run --release -p ocw-server -- --world data/pilot/liechtenstein.ocwpkg \
    --bots 3 --seconds 300 --fast [--seed N]                       # free ride on a package
```

It prints the riders' positions and speeds while it runs. Options:
`--bots N` (test riders, default 3), `--seconds S` (0 = until stopped with
Ctrl+C), `--fast` (no real-time pacing), `--world PKG|ID` and `--seed N` (free
ride on a world package's road graph, start point seed; verified like in the
desktop app, `--allow-unsigned` and `--trust-root FILE` as there), `--route NAME`
(with `--world`: ride one of the package's routes).

Clients and the server must use the same build of a world package (junctions
are identified by vertex index): the Hello and Welcome carry the package name
and a fingerprint of its road graph, and a mismatch is refused.

## Build servers

All building and testing runs on remote machines over SSH
(`scripts/remote.sh`), described in `config/machines.toml` (git-ignored; keys
in `config/ssh/`). Details and background: [config/README.md](config/README.md).

### 1. On this machine: key and entry

```bash
cp config/machines.example.toml config/machines.toml      # once; then add an entry per machine
mkdir -p config/ssh && chmod 700 config/ssh
ssh-keygen -t ed25519 -N "" -C "ocw-build" -f config/ssh/ocw_build_ed25519   # one key per machine is best
```

An entry in `config/machines.toml` needs `name`, `os` (`linux` / `macos` /
`windows`), `host`, `user`, `identity_file`, `shell` and `repo_path`; see the
documented template.

### 2. On the build machine: SSH server, build user, key

**Linux (Debian / Ubuntu):**

```bash
sudo apt install openssh-server && sudo systemctl enable --now ssh
```

**Linux (Fedora):**

```bash
sudo dnf install openssh-server && sudo systemctl enable --now sshd
sudo firewall-cmd --permanent --add-service=ssh && sudo firewall-cmd --reload   # if firewalld is active
```

Then, as the build user (a separate account is best), install the public key:

```bash
mkdir -p ~/.ssh && chmod 700 ~/.ssh
echo '<contents of config/ssh/<key>.pub>' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys
```

**macOS:** *System Settings → General → Sharing → Remote Login* on (for the
build user), then add the public key to `~/.ssh/authorized_keys` as above.

**Windows (10 / 11, any language):** copy `scripts/setup-windows-buildserver.ps1`
and the public key (`.pub`, never the private key) to the machine, open
**PowerShell as Administrator** and run:

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass   # allow scripts in this window only
.\setup-windows-buildserver.ps1 -PublicKeyFile .\ocw_build_ed25519.pub
```

It installs and starts the OpenSSH server (automatic start, firewall rule,
PowerShell as login shell), creates the local, non-admin build user `claude`
(`-User NAME` to change; asks for its password) with the key, and a writable
repo folder (`-RepoPath`, default `C:\src\OCW`). It is safe to run again. To
allow scripts permanently for your user instead:
`Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned`, and
`Unblock-File .\setup-windows-buildserver.ps1` for a downloaded script.

Then, still **as Administrator**, install the build toolchain. Rust on
Windows links with Microsoft's linker, so the **Visual Studio Build Tools**
with the C++ workload (MSVC compiler and linker, Windows SDK) are required;
they are installed system-wide, which the non-admin build user cannot do
itself, and `winget` is usually not available to that user either:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
  --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

(Several GB; or use the installer *Build Tools for Visual Studio* from
visualstudio.microsoft.com and select **Desktop development with C++**.)
Git and make are optional: `remote.sh` copies the tree with `tar`, which
Windows has built in.

Rust itself is installed **as the build user** (no admin rights needed), e.g.
over SSH from this machine:

```bash
scripts/remote.sh <name> ssh 'Invoke-WebRequest https://win.rustup.rs/x86_64 -OutFile $env:TEMP\rustup-init.exe; & $env:TEMP\rustup-init.exe -y --default-toolchain none'
scripts/remote.sh <name> run 'rustup show active-toolchain'   # installs the pinned toolchain
```

Check the Windows machine: a *Private* network profile, sleep disabled
(*Settings → System → Power*), and a fixed IP address (e.g. a DHCP
reservation) – otherwise SSH fails with *no route to host*.

### 3. On this machine: host key and test

```bash
ssh-keyscan -p 22 <host> >> config/ssh/known_hosts                  # once (connections are verified strictly)
scripts/remote.sh <name> ssh 'uname -a'                              # Windows: '$PSVersionTable.PSVersion'
```

### 4. Build tools on the machine

| OS | Command (in the copied checkout, e.g. `scripts/remote.sh <name> run 'make …'`, or locally on the machine) |
|---|---|
| Linux | `make deps-linux` – detects Debian/Ubuntu (apt), Fedora (dnf) or Arch (pacman), installs the packages (sudo; run it yourself in a terminal there) and rustup with the pinned Rust; or the parts: `make deps-linux-debian` / `-fedora` / `-arch`, `make deps-rust` |
| macOS | `make deps-macos` – Xcode Command Line Tools, rustup, ffmpeg (if Homebrew is installed; otherwise it prints the commands to install Homebrew and ffmpeg, which needs an admin password) |
| Windows | `make deps-windows` – winget: Git, Visual Studio Build Tools (C++), rustup (needs GNU make, e.g. `winget install ezwinports.make`) |

rustup can also be installed without sudo and without touching shell
profiles: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain none`
(`remote.sh` loads `~/.cargo/env` itself).

### 5. Use it

```bash
scripts/remote.sh <name> run 'cargo test --workspace'        # copy the working tree, run there
scripts/remote.sh <name> run '.claude/skills/run-ocw/drive.sh shot-wayland /tmp/ocw.png 6 -- --virtual'
scripts/remote.sh <name> fetch test-output ./from-machine     # copy results back
scripts/deploy-test.sh <name>                                  # release build for the kiosk on its screen
```

- At most 2 `remote.sh run` commands run at once per machine (others wait;
  `OCW_REMOTE_SLOTS`), because parallel Bevy builds can run a machine out of
  memory; branch checkouts use 4 build jobs (`OCW_REMOTE_JOBS`).
- Branches other than `main` get their own checkout on the machine
  (`<repo_path>-<branch>`), several GB each; delete the ones of merged branches
  now and then.
- Screenshots and videos on machines with a real GPU: use the Wayland modes
  (`shot-wayland`, `video-wayland`; headless weston). Xvfb (X11) cannot
  present a real GPU's Vulkan output.
- The kiosk (the app permanently on a machine's screen, restarted on every
  deployment) is set up once on that machine's desktop: see
  [config/README.md](config/README.md#test-app-on-a-machines-screen-kiosk).

**Troubleshooting:** `Connection refused` – no SSH server running (step 2).
`Connection timed out during banner exchange` – the machine is overloaded or
hanging (e.g. out of memory). `Host key verification failed` – step 3 missing
or the machine was reinstalled (remove its old line from
`config/ssh/known_hosts`). On Windows, a key in the wrong file for an admin
account (`C:\ProgramData\ssh\administrators_authorized_keys`) or a network set
to *Public* are the usual causes; the server log is in Event Viewer under
*Applications and Services Logs → OpenSSH*.

## Tools and scripts

| Tool | Use |
|---|---|
| `make help` | All make targets: deps, build, test, check, smoke screenshots, videos, deploy |
| `cargo run -p ocw-server -- --bots 3 --seconds 10 --fast` | Headless game server (`--world PKG [--seed N]`: free ride on a package) |
| `world-builder graph` / `route` / `package` / `check` | OSM + elevation import: road graph statistics, route files, world packages, package validation (`cargo run --release -p world-builder -- help`) |
| `world-builder gpx --world PKG --gpx FILE [--name N] [--route-file F] [--dry-run]` | Import a GPX track: map-match it onto the package's road graph and store it as a package route (section `ROUT`; optionally also as an `ocw-graph-route` file); off-road stretches and gaps are reported |
| `world-builder route --world PKG --from LAT,LON --to LAT,LON --name N` | Store the shortest main-road route between two points as a package route |
| `world-builder routes --world PKG [--from OLD_PKG] [--remove N] [--export-gpx]` | List and check a package's routes; carry the routes of an older build over (by stable edge ID, re-matching broken ones); export them as GPX |
| `world-builder keys init-root` / `keys new-signing` / `sign` / `verify` / `publish` / `diff` | Package signing and distribution, see below |
| `scripts/fetch-pilot.sh` | Download the pilot region's OSM extract and elevation data |
| `scripts/remote.sh MACHINE run 'CMD'` | Copy the tree to a build machine (`config/machines.toml`) and run a command there |
| `python3 tools/build-dashboard/dashboard.py` | Web page (port 8765) with the build machines' load and their running/queued tasks; `--once` prints it as text |
| `scripts/deploy-test.sh MACHINE` | Release build on a test machine, deployed for its kiosk |
| `scripts/ocw-kiosk.sh install\|run\|status` | Keep the test app running on a machine's screen, restarted on each deploy |
| `scripts/build-appimage.sh` | Linux AppImage (needs `appimagetool`) |
| `scripts/build-macos-app.sh` | macOS app bundle, DMG and zip, ad-hoc signed (`make macos-app`, on a Mac) |
| `scripts/setup-windows-buildserver.ps1` | Windows build machine: OpenSSH server and a local build user |
| `.claude/skills/run-ocw/drive.sh` | Headless screenshots and videos (X11 / Wayland) for tests |

**Signing and publishing world packages** (spec §4.7): an offline root key
certifies signing keys; a signing key signs package manifests. Secret keys
(`*.key`) stay in `~/.config/ocw/keys` (`$OCW_KEY_DIR`), never in the
repository.

```bash
world-builder keys init-root --name "OCW root"                  # once; keep root.key offline
world-builder keys new-signing --name "builds 2026" --days 365  # signing key, certified by the root
world-builder package --id europe/liechtenstein --sign-key ~/.config/ocw/keys/builds-2026.key
world-builder sign --key ~/.config/ocw/keys/builds-2026.key data/pilot/liechtenstein.ocwpkg
world-builder verify data/pilot/liechtenstein.ocwpkg [--trust-root root.pub.json]
world-builder publish data/pilot/liechtenstein.ocwpkg --out /srv/worlds   # <id>/<version>/ layout
world-builder diff old.ocwpkg new.ocwpkg                        # stable edge IDs kept / removed / added
```

`package` always writes the manifest (unsigned without `--sign-key`); the
version counts up when the package content changes (`--pkg-version N` sets
it), `--dem-attribution TEXT` replaces the SRTM notice. Clients trust a root
by its public key file `root.pub.json`.

Release builds for Linux and macOS are in `release/`, screenshots and videos
from development in `artwork/build-history/`.

## Repository layout

| Path | Contents |
|---|---|
| `apps/desktop` | Desktop client: wires devices, local server, prediction and rendering |
| `services/server` | Game server logic, in-process local server, headless server binary |
| `crates/ocw-core` | Shared types, units, constants |
| `crates/ocw-sim` | Physics, session logic, client prediction, interpolation |
| `crates/ocw-protocol` | Protobuf protocol (`proto/`) and conversions |
| `crates/ocw-devices` | Device backends: virtual trainer, Bluetooth trainers and sensors (FTMS, Cycling Power, CSC, Heart Rate, Wahoo) |
| `crates/ocw-world` | Routes, road graph, terrain, world packages, surfaces |
| `crates/ocw-worldgen`, `apps/world-builder` | OSM + elevation import: road graph, profiles, world packages |
| `crates/ocw-worldstore` | World package files, keys, trusted roots, local cache, download |
| `crates/ocw-fit` | FIT activity file writer (ride export) |
| `crates/ocw-render` | Bevy plugins: world, riders, camera, HUD, console |
| `artwork/` | Logos and other original artwork |

## License

Code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
