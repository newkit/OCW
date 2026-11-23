#!/usr/bin/env python3
"""Plan view of a junction's drawn sides (spec 4.8.8 Rule): the triangles
dumped by `junction_verges_dump` (crates/ocw-render/tests/junction_seams.rs)
coloured by part – carriageway grey, verge (shoulder) sand, sidewalk light
grey; the junction surface's parts darker.
Usage: verge-plot.py DUMP.csv OUT.png [X,Z [RADIUS]]"""
import csv
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import Polygon  # noqa: E402

dump, out = sys.argv[1], sys.argv[2]
centre = None
radius = 12.0
if len(sys.argv) > 3:
    centre = [float(v) for v in sys.argv[3].split(",")]
if len(sys.argv) > 4:
    radius = float(sys.argv[4])
COL = {
    ("edge", 0): "#8a8580", ("patch", 0): "#6d6760", ("strip", 0): "#8a8580",
    ("edge", 2): "#d8c49c", ("patch", 2): "#b89a62", ("strip", 2): "#d8c49c",
    ("edge", 4): "#c8c8c8", ("patch", 4): "#a0a0a0", ("strip", 4): "#c8c8c8",
}
fig, ax = plt.subplots(figsize=(10, 10))
xs, zs = [], []
with open(dump) as f:
    for row in csv.reader(f):
        owner, kind = row[0], int(row[2])
        if kind not in (0, 2, 4):
            continue
        v = [float(t) for t in row[3:]]
        pts = [(v[0], v[2]), (v[3], v[5]), (v[6], v[8])]
        ax.add_patch(Polygon(pts, closed=True, facecolor=COL[(owner, kind)],
                             edgecolor="#00000030" if kind == 2 else "none",
                             linewidth=0.3, alpha=0.9))
        xs += [p[0] for p in pts]
        zs += [p[1] for p in pts]
if centre:
    ax.set_xlim(centre[0] - radius, centre[0] + radius)
    ax.set_ylim(centre[1] - radius, centre[1] + radius)
elif xs:
    ax.set_xlim(min(xs), max(xs))
    ax.set_ylim(min(zs), max(zs))
ax.set_aspect("equal")
ax.set_facecolor("#9fa676")
ax.set_title(dump.split("/")[-1] + ": carriageway grey, verge sand (junction surface darker)")
fig.savefig(out, dpi=110, bbox_inches="tight")
