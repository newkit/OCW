# The project's own world patches for a package build (spec 14 Phase 16,
# 16C *Signing and trust*, 16H Q10): sourced by scripts/build-*.sh.
#
#   world_patches REGION DATA_DIR STEM
#
# sets PATCH_ARGS to `--patch FILE ...` for the patches of package STEM:
# worlds/REGION/patches/STEM.ocwpatch first, then STEM.*.ocwpatch in name
# order (a stack, e.g. `blackforest-north.020-bridge.ocwpatch`). The folder
# is in git and reviewed like code. The old place, DATA_DIR/patches/ (not in
# git), is still read with a notice when the new one holds none for STEM.
# OCW_NO_PATCHES=1 builds without them.
world_patches() {
  local region=$1 data=$2 stem=$3 dir f
  PATCH_ARGS=()
  [ -z "${OCW_NO_PATCHES:-}" ] || return 0
  for dir in "worlds/$region/patches" "$data/patches"; do
    [ -d "$dir" ] || continue
    for f in "$dir/$stem.ocwpatch" "$dir/$stem".*.ocwpatch; do
      [ -f "$f" ] && PATCH_ARGS+=(--patch "$f")
    done
    if [ ${#PATCH_ARGS[@]} -gt 0 ]; then
      if [ "$dir" = "$data/patches" ]; then
        echo "notice: world patches for $stem read from the old place $dir/ (not in git); move them to worlds/$region/patches/ (spec 16H Q10)" >&2
      fi
      echo "world patches for $stem: ${PATCH_ARGS[*]}" >&2
      return 0
    fi
  done
}
