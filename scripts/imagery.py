#!/usr/bin/env python3
"""Imagery downloads for region calibration (spec §14 Phase 8 T3, 8B).

Run by scripts/fetch-imagery.sh on the build machine; writes one folder per
source with an index.json that `world-builder calibrate` reads
(crates/ocw-worldgen/src/imagery.rs):

  imagery.py ortho     orthophoto cells from an official WMS (GetMap per
                       cell, JPEG, already resampled by the server to the
                       ground resolution the builder needs)
  imagery.py seasonal  our own Sentinel-2 L2A seasonal median composites
                       (spring, summer, autumn, winter) from the true-colour
                       COGs, clouds masked with the scene classification
                       (needs GDAL's Python bindings and numpy)

Only open sources under spec 8A licences; never Google / Bing / Esri /
Mapbox and never EOX mosaics (CC BY-NC-SA). No network in the unit tests:
the Rust side is tested with fixtures.
"""

import argparse
import concurrent.futures
import datetime
import json
import os
import sys
import time
import urllib.parse
import urllib.request

UA = "OCW-world-builder/1 (open data import)"


def http_get(url, data=None, headers=None, tries=4):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, data=data, headers={"User-Agent": UA, **(headers or {})})
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.headers.get("Content-Type", ""), r.read()
        except Exception:  # noqa: BLE001 - retried, then raised
            if i == tries - 1:
                raise
            time.sleep(2 * (i + 1))
    raise RuntimeError("unreachable")


def write_index(out, meta):
    tmp = os.path.join(out, "index.json.part")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=1)
    os.replace(tmp, os.path.join(out, "index.json"))


def source_meta(a):
    return {
        "id": a.id,
        "name": a.name,
        "licence": a.licence,
        "notice": a.notice,
        "url": a.url,
        "date": a.date,
    }


# --- Orthophotos ------------------------------------------------------------


def ortho(a):
    os.makedirs(a.out, exist_ok=True)
    cells = [l.strip() for l in open(a.cells) if l.strip()]
    px = int(round(a.cell / a.pixel))
    base = a.wms + ("&" if "?" in a.wms else "?")

    def one(c):
        e_km, n_km = (float(v) for v in c.split("_"))
        name = f"{c}.jpg"
        path = os.path.join(a.out, name)
        if os.path.exists(path) and os.path.getsize(path) > 0:
            return "have"
        e, n = e_km * 1000, n_km * 1000
        q = {
            "SERVICE": "WMS",
            "VERSION": "1.3.0",
            "REQUEST": "GetMap",
            "LAYERS": a.layer,
            "STYLES": "",
            "CRS": f"EPSG:{a.epsg}",
            "BBOX": f"{e},{n},{e + a.cell},{n + a.cell}",
            "WIDTH": str(px),
            "HEIGHT": str(px),
            "FORMAT": "image/jpeg",
        }
        ctype, body = http_get(base + urllib.parse.urlencode(q))
        if not ctype.startswith("image/jpeg") or body[:2] != b"\xff\xd8":
            raise RuntimeError(f"{c}: {ctype}: {body[:200]!r}")
        with open(path + ".part", "wb") as f:
            f.write(body)
        os.replace(path + ".part", path)
        return "new"

    done = {"have": 0, "new": 0}
    errors = []
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for c, r in zip(cells, ex.map(lambda c: _safe(one, c), cells)):
            if isinstance(r, Exception):
                errors.append(f"{c}: {r}")
            else:
                done[r] += 1
    meta = source_meta(a) | {
        "kind": "ortho",
        "crs": a.crs,
        "cell_m": a.cell,
        "pixel_m": a.pixel,
        "format": "jpeg",
        # Outside a service's coverage the WMS paints its background.
        "nodata": [[255, 255, 255], [0, 0, 0]],
        "nodata_tolerance": 3,
        "service": a.wms,
        "layer": a.layer,
    }
    write_index(a.out, meta)
    size = sum(os.path.getsize(os.path.join(a.out, f)) for f in os.listdir(a.out) if f.endswith(".jpg"))
    print(f"{a.id}: {len(cells)} cells ({done['new']} new, {done['have']} kept, {len(errors)} failed), {size / 1e6:.0f} MB")
    for e in errors[:10]:
        print("  error:", e, file=sys.stderr)
    return 1 if errors else 0


def _safe(f, x):
    try:
        return f(x)
    except Exception as e:  # noqa: BLE001 - reported per cell
        return e


# --- Sentinel-2 seasonal composites -----------------------------------------

# The composites' date windows (month, day), northern hemisphere: fresh
# spring green, summer, the autumn colours, winter (snow). The centre day is
# what the renderer blends by (ocw_world::look::season_weights).
SEASONS = {
    "spring": ((4, 1), (5, 31)),
    "summer": ((6, 15), (8, 31)),
    "autumn": ((9, 25), (11, 10)),
    "winter": ((12, 1), (2, 28)),
}
# Scene classification classes kept: vegetation, not vegetated, water, snow.
SCL_KEEP = (4, 5, 6, 11)


def doy(m, d):
    return datetime.date(2025, m, d).timetuple().tm_yday


