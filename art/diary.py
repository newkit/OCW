#!/usr/bin/env python3
"""OCW Build Diary: the art project's tool (art/ART-PROJECT.md).

    art/diary.py fetch   [DAY...]   download and unpack the day zips from the
                                    GitHub release `build-history`
    art/diary.py gather  [DAY...]   collect every image, video, sound, text and
                                    log of the day into artwork/build-history/DAY/
    art/diary.py stamp   [DAY...]   write the ISO 8601 capture time into every
                                    archived PNG/MP4 that has none (gather does it)
    art/diary.py prompts [DAY...]   the user's prompts of the day (from the Claude
                                    Code transcripts, masked) – journal runs it
    art/diary.py journal [DAY...]   write DAY/journal.md + journal.json (commits,
                                    merges, spec versions, tests, media)
    art/diary.py sheet   DAY        contact sheets of the day's media (for
                                    writing the day's script)
    art/diary.py render  [DAY...]   render the day video(s) from art/days/DAY.toml
    art/diary.py diary              chain all day videos into the diary video
    art/diary.py publish [DAY...]   zip the day folders and upload zips and
                                    videos to the release `build-history`
    art/diary.py all     [DAY...]   gather + journal + render + diary

DAY is YYYY-MM-DD; without DAY: every day that has a script in art/days/.
Needs Python 3.11+ (tomllib), ffmpeg/ffprobe with libass, and `gh` for
fetch/publish. Media stay out of git (spec §11.3): everything lands in the
git-ignored artwork/build-history/.
"""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import math
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tomllib
import wave
from array import array
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ART = ROOT / "art"
DAYS = ART / "days"
HIST = ROOT / "artwork" / "build-history"
OUT = HIST / "diary"
WORK = HIST / ".work"
TEST_OUTPUT = ROOT / "test-output"
FONTS = ROOT / "crates" / "ocw-render" / "assets" / "fonts"
RELEASE = "build-history"

IMAGE = {".png", ".jpg", ".jpeg", ".gif", ".webp"}
VIDEO = {".mp4", ".webm", ".mkv", ".mov"}
AUDIO = {".wav", ".ogg", ".mp3", ".flac"}
TEXT = {".txt", ".md"}
LOGS = {".log", ".json", ".jsonl", ".args", ".route", ".toml", ".csv"}

W, H, FPS = 1920, 1080, 30
RATE = 44100

# Brand colours (artwork/logos/ocw.svg): rider pink, deep violet, gold.
PINK, VIOLET, GOLD, INK = "FF1472", "3A2F55", "C98A00", "2B2B3A"


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def run(cmd, **kw):
    kw.setdefault("check", True)
    return subprocess.run([str(c) for c in cmd], **kw)


def ffmpeg(*args):
    run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args])


def probe_duration(path: Path) -> float:
    out = run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
        capture_output=True,
        text=True,
    ).stdout.strip()
    try:
        return float(out)
    except ValueError:
        return 0.0


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def days_with_scripts() -> list[str]:
    return sorted(p.stem for p in DAYS.glob("????-??-??.toml"))


def all_days() -> list[str]:
    """Days with commits (local time)."""
    out = run(
        ["git", "-C", ROOT, "log", "--format=%ad", "--date=format-local:%Y-%m-%d"],
        capture_output=True,
        text=True,
    ).stdout.split()
    return sorted(set(out))


def day_number(day: str) -> int:
    return all_days().index(day) + 1


# ------------------------------------------------------------------ gather


def fetch(day: str):
    HIST.mkdir(parents=True, exist_ok=True)
    z = HIST / f"ocw-build-history-{day}.zip"
    if not z.exists():
        r = run(
            ["gh", "release", "download", RELEASE, "-R", "newkit/OCW", "-D", HIST,
             "-p", z.name],
            check=False,
        )
        if r.returncode:
            log(f"{day}: no zip on the release")
            return
    if not (HIST / day).exists():
        run(["unzip", "-q", "-n", z, "-d", HIST])
    log(f"{day}: unpacked {z.name}")


TIME_RE = re.compile(r"^(\d{6})-")
DIR_TIME_RE = re.compile(r"(\d{4}-?\d{2}-?\d{2})[-_T]?(\d{6})")


def file_time(path: Path) -> tuple[str, str]:
    """(YYYY-MM-DD, HHMMSS) of a file: name, folder name, else mtime."""
    for part in [path.name, *[p.name for p in path.parents][:3]]:
        m = DIR_TIME_RE.search(part)
        if m:
            d = m.group(1).replace("-", "")
            return f"{d[:4]}-{d[4:6]}-{d[6:]}", m.group(2)
    t = dt.datetime.fromtimestamp(path.stat().st_mtime)
    m = TIME_RE.match(path.name)
    hms = m.group(1) if m else t.strftime("%H%M%S")
    # A date folder (test-output/YYYY-MM-DD/...) is the day.
    for p in path.parents:
        if re.fullmatch(r"\d{4}-\d{2}-\d{2}", p.name):
            return p.name, hms
    return t.strftime("%Y-%m-%d"), hms


def slug(s: str) -> str:
    s = re.sub(r"[^A-Za-z0-9.,+-]+", "-", s).strip("-")
    return re.sub(r"-{2,}", "-", s)


def sources() -> list[Path]:
    """Every checkout's test-output (main + agent worktrees)."""
    out = [TEST_OUTPUT]
    out += sorted((ROOT / ".claude" / "worktrees").glob("*/test-output"))
    return [p for p in out if p.is_dir()]


def gather(day: str):
    dest = HIST / day
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "logs").mkdir(exist_ok=True)
    # Checksums of the gathered originals: the copies here get metadata
    # (stamp), so their own checksums differ from the sources'.
    manifest = dest / "logs" / ".sources.json"
    known = json.loads(manifest.read_text()) if manifest.exists() else {}
    for f in dest.rglob("*"):
        if f.is_file() and f.name != manifest.name:
            known.setdefault(sha(f), f.name)
    added = 0
    for src in sources():
        for f in sorted(src.rglob("*")):
            if not f.is_file() or HIST in f.parents:
                continue
            ext = f.suffix.lower()
            if ext not in IMAGE | VIDEO | AUDIO | TEXT | LOGS or f.name == "index.md":
                continue
            d, hms = file_time(f)
            if d != day:
                continue
            if "build-log" in f.parts:
                # The build log (remote.sh runs, dashboard samples) grows
                # during the day: keep one copy per file, updated in place.
                target = dest / "logs" / "build-log" / f.name
                if not target.exists() or target.stat().st_size != f.stat().st_size:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(f, target)
                    added += 1
                continue
            h = sha(f)
            if h in known:
                continue
            rel = f.relative_to(src).with_suffix("").as_posix()
            stem = re.sub(r"^\d{4}-\d{2}-\d{2}/", "", rel)
            stem = DIR_TIME_RE.sub("", stem)
            stem = re.sub(r"(^|/)\d{6}-", r"\1", stem)
            stem = re.sub(r"(^|[-/])(nogit|[0-9a-f]{7}(-dirty)?)-", r"\1", stem)
            stem = re.sub(r"agent-[0-9a-f]{17}", "agent", stem)
            name = f"{hms}-{slug(stem)[:90] or 'file'}{ext}"
            target = (dest / "logs" if ext in LOGS else dest) / name
            n = 2
            while target.exists():
                target = target.with_name(f"{hms}-{slug(stem)[:90]}-{n}{ext}")
                n += 1
            shutil.copy2(f, target)
            known[h] = target.name
            added += 1
    manifest.write_text(json.dumps(known, indent=0))
    stamp(day)
    media = [f for f in dest.iterdir() if f.suffix.lower() in IMAGE | VIDEO | AUDIO]
    log(f"{day}: {added} new files; {len(media)} images/videos/sounds in {dest}")


# ------------------------------------------------------------------- stamp
#
# Every photo and video carries its ISO 8601 date and time in the file
# (ART-PROJECT.md §2): PNG `tEXt` "Creation Time", MP4 `creation_time`.
# The app writes them itself (spec 2.24+); for older files the archive copy
# gets them from the capture time (file name / folder / mtime).


def capture_time(day: str, name: str) -> dt.datetime:
    m = TIME_RE.match(name)
    hms = m.group(1) if m else "000000"
    t = dt.datetime.strptime(f"{day} {hms}", "%Y-%m-%d %H%M%S")
    return t.astimezone()  # local time zone of this machine (the build's)


def png_text(path: Path) -> dict[str, str]:
    data = path.read_bytes()
    out, i = {}, 8
    while i + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        if kind == b"tEXt":
            k, _, v = data[i + 8:i + 8 + n].partition(b"\0")
            out[k.decode("latin-1")] = v.decode("latin-1")
        if kind == b"IEND":
            break
        i += 12 + n
    return out


def png_add_text(path: Path, items: dict[str, str]):
    """Insert tEXt chunks before the first IDAT."""
    import zlib

    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return
    i = 8
    while i + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        if kind == b"IDAT":
            break
        i += 12 + n
    chunks = b""
    for k, v in items.items():
        body = k.encode("latin-1") + b"\0" + v.encode("latin-1")
        chunks += struct.pack(">I", len(body)) + b"tEXt" + body + struct.pack(
            ">I", zlib.crc32(b"tEXt" + body) & 0xFFFFFFFF)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(data[:i] + chunks + data[i:])
    os.utime(tmp, (path.stat().st_atime, path.stat().st_mtime))
    tmp.replace(path)


