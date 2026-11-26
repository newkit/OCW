#!/usr/bin/env bash
# The accounts backend's tests against a throw-away PostgreSQL (spec 14
# Phase 4, 4O W5a; `make test-accounts`, run on marge or leela):
#
#   1. a pinned PostgreSQL build (the portable Linux x86_64 binaries of
#      io.zonky.test.postgres, checked by SHA-256) is unpacked once into
#      ~/.cache/ocw/postgres-VERSION – nothing is installed system-wide, no
#      service, no root;
#   2. `initdb` makes a cluster in a temporary directory, `pg_ctl` starts it
#      on a free loopback port (trust authentication, only 127.0.0.1 and a
#      socket in that directory);
#   3. `cargo test -p ocw-accounts -p ocw-account-api` runs with
#      OCW_TEST_DATABASE_URL pointing at it (OCW_REQUIRE_POSTGRES=1: the
#      PostgreSQL tests fail instead of skipping without it);
#   4. the cluster is stopped and its directory removed, also on failure.
#
# Extra arguments go to cargo test (e.g. a test name filter).
set -euo pipefail
cd "$(dirname "$0")/.."

PG_VERSION=17.11.0
PG_JAR_SHA256=0dd7b72b6f335b8ecfb355fa24c5781e8a93edd09880bb77eb52ebbf29b3e96d
PG_URL="https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-amd64/$PG_VERSION/embedded-postgres-binaries-linux-amd64-$PG_VERSION.jar"

if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "test-accounts: the pinned PostgreSQL build is for Linux x86_64 (run it on marge or leela)" >&2
  exit 2
fi

cache=${OCW_PG_CACHE:-$HOME/.cache/ocw/postgres-$PG_VERSION}
if [ ! -x "$cache/bin/postgres" ]; then
  echo "test-accounts: fetching PostgreSQL $PG_VERSION into $cache"
  part=$(mktemp -d "${TMPDIR:-/tmp}/ocw-pg-fetch.XXXXXX")
  curl -fsSL -o "$part/pg.jar" "$PG_URL"
  echo "$PG_JAR_SHA256  $part/pg.jar" | sha256sum -c --quiet -
  mkdir -p "$part/pg"
  unzip -p "$part/pg.jar" postgres-linux-x86_64.txz | tar -xJ -C "$part/pg"
  mkdir -p "$(dirname "$cache")"
  rm -rf "$cache"
  mv "$part/pg" "$cache"
  rm -rf "$part"
fi
"$cache/bin/postgres" --version

run=$(mktemp -d "${TMPDIR:-/tmp}/ocw-pg.XXXXXX")
stop() {
  "$cache/bin/pg_ctl" -D "$run/data" -m immediate stop >/dev/null 2>&1 || true
  rm -rf "$run"
}
trap stop EXIT

"$cache/bin/initdb" -D "$run/data" -U ocw --auth=trust -E UTF8 --locale=C >"$run/initdb.log"
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
"$cache/bin/pg_ctl" -D "$run/data" -l "$run/postgres.log" -w \
  -o "-c listen_addresses=127.0.0.1 -p $port -k $run -c fsync=off -c max_connections=200" start >/dev/null \
  || { cat "$run/postgres.log" >&2; exit 1; }
echo "test-accounts: PostgreSQL $PG_VERSION on 127.0.0.1:$port (throw-away cluster)"

export OCW_TEST_DATABASE_URL="postgres://ocw@127.0.0.1:$port/postgres"
export OCW_REQUIRE_POSTGRES=1
${CARGO:-cargo} test -p ocw-accounts -p ocw-account-api "$@"
