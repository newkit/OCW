#!/usr/bin/env bash
# The Windows VR kiosk's deploy (spec §14 Phase 11, 11I): what
# `scripts/deploy-test.sh MACHINE` runs for a Windows machine (os =
# "windows" in config/machines.toml, e.g. Weela with the Meta Quest).
#
#   scripts/deploy-test.sh Weela        (or scripts/deploy-test-windows.sh Weela)
#
# 1. On the machine: a release build with VR (`cargo build --release -p
#    ocw-desktop --features vr`) and the material library (`cargo run -p
#    ocw-assets -- materials fetch`); it says which world the kiosk rides
#    and the SHA-256 of the world files it has (scripts/ocw-kiosk.ps1
#    worldinfo).
# 2. Here: the Khronos OpenXR loader, pinned (scripts/fetch-openxr-loader.sh),
#    and the world files that differ - the package, its manifest and the
#    route files - from the Linux kiosk on OCW_WORLD_SOURCE (default marge,
#    its install folder /var/tmp/ocw), through target/kiosk-windows/ here;
#    sent to the machine and their SHA-256 compared there with the source's.
# 3. On the machine: scripts/ocw-kiosk.ps1 deploy installs the build into
#    the kiosk folder (%LOCALAPPDATA%\OCW\kiosk) beside the previous one
#    and switches to it last; the running kiosk restarts the app on it.
#
# Environment (as scripts/deploy-test.sh): OCW_KIOSK_WORLD (liechtenstein,
# blackforest, tenerife; default: keep the kiosk's, else tenerife),
# OCW_KIOSK_ROUTE, OCW_KIOSK_ARGS (replaces world and show mode; --vr
# --fullscreen and kiosk-local.args stay), OCW_DEPLOY_WORLD=0 (send no world
# files), OCW_WORLD_SOURCE / OCW_WORLD_SOURCE_DIR (where the world files
# come from), OCW_DEPLOY_FEATURES (default "vr").
set -euo pipefail
cd "$(dirname "$0")/.."
machine=${1:?usage: deploy-test-windows.sh MACHINE}
version="$(scripts/build-id.sh) $(date '+%F %T')"
kiosk_world=${OCW_KIOSK_WORLD:-keep}
case "$kiosk_world" in liechtenstein|blackforest|tenerife|keep) ;; *) echo "OCW_KIOSK_WORLD: liechtenstein, blackforest or tenerife" >&2; exit 2 ;; esac
src=${OCW_WORLD_SOURCE:-marge}
src_dir=${OCW_WORLD_SOURCE_DIR:-/var/tmp/ocw}
features=${OCW_DEPLOY_FEATURES:-vr}
local_stage=target/kiosk-windows
remote() { OCW_REMOTE_DEPLOY=1 scripts/remote.sh "$machine" "$@"; }
b64() { printf '%s' "$1" | base64 -w0; }
# A PowerShell single-quoted literal.
psq() { local q="'"; printf "'%s'" "${1//$q/$q$q}"; }

# The world files of a world on the source, in order of preference for the
# package (Black Forest: the north package, else the south one); the
# route files go with every world. Prints file names, one per line.
# `world_files NAME LISTING` (LISTING: the source's file names).
world_files() {
  local name=$1 listing=$2 pkg=""
  case "$name" in
    tenerife) pkg=tenerife.ocwpkg ;;
    liechtenstein) pkg=liechtenstein.ocwpkg ;;
    blackforest)
      if grep -qx blackforest-north.ocwpkg <<<"$listing"; then pkg=blackforest-north.ocwpkg; else pkg=blackforest.ocwpkg; fi ;;
  esac
  grep -qx "$pkg" <<<"$listing" || return 0
  echo "$pkg"
  grep -qx "$pkg.manifest.json" <<<"$listing" && echo "$pkg.manifest.json"
  grep '\.route$' <<<"$listing" || true
}
[ "${1:-}" = --world-files ] && { world_files "$2" "$3"; exit 0; }

echo "== build on $machine (release, features: $features)"
out=$(remote run "cargo build -q --release -p ocw-desktop --features $features; if (\$LASTEXITCODE -ne 0) { exit 1 }; \$env:OCW_MATERIALS_OPTIONAL = '1'; cargo run -q --release -p ocw-assets -- materials fetch; if (\$LASTEXITCODE -ne 0) { Write-Warning 'material library not fetched: the app draws flat colours' }; Set-ExecutionPolicy -Scope Process Bypass -Force; & .\\scripts\\ocw-kiosk.ps1 worldinfo -World $kiosk_world" | tr -d '\r' | tee /dev/stderr) || { echo "build on $machine failed" >&2; exit 1; }
world=$(sed -n 's/^@@world //p' <<<"$out" | tail -n 1)
kdir=$(sed -n 's/^@@dir //p' <<<"$out" | tail -n 1)
have=$(sed -n 's/^@@sha //p' <<<"$out")
[ -n "$world" ] || { echo "no world from ocw-kiosk.ps1 worldinfo" >&2; exit 1; }
echo "== the kiosk rides: $world (kiosk folder $kdir)"

