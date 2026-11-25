#!/usr/bin/env python3
"""The junctions whose side findings one `side_dump` has and another has not
(spec 4.8.8 Rule, J9 working tool): per junction vertex its centre, the
findings before and after and whether its patch became a cluster or its
mouths moved out – for picking comparison views.
Usage: side-fixed.py BEFORE.jsonl AFTER.jsonl [KIND]"""
import json
import sys

kind = sys.argv[3] if len(sys.argv) > 3 else "corner"


def load(path):
    out = {}
    for line in open(path):
        r = json.loads(line)
        out[r["vertex"]] = r
    return out


a, b = load(sys.argv[1]), load(sys.argv[2])
gone, new = [], []
for v, r in a.items():
    if any(f["kind"] == kind for f in r["faults"]):
        rb = b.get(v)
        # A member of a new cluster: its record is at the cluster's first.
        if rb is None:
            rb = next((x for x in b.values() if v in (x.get("members") or [])), None)
        if rb is None or not any(f["kind"] == kind for f in rb["faults"]):
            how = "no record" if rb is None else (
                f"cluster {rb['members']}" if len(rb.get("members") or []) > 1 else
                "mouths " + ",".join(f"{x['mouth_m']:.1f}" for x in rb["arms"]))
            gone.append((v, [round(c, 1) for c in r["centre"][::2]], how))
for v, r in b.items():
    if any(f["kind"] == kind for f in r["faults"]) and not (
            v in a and any(f["kind"] == kind for f in a[v]["faults"])):
        new.append((v, [round(c, 1) for c in r["centre"][::2]]))
print(f"{kind}: gone {len(gone)}, new {len(new)}")
for g in gone[:40]:
    print("gone", *g)
for n in new[:20]:
    print("new", *n)
