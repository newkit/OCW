#!/usr/bin/env python3
"""The README's browser walkthrough as a test (`make test-web-readme`;
spec §11.4, §14 Phase 10): a new user following "Browser app (preview)".

    scripts/test-web-readme.py [--out test-output/web] [--seconds 12] [--keep]
                               [--reuse-target] [--chromium PATH] [--readme README.md]

1. A fresh copy of the current commit in target/readme-walkthrough/OCW
   (`git archive HEAD` in a git checkout; in a copy without .git, as
   scripts/remote.sh sends one, the files git would not ignore), with a
   git repository of its own.
2. `make web-serve` there before any build must fail and say what to run.
3. The README's own commands, in its order (the `make …` lines of its bash
   blocks in steps 2 – 5): `make deps-web`, `make web`, `make web-worlds`,
   `make web-serve` (on a free port: WEB_PORT, which the README documents).
   The one step not taken literally: the world package. A new user builds
   it with scripts/fetch-pilot.sh and scripts/build-liechtenstein.sh (the
   README's "Routes and worlds"); the test links the build machine's
   data/pilot/liechtenstein.ocwpkg into the copy instead (the verdict says
   so).
4. Every URL the section shows, in the downloaded browser
   (scripts/web_browser.py): steps 3 – 5 with the real key of
   target/web-keys/root.hex in place of the placeholder and the example
   key, the URLs the README's one-liner prints, and the links
   `make web-serve` prints. Multiplayer (`server=`) needs a game server
   and is left to `make test-web-multiplayer`.
   Each page: index.html with HTTP 200, the app starts, the ride ticks and
   moves, the expected GPU build, no HTTP error in the network log or the
   server's log, no uncaught exception. One screenshot per URL.

Writes OUT/readme-<time>/: NN-<slug>.jpg per URL (world pages also
-first-loading.jpg), make-*.log, serve.log, browser.log and verdict.json.
Exits 0 when every step and every URL passed.
"""

import argparse
import datetime
import importlib.util
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import web_browser  # noqa: E402

SECTION = "### Browser app (preview)"
README_PORT = 8080
PLACEHOLDER = re.compile(r"<[^<>]*root\.hex[^<>]*>")
HEX64 = re.compile(r"(trust-root=)[0-9a-f]{64}")
URL = re.compile(r"https?://(?:127\.0\.0\.1|localhost):8080/[^\s`<>\"')\]]*")


def test_web():
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# --- The README -----------------------------------------------------------

def section(text):
    """The "Browser app (preview)" section (up to the next ### heading)."""
    i = text.index(SECTION)
    j = text.find("\n### ", i + len(SECTION))
    return text[i:j if j >= 0 else len(text)]


def blocks(sec):
    """[(lang, body)] of the fenced code blocks, in order."""
    return [(m.group(1), m.group(2))
            for m in re.finditer(r"^ *```(\w*)\n(.*?)^ *```", sec, re.S | re.M)]


def heading_of(sec, pos):
    """The #### heading above `pos` ("4. Ride solo")."""
    hs = [m for m in re.finditer(r"^#### (.*)$", sec, re.M) if m.start() < pos]
    return hs[-1].group(1).strip() if hs else "intro"


def make_steps(sec):
    """The `make …` commands of the bash blocks of steps 2 – 5, in order,
    without their comments (the README's own build and serve steps)."""
    out = []
    for m in re.finditer(r"^ *```bash\n(.*?)^ *```", sec, re.S | re.M):
        h = heading_of(sec, m.start())
        if not re.match(r"[2-5]\.", h):
            continue
        for line in m.group(1).splitlines():
            cmd = line.split("#", 1)[0].strip()
            # `make web-remote` is the way for a machine without a local
            # build (it runs make web on a build machine, where this runs).
            if cmd.startswith("make ") and not cmd.startswith("make web-remote") and cmd not in out:
                out.append(cmd)
    return out


