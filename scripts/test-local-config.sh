#!/usr/bin/env bash
# Headless run of the local config file and the start choice (spec 3.2.4):
# a throwaway config folder with the old files of an earlier version
# (settings.toml, riders.toml, devices.toml, joins.txt, strava.toml,
# strava-token.toml; made-up data only) is migrated into ocw.toml and
# secrets.toml at the first start; then the pictures:
#
#   start-<ws>.jpg    the start choice, the last choice (Ride locally) preselected
#   server-<ws>.jpg   Connect to a server: the address field and the recent servers
#   riders-<ws>.jpg   Settings -> Riders with the local riders from ocw.toml
#   listing-<ws>.txt  the migrated folder: files, modes, ocw.toml (secrets.toml
#                     only by its keys)
#
#   scripts/test-local-config.sh OUT_DIR [x11|wayland]
set -euo pipefail
out=${1:?OUT_DIR}
ws=${2:-x11}
case $ws in
  x11) mode=shot ;;
  wayland) mode=shot-wayland ;;
  *) echo "x11 or wayland, not $ws" >&2; exit 2 ;;
esac
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
export OCW_ACTIVITIES_DIR=$work/Activities/OCW
export OCW_TEST_OUTPUT=off OCW_SIZE=${OCW_SIZE:-1920x1080}
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
c=$OCW_CONFIG_DIR
# The old files (made-up riders, devices, servers and credentials).
cat > "$c/settings.toml" <<'EOF'
[audio]
master = 60

[display]
units = "metric"
EOF
cat > "$c/riders.toml" <<'EOF'
active = "Ada Rider"

[[rider]]
name = "Ada Rider"
weight_kg = 61.0
height_cm = 168.0
ftp_w = 230

[[rider]]
name = "Bea Climber"
weight_kg = 55.0
height_cm = 162.0
ftp_w = 250
EOF
cat > "$c/devices.toml" <<'EOF'
[[device]]
id = "hci0/dev_00_00_5E_00_53_01"
name = "Made-up Trainer 1234"
kind = "trainer"
EOF
printf 'ocw://127.0.0.1:47800\troute hills\nocw://rides.example.org:47800?cert=00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff\tBlack Forest north\n' > "$c/joins.txt"
printf 'client_id = "12345"\nclient_secret = "made-up-secret"\nauto_upload = false\n' > "$c/strava.toml"
printf 'access_token = "made-up-a"\nrefresh_token = "made-up-r"\nexpires_at = 1800000000\nathlete = "Ada Rider"\n' > "$c/strava-token.toml"
common=(--bots 3 --virtual --power 220 --strava-dry-run)
# 1. The start choice (screenshots decide by themselves: --start-choice ask).
.claude/skills/run-ocw/drive.sh "$mode" "$out/start-$ws.jpg" 6 -- "${common[@]}" --start-choice ask
cp "$out/start-$ws.log" "$out/migration-$ws.log" 2>/dev/null || true
# 2. Connect to a server: the field and the recent servers (from joins.txt).
.claude/skills/run-ocw/drive.sh "$mode" "$out/server-$ws.jpg" 6 -- "${common[@]}" --start-choice server
# 3. Settings -> Riders: the local riders of ocw.toml.
.claude/skills/run-ocw/drive.sh "$mode" "$out/riders-$ws.jpg" 6 -- "${common[@]}" \
  --cmd "menu workshop-page" --cmd "menu tab-next"
{
  echo "# The test config folder after the first start ($ws; made-up data)"
  echo "## files (mode, name)"
  (cd "$c" && stat -c '%A %n' * | sort -k2)
  echo
  echo "## ocw.toml"
  cat "$c/ocw.toml"
  echo
  echo "## secrets.toml (keys only)"
  grep -E '^\[|^[a-z_]+ =' "$c/secrets.toml" | sed -E 's/=.*/= …/'
  echo
  echo "## the migration in the log"
  grep -h 'config: moved' "$out/migration-$ws.log" | sed "s|$work/||g; s|^.*config: moved|config: moved|" | sort -u || true
} > "$out/listing-$ws.txt"
grep -h -E 'start choice|config: moved|riding locally' "$out/start-$ws.log" | sed "s|$work/||g" | tail -12 || true
cat "$out/listing-$ws.txt"
