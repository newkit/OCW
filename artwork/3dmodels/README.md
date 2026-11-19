# artwork/3dmodels – the model files of Open Cycling World

Every 3D object with a fixed shape is a **glTF 2.0** file here (`.glb`), read
by the app at run time through `ocw_assets::library` (spec §14 Phase 15).
The files are **Git LFS** objects (`.gitattributes`; spec Q2): a clone needs
`git-lfs` (`make deps`) and, when its LFS filter skips downloads,
`git lfs pull`. A pointer file instead of a model is reported by the build
and the loader.

Own models are licensed **MIT OR Apache-2.0** like the rest of `artwork/`.
Third-party models are welcome when their licence asks no more than
attribution (CC0-1.0, CC-BY-4.0, MIT, Apache-2.0, BSD; spec Q3), with the
author, source URL and attribution text in `manifest.toml`; never
share-alike, non-commercial or no-derivatives licences, never brands.

## Folders

| Folder | Kind | Contents |
|---|---|---|
| `bikes/` | `bike` | one bike per physics profile, with `lod1` |
| `riders/` | `rider` | the rider (`rider-body`) and its far model (`rider-body-low`) |
| `wear/` | `wearable` | helmets, the status signs of spec §2.6 (headphones, VR headset, the X over a muted mouth), later glasses and caps, worn at an anchor |
| `parts/<slot>/` | `part` | catalogue parts by workshop slot (later, M3) |
| `props/` | `prop` | the point obstacles (`bollard-round`, `post-hexagonal`, `pylon-pyramid`, `block-concrete-box`, `gate-swing`, `barrier-lift-arm`, `barrier-cycle-staggered`, `barrier-post-hexagonal` for any other barrier), the street lamps (`lamp-street-arm-main` 8 m, `-road` 6 m, `-path` 4.5 m) – drawn by the game from these files (spec 15H MX); the phone of the status signs (held in the hand: its origin is its centre) |
| `flora/` | `plant` | at least three shape variants per plant species, each with `lod1` |
| `creatures/` | `creature` | the unicorn (later, M4) |
| `workshop/` | `workshop` | the workshop's pieces (later, M3) |
| `sources/` | – | Blender sources (`.blend`), never loaded |

## The contract (spec §14 Phase 15, 15B)

- **Units and axes:** metres, +Y up, glTF's +Z is the model's front (the app
  turns it to its −Z). In Blender the model faces −Y (the *Front* view) and is
  exported with "+Y up".
- **Origin per kind:**

  | Kind | Origin |
  |---|---|
  | bike | ground contact under the bottom bracket |
  | rider | between the sit bones |
  | wearable | its anchor on the rider (`head`, `face`, `foot_l`, …) |
  | part | its hotspot `hs_<slot>` on the bike |
  | prop | the foot centre, on the ground |
  | plant | the stem base |
  | creature | under the body's centre |
  | workshop | the piece's floor point |

- **Node names are the interface:** anchors `anchor_<name>`, workshop
  hotspots `hs_<slot>`, moving parts by role (`wheel_front`, `crank`,
  `pedal_l`, the rider's rig names), LOD nodes `lod1` (plants: also `lod2`,
  `billboard` later), collision proxies `col_<shape>` (never drawn).
- **Materials are paint slots** (`ocw_assets::slots`): `frame`, `jersey`,
  `helmet`, … for bikes and riders; `leaf`, `needle`, `bark`, `dry` for
  plants. The app tints the slots (the rider's look; a plant's own shade and
  the season); colours in the file are the defaults. Vertex colours
  (`COLOR_0`) multiply the slot's colour, as in glTF (the plants' shading).
- **Metadata in `extras`, never physics:** the asset carries
  `extras.ocw = { "schema": 1, "kind": …, "id": … }` plus per kind: a bike's
  `profile`, its `frame` geometry numbers and `sizing` (the reference rider
  height and the saddle travel: the loader fits the frame to every rider
  height, so a new size needs no new file – spec Q7); a plant's `species`,
  `variant`, `height_m`; a wearable's `anchor`; a prop's `osm` tag and its
  `fit` (below). Physics keys (`mass`, `cda`, `crr`, …) are rejected.