def mp4_creation_time(path: Path) -> str:
    return run(["ffprobe", "-v", "error", "-show_entries", "format_tags=creation_time",
                "-of", "csv=p=0", path], capture_output=True, text=True,
               check=False).stdout.strip()


def stamp(day: str):
    """Writes the capture time into every archived PNG and MP4 that has none."""
    n = 0
    for f in sorted((HIST / day).iterdir()):
        ext = f.suffix.lower()
        if ext not in {".png", ".mp4"}:
            continue
        t = capture_time(day, f.name)
        if ext == ".png":
            if "Creation Time" in png_text(f):
                continue
            png_add_text(f, {"Creation Time": t.isoformat(timespec="seconds"),
                             "Comment": "OCW build history (time from the capture's file name)"})
        else:
            if mp4_creation_time(f):
                continue
            tmp = f.with_name(f.stem + ".stamp.mp4")
            ffmpeg("-i", f, "-map", "0", "-c", "copy", "-metadata",
                   f"creation_time={t.astimezone(dt.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.000000Z')}",
                   "-metadata", "comment=OCW build history (time from the capture's file name)", tmp)
            os.utime(tmp, (f.stat().st_atime, f.stat().st_mtime))
            tmp.replace(f)
        n += 1
    log(f"{day}: stamped {n} files with their ISO 8601 capture time")


def now_meta() -> list[str]:
    t = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000000Z")
    return ["-metadata", f"creation_time={t}"]


# ----------------------------------------------------------------- prompts
#
# The user's prompts to Claude Code are part of the story (ART-PROJECT §2.3):
# taken from the session transcripts (~/.claude/projects/*OCW*/*.jsonl, the
# main sessions only, not the agents'), with host names, user names, e-mail
# addresses and keys masked, into DAY/prompts.jsonl and DAY/prompts.md.

TRANSCRIPTS = os.environ.get("OCW_TRANSCRIPTS", str(Path.home() / ".claude" / "projects"))
REMINDER_RE = re.compile(r"<system-reminder>.*?</system-reminder>", re.S)
EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
# Private network addresses (10/8, 172.16/12, 192.168/16, 100.64/10).
PRIVATE_IP_RE = re.compile(
    r"\b(?:10\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])|192\.168|100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7]))"
    r"\.\d{1,3}\.\d{1,3}\b")


def _secrets() -> list[tuple[str, str]]:
    """(text, replacement) pairs: every build machine's host and user (asked
    from scripts/remote.sh, which reads config/machines.toml) and the home
    directory. Never printed."""
    out = []
    conf = ROOT / "config" / "machines.toml"
    names = re.findall(r'^\s*name\s*=\s*"([^"]+)"', conf.read_text(), re.M) if conf.exists() else []
    for name in names:
        for k in ("host", "user"):
            v = run([ROOT / "scripts" / "remote.sh", name, "field", k], capture_output=True,
                    text=True, check=False).stdout.strip()
            if len(v) >= 3:
                out.append((v, name if k == "host" else "user"))
    out.append((str(Path.home()), "~"))
    return sorted(out, key=lambda kv: -len(kv[0]))


def mask(text: str, secrets=None) -> str:
    for v, r in (secrets if secrets is not None else _secrets()):
        text = text.replace(v, r)
    return PRIVATE_IP_RE.sub("<ip>", EMAIL_RE.sub("<e-mail>", text))


def prompt_text(raw: str) -> str | None:
    """The user's words from a transcript message, or None for anything that
    is not a human prompt (tool output, notifications, agent messages)."""
    t = REMINDER_RE.sub("", raw).strip()
    if not t:
        return None
    m = re.match(r"<command-name>(/?[^<]+)</command-name>.*?(?:<command-args>(.*?)</command-args>)?", t, re.S)
    if m:
        return (("/" + m.group(1).lstrip("/")) + (" " + m.group(2).strip() if m.group(2) else "")).strip()
    bad = ("<task-notification", "<agent-message", "[SYSTEM NOTIFICATION", "Another Claude session",
           "<local-command", "[Request interrupted", "Caveat:", "<command-message", "This session is being continued")
    if any(t.startswith(b) or b in t[:200] for b in bad):
        return None
    return t


def prompts_of(day: str) -> list[dict]:
    seen, out = set(), []
    files = [f for f in Path(TRANSCRIPTS).glob("*OCW*/*.jsonl")]
    for f in files:
        for line in f.open(errors="replace"):
            if '"user"' not in line and "queued_command" not in line:
                continue
            try:
                d = json.loads(line)
            except ValueError:
                continue
            text, ts = None, d.get("timestamp", "")
            if d.get("type") == "user" and not d.get("isMeta") and d.get("userType", "external") == "external":
                c = (d.get("message") or {}).get("content")
                if isinstance(c, str):
                    text = prompt_text(c)
            elif d.get("type") == "attachment":
                a = d.get("attachment") or {}
                if a.get("type") == "queued_command" and (a.get("origin") or {}).get("kind") == "human":
                    text = prompt_text(str(a.get("prompt", "")))
            if not text or not ts:
                continue
            local = dt.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone()
            if local.strftime("%Y-%m-%d") != day:
                continue
            key = (text, local.strftime("%H:%M"))
            if key in seen:
                continue
            seen.add(key)
            out.append({"time": local.strftime("%H:%M:%S"), "iso": local.isoformat(timespec="seconds"),
                        "text": text})
    secrets = _secrets()
    for p_ in out:
        p_["text"] = mask(p_["text"], secrets)
    out.sort(key=lambda p_: p_["time"])
    return out


def prompts(day: str) -> list[dict]:
    ps = prompts_of(day)
    d = HIST / day
    d.mkdir(parents=True, exist_ok=True)
    (d / "prompts.jsonl").write_text("".join(json.dumps(p_, ensure_ascii=False) + "\n" for p_ in ps))
    md = [f"# Prompts – {day}", "", f"{len(ps)} prompts to Claude Code (main session; masked).", ""]
    for p_ in ps:
        md += [f"**{p_['time']}**", "", "> " + p_["text"].replace("\n", "\n> "), ""]
    (d / "prompts.md").write_text("\n".join(md))
    log(f"{day}: {len(ps)} prompts")
    return ps


# ------------------------------------------------------------------ tokens
#
# Token usage of the build (ART-PROJECT §2.4): every assistant message of the
# Claude Code sessions of this project – the main session and every agent
# (subagents/agent-*.jsonl with their meta.json) – counted once per message
# (de-duplicated by message id), by the local day of its timestamp.
#   task:    an agent's task description; for the main session the user
#            prompt the message answered ("coordination: <prompt>")
#   feature: a coarse group from keywords in the task (FEATURES below)
# Totals: input (uncached), cache writes, cache reads, output.

FEATURES = [  # first match wins: specific features before broad ones
    ("Art project", r"art project|diary|build.?history|readme|logo|prompts? (in|of)|token"),
    ("Materials & look", r"material|texture|shadow|tunnel|marking|wheel|graphic"),
    ("UI & menus", r"menu|hud|timestamp|screenshot|capture|kiosk|dashboard"),
    ("Sky, time & weather", r"\bsun\b|sky|time machine|weather|season|night|day/night"),
    ("Audio & voice", r"audio|sound|freehub|voice|noise"),
    ("Riders & bikes", r"rider|bike|model|animation|\blegs?\b"),
    ("Multiplayer & server", r"multiplayer|server|quic|\bbots?\b"),
    ("Worlds & terrain", r"world|terrain|black ?forest|liechtenstein|tenerif|osm|\bdem\b|bridge|clean|package|route|mummelsee"),
    ("Build, release & infra", r"build|release|deploy|\bci\b|windows|\bmac|clang|version|machine|coordinator"),
    ("Planning & spec", r"plan|spec|phase|\bvr\b|browser|question"),
]


def feature_of(task: str) -> str:
    t = (task or "").lower()
    for name, rx in FEATURES:
        if re.search(rx, t):
            return name
    return "Other"


def _usage(u: dict) -> dict:
    return {"input": int(u.get("input_tokens") or 0),
            "cache_write": int(u.get("cache_creation_input_tokens") or 0),
            "cache_read": int(u.get("cache_read_input_tokens") or 0),
            "output": int(u.get("output_tokens") or 0)}


def _add(acc: dict, u: dict):
    for k, v in u.items():
        acc[k] = acc.get(k, 0) + v
    acc["total"] = acc.get("total", 0) + sum(u.values())
    acc["messages"] = acc.get("messages", 0) + 1


