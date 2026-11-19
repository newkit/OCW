#!/usr/bin/env python3
"""Writes scripts/fixtures/history-laps-map.json: the demo history
(history-demo.json) with laps and a track for its newest ride, for the
training history's scenes (spec 14 Phase 7, 7A: a ride's laps and map).
The track is made up (a line up the valley with switchbacks), not a real
road."""

import json
import math
import os

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    with open(os.path.join(HERE, "history-demo.json")) as f:
        d = json.load(f)
    r = d["rides"][-1]
    pts = []
    n = 160
    for i in range(n + 1):
        t = i / n
        lat = 47.141 + (47.101 - 47.141) * t
        lon = 9.521 + (9.609 - 9.521) * t
        if 0.25 < t < 0.8:
            u = (t - 0.25) / 0.55
            w = math.sin(u * math.pi * 9) * 0.0035 * math.sin(u * math.pi)
            lat += w * 0.6
            lon -= w * 0.8
        pts.append([round(lat, 5), round(lon, 5)])
    r["track"] = pts
    r["laps"] = [
        {"start_s": 0.0, "start_m": 0.0, "elapsed_s": 1210.0, "distance_m": 7800.0,
         "ascent_m": 310.0, "avg_power_w": 196.0, "avg_hr_bpm": 141.0},
        {"start_s": 1210.0, "start_m": 7800.0, "elapsed_s": 1820.0, "distance_m": 8300.0,
         "ascent_m": 620.0, "avg_power_w": 224.0, "avg_hr_bpm": 156.0},
        {"start_s": 3030.0, "start_m": 16100.0, "elapsed_s": 1002.0, "distance_m": 5300.0,
         "ascent_m": 250.0, "avg_power_w": 201.0, "avg_hr_bpm": 150.0},
    ]
    with open(os.path.join(HERE, "history-laps-map.json"), "w") as f:
        json.dump(d, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    main()
