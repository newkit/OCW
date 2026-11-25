# Black Forest – our world patches

The fixes the project applies to the official Black Forest packages (`blackforest`, `blackforest-middle`, `blackforest-north`),
as world patches (`.ocwpatch`, spec 14 Phase 16, 16C *Format*). They live in
git and are **reviewed like code** (spec 16C *Signing and trust*, 16H Q10):
an official package is signed by our root only after its patches are merged
here.

- **Names:** `<package>.ocwpatch` is the patch of package `<package>`
  (e.g. `blackforest-north.ocwpatch`); more patches for it stack as
  `<package>.<NNN>-<topic>.ocwpatch` (e.g. `blackforest-north.010-bridge-heights.ocwpatch`),
  applied after it in name order. The header's `package` is the package id
  (`world-builder` refuses a patch for another package).
- **Build:** `scripts/build-blackforest.sh` passes them to `world-builder package --patch`
  (`scripts/world-patches.sh`; `OCW_NO_PATCHES=1` builds without them). The
  old place, `data/blackforest/patches/` (not in git), is still read with a notice
  when this folder has none for a package – move such patches here.
- **With the package:** the build puts the patches it used next to the
  package (`<package>.ocwpatch`, `<package>.2.ocwpatch`, …) and records their
  SHA-256 in its manifest; `world-builder publish` and
  `scripts/prepare-world-release.sh` ship them with it (16H Q9) – a patch
  changes OpenStreetMap-derived data, so it is part of the Derivative
  Database and offered under the ODbL (16C *Licences*).
- **Before a merge:** `world-builder patch check --world data/blackforest/<package>.ocwpkg
  --pbf … FILE` lists every operation as applied or MISSING; a review checks
  what each operation changes and why, and that imported data (e.g. GPX
  geometry) is our own or openly licensed and named in a `[[source]]` entry.
- **Edit:** OCW Studio opens a package with its patch (World workspace, E5),
  saves `<package>.ocwpatch` next to it; copy the result here for review.