def oneliner(sec):
    """The bash block that prints the URLs with the machine's key."""
    for lang, body in blocks(sec):
        if lang == "bash" and "target/web-keys/root.hex" in body and "echo" in body:
            return body
    return None


def urls(sec):
    """Every URL of the section the walkthrough opens or skips:
    [{"source", "readme", "skip"?}], in order, each once. Lines of text
    blocks are whole URLs (with placeholders); elsewhere the URLs in the
    prose and comments."""
    out, seen = [], set()

    def add(url, pos, skip=None):
        if url in seen:
            return
        seen.add(url)
        e = {"source": heading_of(sec, pos), "readme": url}
        if skip:
            e["skip"] = skip
        out.append(e)

    taken = []
    for m in re.finditer(r"^ *```(\w*)\n(.*?)^ *```", sec, re.S | re.M):
        taken.append((m.start(), m.end()))
        if m.group(1) == "text":
            for line in m.group(2).splitlines():
                line = line.strip()
                if not line.startswith("http"):
                    continue
                if "server=" in line or "…" in line:
                    add(line, m.start(), "needs a running ocw-server and its certificate hash "
                                         "(make test-web-multiplayer)")
                else:
                    add(line, m.start())
        else:
            for u in URL.findall(m.group(2)):
                # Shell expressions ($KEY): the walkthrough runs that block
                # and opens what it prints (oneliner).
                if "$" not in u:
                    add(u, m.start())
    for m in URL.finditer(sec):
        if any(a <= m.start() < b for a, b in taken):
            continue
        add(m.group(0), m.start())
    return out


def concrete(url, key, port):
    """The URL to open: the real key for the placeholder and the README's
    example key, and the walkthrough's port."""
    url = PLACEHOLDER.sub(key, url)
    url = HEX64.sub(lambda m: m.group(1) + key, url)
    return url.replace(":8080/", f":{port}/")


def expects(url):
    """What a page must show: a world? which GPU build?"""
    q = url.split("?", 1)[1] if "?" in url else ""
    opts = dict(kv.split("=", 1) for kv in q.split("&") if "=" in kv)
    return {"world": "world" in opts, "gpu": "webgl2" if opts.get("gpu") == "webgl2" else "webgpu"}


def slug(url):
    q = url.split("?", 1)[1] if "?" in url else "root"
    q = re.sub(r"trust-root=[0-9a-f]+", "key", q)
    s = re.sub(r"[^A-Za-z0-9]+", "-", q).strip("-").lower()
    return (s or "root")[:60]


def http_errors(events, since=0):
    """[(status, url)] of HTTP errors and failed loads in the CDP events."""
    bad = []
    for e in events[since:]:
        if e["method"] == "Network.responseReceived":
            r = e["params"]["response"]
            if r.get("status", 0) >= 400:
                bad.append((r["status"], r["url"]))
        elif e["method"] == "Network.loadingFailed":
            p = e["params"]
            if not p.get("canceled") and p.get("errorText") != "net::ERR_ABORTED":
                bad.append((p.get("errorText"), p.get("requestId")))
    return bad


def document_status(events, since=0):
    for e in events[since:]:
        if e["method"] == "Network.responseReceived" and e["params"].get("type") == "Document":
            return e["params"]["response"].get("status")
    return None


def server_errors(log_text):
    """HTTP status >= 400 in web-serve.py's request log."""
    return [m.group(0) for m in re.finditer(r'"[A-Z]+ [^"]*" (4\d\d|5\d\d)', log_text)]


# --- The walkthrough ------------------------------------------------------

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run(cmd, cwd, log_path, timeout=None, env=None):
    t0 = time.time()
    with open(log_path, "w") as log:
        log.write(f"$ {cmd}\n")
        log.flush()
        try:
            p = subprocess.run(cmd, shell=True, cwd=cwd, stdout=log, stderr=subprocess.STDOUT,
                               timeout=timeout, env=env)
            code = p.returncode
        except subprocess.TimeoutExpired:
            code = "timeout"
    return {"cmd": cmd, "ok": code == 0, "exit": code, "seconds": round(time.time() - t0, 1),
            "log": log_path}


