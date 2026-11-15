#!/usr/bin/env python3
"""Measured crop maps (spec §4.6.2, §14 Phase 8 T8).

Run by scripts/fetch-crops.sh on the build machine (needs the GDAL Python
bindings and numpy, as on marge); writes the folder that `world-builder
vegetation --crops DIR` reads (crates/ocw-worldgen/src/vegetation.rs): an
index.json and one raw 8-bit raster in ETRS89-LAEA Europe (EPSG:3035),
10 m, rows from the north, holding OCW crop codes (`FieldCrop`: 0 none,
1 cereal, 2 maize, 3 rapeseed, 4 sunflower, 5 potato, 6 beet, 7 grass,
8 vegetables, 9 vineyard, 10 orchard, 11 banana, 12 berries, 13 fallow,
14 greenhouse, 15 avocado / citrus; 255 no data).

Sources (licences checked 2026-11-15, no account needed):

  eucropmap  JRC EUCROPMAP 2022 (EU-27 crop types at 10 m from Sentinel-1,
             LUCAS 2022), one tiled BigTIFF in EPSG:3035 read by HTTP range
             requests (GDAL /vsicurl/):
             https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/EUCROPMAP/2022/
             Licence CC BY 4.0 ("© European Union, Joint Research Centre").
             Covers Germany (the Black Forest); not Liechtenstein, not the
             Canary Islands. Its class 300 lumps woodland, vineyards and
             orchards: those stay with OSM.
  canarias   Mapa de Cultivos de Canarias 2021, Tenerife (Gobierno de
             Canarias; one polygon per parcel with its crop), shapefile from
             opendata.sitcan.es (gobcan_mapa-cultivos_tf_shp.zip), licence
             CC BY 4.0; rasterised here.

  crops.py eucropmap --out DIR --cells FILE [--jobs N]
  crops.py canarias  --out DIR --cells FILE --shp FILE.shp

`--cells` lists 1 km cells in EPSG:3035 as `E_N` (km, lower-left corners;
`world-builder imagery-cells --crs laea`).
"""

import argparse
import concurrent.futures
import json
import os
import sys
import time

import numpy as np
from osgeo import gdal, ogr, osr

gdal.UseExceptions()
ogr.UseExceptions()
UA = "OCW-world-builder/1 (open data import)"
NO_DATA = 255
PIXEL = 10.0
EUCROPMAP_URL = (
    "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/EUCROPMAP/2022/EU_CropMap_22_v1_stratum_EU27-HR.tif"
)

# EUCROPMAP legend (EuroCropMap2022.qml) -> OCW crop codes.
EUCROPMAP_CODES = {
    211: 1, 212: 1, 213: 1, 214: 1, 215: 1, 217: 1, 218: 1, 219: 1,  # cereals
    216: 2,  # maize
    232: 3,  # rape and turnip rape
    231: 4,  # sunflower
    221: 5,  # potatoes
    222: 6, 223: 6,  # sugar beet, other root crops
    250: 7, 500: 7,  # fodder crops, grassland
    230: 8, 233: 8, 240: 8,  # other industrial crops, soya, dry pulses
    290: 13,  # bare arable land
}
# 100 artificial, 300 woodland / permanent crops, 600 bare land, 700 water,
# 800 wetland: 0 (no field crop, OSM decides).

SOURCES = {
    "eucropmap": {
        "id": "jrc-eucropmap-2022",
        "name": "JRC EUCROPMAP 2022",
        "licence": "CC BY 4.0",
        "notice": "EUCROPMAP 2022 © European Union, Joint Research Centre (JRC), CC BY 4.0; crop types reclassified and resampled by OCW",
        "url": "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/EUCROPMAP/",
        "date": "2022",
    },
    "canarias": {
        "id": "gobcan-mapa-cultivos-2021",
        "name": "Mapa de Cultivos de Canarias 2021 (Tenerife)",
        "licence": "CC BY 4.0",
        "notice": "Mapa de Cultivos de Canarias 2021 © Gobierno de Canarias, CC BY 4.0; crops reclassified and rasterised by OCW",
        "url": "https://opendata.sitcan.es/upload/medio-rural/gobcan_mapa-cultivos_tf_shp.zip",
        "date": "2021",
    },
}


