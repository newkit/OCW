#!/usr/bin/env python3
"""The Blender round trip of the model files (spec 14 Phase 15, 15F; `make test-blender`).

For every file of artwork/3dmodels/manifest.toml (or the ones named with
--only): Blender imports the `.glb` and exports it again with the export
preset of artwork/3dmodels/README.md (tools/blender/roundtrip.py, one
`blender -b` for all files), then `ocw-assets model import` takes the
export back under the file's own manifest entry into a scratch model
folder, and `ocw-assets model diff --ignore-manifest` compares it with the
committed file: it must report no structural change.

Skipped with a notice (exit 0) where Blender is missing (`make deps-blender`
installs it on marge); --require turns the skip into a failure.

Writes OUT/<date>-<time>/summary.md (the table of files), roundtrip.log (every
command's output) and results.json, and adds a line to test-output/index.md.
Exit 1 when any file fails.
"""

from __future__ import annotations

import argparse
import datetime as dt
import fnmatch
import json
import os
import shutil
import subprocess
import sys
import time
import tomllib
from dataclasses import asdict, dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "artwork" / "3dmodels"
BLENDER_SCRIPT = ROOT / "tools" / "blender" / "roundtrip.py"
# Validator codes for what Blender cannot keep (ocw_assets::check).
BLENDER_LOSSES = {"mesh.color-above-one"}


@dataclass
class FileResult:
    file: str
    stage: str = "ok"  # ok | blender | import | diff
    detail: list[str] = field(default_factory=list)
    # The validator's findings that explain a diff (BLENDER_LOSSES).
    cause: list[str] = field(default_factory=list)
    seconds: float = 0.0

    @property
    def ok(self) -> bool:
        return self.stage == "ok"


def find_blender(arg: str | None) -> str | None:
    """--blender, $BLENDER, `blender` on PATH, then ~/.local/bin/blender."""
    for c in (arg, os.environ.get("BLENDER")):
        if c:
            return c if shutil.which(c) else None
    found = shutil.which("blender")
    if found:
        return found
    local = Path.home() / ".local" / "bin" / "blender"
    return str(local) if os.access(local, os.X_OK) else None


def entries(manifest: Path, only: list[str]) -> list[dict]:
    with open(manifest, "rb") as f:
        assets = tomllib.load(f).get("asset", [])
    if only:
        assets = [
            a
            for a in assets
            if any(fnmatch.fnmatch(a["file"], p) or fnmatch.fnmatch(a["file"], p + ".glb") for p in only)
        ]
    return assets


def import_args(entry: dict) -> list[str]:
    """`model import` options that give back the file's own manifest entry."""
    file = entry["file"]
    args = ["--kind", entry["kind"], "--id", file.removesuffix(".glb")]
    for key in ("license", "source", "author", "profile", "species", "variant", "anchor"):
        if entry.get(key):
            args += [f"--{key}", entry[key]]
    return args


def diff_lines(stdout: str) -> list[str]:
    return [l for l in stdout.splitlines() if l.strip() and l.strip() != "no structural change"]


def summarize(results: list[FileResult], blender: str, elapsed: float) -> str:
    ok = [r for r in results if r.ok]
    bad = [r for r in results if not r.ok]
    lines = [
        "# Blender round trip (spec 15F)",
        "",
        f"{blender}: {len(results)} model files, **{len(ok)} without a structural change**, "
        f"{len(bad)} failed ({elapsed:.0f} s).",
        "",
        "Blender imports each `.glb`, exports it with the preset of `artwork/3dmodels/README.md`; "
        "`ocw-assets model import` takes it back under its manifest entry and "
        "`model diff --ignore-manifest` compares it with the committed file.",
        "",
    ]
    if bad:
        by_stage: dict[str, int] = {}
        for r in bad:
            by_stage[r.stage] = by_stage.get(r.stage, 0) + 1
        lines += ["Failures by stage: " + ", ".join(f"{k} {v}" for k, v in sorted(by_stage.items())), ""]
        explained = sum(bool(r.cause) for r in bad)
        if explained:
            lines += [f"{explained} of the failures have vertex colours above 1, which the validator "
                      "names (`mesh.color-above-one`): Blender keeps vertex colours as 8 bits, clamped to 1.", ""]
        lines += ["`model diff` says what alone moved in a mesh at the same triangles: "
                  "`normals`, `colours`, or `positions` (the shape itself).", ""]
        lines += ["| File | Stage | Known cause | What differs |", "|---|---|---|---|"]
        for r in bad:
            what = "; ".join(r.detail[:6]).replace("|", "\\|")
            if len(r.detail) > 6:
                what += f"; … ({len(r.detail)} lines)"
            cause = ", ".join(r.cause) or "–"
            lines.append(f"| `{r.file}` | {r.stage} | {cause} | {what} |")
        lines.append("")
    lines += ["| Folder | Files | Unchanged |", "|---|---|---|"]
    folders: dict[str, list[FileResult]] = {}
    for r in results:
        folders.setdefault(r.file.split("/")[0], []).append(r)
    for k, rs in sorted(folders.items()):
        lines.append(f"| `{k}/` | {len(rs)} | {sum(r.ok for r in rs)} |")
    return "\n".join(lines) + "\n"