def fresh_copy(src, dest):
    """`dest` as a fresh checkout of `src`'s current commit; returns how."""
    os.makedirs(dest)
    top = subprocess.run(["git", "-C", src, "rev-parse", "--show-toplevel"],
                         capture_output=True, text=True)
    if top.returncode == 0 and os.path.realpath(top.stdout.strip()) == os.path.realpath(src):
        commit = subprocess.run(["git", "-C", src, "rev-parse", "HEAD"], capture_output=True,
                                text=True, check=True).stdout.strip()
        subprocess.run(f"git -C '{src}' archive HEAD | tar -x -C '{dest}'", shell=True, check=True)
        how = f"git archive {commit}"
    else:
        # A copy without .git (scripts/remote.sh): the files git would not
        # ignore, listed by a throwaway repository over the tree.
        tmp = tempfile.mkdtemp(prefix="ocw-ls-")
        try:
            subprocess.run(["git", "init", "-q", tmp], check=True)
            files = subprocess.run(
                ["git", f"--git-dir={tmp}/.git", f"--work-tree={src}", "ls-files", "-o",
                 "--exclude-standard", "-z"], capture_output=True, check=True).stdout
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
        names = [n for n in files.decode().split("\0") if n and not n.startswith("config/")
                 or n in ("config/README.md",) or (n.startswith("config/") and ".example." in n)]
        for n in names:
            os.makedirs(os.path.join(dest, os.path.dirname(n)), exist_ok=True)
            shutil.copy2(os.path.join(src, n), os.path.join(dest, n), follow_symlinks=False)
        commit = os.environ.get("OCW_BUILD_ID", "unknown")
        how = f"the files of the checkout git does not ignore ({len(names)}; build {commit})"
    env = dict(os.environ, GIT_AUTHOR_NAME="walkthrough", GIT_AUTHOR_EMAIL="walkthrough@localhost",
               GIT_COMMITTER_NAME="walkthrough", GIT_COMMITTER_EMAIL="walkthrough@localhost")
    subprocess.run("git init -q && git add -A && git commit -q -m walkthrough", shell=True,
                   cwd=dest, check=True, env=env)
    return how


def world_package(src):
    """The build machine's Liechtenstein package (as the Makefile finds it)."""
    for c in ("data/pilot/liechtenstein.ocwpkg", "../OCW/data/pilot/liechtenstein.ocwpkg",
              "/var/tmp/ocw/liechtenstein.ocwpkg"):
        p = os.path.normpath(os.path.join(src, c))
        if os.path.isfile(p):
            return p
    return None


def check_serve_without_build(clone, port, log_path):
    """`make web-serve` with no target/web/: must exit non-zero, soon, and
    name the commands."""
    r = run(f"make web-serve WEB_PORT={port}", clone, log_path, timeout=60)
    text = open(log_path).read()
    r["message_ok"] = "make web" in text and "make web-remote" in text
    r["ok"] = r["exit"] not in (0, "timeout") and r["message_ok"]
    r["output"] = text.strip().splitlines()[-6:]
    return r


