#!/usr/bin/env python3
"""The side.corner findings of a `side_dump` file by the corner they lie on
(spec 4.8.8 Rule, J9 working tool): the chain nearest to the finding, the
angle between its two arms, the gap between their mouth corners (a fork's
island nose), whether the patch is a cluster, and the classes.
Usage: side-other.py DUMP.jsonl"""
import json
import math
import sys
from collections import Counter

groups = Counter()
ex = {}
for line in open(sys.argv[1]):
    r = json.loads(line)
    fs = [f for f in r["faults"] if f["kind"] == "corner"]
    if not fs:
        continue
    at = fs[0]["at"]
    best = None
    for ch in r["chains"]:
        d = min(math.dist([p[0], p[2]], at) for p in ch["points"])
        if best is None or d < best[0]:
            best = (d, ch)
    ch = best[1]
    a, b = r["arms"][ch["from"]], r["arms"][ch["to"]]
    ang = math.degrees(abs(math.atan2(a["dir"][0] * b["dir"][1] - a["dir"][1] * b["dir"][0],
                                      a["dir"][0] * b["dir"][0] + a["dir"][1] * b["dir"][1])))
    p0, p1 = ch["points"][0], ch["points"][-1]
    gap = math.dist([p0[0], p0[2]], [p1[0], p1[2]])
    members = len(r.get("members") or []) - len(r.get("run_on") or [])
    kind = "cluster" if members > 1 else "single"
    if ang < 40:
        g = f"{kind} fork <40deg, nose gap {'<1.4' if gap < 1.4 else '>=1.4'}"
    elif ang > 140:
        g = f"{kind} near-straight >140deg"
    else:
        g = f"{kind} corner 40-140deg"
    capped = any(x["mouth_m"] >= 0.45 * x["len"] - 0.05 for x in (a, b))
    g += ", capped" if capped else ""
    groups[g] += 1
    ex.setdefault(g, []).append((r["vertex"], [round(x, 1) for x in at], round(ang), round(gap, 2)))
for g, n in groups.most_common():
    print(f"{n:4d}  {g}  {ex[g][:4]}")
