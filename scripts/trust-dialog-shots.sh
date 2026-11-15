#!/usr/bin/env bash
# Headless screenshots of the trust dialog (spec §4.7 *User keys*, v3.63) on
# X11 and Wayland, through .claude/skills/run-ocw/drive.sh:
#
#   scripts/trust-dialog-shots.sh [PACKAGE] [OUT_DIR]
#
# A throwaway user key "Ada Rider" (a temporary directory, deleted at the
# end, never committed) signs a copy of PACKAGE (default
# data/pilot/liechtenstein.ocwpkg) as user/ada/<name>; the app starts with
# `--world` on it and shows the dialog (it rides the built-in loop until the
# key is trusted). With $OCW_TRUSTED_KEYS in the temporary directory, the
# user's own trusted keys are not touched. A third shot shows Settings ->
# Account -> Trusted world keys -> the key, after `world trust add`.
# Screenshots: OUT_DIR/trust-dialog-x11.jpg, -wayland.jpg,
# trusted-key-page.jpg (default OUT_DIR test-output/trust).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
pkg=${1:-data/pilot/liechtenstein.ocwpkg}
out=${2:-test-output/trust}
mkdir -p "$out"
tmp=$(mktemp -d /tmp/ocw-trust-XXXXXX)
trap 'rm -rf "$tmp"' EXIT
cargo build -q -p world-builder
wb=target/debug/world-builder
"$wb" keys init-user --name "Ada Rider" --dir "$tmp/keys"
name=$(basename "$pkg" .ocwpkg)
cp "$pkg" "$tmp/ada-$name.ocwpkg"
"$wb" sign "$tmp/ada-$name.ocwpkg" --key "$tmp/keys/user.key" --id "user/ada/$name"
"$wb" keys fingerprint "$tmp/ada-$name.ocwpkg"
export OCW_TRUSTED_KEYS=$tmp/trusted-keys.json
drive=.claude/skills/run-ocw/drive.sh
"$drive" shot-wayland "$out/trust-dialog-wayland.jpg" 8 -- --world "$tmp/ada-$name.ocwpkg"
"$drive" shot "$out/trust-dialog-x11.jpg" 8 -- --world "$tmp/ada-$name.ocwpkg"
# The trusted key's page: Top -> Settings -> Account -> Trusted world keys
# (the last row) -> the key.
"$drive" shot-wayland "$out/trusted-key-page.jpg" 8 -- \
  --cmd "world trust add $tmp/keys/user-root.pub.json" \
  --cmd "menu open" --cmd "menu up" --cmd "menu up" --cmd "menu select" \
  --cmd "menu select" --cmd "menu up" --cmd "menu up" --cmd "menu select" --cmd "menu select"
echo "screenshots in $out"