def windows(season, years):
    (m0, d0), (m1, d1) = SEASONS[season]
    out = []
    for y in years:
        y0 = y - 1 if m0 > m1 else y  # winter starts in the year before
        out.append((datetime.date(y0, m0, d0), datetime.date(y, m1, d1)))
    return out


def stac_search(stac, collection, bbox, start, end, max_cloud):
    items, body = [], {
        "collections": [collection],
        "bbox": bbox,
        "datetime": f"{start}T00:00:00Z/{end}T23:59:59Z",
        "limit": 200,
        "query": {"eo:cloud_cover": {"lt": max_cloud}},
    }
    url = stac.rstrip("/") + "/search"
    while url:
        _, raw = http_get(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
        d = json.loads(raw)
        items += d["features"]
        nxt = [l for l in d.get("links", []) if l.get("rel") == "next"]
        if not nxt:
            break
        url, body = nxt[0]["href"], nxt[0].get("body", body)
    return items


def seasonal(a):
    import numpy as np
    from osgeo import gdal, osr

    gdal.UseExceptions()
    for k, v in {
        "GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR",
        "CPL_VSIL_CURL_ALLOWED_EXTENSIONS": ".tif",
        "GDAL_HTTP_MULTIRANGE": "YES",
        "GDAL_HTTP_MERGE_CONSECUTIVE_RANGES": "YES",
        "GDAL_HTTP_MAX_RETRY": "5",
        "GDAL_HTTP_RETRY_DELAY": "2",
        "VSI_CACHE": "TRUE",
    }.items():
        gdal.SetConfigOption(k, v)
    os.makedirs(a.out, exist_ok=True)
    # The grid: the extent of the package's cells in the output CRS
    # (`world-builder imagery-cells`, 1 km), snapped to the pixel size.
    cells = [tuple(float(v) * 1000 for v in l.split("_")) for l in open(a.cells) if l.strip()]
    west = min(c[0] for c in cells) - a.margin
    south = min(c[1] for c in cells) - a.margin
    east = max(c[0] for c in cells) + 1000 + a.margin
    north = max(c[1] for c in cells) + 1000 + a.margin
    p = a.pixel
    west, south = p * np.floor(west / p), p * np.floor(south / p)
    east, north = p * np.ceil(east / p), p * np.ceil(north / p)
    width, height = int(round((east - west) / p)), int(round((north - south) / p))
    srs = osr.SpatialReference()
    srs.ImportFromEPSG(a.epsg)
    srs.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    wgs = osr.SpatialReference()
    wgs.ImportFromEPSG(4326)
    wgs.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    to_ll = osr.CoordinateTransformation(srs, wgs)
    corners = [to_ll.TransformPoint(x, y)[:2] for x in (west, east) for y in (south, north)]
    bbox = [min(c[0] for c in corners), min(c[1] for c in corners), max(c[0] for c in corners), max(c[1] for c in corners)]
    print(f"grid {width} x {height} at {p} m, EPSG:{a.epsg}, bbox {', '.join(f'{v:.3f}' for v in bbox)}")
    years = range(int(a.years.split("-")[0]), int(a.years.split("-")[-1]) + 1)
    meta = source_meta(a) | {
        "kind": "seasonal",
        "crs": a.crs,
        "pixel_m": p,
        "west": west,
        "north": north,
        "width": width,
        "height": height,
        "seasons": [],
        "stac": a.stac,
        "collection": a.collection,
    }
    for season in a.seasons.split(","):
        out_file = f"{season}.rgb"
        info_file = os.path.join(a.out, f"{season}.json")
        items = []
        for start, end in windows(season, years):
            items += stac_search(a.stac, a.collection, bbox, start, end, a.max_cloud)
        # The clearest scenes per MGRS tile, at most --scenes each.
        by_tile = {}
        for it in items:
            code = it["properties"].get("grid:code") or it["properties"].get("s2:mgrs_tile") or it["id"].split("_")[1]
            by_tile.setdefault(code, []).append(it)
        chosen = []
        for code, its in sorted(by_tile.items()):
            its.sort(key=lambda i: i["properties"].get("eo:cloud_cover", 100))
            chosen += its[: a.scenes]
        (m0, d0), (m1, d1) = SEASONS[season]
        first, last = doy(m0, d0), doy(m1, d1)
        span = (last - first) % 365
        centre = (first + span // 2 - 1) % 365 + 1
        entry = {"season": season, "centre_doy": centre, "first_doy": first, "last_doy": last, "file": out_file}
        if os.path.exists(os.path.join(a.out, out_file)) and os.path.exists(info_file) and not a.force:
            entry = json.load(open(info_file))
            print(f"{season}: have ({entry['scenes']} scenes)")
            meta["seasons"].append(entry)
            continue
        print(f"{season}: {len(items)} scenes found in {len(by_tile)} MGRS tiles, {len(chosen)} used ({first}-{last})")
        stack_path = os.path.join(a.out, f".{season}.stack")
        stack = np.lib.format.open_memmap(stack_path, mode="w+", dtype=np.uint8, shape=(len(chosen), height, width, 3))

        def warp(i_it):
            i, it = i_it
            href = lambda k: "/vsicurl/" + it["assets"][k]["href"]  # noqa: E731
            opts = dict(
                format="MEM",
                outputBounds=(west, south, east, north),
                width=width,
                height=height,
                dstSRS=f"EPSG:{a.epsg}",
                multithread=False,
            )
            try:
                tci = gdal.Warp("", href("visual"), resampleAlg="average", srcNodata=0, dstNodata=0, **opts)
                scl = gdal.Warp("", href("scl"), resampleAlg="near", srcNodata=0, dstNodata=0, **opts)
                rgb = np.stack([tci.GetRasterBand(b).ReadAsArray() for b in (1, 2, 3)], axis=-1)
                cls = scl.GetRasterBand(1).ReadAsArray()
                ok = np.isin(cls, SCL_KEEP) & (rgb.min(axis=-1) > 0)
                rgb[~ok] = 0
                stack[i] = rgb
                return int(ok.sum())
            except Exception as e:  # noqa: BLE001 - one bad scene is skipped
                print(f"  {it['id']}: {e}", file=sys.stderr)
                return 0

        with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
            valid = list(ex.map(warp, enumerate(chosen)))
        used = sum(1 for v in valid if v > 0)
        # Per-pixel median of the clear observations, in strips.
        out = np.zeros((height, width, 3), np.uint8)
        counts = np.zeros((height, width), np.uint16)
        for y0 in range(0, height, 128):
            s = stack[:, y0 : y0 + 128].astype(np.float32)
            clear = s.min(axis=-1) > 0
            s[~clear] = np.nan
            with np.errstate(all="ignore"):
                import warnings

                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", category=RuntimeWarning)
                    med = np.nanmedian(s, axis=0)
            med = np.nan_to_num(med, nan=0.0)
            out[y0 : y0 + 128] = np.clip(np.round(med), 0, 255).astype(np.uint8)
            counts[y0 : y0 + 128] = clear.sum(axis=0)
        # A pixel with a median of exactly 0 in a channel would read as no
        # data: lift it to 1.
        has = counts > 0
        out[has] = np.maximum(out[has], 1)
        out[~has] = 0
        out.tofile(os.path.join(a.out, out_file + ".part"))
        os.replace(os.path.join(a.out, out_file + ".part"), os.path.join(a.out, out_file))
        del stack
        os.remove(stack_path)
        # A preview (JPEG) to look at.
        mem = gdal.GetDriverByName("MEM").Create("", width, height, 3, gdal.GDT_Byte)
        for b in range(3):
            mem.GetRasterBand(b + 1).WriteArray(out[:, :, b])
        gdal.GetDriverByName("JPEG").CreateCopy(os.path.join(a.out, f"{season}-preview.jpg"), mem, options=["QUALITY=85"])
        cover = 100.0 * has.mean()
        dates = sorted(it["properties"]["datetime"][:10] for it, v in zip(chosen, valid) if v > 0)
        entry |= {
            "scenes": used,
            "coverage_percent": round(cover, 2),
            "median_observations": int(np.median(counts[has])) if has.any() else 0,
            "dates": f"{dates[0]} .. {dates[-1]}" if dates else "",
            "scene_ids": [it["id"] for it, v in zip(chosen, valid) if v > 0],
        }
        json.dump(entry, open(info_file, "w"), indent=1)
        print(f"{season}: {used} scenes, {cover:.1f} % covered, median {entry['median_observations']} clear observations per pixel")
        meta["seasons"].append(entry)
    write_index(a.out, meta)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("ortho", "seasonal"):
        s = sub.add_parser(name)
        s.add_argument("--out", required=True)
        s.add_argument("--cells", required=True, help="E_N cells (km), from world-builder imagery-cells")
        s.add_argument("--crs", required=True, help="CRS name for the builder (lv95, utm32, utm28)")
        s.add_argument("--epsg", type=int, required=True)
        s.add_argument("--pixel", type=float, required=True, help="metres per pixel")
        s.add_argument("--jobs", type=int, default=4)
        for k in ("id", "name", "licence", "notice", "url", "date"):
            s.add_argument("--" + k, required=k not in ("url", "date"), default="")
    o = sub.choices["ortho"]
    o.add_argument("--wms", required=True)
    o.add_argument("--layer", required=True)
    o.add_argument("--cell", type=float, default=1000.0)
    se = sub.choices["seasonal"]
    se.add_argument("--stac", default="https://earth-search.aws.element84.com/v1")
    se.add_argument("--collection", default="sentinel-2-c1-l2a")
    se.add_argument("--years", default="2023-2025")
    se.add_argument("--seasons", default="spring,summer,autumn,winter")
    se.add_argument("--scenes", type=int, default=12, help="clearest scenes per MGRS tile and season")
    se.add_argument("--max-cloud", type=float, default=60.0)
    se.add_argument("--margin", type=float, default=500.0)
    se.add_argument("--force", action="store_true")
    a = ap.parse_args()
    sys.exit(ortho(a) if a.cmd == "ortho" else seasonal(a))


if __name__ == "__main__":
    main()
