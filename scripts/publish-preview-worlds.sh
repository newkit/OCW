#!/usr/bin/env bash
# Signs and publishes the worlds of the browser app's public preview (spec
# §14 Phase 10 B8a, `make web-preview-worlds`). Runs on the build machine
# that holds the preview's signing keys (marge):
#
#   scripts/publish-preview-worlds.sh [--upload] [PKG...]
#                                  (default: data/pilot/liechtenstein.ocwpkg)
#
# 1. Each package is signed (a staged copy; the package itself is not
#    changed, its manifest keeps id, version and attribution) with the web
#    preview's signing key and published in the `world-builder publish`
#    layout into target/web-preview-worlds/worlds/<id>/<version>/ and
#    <id>/latest; `world-builder verify` checks it against the preview root.
# 2. The release directory target/web-preview-worlds/release/ gets every
#    file under a flat asset name (liechtenstein-3.ocwpkg,
#    liechtenstein-3.manifest.json, liechtenstein.latest), SHA256SUMS and
#    preview-worlds.json: per world its id, name, version, routes, the
#    layout (site path → asset) and the hashes. The site assembly
#    (scripts/web-preview-site.py, the Pages workflow) builds worlds/ from
#    it.
# 3. With --upload, the assets go to the GitHub pre-release
#    `web-preview-worlds` (created if missing; existing assets replaced),
#    where the Pages workflow downloads them: no secret is ever needed in CI.
#
# The keys (OCW_PREVIEW_KEYS, default ~/.config/ocw/preview-keys/: root.key,
# root.pub.json, web-preview-2026.key) never leave that directory: never
# commit, print or upload them. Their public half is
# crates/ocw-worldstore/keys/preview-roots.json, compiled into the preview
# build. A new signing key (when this one expires, see `keys new-signing
# --days`):
#   world-builder keys new-signing --name web-preview-YYYY \
#     --root ~/.config/ocw/preview-keys/root.key --days 730 \
#     --out ~/.config/ocw/preview-keys/web-preview-YYYY.key
# and OCW_PREVIEW_SIGNING_KEY=…/web-preview-YYYY.key.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

upload=0
if [ "${1:-}" = --upload ]; then upload=1; shift; fi
keys=${OCW_PREVIEW_KEYS:-$HOME/.config/ocw/preview-keys}
key=${OCW_PREVIEW_SIGNING_KEY:-$keys/web-preview-2026.key}
tag=${OCW_PREVIEW_RELEASE:-web-preview-worlds}
# The repository (build checkouts have no .git for gh to ask).
repo=${OCW_PREVIEW_REPO:-newkit/OCW}
out=target/web-preview-worlds
for f in "$key" "$keys/root.pub.json"; do
  [ -f "$f" ] || { echo "$f missing: the preview's keys live on the build machine that signs the worlds (marge)" >&2; exit 1; }
done
# The key the build trusts must be the one that signs.
python3 -I - "$keys/root.pub.json" crates/ocw-worldstore/keys/preview-roots.json <<'EOF'
import json, sys
root = json.load(open(sys.argv[1]))
built_in = json.load(open(sys.argv[2]))
if root["public_key"] not in [k["public_key"] for k in built_in]:
    sys.exit(f"{sys.argv[1]} is not the preview root in {sys.argv[2]}")
EOF

wb=${WB:-}
if [ -z "$wb" ]; then
  cargo build -q --release -p world-builder
  wb=target/release/world-builder
fi