def tokens_of(day: str) -> dict:
    root = Path(TRANSCRIPTS)
    seen = set()
    total, per_task, per_feature, per_model = {}, {}, {}, {}
    sources = []
    for f in root.glob("*OCW*/*.jsonl"):
        sources.append((f, None))
        for a in (f.parent / f.stem / "subagents").glob("agent-*.jsonl"):
            meta = a.with_suffix(".meta.json")
            desc = ""
            if meta.exists():
                try:
                    desc = json.loads(meta.read_text()).get("description", "")
                except ValueError:
                    pass
            sources.append((a, desc or a.stem))
    secrets = _secrets()
    for f, agent in sources:
        last_prompt = None
        for line in f.open(errors="replace"):
            if agent is None and ('"user"' in line or "queued_command" in line):
                try:
                    d = json.loads(line)
                except ValueError:
                    continue
                text = None
                if d.get("type") == "user" and isinstance((d.get("message") or {}).get("content"), str):
                    text = prompt_text(d["message"]["content"])
                elif d.get("type") == "attachment":
                    a_ = d.get("attachment") or {}
                    if a_.get("type") == "queued_command" and (a_.get("origin") or {}).get("kind") == "human":
                        text = prompt_text(str(a_.get("prompt", "")))
                if text:
                    last_prompt = text
                continue
            if '"usage"' not in line or '"assistant"' not in line:
                continue
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if d.get("type") != "assistant":
                continue
            m = d.get("message") or {}
            key = (m.get("id"), d.get("requestId"))
            if key in seen or not m.get("usage"):
                continue
            seen.add(key)
            ts = d.get("timestamp", "")
            try:
                local = dt.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone()
            except ValueError:
                continue
            if local.strftime("%Y-%m-%d") != day:
                continue
            u = _usage(m["usage"])
            if agent is not None:
                task = "agent: " + agent
            else:
                p = " ".join((last_prompt or "(session start)").split())
                task = "coordination: " + mask(p[:80] + ("…" if len(p) > 80 else ""), secrets)
            feat = feature_of(agent if agent is not None else (last_prompt or ""))
            _add(total, u)
            _add(per_task.setdefault(task, {"feature": feat}), u)
            _add(per_feature.setdefault(feat, {}), u)
            _add(per_model.setdefault(m.get("model", "?"), {}), u)
    return {"day": day, "total": total, "tasks": per_task, "features": per_feature, "models": per_model}


def tokens(day: str) -> dict:
    t = tokens_of(day)
    d = HIST / day
    d.mkdir(parents=True, exist_ok=True)
    (d / "tokens.json").write_text(json.dumps(t, indent=1, ensure_ascii=False))
    log(f"{day}: {t['total'].get('total', 0):,} tokens in {t['total'].get('messages', 0)} messages")
    return t


def fmt_tokens(n: int) -> str:
    return f"{n / 1e9:.2f} B" if n >= 1e9 else f"{n / 1e6:.1f} M" if n >= 1e6 else f"{n / 1e3:.0f} k"


# ----------------------------------------------------------------- journal


def git_lines(*args) -> list[str]:
    return run(["git", "-C", ROOT, *args], capture_output=True, text=True).stdout.splitlines()


def commits_of(day: str) -> list[dict]:
    out = []
    fmt = "%H%x1f%h%x1f%ad%x1f%P%x1f%s"
    for line in git_lines(
        "log", "--reverse", f"--format={fmt}", "--date=format-local:%Y-%m-%d %H:%M:%S"
    ):
        full, short, when, parents, subject = line.split("\x1f")
        if not when.startswith(day):
            continue
        out.append(
            {
                "sha": short,
                "time": when[11:],
                "merge": len(parents.split()) > 1,
                "subject": subject,
            }
        )
    # Lines changed (non-merge commits).
    stats = {}
    cur = None
    for line in git_lines(
        "log", "--no-merges", "--format=@%h", "--shortstat",
        "--date=format-local:%Y-%m-%d",
    ):
        if line.startswith("@"):
            cur = line[1:]
        elif cur and "changed" in line:
            ins = re.search(r"(\d+) insertion", line)
            dels = re.search(r"(\d+) deletion", line)
            stats[cur] = (int(ins.group(1)) if ins else 0, int(dels.group(1)) if dels else 0)
    for c in out:
        c["added"], c["removed"] = stats.get(c["sha"], (0, 0))
    out.sort(key=lambda c: c["time"])
    return out


def media_of(day: str) -> list[dict]:
    d = HIST / day
    out = []
    for f in sorted(d.iterdir()) if d.exists() else []:
        ext = f.suffix.lower()
        if ext not in IMAGE | VIDEO | AUDIO or f.stat().st_size == 0:
            continue  # empty files (an aborted capture) are kept but not shown
        m = TIME_RE.match(f.name)
        hms = m.group(1) if m else "000000"
        kind = "image" if ext in IMAGE else "video" if ext in VIDEO else "audio"
        out.append({"file": f.name, "time": f"{hms[:2]}:{hms[2:4]}:{hms[4:]}", "kind": kind})
    out.sort(key=lambda m: (m["time"], m["file"]))
    return out


def tests_of(commits: list[dict]) -> int | None:
    """The highest test count a commit message of the day mentions."""
    best = None
    for c in commits:
        for m in re.finditer(r"(\d{3,4}) tests", c["subject"]):
            best = max(best or 0, int(m.group(1)))
    return best


def builds_of(day: str) -> dict:
    """Summary of the day's build log: remote.sh runs and build-server samples."""
    d = HIST / day / "logs" / "build-log"
    runs, samples = [], []
    for name, out in (("runs.jsonl", runs), ("servers.jsonl", samples)):
        p = d / name
        if p.exists():
            for line in p.read_text(errors="replace").splitlines():
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    machines = {}
    for r in runs:
        m = machines.setdefault(r.get("machine", "?"), {"runs": 0, "seconds": 0.0, "failed": 0,
                                                         "cargo_s": 0.0, "compiled": 0, "wait_s": 0.0})
        m["runs"] += 1
        m["seconds"] += float(r.get("seconds") or 0)
        m["failed"] += int(r.get("exit", 0) != 0)
        # Cargo's own times, crates compiled and build-slot waits (spec
        # §11.2; runs logged since 2026-11-13, null where cargo was quiet).
        m["cargo_s"] += float(r.get("cargo_s") or 0)
        m["compiled"] += int(r.get("compiled") or 0)
        m["wait_s"] += float(r.get("wait_s") or 0)
    online = {}
    for s_ in samples:
        for m in (s_.get("status") or {}).get("machines", []) or []:
            o = online.setdefault(m.get("name", "?"), [0, 0])
            o[0] += 1 if m.get("online") else 0
            o[1] += 1
    return {
        "runs": len(runs),
        "seconds": sum(float(r.get("seconds") or 0) for r in runs),
        "cargo_s": sum(float(r.get("cargo_s") or 0) for r in runs),
        "compiled": sum(int(r.get("compiled") or 0) for r in runs),
        "wait_s": sum(float(r.get("wait_s") or 0) for r in runs),
        "failed": [r for r in runs if r.get("exit", 0) != 0],
        "longest": sorted(runs, key=lambda r: -float(r.get("seconds") or 0))[:5],
        "machines": machines,
        "online": {k: (v[0] / v[1] if v[1] else 0.0) for k, v in online.items()},
        "samples": len(samples),
    }


def journal(day: str) -> dict:
    commits = commits_of(day)
    builds = builds_of(day)
    ps = prompts(day)
    tk = tokens(day)
    media = media_of(day)
    spec = [
        m.group(1)
        for c in commits
        for m in [re.search(r"[Ss]pec (?:v)?(\d+\.\d+)", c["subject"])]
        if m
    ]
    j = {
        "day": day,
        "number": day_number(day),
        "commits": commits,
        "merges": sum(c["merge"] for c in commits),
        "lines_added": sum(c["added"] for c in commits),
        "lines_removed": sum(c["removed"] for c in commits),
        "spec_versions": spec,
        "tests": tests_of(commits),
        "media": media,
        "images": sum(m["kind"] == "image" for m in media),
        "videos": sum(m["kind"] == "video" for m in media),
        "sounds": sum(m["kind"] == "audio" for m in media),
        "builds": {k: v for k, v in builds.items() if k not in ("longest",)},
        "prompts": ps,
        "tokens": {"total": tk["total"], "features": tk["features"]},
    }
    d = HIST / day
    d.mkdir(parents=True, exist_ok=True)
    (d / "journal.json").write_text(json.dumps(j, indent=1, ensure_ascii=False))
    md = [
        f"# OCW build diary – day {j['number']}, {day}",
        "",
        f"{len(commits)} commits ({j['merges']} merges), +{j['lines_added']} / "
        f"−{j['lines_removed']} lines, {j['images']} images, {j['videos']} videos, "
        f"{j['sounds']} sounds" + (f", {j['tests']} tests" if j["tests"] else "") + ".",
        "",
        "## Commits",
        "",
        "| Time | Commit | |",
        "|---|---|---|",
    ]
    for c in commits:
        md.append(
            f"| {c['time'][:5]} | `{c['sha']}`{' (merge)' if c['merge'] else ''} | "
            f"{c['subject'].replace('|', '/')} |"
        )
    if builds["runs"] or builds["samples"]:
        md += ["", "## Builds and build servers", "",
               f"{builds['runs']} build/test runs on the build machines, "
               f"{builds['seconds'] / 3600:.1f} h in total, {len(builds['failed'])} failed; "
               f"{builds['samples']} server samples; cargo compiled {builds['compiled']} crates "
               f"in {builds['cargo_s'] / 3600:.1f} h, {builds['wait_s'] / 3600:.1f} h waiting for build slots.", "",
               "| Machine | Runs | Hours | Cargo h | Crates | Waited h | Failed | Online |",
               "|---|---|---|---|---|---|---|---|"]
        for name in sorted(set(builds["machines"]) | set(builds["online"])):
            m = builds["machines"].get(name, {"runs": 0, "seconds": 0, "failed": 0})
            on = builds["online"].get(name)
            md.append(f"| {name} | {m['runs']} | {m['seconds'] / 3600:.1f} | "
                      f"{m.get('cargo_s', 0) / 3600:.1f} | {m.get('compiled', 0)} | "
                      f"{m.get('wait_s', 0) / 3600:.1f} | {m['failed']} | "
                      + (f"{on:.0%}" if on is not None else "–") + " |")
        if builds["failed"]:
            md += ["", "Failed runs:", ""]
            md += [f"- {r.get('start', '')[11:16]} {r.get('machine')}: `{r.get('command', '')[:80]}` "
                   f"(exit {r.get('exit')}, log `build-log/{r.get('log')}`)" for r in builds["failed"]]
        if builds["longest"]:
            md += ["", "Longest runs:", ""]
            md += [f"- {float(r.get('seconds') or 0) / 60:.0f} min on {r.get('machine')}: "
                   f"`{r.get('command', '')[:80]}`" for r in builds["longest"]]
    if tk["total"].get("total"):
        tt = tk["total"]
        md += ["", "## Token usage", "",
               f"{fmt_tokens(tt['total'])} tokens in {tt['messages']:,} model messages "
               f"(output {fmt_tokens(tt['output'])}, cache reads {fmt_tokens(tt['cache_read'])}, "
               f"cache writes {fmt_tokens(tt['cache_write'])}, uncached input {fmt_tokens(tt['input'])}). "
               "Full breakdown: [tokens.json](tokens.json).", "",
               "| Feature | Tokens | Output | Messages |", "|---|---|---|---|"]
        for name, v in sorted(tk["features"].items(), key=lambda kv: -kv[1]["total"]):
            md.append(f"| {name} | {fmt_tokens(v['total'])} | {fmt_tokens(v['output'])} | {v['messages']:,} |")
        md += ["", "| Task | Feature | Tokens | Output |", "|---|---|---|---|"]
        for name, v in sorted(tk["tasks"].items(), key=lambda kv: -kv[1]["total"])[:40]:
            md.append(f"| {name.replace('|', '/')} | {v['feature']} | {fmt_tokens(v['total'])} | {fmt_tokens(v['output'])} |")
    if ps:
        md += ["", "## Prompts", "", f"{len(ps)} prompts (full text: [prompts.md](prompts.md)).", "",
               "| Time | Prompt |", "|---|---|"]
        md += [f"| {p_['time'][:5]} | {p_['text'][:140].replace('|', '/').replace(chr(10), ' ')} |" for p_ in ps]
    md += ["", "## Media", "", "| Time | File |", "|---|---|"]
    md += [f"| {m['time']} | [{m['file']}]({m['file']}) |" for m in media]
    (d / "journal.md").write_text("\n".join(md) + "\n")
    log(f"{day}: journal – {len(commits)} commits, {len(media)} media")
    return j


