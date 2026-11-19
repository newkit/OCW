#!/usr/bin/env bash
# Tests scripts/models-diff.sh (`make models-diff`) in a throwaway git
# repository with a few of the committed models as LFS objects (when
# git-lfs is installed): a changed file, a removed one, an untracked new
# one, a manifest entry changed without its file, nothing changed, and the
# staged form a build machine diffs (--stage, a tar, --staged).
# OCW_ASSETS names a built ocw-assets (default: cargo build -p ocw-assets).
set -euo pipefail
cd "$(dirname "$0")/.."
src=$PWD
fail=0
check() { if printf '%s\n' "$1" | grep -qF -- "$2"; then echo "ok   $3"; else echo "FAIL $3 (no '$2' in:)"; printf '%s\n' "$1" | sed 's/^/     | /'; fail=1; fi; }
check_not() { if printf '%s\n' "$1" | grep -qF -- "$2"; then echo "FAIL $3 ('$2' found)"; fail=1; else echo "ok   $3"; fi; }

if [ -z "${OCW_ASSETS:-}" ]; then
  ${CARGO:-cargo} build -q -p ocw-assets --bin ocw-assets
  OCW_ASSETS=$src/${CARGO_TARGET_DIR:-target}/debug/ocw-assets
fi
export OCW_ASSETS
head -c 4 artwork/3dmodels/bikes/bike-tt.glb | grep -q glTF || { echo "FAIL: the models are LFS pointers here (git lfs pull)"; exit 1; }

d=$(mktemp -d "${TMPDIR:-/tmp}/test-models-diff.XXXXXX")
trap 'rm -rf "$d"' EXIT
r=$d/repo
mkdir -p "$r/scripts" "$r/artwork/3dmodels/bikes" "$r/artwork/3dmodels/props"
cp scripts/models-diff.sh "$r/scripts/"
cp artwork/3dmodels/manifest.toml "$r/artwork/3dmodels/"
for f in bikes/bike-tt.glb bikes/bike-gravel.glb props/phone-slab.glb; do cp "artwork/3dmodels/$f" "$r/artwork/3dmodels/$f"; done
g() { git -C "$r" -c user.name="Ada Rider" -c user.email=ada@example.invalid "$@"; }
g init -q -b main
lfs=no
if git lfs version >/dev/null 2>&1; then
  lfs=yes
  g lfs install --local >/dev/null
  echo 'artwork/3dmodels/**/*.glb filter=lfs diff=lfs merge=lfs -text' >"$r/.gitattributes"
fi
g add -A && g commit -qm base
if [ $lfs = yes ]; then
  check "$(g show HEAD:artwork/3dmodels/bikes/bike-tt.glb | head -c 40)" "version https://git-lfs" "the base files are LFS pointers"
fi

out=$(OCW_MODELS_DIFF_ROOT=$r "$r/scripts/models-diff.sh" 2>&1)
check "$out" "no model file changed" "nothing changed"

g checkout -q -b feature
cp artwork/3dmodels/bikes/bike-road-aero.glb "$r/artwork/3dmodels/bikes/bike-tt.glb"
g commit -qam "a committed change"
g rm -q artwork/3dmodels/props/phone-slab.glb
mkdir -p "$r/artwork/3dmodels/props"
cp artwork/3dmodels/props/phone-slab.glb "$r/artwork/3dmodels/props/phone-slab-copy.glb"
sed -i '/file = "bikes\/bike-gravel.glb"/,/^author/ s/^author = .*/author = "Bea Climber"/' "$r/artwork/3dmodels/manifest.toml"
out=$(OCW_MODELS_DIFF_ROOT=$r "$r/scripts/models-diff.sh" 2>&1)
check "$out" "Model files changed against main" "the header names the base"
check "$out" "~ artwork/3dmodels/bikes/bike-tt.glb" "a committed change is listed"
check "$out" 'extras ocw.profile: "tt" → "road-aero"' "its structural diff is shown"
check "$out" "- artwork/3dmodels/props/phone-slab.glb (removed)" "a removed file"
check "$out" "+ artwork/3dmodels/props/phone-slab-copy.glb (added" "an untracked new file"
check "$out" "~ artwork/3dmodels/manifest.toml" "the manifest changed"
check "$out" '+author = "Bea Climber"' "the changed entry line"
check_not "$out" "bike-gravel.glb (" "an unchanged file is not listed"
check_not "$out" "version https://git-lfs" "no pointer reaches the diff"

# The staged form: git here, the diff elsewhere (a tar, no git needed).
OCW_MODELS_DIFF_ROOT=$r "$r/scripts/models-diff.sh" --stage "$d/stage" >/dev/null
tar -cf "$d/stage.tar" -C "$d/stage" .
rm -rf "$d/stage"
staged=$(OCW_MODELS_DIFF_ROOT=$r "$r/scripts/models-diff.sh" --staged "$d/stage.tar" 2>&1)
check "$staged" 'extras ocw.profile: "tt" → "road-aero"' "--staged TAR gives the same diff"
[ "$staged" = "$out" ] && echo "ok   --staged output equals the direct one" || { echo "FAIL --staged output differs"; diff <(echo "$out") <(echo "$staged") | head; fail=1; }

out=$(OCW_MODELS_DIFF_ROOT=$r "$r/scripts/models-diff.sh" --base nosuchbranch 2>&1) && st=0 || st=$?
check "$st" 2 "an unknown base exits 2"
exit $fail