def canarias_code(categoria, agrupacion, cultivo):
    """OCW crop code of a parcel of the Mapa de Cultivos de Canarias."""
    c, a, n = (categoria or ""), (agrupacion or ""), (cultivo or "")
    if c == "Platanera":
        return 11
    if c == "Viña":
        return 9
    if a == "Papa":
        return 5
    if a == "Barbecho" or c.startswith("Superficie agrícola no utilizada") or c.startswith("Pastoreo"):
        return 13
    if c == "Frutales":
        if a in ("Frutales Subtropicales", "Cítricos"):
            return 0 if n == "Tunera" else 15
        return 10
    if n == "Millo":
        return 2
    if n == "Cereal Otros":
        return 1
    if n == "Tagasaste":
        return 7
    if c in ("Huerta", "Tomate", "Ornamentales") or a in ("Hortalizas", "Cereales, Leguminosas y Forrajeras"):
        return 8
    return 0


def read_cells(path):
    cells = set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                ce, cn = line.split("_")
                cells.add((round(float(ce)), round(float(cn))))
    if not cells:
        sys.exit(f"{path}: no cells")
    return cells


def grid(cells):
    w = min(c[0] for c in cells) * 1000.0
    s = min(c[1] for c in cells) * 1000.0
    e = (max(c[0] for c in cells) + 1) * 1000.0
    n = (max(c[1] for c in cells) + 1) * 1000.0
    return w, s, e, n, round((e - w) / PIXEL), round((n - s) / PIXEL)


def write(out, source, w, n, width, height, mosaic):
    os.makedirs(out, exist_ok=True)
    tmp = os.path.join(out, "crops.u8.part")
    mosaic.astype(np.uint8).tofile(tmp)
    os.replace(tmp, os.path.join(out, "crops.u8"))
    meta = {
        "kind": "crops",
        "crs": "laea",
        "pixel_m": PIXEL,
        "west": w,
        "north": n,
        "width": width,
        "height": height,
        "crops": "crops.u8",
        "sources": [SOURCES[source]],
    }
    tmp = os.path.join(out, "index.json.part")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=1)
    os.replace(tmp, os.path.join(out, "index.json"))
    counts = np.bincount(mosaic.ravel(), minlength=256)
    names = ["none", "cereal", "maize", "rapeseed", "sunflower", "potato", "beet", "grass",
             "vegetables", "vineyard", "orchard", "banana", "berries", "fallow", "greenhouse", "avocado/citrus"]
    shown = ", ".join(f"{names[i]} {counts[i] / 100:.0f} ha" for i in range(1, 16) if counts[i])
    print(f"crops: {width} × {height} px of {PIXEL} m: {shown}; {counts[NO_DATA]} px no data")


