#!/usr/bin/env python3
"""Assembles the browser app's public preview site (spec §14 Phase 10 B8a).

    scripts/web-preview-site.py [--build target/web-preview]
        [--worlds target/web-preview-worlds/release] [--out target/web-preview-site]
        [--crates crates.json] [--build-id TEXT] [--max-mb 1000]
        [--xr-build target/web-preview-xr]

Inputs:
  --build    the preview build (`scripts/build-web.sh --preview`)
  --worlds   the preview's world assets: the GitHub pre-release
             `web-preview-worlds` downloaded (`gh release download`), or
             target/web-preview-worlds/release/ of
             scripts/publish-preview-worlds.sh. preview-worlds.json maps each
             asset to its path in the site; every file is checked against
             its SHA-256 there (and the app checks the signature chain and
             every chunk again).
  --crates   optional `cargo metadata --format-version 1 --filter-platform
             wasm32-unknown-unknown` output: the Rust crates compiled into
             the module, listed with their licences on the credits page.
  --xr-build optional: the WebXR preview build (`make web-preview-xr`:
             `WEB_GPU=webxr scripts/build-web.sh --preview`), published
             under xr/ (spec 14 Phase 14, 14I Q6 (a): VR test builds at
             /xr/ next to the normal preview, which stays as it is). Its
             page shares the site's worlds (../worlds/), its rides open
             the WebXR module (xr=1), its credits link goes to the site's.
             Skipped with a note when the directory has no WebXR module.

Writes OUT/: the build (index.html, the two modules, ocw-voice.js, ocw-gpu.js, the
logo), worlds/ in the `world-builder publish` layout, config.json (the
page configuration: worlds/, the public server's join info and voice
(PUBLIC_SERVER, spec 11.5 *Join info*; --no-server: none), no sign-in, the
start page's rides), credits.html (attribution, licences, the privacy note),
licenses/ (the font's OFL), .nojekyll, and site.txt (every file with its
size). Fails when the site is over --max-mb (GitHub Pages: 1 GB).
"""

import argparse
import gzip
import hashlib
import html
import json
import os
import shutil
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = "https://github.com/newkit/OCW"

# The built-in test routes (ocw_world::Route::test_hills / test_loop).
TEST_RIDES = [
    {"title": "Hills (test route)",
     "text": "A 3.9 km loop with climbs and descents from −10 % to +12 %, a gravel stretch, 5 bots.",
     "query": "route=hills"},
    {"title": "Loop (test route)",
     "text": "A rolling 4 km loop with one climb of about 30 m, 5 bots.",
     "query": "route=loop"},
]

# The public server (spec 11.5, README "Hosting the server on Fly.io"):
# the page fetches its join info at runtime (the WebTransport URL and the
# certificate hashes change at renewals and restarts) and joins with
# ?together=1; voice goes to its voice server over wss://. `world` is the
# package it rides: the start page's ride needs it among the preview's
# worlds (the same package file, or the road graphs differ).
PUBLIC_SERVER = {
    "join": "https://ocw-server.fly.dev/join.json",
    "voice": "wss://ocw-server.fly.dev",
    "world": "liechtenstein",
}