# ------------------------------------------------------------------ frames


def norm_still(src: Path, day: str) -> Path:
    """The image (or a video's middle frame) as a 1920x1080 JPEG: fitted over
    a blurred, darkened copy of itself (no black bars)."""
    out = WORK / day / "norm" / (src.name + ".jpg")
    if out.exists():
        return out
    out.parent.mkdir(parents=True, exist_ok=True)
    pre = []
    if src.suffix.lower() in VIDEO:
        pre = ["-ss", f"{max(0.0, probe_duration(src) / 2):.2f}"]
    ffmpeg(*pre, "-i", src, "-frames:v", "1", "-filter_complex", FIT, "-q:v", "3", out)
    return out


FIT = (
    f"[0:v]split[a][b];"
    f"[a]scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},"
    f"boxblur=24:2,eq=brightness=-0.18:saturation=1.2[bg];"
    f"[b]scale={W}:{H}:force_original_aspect_ratio=decrease[fg];"
    f"[bg][fg]overlay=(W-w)/2:(H-h)/2,setsar=1,format=yuv420p"
)


def sheet(day: str):
    media = [m for m in media_of(day) if m["kind"] != "audio"]
    out = WORK / day / "sheets"
    out.mkdir(parents=True, exist_ok=True)
    per = 20
    for i in range(0, len(media), per):
        chunk = media[i : i + per]
        inputs, filt = [], []
        for k, m in enumerate(chunk):
            inputs += ["-i", norm_still(HIST / day / m["file"], day)]
            label = m["file"].replace(":", r"\:")[:44]
            filt.append(
                f"[{k}:v]scale=480:270,drawbox=y=240:h=30:color=black@0.7:t=fill,"
                f"drawtext=fontfile={FONTS / 'FiraMono-Regular.ttf'}:text='{label}':"
                f"x=4:y=246:fontsize=15:fontcolor=white[t{k}]"
            )
        for k in range(len(chunk), per):
            filt.append(f"color=black:s=480x270:d=1[t{k}]")
        filt.append("".join(f"[t{k}]" for k in range(per)) + "xstack=inputs=20:layout=" + "|".join(
            f"{(k % 4) * 480}_{(k // 4) * 270}" for k in range(per)
        ) + "[v]")
        target = out / f"sheet-{i // per + 1:02d}.jpg"
        ffmpeg(*inputs, "-filter_complex", ";".join(filt), "-map", "[v]", "-frames:v", "1",
               "-q:v", "4", target)
        log(target)




# ------------------------------------------------------------------- shots

XF = 0.5  # cross-transition between segments, s
TRANSITIONS = ["pixelize", "slideleft", "circleopen", "wipeup", "smoothleft",
               "radial", "hblur", "squeezeh", "diagtl", "slideup"]
ENC = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p",
       "-r", str(FPS)]


def media_path(day: str, name: str) -> Path:
    """A shot's file: the day folder, else a path in the repository."""
    p = HIST / day / name
    if p.exists():
        return p
    p = ROOT / name
    if p.exists():
        return p
    raise SystemExit(f"{day}: shot file not found: {name}")


def seeded(name: str) -> random.Random:
    return random.Random(int(hashlib.md5(name.encode()).hexdigest()[:8], 16))


def kenburns(src: Path, seconds: float, out: Path, day: str):
    """A still with a slow zoom and drift (direction from the file name)."""
    still = norm_still(src, day)
    n = max(1, round(seconds * FPS))
    r = seeded(src.name)
    z0, z1 = (1.0, 1.14) if r.random() < 0.6 else (1.14, 1.0)
    dx, dy = r.choice([-1, 1]) * r.uniform(0.2, 0.5), r.uniform(-0.3, 0.3)
    z = f"{z0}+({z1 - z0})*on/{n}"
    x = f"(iw-iw/zoom)/2*(1+{dx:.3f}*(on/{n}-0.5)*2)"
    y = f"(ih-ih/zoom)/2*(1+{dy:.3f}*(on/{n}-0.5)*2)"
    ffmpeg("-i", still, "-filter_complex",
           f"[0:v]scale=3840:2160,zoompan=z='{z}':x='{x}':y='{y}':d={n}:s={W}x{H}:fps={FPS},"
           f"setsar=1,format=yuv420p[v]",
           "-map", "[v]", "-frames:v", str(n), *ENC, out)


def clip(src: Path, start: float, seconds: float, speed: float, out: Path):
    """A video shot: from `start`, `speed` times faster, fitted to 16:9."""
    n = max(1, round(seconds * FPS))
    dur = probe_duration(src)
    loop = ["-stream_loop", "-1"] if dur and start + seconds * speed > dur else []
    ffmpeg(*loop, "-ss", f"{start:.2f}", "-i", src, "-filter_complex",
           f"[0:v]setpts=PTS/{speed},fps={FPS},{FIT.removeprefix('[0:v]')}[v]",
           "-map", "[v]", "-an", "-frames:v", str(n), *ENC, out)


def split(a: Path, b: Path, seconds: float, out: Path, day: str):
    """Before | after: the left half of one still next to the right half of
    the other, with a bright seam."""
    n = max(1, round(seconds * FPS))
    sa, sb = norm_still(a, day), norm_still(b, day)
    ffmpeg("-loop", "1", "-i", sa, "-loop", "1", "-i", sb, "-filter_complex",
           f"[0:v]crop={W // 2}:{H}:{W // 4}:0[l];[1:v]crop={W // 2}:{H}:{W // 4}:0[r];"
           f"[l][r]hstack,drawbox=x={W // 2 - 4}:y=0:w=8:h={H}:color=0x{PINK}:t=fill,"
           f"zoompan=z='1+0.06*on/{n}':x='(iw-iw/zoom)/2':y='(ih-ih/zoom)/2':d=1:s={W}x{H}:fps={FPS},"
           f"setsar=1,format=yuv420p[v]",
           "-map", "[v]", "-frames:v", str(n), *ENC, out)


def card(seconds: float, out: Path, color: str = VIOLET):
    n = max(1, round(seconds * FPS))
    ffmpeg("-f", "lavfi", "-i", f"color=c=0x{color}:s={W}x{H}:r={FPS}", "-vf",
           "vignette=PI/4,format=yuv420p", "-frames:v", str(n), *ENC, out)


def concat(parts: list[Path], out: Path):
    lst = out.with_suffix(".txt")
    lst.write_text("".join(f"file '{p}'\n" for p in parts))
    ffmpeg("-f", "concat", "-safe", "0", "-i", lst, "-vf",
           f"scale=out_range=tv,format=yuv420p,setsar=1,fps={FPS}", *ENC, out)


