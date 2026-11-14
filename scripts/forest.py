#!/usr/bin/env python3
"""Forest type and density downloads (spec §14 Phase 8 T3).

Run by scripts/fetch-forest.sh on the build machine; writes the folder that
`world-builder calibrate --forest DIR` / `world-builder forest` read
(crates/ocw-worldgen/src/forest.rs): an index.json and two raw 8-bit
rasters in ETRS89-LAEA Europe (EPSG:3035), rows from the north:

  leaf-type.u8  Copernicus HRL Dominant Leaf Type 2018, 10 m
                (0 no trees, 1 broadleaved, 2 coniferous, 254 unclassifiable,
                255 outside the coverage)
  density.u8    Copernicus HRL Tree Cover Density 2018, 10 m (0-100 %,
                254 unclassifiable, 255 outside)

Both come from the EEA's public ArcGIS ImageServers (image.discomap.eea.
europa.eu, GioLandPublic/HRL_DominantLeafType2018 and
HRL_TreeCoverDensity_2018): `exportImage` in EPSG:3035 at 10 m with
nearest-neighbour sampling, as raw band-sequential bytes (format=bsq: the
values, then a validity bit mask), in blocks of at most 2,000 px. No
account is needed (the CLMS portal's downloads need an EU Login).

Licence: Copernicus Land Monitoring Service data policy (free, full and
open; the source must be named and modifications stated): "© European
Union, Copernicus Land Monitoring Service 2018, European Environment Agency
(EEA)".

  forest.py fetch --out DIR (--cells FILE | --bbox W,S,E,N) [--jobs N]

`--cells` lists 1 km cells in EPSG:3035 as `E_N` (km, lower-left corners,
`world-builder imagery-cells --crs laea`); `--bbox` is a box in EPSG:3035
metres (aligned to the pixel size). Only blocks with a listed cell are
downloaded; the rest of the mosaic is 255.
"""

import argparse
import concurrent.futures
import json
import os
import sys
import time
import urllib.request

UA = "OCW-world-builder/1 (open data import)"
BASE = "https://image.discomap.eea.europa.eu/arcgis/rest/services/GioLandPublic/{}/ImageServer/exportImage"
LAYERS = {
    "leaf": ("HRL_DominantLeafType2018", "leaf-type.u8"),
    "density": ("HRL_TreeCoverDensity_2018", "density.u8"),
}
NOTICE = "© European Union, Copernicus Land Monitoring Service 2018, European Environment Agency (EEA)"
SOURCES = [
    {
        "id": "hrl-dlt-2018",
        "name": "Copernicus HRL Dominant Leaf Type 2018",
        "licence": "Copernicus Land Monitoring Service data policy (free, full and open; attribution)",
        "notice": NOTICE + "; Dominant Leaf Type 2018, 10 m, resampled by OCW",
        "url": "https://land.copernicus.eu/en/products/high-resolution-layer-dominant-leaf-type",
        "date": "2018",
    },
    {
        "id": "hrl-tcd-2018",
        "name": "Copernicus HRL Tree Cover Density 2018",
        "licence": "Copernicus Land Monitoring Service data policy (free, full and open; attribution)",
        "notice": NOTICE + "; Tree Cover Density 2018, 10 m, resampled by OCW",
        "url": "https://land.copernicus.eu/en/products/high-resolution-layer-tree-cover-density",
        "date": "2018",
    },
]
NO_DATA = 255


def http_get(url, tries=4):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=180) as r:
                return r.headers.get("Content-Type", ""), r.read()
        except Exception:  # noqa: BLE001 - retried, then raised
            if i == tries - 1:
                raise
            time.sleep(3 * (i + 1))
    raise RuntimeError("unreachable")


def export(service, w, s, e, n, pixel):
    """One block of `service` as bytes (rows from the north), 255 where the
    validity mask says no data."""
    cols, rows = round((e - w) / pixel), round((n - s) / pixel)
    url = (
        BASE.format(service)
        + f"?bbox={w},{s},{e},{n}&bboxSR=3035&imageSR=3035&size={cols},{rows}"
        + "&format=bsq&pixelType=U8&noData=255&interpolation=RSP_NearestNeighbor&f=image"
    )
    for attempt in range(3):
        ctype, body = http_get(url)
        count = cols * rows
        if len(body) == count:
            return bytes(body)
        if len(body) == count + (count + 7) // 8:
            vals = bytearray(body[:count])
            mask = body[count:]
            for i in range(count):
                if not (mask[i >> 3] >> (7 - (i & 7))) & 1:
                    vals[i] = NO_DATA
            return bytes(vals)
        msg = body[:200].decode("utf-8", "replace")
        print(f"  {service} {w},{s}: unexpected answer ({ctype}, {len(body)} bytes): {msg}", file=sys.stderr)
        time.sleep(5 * (attempt + 1))
    raise RuntimeError(f"{service}: no raster for {w},{s},{e},{n}")