BUILD_FILES = ("index.html", "ocw_web.js", "ocw_web_bg.wasm", "ocw_web_webgl2.js",
               "ocw_web_webgl2_bg.wasm", "ocw-voice.js", "ocw-gpu.js", "ocw-log.js", "ocw-xr.js", "ocw-net-worker.js", "ocw.svg", "sizes.txt",
               # Add to Home Screen (spec 3.2.3): the manifest and its icons.
               "manifest.webmanifest", "ocw-256.png", "ocw-512.png",
               # Voice (B5) with the public server: LiveKit's JS SDK and its licence.
               "livekit-client.umd.js", "livekit-client.LICENSE.txt")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def place(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    try:
        os.link(src, dst)
    except OSError:
        shutil.copyfile(src, dst)


def worlds(index_dir, out):
    """Places the world assets; returns the worlds of preview-worlds.json
    with their manifests."""
    path = os.path.join(index_dir, "preview-worlds.json")
    if not os.path.isfile(path):
        sys.exit(f"{path} missing: download the release (gh release download web-preview-worlds)"
                 " or run scripts/publish-preview-worlds.sh")
    index = json.load(open(path))
    found = []
    for w in index["worlds"]:
        for site_path, asset in w["files"].items():
            src = os.path.join(index_dir, asset)
            if not site_path.startswith("worlds/") or ".." in site_path.split("/"):
                sys.exit(f"{asset}: bad site path {site_path}")
            got = sha256(src)
            if got != w["sha256"][asset]:
                sys.exit(f"{asset}: SHA-256 {got} is not the one in preview-worlds.json")
            place(src, os.path.join(out, site_path))
        manifest = next(p for p in w["files"] if p.endswith("/manifest.json"))
        w["manifest"] = json.load(open(os.path.join(out, manifest)))["signed"]
        w["package_path"] = next(p for p in w["files"] if p.endswith(".ocwpkg"))
        w["release"] = index.get("release", "web-preview-worlds")
        found.append(w)
    return found


# The line speed the start page's load times are given for (a typical
# home connection; the time scales linearly).
LINE_MBIT = 50


def load_time(nbytes, mbit=LINE_MBIT):
    """How long a download takes at `mbit` Mbit/s, rounded for people."""
    s = nbytes * 8 / (mbit * 1e6)
    if s < 1:
        return "under 1 s"
    if s < 90:
        return f"about {s:.0f} s"
    return f"about {s / 60:.0f} min"


def built_with_simd(build):
    """Q10 (a): the live page serves the SIMD builds – scripts/build-web.sh
    checked cargo's modules (scripts/wasm_features.py) and says so in
    sizes.txt."""
    sizes = os.path.join(build, "sizes.txt")
    if not os.path.isfile(sizes):
        return False
    with open(sizes) as f:
        return "wasm features: +simd128" in f.read()


MODULES = (("ocw_web_bg.wasm", "ocw_web.js"), ("ocw_web_webgl2_bg.wasm", "ocw_web_webgl2.js"))
# The WebXR build's files (14I W1; ocw-web's features webgl2,webxr) and the
# page files it needs beside them.
XR_MODULES = (("ocw_web_webxr_bg.wasm", "ocw_web_webxr.js"),)
XR_FILES = ("index.html", "ocw_web_webxr.js", "ocw_web_webxr_bg.wasm", "ocw-voice.js", "ocw-gpu.js", "ocw-log.js",
            "ocw-xr.js", "ocw-net-worker.js", "ocw.svg", "sizes.txt", "manifest.webmanifest", "ocw-256.png", "ocw-512.png")


def app_bytes(build, modules=MODULES):
    """The app's first download: both wasm modules are built, the page loads
    one, served gzip-compressed by GitHub Pages (B8a) – so the larger
    gzipped module plus its JS glue."""
    best = 0
    for wasm, js in modules:
        w = os.path.join(build, wasm)
        if not os.path.isfile(w):
            continue
        with open(w, "rb") as f:
            n = len(gzip.compress(f.read(), 6))
        j = os.path.join(build, js)
        n += os.path.getsize(j) if os.path.isfile(j) else 0
        best = max(best, n)
    return best


def size_line(app, world=0):
    """The ride card's download line: what is fetched on the first visit and
    how long it takes at LINE_MBIT; everything is cached for the next one."""
    if not world:
        return (f"Download: {app / 1e6:.0f} MB (the app), {load_time(app)} at {LINE_MBIT} Mbit/s;"
                " no world data.")
    return (f"Download: {app / 1e6:.0f} MB app + up to {world / 1e6:.0f} MB world, streamed while you ride"
            f" (all of it: {load_time(app + world)} at {LINE_MBIT} Mbit/s); cached for the next visit.")


def xr_site(xr_build, found, out, build_id=""):
    """The WebXR preview under OUT/xr/ (14I Q6 (a)): the XR build's page and
    module, a config.json with the site's worlds (../worlds/) and the rides
    with xr=1, a credits.html that leads to the site's. Returns the number
    of files placed (0: no WebXR module in xr_build, skipped)."""
    if not os.path.isfile(os.path.join(xr_build, "ocw_web_webxr_bg.wasm")):
        print(f"note: {xr_build}/ocw_web_webxr_bg.wasm missing: no /xr/ (make web-preview-xr)")
        return 0
    # Q10 (a): /xr/ serves a SIMD build too.
    if not built_with_simd(xr_build):
        sys.exit(f"{xr_build}: not built with WebAssembly SIMD (sizes.txt): rebuild with make web-preview-xr")
    xr = os.path.join(out, "xr")
    os.makedirs(xr, exist_ok=True)
    n = 0
    for f in XR_FILES:
        src = os.path.join(xr_build, f)
        if os.path.isfile(src):
            place(src, os.path.join(xr, f))
            n += 1
    rides = [dict(r, query=r["query"] + "&xr=1") for r in start_rides(found, app_bytes(xr_build, XR_MODULES))]
    config = {
        "channel": "preview",
        "worlds": "../worlds/",
        "server": "", "voice": "", "signin": "",
        "build": (build_id + " · WebXR").strip(" ·"),
        "start": rides,
    }
    with open(os.path.join(xr, "config.json"), "w") as f:
        json.dump(config, f, indent=2, ensure_ascii=False)
        f.write("\n")
    with open(os.path.join(xr, "credits.html"), "w") as f:
        f.write('<!doctype html><meta charset="utf-8"><title>Credits</title>'
                '<meta http-equiv="refresh" content="0; url=../credits.html">'
                '<a href="../credits.html">Credits</a>\n')
    return n + 2


def public_server(found, enabled=True):
    """The public server's world among the preview's (None: no riding
    together on this site)."""
    if not enabled:
        return None
    w = next((w for w in found if w["id"] == PUBLIC_SERVER["world"]), None)
    if w is None:
        print(f"warning: world {PUBLIC_SERVER['world']} (the public server's) is not among the"
              " preview's worlds: no riding together", file=sys.stderr)
    return w


def together_ride(w, app=0):
    """The start page's ride on the public server (index.html fetches its
    join info for ?together=1)."""
    return {
        "title": f"Ride together: {w['name']} (public server)",
        "text": "Free ride with everyone on the public server, with voice chat (your voice starts off: M).",
        "size": size_line(app, w["bytes"]),
        "query": "together=1",
    }


def page_config(found, server, build_id, start, worlds="worlds/"):
    """config.json: the worlds, the public server's join info and voice
    when it rides one of them, no sign-in."""
    return {
        "channel": "preview",
        "worlds": worlds,
        "server": PUBLIC_SERVER["join"] if server else "",
        "voice": PUBLIC_SERVER["voice"] if server else "",
        "signin": "",
        "build": build_id,
        "start": start,
    }


def start_rides(found, app=0, server=None):
    rides = [dict(r, size=size_line(app)) for r in TEST_RIDES]
    if server:
        rides.insert(0, together_ride(server, app))
    for w in found:
        for r in w["routes"]:
            rides.append({
                "title": f"{w['name']}: {r['name']}",
                "text": f"{r['km']:.1f} km on real roads.",
                "size": size_line(app, w["bytes"]),
                "query": f"world={w['id']}&route={r['slug']}",
            })
        rides.append({
            "title": f"{w['name']}: free ride",
            "text": "Ride wherever the roads go; junctions take their default exit.",
            "size": size_line(app, w["bytes"]),
            "query": f"world={w['id']}&free=1",
        })
    return rides


def crates_table(path):
    """The Rust crates in ocw-web's build for wasm32 (normal dependencies),
    with their licences."""
    meta = json.load(open(path))
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    start = next(p["id"] for p in meta["packages"] if p["name"] == "ocw-web")
    seen, todo = set(), [start]
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i].get("deps", []):
            if any(k.get("kind") is None for k in d.get("dep_kinds", [])):
                todo.append(d["pkg"])
    rows = []
    for i in seen:
        p = pkgs[i]
        if p.get("source") is None:  # our own crates
            continue
        rows.append((p["name"], p["version"], p.get("license") or "see the crate",
                     p.get("repository") or f"https://crates.io/crates/{p['name']}"))
    return sorted(rows)