[ $# -gt 0 ] || set -- "$(ls data/pilot/liechtenstein.ocwpkg ../OCW/data/pilot/liechtenstein.ocwpkg 2>/dev/null | head -1)"
rm -rf "$out"
mkdir -p "$out/worlds" "$out/release" "$out/stage"
routes_json=$out/routes.jsonl
: > "$routes_json"
for pkg in "$@"; do
  [ -f "$pkg" ] || { echo "$pkg: no such package" >&2; exit 1; }
  stage=$out/stage
  rm -rf "${stage:?}"/*
  name=$(basename "$pkg")
  ln "$pkg" "$stage/$name" 2>/dev/null || cp "$pkg" "$stage/$name"
  [ -f "$pkg.manifest.json" ] && cp "$pkg.manifest.json" "$stage/$name.manifest.json"
  "$wb" sign "$stage/$name" --key "$key"
  "$wb" publish "$stage/$name" --out "$out/worlds"
  # The routes for the start page (`world-builder routes` lines: "NAME: 13.29 km, …").
  "$wb" routes --world "$stage/$name" | python3 -I -c '
import json, re, sys
pkg = sys.argv[1]
for line in sys.stdin:
    m = re.match(r"^(.+?): ([\d.]+) km, .*\bok\b", line.strip())
    if m:
        print(json.dumps({"package": pkg, "name": m.group(1), "km": float(m.group(2))}))
' "$name" >> "$routes_json"
done
# Every published manifest checked against the preview root.
for m in "$out"/worlds/*/*/manifest.json; do
  dir=$(dirname "$m")
  for p in "$dir"/*.ocwpkg; do
    "$wb" verify "$p" --trust-root "$keys/root.pub.json" --no-dev-root >/dev/null ||
      { echo "$p: does not verify against the preview root" >&2; exit 1; }
  done
done

python3 -I - "$out" "$tag" <<'EOF'
import hashlib, json, os, shutil, sys
out, tag = sys.argv[1], sys.argv[2]
worlds = os.path.join(out, "worlds")
release = os.path.join(out, "release")
routes = [json.loads(l) for l in open(os.path.join(out, "routes.jsonl")) if l.strip()]

def slug(name):
    s = "".join(c if c.isalnum() else "-" for c in name.lower())
    return "-".join(p for p in s.split("-") if p)

def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()

index = {"release": tag, "worlds": []}
sums = []
for wid in sorted(os.listdir(worlds)):
    wdir = os.path.join(worlds, wid)
    latest = open(os.path.join(wdir, "latest")).read().strip()
    vdir = os.path.join(wdir, latest)
    manifest = json.load(open(os.path.join(vdir, "manifest.json")))
    signed = manifest["signed"]
    flat = wid.replace("/", "-")
    files = {f"worlds/{wid}/latest": f"{flat}.latest",
             f"worlds/{wid}/{latest}/manifest.json": f"{flat}-{latest}.manifest.json"}
    for f in signed["files"]:
        stem, ext = os.path.splitext(f["path"])
        files[f"worlds/{wid}/{latest}/{f['path']}"] = f"{flat}-{latest}{ext}"
    hashes = {}
    for site_path, asset in files.items():
        src = os.path.join(out, site_path)
        dst = os.path.join(release, asset)
        try:
            os.link(src, dst)
        except OSError:
            shutil.copyfile(src, dst)
        hashes[asset] = sha256(dst)
        sums.append(f"{hashes[asset]}  {asset}")
    key = manifest.get("signature", {}).get("key", {})
    index["worlds"].append({
        "id": wid, "name": signed["name"], "version": int(latest),
        "bytes": sum(os.path.getsize(os.path.join(release, a)) for a in files.values()),
        "routes": [{"name": r["name"], "slug": slug(r["name"]), "km": r["km"]}
                   for r in routes if r["package"] == signed["files"][0]["path"]],
        "files": files, "sha256": hashes,
    })
with open(os.path.join(release, "preview-worlds.json"), "w") as f:
    json.dump(index, f, indent=2, ensure_ascii=False)
    f.write("\n")
with open(os.path.join(release, "SHA256SUMS"), "w") as f:
    f.write("\n".join(sums) + "\n")
for w in index["worlds"]:
    print(f"{w['id']}@{w['version']}: {w['bytes'] / 1e6:.1f} MB, routes: {', '.join(r['name'] for r in w['routes']) or 'none'}")
EOF
rm -rf "$out/stage"
ls -l "$out/release"

if [ $upload = 1 ]; then
  if ! gh release view "$tag" -R "$repo" >/dev/null 2>&1; then
    gh release create "$tag" -R "$repo" --prerelease --title "Web preview worlds" --notes \
"World packages of the browser app's public preview (spec §14 Phase 10 B8a), signed by the web preview's root (key id $(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["key_id"])' "$keys/root.pub.json")). The Pages workflow (.github/workflows/web-preview.yml) copies them into the site; preview-worlds.json maps each asset to its path there.

Map data © OpenStreetMap contributors, ODbL 1.0: the road graph in each package (section GRPH) is a Derivative Database offered here under the ODbL. Elevation, imagery and forest sources: see each manifest's attribution and the preview's credits page."
  fi
  gh release upload "$tag" -R "$repo" --clobber "$out"/release/*
  echo "uploaded to the release $tag"
fi
