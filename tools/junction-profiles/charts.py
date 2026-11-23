#!/usr/bin/env python3
"""Slope profile charts of side roads at junctions, one package against
another (spec 4.8.8 Q6 (d) / Q7).

Input: the JSON lines of `cargo run -p ocw-worldgen --release --example
junction_profiles -- A.ocwpkg B.ocwpkg OUT.jsonl` (B is the "j3" field, A
the "live" field). More such files can be given: the first for the live
package, one for a J2b build (its "live" field is the J2b package) and one
for a J4 build (spec 4.8.8 Q7; made from the live package and the J4
package: its "j3" field is the J4 package) and one for a J5 build (spec
4.8.8 Rule; likewise, its "j3" field is the J5 package).

    charts.py --live LIVE.jsonl [--j2b J2B.jsonl] [--j4 J4.jsonl] [--j5 J5.jsonl] --out DIR \
        NAME:VERTEX:EDGE[:METRES][:j2b][:far] ...

Per example a PNG: the side road's height (above the main road's centre)
and its grade over 5 m stretches from the junction's centre, the main road's
edge and the junction surface's end (mouth) marked, kinks (a change of
grade of more than 8 points between two 5 m stretches) highlighted. Needs
matplotlib.
"""
import argparse
import json
import os

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

KINK = 8.0
STYLE = {
    # Fixed categorical order (blue, orange, aqua) plus a line style each,
    # so the series never depend on colour alone.
    "live": dict(color="#2a78d6", ls=(0, (6, 3)), lw=2.0, label="live package (as main draws it)"),
    "j2b": dict(color="#eb6834", ls=(0, (1.5, 2)), lw=2.2, label="J2b build (before Q6 (d), never installed)"),
    "j3": dict(color="#1baf7a", ls="-", lw=2.0, label="J3 package (Q6 (d))"),
    "j4": dict(color="#c2185b", ls=(0, (5, 2, 1, 2)), lw=2.4, label="J4 package (Q7: creases rounded, blended planes)"),
    "j5": dict(color="#5b3fb3", ls="-", lw=2.6, label="J5 package (Rule: the sides as smooth as the road)"),
}
INK, INK2, GRID, SURF = "#0b0b0b", "#52514e", "#e4e3df", "#fcfcfb"
BAND = "#fab219"


def load(path):
    out = {}
    with open(path) as f:
        for line in f:
            r = json.loads(line)
            out[(r["vertex"], r["edge"])] = r
    return out


def stretches(h, n):
    """Grades (%) of the 5 m stretches from 0 over the first n metres."""
    g = []
    for i in range(0, min(n, len(h) - 1) - 4, 5):
        g.append((i, (h[i + 5] - h[i]) / 5 * 100))
    return g


def kinks(g):
    return [(g[i + 1][0], g[i][1], g[i + 1][1]) for i in range(len(g) - 1) if abs(g[i + 1][1] - g[i][1]) > KINK]


