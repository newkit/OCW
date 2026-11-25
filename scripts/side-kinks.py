#!/usr/bin/env python3
"""The side.kink findings of a `side_dump` file (spec 4.8.8 Rule, J9 working
tool): where each lies – at a mouth corner (within 0.5 m of a chain's end:
where the corner's band turns into an arm's strip edge) or along a chain –
and the grades of the two arms of that chain.
Usage: side-kinks.py DUMP.jsonl"""
import json
import math
import sys
from collections import Counter

groups = Counter()
vals = {}
for line in open(sys.argv[1]):
    r = json.loads(line)
    fs = [f for f in r["faults"] if f["kind"] == "kink"]
    if not fs:
        continue
    at = fs[0]["at"]
    best = None
    for ch in r["chains"]:
        for k, p in enumerate(ch["points"]):
            d = math.dist([p[0], p[2]], at)
            if best is None or d < best[0]:
                best = (d, ch, k)
    _, ch, k = best
    n = len(ch["points"])
    end = k == 0 or k == n - 1 or min(
        math.dist([ch["points"][0][0], ch["points"][0][2]], at),
        math.dist([ch["points"][-1][0], ch["points"][-1][2]], at)) < 0.5
    a, b = r["arms"][ch["from"]], r["arms"][ch["to"]]
    steep = max(abs(a["grade"]), abs(b["grade"]))
    dg = abs(a["grade"] - b["grade"])
    g = ("mouth corner" if end else "along chain") + (
        ", arm > 6 %" if steep > 0.06 else ", arms <= 6 %") + (
        ", grades differ > 6 pts" if dg > 0.06 else "")
    groups[g] += 1
    vals.setdefault(g, []).append((r["vertex"], round(fs[0]["value"], 1)))
for g, n in groups.most_common():
    print(f"{n:4d}  {g}  {vals[g][:5]}")
# The arm whose strip edge meets the chain at a mouth-corner kink: warped
# (a cross slope turning flat past the mouth), blended, or plain.
arms = Counter()
for line in open(sys.argv[1]):
    r = json.loads(line)
    fs = [f for f in r["faults"] if f["kind"] == "kink"]
    if not fs:
        continue
    at = fs[0]["at"]
    for a in r["arms"]:
        for c in a["row"]:
            if math.dist([c[0], c[2]], at) < 0.05:
                kind = "blended" if a["blend_m"] > 0 else "warped" if abs(a["cross"]) > 1e-6 else "plain"
                room = a["len"] - a["mouth_m"]
                arms[(kind, "room < 10 m" if room < 10 else "room >= 10 m")] += 1
print("mouth-corner kinks by arm:", dict(arms))