def credits(found, crates, out):
    e = html.escape
    sec = []
    for w in found:
        m = w["manifest"]
        items = "".join(
            f"<li><b>{e(a['source'])}</b> – {e(a['notice'])}<br><span class=dim>Licence: {e(a['licence'])}"
            + (f" · <a href=\"{e(a['url'])}\">{e(a['url'])}</a>" if a.get("url") else "")
            + "</span></li>"
            for a in m.get("attribution", []))
        pkg_url = w["package_path"]
        asset = w["files"][pkg_url]
        rel = f"{REPO}/releases/download/{w['release']}/{asset}"
        sec.append(f"""
<h3>{e(w['name'])} (world package {e(w['id'])}, version {w['version']})</h3>
<ul>{items}</ul>
<p><b>The road graph (ODbL).</b> The package's road graph – section <code>GRPH</code> of
<code>{e(os.path.basename(pkg_url))}</code>: roads, junctions, names, widths and elevation profiles derived from
OpenStreetMap – is a Derivative Database under the
<a href="https://opendatacommons.org/licenses/odbl/1-0/">Open Database License 1.0</a> and is offered
under that licence. Download it with the package: <a href="{e(pkg_url)}">from this site</a> or
<a href="{e(rel)}">from the GitHub release</a> (the manifest next to it has its SHA-256). The format is
described in <a href="{REPO}/blob/main/SPECIFICATION.md">SPECIFICATION.md</a> §4.7; the code that writes
and reads it (<code>world-builder</code>, <code>ocw-world</code>) is in the repository.</p>""")
    manifest = tomllib.load(open(os.path.join(ROOT, "assets/materials.toml"), "rb"))
    mats = "".join(
        f"<tr><td>{e(m.get('name', m['id']))}</td><td>{e(m.get('title', ''))}</td>"
        f"<td>{e(m.get('author', ''))}</td><td>{e(m.get('license', ''))}</td>"
        f"<td><a href=\"{e(m['source'])}\">source</a></td></tr>"
        for m in manifest.get("material", []) if m.get("source"))
    models = tomllib.load(open(os.path.join(ROOT, "artwork/3dmodels/manifest.toml"), "rb"))
    model_licences = sorted({a["license"] for a in models.get("asset", [])})
    os.makedirs(os.path.join(out, "licenses"), exist_ok=True)
    shutil.copyfile(os.path.join(ROOT, "crates/ocw-render/assets/fonts/OFL.txt"),
                    os.path.join(out, "licenses/FiraMono-OFL.txt"))
    crate_rows = ""
    if crates:
        crate_rows = (f"<p>{len(crates)} Rust crates are compiled into the module, each under its own licence:</p>"
                      "<details><summary>The crates and their licences</summary><table><tr><th>Crate</th>"
                      "<th>Version</th><th>Licence</th></tr>"
                      + "".join(f"<tr><td><a href=\"{e(r)}\">{e(n)}</a></td><td>{e(v)}</td><td>{e(l)}</td></tr>"
                                for n, v, l, r in crates)
                      + "</table></details>")
    page = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Credits and privacy – Open Cycling World</title>
