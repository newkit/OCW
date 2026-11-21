#!/usr/bin/env python3
"""Headless browser test of the web app (spec §14 Phase 10, B1 / B2 / B6; §11.4).

    scripts/test-web.py [--out test-output/web] [--seconds 12] [--query ...]
                        [--chromium chromium] [--size 1280x720] [--software]
                        [--world ID|URL [--route NAME] [--teleport KM] [--no-reload]]
                        [--no-ui] [--god [--god-height M]] [--max-wasm-mb MB]

Serves target/web/ (`make web`; with --world also the published worlds in
target/web/worlds/, `make web-worlds`) on a free local port with range
requests, starts headless Chromium with WebGPU, loads the page and drives
it over the DevTools protocol (on a pipe: no extra Python packages). Waits
until the app logs that its local server ticks, rides `--seconds`, then
saves:

  OUT/web-<time>.jpg      the screenshot (the canvas with HUD and ISO time)
  OUT/web-<time>.log      the browser console, the GPU adapter, timings
  OUT/web-<time>.json     the summary (ticks, load times, adapter, errors)

With --world (B2, `make test-web-world`): the world package is streamed
from the dev server (the page gets `world=…&route=…` and the dev root of
target/web-keys/root.hex as `trust-root`). The test also saves the loading
screen (`-loading.jpg`), optionally teleports along the route to stream
new tiles (`-teleport.jpg`), and – unless --no-reload – loads the page a
second time in the same browser profile: that load must come from the
OPFS cache (no package bytes over the network) and be faster. Load times,
bytes from cache / network and the peak wasm memory go into the summary.

Without --world (and unless --no-ui) the UI part follows (B6,
scripts/web_ui_test.py): the menus by keyboard and by a fake gamepad,
Settings → Display, World → Weather, the rider list, the HUD with weather,
wind and damage, the GPS line (the test route gets `origin=` Vaduz) and
the console's Tab completion (spec 3.40), Captures → screenshot / FIT
downloads, a video, and a setting kept over a reload; screenshots
OUT/web-<time>-menu.jpg, -settings.jpg, -weather.jpg, -calendar.jpg, -hud.jpg,
-gps.jpg, -console.jpg, -riders.jpg, -gamepad.jpg and the downloads in
OUT/web-<time>-downloads/.

With --world and --god: unicorn mode over the world after the ride
(scripts/web_ui_test.py's unicorn part; `make test-web-unicorn`).

Exits 0 when the module loaded, the server ticked, the rider rode and no
uncaught exception happened (with --world: both loads were ready, the
second from the cache, and the wasm memory stayed below --max-wasm-mb,
default 1500 MB); 1 otherwise (the log says why).
"""

import argparse
import base64
import datetime
import importlib.util
import json
import os
import re
import select
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)


