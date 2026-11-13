#!/usr/bin/env python3
"""Scene-rendering harness: renders the scenes of scripts/scenes.toml
headlessly with the run-ocw skill's drive.sh (screenshots and short videos).

Run it through scripts/render-scenes.sh (picks a Python with tomllib, 3.11+);
`--help` lists the options, the README section "Scene harness" explains the
scene file. Every scene runs with its own empty config and data directories
(OCW_CONFIG_DIR, OCW_DATA_DIR: no user settings, devices, ghosts or saved
rides leak in) and the virtual trainer.
"""

from __future__ import annotations

import argparse
import datetime as dt
import fnmatch
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    sys.exit("render_scenes.py needs Python 3.11+ (tomllib); run scripts/render-scenes.sh")

ROOT = Path(__file__).resolve().parent.parent
DRIVE = ROOT / ".claude/skills/run-ocw/drive.sh"
DEFAULT_SCENES = ROOT / "scripts/scenes.toml"
# --pick copies the chosen renders here (with index.md), ready to be packed
# into the zip of the GitHub release `build-history`.
HISTORY = ROOT / "test-output/build-history"
# The world packages scenes can use (`world = true` is Liechtenstein,
# `world = "blackforest"` the Black Forest), by their path in a checkout
# (scripts/fetch-pilot.sh REGION, then world-builder package).
WORLDS = {
    "liechtenstein": "data/pilot/liechtenstein.ocwpkg",
    "blackforest": "data/blackforest/blackforest.ocwpkg",
    "blackforest-middle": "data/blackforest/blackforest-middle.ocwpkg",
    "blackforest-north": "data/blackforest/blackforest-north.ocwpkg",
}
DEFAULT_WORLD = "liechtenstein"
SCENE_KEYS = {
    "name", "description", "world", "args", "cmds", "at", "shot", "video",
    "warmup", "riders", "max_video_mb", "pick",
}


def world_name(scene: dict) -> str | None:
    """The world package a scene needs: None, or a key of WORLDS."""
    w = scene.get("world")
    if not w:
        return None
    name = DEFAULT_WORLD if w is True else str(w)
    if name not in WORLDS:
        sys.exit(f"scene {scene['name']!r}: unknown world {name!r} (known: {', '.join(WORLDS)})")
    return name


def world_env(name: str) -> str:
    """The environment variable naming a world package: $OCW_WORLD for
    Liechtenstein (as before), $OCW_WORLD_<NAME> for the others."""
    return "OCW_WORLD" if name == DEFAULT_WORLD else f"OCW_WORLD_{name.upper().replace('-', '_')}"


def parse_world_args(values: list[str]) -> dict[str, str]:
    """--world PKG (Liechtenstein) or --world NAME=PKG, repeatable."""
    out = {}
    for v in values:
        name, sep, path = v.partition("=")
        if sep and name in WORLDS:
            out[name] = path
        else:
            out[DEFAULT_WORLD] = v
    return out


def find_world(name: str, explicit: str | None) -> Path | None:
    """A world package: --world, its environment variable, else the first
    candidate that exists: this checkout, the main checkout of a git
    worktree, the main checkout beside a scripts/remote.sh branch checkout,
    the kiosk install (scripts/deploy-test.sh)."""
    for p in [explicit, os.environ.get(world_env(name))]:
        if p:
            path = Path(p).expanduser()
            if not path.is_file():
                sys.exit(f"world package not found: {path}")
            return path.resolve()
    rel = WORLDS[name]
    candidates = [ROOT / rel]
    # The main checkout of a git worktree (agent branches) has the data too.
    try:
        common = subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True, text=True, check=True).stdout.strip()
        candidates.append(Path(common).parent / rel)
    except (OSError, subprocess.CalledProcessError):
        pass
    # A branch checkout made by scripts/remote.sh is REPO-BRANCH beside the
    # main checkout REPO (no .git there).
    for i, ch in enumerate(ROOT.name):
        if ch == "-" and i > 0:
            candidates.append(ROOT.parent / ROOT.name[:i] / rel)
    candidates.append(Path("/var/tmp/ocw") / Path(rel).name)
    return next((c.resolve() for c in candidates if c.is_file()), None)