def run(cmd: list[str], log, timeout: float | None = None) -> subprocess.CompletedProcess:
    log.write("$ " + " ".join(cmd) + "\n")
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    log.write(p.stdout)
    log.write(p.stderr)
    log.write(f"[exit {p.returncode}]\n")
    log.flush()
    return p


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--blender", help="the Blender binary (default: $BLENDER, PATH, ~/.local/bin/blender)")
    ap.add_argument("--require", action="store_true", help="fail instead of skipping when Blender is missing")
    ap.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
    ap.add_argument("--ocw-assets", help="a built ocw-assets binary (default: cargo build -p ocw-assets)")
    ap.add_argument("--only", nargs="*", default=[], help="file patterns of the manifest (bikes/*, wear/helmet-*)")
    ap.add_argument("--out", default=str(ROOT / "test-output" / "blender-roundtrip"))
    ap.add_argument("--work", default=str(ROOT / "target" / "blender-roundtrip"))
    ap.add_argument("--index", default=str(ROOT / "test-output" / "index.md"))
    args = ap.parse_args()

    blender = find_blender(args.blender)
    if not blender:
        msg = "test-blender: skipped – Blender is not installed (`make deps-blender` installs the pinned release on Linux x64; marge has it)"
        print(msg, file=sys.stderr)
        return 1 if args.require else 0

    tool = args.ocw_assets
    if not tool:
        subprocess.run([args.cargo, "build", "-q", "-p", "ocw-assets", "--bin", "ocw-assets"], cwd=ROOT, check=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        tool = str(target / "debug" / "ocw-assets")

    assets = entries(MODELS / "manifest.toml", args.only)
    if not assets:
        print("test-blender: no model file matches", args.only, file=sys.stderr)
        return 2

    now = dt.datetime.now()
    out = Path(args.out) / now.strftime("%Y-%m-%d-%H%M%S")
    out.mkdir(parents=True, exist_ok=True)
    work = Path(args.work)
    shutil.rmtree(work, ignore_errors=True)
    (work / "blender").mkdir(parents=True)
    # The scratch model folder starts as a copy of the committed one, so the
    # import replaces each file as it would in artwork/3dmodels (and keeps
    # what Blender drops from the file it replaces: asset extras, copyright).
    shutil.copytree(MODELS, work / "models", ignore=shutil.ignore_patterns("sources", "*.md"))
    t0 = time.monotonic()

    with open(out / "roundtrip.log", "w", encoding="utf-8") as log:
        version = run([blender, "-b", "--factory-startup", "--version"], log).stdout.splitlines()
        version = version[0] if version else blender
        print(f"test-blender: {version}, {len(assets)} model files")

        jobs = []
        for a in assets:
            dst = work / "blender" / a["file"]
            dst.parent.mkdir(parents=True, exist_ok=True)
            jobs.append({"src": str(MODELS / a["file"]), "out": str(dst)})
        (work / "jobs.json").write_text(json.dumps(jobs, indent=1))
        bres = work / "blender-results.json"
        run(
            [blender, "-b", "--factory-startup", "--python-exit-code", "1", "--python", str(BLENDER_SCRIPT),
             "--", str(work / "jobs.json"), str(bres)],
            log,
            timeout=1800,
        )
        done = {}
        if bres.is_file():
            done = {r["src"]: r for r in json.loads(bres.read_text())["results"]}

        # The validator's findings on what Blender cannot keep: the known
        # cause of a failure.
        causes: dict[str, list[str]] = {}
        p = run([tool, "validate", str(MODELS), "--json"], log)
        for line in p.stdout.splitlines():
            try:
                f = json.loads(line)
            except ValueError:
                continue
            if f.get("code") in BLENDER_LOSSES:
                node = (f.get("target") or {}).get("name") or "file"
                causes.setdefault(f["file"], []).append(f"{f['code']} ({node})")

        results = []
        for a, job in zip(assets, jobs):
            r = FileResult(a["file"])
            b = done.get(job["src"])
            if not b or not b["ok"]:
                r.stage = "blender"
                r.detail = [b["error"] if b else "not exported (Blender stopped before it; see roundtrip.log)"]
                results.append(r)
                continue
            r.seconds = b["seconds"]
            p = run([tool, "model", "import", job["out"], *import_args(a), "--out", str(work / "models")], log)
            if p.returncode != 0:
                r.stage = "import"
                r.detail = [l for l in (p.stderr + p.stdout).splitlines() if l.strip()]
                results.append(r)
                continue
            p = run([tool, "model", "diff", "--ignore-manifest", str(MODELS / a["file"]),
                     str(work / "models" / a["file"])], log)
            if p.returncode != 0:
                r.stage = "diff"
                r.detail = diff_lines(p.stdout) + [l for l in p.stderr.splitlines() if l.strip()]
                r.cause = causes.get(a["file"], [])
            results.append(r)
            print(f"  {'ok  ' if r.ok else 'FAIL'} {a['file']}" + ("" if r.ok else f" ({r.stage})"))

    summary = summarize(results, version, time.monotonic() - t0)
    (out / "summary.md").write_text(summary)
    (out / "results.json").write_text(json.dumps([asdict(r) for r in results], indent=1))
    print()
    print(summary)
    bad = sum(not r.ok for r in results)
    index = Path(args.index)
    try:
        rel = out.relative_to(index.parent)
        if not index.exists():
            index.write_text("# OCW test output\n\n| Date | Time | Commit | Mode | Window system | App arguments | Files |\n|---|---|---|---|---|---|---|\n")
        commit = os.environ.get("OCW_COMMIT") or os.environ.get("OCW_BUILD_ID") or subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip() or "-"
        with open(index, "a", encoding="utf-8") as f:
            f.write(f"| {now:%Y-%m-%d} | {now:%H:%M:%S} | {commit} | blender-roundtrip | - | "
                    f"{len(results) - bad}/{len(results)} files unchanged ({version}) | "
                    f"[summary.md]({rel}/summary.md) [roundtrip.log]({rel}/roundtrip.log) |\n")
    except ValueError:
        pass
    print(f"test-blender: {out}/summary.md")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
