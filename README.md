# Open Cycling World

A cross-platform indoor cycling simulation in Rust: real bicycle hardware, real
routes from OpenStreetMap, rendered locally with Bevy, with server-authoritative
multiplayer.

**Status:** Phase 1 (technical foundation). The requirements and architecture are in
[SPECIFICATION.md](SPECIFICATION.md).

## Build and run (Linux)

Install the system packages (Debian/Ubuntu):

```bash
sudo apt-get install -y libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-0 libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev mesa-vulkan-drivers
```

Install Rust with [rustup](https://rustup.rs). The toolchain version is pinned in
`rust-toolchain.toml` and installed automatically.

```bash
cargo run -p ocw-desktop            # desktop client with a virtual trainer and 4 test riders
cargo run -p ocw-server -- --help   # headless game server
cargo test --workspace
```

In the desktop client, PgUp/PgDn change the virtual trainer's power. `` ` `` or
F12 opens the console (`help` lists the commands).

## Repository layout

| Path | Contents |
|---|---|
| `apps/desktop` | Desktop client: wires devices, local server, prediction and rendering |
| `services/server` | Game server logic, in-process local server, headless server binary |
| `crates/ocw-core` | Shared types, units, constants |
| `crates/ocw-sim` | Physics, session logic, client prediction, interpolation |
| `crates/ocw-protocol` | Protobuf protocol (`proto/`) and conversions |
| `crates/ocw-devices` | Device backends (virtual trainer; Bluetooth FTMS in Phase 2) |
| `crates/ocw-world` | Routes and road geometry |
| `crates/ocw-render` | Bevy plugins: world, riders, camera, HUD, console |

## License

Code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
