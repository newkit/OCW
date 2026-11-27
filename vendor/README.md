# Vendored crates

Third-party crates kept in the tree because OCW needs a version that is not
published. They form their own Cargo workspace (`vendor/Cargo.toml`); the
root workspace excludes this directory, so its clippy / fmt rules do not
apply to upstream code.

| Crate | Origin | Version | Licence | Our changes |
|---|---|---|---|---|
| `bevy_mod_xr` | <https://github.com/awtterpip/bevy_oxr> (crates.io `bevy_mod_xr` 0.6.0) | 0.6.0 → `0.6.0-ocw.1` | MIT OR Apache-2.0 | Ported from Bevy 0.19 to Bevy 0.20.1 (spec §14 Phase 11 X1, Q3 variant A); clean manifest on our pinned Bevy crates |
| `bevy_mod_openxr` | <https://github.com/awtterpip/bevy_oxr> (crates.io `bevy_mod_openxr` 0.6.0) | 0.6.0 → `0.6.0-ocw.1` | MIT OR Apache-2.0 | Ported from Bevy 0.19 / wgpu 29 to Bevy 0.20.1 / wgpu 30.0.1 (spec §14 Phase 11 X1, Q3 variant A); clean manifest |

| `bevy_render` | <https://github.com/bevyengine/bevy> (crates.io `bevy_render` 0.20.1) | 0.20.1 (same version: `[patch.crates-io]` in the root `Cargo.toml`) | MIT OR Apache-2.0 | `render_resource/pipeline_cache.rs` only (spec 14 Phase 10, 10G *Bisecting a phone's crash*): `PipelineCache::compile_budget` (at most N pipelines start compiling per frame, the oldest first; a pipeline waiting for a shader or import does not count) and `PipelineCache::compile_log` (a callback with each attempt's label, shader, ms and outcome, `PipelineCompileReport`). Both `None` by default: Bevy's behaviour. And (v6.77) `render_resource/gpu_allocated.rs` with a counted field in `Texture` and `Buffer` (`texture.rs`, `buffer.rs`): the bytes of every texture and buffer made through the wrappers while it lives (`gpu_allocated()`, for the browser's page log; no change of behaviour). Drop the patch when upstream has an equivalent |

`bevy_render` is used by the main workspace through the patch (it is not a
member of `vendor/Cargo.toml`'s workspace). The first commit that added them holds the sources exactly as published on
crates.io; `git diff` against it shows the port. Check the port with

```bash
cd vendor && CARGO_TARGET_DIR=../target cargo check --workspace --examples
```