def web_serve():
    """scripts/web-serve.py as a module (its range-capable handler)."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web-serve.py")
    spec = importlib.util.spec_from_file_location("web_serve", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Cdp:
    """Chrome DevTools protocol over --remote-debugging-pipe (fd 3 / 4)."""

    def __init__(self, proc, to_browser, from_browser):
        self.proc, self.w, self.r = proc, to_browser, from_browser
        self.next_id = 0
        self.buf = b""
        self.events = []
        self.session = None

    def send(self, method, params=None, session=True):
        self.next_id += 1
        msg = {"id": self.next_id, "method": method, "params": params or {}}
        if session and self.session:
            msg["sessionId"] = self.session
        os.write(self.w, json.dumps(msg).encode() + b"\0")
        return self.next_id

    def call(self, method, params=None, session=True, timeout=30):
        want = self.send(method, params, session)
        deadline = time.time() + timeout
        while time.time() < deadline:
            for m in self.pump(0.2):
                if m.get("id") == want:
                    if "error" in m:
                        raise RuntimeError(f"{method}: {m['error']}")
                    return m.get("result", {})
        raise TimeoutError(method)

    def pump(self, timeout):
        """Reads what arrived within `timeout` s; events are kept."""
        out = []
        ready, _, _ = select.select([self.r], [], [], timeout)
        if ready:
            chunk = os.read(self.r, 1 << 20)
            if not chunk:
                raise RuntimeError("the browser closed the pipe")
            self.buf += chunk
        while b"\0" in self.buf:
            raw, self.buf = self.buf.split(b"\0", 1)
            m = json.loads(raw)
            if "method" in m:
                self.events.append(m)
            out.append(m)
        return out

    def eval(self, expression, await_promise=False):
        r = self.call("Runtime.evaluate", {
            "expression": expression, "awaitPromise": await_promise, "returnByValue": True})
        return r.get("result", {}).get("value")

    def screenshot(self, path):
        shot = self.call("Page.captureScreenshot", {"format": "jpeg", "quality": 90})
        with open(path, "wb") as f:
            f.write(base64.b64decode(shot["data"]))
        return path


def console_text(event):
    """One console line from a Runtime.consoleAPICalled event (Bevy's log
    lines come with %c style arguments; those are dropped)."""
    parts = []
    for a in event["params"].get("args", []):
        v = a.get("value", a.get("description", ""))
        parts.append(str(v))
    text = " ".join(parts)
    text = re.sub(r"%c", "", text)
    # Drop the CSS strings of the %c arguments.
    text = re.sub(r"\s*(color|font-weight|background)[^;]*;?", "", text)
    return text.strip()


MB = r"([\d.]+) (MB|kB)"
# A world ride fails above this much wasm memory (spec §14 Phase 8 T8: the
# vegetation of v3.49 took 2.7 GB in Liechtenstein and ran out of the 4 GB
# address space on Tenerife; a ride needs a few hundred MB). --max-wasm-mb
# changes it, 0 turns the check off.
WORLD_WASM_LIMIT_MB = 1500.0


def mb(value, unit):
    return float(value) * (1.0 if unit == "MB" else 0.001)


def streaming_line(text):
    """The numbers of the app's "world streaming: …" status line (None for
    other lines): tiles, the wasm memory and – since the memory work of
    B2 – the live heap and the GPU memory (apps/web/src/heap.rs,
    gpu_memory.rs)."""
    m = re.search(r"world streaming: (\d+) of (\d+) tiles resident \(" + MB + r", peak " + MB
                  + r"\), (\d+) loaded, (\d+) dropped", text)
    if not m:
        return None
    s = {"resident": int(m.group(1)), "tiles": int(m.group(2)),
         "resident_mb": mb(m.group(3), m.group(4)), "peak_mb": mb(m.group(5), m.group(6)),
         "loaded": int(m.group(7)), "dropped": int(m.group(8))}
    w = re.search(r"wasm memory " + MB, text)
    if w:
        s["wasm_mb"] = mb(w.group(1), w.group(2))
    h = re.search(r"heap ([\d.]+) MB \(peak ([\d.]+) MB\)", text)
    if h:
        s["heap_mb"], s["heap_peak_mb"] = float(h.group(1)), float(h.group(2))
    g = re.search(r"gpu meshes ([\d.]+) MB in (\d+) \(buffers ([\d.]+) MB\), textures ([\d.]+) MB", text)
    if g:
        s.update({"gpu_meshes_mb": float(g.group(1)), "meshes": int(g.group(2)),
                  "gpu_buffers_mb": float(g.group(3)), "gpu_textures_mb": float(g.group(4))})
    return s


def visit(cdp, url, a, base, lines, label, world, stats):
    """Loads `url`, rides, takes the screenshots; returns the visit's summary."""
    for k in list(stats):
        stats[k] = 0
    seen = len(cdp.events)
    t0 = time.time()
    cdp.call("Page.navigate", {"url": url})
    r = {"label": label, "url": url}
    first_tick = joined = ready = None
    ticks = 0
    last = {}
    errors = []
    streaming = []
    loading_shot = None
    teleported = None
    deadline = t0 + a.timeout
    last_status_poll = 0.0
    set_time = False
    while time.time() < deadline:
        cdp.pump(0.2)
        for e in cdp.events[seen:]:
            now = time.time() - t0
            if e["method"] == "Runtime.consoleAPICalled":
                text = console_text(e)
                lines.append(f"[{label}] {now:7.2f} s  {e['params'].get('type', 'log'):7} {text}")
                if "joined the local server" in text and joined is None:
                    joined = now
                m = re.search(r"local server running.*?ticks\s*=\s*(\d+)", text)
                if m:
                    ticks = int(m.group(1))
                    if first_tick is None and ticks > 0:
                        first_tick = now
                        set_time = bool(a.time)
                    last = dict(re.findall(r'(\w+)\s*=\s*"?([\d.]+)', text))
                m = re.search(r"world ready: (.*?) in ([\d.]+) s .*from cache " + MB
                              + r" in (\d+) chunks, from network " + MB + r" in (\d+) chunks, (\d+) requests", text)
                if m and ready is None:
                    ready = now
                    r.update({
                        "package": m.group(1), "ready_app_s": float(m.group(2)),
                        "cache_mb": mb(m.group(3), m.group(4)), "cache_chunks": int(m.group(5)),
                        "network_mb": mb(m.group(6), m.group(7)), "network_chunks": int(m.group(8)),
                        "requests": int(m.group(9)),
                    })
                s = streaming_line(text)
                if s:
                    s["t"] = round(now, 2)
                    streaming.append(s)
                if e["params"].get("type") == "error" and re.search(r"world|server rejected", text):
                    errors.append(text)
                # A GPU validation error stops Bevy's renderer (B7: what
                # WebGL 2 cannot do must never get that far).
                if re.search(r"Caught rendering error|Quitting the application", text):
                    errors.append(text[:400])
            elif e["method"] == "Runtime.exceptionThrown":
                d = e["params"]["exceptionDetails"]
                text = d.get("exception", {}).get("description") or d.get("text", "")
                if "Using exceptions for control flow" not in text:
                    errors.append(text)
                lines.append(f"[{label}] {now:7.2f} s  EXCEPTION {text}")
            elif e["method"] == "Log.entryAdded":
                entry = e["params"]["entry"]
                lines.append(f"[{label}] {now:7.2f} s  {entry['level']:7} [{entry['source']}] {entry['text']}")
        seen = len(cdp.events)
        if set_time:
            set_time = False
            cdp.eval(f"window.ocw && window.ocw.console('time {a.time}')")
        now = time.time() - t0
        # The loading screen, once, while it shows the progress.
        if world and ready is None and loading_shot is None and now - last_status_poll > 0.1:
            last_status_poll = now
            status = cdp.eval("document.getElementById('ocw-status')?.hidden ? '' : "
                              "(document.getElementById('ocw-status')?.textContent || '')") or ""
            if "Loading world" in status:
                r["loading_screen"] = status
                if "road graph" in status and ("✓ manifest" in status):
                    loading_shot = cdp.screenshot(f"{base}-{label}-loading.jpg")
            if "Error:" in status:
                errors.append("loading screen: " + status.replace("\n", " | "))
        if world and ready is None and errors:
            break
        if first_tick is not None:
            if a.teleport and teleported is None and label == "first" and now >= first_tick + a.seconds:
                r["screenshot"] = cdp.screenshot(f"{base}.jpg")
                cdp.eval(f"window.ocw && window.ocw.console('teleport {a.teleport}')")
                teleported = now
            done_at = first_tick + a.seconds + (a.teleport_wait if teleported else 0)
            # A later world ride stops once it is ready and the rider has
            # moved (> 5 m, the "rode" rule below) – 3 s from the first tick
            # were not always enough from a standstill (2026-11-15: the live
            # preview's world checks failed at 16.7 km/h, 2-4 m ridden) – or
            # after `seconds` at the latest.
            moved = float(last.get("distance_m", 0) or 0) > 5.0
            if (label != "first" and world and ready is not None and now >= first_tick + 3
                    and (moved or now >= first_tick + max(a.seconds, 3))) or \
               (label == "first" and now >= done_at and (not a.teleport or teleported)):
                break
        if errors and first_tick is None and not world:
            break
    r["status_text"] = cdp.eval("document.getElementById('ocw-status')?.hidden ? '' : "
                                "(document.getElementById('ocw-status')?.textContent || '')")
    mem = cdp.eval("JSON.stringify({wasm: window.ocw && window.ocw.wasmMemory ? window.ocw.wasmMemory() : null,"
                   " js_heap: performance.memory ? performance.memory.usedJSHeapSize : null})")
    r["memory"] = json.loads(mem) if mem else None
    heap = cdp.eval("window.ocw && window.ocw.heap ? window.ocw.heap() : null")
    if r["memory"] is not None and heap:
        r["memory"]["heap"] = json.loads(heap)
    r["opfs"] = cdp.eval("""(async () => {
        try {
            const root = await navigator.storage.getDirectory();
            const dir = await root.getDirectoryHandle('ocw-worlds');
            let n = 0, bytes = 0;
            for await (const [name, h] of dir.entries()) {
                if (h.kind === 'file') { n++; bytes += (await h.getFile()).size; }
            }
            const est = await navigator.storage.estimate();
            return {files: n, bytes, usage: est.usage, quota: est.quota,
                    persisted: await navigator.storage.persisted()};
        } catch (e) { return {error: String(e)}; }
    })()""", await_promise=True) if world else None
    if label == "first" or not world:
        key = "screenshot_teleport" if teleported else "screenshot"
        r[key] = cdp.screenshot(f"{base}-teleport.jpg" if teleported else f"{base}.jpg")
    else:
        r["screenshot"] = cdp.screenshot(f"{base}-{label}.jpg")
    r.update({
        "loading_screenshot": loading_shot,
        "joined_s": joined, "first_tick_s": first_tick, "ready_s": ready,
        "ticks": ticks, "last": last, "errors": errors,
        "streaming_last": streaming[-1] if streaming else None,
        "peak_resident_mb": max((s["peak_mb"] for s in streaming), default=None),
        "peak_wasm_mb": max((s.get("wasm_mb", 0) for s in streaming), default=None),
        "dropped": max((s["dropped"] for s in streaming), default=0),
        "peak_heap_mb": max((s.get("heap_peak_mb", 0) for s in streaming), default=None),
        "peak_gpu_mb": max((s.get("gpu_buffers_mb", 0) + s.get("gpu_textures_mb", 0)
                            for s in streaming), default=None),
        "memory_timeline": [{k: v for k, v in s.items()
                             if k in ("t", "resident", "wasm_mb", "heap_mb", "heap_peak_mb",
                                      "gpu_meshes_mb", "gpu_buffers_mb", "gpu_textures_mb", "meshes")}
                            for s in streaming],
        "server": dict(stats),
    })
    rode = float(last.get("distance_m", 0)) > 5.0
    limit = wasm_limit(a, world)
    r["wasm_limit_mb"] = limit or None
    over = wasm_error(r["peak_wasm_mb"], limit)
    if over:
        errors.append(over)
    r["ok"] = first_tick is not None and rode and not errors and (ready is not None or not world)
    return r


