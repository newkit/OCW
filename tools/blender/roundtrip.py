# SPDX-License-Identifier: MIT OR Apache-2.0
"""The Blender half of the round-trip test (spec 14 Phase 15, 15F).

Run by scripts/blender-roundtrip.py, never by hand inside a session:

    blender -b --factory-startup --python tools/blender/roundtrip.py -- JOBS.json RESULTS.json

JOBS.json is a list of {"src": "in.glb", "out": "out.glb"}. For each job a
fresh empty scene imports `src` with Blender's glTF importer and exports
the whole scene to `out` with the export preset of artwork/3dmodels/README.md
(15D): glTF Binary, +Y up, modifiers applied, custom properties (extras) on,
materials exported, no Draco; vertex colours kept (the plants' shading,
COLOR_0). RESULTS.json gets one {"src", "out", "ok", "error", "seconds"}
per job, written after every job so a crash leaves the ones before.

This script only drives Blender (`bpy`); it is a test tool, not shipped and
not an add-on (Q5 keeps the add-on for later).
"""

import json
import sys
import time
import traceback

import bpy

# The export preset of artwork/3dmodels/README.md. Names that this Blender's
# exporter does not know are dropped (the exporter's options change between
# releases), and the dropped ones are reported once.
EXPORT_PRESET = {
    "export_format": "GLB",
    "use_selection": False,  # the preset says "selected objects"; here: all of them
    "export_yup": True,
    "export_apply": True,
    "export_extras": True,
    "export_materials": "EXPORT",
    "export_draco_mesh_compression_enable": False,
    "export_texcoords": True,
    "export_normals": True,
    "export_vertex_color": "ACTIVE",  # Blender 4.2+: keep COLOR_0 even without a material using it
    "export_all_vertex_colors": False,
    "export_attributes": False,
    "export_cameras": False,
    "export_lights": False,
    "export_animations": False,
    "export_skins": False,
    "export_morph": False,
}

IMPORT_PRESET = {
    "merge_vertices": False,
    "import_shading": "NORMALS",
    "guess_original_bind_pose": False,
}


def supported(op, wanted):
    """The options of `wanted` that `op` has, and the names it lacks."""
    props = {p.identifier for p in op.get_rna_type().properties}
    return ({k: v for k, v in wanted.items() if k in props}, sorted(set(wanted) - props))


def main(argv):
    jobs_path, results_path = argv
    with open(jobs_path, encoding="utf-8") as f:
        jobs = json.load(f)
    imp, imp_missing = supported(bpy.ops.import_scene.gltf, IMPORT_PRESET)
    exp, exp_missing = supported(bpy.ops.export_scene.gltf, EXPORT_PRESET)
    print(f"ocw-roundtrip: Blender {bpy.app.version_string}, {len(jobs)} files")
    if imp_missing or exp_missing:
        print(f"ocw-roundtrip: options this Blender lacks: import {imp_missing}, export {exp_missing}")
    results = []
    for job in jobs:
        t0 = time.monotonic()
        r = {"src": job["src"], "out": job["out"], "ok": False, "error": None}
        try:
            bpy.ops.wm.read_factory_settings(use_empty=True)
            res = bpy.ops.import_scene.gltf(filepath=job["src"], **imp)
            if "FINISHED" not in res:
                raise RuntimeError(f"import: {res}")
            res = bpy.ops.export_scene.gltf(filepath=job["out"], **exp)
            if "FINISHED" not in res:
                raise RuntimeError(f"export: {res}")
            r["ok"] = True
        except Exception as e:  # noqa: BLE001 – every failure is a result
            r["error"] = f"{type(e).__name__}: {e}"
            traceback.print_exc()
        r["seconds"] = round(time.monotonic() - t0, 3)
        results.append(r)
        with open(results_path, "w", encoding="utf-8") as f:
            json.dump({"blender": bpy.app.version_string, "results": results}, f, indent=1)


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(argv) != 2:
        sys.exit("usage: blender -b --factory-startup --python tools/blender/roundtrip.py -- JOBS.json RESULTS.json")
    main(argv)