def fetch(a):
    os.makedirs(a.out, exist_ok=True)
    px = a.pixel
    if a.bbox:
        w, s, e, n = (float(v) for v in a.bbox.split(","))
        cells = None
    else:
        cells = set()
        with open(a.cells, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    ce, cn = line.split("_")
                    cells.add((round(float(ce)), round(float(cn))))
        if not cells:
            sys.exit(f"{a.cells}: no cells")
        w = min(c[0] for c in cells) * 1000.0
        s = min(c[1] for c in cells) * 1000.0
        e = (max(c[0] for c in cells) + 1) * 1000.0
        n = (max(c[1] for c in cells) + 1) * 1000.0
    width, height = round((e - w) / px), round((n - s) / px)
    step = a.block * px
    blocks = []
    y = n
    while y > s:
        x = w
        while x < e:
            bw, bs, be, bn = x, max(s, y - step), min(e, x + step), y
            if cells is None or any(
                bw <= c[0] * 1000.0 < be and bs <= c[1] * 1000.0 < bn for c in cells
            ):
                blocks.append((bw, bs, be, bn))
            x += step
        y -= step
    print(f"forest rasters: {width} × {height} px of {px} m, {len(blocks)} blocks, {a.out}")
    mosaics = {k: bytearray([NO_DATA]) * (width * height) for k in LAYERS}

    def one(job):
        key, (bw, bs, be, bn) = job
        return key, (bw, bs, be, bn), export(LAYERS[key][0], bw, bs, be, bn, px)

    jobs = [(k, b) for b in blocks for k in LAYERS]
    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for i, (key, (bw, bs, be, bn), data) in enumerate(ex.map(one, jobs)):
            cols = round((be - bw) / px)
            x0, y0 = round((bw - w) / px), round((n - bn) / px)
            m = mosaics[key]
            for r in range(round((bn - bs) / px)):
                at = (y0 + r) * width + x0
                m[at : at + cols] = data[r * cols : (r + 1) * cols]
            if (i + 1) % 10 == 0 or i + 1 == len(jobs):
                print(f"  {i + 1}/{len(jobs)} blocks, {time.time() - t0:.0f} s")
    for key, (_, name) in LAYERS.items():
        tmp = os.path.join(a.out, name + ".part")
        with open(tmp, "wb") as f:
            f.write(mosaics[key])
        os.replace(tmp, os.path.join(a.out, name))
    meta = {
        "kind": "forest",
        "crs": "laea",
        "pixel_m": px,
        "west": w,
        "north": n,
        "width": width,
        "height": height,
        "leaf": LAYERS["leaf"][1],
        "density": LAYERS["density"][1],
        "sources": SOURCES,
    }
    tmp = os.path.join(a.out, "index.json.part")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=1)
    os.replace(tmp, os.path.join(a.out, "index.json"))
    leaf = mosaics["leaf"]
    counts = [leaf.count(v) for v in (0, 1, 2)]
    total = max(1, sum(counts))
    print(
        f"leaf type: {100 * counts[1] / total:.1f} % broadleaved, {100 * counts[2] / total:.1f} % coniferous, "
        f"{100 * counts[0] / total:.1f} % without trees ({width * height - sum(counts)} px no data)"
    )


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("--out", required=True)
    f.add_argument("--cells")
    f.add_argument("--bbox")
    f.add_argument("--pixel", type=float, default=10.0)
    f.add_argument("--block", type=int, default=2000, help="block size in pixels (the server allows 4,100)")
    f.add_argument("--jobs", type=int, default=4)
    a = p.parse_args()
    if a.cmd == "fetch":
        if not (a.cells or a.bbox):
            sys.exit("--cells or --bbox")
        fetch(a)


if __name__ == "__main__":
    main()