def wasm_limit(a, world):
    """The wasm memory limit of a visit (MB; 0: none): --max-wasm-mb, else
    WORLD_WASM_LIMIT_MB for world rides."""
    limit = getattr(a, "max_wasm_mb", None)
    if limit is None:
        limit = WORLD_WASM_LIMIT_MB if world else 0.0
    return float(limit)


def wasm_error(peak_mb, limit_mb):
    """The error of a visit whose wasm memory went above the limit, or None."""
    if limit_mb and (peak_mb or 0) > limit_mb:
        return f"wasm memory {peak_mb:.0f} MB, above the limit of {limit_mb:.0f} MB"
    return None


def chromium_flags(profile, w, h, gpu="webgpu", software=False):
    """Headless Chromium's flags for the tests: the DevTools pipe, a fresh
    profile, and the GPU API (B7)."""
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        # No proxy detection, and no keyring over D-Bus: without a desktop
        # session the latter waits for D-Bus' 25 s timeout before the first
        # page loads.
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        "--ignore-gpu-blocklist", "--enable-gpu-rasterization",
    ]
    if gpu == "webgpu":
        # WebGPU in headless Chromium on Linux: Dawn on Vulkan.
        flags += ["--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPUService",
                  "--use-angle=vulkan"]
    else:
        # No WebGPU (B7): the page must find no navigator.gpu and load the
        # WebGL 2 build; WebGL 2 through ANGLE on Vulkan (the GPU).
        flags += ["--disable-features=WebGPU,WebGPUService", "--disable-blink-features=WebGPU",
                  "--enable-features=Vulkan", "--use-angle=vulkan"]
    if software:
        flags += (["--use-webgpu-adapter=swiftshader"] if gpu == "webgpu"
                  else ["--use-angle=swiftshader"]) + ["--enable-unsafe-swiftshader"]
    return flags


