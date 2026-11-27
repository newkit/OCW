#!/usr/bin/env bash
# Before / after of a world's load in WebKit (spec 14 Phase 10, 10G, v6.99):
# the page build in target/web as it is ("old"), then `make web` ("new"),
# each loading Andorra, Tenerife and Liechtenstein as the reporting iPhone
# (scripts/web-load-memory.sh). DATA: the folder with the packages
# (default ~/src/OCW/data). Into test-output/web-load/.
set -uo pipefail
cd "$(dirname "$0")/.."
data=${DATA:-$HOME/src/OCW/data}
a=$data/andorra/andorra.ocwpkg:europe/andorra
t=$data/tenerife/tenerife.ocwpkg:europe/spain/tenerife
l=$data/pilot/liechtenstein.ocwpkg:liechtenstein
out=test-output/web-load
if [ -z "${SKIP_OLD:-}" ]; then
  scripts/publish-web-worlds.sh "${a%%:*}" "${t%%:*}" "${l%%:*}" > "$out-publish.txt" 2>&1 || { tail -5 "$out-publish.txt"; exit 1; }
  WEB_LOAD_NO_PUBLISH=1 WEB_LOAD_LABEL=old-all WEB_LOAD_QUERY=base=all scripts/web-load-memory.sh "$out/old" "$a" "$t" "$l"
fi
make web > "$out-make-web.txt" 2>&1 || { tail -30 "$out-make-web.txt"; exit 1; }
WEB_LOAD_NO_PUBLISH=1 WEB_LOAD_LABEL=new scripts/web-load-memory.sh "$out/new" "$a" "$t" "$l"
WEB_LOAD_NO_PUBLISH=1 WEB_LOAD_LABEL=new-minimal WEB_LOAD_QUERY=memory=minimal scripts/web-load-memory.sh "$out/new" "$a" "$t"
WEB_LOAD_NO_PUBLISH=1 WEB_LOAD_LABEL=new-all WEB_LOAD_QUERY=base=all scripts/web-load-memory.sh "$out/new" "$a" "$t"
