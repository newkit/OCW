#!/usr/bin/env python3
"""Classify side_dump findings by cause (J7 working tool).
Usage: classify.py DUMP.jsonl"""
import json
import math
import sys
from collections import Counter

PATHS = {"Path", "Cycleway"}
cnt = Counter()
examples = {}
for line in open(sys.argv[1]):
    r = json.loads(line)
    arms = r["arms"]
    kinds = {f["kind"] for f in r["faults"]}
    if not kinds:
        continue
    # (J8: way splits an arm runs on past are no members of a cluster.)
    members = len(r.get("members") or []) - len(r.get("run_on") or [])
    path_arms = [a for a in arms if a["class"] in PATHS]
    # A path arm nearly parallel to another arm (within 30 deg).
    par = False
    for a in path_arms:
        for b in arms:
            if b is a:
                continue
            da = math.atan2(a["dir"][1], a["dir"][0])
            db = math.atan2(b["dir"][1], b["dir"][0])
            d = abs((da - db + math.pi) % (2 * math.pi) - math.pi)
            if d < math.radians(30) or abs(d - math.pi) < math.radians(30) and False:
                par = True
    short = any(a["mouth_m"] < 0.45 * a["len"] - 0.3 and a["len"] < 25 for a in arms) or any(
        a["len"] < 20 for a in arms)
    if members > 1 and path_arms:
        cause = "1 cluster with paths"
    elif members > 1:
        cause = "1b cluster without paths"
    elif par:
        cause = "4 fork with a path"
    elif short:
        cause = "2 short arms"
    else:
        cause = "other"
    for k in kinds:
        cnt[(cause, k)] += 1
    examples.setdefault(cause, r["vertex"])
for k in sorted(cnt):
    print(k, cnt[k])
print(examples)