def stills_of(day: str) -> list[dict]:
    """The day's pictures for the time-lapse: in time order, without review
    sheets and before/after crops, a video only without its own still."""
    media = media_of(day)
    names = {m["file"] for m in media}
    out = []
    for m in media:
        f = m["file"]
        if m["kind"] == "audio" or re.search(r"sheet|before|after|crop|cmp|-first", f):
            continue
        stem = f.rsplit(".", 1)[0]
        if m["kind"] == "video" and ({f"{stem}.png", f"{stem}.jpg"} & names):
            continue
        out.append(m)
    return out


def timelapse(day: str, out: Path) -> tuple[float, list[tuple[float, float, dict]]]:
    """Every picture of the day, fast, in time order. Returns the duration
    and (start, end, picture) for the overlay."""
    pics = stills_of(day)
    total = min(26.0, max(10.0, len(pics) * 0.11))
    per = total / max(1, len(pics))
    frames = max(2, round(per * FPS))
    lst = out.with_suffix(".txt")
    rows, t, lines = [], 0.0, []
    for m in pics:
        p = norm_still(HIST / day / m["file"], day)
        d = frames / FPS
        lines.append(f"file '{p}'\nduration {d:.4f}\n")
        rows.append((t, t + d, m))
        t += d
    lines.append(f"file '{norm_still(HIST / day / pics[-1]['file'], day)}'\n")
    lst.write_text("".join(lines))
    ffmpeg("-f", "concat", "-safe", "0", "-i", lst, "-vf",
           f"fps={FPS},eq=saturation=1.15:contrast=1.05,vignette=PI/5,format=yuv420p",
           "-frames:v", str(round(t * FPS)), *ENC, out)
    return t, rows


def mosaic(day: str, seconds: float, out: Path):
    """The end card's background: 24 of the day's pictures, dimmed."""
    pics = stills_of(day)
    pick = [pics[round(i * (len(pics) - 1) / 23)] for i in range(24)]
    inputs, filt = [], []
    for k, m in enumerate(pick):
        inputs += ["-i", norm_still(HIST / day / m["file"], day)]
        filt.append(f"[{k}:v]scale=320:180[t{k}]")
    filt.append("".join(f"[t{k}]" for k in range(24)) + "xstack=inputs=24:layout=" + "|".join(
        f"{(k % 6) * 320}_{(k // 6) * 180}" for k in range(24)) + ",scale=1920:720[m]")
    n = max(1, round(seconds * FPS))
    tile = WORK / day / "mosaic.jpg"
    ffmpeg(*inputs, "-filter_complex", ";".join(filt), "-map", "[m]", "-frames:v", "1", tile)
    ffmpeg("-loop", "1", "-i", tile, "-filter_complex",
           f"[0:v]scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},"
           f"eq=brightness=-0.32:saturation=0.7,boxblur=3:1,"
           f"zoompan=z='1.12-0.1*on/{n}':x='(iw-iw/zoom)/2':y='(ih-ih/zoom)/2':d=1:s={W}x{H}:fps={FPS},"
           f"vignette=PI/4,format=yuv420p[v]",
           "-map", "[v]", "-frames:v", str(n), *ENC, out)


# --------------------------------------------------------------- subtitles


def col(hexrgb: str, alpha: int = 0) -> str:
    r, g, b = hexrgb[0:2], hexrgb[2:4], hexrgb[4:6]
    return f"&H{alpha:02X}{b}{g}{r}".upper()


def num_fmt(v: int) -> str:
    """12345 -> "12 345" (thin space groups)."""
    return f"{v:,}".replace(",", "\u2009")


def ts(t: float) -> str:
    t = max(0.0, t)
    h, rem = divmod(t, 3600)
    m, s = divmod(rem, 60)
    return f"{int(h)}:{int(m):02d}:{s:05.2f}"


def esc(text: str) -> str:
    return text.replace("\\", "\\\\").replace("{", "(").replace("}", ")").replace("\n", "\\N")


def prompt_lines(text: str, width: int = 52, lines: int = 3) -> str:
    """A prompt wrapped for a chat bubble (≤ `lines` lines, ellipsis)."""
    words, out, cur = " ".join(text.split()).split(" "), [], ""
    for w in words:
        if len(cur) + len(w) + 1 > width:
            out.append(cur)
            cur = w
            if len(out) == lines:
                break
        else:
            cur = (cur + " " + w).strip()
    else:
        out.append(cur)
    body = "\\N".join(esc(x) for x in out[:lines])
    return body + ("…" if len(out) >= lines and len(" ".join(out)) < len(" ".join(words)) else "")


def ass_header() -> str:
    def style(name, font, size, fg, bold=0, italic=0, border=1, outline=0.0, shadow=0.0,
              back="000000", back_a=255, outline_c="000000", align=7, spacing=0.0, ml=0, mr=0, mv=0):
        return (f"Style: {name},{font},{size},{col(fg)},{col(fg, 255)},{col(outline_c)},"
                f"{col(back, back_a)},{-1 if bold else 0},{-1 if italic else 0},0,0,100,100,"
                f"{spacing},0,{border},{outline},{shadow},{align},{ml},{mr},{mv},1")

    styles = [
        style("Kicker", "Fira Mono", 32, "FFFFFF", bold=1, spacing=6, border=3, outline=10,
              back=PINK, back_a=0, outline_c=PINK, align=5),
        style("Title", "Lato Black", 168, "FFFFFF", shadow=7, back=VIOLET, back_a=0, align=5),
        style("Date", "Fira Mono", 40, "FFFFFF", spacing=4, align=5),
        style("Tag", "Lato Medium", 54, "FFFFFF", italic=1, shadow=3, back="000000", back_a=96,
              align=5),
        style("ChTime", "Fira Mono", 40, "1A1A1A", bold=1, border=3, outline=10, back=GOLD,
              back_a=0, outline_c=GOLD, align=1),
        style("ChTitle", "Lato Black", 82, "FFFFFF", border=3, outline=16, back=VIOLET,
              back_a=40, outline_c=VIOLET, align=1),
        style("Caption", "Lato Semibold", 50, "FFFFFF", border=3, outline=14, back="101018",
              back_a=70, outline_c="101018", align=2, ml=140, mr=140),
        style("Note", "Lato Heavy", 46, "1A1A1A", border=3, outline=12, back="FFE45C",
              back_a=0, outline_c="FFE45C", align=9),
        style("Badge", "Fira Mono", 38, "FFFFFF", bold=1, border=3, outline=10, back=PINK,
              back_a=0, outline_c=PINK, align=3),
        style("Ticker", "Fira Mono", 26, "C8FFD8", border=3, outline=12, back="0B0B12",
              back_a=60, outline_c="0B0B12", align=3),
        style("TickerHi", "Fira Mono", 26, "FFFFFF", bold=1, border=3, outline=12, back=PINK,
              back_a=0, outline_c=PINK, align=3),
        style("Clock", "Fira Mono", 110, "FFFFFF", bold=1, shadow=5, back="000000", back_a=60,
              align=7),
        style("Counter", "Fira Mono", 36, "FFFFFF", border=3, outline=10, back="0B0B12",
              back_a=60, outline_c="0B0B12", align=7),
        style("StatNum", "Lato Black", 124, PINK, shadow=4, back="000000", back_a=80, align=5),
        style("StatLabel", "Fira Mono", 30, "FFFFFF", spacing=3, align=5),
        style("Outro", "Lato Heavy", 64, "FFFFFF", shadow=4, back="000000", back_a=80, align=5),
        style("Teaser", "Fira Mono", 38, "FFE45C", spacing=2, align=5),
        style("Prompt", "Lato Semibold", 38, "1A1A2E", border=3, outline=16, back="FFFFFF",
              back_a=10, outline_c="FFFFFF", align=7),
        style("PromptWho", "Fira Mono", 28, "FFFFFF", bold=1, border=3, outline=8, back=PINK,
              back_a=0, outline_c=PINK, align=7),
    ]
    return "\n".join([
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {W}", f"PlayResY: {H}",
        "WrapStyle: 0", "ScaledBorderAndShadow: yes", "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, "
        "BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, "
        "BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        *styles, "",
        "[Events]",
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ])


class Subs:
    def __init__(self):
        self.lines = []

    def add(self, start, end, style, text, layer=1):
        if end > start:
            self.lines.append(f"Dialogue: {layer},{ts(start)},{ts(end)},{style},,0,0,0,,{text}")

    def typed(self, start, end, style, text, pre="", cps=40):
        """Typewriter: the text appears a few characters at a time; the rest
        is already laid out but fully transparent (so nothing jumps)."""
        chars = list(text)
        # Keep escapes (\\N, \\h) as single units.
        units, k = [], 0
        while k < len(chars):
            if chars[k] == "\\" and k + 1 < len(chars):
                units.append(chars[k] + chars[k + 1])
                k += 2
            else:
                units.append(chars[k])
                k += 1
        step = 2
        dt_ = step / cps
        t = start
        for n in range(step, len(units) + step, step):
            shown, rest = "".join(units[:n]), "".join(units[n:])
            t_end = end if n >= len(units) else min(end, t + dt_)
            body = shown + (f"{{\\alpha&HFF&}}{rest}" if rest else "")
            # Fades only on the last step (it holds until `end`).
            p = pre if t_end >= end else re.sub(r"\\fad\([^)]*\)", "", pre)
            self.add(t, t_end, style, p + body)
            t = t_end
            if t >= end:
                break

    def write(self, path: Path):
        path.write_text(ass_header() + "\n" + "\n".join(self.lines) + "\n")


