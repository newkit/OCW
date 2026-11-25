#!/usr/bin/env python3
"""The side findings at corners squeezed by short arms (spec 4.8.8 Rule, J9
working tool): per finding of a `side_dump` record, the arm nearest to it
whose mouth sits at its share of the edge (45 %) – its length, what lies at
its far end (a junction: its degree, how far its patch reaches, whether
the two would be a cluster; a split; a bridge) – summed by those groups.
Usage: side-short.py DUMP.jsonl [KIND]"""
import json
import math
import sys
from collections import Counter

kind = sys.argv[2] if len(sys.argv) > 2 else "corner"
groups = Counter()
lens = Counter()
examples = {}
for line in open(sys.argv[1]):
    r = json.loads(line)
    fs = [f for f in r["faults"] if f["kind"] == kind]
    if not fs:
        continue
    at = fs[0]["at"]
    capped = [a for a in r["arms"] if a["mouth_m"] >= 0.45 * a["len"] - 0.05]
    if not capped:
        groups["no capped arm"] += 1
        examples.setdefault("no capped arm", (r["vertex"], at))
        continue
    a = min(capped, key=lambda a: math.dist(a["centre"], at))
    far = a["far_deg"]
    if a.get("far_next") and a["far_next"]["bridge"]:
        g = "far: bridge/tunnel"
    elif far == 2:
        nx = a.get("far_next") or {}
        g = "far: split (" + ("class/width change" if a.get("split") is None else "plain") + ")"
    elif far == 1:
        g = "far: dead end"
    elif a["far_cluster"]:
        g = "far: junction, would-be cluster (not made)"
    else:
        g = "far: junction %s" % ("<=12" if a["len"] <= 12 else "12-20" if a["len"] <= 20 else ">20")
    groups[g] += 1
    lens[(g, round(a["len"]))] += 0
    examples.setdefault(g, []).append((r["vertex"], [round(x, 1) for x in at], round(a["len"], 1),
                                      round(a["far_reach"], 1), a["class"]))
for g, n in groups.most_common():
    print(f"{n:4d}  {g}")
for g, ex in examples.items():
    if isinstance(ex, list):
        print(g, ex[:6])
# Why the clusters were not made.
why = Counter()
for line in open(sys.argv[1]):
    r = json.loads(line)
    if any(f["kind"] == kind for f in r["faults"]) and r.get("cluster_failure"):
        why[r["cluster_failure"]] += 1
print("clusters not made:", dict(why))