def eucropmap(a):
    cells = read_cells(a.cells)
    w, s, e, n, width, height = grid(cells)
    for k, v in {
        "GDAL_HTTP_USERAGENT": UA,
        "GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR",
        "CPL_VSIL_CURL_ALLOWED_EXTENSIONS": ".tif",
        "GDAL_HTTP_MULTIRANGE": "YES",
        "GDAL_HTTP_MERGE_CONSECUTIVE_RANGES": "YES",
        "GDAL_HTTP_MAX_RETRY": "5",
        "GDAL_HTTP_RETRY_DELAY": "3",
    }.items():
        gdal.SetConfigOption(k, v)
    ds = gdal.Open("/vsicurl/" + EUCROPMAP_URL)
    gt = ds.GetGeoTransform()
    if abs(gt[1] - PIXEL) > 1e-6 or abs(gt[5] + PIXEL) > 1e-6:
        sys.exit(f"unexpected pixel size {gt}")
    lut = np.zeros(1024, dtype=np.uint8)
    for k, v in EUCROPMAP_CODES.items():
        lut[k] = v
    mosaic = np.full((height, width), NO_DATA, dtype=np.uint8)
    block = 2048
    jobs = []
    for y0 in range(0, height, block):
        for x0 in range(0, width, block):
            bw, bn = w + x0 * PIXEL, n - y0 * PIXEL
            be, bs = min(e, bw + block * PIXEL), max(s, bn - block * PIXEL)
            if any(bw <= c[0] * 1000.0 < be and bs <= c[1] * 1000.0 < bn for c in cells):
                jobs.append((x0, y0, round((be - bw) / PIXEL), round((bn - bs) / PIXEL)))
    print(f"EUCROPMAP 2022: {width} × {height} px, {len(jobs)} blocks of {block} px")

    def one(job):
        x0, y0, cols, rows = job
        # Pixel offsets in the source (its origin is on the 10 m grid).
        sx = round((w + x0 * PIXEL - gt[0]) / PIXEL)
        sy = round((gt[3] - (n - y0 * PIXEL)) / PIXEL)
        local = gdal.Open("/vsicurl/" + EUCROPMAP_URL)
        band = local.GetRasterBand(1)
        cx0, cy0 = max(sx, 0), max(sy, 0)
        cx1, cy1 = min(sx + cols, local.RasterXSize), min(sy + rows, local.RasterYSize)
        out = np.full((rows, cols), NO_DATA, dtype=np.uint8)
        if cx1 > cx0 and cy1 > cy0:
            for attempt in range(4):
                try:
                    v = band.ReadAsArray(cx0, cy0, cx1 - cx0, cy1 - cy0)
                    break
                except RuntimeError as err:  # retried, then raised
                    if attempt == 3:
                        raise
                    print(f"  retry {job}: {err}", file=sys.stderr)
                    time.sleep(5 * (attempt + 1))
            codes = np.where((v > 0) & (v < 1024), lut[np.clip(v, 0, 1023)], NO_DATA)
            out[cy0 - sy : cy1 - sy, cx0 - sx : cx1 - sx] = codes
        return job, out

    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for i, ((x0, y0, cols, rows), data) in enumerate(ex.map(one, jobs)):
            mosaic[y0 : y0 + rows, x0 : x0 + cols] = data
            print(f"  {i + 1}/{len(jobs)} blocks, {time.time() - t0:.0f} s", flush=True)
    write(a.out, "eucropmap", w, n, width, height, mosaic)


def canarias(a):
    cells = read_cells(a.cells)
    w, s, e, n, width, height = grid(cells)
    src = ogr.Open(a.shp)
    layer = src.GetLayer()
    laea = osr.SpatialReference()
    laea.ImportFromEPSG(3035)
    laea.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    srs = layer.GetSpatialRef()
    srs.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    tr = osr.CoordinateTransformation(srs, laea)
    mem = ogr.GetDriverByName("Memory").CreateDataSource("crops")
    out_layer = mem.CreateLayer("crops", laea, ogr.wkbMultiPolygon)
    out_layer.CreateField(ogr.FieldDefn("code", ogr.OFTInteger))
    defn = out_layer.GetLayerDefn()
    kept = 0
    for f in layer:
        code = canarias_code(f.GetField("CATEGORIA"), f.GetField("AGRUPACION"), f.GetField("CULTIVO_NA"))
        g = f.GetGeometryRef()
        if g is None:
            continue
        g = g.Clone()
        g.Transform(tr)
        o = ogr.Feature(defn)
        o.SetField("code", code)
        o.SetGeometry(g)
        out_layer.CreateFeature(o)
        kept += 1
    print(f"Mapa de Cultivos: {kept} parcels")
    drv = gdal.GetDriverByName("MEM")
    ds = drv.Create("", width, height, 1, gdal.GDT_Byte)
    ds.SetGeoTransform((w, PIXEL, 0.0, n, 0.0, -PIXEL))
    ds.SetProjection(laea.ExportToWkt())
    band = ds.GetRasterBand(1)
    band.Fill(NO_DATA)
    gdal.RasterizeLayer(ds, [1], out_layer, options=["ATTRIBUTE=code"])
    write(a.out, "canarias", w, n, width, height, band.ReadAsArray())


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("eucropmap")
    e.add_argument("--out", required=True)
    e.add_argument("--cells", required=True)
    e.add_argument("--jobs", type=int, default=4)
    c = sub.add_parser("canarias")
    c.add_argument("--out", required=True)
    c.add_argument("--cells", required=True)
    c.add_argument("--shp", required=True)
    a = p.parse_args()
    {"eucropmap": eucropmap, "canarias": canarias}[a.cmd](a)


if __name__ == "__main__":
    main()