# ------------------------------------------------------------------- audio


def env(n, a, r):
    """Attack/release envelope of n samples."""
    out = array("f", [1.0]) * n
    for i in range(min(a, n)):
        out[i] = i / a
    for i in range(min(r, n)):
        out[n - 1 - i] *= i / r
    return out


NOTE = {"C": 0, "C#": 1, "D": 2, "D#": 3, "E": 4, "F": 5, "F#": 6, "G": 7, "G#": 8,
        "A": 9, "A#": 10, "B": 11}


def hz(semitone_from_a4: float) -> float:
    return 440.0 * 2 ** (semitone_from_a4 / 12)


class Music:
    """A small chiptune-pop band, synthesised (no samples, no licences):
    kick, clap, hats, square bass, pulse arpeggio and a soft pad over a
    vi–IV–I–V progression. Bars are rendered once per chord and variant and
    then laid out along the timeline."""

    PROG = [(-3, "m"), (-7, ""), (0, ""), (-5, "")]  # Am F C G relative to C

    def __init__(self, bpm=108, key="C", seed=1):
        self.bpm, self.root, self.rng = bpm, NOTE[key], random.Random(seed)
        self.beat = 60.0 / bpm
        self.bar_n = round(4 * self.beat * RATE)
        self.cache = {}

    def chord(self, idx):
        off, q = self.PROG[idx % 4]
        r = self.root + off - 9  # relative to A4
        third = 3 if q == "m" else 4
        return [r, r + third, r + 7]

    def bar(self, idx, variant):
        key = (idx % 4, variant)
        if key in self.cache:
            return self.cache[key]
        n, b = self.bar_n, self.beat
        out = array("f", [0.0]) * n
        notes = self.chord(idx)
        rng = random.Random(idx % 4 * 31 + hash(variant) % 97)

        def put(at, samples, gain):
            s = round(at * RATE)
            for i, v in enumerate(samples):
                if s + i < n:
                    out[s + i] += v * gain

        drums = variant in ("drive", "groove")
        if drums:
            kick = [math.sin(2 * math.pi * (45 * i / RATE + 70 * (1 - math.exp(-i / 900)) * 0.0133))
                    * math.exp(-i / 5200) for i in range(9000)]
            beats = [0, 1, 2, 3] if variant == "drive" else [0, 1.5, 2.5]
            for k in beats:
                put(k * b, kick, 0.55)
            clap = [rng.uniform(-1, 1) * math.exp(-i / 1800) for i in range(6000)]
            for k in (1, 3):
                put(k * b, clap, 0.22)
            step = 0.5 if variant == "groove" else 0.25
            prev = 0.0
            hat = []
            for i in range(1400):
                x = rng.uniform(-1, 1)
                hat.append((x - prev) * math.exp(-i / 300))
                prev = x
            t = 0.0
            while t < 4:
                put(t * b, hat, 0.10 if (t * 2) % 1 else 0.14)
                t += step
        if variant != "intro" or True:
            # Square bass, 8th notes on the root.
            f = hz(notes[0] - 24)
            for k in range(8):
                ln = round(b / 2 * RATE * 0.9)
                e = env(ln, 60, 900)
                put(k * b / 2, [(1 if math.sin(2 * math.pi * f * i / RATE) > 0 else -1) * e[i] * 0.6
                                + math.sin(2 * math.pi * f * i / RATE) * e[i] * 0.4
                                for i in range(ln)], 0.16 if variant != "intro" else 0.10)
        # Pulse arpeggio, 16ths, two octaves up.
        arp = [notes[0], notes[1], notes[2], notes[1] + 12, notes[2] + 12, notes[0] + 12]
        for k in range(16):
            f = hz(arp[(k + (idx % 2) * 2) % len(arp)] + 12)
            ln = round(b / 4 * RATE * 0.8)
            e = env(ln, 30, 1500)
            per = RATE / f
            put(k * b / 4, [(0.6 if (i % per) < per * 0.25 else -0.4) * e[i] for i in range(ln)],
                0.07 if variant != "outro" else 0.05)
        # Soft pad: three detuned saws through a one-pole low-pass.
        ln = n
        e = env(ln, 9000, 9000)
        lp, prevv = 0.0, 0.0
        pad = []
        fs = [hz(x) for x in notes]
        for i in range(ln):
            v = sum(((i * f * (1 + d) / RATE) % 1.0) * 2 - 1 for f in fs for d in (-0.003, 0.003))
            lp += 0.04 * (v - lp)
            pad.append(lp * e[i] / 6)
        put(0, pad, 0.30 if variant in ("intro", "outro") else 0.18)
        self.cache[key] = out
        return out

    def render(self, seconds, section_at, duck=()):
        n = round(seconds * RATE)
        out = array("f", [0.0]) * n
        bar = 0
        while bar * self.bar_n < n:
            s = bar * self.bar_n
            v = section_at(s / RATE)
            data = self.bar(bar, v)
            for i in range(min(self.bar_n, n - s)):
                out[s + i] += data[i]
            bar += 1
        for a, b, g in duck:
            for i in range(max(0, round(a * RATE)), min(n, round(b * RATE))):
                out[i] *= g
        # Fade in/out.
        for i in range(min(n, RATE)):
            out[i] *= i / RATE
            out[n - 1 - i] *= i / RATE
        return out


def sfx_whoosh(rng):
    n = round(0.7 * RATE)
    out, lp = [], 0.0
    for i in range(n):
        t = i / n
        a = 0.02 + 0.5 * math.sin(math.pi * t) ** 2
        lp += a * (rng.uniform(-1, 1) - lp)
        out.append(lp * math.sin(math.pi * t) * 0.9)
    return out


def sfx_bell():
    """A bicycle bell: two strikes of an inharmonic, decaying ring."""
    n = round(1.6 * RATE)
    parts = [(2300, 1.0, 1.4), (2300 * 2.76, 0.4, 0.7), (2300 * 5.4, 0.2, 0.35), (2310, 0.6, 1.4)]
    one = [sum(a * math.sin(2 * math.pi * f * i / RATE) * math.exp(-i / (d * RATE / 3))
               for f, a, d in parts) * 0.2 for i in range(n)]
    out = [0.0] * (n + round(0.18 * RATE))
    for start in (0, round(0.18 * RATE)):
        for i, v in enumerate(one):
            out[start + i] += v
    return out


def sfx_pop():
    n = round(0.12 * RATE)
    return [math.sin(2 * math.pi * (900 - 500 * i / n) * i / RATE) * (1 - i / n) * 0.35
            for i in range(n)]


def sfx_tick():
    n = round(0.025 * RATE)
    return [math.sin(2 * math.pi * 1800 * i / RATE) * (1 - i / n) * 0.18 for i in range(n)]


def sfx_ding():
    n = round(1.2 * RATE)
    return [(math.sin(2 * math.pi * 1318.5 * i / RATE) + 0.5 * math.sin(2 * math.pi * 1975.5 * i / RATE))
            * math.exp(-i / (0.35 * RATE)) * 0.25 for i in range(n)]


def write_wav(path: Path, data):
    peak = max(1e-9, max(abs(x) for x in data))
    g = min(1.0, 0.89 / peak)
    pcm = array("h", (int(max(-1, min(1, x * g)) * 32767) for x in data))
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm.tobytes())


# ------------------------------------------------------------------ render


def load_script(day: str) -> dict:
    p = DAYS / f"{day}.toml"
    if not p.exists():
        raise SystemExit(f"no script for {day}: write art/days/{day}.toml (see art/ART-PROJECT.md)")
    return tomllib.loads(p.read_text())