mkdir -p "$local_stage/worlds"
rm -f "$local_stage"/send/* 2>/dev/null || true
mkdir -p "$local_stage/send"
scripts/fetch-openxr-loader.sh "$local_stage/send" >/dev/null
echo "OpenXR loader: $(sha256sum "$local_stage/send/openxr_loader.dll" | cut -c1-16)…"

expect=""
if [ "${OCW_DEPLOY_WORLD:-1}" != 0 ]; then
  listing=$(scripts/remote.sh "$src" ssh "cd $(printf '%q' "$src_dir") && ls -1")
  mapfile -t files < <(world_files "$world" "$listing")
  if [ ${#files[@]} -eq 0 ]; then
    echo "warning: $world is not installed in $src_dir on $src: no world files sent" >&2
  else
    sums=$(scripts/remote.sh "$src" ssh "cd $(printf '%q' "$src_dir") && sha256sum $(printf '%q ' "${files[@]}")")
    need=()
    while read -r sum name; do
      grep -qx "$sum $name" <<<"$have" && continue
      need+=("$name"); expect+="$sum $name"$'\n'
    done <<<"$sums"
    if [ ${#need[@]} -eq 0 ]; then
      echo "world files: all ${#files[@]} up to date on $machine"
    else
      echo "world files to send: ${need[*]}"
      fetch=()
      for n in "${need[@]}"; do
        want=$(awk -v n="$n" '$2 == n { print $1 }' <<<"$sums")
        [ -f "$local_stage/worlds/$n" ] && [ "$(sha256sum "$local_stage/worlds/$n" | cut -d' ' -f1)" = "$want" ] || fetch+=("$n")
      done
      if [ ${#fetch[@]} -gt 0 ]; then
        echo "copying ${#fetch[@]} file(s) from $src"
        scripts/remote.sh "$src" ssh "tar -cf - -C $(printf '%q' "$src_dir") $(printf '%q ' "${fetch[@]}")" | tar -xf - -C "$local_stage/worlds"
      fi
      for n in "${need[@]}"; do
        want=$(awk -v n="$n" '$2 == n { print $1 }' <<<"$sums")
        [ "$(sha256sum "$local_stage/worlds/$n" | cut -d' ' -f1)" = "$want" ] || { echo "$n: SHA-256 differs from $src after copying" >&2; exit 1; }
      done
      paths=(); for n in "${need[@]}"; do paths+=("$local_stage/worlds/$n"); done
      echo "sending them to $machine"
      remote put "${paths[@]}" target/kiosk-stage/worlds
    fi
  fi
fi
remote put "$local_stage/send/openxr_loader.dll" "$local_stage/send/openxr_loader.LICENSE.txt" target/kiosk-stage

args_ps='$null'
[ "${OCW_KIOSK_ARGS+set}" = set ] && args_ps="[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$(b64 "$OCW_KIOSK_ARGS")'))"
echo "== install on $machine"
# The PowerShell side as one base64 script block (remote.sh sends one line).
install_ps=$(cat <<PS
\$ErrorActionPreference = 'Stop'
Set-ExecutionPolicy -Scope Process Bypass -Force
\$st = 'target\\kiosk-stage'
New-Item -ItemType Directory -Force \$st | Out-Null
# The world files sent: their SHA-256 must match the source's.
\$expect = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$(b64 "$expect")'))
foreach (\$l in (\$expect -split "\`n")) {
  if (-not \$l.Trim()) { continue }
  \$sum, \$name = \$l.Split(' ', 2)
  \$got = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path \$st "worlds\\\$name")).Hash.ToLower()
  if (\$got -ne \$sum) { throw "\$name : SHA-256 differs after copying" }
  Write-Host "checked \$name \$sum (as on $src)"
}
Copy-Item -Force target\\release\\ocw-desktop.exe, scripts\\ocw-kiosk.ps1, scripts\\kiosk-show.args \$st
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path \$st 'materials')
if (Test-Path assets\\materials\\manifest.toml) {
  New-Item -ItemType Directory -Force (Join-Path \$st 'materials') | Out-Null
  Copy-Item assets\\materials\\*.ktx2, assets\\materials\\manifest.toml, assets\\materials\\CREDITS.md (Join-Path \$st 'materials')
}
\$route = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$(b64 "${OCW_KIOSK_ROUTE-}")'))
\$a = $args_ps
if (\$null -ne \$a) {
  & .\\scripts\\ocw-kiosk.ps1 deploy -Stage \$st -Version $(psq "$version") -World $kiosk_world -Route \$route -ArgsText \$a
} else {
  & .\\scripts\\ocw-kiosk.ps1 deploy -Stage \$st -Version $(psq "$version") -World $kiosk_world -Route \$route
}
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path \$st 'worlds')
\$global:LASTEXITCODE = 0
PS
)
remote run "& ([scriptblock]::Create([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$(b64 "$install_ps")'))))"
