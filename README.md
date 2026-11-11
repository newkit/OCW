# Open Cycling World

A cross-platform indoor cycling simulation in Rust: real bicycle hardware, real
routes from OpenStreetMap, rendered locally with Bevy, with server-authoritative
multiplayer.

**Status:** Phase 1 (technical foundation). The requirements and architecture are in
[SPECIFICATION.md](SPECIFICATION.md).

## Build and run (Linux)

```bash
make deps     # this platform: make deps-linux | deps-windows | deps-macos (packages + pinned Rust)
make check    # fmt, clippy, tests, wasm build – what CI runs
make run      # desktop client with a virtual trainer and 4 test riders
make help     # all targets (build, release, test, server, screenshot, ...)
```

The Rust version is pinned in `rust-toolchain.toml`; Debian's system Rust is too
old for Bevy. `make` finds the rustup toolchain in `~/.cargo/bin` by itself; for
plain `cargo`, add `. "$HOME/.cargo/env"` to your shell profile.

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