def load_scenes(path: Path) -> tuple[dict, list[dict]]:
    with open(path, "rb") as f:
        data = tomllib.load(f)
    defaults = data.get("defaults", {})
    scenes = []
    seen = set()
    for raw in data.get("scene", []):
        unknown = set(raw) - SCENE_KEYS
        if unknown:
            sys.exit(f"{path}: scene {raw.get('name')!r}: unknown keys {sorted(unknown)}")
        s = {**defaults, **raw}
        for key in ("args", "cmds", "at"):
            s[key] = list(defaults.get(key, [])) + list(raw.get(key, []))
        if not s.get("name") or not s.get("description"):
            sys.exit(f"{path}: every scene needs a name and a description")
        if s["name"] in seen:
            sys.exit(f"{path}: scene {s['name']!r} twice")
        if "shot" not in raw and "video" not in raw:
            sys.exit(f"{path}: scene {s['name']!r} needs shot = DELAY_S and/or video = SECONDS")
        seen.add(s["name"])
        s["kinds"] = [k for k in ("shot", "video") if k in raw]
        scenes.append(s)
    return defaults, scenes


def expand(text: str, world: Path | None) -> str:
    if world is not None:
        text = text.replace("{world}", str(world)).replace("{world_dir}", str(world.parent))
    return text


def app_args(scene: dict, world: Path | None) -> list[str]:
    args = [expand(a, world) for a in scene["args"]]
    for c in scene["cmds"]:
        args += ["--cmd", expand(c, world)]
    for entry in scene["at"]:
        t, sep, cmd = str(entry).partition("=")
        if not sep:
            sys.exit(f"scene {scene['name']!r}: 'at' entries are \"SECONDS=COMMAND\", got {entry!r}")
        args += ["--at", f"{float(t):g}={expand(cmd.strip(), world)}"]
    return args


def describe(scene: dict, world: Path | None) -> str:
    kinds = []
    if "shot" in scene["kinds"]:
        kinds.append(f"shot@{scene['shot']}s")
    if "video" in scene["kinds"]:
        kinds.append(f"video {scene['video']}s")
    w = world_name(scene)
    needs = f" [{w}]" if w else ""
    return f"{scene['name']:<24} {', '.join(kinds):<22}{needs}  {scene['description']}"


def run_scene(scene: dict, world: Path | None, out: Path, mode: str) -> bool:
    """Renders one scene; true if every output was written."""
    ok = True
    args = app_args(scene, world)
    with tempfile.TemporaryDirectory(prefix=f"ocw-scene-{scene['name']}-") as tmp:
        env = dict(os.environ)
        env["OCW_CONFIG_DIR"] = f"{tmp}/config"
        env["OCW_DATA_DIR"] = f"{tmp}/data"
        env["OCW_WORLD_CACHE"] = f"{tmp}/worlds"
        os.makedirs(env["OCW_CONFIG_DIR"])
        if scene.get("riders"):
            Path(env["OCW_CONFIG_DIR"], "riders.toml").write_text(scene["riders"])
        for kind in scene["kinds"]:
            suffix = "" if mode == "x11" else "-wayland"
            if kind == "shot":
                target = out / f"{scene['name']}.jpg"
                cmd = [str(DRIVE), f"shot{suffix}", str(target), f"{scene['shot']:g}", "--", *args]
            else:
                target = out / f"{scene['name']}.mp4"
                extra = ["--capture-warmup", f"{scene['warmup']:g}"] if "warmup" in scene else []
                cmd = [str(DRIVE), f"video{suffix}", str(target), f"{scene['video']:g}", "--",
                       *extra, *args]
            print(f"== {scene['name']} ({kind}): {shlex.join(cmd[1:])}", flush=True)
            started = time.monotonic()
            r = subprocess.run(cmd, cwd=ROOT, env=env)
            if r.returncode != 0:
                print(f"!! {scene['name']} ({kind}) failed (exit {r.returncode})", flush=True)
                ok = False
                continue
            print(f"   done in {time.monotonic() - started:.0f} s", flush=True)
    return ok


