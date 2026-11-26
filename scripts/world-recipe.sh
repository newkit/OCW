# The build recipe of an official package (spec 14 Phase 16, 16H Q17 (a)):
# sourced by scripts/build-*.sh after world-patches.sh.
#
#   recipe_start OUT_DIR       every world-builder run from here on appends
#                              its command line to OUT_DIR/.recipe-build.log
#                              (OCW_RECIPE_LOG); not for OCW_SRTM_ONLY builds
#   recipe_write PACKAGE REGION FETCH_REGION SCRIPT
#                              writes PACKAGE's <stem>.recipe.toml from the
#                              steps that made or changed it (package, routes,
#                              calibration, forest, vegetation) with their
#                              sources (size and SHA-256 of each file) and
#                              records it in the manifest – before signing
#
# OCW Studio's Build package runs the recipe's steps with the user's patch
# when it finds the named sources; world-builder publish and
# scripts/prepare-world-release.sh ship it next to the package.
recipe_start() {
  [ -z "${OCW_SRTM_ONLY:-}" ] || return 0
  export OCW_RECIPE_LOG="$1/.recipe-build.log"
  : > "$OCW_RECIPE_LOG"
}
recipe_write() {
  [ -n "${OCW_RECIPE_LOG:-}" ] || return 0
  $WB recipe write --world "$1" --log "$OCW_RECIPE_LOG" --region "$2" --fetch "$3" --script "$4"
}