def chart(name, rec, series, metres, out, title):
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 7.2), sharex=True, gridspec_kw=dict(height_ratios=[1.1, 1]))
    fig.patch.set_facecolor(SURF)
    base = rec["j3"]["drawn"][0]
    lines = []
    all_kinks = {}
    for key, h in sorted(series, key=lambda kh: kh[0] not in ("j3", "j4", "j5")):
        h = h[: metres + 1]
        st = STYLE[key]
        x = list(range(len(h)))
        label = st["label"]
        if key == "live" and all(abs(p - q) < 0.005 for p, q in zip(h, rec["j3"]["drawn"])):
            label += " - the same as J3"
        (ln,) = ax1.plot(x, [v - base for v in h], color=st["color"], ls=st["ls"], lw=st["lw"], label=label)
        lines.append(ln)
        g = stretches(h, metres)
        xs, ys = [], []
        for s0, gr in g:
            xs += [s0, s0 + 5]
            ys += [gr, gr]
        ax2.plot(xs, ys, color=st["color"], ls=st["ls"], lw=st["lw"])
        k = kinks(g)
        all_kinks[key] = k
        for s, a, b in k:
            ax2.plot([s, s], [a, b], color=st["color"], lw=1.2)
            ax2.plot([s], [b], "o", ms=9, mfc=SURF, mec=st["color"], mew=2)
    # The newest package's kinks: shaded and labelled.
    newest = "j5" if "j5" in all_kinks else "j4" if "j4" in all_kinks else "j3"
    for i, (s, a, b) in enumerate(all_kinks.get(newest, [])):
        for ax in (ax1, ax2):
            ax.axvspan(s - 1.2, s + 1.2, color=BAND, alpha=0.28, lw=0)
        ax2.annotate(
            f"kink {abs(b - a):.0f} points\n({a:+.0f} % to {b:+.0f} %)",
            (s, (a + b) / 2), xytext=(40, 26 - 40 * i), textcoords="offset points", fontsize=9, color=INK,
            arrowprops=dict(arrowstyle="-", color=INK2, lw=0.8),
            bbox=dict(boxstyle="round,pad=0.25", fc=SURF, ec="none", alpha=0.9),
        )
    edge = rec["edge_m"]
    marks = []
    if edge > 0:
        marks.append((edge, "main road edge"))
    marks.append((rec["mouth_m"], "junction surface ends"))
    if rec.get("far_edge"):
        marks.append((rec["far_edge"], "next junction:\nmain road edge"))
    for m, label in marks:
        for ax in (ax1, ax2):
            ax.axvline(m, color=INK2, lw=1, ls=(0, (4, 3)) if "surface" in label else "-")
        ax1.annotate(label, (m, 1.0), xycoords=("data", "axes fraction"), xytext=(4, -4), textcoords="offset points",
                     fontsize=9, color=INK2, va="top")
    ax1.set_ylabel("height above the main road's centre (m)", color=INK)
    ax2.set_ylabel("grade over 5 m (%)", color=INK)
    ax2.set_xlabel("distance along the side road from the main road's centre (m)", color=INK)
    ax2.axhline(0, color=INK2, lw=0.8)
    for ax in (ax1, ax2):
        ax.set_facecolor(SURF)
        ax.grid(True, color=GRID, lw=0.8)
        ax.set_axisbelow(True)
        for sp in ("top", "right"):
            ax.spines[sp].set_visible(False)
        for sp in ("left", "bottom"):
            ax.spines[sp].set_color(INK2)
        ax.tick_params(colors=INK2)
    ax2.set_xlim(0, metres)
    fig.suptitle(title, x=0.01, ha="left", fontsize=12.5, color=INK)
    ax1.legend(handles=lines, loc="lower right" if series[-1][1][metres] > series[-1][1][0] else "lower left",
               frameon=False, fontsize=9)
    fig.text(0.01, 0.005, f"shaded: a kink in the {newest.upper()} package (grade changes by more than 8 points between two 5 m stretches); "
             "rings: kinks of each package", fontsize=8.5, color=INK2)
    fig.tight_layout(rect=(0, 0.02, 1, 0.97))
    path = os.path.join(out, f"{name}.png")
    fig.savefig(path, dpi=130, facecolor=SURF)
    plt.close(fig)
    return path, all_kinks


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--live", required=True)
    ap.add_argument("--j2b")
    ap.add_argument("--j4")
    ap.add_argument("--j5")
    ap.add_argument("--out", required=True)
    ap.add_argument("--title", action="append", default=[], help="NAME=TITLE")
    ap.add_argument("examples", nargs="+")
    a = ap.parse_args()
    live = load(a.live)
    j2b = load(a.j2b) if a.j2b else {}
    j4 = load(a.j4) if a.j4 else {}
    j5 = load(a.j5) if a.j5 else {}
    titles = dict(t.split("=", 1) for t in a.title)
    os.makedirs(a.out, exist_ok=True)
    for ex in a.examples:
        parts = ex.split(":")
        name, v, e = parts[0], int(parts[1]), int(parts[2])
        metres = int(parts[3]) if len(parts) > 3 and parts[3] else 60
        rec = live[(v, e)]
        series = [("live", rec["live"]["drawn"]), ("j3", rec["j3"]["drawn"])]
        if "j2b" in parts[4:] and (v, e) in j2b:
            series.insert(1, ("j2b", j2b[(v, e)]["live"]["profile"]))
        if (v, e) in j4:
            series.append(("j4", j4[(v, e)]["j3"]["drawn"]))
        if (v, e) in j5:
            series.append(("j5", j5[(v, e)]["j3"]["drawn"]))
        metres = min(metres, min(len(h) for _, h in series) - 1)
        if "far" in parts[4:]:
            # The other end's edge point (a short road between two junctions).
            far = [r for (vv, ee), r in live.items() if ee == e and vv != v]
            if far and far[0]["edge_m"] > 0:
                rec["far_edge"] = rec["len_m"] - far[0]["edge_m"]
        path, k = chart(name, rec, series, metres, a.out, titles.get(name, name))
        print(path, {s: [(x, round(p, 1), round(q, 1)) for x, p, q in kk] for s, kk in k.items()})


if __name__ == "__main__":
    main()
