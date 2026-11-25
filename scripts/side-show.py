#!/usr/bin/env python3
"""One junction of a `side_dump` file in numbers (spec 4.8.8 Rule, J9 working
tool): its findings, per arm the grade, mouth and first strip rows' edge
heights, per chain its points (x, y, z) with the grade between them.
Usage: side-show.py DUMP.jsonl VERTEX"""
import json
import math
import sys

want = int(sys.argv[2])
for line in open(sys.argv[1]):
    r = json.loads(line)
    if r["vertex"] != want:
        continue
    print("faults", r["faults"])
    print("members", r.get("members"), "through", r.get("through"))
    for a in r["arms"]:
        print(f"arm {a['i']} edge {a['edge']} {a['class']} len {a['len']:.1f} mouth {a['mouth_m']:.2f} "
              f"grade {a['grade']:+.3f} blend {a['blend_m']:.1f} cross {a['cross']:+.3f} h {a['h']:.2f} "
              f"row {[[round(c, 2) for c in p] for p in a['row']]}")
        for s in a["strips"][:1]:
            rows = s["rows"][:6]
            for i, row in rows:
                print("   row", i, [[round(c, 2) for c in p] for p in row])
    for ch in r["chains"]:
        pts = ch["points"]
        out = []
        for k, p in enumerate(pts):
            g = ""
            if k:
                d = math.dist([p[0], p[2]], [pts[k - 1][0], pts[k - 1][2]])
                g = f"{(p[1] - pts[k - 1][1]) / max(d, 1e-9) * 100:+.0f}%/{d:.2f}"
            out.append(f"({p[0]:.1f},{p[1]:.2f},{p[2]:.1f}){g}")
        print(f"chain {ch['from']}->{ch['to']}:", " ".join(out))
