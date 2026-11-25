# World sources in git

The project's own changes to the official worlds, versioned and reviewed like
code (spec 14 Phase 16, 16C *Signing and trust*, 16H Q10). The large
downloads the worlds are built from (OSM extracts, elevation models,
imagery) stay in the git-ignored `data/<region>/` of the build machines.

| Folder | Region | Packages (built by) | Data folder |
|---|---|---|---|
| `liechtenstein/patches/` | Liechtenstein | `liechtenstein` (`scripts/build-liechtenstein.sh`) | `data/pilot/` |
| `blackforest/patches/` | Black Forest | `blackforest`, `blackforest-middle`, `blackforest-north` (`scripts/build-blackforest.sh`) | `data/blackforest/` |
| `tenerife/patches/` | Tenerife | `tenerife` (`scripts/build-tenerife.sh`) | `data/tenerife/` |

Each `patches/` folder holds world patches (`.ocwpatch`, spec 16C *Format*)
for the packages of its region; its README says how they are named,
checked and used.