<link rel="icon" href="ocw.svg" type="image/svg+xml">
<style>
  :root {{ color-scheme: dark; }}
  body {{ margin: 0; background: #10141a; color: #d8dee9; font: 16px/1.55 system-ui, sans-serif; }}
  main {{ max-width: 860px; margin: 0 auto; padding: 32px 16px 48px; }}
  h1 {{ font-size: 26px; margin: 0 0 8px; }} h2 {{ font-size: 20px; margin: 32px 0 8px; }}
  h3 {{ font-size: 17px; margin: 20px 0 6px; }}
  a {{ color: #88c0d0; }} code {{ font-size: 14px; }}
  .dim {{ color: #aab4c3; font-size: 14px; }}
  li {{ margin-bottom: 6px; }}
  table {{ border-collapse: collapse; font-size: 14px; width: 100%; display: block; overflow-x: auto; }}
  td, th {{ text-align: left; padding: 3px 10px 3px 0; border-bottom: 1px solid #2c3644; vertical-align: top; }}
  details {{ margin: 8px 0; }} summary {{ cursor: pointer; color: #88c0d0; }}
</style>
</head>
<body>
<main>
<p><a href="./">← Open Cycling World browser preview</a></p>
<h1>Credits, licences and privacy</h1>
<p>Open Cycling World is open source: the code is dual-licensed MIT / Apache-2.0
(<a href="{REPO}">{REPO}</a>). This page lists the data and assets in the browser preview.</p>

<h2>Map and world data</h2>
<p>Map data © <a href="https://www.openstreetmap.org/copyright">OpenStreetMap contributors</a>, available under the
<a href="https://opendatacommons.org/licenses/odbl/1-0/">Open Database License (ODbL) 1.0</a>. The worlds are built
from OpenStreetMap and open elevation, imagery and forest data by OCW's <code>world-builder</code>; no map
images or tile servers are used. Each world names its sources:</p>
{''.join(sec)}
<p>The built-in test routes (Hills, Loop) are generated by the app.</p>

<h2>Models, fonts and textures</h2>
<ul>
<li><b>Bikes, riders and props:</b> model files made by OCW (<code>artwork/3dmodels/</code>, exported by <code>crates/ocw-assets</code>), own work, licence
{e(' / '.join(model_licences))}.</li>
<li><b>Font:</b> Fira Mono (Mozilla, Carrois Apostrophe), SIL Open Font License 1.1 –
<a href="licenses/FiraMono-OFL.txt">licence text</a>.</li>
<li><b>Textures:</b> the browser preview draws the ground and roads in flat colours; the desktop app's
texture library is CC0 (ambientCG, Poly Haven):
<details><summary>The textures of the desktop app</summary><table>
<tr><th>Material</th><th>Title</th><th>Author</th><th>Licence</th><th></th></tr>{mats}</table></details></li>
<li><b>Voice chat:</b> LiveKit's JavaScript SDK (<code>livekit-client</code>, LiveKit, Inc., Apache-2.0) –
<a href="livekit-client.LICENSE.txt">licence text</a>.</li>
</ul>

<h2>Software</h2>
<p>The app is written in Rust with the <a href="https://bevyengine.org">Bevy</a> engine (MIT / Apache-2.0)
and compiled to WebAssembly with <code>wasm-bindgen</code> and binaryen's <code>wasm-opt</code>.</p>
{crate_rows}

<h2 id="privacy">Privacy</h2>
<ul>
<li><b>No tracking:</b> no analytics, no cookies, no advertising, no accounts. The page loads only from
this site; riding alone it does not call other servers.</li>
<li><b>Riding together</b> (the rides on the public server): the page connects to OCW's game and voice
server at <code>ocw-server.fly.dev</code> (hosted by Fly.io in Frankfurt). Everyone on it sees the name you
ride with, your rider and where you ride; the server gets your power, cadence and steering to run the
ride; its log at Fly.io notes when a rider joins and leaves, with the name and IP address. While you ride
there, the name you ride with is also shown to anyone on the server's public status page
(<a href="https://ocw-server.fly.dev/">ocw-server.fly.dev</a>), with how long and how far you have ridden this
time – never your IP address or where you ride. Your voice is off when a ride starts; when you turn it on (M), it
goes through the voice server to the riders who hear you, and is not recorded
(<a href="https://fly.io/legal/privacy-policy/">Fly.io's privacy policy</a>).</li>
<li><b>Your settings</b> (graphics, audio, controls, the trainer's mode, remembered Bluetooth devices) are kept
in your browser's <code>localStorage</code> for this site, and never leave your browser.</li>
<li><b>Worlds</b> are downloaded from this site and cached in your browser's storage (the Origin Private File
System), so the next visit loads them from there. Clear them with the browser's site data settings.</li>
<li><b>Bluetooth:</b> trainers and sensors connect only after you choose them in the browser's own chooser;
their data stays in the page.</li>
<li><b>Rides</b> are not uploaded anywhere; screenshots, videos and FIT files are downloads to your computer.</li>
<li><b>Hosting:</b> the site is served by GitHub Pages, which logs visitors' IP addresses for security
(<a href="https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement">GitHub's privacy statement</a>).</li>
</ul>
<p class="dim">Sign-in is coming in the hosted version; this note will grow with it.</p>
</main>
</body>
</html>
"""
    with open(os.path.join(out, "credits.html"), "w") as f:
        f.write(page)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--build", default="target/web-preview")
    p.add_argument("--worlds", default="target/web-preview-worlds/release")
    p.add_argument("--out", default="target/web-preview-site")
    p.add_argument("--no-server", action="store_true",
                   help="no riding together (config.json without the public server)")
    p.add_argument("--crates", default=None)
    p.add_argument("--build-id", default="")
    p.add_argument("--max-mb", type=float, default=1000.0)
    p.add_argument("--xr-build", default=None)
    a = p.parse_args()
    for f in ("index.html", "ocw_web_bg.wasm", "ocw_web_webgl2_bg.wasm"):
        if not os.path.isfile(os.path.join(a.build, f)):
            sys.exit(f"{a.build}/{f} missing: build the preview first (make web-preview)")
    if not built_with_simd(a.build):
        sys.exit(f"{a.build}: not built with WebAssembly SIMD (sizes.txt): rebuild with make web-preview")
    shutil.rmtree(a.out, ignore_errors=True)
    os.makedirs(a.out)
    for f in BUILD_FILES:
        src = os.path.join(a.build, f)
        if os.path.isfile(src):
            place(src, os.path.join(a.out, f))
    found = worlds(a.worlds, a.out)
    server = public_server(found, not a.no_server)
    if server and not os.path.isfile(os.path.join(a.out, "livekit-client.umd.js")):
        sys.exit(f"{a.build}/livekit-client.umd.js missing: the voice of the public server needs it"
                 " (scripts/build-web.sh --preview fetches it)")
    config = page_config(found, server, a.build_id, start_rides(found, app_bytes(a.build), server))
    with open(os.path.join(a.out, "config.json"), "w") as f:
        json.dump(config, f, indent=2, ensure_ascii=False)
        f.write("\n")
    credits(found, crates_table(a.crates) if a.crates else [], a.out)
    xr_files = xr_site(a.xr_build, found, a.out, a.build_id) if a.xr_build else 0
    open(os.path.join(a.out, ".nojekyll"), "w").close()
    total, lines = 0, []
    for d, _, files in os.walk(a.out):
        for f in sorted(files):
            path = os.path.join(d, f)
            size = os.path.getsize(path)
            total += size
            lines.append(f"{size:>12}  {os.path.relpath(path, a.out)}")
    lines.append(f"{total:>12}  total ({total / 1e6:.1f} MB)")
    with open(os.path.join(a.out, "site.txt"), "w") as f:
        f.write("\n".join(sorted(lines[:-1], key=lambda l: l.split()[-1])) + "\n" + lines[-1] + "\n")
    print("\n".join(lines[-1:]))
    print(f"site: {a.out}/ ({len(lines) - 1} files, {len(found)} worlds, {len(config['start'])} rides"
          + (f"; WebXR at xr/: {xr_files} files" if xr_files else "") + ")")
    if total > a.max_mb * 1e6:
        sys.exit(f"the site is {total / 1e6:.0f} MB, over {a.max_mb:.0f} MB (GitHub Pages: 1 GB)")


if __name__ == "__main__":
    main()
