#!/usr/bin/env python3
"""Test copies of the street lamp files with a red `lod1` (spec 16H Q24 (a)).

The shipped props have no `lod1` yet, so the props' switch at 60 m changes
nothing visible in the game. For the screenshots of the switch this writes
copies of the lamp files whose `lod1` is the near model again (the same
meshes, the same node roles) painted in `paint_red`, the glowing head kept
in `bulb`: beyond 60 m a lamp turns red. Never commit its output.

    scripts/lod-demo-lamps.py OUT_DIR [IN_DIR]   (IN_DIR: artwork/3dmodels/props)
"""
import json
import struct
import sys
from pathlib import Path

LAMPS = ["lamp-street-arm-main", "lamp-street-arm-road", "lamp-street-arm-path"]


def read(path):
    b = path.read_bytes()
    assert b[:4] == b"glTF", f"{path}: not a glb (run git lfs pull)"
    n = struct.unpack("<I", b[12:16])[0]
    return json.loads(b[20 : 20 + n]), b[20 + n :]


def write(path, doc, rest):
    t = json.dumps(doc, separators=(",", ":")).encode()
    t += b" " * (-len(t) % 4)
    path.write_bytes(b"glTF" + struct.pack("<III", 2, 20 + len(t) + len(rest), len(t)) + b"JSON" + t + rest)


def with_red_lod1(doc):
    nodes, meshes, mats = doc["nodes"], doc["meshes"], doc["materials"]
    assert not any(n.get("name") == "lod1" for n in nodes), "has a lod1 already"
    mats.append({"name": "paint_red", "pbrMetallicRoughness": {"baseColorFactor": [0.9, 0.05, 0.05, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6}})
    red = len(mats) - 1
    bulb = next(i for i, m in enumerate(mats) if m.get("name") == "bulb")
    root = doc["scenes"][doc.get("scene", 0)]["nodes"][0]
    lod = {"name": "lod1", "children": []}
    nodes.append(lod)
    nodes[root].setdefault("children", []).append(len(nodes) - 1)
    for i in list(nodes[root]["children"]):
        n = nodes[i]
        if "mesh" not in n:
            continue
        m = json.loads(json.dumps(meshes[n["mesh"]]))
        for p in m["primitives"]:
            if p.get("material") != bulb:
                p["material"] = red
        m["name"] = n["name"] + "_lod1"
        meshes.append(m)
        c = {k: v for k, v in n.items() if k in ("translation", "rotation", "scale", "extras")}
        c["name"] = n["name"] + "_lod1"
        c["mesh"] = len(meshes) - 1
        nodes.append(c)
        lod["children"].append(len(nodes) - 1)
    return doc


def main():
    out = Path(sys.argv[1])
    src = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("artwork/3dmodels/props")
    out.mkdir(parents=True, exist_ok=True)
    for name in LAMPS:
        doc, rest = read(src / f"{name}.glb")
        write(out / f"{name}.glb", with_red_lod1(doc), rest)
        print(out / f"{name}.glb")


if __name__ == "__main__":
    main()