def select(scenes: list[dict], only: list[str]) -> list[dict]:
    if not only:
        return scenes
    pats = [p for o in only for p in o.split(",") if p]
    chosen = [s for s in scenes if any(fnmatch.fnmatch(s["name"], p) for p in pats)]
    for p in pats:
        if not any(fnmatch.fnmatch(s["name"], p) for s in scenes):
            sys.exit(f"no scene matches {p!r} (--list shows them)")
    return chosen


def remote(args: argparse.Namespace, passthrough: list[str]) -> int:
    """Runs the harness on a build machine via scripts/remote.sh, then fetches the output."""
    run_id = args.out_name or dt.datetime.now().strftime("%Y-%m-%d-%H%M%S")
    rel = f"test-output/scenes/{run_id}"
    inner = ["scripts/render-scenes.sh", "--out", rel, *passthrough]
    status = subprocess.run([str(ROOT / "scripts/remote.sh"), args.remote, "run",
                             shlex.join(inner)], cwd=ROOT).returncode
    local = ROOT / "test-output/scenes"
    fetched = subprocess.run([str(ROOT / "scripts/remote.sh"), args.remote, "fetch", rel,
                              str(local)], cwd=ROOT).returncode
    # drive.sh's run archive too (test-output/YYYY-MM-DD/, without index.md:
    # scripts/collect-test-output.sh adds its rows).
    day = dt.date.today().isoformat()
    subprocess.run([str(ROOT / "scripts/remote.sh"), args.remote, "fetch",
                    f"test-output/{day}", str(ROOT / "test-output")], cwd=ROOT)
    print(f"fetched into {local / run_id}" if fetched == 0 else "fetch failed")
    return status or fetched


# --- picking renders for the build history (test-output/build-history/) ---

def png_text_chunks(data: bytes) -> list[bytes]:
    """The tEXt chunks (whole, with length and CRC) of a PNG file."""
    out, at = [], 8
    while at + 8 <= len(data):
        n = int.from_bytes(data[at:at + 4], "big")
        if data[at + 4:at + 8] == b"tEXt":
            out.append(data[at:at + 12 + n])
        at += 12 + n
    return out


def jpeg_app1(data: bytes) -> list[bytes]:
    """The APP1 segments (EXIF, XMP) of a JPEG file, before its image data."""
    out, at = [], 2
    while at + 4 <= len(data) and data[at] == 0xFF and data[at + 1] not in (0xDA, 0xD9):
        n = int.from_bytes(data[at + 2:at + 4], "big")
        if data[at + 1] == 0xE1:
            out.append(data[at:at + 2 + n])
        at += 2 + n
    return out


def keep_metadata(src: Path, dst: Path) -> None:
    """Copies the time/software metadata of src into dst, which ffmpeg
    re-encoded without it (PNG tEXt chunks after IHDR, JPEG APP1 after SOI)."""
    a, b = src.read_bytes(), dst.read_bytes()
    if a.startswith(b"\x89PNG") and b.startswith(b"\x89PNG"):
        end = 8 + 12 + int.from_bytes(b[8:12], "big")
        dst.write_bytes(b[:end] + b"".join(png_text_chunks(a)) + b[end:])
    elif a.startswith(b"\xff\xd8") and b.startswith(b"\xff\xd8"):
        # After the JFIF header (APP0), if there is one.
        at = 2 + 2 + int.from_bytes(b[4:6], "big") if b[2:4] == b"\xff\xe0" else 2
        dst.write_bytes(b[:at] + b"".join(jpeg_app1(a)) + b[at:])


