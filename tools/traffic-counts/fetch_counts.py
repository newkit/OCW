#!/usr/bin/env python3
"""Open traffic counts for the worlds (spec 14 Phase 18, 18D, 18E T11).

Downloads the open counting-station data of a world once (cached in the
world's data folder, `DATA/traffic-counts/<source>/`; a file already there
is never fetched again, two seconds between requests), checks that each
source's licence is still the one 18D recorded, and writes one counts file
per source in the OCW format that `world-builder traffic-counts --counts`
reads (`DATA/traffic-counts/<source>.counts.csv`):

    python3 -I tools/traffic-counts/fetch_counts.py tenerife data/tenerife
    python3 -I tools/traffic-counts/fetch_counts.py liechtenstein data/pilot
    python3 -I tools/traffic-counts/fetch_counts.py blackforest data/blackforest

Sources (18D, re-checked 2026-11-28):
  cabildo-tenerife-imd  Cabildo de Tenerife, IMD 2025 per station + station
                        positions (datos.tenerife.es, CKAN, CC BY)
  li-miv                Liechtenstein's permanent MIV stations: positions
                        (opendata.swiss, Esri file geodatabase) and the 2025
                        yearly report (DTV per station, PDF), terms_by
  astra-svz             Swiss ASTRA counts on the principal network, DTV
                        2025 (data.geo.admin.ch, INTERLIS), terms_open
  bw-svz                Baden-Württemberg's SVZ stations with DTV 2024
                        (MobiData BW, CSV), DL-DE-BY 2.0
Andorra: no open counting-station data found (18D).

Needs only the standard library, plus `pdftotext` (poppler) and the Python
package `pyogrio` for Liechtenstein (its station positions are a file
geodatabase); run it with an interpreter that has pyogrio, e.g. a venv.
"""

import csv
import io
import json
import math
import os
import re
import struct
import subprocess
import sys
import time
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

UA = "ocw-world-builder/1 (open-source cycling game; one-time download of open traffic counts)"
_last = [0.0]


def fetch(url, path):
    """Downloads url to path unless it is there (gently: 2 s apart)."""
    if os.path.exists(path) and os.path.getsize(path) > 0:
        return path
    os.makedirs(os.path.dirname(path), exist_ok=True)
    wait = 2.0 - (time.time() - _last[0])
    if wait > 0:
        time.sleep(wait)
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r:
        data = r.read()
    _last[0] = time.time()
    tmp = path + ".part"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)
    print(f"  fetched {url} ({len(data)} bytes)")
    return path


def ckan(base, dataset, folder):
    path = fetch(f"{base}/api/3/action/package_show?id={dataset}",
                 os.path.join(folder, f"{dataset}.package.json"))
    with open(path, encoding="utf-8") as f:
        return json.load(f)["result"]


def expect(what, got, want):
    if got not in want:
        sys.exit(f"{what}: licence is now {got!r}, 18D recorded {want!r} – re-check the terms "
                 "before using it (delete the cached .package.json to fetch it again)")


def write_counts(out, meta, rows):
    cols = ["id", "name", "lat", "lon", "ref", "aadt", "aadt_dir1", "aadt_dir2", "dir1_bearing_deg"]
    with open(out, "w", encoding="utf-8", newline="") as f:
        f.write("# ocw-traffic-counts 1\n")
        for k in ["source", "title", "licence", "notice", "url", "year"]:
            v = str(meta.get(k, "")).replace("\n", " ")
            f.write(f"# {k}: {v}\n")
        w = csv.writer(f, lineterminator="\n")
        w.writerow(cols)
        for r in rows:
            w.writerow(["" if r.get(c) is None else r.get(c) for c in cols])
    print(f"  {out}: {len(rows)} stations")


def bearing(a, b):
    """Compass bearing (deg) from (lat, lon) a to b."""
    la1, la2 = math.radians(a[0]), math.radians(b[0])
    dl = math.radians(b[1] - a[1])
    y = math.sin(dl) * math.cos(la2)
    x = math.cos(la1) * math.sin(la2) - math.sin(la1) * math.cos(la2) * math.cos(dl)
    return round((math.degrees(math.atan2(y, x)) + 360.0) % 360.0, 1)


def lv95_to_wgs84(e, n):
    """swisstopo's approximate formulas (about 1 m)."""
    y = (e - 2600000.0) / 1e6
    x = (n - 1200000.0) / 1e6
    lon = 2.6779094 + 4.728982 * y + 0.791484 * y * x + 0.1306 * y * x * x - 0.0436 * y ** 3
    lat = (16.9023892 + 3.238272 * x - 0.270978 * y * y - 0.002528 * x * x
           - 0.0447 * y * y * x - 0.0140 * x ** 3)
    return round(lat * 100 / 36, 7), round(lon * 100 / 36, 7)


