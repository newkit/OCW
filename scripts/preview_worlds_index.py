#!/usr/bin/env python3
"""The release index of the preview's worlds (spec §14 Phase 10 B8a), called
by scripts/publish-preview-worlds.sh after `world-builder publish`:

    scripts/preview_worlds_index.py OUT TAG

OUT/worlds/ holds the publish layout (<id>/latest, <id>/<version>/…; ids
may have several levels, e.g. europe/spain/tenerife), OUT/routes.jsonl the
routes. Writes OUT/release/: every file under a flat asset name,
preview-worlds.json and SHA256SUMS.
"""
import hashlib
import json
import os
import shutil
import sys


def slug(name):
    s = "".join(c if c.isalnum() else "-" for c in name.lower())
    return "-".join(p for p in s.split("-") if p)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def world_ids(worlds):
    """Every world id under the publish layout: the directories holding a
    `latest` file, as '/'-separated paths (several levels allowed – a
    listdir of the top level saw only "europe", bug 2026-11-15)."""
    ids = []
    for d, _, files in os.walk(worlds):
        if "latest" in files:
            ids.append(os.path.relpath(d, worlds).replace(os.sep, "/"))
    return sorted(ids)


def build(out, tag):
    worlds = os.path.join(out, "worlds")
    release = os.path.join(out, "release")
    os.makedirs(release, exist_ok=True)
    routes = [json.loads(l) for l in open(os.path.join(out, "routes.jsonl")) if l.strip()]
    index = {"release": tag, "worlds": []}
    sums = []
    for wid in world_ids(worlds):
        wdir = os.path.join(worlds, *wid.split("/"))
        latest = open(os.path.join(wdir, "latest")).read().strip()
        vdir = os.path.join(wdir, latest)
        manifest = json.load(open(os.path.join(vdir, "manifest.json")))
        signed = manifest["signed"]
        flat = wid.replace("/", "-")
        files = {f"worlds/{wid}/latest": f"{flat}.latest",
                 f"worlds/{wid}/{latest}/manifest.json": f"{flat}-{latest}.manifest.json"}
        for f in signed["files"]:
            ext = os.path.splitext(f["path"])[1]
            files[f"worlds/{wid}/{latest}/{f['path']}"] = f"{flat}-{latest}{ext}"
        hashes = {}
        for site_path, asset in files.items():
            src = os.path.join(out, *site_path.split("/"))
            dst = os.path.join(release, asset)
            if os.path.exists(dst):
                os.remove(dst)
            try:
                os.link(src, dst)
            except OSError:
                shutil.copyfile(src, dst)
            hashes[asset] = sha256(dst)
            sums.append(f"{hashes[asset]}  {asset}")
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
    return index


def main():
    out, tag = sys.argv[1], sys.argv[2]
    for w in build(out, tag)["worlds"]:
        names = ", ".join(r["name"] for r in w["routes"]) or "none"
        print(f"{w['id']}@{w['version']}: {w['bytes'] / 1e6:.1f} MB, routes: {names}")


if __name__ == "__main__":
    main()