def shrink_image(src: Path, dst: Path, max_kb: int) -> None:
    """Copies a screenshot; a larger one than max_kb is made smaller (PNG: a
    256-colour palette; JPEG: quality ~80), keeping its metadata."""
    if src.stat().st_size <= max_kb * 1024 or not shutil.which("ffmpeg"):
        shutil.copy2(src, dst)
        return
    if src.suffix == ".png":
        enc = ["-vf", "split[a][b];[a]palettegen=max_colors=256:stats_mode=full[p];"
               "[b][p]paletteuse=dither=sierra2_4a", "-pix_fmt", "pal8"]
    else:
        enc = ["-q:v", "5"]
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(src), *enc, str(dst)],
                   check=True)
    if dst.stat().st_size >= src.stat().st_size:
        shutil.copy2(src, dst)
    else:
        keep_metadata(src, dst)


def shrink_mp4(src: Path, dst: Path, max_mb: float) -> None:
    """Copies a video; one larger than max_mb is re-encoded (higher CRF, then 960 px)."""
    shutil.copy2(src, dst)
    for crf, width in ((28, 1280), (30, 960), (33, 854)):
        if dst.stat().st_size <= max_mb * 1e6:
            return
        # The sound and the metadata (creation_time, comment) stay.
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(src), "-vf",
                        f"scale='min({width},iw)':-2", "-c:v", "libx264", "-preset", "slow",
                        "-crf", str(crf), "-pix_fmt", "yuv420p", "-c:a", "copy",
                        "-map_metadata", "0", "-movflags", "+faststart", str(dst)], check=True)


def update_count_line(index: Path) -> None:
    files = [p for p in HISTORY.rglob("*") if p.is_file() and p.name != "index.md"]
    by_ext: dict[str, int] = {}
    for p in files:
        by_ext[p.suffix.lstrip(".").upper()] = by_ext.get(p.suffix.lstrip(".").upper(), 0) + 1
    parts = ", ".join(f"{n} {e}" for e, n in sorted(by_ext.items(), key=lambda kv: -kv[1]))
    total = sum(p.stat().st_size for p in files) / 1e6
    line = f"{len(files)} files ({parts}), {total:.1f} MB in total."
    text = index.read_text().splitlines()
    for i, l in enumerate(text):
        if l.endswith("MB in total."):
            text[i] = line
            break
    index.write_text("\n".join(text) + "\n")


INDEX_HEADER = """# OCW build history (scene renders)

Picked by `scripts/render-scenes.sh --pick`; packed into the zip of the
GitHub release `build-history`. Map renders contain OpenStreetMap-derived
data: © OpenStreetMap contributors (ODbL).

0 files, 0.0 MB in total.

| Date | File | What it shows |
|---|---|---|
"""


def pick(src_dir: Path, scenes: list[dict], max_png_kb: int) -> int:
    readme = HISTORY / "index.md"
    HISTORY.mkdir(parents=True, exist_ok=True)
    if not readme.exists():
        readme.write_text(INDEX_HEADER)
    rows = []
    by_name = {s["name"]: s for s in scenes}
    for f in sorted(src_dir.iterdir()):
        scene = by_name.get(f.stem)
        # .png: runs from before JPEG screenshots (spec §11.4).
        if scene is None or f.suffix not in (".jpg", ".png", ".mp4") or not scene.get("pick", True):
            continue
        # A video's last frame (NAME.jpg beside NAME.mp4) is not picked
        # unless the scene also has its own screenshot (a separate run).
        if f.suffix in (".jpg", ".png") and "shot" not in scene["kinds"]:
            continue
        stamp = dt.datetime.fromtimestamp(f.stat().st_mtime)
        day = stamp.strftime("%Y-%m-%d")
        name = f"{stamp:%H%M%S}-{scene['name']}{f.suffix}"
        (HISTORY / day).mkdir(parents=True, exist_ok=True)
        dst = HISTORY / day / name
        if f.suffix in (".jpg", ".png"):
            shrink_image(f, dst, max_png_kb)
        else:
            shrink_mp4(f, dst, float(scene.get("max_video_mb", 3.0)))
        what = " ".join(scene["description"].split())
        rows.append((f"{day} {stamp:%H:%M:%S}",
                     f"| {day} {stamp:%H:%M:%S} | [{name}]({day}/{name}) | {what} "
                     f"(scene `{scene['name']}`) |"))
        print(f"picked {day}/{name} ({dst.stat().st_size / 1e6:.2f} MB)")
    if not rows:
        print("nothing to pick")
        return 1
    lines = readme.read_text().splitlines()
    existing = set(lines)
    lines += [r for _, r in sorted(rows) if r not in existing]
    readme.write_text("\n".join(lines) + "\n")
    update_count_line(readme)
    return 0