def start_serve(clone, port, log_path, bind=None):
    env = dict(os.environ, OCW_WEB_SERVE_LOG="1", PYTHONUNBUFFERED="1")
    log = open(log_path, "w")
    args = ["make", "web-serve", f"WEB_PORT={port}"] + ([f"WEB_BIND={bind}"] if bind else [])
    proc = subprocess.Popen(args, cwd=clone, env=env,
                            stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    printed = []
    deadline = time.time() + 30
    while time.time() < deadline:
        time.sleep(0.3)
        text = open(log_path).read()
        if "serving " in text:
            time.sleep(0.5)
            text = open(log_path).read()
            printed = re.findall(r"world \S+: (http://\S+)", text)
            if bind:
                return proc, log, {"text": text, "lan": re.findall(r"^\s+(http://\S+)$", text, re.M)}
            return proc, log, printed
        if proc.poll() is not None:
            break
    return proc, log, None


def stop(proc):
    if proc and proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--readme", default=os.path.join(ROOT, "README.md"))
    p.add_argument("--seconds", type=float, default=12.0,
                   help="riding per page after the first tick (the app logs its distance every few seconds)")
    p.add_argument("--timeout", type=float, default=150.0, help="per page")
    p.add_argument("--work", default=os.path.join(ROOT, "target", "readme-walkthrough"))
    p.add_argument("--keep", action="store_true", help="keep the copy (and its target/)")
    p.add_argument("--reuse-target", action="store_true",
                   help="move the previous copy's target/ into the fresh copy (a quicker rerun;"
                        " the sources are still fresh)")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()

    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = os.path.abspath(os.path.join(a.out, f"readme-{stamp}"))
    os.makedirs(out)
    sec = section(open(a.readme).read())
    verdict = {"ok": False, "started": stamp, "browser": web_browser.describe(a.chromium),
               "steps": [], "urls": [], "deviations": []}

    # The machine's LAN addresses stay out of the test output (it is
    # archived and published): they are replaced by LAN-ADDRESS.
    lan_hosts = []

    def redact(text):
        for h in lan_hosts:
            text = text.replace(h, "LAN-ADDRESS")
        return text

    def save():
        with open(os.path.join(out, "verdict.json"), "w") as f:
            f.write(redact(json.dumps(verdict, indent=2)))

    clone = os.path.join(a.work, "OCW")
    old_target = None
    if a.reuse_target and os.path.isdir(os.path.join(clone, "target")):
        old_target = os.path.join(a.work, "target.prev")
        shutil.rmtree(old_target, ignore_errors=True)
        os.rename(os.path.join(clone, "target"), old_target)
    shutil.rmtree(clone, ignore_errors=True)
    verdict["copy"] = fresh_copy(ROOT, clone)
    print(f"readme walkthrough in {clone} ({verdict['copy']})", flush=True)

    # Before any build (a kept target/ comes back afterwards).
    port = free_port()
    verdict["serve_without_build"] = check_serve_without_build(
        clone, port, os.path.join(out, "make-web-serve-without-build.log"))
    if old_target:
        os.rename(old_target, os.path.join(clone, "target"))
        verdict["deviations"].append("--reuse-target: the previous copy's target/ was kept")
    print(f"web-serve without a build: {'ok' if verdict['serve_without_build']['ok'] else 'FAIL'}",
          flush=True)

    steps = make_steps(sec)
    verdict["readme_steps"] = steps
    pkg = world_package(ROOT)
    serve_proc = serve_log = lan_proc = lan_log = None
    ok = True  # the build and serve steps; the check above counts in the verdict
    try:
        for cmd in steps:
            if cmd == "make web-serve":
                continue
            if cmd.startswith("make web-worlds") and not os.path.exists(
                    os.path.join(clone, "data/pilot/liechtenstein.ocwpkg")):
                if not pkg:
                    verdict["steps"].append({"cmd": cmd, "ok": False,
                                             "exit": "no data/pilot/liechtenstein.ocwpkg on this machine"})
                    ok = False
                    break
                os.makedirs(os.path.join(clone, "data/pilot"), exist_ok=True)
                for suffix in ("", ".manifest.json"):
                    if os.path.exists(pkg + suffix):
                        os.symlink(pkg + suffix, os.path.join(clone, "data/pilot",
                                                              "liechtenstein.ocwpkg" + suffix))
                verdict["deviations"].append(
                    f"world package linked from {os.path.relpath(pkg, ROOT)} instead of "
                    "scripts/fetch-pilot.sh + scripts/build-liechtenstein.sh")
            name = re.sub(r"[^a-z0-9]+", "-", cmd.lower()).strip("-")
            print(f"$ {cmd}", flush=True)
            r = run(cmd, clone, os.path.join(out, f"{name}.log"))
            verdict["steps"].append(r)
            print(f"  {'ok' if r['ok'] else 'FAIL'} in {r['seconds']} s", flush=True)
            save()
            if not r["ok"]:
                ok = False
                break
        if "make web-serve" not in steps:
            verdict["steps"].append({"cmd": "make web-serve", "ok": False,
                                     "exit": "not in the README's steps"})
            ok = False
        key_file = os.path.join(clone, "target/web-keys/root.hex")
        key = open(key_file).read().strip() if os.path.isfile(key_file) else None
        verdict["key"] = key
        if ok:
            serve_proc, serve_log, printed = start_serve(clone, port, os.path.join(out, "serve.log"))
            verdict["steps"].append({"cmd": f"make web-serve WEB_PORT={port}", "ok": printed is not None,
                                     "printed": printed})
            ok = printed is not None
        todo = []
        if ok:
            for u in urls(sec):
                e = dict(u)
                if "skip" not in e:
                    e["url"] = concrete(u["readme"], key or "", port)
                todo.append(e)
            body = oneliner(sec)
            if body:
                o = subprocess.run(["bash", "-c", body], cwd=clone, capture_output=True, text=True)
                for line in o.stdout.splitlines():
                    line = line.strip()
                    if line.startswith("http"):
                        todo.append({"source": "5. one-liner", "readme": line,
                                     "url": concrete(line, key or "", port)})
            for line in printed or []:
                todo.append({"source": "make web-serve's link", "readme": line, "url": line})
            # Each URL once (the first source names it; the others are kept).
            merged = {}
            for e in todo:
                k = e.get("url") or e["readme"]
                if k in merged:
                    merged[k].setdefault("also", []).append(e["source"])
                else:
                    merged[k] = e
            todo = list(merged.values())
            # Step 3's LAN server (WEB_BIND=0.0.0.0): the warning, the LAN
            # URLs, and a page by LAN address is no secure context, so it
            # gets the WebGL 2 build.
            lan_port = free_port()
            lan_serve = os.path.join(out, "serve-lan.log")
            lan_proc, lan_log, lan = start_serve(clone, lan_port, lan_serve, bind="0.0.0.0")
            text = (lan or {}).get("text", "")
            lan_step = {"cmd": f"make web-serve WEB_BIND=0.0.0.0 WEB_PORT={lan_port}",
                        "warning": "not a secure context" in text and "ssh -N -L" in text,
                        "lan_urls": (lan or {}).get("lan", [])}
            lan_step["ok"] = bool(lan and lan_step["warning"] and lan_step["lan_urls"])
            verdict["steps"].append(lan_step)
            lan_hosts.extend(re.sub(r"^http://|:\d+/.*$", "", u) for u in lan_step["lan_urls"])
            if lan_step["lan_urls"]:
                todo.append({"source": "3. LAN (WEB_BIND=0.0.0.0)", "readme": "http://ADDRESS:8080/",
                             "url": lan_step["lan_urls"][0], "expect_gpu": "webgl2",
                             "shown": redact(lan_step["lan_urls"][0]),
                             "serve_log": lan_serve})
            ok = visit_all(tw, a, todo, out, port, verdict, save) and ok
    finally:
        for proc, log in ((serve_proc, serve_log), (lan_proc, lan_log)):
            stop(proc)
            if log:
                log.close()
        steps_ok = all(st["ok"] for st in verdict["steps"])
        serve_ok = verdict["serve_without_build"]["ok"]
        urls_ok = bool(verdict["urls"]) and all(u.get("ok") or u.get("skip") for u in verdict["urls"])
        verdict["ok"] = bool(ok and steps_ok and serve_ok and urls_ok)
        save()
        for name in ("serve-lan.log", "browser.log"):
            path = os.path.join(out, name)
            if lan_hosts and os.path.exists(path):
                text = open(path, errors="replace").read()
                with open(path, "w") as f:
                    f.write(redact(text))
        if not a.keep:
            shutil.rmtree(clone, ignore_errors=True)

    for u in verdict["urls"]:
        state = "skip" if u.get("skip") else ("ok  " if u.get("ok") else "FAIL")
        print(redact(f"{state} [{u['source']}] {u.get('url') or u['readme']}"))
        if u.get("why"):
            print(f"       {u['why']}")
        if u.get("screenshot"):
            print(f"       {u['screenshot']}")
    print(f"verdict: {'ok' if verdict['ok'] else 'FAIL'}  {os.path.join(out, 'verdict.json')}")
    sys.exit(0 if verdict["ok"] else 1)


def visit_all(tw, a, todo, out, port, verdict, save):
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = tw.chromium_flags(profile, w, h, "webgpu")
    blog = open(os.path.join(out, "browser.log"), "w")
    proc, cdp = tw.start_browser(a.chromium, flags, blog)
    verdict["flags"] = flags
    lines = []
    all_ok = True
    va = types.SimpleNamespace(timeout=a.timeout, seconds=a.seconds, teleport=0.0,
                               teleport_wait=0.0, time=None)
    serve_log = os.path.join(out, "serve.log")
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable", "Network.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
        n = 0
        for e in todo:
            verdict["urls"].append(e)
            if e.get("skip"):
                continue
            n += 1
            url = e["url"]
            want = expects(url)
            if e.get("expect_gpu"):
                want["gpu"] = e["expect_gpu"]
            slog = e.pop("serve_log", serve_log)
            base = os.path.join(out, f"{n:02d}-{slug(url)}")
            print(f"open {e.pop('shown', url)}", flush=True)
            mark = len(cdp.events)
            log_mark = os.path.getsize(slog) if os.path.exists(slog) else 0
            try:
                r = tw.visit(cdp, url, va, base, lines, "first", want["world"], {})
            except Exception as ex:  # noqa: BLE001 – reported per URL
                r = {"ok": False, "errors": [repr(ex)]}
            gpu = cdp.eval("JSON.stringify(window.ocwGpu || null)")
            gpu = json.loads(gpu) if gpu else None
            with open(slog) as f:
                f.seek(log_mark)
                srv = server_errors(f.read())
            bad = http_errors(cdp.events, mark)
            status = document_status(cdp.events, mark)
            dist = float(r.get("last", {}).get("distance_m", 0) or 0)
            why = []
            if status != 200:
                why.append(f"index.html: HTTP {status}")
            if r.get("first_tick_s") is None:
                why.append("the app did not start (no server tick)")
            elif dist <= 5:
                why.append(f"the rider did not move ({dist} m)")
            if not gpu or gpu.get("api") != want["gpu"]:
                why.append(f"GPU build {gpu and gpu.get('api')}, expected {want['gpu']}")
            if bad:
                why.append(f"network errors: {bad[:5]}")
            if srv:
                why.append(f"server errors: {srv[:5]}")
            if r.get("errors"):
                why.append(f"errors: {[x[:200] for x in r['errors'][:3]]}")
            if want["world"] and r.get("ready_s") is None:
                why.append("the world did not get ready")
            e.update({
                "ok": not why, "why": "; ".join(why), "index_status": status,
                "first_tick_s": r.get("first_tick_s"), "ticks": r.get("ticks"), "distance_m": dist,
                "gpu": gpu, "world_ready_s": r.get("ready_s"), "package": r.get("package"),
                "http_errors": bad, "server_errors": srv,
                "screenshot": r.get("screenshot"), "loading_screenshot": r.get("loading_screenshot"),
                "status_text": (r.get("status_text") or "")[:300],
            })
            all_ok = all_ok and e["ok"]
            print(f"  {'ok' if e['ok'] else 'FAIL'} {e['why']}", flush=True)
            save()
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        blog.write("\n--- console ---\n" + "\n".join(lines) + "\n")
        blog.close()
        shutil.rmtree(profile, ignore_errors=True)
    return all_ok


if __name__ == "__main__":
    main()