def render(day: str) -> Path:
    sc = load_script(day)
    j = journal(day)
    work = WORK / day / "render"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    num = j["number"]
    date = dt.date.fromisoformat(day)
    segs: list[tuple[Path, float, str]] = []  # (file, seconds, music section)

    # 1. Title.
    title_s = float(sc.get("title_seconds", 6.5))
    t1 = work / "00-title.mp4"
    kenburns(media_path(day, sc["hero"]), title_s, t1, day)
    segs.append((t1, title_s, "intro"))

    # 2. Time-lapse of the whole day.
    t2 = work / "01-timelapse.mp4"
    tl_s, tl_rows = timelapse(day, t2)
    segs.append((t2, tl_s, "drive"))

    # 3. Chapters.
    chapters = sc.get("chapter", [])
    ch_shots = []
    for ci, ch in enumerate(chapters):
        parts, shots = [], []
        for si, shot in enumerate(ch["shots"]):
            out = work / f"ch{ci:02d}-{si:02d}.mp4"
            secs = float(shot.get("seconds", 3.5))
            if "split" in shot:
                a, b = shot["split"]
                split(media_path(day, a), media_path(day, b), secs, out, day)
            elif "card" in shot:
                card(secs, out, shot.get("color", VIOLET))
            else:
                src = media_path(day, shot["file"])
                if src.suffix.lower() in VIDEO:
                    clip(src, float(shot.get("start", 0)), secs, float(shot.get("speed", 1)), out)
                else:
                    kenburns(src, secs, out, day)
            parts.append(out)
            shots.append((secs, shot))
        seg = work / f"ch{ci:02d}.mp4"
        concat(parts, seg)
        segs.append((seg, sum(s for s, _ in shots), "groove"))
        ch_shots.append(shots)

    # 4. End card.
    end_s = float(sc.get("end_seconds", 9))
    t4 = work / "99-end.mp4"
    mosaic(day, end_s, t4)
    segs.append((t4, end_s, "outro"))

    # Timeline: segment i starts at start[i]; xfades overlap by XF.
    starts, t = [], 0.0
    for _, d, _ in segs:
        starts.append(t)
        t += d - XF
    total = t + XF

    # ---- overlay text
    sub = Subs()
    # Title.
    sub.add(0.4, title_s - 0.3, "Kicker",
            "{\\fad(300,300)\\pos(960,300)}OCW BUILD DIARY")
    sub.add(0.7, title_s - 0.3, "Title",
            f"{{\\fad(150,300)\\pos(960,450)\\fscx150\\fscy150\\t(0,450,\\fscx100\\fscy100)}}DAY {num}")
    sub.add(1.2, title_s - 0.3, "Date",
            f"{{\\fad(300,300)\\pos(960,590)}}{date.strftime('%A, %d %B %Y').upper()}")
    sub.typed(1.9, title_s - 0.2, "Tag", esc(sc["tagline"]), pre="{\\pos(960,720)\\fad(0,300)}")
    if sc.get("title"):
        sub.add(1.5, title_s - 0.3, "Caption",
                f"{{\\fad(300,300)\\pos(960,880)}}{esc(sc['title'])}")

    # Time-lapse: clock, counters and the commit ticker.
    t0 = starts[1]
    commits = j["commits"]
    sub.add(t0 + 0.2, t0 + tl_s - 0.2, "Kicker",
            "{\\an9\\pos(1860,40)\\fad(200,200)}THE DAY IN " + f"{round(tl_s)}" + " SECONDS")
    for k, (a, b, m) in enumerate(tl_rows):
        hhmm = m["time"][:5]
        done = [c for c in commits if c["time"] <= m["time"]]
        sub.add(t0 + a, t0 + b, "Clock", f"{{\\an9\\pos(1860,84)}}{hhmm}")
        sub.add(t0 + a, t0 + b, "Counter",
                f"{{\\an9\\pos(1856,222)}}commits {len(done):3d}  ·  merges "
                f"{sum(c['merge'] for c in done):2d}  ·  pictures {k + 1:3d}")
        last = done[-6:]
        y = 1040
        for i, c in enumerate(reversed(last)):
            txt = esc(f"{c['time'][:5]} {c['sha']} {c['subject'][:64]}")
            style = "TickerHi" if i == 0 and c is done[-1] and (
                k == 0 or c["time"] > tl_rows[k - 1][2]["time"]) else "Ticker"
            alpha = min(200, i * 34)
            sub.add(t0 + a, t0 + b, style, f"{{\\pos(1880,{y})\\alpha&H{alpha:02X}&}}{txt}")
            y -= 46

    # Prompts in the time-lapse: each one as a chat bubble when its time comes.
    ps = j.get("prompts") or []
    shown, bubbles = set(), []
    for k, (a, b, m) in enumerate(tl_rows):
        due = [p_ for p_ in ps if p_["time"] <= m["time"] and p_["time"] not in shown]
        if not due or (bubbles and a - bubbles[-1] < 1.2):
            continue  # at most one new bubble per 1.2 s; the newest prompt wins
        p_ = due[-1]
        shown.update(x["time"] for x in due)
        bubbles.append(a)
        nxt = next((r[0] for r in tl_rows[k + 1:] if r[0] - a >= 1.2), tl_s)
        end = min(t0 + tl_s - 0.2, t0 + a + 2.2, t0 + nxt)
        txt = prompt_lines(p_["text"])
        sub.add(t0 + a, end, "PromptWho", f"{{\\pos(60,330)\\fad(80,150)}}YOU · {p_['time'][:5]}")
        sub.add(t0 + a, end, "Prompt", f"{{\\pos(60,380)\\fad(80,150)}}{txt}")

    # Chapters.
    sfx_events = [(0.25, "bell")]
    duck = []
    for ci, ch in enumerate(chapters):
        cs = starts[2 + ci]
        cd = segs[2 + ci][1]
        sub.add(cs + 0.25, cs + min(cd, 3.6), "ChTime",
                f"{{\\move(-300,700,80,700,0,260)\\fad(0,250)}}{esc(ch.get('time', ''))}")
        sub.add(cs + 0.35, cs + min(cd, 3.6), "ChTitle",
                f"{{\\move(-1400,830,80,830,40,330)\\fad(0,250)}}{esc(ch['title'])}")
        if ch.get("prompt"):
            ptxt = prompt_lines(mask(ch["prompt"]))
            sub.add(cs + 0.4, cs + min(cd, 4.5), "PromptWho", "{\\pos(60,300)\\fad(150,200)}YOU ASKED")
            sub.add(cs + 0.4, cs + min(cd, 4.5), "Prompt", f"{{\\pos(60,350)\\fad(150,200)}}{ptxt}")
        if ch.get("caption"):
            sub.typed(cs + min(cd - 1.0, 2.4), cs + cd - 0.25, "Caption", esc(ch["caption"]),
                      pre="{\\fad(0,200)}")
        tt = cs
        for secs, shot in ch_shots[ci]:
            if shot.get("note"):
                sub.add(tt + 0.5, tt + secs - 0.15, "Note",
                        f"{{\\pos(1860,70)\\frz{seeded(shot['note']).uniform(-5, 5):.1f}"
                        f"\\fscx40\\fscy40\\t(0,180,\\fscx112\\fscy112)\\t(180,280,\\fscx100\\fscy100)"
                        f"\\fad(0,150)}}{esc(shot['note'])}")
                sfx_events.append((tt + 0.5, "pop"))
            spd = float(shot.get("speed", 1))
            if spd > 1:
                sub.add(tt + 0.2, tt + secs - 0.1, "Badge",
                        f"{{\\pos(1880,1040)}}▶▶ TIME-LAPSE ×{spd:g}")
            tt += secs
        if ch.get("sound"):
            duck.append((cs, cs + cd, 0.35))
        sfx_events.append((cs - XF / 2, "whoosh"))
    sfx_events.append((starts[1] - XF / 2, "whoosh"))
    sfx_events.append((starts[-1] - XF / 2, "whoosh"))

    # End card: count-up stats.
    es = starts[-1]
    stats = [
        (j["commits"].__len__(), "COMMITS"),
        (j["merges"], "MERGES"),
        (j["lines_added"], "LINES WRITTEN"),
        (j["images"] + j["videos"], "PICTURES & VIDEOS"),
    ]
    if sc.get("tests") or j["tests"]:
        stats.append((int(sc.get("tests") or j["tests"]), "TESTS PASSING"))
    if j.get("prompts"):
        stats.append((len(j["prompts"]), "PROMPTS"))
    if (j.get("tokens") or {}).get("total", {}).get("total"):
        stats.append((round(j["tokens"]["total"]["total"] / 1e6), "MILLION TOKENS"))
    b = j.get("builds") or {}
    if b.get("runs"):
        stats.append((b["runs"], "BUILD RUNS"))
        stats.append((round(b["seconds"] / 3600), "BUILD HOURS"))
    # One row for up to five numbers, else two rows with smaller numbers.
    rows = [stats] if len(stats) <= 5 else [stats[: (len(stats) + 1) // 2],
                                            stats[(len(stats) + 1) // 2 :]]
    pos = []  # (x, y of the number, y of the label, scale %)
    for r, row in enumerate(rows):
        step = min(360, 1760 / max(1, len(row)))
        y, sc_ = (380, 100) if len(rows) == 1 else (300 + r * 190, 78)
        pos += [(960 + (i - (len(row) - 1) / 2) * step, y, y + 72 * sc_ / 100 + 18, sc_)
                for i in range(len(row))]
    outro_y = 650 if len(rows) == 1 else 700
    for i, (val, label) in enumerate(stats):
        x, y, ly, scl = pos[i]
        fs = f"\\fscx{scl}\\fscy{scl}"
        a = es + 0.5 + i * 0.25
        steps = 24
        for k in range(steps):
            v = round(val * ((k + 1) / steps) ** 0.6)
            sub.add(a + k / FPS * 1.4, a + (k + 1) / FPS * 1.4, "StatNum",
                    f"{{\\pos({x:.0f},{y}){fs}}}{num_fmt(v)}")
            if k % 3 == 0:
                sfx_events.append((a + k / FPS * 1.4, "tick"))
        sub.add(a + steps / FPS * 1.4, es + end_s - 0.2, "StatNum",
                f"{{\\pos({x:.0f},{y}){fs}\\t(0,120,\\fscx{scl * 1.15:.0f}\\fscy{scl * 1.15:.0f})"
                f"\\t(120,260,\\fscx{scl}\\fscy{scl})\\fad(0,300)}}"
                f"{num_fmt(val)}")
        sub.add(a, es + end_s - 0.2, "StatLabel",
                f"{{\\pos({x:.0f},{ly:.0f})\\fad(200,300)}}{label}")
    sfx_events.append((es + 0.5 + len(stats) * 0.25 + 1.4, "ding"))
    sub.typed(es + 3.2, es + end_s - 0.2, "Outro", esc(sc["outro"]), pre=f"{{\\pos(960,{outro_y})\\fad(0,300)}}")
    if sc.get("teaser"):
        sub.add(es + 5.2, es + end_s - 0.1, "Teaser",
                f"{{\\pos(960,800)\\fad(400,300)}}{esc(sc['teaser'])}")
    sub.add(es + 0.3, es + end_s - 0.1, "Kicker",
            f"{{\\pos(960,170)\\fad(300,300)}}DAY {num}  ·  {date.strftime('%d.%m.%Y')}  ·  "
            "OPEN CYCLING WORLD")
    ass = work / "overlay.ass"
    sub.write(ass)

    # ---- soundtrack
    def section_at(t):
        for i in range(len(segs) - 1, -1, -1):
            if t >= starts[i]:
                return segs[i][2]
        return "intro"

    music = Music(bpm=int(sc.get("bpm", 108)), key=sc.get("key", "C"), seed=num).render(
        total, section_at, duck)
    rng = random.Random(num)
    fx = {"bell": sfx_bell(), "pop": sfx_pop(), "tick": sfx_tick(), "ding": sfx_ding()}
    for at, kind in sfx_events:
        data = sfx_whoosh(rng) if kind == "whoosh" else fx[kind]
        s = round(max(0.0, at) * RATE)
        for i, v in enumerate(data):
            if s + i < len(music):
                music[s + i] += v * (0.5 if kind == "whoosh" else 1.0)
    track = work / "soundtrack.wav"
    write_wav(track, music)

    # ---- assemble: xfade chain + overlay + audio, one encode
    inputs, filt = [], []
    for p, _, _ in segs:
        inputs += ["-i", p]
    for i in range(len(segs)):
        filt.append(f"[{i}:v]scale=out_range=tv,format=yuv420p,setsar=1,fps={FPS},"
                    f"settb=1/{FPS * 512}[n{i}]")
    prev = "[n0]"
    for i in range(1, len(segs)):
        off = starts[i]
        tr = TRANSITIONS[(i + num) % len(TRANSITIONS)]
        filt.append(f"{prev}[n{i}]xfade=transition={tr}:duration={XF}:offset={off:.3f}[x{i}]")
        prev = f"[x{i}]"
    filt.append(f"{prev}ass={ass}:fontsdir={FONTS},format=yuv420p[v]")
    audio_in = len(segs)
    inputs += ["-i", track]
    amix = [f"[{audio_in}:a]volume=1.0[m]"]
    mix_labels = ["[m]"]
    k = audio_in + 1
    for ci, ch in enumerate(chapters):
        if ch.get("sound"):
            src = media_path(day, ch["sound"])
            cd = segs[2 + ci][1]
            inputs += ["-i", src]
            ms = round(starts[2 + ci] * 1000)
            amix.append(f"[{k}:a]aresample={RATE},atrim=0:{cd:.2f},afade=t=in:d=0.3,"
                        f"afade=t=out:st={max(0, cd - 0.6):.2f}:d=0.6,"
                        f"volume={float(ch.get('sound_gain', 1.0))},adelay={ms}|{ms}[s{ci}]")
            mix_labels.append(f"[s{ci}]")
            k += 1
    amix.append("".join(mix_labels) + f"amix=inputs={len(mix_labels)}:normalize=0,"
                "alimiter=limit=0.95[a]")
    OUT.mkdir(parents=True, exist_ok=True)
    out = OUT / f"ocw-diary-day-{num:02d}-{day}.mp4"
    ffmpeg(*inputs, "-filter_complex", ";".join(filt + amix), "-map", "[v]", "-map", "[a]",
           "-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p",
           "-r", str(FPS), "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2",
           "-t", f"{total:.3f}", *now_meta(), "-metadata",
           f"title=OCW Build Diary – day {num} ({day})", "-movflags", "+faststart", out)
    log(f"{day}: {out} ({total:.1f} s)")
    return out


def diary() -> Path:
    """The diary: a short opener, then every day video in order."""
    days = [p for p in sorted(OUT.glob("ocw-diary-day-*.mp4"))]
    if not days:
        raise SystemExit("no day videos yet: art/diary.py render")
    work = WORK / "diary"
    work.mkdir(parents=True, exist_ok=True)
    opener = work / "opener.mp4"
    sec = 5.0
    sub = Subs()
    sub.add(0.3, sec - 0.2, "Kicker", "{\\pos(960,330)\\fad(300,300)}AN ART PROJECT")
    sub.add(0.6, sec - 0.2, "Title",
            "{\\pos(960,480)\\fad(200,300)\\fscx120\\fscy120\\t(0,500,\\fscx100\\fscy100)}"
            "The Build Diary")
    sub.typed(1.4, sec - 0.2, "Tag",
              esc(f"How Open Cycling World was built, one day at a time · {len(days)} "
                  f"day{'s' if len(days) != 1 else ''} so far"), pre="{\\pos(960,640)\\fad(0,300)}")
    ass = work / "opener.ass"
    sub.write(ass)
    logo = ROOT / "artwork" / "logos" / "ocw-512.png"
    ffmpeg("-f", "lavfi", "-i", f"color=c=0x{VIOLET}:s={W}x{H}:r={FPS}", "-loop", "1", "-i", logo,
           "-f", "lavfi", "-i", f"anullsrc=r=48000:cl=stereo",
           "-filter_complex",
           f"[1:v]scale=230:-1[l];[0:v][l]overlay=(W-w)/2:40,vignette=PI/4,"
           f"ass={ass}:fontsdir={FONTS},format=yuv420p[v]",
           "-map", "[v]", "-map", "2:a", "-t", f"{sec}", "-c:v", "libx264", "-preset", "medium",
           "-crf", "20", "-pix_fmt", "yuv420p", "-r", str(FPS), "-c:a", "aac", "-b:a", "192k",
           "-ar", "48000", "-ac", "2", opener)
    bell = work / "bell.wav"
    write_wav(bell, [0.0] * round(0.3 * RATE) + sfx_bell())
    opener2 = work / "opener-bell.mp4"
    ffmpeg("-i", opener, "-i", bell, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac",
           "-b:a", "192k", "-ar", "48000", "-ac", "2", "-af", "apad", "-t", f"{sec}", opener2)
    out = OUT / "ocw-build-diary.mp4"
    lst = work / "diary.txt"
    lst.write_text("".join(f"file '{p}'\n" for p in [opener2, *days]))
    ffmpeg("-f", "concat", "-safe", "0", "-i", lst, "-c", "copy", *now_meta(),
           "-metadata", "title=OCW Build Diary", "-movflags", "+faststart", out)
    log(f"diary: {out} ({probe_duration(out):.1f} s, {len(days)} days)")
    return out


def masked_copy(day: str) -> Path:
    """A copy of the day folder for the public zip: text files (logs, build
    log, journal, prompts) masked like the prompts (§2.3: build machines'
    hosts and users, the home directory, e-mail addresses); media are
    hard-linked. The archive itself is left as it was."""
    secrets = _secrets()
    src = HIST / day
    stage = WORK / "publish"
    dest = stage / day
    if dest.exists():
        shutil.rmtree(dest)
    for f in sorted(src.rglob("*")):
        if not f.is_file():
            continue
        t = dest / f.relative_to(src)
        t.parent.mkdir(parents=True, exist_ok=True)
        if f.suffix.lower() in TEXT | LOGS:
            t.write_text(mask(f.read_text(errors="replace"), secrets))
            shutil.copystat(f, t)
        else:
            try:
                os.link(f, t)
            except OSError:
                shutil.copy2(f, t)
    return stage


def publish(days: list[str]):
    files = []
    for d in days:
        folder = HIST / d
        if not folder.exists():
            continue
        z = HIST / f"ocw-build-history-{d}.zip"
        if z.exists():
            z.unlink()
        stage = masked_copy(d)
        run(["zip", "-q", "-r", z, d], cwd=stage)
        shutil.rmtree(stage / d)
        files.append(z)
    files += sorted(OUT.glob("*.mp4"))
    # Checksums of every zip and video known locally (all days, not only these).
    sums = HIST / "SHA256SUMS"
    sums.write_text("".join(f"{sha(f)}  {f.name}\n" for f in
                            [*sorted(HIST.glob("ocw-build-history-*.zip")), *sorted(OUT.glob("*.mp4"))]))
    files.append(sums)
    for f in files:
        log(f"upload {f.name} ({f.stat().st_size / 1e6:.1f} MB)")
    run(["gh", "release", "upload", RELEASE, "-R", "newkit/OCW", "--clobber", *files])


def main():
    if len(sys.argv) < 2 or sys.argv[1] in {"-h", "--help", "help"}:
        print(__doc__)
        return
    cmd, days = sys.argv[1], sys.argv[2:]
    if cmd == "diary":
        diary()
        return
    if not days:
        days = all_days() if cmd in {"fetch", "gather", "journal", "publish", "stamp", "prompts", "tokens"} else days_with_scripts()
    for d in days:
        if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", d):
            raise SystemExit(f"not a day: {d}")
    if cmd == "all":
        for d in days:
            gather(d)
            journal(d)
            render(d)
        diary()
        return
    fn = {"fetch": fetch, "gather": gather, "journal": journal, "sheet": sheet,
          "render": render, "stamp": stamp, "prompts": prompts,
          "tokens": tokens}.get(cmd)
    if cmd == "publish":
        publish(days)
    elif fn:
        for d in days:
            fn(d)
    else:
        raise SystemExit(f"unknown command {cmd}; see art/diary.py --help")


if __name__ == "__main__":
    main()
