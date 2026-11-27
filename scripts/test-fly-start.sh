#!/usr/bin/env bash
# Tests deploy/fly/ocw-server/start.sh (spec 11.5, 18J) without Fly, root or
# a server: a copy of the script with /data in a temporary folder, chown
# left out and the user switch (`setpriv`) replaced by a stub that records
# its environment and arguments. Checks that OCW_ADSB_LOL_KEY and
# OCW_LIVE_AIR_OFFLINE reach the server when set (and only then), that
# everything else of the machine's environment is dropped, and that the
# key's value is in no command line and no log line.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$here/../deploy/fly/ocw-server/start.sh
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "test-fly-start: $*" >&2; exit 1; }

mkdir -p "$t/data/worlds" "$t/bin"
touch "$t/data/worlds/test.ocwpkg"
# The stub for `setpriv … --`: its environment and every argument it got.
cat >"$t/bin/fake-setpriv" <<EOF
#!/bin/sh
env >"$t/server.env"
printf '%s\n' "\$@" >"$t/server.args"
EOF
# What `env` itself is started with (the command line `ps` would show).
cat >"$t/bin/record-argv" <<EOF
#!/bin/sh
printf '%s\n' "\$@" >"$t/env.args"
exec "\$@"
EOF
chmod +x "$t/bin/fake-setpriv" "$t/bin/record-argv"
grep -q 'setpriv --reuid=ocw --regid=ocw --clear-groups --' "$src" || fail "start.sh: no setpriv line"
grep -q '^exec env "\$@"' "$src" || fail "start.sh: no exec env line"
grep -q -- '--reset-env' "$src" && fail "start.sh: setpriv --reset-env drops the server's variables"
sed -e "s#/data#$t/data#g" \
    -e 's#^chown .*#:#' \
    -e "s#setpriv --reuid=ocw --regid=ocw --clear-groups --#$t/bin/fake-setpriv#" \
    -e "s#^exec env \"\$@\"#exec $t/bin/record-argv env \"\$@\"#" \
    "$src" >"$t/start.sh"

run() {
  rm -f "$t/server.env" "$t/server.args" "$t/env.args"
  env -i PATH=/usr/bin:/bin OCW_PUBLIC_HOST=192.0.2.10 OCW_WORLD="$t/data/worlds/test.ocwpkg" \
    FLY_APP_NAME=ocw-test-app OTHER_SECRET=do-not-pass "$@" \
    sh "$t/start.sh" >"$t/out" 2>&1 || fail "start.sh failed: $(cat "$t/out")"
  [ -f "$t/server.env" ] || fail "the server was not started: $(cat "$t/out")"
}

key=adsb-test-key-1234567890
run OCW_ADSB_LOL_KEY="$key" OCW_LIVE_AIR_OFFLINE=1
grep -qx "OCW_ADSB_LOL_KEY=$key" "$t/server.env" || fail "the key does not reach the server"
grep -qx "OCW_LIVE_AIR_OFFLINE=1" "$t/server.env" || fail "OCW_LIVE_AIR_OFFLINE does not reach the server"
grep -qx "HOME=$t/data/home" "$t/server.env" || fail "HOME: $(grep HOME "$t/server.env")"
grep -q '^OTHER_SECRET=' "$t/server.env" && fail "the machine's other variables reach the server"
grep -q '^FLY_APP_NAME=' "$t/server.env" && fail "FLY_APP_NAME reaches the server"
grep -q "$key" "$t/server.args" "$t/env.args" "$t/out" && fail "the key's value is on a command line or in the log"
grep -qx -- "--world" "$t/server.args" || fail "server arguments: $(cat "$t/server.args")"
grep -q "passing OCW_ADSB_LOL_KEY" "$t/out" || fail "no log line for the key"

# Unset: not passed (not even empty).
run
grep -q '^OCW_ADSB_LOL_KEY' "$t/server.env" && fail "an unset key is passed"
grep -q '^OCW_LIVE_AIR_OFFLINE' "$t/server.env" && fail "an unset OCW_LIVE_AIR_OFFLINE is passed"
grep -q "passing" "$t/out" && fail "a log line for unset variables"
echo "test-fly-start: ok"