- **How a prop is fitted to its place** (`ocw_assets::props`): `extras.ocw.fit
  = { height_m, sink_m, span_m?, reach_m? }` – the file's height above the
  ground, how deep it is set into the ground (the mesh starts at the origin;
  the game lowers it by `sink_m`), the width a gate spans across (model X),
  the reach of a lamp's arm (model +X). A bollard of another height is
  scaled vertically by `(h + sink) / (height + sink)`. Node extras
  `ocw.span` = `stretch` (default: bars, arms) / `move` (posts keep their
  thickness), `ocw.reach` = `stretch` (the arm, from its node's origin) /
  `move` (the head and its `anchor_light`), `ocw.ground` = `model` (default:
  on the ground under the origin) / `node` (a post on the ground under its
  own origin) / `drape` (bars and rails following the slope); children
  inherit them. Collision proxies `col_box*` / `col_cylinder*` /
  `col_capsule*` are either a modelled mesh (its bounds) or an empty (the
  unit shape, half extents 1, scaled by the node) – both are read
  (`props::proxies`; spec 15H Q9 is open); the exported props use empties.
- **Budgets** (triangles; the validator checks them):

  | Object | Near | Far (`lod1`) |
  |---|---|---|
  | bike | 15 000 | 4 000 |
  | rider | 8 000 | 4 000 |
  | creature | 3 000 | 600 |
  | wearable | 1 500 | – |
  | part | 1 500 | – |
  | prop | 1 500 | 300 |
  | plant | 2 000 | 200 |
  | workshop piece | 10 000 | – |

  Textures (later): at most 1024², power of two, embedded. The validator
  warns above 2 MB per file.
- **Names: function first, then shape:**
  `<folder>/<function>-<shape>[-<detail>][-<variant>].glb`, lower-case ASCII
  letters and digits with hyphens, no colours, no versions. The id of a file
  is its path without `.glb` (`bikes/bike-tt`).
- **Deterministic files:** the writer gives byte-identical files for the same
  model (no timestamps; `asset.generator = "ocw-assets (Open Cycling World)"`),
  so a re-export without changes leaves git clean.

## The manifest

`manifest.toml` has one `[[asset]]` per file: `file`, `kind`, `license`,
`source`, `author`, and per kind `profile` (bike), `species` and `variant`
(plant), `anchor` (wearable); `generator` names the code a file was exported
from. Files without `generator` were made as files (spec Q6: the file is the
source; the status signs of §2.6): the exporters keep their entries.
It is sorted by `file`; the exporters keep it so.

## Tools

```bash
cargo run -p ocw-assets -- validate artwork/3dmodels [--json]   # the contract, licences, budgets (findings with fixes)
cargo run -p ocw-assets -- info artwork/3dmodels/bikes/bike-tt.glb
cargo run -p ocw-assets -- model diff A B [--json]      # structural diff of two files or ids (exit 1: they differ)
cargo run -p ocw-assets -- model export bikes/bike-tt [--gltf] --out DIR   # an editable copy for Blender
cargo run -p ocw-assets -- model import FILE.glb --kind prop --id post-square [--license … --source … --author …] [--dry-run]
cargo run -p ocw-studio -- --model bikes/bike-tt          # the Models workspace: view, stats, findings, try-on
cargo run -p ocw-assets -- model export-all [--check]    # bikes, riders, helmets from the generators
cargo run -p ocw-render --example export-flora [-- --check]   # the plants
# props/: the obstacle and lamp files are the source (exported once, spec 15H MX; Q6)
ocw-desktop --models artwork/3dmodels                     # read the files from disk (also $OCW_MODELS)
OCW_FLORA=files ocw-desktop …                             # draw the plants from their files
```

Until the generators are removed (spec Q6), the tests check that every
exported file is what its generator makes, mesh by mesh and byte for byte.

## Blender

Import: *File → Import → glTF 2.0*. Export: *File → Export → glTF 2.0* with
**glTF Binary (.glb)**, *Include → Selected Objects* and *Custom Properties*
on, *Transform → +Y Up* on, *Mesh → Apply Modifiers* on, *Materials →
Export*, no Draco compression. Keep material names as paint-slot names and
node names as they are (Blender adds `.001` to duplicate names: rename them
back). Then validate the folder before committing.

`model import` does the clean-up for you (spec §14 Phase 15, 15D): it strips
Blender's `.001` suffixes where the plain name is free, renames materials
that are close to a paint slot (`Frame.001`, `Bar Tape`), merges a node's
primitives per material, drops unused materials, computes missing normals,
stamps `extras.ocw` and writes the canonical file with its manifest entry. A
model lying on its back (exported without *+Y Up*) or with any contract error
is refused with the findings and their fixes.
