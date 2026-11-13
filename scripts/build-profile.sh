#!/usr/bin/env bash
# Build profiling (spec §11.2): clean, no-op and incremental builds of the
# workspace's main cargo commands, with cargo's --timings reports. Runs in a
# checkout on a Linux or macOS build machine, e.g.
#
#   scripts/remote.sh marge run scripts/build-profile.sh --label marge-before
#   scripts/remote.sh marge fetch test-output/build-profile test-output
#
#   --label TEXT      name of the run (default: the OS); the results go to
#                     test-output/build-profile/LABEL/
#   --commands LIST   comma-separated subset of build,test,clippy,release,wasm
#                     (default: build,test,clippy,release)
#   --steps LIST      subset of clean,noop,touch,edit (default:
#                     clean,noop,touch,edit)
#   --target DIR      the cargo target directory to use (default:
#                     target/build-profile; removed before `clean` and after
#                     each command unless --keep)
#   --keep            keep the target directory between commands (then
#                     `clean` is only clean for the first command)
#   --config KEY=VAL  extra `cargo --config` (repeatable), e.g. to try a
#                     profile setting: --config 'profile.dev.debug="line-tables-only"'
#
# The commands:
#   build    cargo build --workspace
#   test     cargo test --workspace --no-run
#   clippy   cargo clippy --workspace --all-targets
#   release  cargo build --release -p ocw-desktop --features ocw-desktop/voice
#            (CC=clang-21 CXX=clang++-21 where clang-21 is installed)
#   wasm     cargo check of the wasm crates (make test-wasm)
# Steps: clean (empty target directory), noop (again, nothing changed),
# touch (touch one file each in ocw-world, ocw-render and apps/desktop, a
# build after each: unchanged content, rustc's incremental cache answers),
# edit (append a private function to each of these files, build, measured;
# then restore the file and build again, not measured).
#
# Output in test-output/build-profile/LABEL/: results.jsonl (one line per
# build: command, step, seconds, crates compiled, load average), the
# --timings HTML reports (COMMAND-STEP.html) and summary.md (a table and the
# slowest crates of each clean build, from the reports' unit data).
# CARGO_BUILD_JOBS and RUSTC_WRAPPER from the environment apply (remote.sh
# sets them).
set -euo pipefail
# Decimal points in the numbers (a German locale prints 8,0).
export LC_ALL=C
cd "$(dirname "$0")/.."

label=$(uname -s | tr '[:upper:]' '[:lower:]')
commands=build,test,clippy,release
steps=clean,noop,touch,edit
tdir=target/build-profile
keep=0
config=()
while [ $# -gt 0 ]; do
  case "$1" in
    --label) label=$2; shift 2 ;;
    --commands) commands=$2; shift 2 ;;
    --steps) steps=$2; shift 2 ;;
    --target) tdir=$2; shift 2 ;;
    --keep) keep=1; shift ;;
    --config) config+=(--config "$2"); shift 2 ;;
    -h | --help) sed -n '2,/^[^#]/{/^#/p}' "$0"; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
out=test-output/build-profile/$label
mkdir -p "$out"
results=$out/results.jsonl

now() { if date +%s.%N 2>/dev/null | grep -qv N; then date +%s.%N; else python3 -c 'import time; print(time.time())'; fi; }
load() { uptime | sed 's/.*load averages*: *//; s/,/ /g' | awk '{print $1}'; }
has() { case ",$1," in *",$2,"*) return 0 ;; *) return 1 ;; esac; }

voice_env=()
if command -v clang++-21 >/dev/null 2>&1; then voice_env=(CC=clang-21 CXX=clang++-21); fi
# macOS: the libc++ header workaround of scripts/build-macos-app.sh.
if [ "$(uname -s)" = Darwin ] && ! printf '#include <string>\n' | c++ -x c++ -std=c++17 -fsyntax-only - 2>/dev/null; then
  export CXXFLAGS="${CXXFLAGS:-} -nostdinc++ -isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"
fi