# --- Tenerife -----------------------------------------------------------------

def tenerife(data):
    base = "https://datos.tenerife.es/ckan"
    folder = os.path.join(data, "traffic-counts", "cabildo-tenerife-imd")
    imd = ckan(base, "intensidad-media-diaria-imd-de-trafico-de-tenerife", folder)
    sta = ckan(base, "estaciones-de-aforo-por-tramos-de-la-red-de-carreteras-de-tenerife", folder)
    for p in (imd, sta):
        expect(p["name"], p.get("license_id"), {"cc-by"})
    year = max(int(m.group(1)) for r in imd["resources"]
               if r.get("format") == "CSV" and (m := re.search(r"(20\d\d)", r["name"])))
    imd_url = next(r["url"] for r in imd["resources"]
                   if r.get("format") == "CSV" and str(year) in r["name"])
    sta_url = next(r["url"] for r in sta["resources"] if r.get("format") == "CSV")
    imd_csv = fetch(imd_url, os.path.join(folder, os.path.basename(imd_url)))
    sta_csv = fetch(sta_url, os.path.join(folder, os.path.basename(sta_url)))
    with open(sta_csv, encoding="utf-8-sig") as f:
        stations = {r["estacion_id"]: r for r in csv.DictReader(f)}
    # Direction 1 is "ascendente" (rising kilometre points): its bearing from
    # the neighbouring stations of the same road.
    by_road = {}
    for s in stations.values():
        by_road.setdefault(s["carretera_codigo"], []).append(s)
    for road in by_road.values():
        road.sort(key=lambda s: float(s["estacion_pk"] or 0))
        pos = [(float(s["estacion_latitud"]), float(s["estacion_longitud"])) for s in road]
        for i, s in enumerate(road):
            if len(road) < 2:
                s["_bearing"] = None
                continue
            a, b = pos[max(i - 1, 0)], pos[min(i + 1, len(road) - 1)]
            s["_bearing"] = bearing(a, b) if a != b else None
    rows = []
    with open(imd_csv, encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            s = stations.get(r["estacion_id"])
            total = float(r["imd_total"] or 0)
            if s is None or total <= 0:
                continue
            rows.append({
                "id": r["estacion_id"], "name": r["estacion_nombre"].strip(),
                "lat": s["estacion_latitud"], "lon": s["estacion_longitud"],
                "ref": r["carretera_codigo"], "aadt": int(total),
                "aadt_dir1": r["imd_ascendentes"] or None,
                "aadt_dir2": r["imd_descendentes"] or None,
                "dir1_bearing_deg": s["_bearing"],
            })
    org = imd.get("organization", {}).get("title", "Cabildo de Tenerife")
    write_counts(os.path.join(data, "traffic-counts", "cabildo-tenerife-imd.counts.csv"), {
        "source": "cabildo-tenerife-imd",
        "title": f"Cabildo de Tenerife – Intensidad Media Diaria (IMD) {year}",
        "licence": "CC-BY",
        "notice": f"Fuente: {org}, Servicio Técnico de Carreteras y Movilidad – «Intensidad Media "
                  f"Diaria (IMD) de tráfico de Tenerife» {year} y «Estaciones de aforo por tramos "
                  "de la red de carreteras de Tenerife» (datos.tenerife.es), Creative Commons "
                  "Attribution (CC BY).",
        "url": f"{base}/dataset/intensidad-media-diaria-imd-de-trafico-de-tenerife",
        "year": year,
    }, rows)


# --- Liechtenstein ------------------------------------------------------------

LLV = "https://www.llv.li/serviceportal2/amtsstellen/amt-fuer-hochbau-und-raumplanung"


def simple_name(s):
    """Letters and digits only: "Eschen – Nendeln" is "Eschen - Nendeln"."""
    return re.sub(r"[\W_]+", "", s.lower())


def num(s):
    return int(s.replace("’", "").replace("'", ""))


def liechtenstein(data):
    folder = os.path.join(data, "traffic-counts", "li-miv")
    p = ckan("https://ckan.opendata.swiss", "verkehrsdaten-fahrrad-und-miv", folder)
    rights = {r.get("rights") for r in p["resources"]}
    expect("verkehrsdaten-fahrrad-und-miv", rights, [{"https://opendata.swiss/terms-of-use#terms_by"}])
    zurl = next(r["url"] for r in p["resources"] if r.get("format") == "ZIP")
    zpath = fetch(zurl, os.path.join(folder, os.path.basename(zurl)))
    year = 2025
    pdf = fetch(f"{LLV}/miv-jahresauswertung-{year}-detail-.pdf",
                os.path.join(folder, f"miv-jahresauswertung-{year}-detail-.pdf"))
    # Station positions (LV95) from the geodatabase.
    try:
        import pyogrio.raw  # noqa: PLC0415
    except ImportError:
        sys.exit("Liechtenstein needs the Python package pyogrio (pip install pyogrio)")
    gdb_dir = os.path.join(folder, "gdb")
    if not os.path.isdir(gdb_dir):
        with zipfile.ZipFile(zpath) as z:
            for n in z.namelist():
                if ".." in n or n.startswith("/"):
                    sys.exit(f"unsafe name in {zpath}: {n}")
            z.extractall(gdb_dir)
    gdb = next(os.path.join(gdb_dir, d) for d in os.listdir(gdb_dir) if d.endswith(".gdb"))
    meta, _fids, geom, fields = pyogrio.raw.read(gdb, layer="verkehrszaehlstellen_li_miv")
    names = list(meta["fields"])
    pos = {}
    for i, g in enumerate(geom):
        order = "<" if g[0] == 1 else ">"
        e, n = struct.unpack(order + "dd", bytes(g[5:21]))
        rec = {k: fields[j][i] for j, k in enumerate(names)}
        key = simple_name(str(rec["name"]))
        pos.setdefault(key, []).append((str(rec["nummer"]), lv95_to_wgs84(e, n)))
    # DTV per station from the yearly report (a page per station).
    text = subprocess.run(["pdftotext", "-layout", pdf, "-"], check=True,
                          capture_output=True, text=True).stdout
    rows, missing = [], []
    for page in text.split("\f"):
        m = re.search(r"Zählstelle:\s+(.+?)\s*$", page, re.M)
        d = None
        for line in page.splitlines():
            if line.strip().startswith("DWV"):
                break
            d = d or re.search(r"\bDTV\s+([\d’']+)\s+([\d’']+)\s+([\d’']+)\s*$", line)
        if not m or not d:
            continue
        name = re.sub(r"\s+", " ", m.group(1)).strip()
        hits = pos.get(simple_name(name))
        if not hits:
            missing.append(name)
            continue
        for nummer, (lat, lon) in hits:
            rows.append({"id": nummer, "name": name, "lat": lat, "lon": lon, "ref": None,
                         "aadt": num(d.group(1)), "aadt_dir1": num(d.group(2)),
                         "aadt_dir2": num(d.group(3)), "dir1_bearing_deg": None})
    if missing:
        print(f"  report stations without a position: {', '.join(missing)}")
    write_counts(os.path.join(data, "traffic-counts", "li-miv.counts.csv"), {
        "source": "li-miv",
        "title": f"Liechtenstein – Verkehrsdaten {year}, permanente Messstellen MIV",
        "licence": "opendata.swiss terms_by (open use, source must be named)",
        "notice": f"Quelle: Amt für Tiefbau und Geoinformation und Amt für Hochbau und "
                  f"Raumplanung, Fürstentum Liechtenstein – Verkehrsdaten Fahrrad und MIV "
                  f"(Zählstellen, opendata.swiss) und Verkehrsdaten {year}, permanente "
                  "Messstellen MIV (llv.li).",
        "url": "https://opendata.swiss/de/dataset/verkehrsdaten-fahrrad-und-miv",
        "year": year,
    }, rows)


def astra(data):
    folder = os.path.join(data, "traffic-counts", "astra-svz")
    p = ckan("https://ckan.opendata.swiss", "strassenverkehrszahlung-ubergeordnetes-netz", folder)
    rights = {r.get("rights") for r in p["resources"]}
    expect("strassenverkehrszahlung-ubergeordnetes-netz", rights,
           [{"https://opendata.swiss/terms-of-use#terms_open"}])
    stac = fetch("https://data.geo.admin.ch/api/stac/v1/collections/"
                 "ch.astra.strassenverkehrszaehlung-uebergeordnet/items",
                 os.path.join(folder, "stac-items.json"))
    with open(stac, encoding="utf-8") as f:
        items = json.load(f)["features"]
    href = next(a["href"] for it in items for a in it["assets"].values()
                if a["href"].endswith(".xtf.zip"))
    zpath = fetch(href, os.path.join(folder, os.path.basename(href)))
    with zipfile.ZipFile(zpath) as z:
        xtf = next(n for n in z.namelist() if n.endswith(".xtf"))
        root = ET.fromstring(z.read(xtf))
    ns = {"i": "http://www.interlis.ch/INTERLIS2.3"}
    pre = "RoadTrafficCensus_V1_1.RoadTrafficCensus."
    locs = {}
    for el in root.iter("{%s}%sMeasurementLocation" % (ns["i"], pre)):
        t = lambda k: (el.findtext("i:" + k, default="", namespaces=ns) or "").strip()  # noqa: E731
        c = el.find(".//i:COORD", ns)
        e, n = float(c.findtext("i:C1", namespaces=ns)), float(c.findtext("i:C2", namespaces=ns))
        locs[el.get("TID")] = {"nr": t("MLocNr"), "name": t("MLocName"),
                               "ref": t("StreetDesignation"), "pos": lv95_to_wgs84(e, n)}
    best = {}
    for el in root.iter("{%s}%sIndicator" % (ns["i"], pre)):
        d = el.find(".//i:Reference", ns).get("REF")
        if d != "ch.astra.roadtrafficcensus.300":  # both directions
            continue
        loc = el.find("i:rMeasurementLocation", ns).get("REF")
        year = int(el.findtext("i:Year", namespaces=ns))
        dtv = int(el.findtext("i:DTV", namespaces=ns))
        if loc in locs and dtv > 0 and (loc not in best or best[loc][0] < year):
            best[loc] = (year, dtv)
    rows = []
    for loc, (year, dtv) in sorted(best.items(), key=lambda x: int(locs[x[0]]["nr"] or 0)):
        l = locs[loc]
        rows.append({"id": f"CH:{int(l['nr']):04d}", "name": l["name"], "lat": l["pos"][0],
                     "lon": l["pos"][1], "ref": l["ref"] or None, "aadt": dtv})
    year = max(y for y, _ in best.values())
    write_counts(os.path.join(data, "traffic-counts", "astra-svz.counts.csv"), {
        "source": "astra-svz",
        "title": f"FEDRO / ASTRA – Swiss automatic road traffic counts, principal network {year}",
        "licence": "opendata.swiss terms_open (open use)",
        "notice": "Source: Federal Roads Office FEDRO (Bundesamt für Strassen ASTRA) – "
                  f"Strassenverkehrszählung übergeordnetes Netz, DTV {year} (data.geo.admin.ch).",
        "url": "https://opendata.swiss/de/dataset/strassenverkehrszahlung-ubergeordnetes-netz",
        "year": year,
    }, rows)


# --- Baden-Württemberg -------------------------------------------------------

def blackforest(data):
    folder = os.path.join(data, "traffic-counts", "bw-svz")
    p = ckan("https://mobidata-bw.de", "karte_strassenverkehrszaehlung", folder)
    expect("karte_strassenverkehrszaehlung", p.get("license_id"),
           {"http://dcat-ap.de/def/licenses/dl-by-de/2.0"})
    url = next(r["url"] for r in p["resources"] if r.get("format") == "CSV")
    path = fetch(url, os.path.join(folder, os.path.basename(url)))
    with open(path, encoding="utf-8-sig") as f:
        raw = f.read()
    col = next((c for c in raw.splitlines()[0].split(",") if c.startswith("DTV20")), None)
    year = int(col[3:]) if col else None
    rows = []
    for r in csv.DictReader(io.StringIO(raw)):
        try:
            aadt = int(float(r[col]))
        except (TypeError, ValueError):
            continue
        if aadt <= 0:
            continue
        ref = f"{r['klasse']} {r['nummer']}".strip() if r.get("klasse") in ("A", "B", "L", "K") else None
        rows.append({"id": r["svznr"], "name": f"{r['RI']} / {r['RII']}".strip(" /"),
                     "lat": r["gpsy1"], "lon": r["gpsx1"], "ref": ref, "aadt": aadt})
    org = (p.get("organization") or {}).get("title", "Verkehrsministerium Baden-Württemberg")
    write_counts(os.path.join(data, "traffic-counts", "bw-svz.counts.csv"), {
        "source": "bw-svz",
        "title": f"Baden-Württemberg – Straßenverkehrszählung (SVZ), DTV {year}",
        "licence": "DL-DE-BY-2.0",
        "notice": f"Datenquelle: {org.replace(' - ', '-')} / MobiData BW® – Karte der "
                  f"Straßenverkehrszählung in Baden-Württemberg (SVZ-Zählstellen, DTV {year}), "
                  "Datenlizenz Deutschland – Namensnennung – Version 2.0 "
                  "(https://www.govdata.de/dl-de/by-2-0).",
        "url": "https://mobidata-bw.de/dataset/karte_strassenverkehrszaehlung",
        "year": year,
    }, rows)


WORLDS = {
    "tenerife": [tenerife],
    "liechtenstein": [liechtenstein, astra],
    "blackforest": [blackforest],
    "andorra": [],
}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in WORLDS:
        sys.exit(__doc__)
    world, data = sys.argv[1], sys.argv[2]
    if not WORLDS[world]:
        print(f"{world}: no open counting-station data (18D) – nothing to fetch")
    for f in WORLDS[world]:
        print(f"{world}: {f.__name__}")
        f(data)


if __name__ == "__main__":
    main()