def main() -> int:
    p = argparse.ArgumentParser(
        prog="render-scenes.sh",
        description="Render the scenes of scripts/scenes.toml headlessly (drive.sh).")
    p.add_argument("--list", action="store_true", help="list the scenes and exit")
    p.add_argument("--only", action="append", default=[], metavar="NAME",
                   help="only these scenes (comma-separated, globs: 'menu-*'; repeatable)")
    p.add_argument("--scenes", type=Path, default=DEFAULT_SCENES, help="scene file")
    p.add_argument("--world", action="append", default=[], metavar="[NAME=]PKG",
                   help="world package: PKG for the Liechtenstein scenes (default $OCW_WORLD, "
                   "data/pilot/, the main checkout's data/pilot/, /var/tmp/ocw/), NAME=PKG for "
                   f"another ({', '.join(WORLDS)}; default $OCW_WORLD_NAME, data/...); repeatable")
    p.add_argument("--x11", action="store_true", help="X11 (Xvfb) instead of Wayland (weston)")
    p.add_argument("--out", type=Path,
                   help="output folder (default test-output/scenes/YYYY-MM-DD-HHMMSS)")
    p.add_argument("--remote", metavar="MACHINE",
                   help="run on a build machine (scripts/remote.sh), then fetch the output")
    p.add_argument("--out-name", help=argparse.SUPPRESS)
    p.add_argument("--pick", type=Path, metavar="DIR", dest="pick_dir",
                   help="copy the renders of a folder (shrunk; --only selects) into "
                   "test-output/build-history/YYYY-MM-DD/HHMMSS-NAME.EXT with index.md rows")
    p.add_argument("--max-png-kb", type=int, default=600,
                   help="--pick: larger PNGs get a 256-colour palette (default 600)")
    args, rest = p.parse_known_args()
    if rest:
        p.error(f"unknown arguments: {rest}")

    _, scenes = load_scenes(args.scenes)
    if args.pick_dir:
        return pick(args.pick_dir, select(scenes, args.only), args.max_png_kb)
    if args.remote:
        passthrough = [a for o in args.only for a in ("--only", o)]
        passthrough += [a for w in args.world for a in ("--world", w)]
        if args.x11:
            passthrough.append("--x11")
        if args.scenes != DEFAULT_SCENES:
            passthrough += ["--scenes", os.path.relpath(args.scenes, ROOT)]
        return remote(args, passthrough)

    explicit = parse_world_args(args.world)
    chosen = select(scenes, args.only)
    needed = sorted({w for s in chosen if (w := world_name(s))} | set(explicit))
    worlds = {name: find_world(name, explicit.get(name)) for name in needed}
    found = ", ".join(f"{n}: {p or 'none'}" for n, p in worlds.items()) or "none needed"
    if args.list:
        print(f"world packages: {found}")
        for s in chosen:
            print(describe(s, worlds.get(world_name(s) or "")))
        return 0
    out = args.out or ROOT / "test-output/scenes" / dt.datetime.now().strftime("%Y-%m-%d-%H%M%S")
    out = out if out.is_absolute() else ROOT / out
    out.mkdir(parents=True, exist_ok=True)
    print(f"world packages: {found}; output: {out}", flush=True)
    failed, skipped, done = [], [], []
    for s in chosen:
        name = world_name(s)
        world = worlds.get(name) if name else None
        if name and world is None:
            skipped.append(s["name"])
            continue
        ok = run_scene(s, world, out, "x11" if args.x11 else "wayland")
        (done if ok else failed).append(s["name"])
    print(f"rendered {len(done)}, failed {len(failed)} {failed or ''}, "
          f"skipped (no world package) {len(skipped)} {skipped or ''}")
    print(f"output: {out}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
