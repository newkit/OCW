#!/usr/bin/env python3
"""Headless browser test of the web app (spec §14 Phase 10, B1; §11.4).

    scripts/test-web.py [--out test-output/web] [--seconds 12] [--query ...]
                        [--chromium chromium] [--gl webgpu|webgl2] [--size 1280x720]

Serves target/web/ (`make web`) on a free local port, starts headless
Chromium with WebGPU, loads the page and drives it over the DevTools
protocol (on a pipe: no extra Python packages). Waits until the app logs
that its local server ticks, rides `--seconds`, then saves:

  OUT/web-<time>.jpg      the screenshot (the canvas with HUD and ISO time)
  OUT/web-<time>.log      the browser console, the GPU adapter, timings
  OUT/web-<time>.json     the summary (ticks, load times, adapter, errors)

Exits 0 when the module loaded, the server ticked, the rider rode and no
uncaught exception happened; 1 otherwise (the log says why).
"""

import argparse
import base64
import datetime
import functools
import http.server
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


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *args):
        pass


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


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--seconds", type=float, default=12.0,
                   help="seconds of riding after the server first ticks")
    p.add_argument("--timeout", type=float, default=120.0)
    p.add_argument("--query", default="", help="page options, e.g. route=loop&power=250")
    p.add_argument("--chromium", default=shutil.which("chromium") or "chromium")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--software", action="store_true",
                   help="SwiftShader (CPU) instead of the GPU")
    a = p.parse_args()
    if not os.path.exists(os.path.join(a.dir, "ocw_web_bg.wasm")):
        sys.exit(f"{a.dir}/ocw_web_bg.wasm missing: run `make web` first")
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"web-{stamp}")

    httpd = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Handler, directory=a.dir))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/" + (f"?{a.query}" if a.query else "")

    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        # No proxy detection, and no keyring over D-Bus: without a desktop
        # session the latter waits for D-Bus' 25 s timeout before the first
        # page loads.
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        # WebGPU in headless Chromium on Linux: Dawn on Vulkan.
        "--enable-unsafe-webgpu", "--ignore-gpu-blocklist",
        "--enable-features=Vulkan,WebGPUService",
        "--use-angle=vulkan", "--enable-gpu-rasterization",
    ]
    if a.software:
        flags += ["--use-webgpu-adapter=swiftshader", "--enable-unsafe-swiftshader"]
    # fds 3 and 4 of the child: commands in, answers out.
    cmd_r, cmd_w = os.pipe()
    res_r, res_w = os.pipe()
    os.set_inheritable(cmd_r, True)
    os.set_inheritable(res_w, True)

    def child_fds():
        os.dup2(cmd_r, 3)
        os.dup2(res_w, 4)

    log = open(base + ".log", "w")
    # close_fds=False: the fd cleanup would run after preexec_fn and close
    # the new 3 and 4 again.
    proc = subprocess.Popen([a.chromium, *flags, "about:blank"], preexec_fn=child_fds,
                            close_fds=False, stdout=log, stderr=subprocess.STDOUT)
    os.close(cmd_r)
    os.close(res_w)
    cdp = Cdp(proc, cmd_w, res_r)
    summary = {"url": url, "flags": flags, "ok": False}
    lines, errors = [], []
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
        t0 = time.time()
        cdp.call("Page.navigate", {"url": url})
        first_tick = joined = None
        ticks = 0
        last = {}
        deadline = t0 + a.timeout
        seen = 0
        while time.time() < deadline:
            cdp.pump(0.2)
            for e in cdp.events[seen:]:
                now = time.time() - t0
                if e["method"] == "Runtime.consoleAPICalled":
                    text = console_text(e)
                    lines.append(f"{now:7.2f} s  {e['params'].get('type', 'log'):7} {text}")
                    if "joined the local server" in text and joined is None:
                        joined = now
                    m = re.search(r"local server running.*?ticks\s*=\s*(\d+)", text)
                    if m:
                        ticks = int(m.group(1))
                        if first_tick is None and ticks > 0:
                            first_tick = now
                        last = dict(re.findall(r'(\w+)\s*=\s*"?([\d.]+)', text))
                elif e["method"] == "Runtime.exceptionThrown":
                    d = e["params"]["exceptionDetails"]
                    text = d.get("exception", {}).get("description") or d.get("text", "")
                    if "Using exceptions for control flow" not in text:
                        errors.append(text)
                    lines.append(f"{now:7.2f} s  EXCEPTION {text}")
                elif e["method"] == "Log.entryAdded":
                    entry = e["params"]["entry"]
                    lines.append(f"{now:7.2f} s  {entry['level']:7} [{entry['source']}] {entry['text']}")
            seen = len(cdp.events)
            if first_tick is not None and time.time() - t0 >= first_tick + a.seconds:
                break
            if errors and first_tick is None:
                break
        adapter = cdp.call("Runtime.evaluate", {
            "expression": """(async () => {
                if (!navigator.gpu) return 'no navigator.gpu';
                const ad = await navigator.gpu.requestAdapter();
                if (!ad) return 'no adapter';
                const i = ad.info || {};
                return [i.vendor, i.architecture, i.device, i.description].filter(Boolean).join(' / ') || 'adapter (no info)';
            })()""",
            "awaitPromise": True, "returnByValue": True})["result"].get("value")
        timing = cdp.call("Runtime.evaluate", {
            "expression": """JSON.stringify({
                wasm: performance.getEntriesByType('resource')
                    .filter(e => e.name.endsWith('.wasm'))
                    .map(e => ({start_ms: e.startTime, end_ms: e.responseEnd, bytes: e.transferSize})),
                html_ms: performance.getEntriesByType('navigation')[0]?.responseEnd,
                dom_ms: performance.getEntriesByType('navigation')[0]?.domContentLoadedEventEnd,
                import_ms: performance.getEntriesByName('ocw-import')[0]?.startTime,
                started_ms: performance.getEntriesByName('ocw-started')[0]?.startTime,
            })""",
            "returnByValue": True})["result"].get("value")
        summary["page_timing"] = json.loads(timing) if timing else None
        shot = cdp.call("Page.captureScreenshot", {"format": "jpeg", "quality": 90})
        with open(base + ".jpg", "wb") as f:
            f.write(base64.b64decode(shot["data"]))
        rode = float(last.get("distance_m", 0)) > 5.0
        summary.update({
            "ok": first_tick is not None and rode and not errors,
            "adapter": adapter,
            "joined_s": joined, "first_tick_s": first_tick,
            "ticks": ticks, "last": last, "errors": errors,
            "screenshot": base + ".jpg",
        })
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
    print("\n".join(lines[-15:]))
    print(json.dumps({k: summary.get(k) for k in
                      ("ok", "adapter", "page_timing", "joined_s", "first_tick_s", "ticks", "last", "errors", "error")},
                     indent=2))
    print(f"log: {base}.log  screenshot: {base}.jpg")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