cmd_of() {
  case "$1" in
    build) echo "build --workspace" ;;
    test) echo "test --workspace --no-run" ;;
    clippy) echo "clippy --workspace --all-targets" ;;
    release) echo "build --release -p ocw-desktop --features ocw-desktop/voice" ;;
    wasm) echo "check -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world -p ocw-voice -p ocw-sky --target wasm32-unknown-unknown" ;;
    *) echo "unknown command $1" >&2; return 1 ;;
  esac
}

# One timed cargo run: COMMAND STEP.
run() {
  local c=$1 step=$2 args t0 t1 secs rc=0 log compiled l0
  read -r -a args <<<"$(cmd_of "$c")"
  log=$out/$c-$step.log
  l0=$(load)
  t0=$(now)
  env ${voice_env[@]+"${voice_env[@]}"} CARGO_TARGET_DIR="$tdir" \
    cargo "${args[@]}" ${config[@]+"${config[@]}"} --timings >"$log" 2>&1 || rc=$?
  t1=$(now)
  secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.1f", b - a }')
  compiled=$(grep -cE '^ *(Compiling|Checking) ' "$log" || true)
  # The newest report is this run's.
  local html
  html=$(ls -t "$tdir"/cargo-timings/cargo-timing-*.html 2>/dev/null | head -n 1 || true)
  [ -n "$html" ] && cp "$html" "$out/$c-$step.html"
  # Size of the target directory after a clean build (disk use per checkout).
  local mb=null
  [ "$step" = clean ] && mb=$(du -sk "$tdir" 2>/dev/null | awk '{ printf "%d", $1 / 1024 }')
  printf '{"label":"%s","command":"%s","step":"%s","seconds":%s,"compiled":%s,"exit":%s,"load":%s,"jobs":"%s","wrapper":"%s","config":"%s","target_mb":%s}\n' \
    "$label" "$c" "$step" "$secs" "$compiled" "$rc" "${l0:-0}" "${CARGO_BUILD_JOBS:-}" \
    "${RUSTC_WRAPPER:-}" "$(printf '%s' "${config[*]+${config[*]}}" | sed 's/"/\\"/g')" "${mb:-null}" >>"$results"
  printf 'build-profile: %-8s %-14s %7ss  %4s crates  exit %s  (load %s)\n' "$c" "$step" "$secs" "$compiled" "$rc" "$l0"
  [ "$rc" = 0 ] || tail -n 15 "$log" >&2
}

touch_files=(crates/ocw-world/src/lib.rs crates/ocw-render/src/lib.rs apps/desktop/src/main.rs)
IFS=, read -r -a cmd_list <<<"$commands"
first=1
for c in "${cmd_list[@]}"; do
  cmd_of "$c" >/dev/null
  if has "$steps" clean && { [ "$keep" = 0 ] || [ "$first" = 1 ]; }; then
    rm -rf "$tdir"
    run "$c" clean
  fi
  first=0
  if has "$steps" noop; then run "$c" noop; fi
  if has "$steps" touch; then
    for f in "${touch_files[@]}"; do
      touch "$f"
      n=${f#*/}; n=${n%%/*}
      run "$c" "touch-$n"
    done
  fi
  if has "$steps" edit; then
    for f in "${touch_files[@]}"; do
      n=${f#*/}; n=${n%%/*}
      cp -p "$f" "$f.build-profile"
      printf '\n#[allow(dead_code)]\nfn build_profile_probe() -> u32 {\n    %s\n}\n' "$RANDOM" >>"$f"
      run "$c" "edit-$n"
      cat "$f.build-profile" >"$f" && rm -f "$f.build-profile"
      env ${voice_env[@]+"${voice_env[@]}"} CARGO_TARGET_DIR="$tdir" \
        cargo $(cmd_of "$c") ${config[@]+"${config[@]}"} >/dev/null 2>&1 || true
    done
  fi
  [ "$keep" = 1 ] || rm -rf "$tdir"
done
python3 "$(dirname "$0")/build_profile.py" summary "$out" >"$out/summary.md" || true
echo "build-profile: results in $out"
