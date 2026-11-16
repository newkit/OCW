#!/usr/bin/env python3
"""The browser app in the real Safari over WebDriver (spec §14 Phase 10 B7; §11.4).

    scripts/test-web-safari.py [--url https://newkit.github.io/OCW/]
                               [--out test-output/web] [--seconds 25]
                               [--query route=hills] [--gpu auto|webgpu|webgl2]

Runs on a Mac (`scripts/remote.sh macos run 'scripts/test-web-safari.py …'`)
with Safari's own `safaridriver` (W3C WebDriver over HTTP; no Python
packages): opens the page, waits `--seconds`, reads what the page knows –
the GPU it chose (`window.ocwGpu`), the WebGPU watchdog's state
(`window.ocwGpuWatch`: frames drawn, its pixel check, a failure), the
status line, the canvas – and takes a WebDriver screenshot. `--gpu webgpu`
forces Safari's WebGPU (`?gpu=webgpu`): the way to see whether the iPhone
report of 2026-11-15 (iOS 26: sound, no image) still happens in macOS
Safari 26, which has the same WebGPU engine.

Needs, once per Mac (an administrator; safaridriver(1)):
  - a logged-in desktop session (Safari does not start from ssh alone:
    "Launch failed … Domain does not support specified action"), and
  - `sudo safaridriver --enable` (allows remote automation).

  OUT/web-safari-<time>-<gpu>.png   the screenshot
  OUT/web-safari-<time>.json        the page's state

Exits 0 when the page drew (no GPU failure, frames, the ride showing),
2 when safaridriver cannot start a session (it says why), 1 otherwise.
"""

import argparse
import base64
import datetime
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

STATE = """return JSON.stringify({gpu: window.ocwGpu || null, failure: window.ocwGpuFailure || null,
  watch: window.ocwGpuWatch ? {frames: window.ocwGpuWatch.frames, ok: window.ocwGpuWatch.ok,
    failed: window.ocwGpuWatch.failed, samples: window.ocwGpuWatch.samples} : null,
  frames: window.ocwFrames, href: location.href, ua: navigator.userAgent, hasGpu: !!navigator.gpu,
  status: document.getElementById('ocw-status') && !document.getElementById('ocw-status').hidden
    ? document.getElementById('ocw-status').textContent : '',
  canvas: (() => { const c = document.getElementById('ocw-canvas');
    return c ? [c.width, c.height, c.clientWidth, c.clientHeight, devicePixelRatio] : null; })(),
  memory: window.ocw && window.ocw.wasmMemory ? window.ocw.wasmMemory() : null});"""


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wd(port, method, path, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read() or b"{}").get("value")
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{method} {path}: {e.code} {e.read()[:500]!r}") from None


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--url", default="https://newkit.github.io/OCW/")
    p.add_argument("--query", default="route=hills")
    p.add_argument("--gpu", choices=("auto", "webgpu", "webgl2"), default="auto")
    p.add_argument("--seconds", type=float, default=25.0)
    p.add_argument("--out", default="test-output/web")
    a = p.parse_args()
    driver = shutil.which("safaridriver") or "/usr/bin/safaridriver"
    if not os.path.exists(driver):
        sys.exit("test-web-safari: no safaridriver (macOS with Safari only)")
    port = free_port()
    proc = subprocess.Popen([driver, "-p", str(port)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    session = None
    os.makedirs(a.out, exist_ok=True)
    base = os.path.join(a.out, "web-safari-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    summary = {"ok": False, "gpu_asked": a.gpu}
    try:
        for _ in range(50):
            try:
                wd(port, "GET", "/status", timeout=2)
                break
            except Exception:  # noqa: BLE001 – not up yet
                time.sleep(0.2)
        try:
            v = wd(port, "POST", "/session", {"capabilities": {"alwaysMatch": {"browserName": "safari"}}})
        except Exception as e:  # noqa: BLE001
            proc.terminate()
            out = proc.communicate(timeout=10)[0].decode(errors="replace")
            print(f"test-web-safari: no WebDriver session: {e}\n{out}\n"
                  "Once per Mac: log in to its desktop, then `sudo safaridriver --enable`.", file=sys.stderr)
            sys.exit(2)
        session = v["sessionId"]
        summary["safari"] = v.get("capabilities", {}).get("browserVersion")
        query = a.query + ("" if a.gpu == "auto" else f"&gpu={a.gpu}")
        url = a.url + ("&" if "?" in a.url else "?") + query
        wd(port, "POST", f"/session/{session}/url", {"url": url}, timeout=120)
        time.sleep(a.seconds)
        st = json.loads(wd(port, "POST", f"/session/{session}/execute/sync", {"script": STATE, "args": []}))
        shot = wd(port, "GET", f"/session/{session}/screenshot")
        path = f"{base}-{a.gpu}.png"
        with open(path, "wb") as f:
            f.write(base64.b64decode(shot))
        summary.update({"url": url, "state": st, "screenshot": path})
        watch = st.get("watch") or {}
        summary["ok"] = (not st.get("failure") and not st.get("status") and (st.get("frames") or 0) > 20
                         and (watch.get("ok") or (st.get("gpu") or {}).get("api") == "webgl2"))
    finally:
        if session:
            try:
                wd(port, "DELETE", f"/session/{session}")
            except Exception:  # noqa: BLE001
                pass
        proc.terminate()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