def start_browser(chromium, flags, log):
    """Starts `chromium` on about:blank with the DevTools protocol on fds 3
    (commands in) and 4 (answers out); returns (process, Cdp)."""
    cmd_r, cmd_w = os.pipe()
    res_r, res_w = os.pipe()
    os.set_inheritable(cmd_r, True)
    os.set_inheritable(res_w, True)

    def child_fds():
        os.dup2(cmd_r, 3)
        os.dup2(res_w, 4)

    # close_fds=False: the fd cleanup would run after preexec_fn and close
    # the new 3 and 4 again.
    proc = subprocess.Popen([chromium, *flags, "about:blank"], preexec_fn=child_fds,
                            close_fds=False, stdout=log, stderr=subprocess.STDOUT)
    os.close(cmd_r)
    os.close(res_w)
    return proc, Cdp(proc, cmd_w, res_r)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--seconds", type=float, default=12.0,
                   help="seconds of riding after the server first ticks")
    p.add_argument("--timeout", type=float, default=120.0)
    p.add_argument("--query", default="", help="page options, e.g. route=loop&power=250")
    p.add_argument("--chromium", default=None,
                   help="the browser (default: OCW_BROWSER, else the pinned Chromium of `make deps-browser`,"
                        " else the system's chromium; scripts/web_browser.py)")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--software", action="store_true",
                   help="SwiftShader (CPU) instead of the GPU")
    p.add_argument("--gpu", choices=("webgpu", "webgl2"), default="webgpu",
                   help="the GPU API (B7): webgl2 starts Chromium without WebGPU, so the page"
                        " falls back to the WebGL 2 build")
    p.add_argument("--fps-seconds", type=float, default=5.0,
                   help="seconds over which the frame rate is measured after the ride")
    p.add_argument("--world", help="stream this world (id or manifest URL) from the dev server")
    p.add_argument("--route", help="a route of the world package (e.g. vaduz-malbun)")
    p.add_argument("--teleport", type=float, default=0.0,
                   help="km along the route to teleport to after --seconds (streams new tiles)")
    p.add_argument("--teleport-wait", type=float, default=10.0)
    p.add_argument("--time", default=None,
                   help="the time machine once the ride runs (console `time …`, e.g. noon)")
    p.add_argument("--no-reload", action="store_true",
                   help="with --world: no second load from the OPFS cache")
    p.add_argument("--god", action="store_true",
                   help="with --world: fly in unicorn mode after the ride (Phase 13 G5; "
                        "screenshots -unicorn-flight, -unicorn-inspect, -unicorn-menu)")
    p.add_argument("--god-height", type=float, default=250.0,
                   help="with --god: put the unicorn this high above the ground first (m)")
    p.add_argument("--no-ui", action="store_true",
                   help="no UI part (menus, settings, downloads; B6)")
    p.add_argument("--scene", action="append", metavar="NAME:SECONDS:CMD|CMD",
                   help="after the ride: run these console commands, wait, screenshot "
                        "BASE-NAME.jpg (repeatable; e.g. 'workshop:6:workshop open')")
    p.add_argument("--max-wasm-mb", type=float, default=None,
                   help=f"fail when the wasm memory goes above this (MB; default {WORLD_WASM_LIMIT_MB:.0f} "
                        "for world rides, none otherwise; 0: no check)")
    p.add_argument("--trust-root", default=None,
                   help="root public key (hex) the page trusts (default: target/web-keys/root.hex)")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    module = "ocw_web_webgl2_bg.wasm" if a.gpu == "webgl2" else "ocw_web_bg.wasm"
    missing = web_serve().missing_build(a.dir, [module])
    if missing:
        sys.exit(missing)
    query = a.query
    # The UI part checks the GPS line (spec 3.40): the test route gets a
    # geographic origin (Vaduz), as a route file has one.
    if not a.world and not a.no_ui and "origin=" not in query:
        query = "&".join(q for q in (query, "origin=47.13996,9.52151") if q)
    if a.world:
        if not os.path.isdir(os.path.join(a.dir, "worlds")):
            sys.exit(f"{a.dir}/worlds missing: run `make web-worlds` first")
        root = a.trust_root
        if root is None and os.path.isfile("target/web-keys/root.hex"):
            root = open("target/web-keys/root.hex").read().strip()
        opts = [f"world={a.world}"] + ([f"route={a.route}"] if a.route else [])
        opts += [f"trust-root={root}"] if root else ["allow-unsigned=1"]
        query = "&".join(opts + ([query] if query else []))
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    kind = ("web-world-" if a.world else "web-") + ("webgl2-" if a.gpu == "webgl2" else "")
    base = os.path.join(a.out, kind + stamp)

    stats = {}
    httpd = web_serve().server(a.dir, stats=stats)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/" + (f"?{query}" if query else "")

    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = chromium_flags(profile, w, h, a.gpu, a.software)
    log = open(base + ".log", "w")
    proc, cdp = start_browser(a.chromium, flags, log)
    summary = {"url": url, "flags": flags, "ok": False, "browser": web_browser.describe(a.chromium)}
    lines = []
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget",
                               {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
        first = visit(cdp, url, a, base, lines, "first", bool(a.world), stats)
        visits = [first]
        # The GPU API the page chose (B7), the app's own word for it (the
        # console's `gpu`) and the frame rate while riding.
        gpu = cdp.eval("JSON.stringify(window.ocwGpu || null)")
        summary["gpu"] = json.loads(gpu) if gpu else None
        mark = len(cdp.events)
        cdp.eval("window.ocw && window.ocw.console('gpu')")
        f0 = cdp.eval("[window.ocwFrames, performance.now()]")
        end = time.time() + a.fps_seconds
        while time.time() < end:
            cdp.pump(0.2)
        f1 = cdp.eval("[window.ocwFrames, performance.now()]")
        fps = (f1[0] - f0[0]) * 1000.0 / max(1.0, f1[1] - f0[1]) if f0 and f1 else None
        # The render distance (spec §14 Phase 10, 10G): dynamic by default
        # (Q8), 60 fps in the browser (Q9); the distance it chose.
        cdp.eval("window.ocw && window.ocw.console('view')")
        end = time.time() + 3.0
        while time.time() < end:
            cdp.pump(0.2)
            if any(e["method"] == "Runtime.consoleAPICalled" and "view:" in console_text(e)
                   for e in cdp.events[mark:]):
                break
        said = [console_text(e) for e in cdp.events[mark:] if e["method"] == "Runtime.consoleAPICalled"]
        gpu_lines = [t for t in said if "gpu:" in t or "quality" in t]
        lines.extend(f"[gpu] {t}" for t in gpu_lines)
        view_lines = [t for t in said if "view:" in t or "view distance:" in t]
        lines.extend(f"[view] {t}" for t in view_lines)
        summary["view_console"] = view_lines
        summary["view_ok"] = any("view:" in t and "(auto" in t and "target 60 fps" in t
                                 for t in view_lines)
        want_api = {"webgpu": "WebGPU", "webgl2": "WebGL2"}[a.gpu]
        summary["fps"] = round(fps, 1) if fps is not None else None
        summary["gpu_console"] = gpu_lines
        summary["gpu_ok"] = bool(summary["gpu"] and summary["gpu"].get("api") == a.gpu
                                 and any(want_api in t for t in gpu_lines))
        first["ok"] = first["ok"] and summary["gpu_ok"] and summary["view_ok"]
        # Console scenes (--scene NAME:SECONDS:CMD|CMD…): the commands, the
        # wait, a screenshot BASE-NAME.jpg (e.g. the workshop, unicorn mode).
        summary["scenes"] = []
        for spec in a.scene or []:
            name, seconds, cmds = spec.split(":", 2)
            for c in filter(None, cmds.split("|")):
                cdp.eval(f"window.ocw && window.ocw.console({json.dumps(c)})")
                end = time.time() + 0.3
                while time.time() < end:
                    cdp.pump(0.1)
            end = time.time() + float(seconds)
            while time.time() < end:
                cdp.pump(0.2)
            summary["scenes"].append({"name": name, "screenshot": cdp.screenshot(f"{base}-{name}.jpg")})
        if not a.world and not a.no_ui and first["ok"]:
            sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
            import web_ui_test

            def reload_and_wait():
                mark = len(cdp.events)
                cdp.call("Page.reload", {})
                end = time.time() + 60
                while time.time() < end:
                    cdp.pump(0.2)
                    if any(e["method"] == "Runtime.consoleAPICalled"
                           and "local server running" in console_text(e)
                           for e in cdp.events[mark:]):
                        return True
                return False

            summary["ui"] = web_ui_test.run(cdp, base, lines, a.out, reload_and_wait)
        if a.world and a.god and first["ok"]:
            sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
            import web_ui_test
            summary["ui"] = web_ui_test.run_god(cdp, base, lines, a.out, height=a.god_height)
        if a.world and not a.no_reload and first.get("ready_s") is not None:
            visits.append(visit(cdp, url, a, base, lines, "cached", True, stats))
        adapter = cdp.eval("""(async () => {
                if (!navigator.gpu) return 'no navigator.gpu';
                const ad = await navigator.gpu.requestAdapter();
                if (!ad) return 'no adapter';
                const i = ad.info || {};
                return [i.vendor, i.architecture, i.device, i.description].filter(Boolean).join(' / ') || 'adapter (no info)';
            })()""", await_promise=True)
        timing = cdp.eval("""JSON.stringify({
                wasm: performance.getEntriesByType('resource')
                    .filter(e => e.name.endsWith('.wasm'))
                    .map(e => ({start_ms: e.startTime, end_ms: e.responseEnd, bytes: e.transferSize})),
                html_ms: performance.getEntriesByType('navigation')[0]?.responseEnd,
                dom_ms: performance.getEntriesByType('navigation')[0]?.domContentLoadedEventEnd,
                import_ms: performance.getEntriesByName('ocw-import')[0]?.startTime,
                started_ms: performance.getEntriesByName('ocw-started')[0]?.startTime,
            })""")
        summary["page_timing"] = json.loads(timing) if timing else None
        summary.update({
            "adapter": adapter,
            "joined_s": first["joined_s"], "first_tick_s": first["first_tick_s"],
            "ticks": first["ticks"], "last": first["last"], "errors": first["errors"],
            "screenshot": first.get("screenshot"),
        })
        ok = first["ok"] and summary.get("ui", {"ok": True})["ok"]
        if a.world:
            summary["visits"] = visits
            cached = visits[1] if len(visits) > 1 else None
            if cached is not None:
                ok = ok and cached["ok"]
                from_cache = cached.get("network_mb", 1.0) == 0.0 and cached.get("cache_mb", 0) > 0
                faster = cached["ready_s"] is not None and cached["ready_s"] < first["ready_s"]
                summary["second_load"] = {
                    "from_cache": from_cache, "faster": faster,
                    "first_ready_s": first["ready_s"], "cached_ready_s": cached["ready_s"],
                    "first_package_mb_served": first["server"].get("worlds_bytes", 0) / 1e6,
                    "cached_package_mb_served": cached["server"].get("worlds_bytes", 0) / 1e6,
                }
                ok = ok and from_cache and faster
            elif not a.no_reload:
                ok = False
            if a.teleport:
                ok = ok and first["dropped"] >= 0 and first.get("screenshot_teleport") is not None
        summary["ok"] = ok
    except Exception as e:  # noqa: BLE001 – reported in the summary
        summary["error"] = repr(e)
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
    log.write("\n--- console ---\n" + "\n".join(lines) + "\n")
    log.write("\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    print("\n".join(lines[-20:]))
    keys = ("ok", "gpu", "gpu_ok", "gpu_console", "view_ok", "view_console", "fps", "adapter", "page_timing", "joined_s",
            "first_tick_s", "ticks", "last", "errors", "error", "second_load")
    print(json.dumps({k: summary.get(k) for k in keys}, indent=2))
    if "ui" in summary:
        for c in summary["ui"]["checks"]:
            print(f"{'ok  ' if c['ok'] else 'FAIL'} {c['check']}: {c['detail']}")
        print(json.dumps(summary["ui"]["screenshots"], indent=2))
    for v in summary.get("visits", []):
        print(json.dumps({k: v.get(k) for k in (
            "label", "ok", "ready_s", "ready_app_s", "cache_mb", "network_mb", "requests",
            "peak_resident_mb", "peak_wasm_mb", "peak_heap_mb", "peak_gpu_mb", "wasm_limit_mb", "dropped", "memory", "opfs", "server",
            "screenshot", "screenshot_teleport", "loading_screenshot", "errors")}, indent=2))
    print(f"log: {base}.log  screenshot: {base}.jpg")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
