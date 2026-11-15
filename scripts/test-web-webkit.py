#!/usr/bin/env python3
"""The browser app in WebKit, Safari's engine (spec §14 Phase 10 B7, B8a; §11.4).

    scripts/test-web-webkit.py [--out test-output/web] [--dir target/web]
                               [--seconds 20] [--python PYTHON]

Playwright's WebKit build (`make deps-webkit`: a venv in
~/.cache/ocw/playwright-venv with Playwright's WebKit, no sudo) on
target/web/ (`make web`). WebKit on Linux has no WebGPU, but Safari 26 has
it on by default – the page must still take the WebGL 2 build there (the
iPhone report of 2026-11-15: WebGPU gave sound but no image), so the test
gives the page a `navigator.gpu` with an adapter, as Safari 26 has:

  iphone   an iPhone 15 (Playwright's device: 393 × 659, pixel ratio 3,
           touch, Safari's user agent): the preview's start page (its
           checkbox "WebGL 2 with low graphics" ticked), then the Hills ride
           on WebGL 2 – it rides and draws, the canvas fills the viewport,
           no pixel-ratio cap (WebKit sizes the canvas without
           devicePixelContentBoxSize: the cap would only shrink the HUD);
  mac      Safari on macOS (1280 × 720): the Hills ride on WebGL 2, draws.

  OUT/web-webkit-<time>-<case>.png   screenshots
  OUT/web-webkit-<time>.json / .log  checks and the console

Without Playwright (no `make deps-webkit`) it says so and exits 2.
Exits 0 when every check passed.
"""

import argparse
import datetime
import importlib.util
import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
VENV = os.path.join(os.path.expanduser("~"), ".cache", "ocw", "playwright-venv")

# Safari 26's navigator.gpu (WebKit on Linux has none): an adapter is there.
FAKE_GPU = """
if (!navigator.gpu) {
  Object.defineProperty(Navigator.prototype, "gpu", {get: () => ({
    requestAdapter: async () => { window.__adapterAsked = (window.__adapterAsked || 0) + 1; return {features: new Set(), limits: {}}; },
    getPreferredCanvasFormat: () => "bgra8unorm",
  })});
}
"""
PREVIEW_CONFIG = json.dumps({
    "channel": "preview", "worlds": "worlds/", "server": "", "voice": "", "signin": "",
    "build": "test-web-webkit",
    "start": [{"title": "Hills (test route)", "text": "A test loop.", "query": "route=hills"}],
})


def in_venv():
    try:
        import playwright  # noqa: F401
        return True
    except ImportError:
        return False


def web_serve():
    spec = importlib.util.spec_from_file_location("web_serve", os.path.join(HERE, "web-serve.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def image_drawn(path):
    try:
        from PIL import Image, ImageStat
    except ImportError:
        size = os.path.getsize(path)
        return size > 60000, f"{size} bytes (no PIL)"
    st = ImageStat.Stat(Image.open(path).convert("RGB"))
    return max(st.stddev) > 8.0, f"stddev {max(st.stddev):.1f}"


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--seconds", type=float, default=20.0, help="riding after the ride shows")
    p.add_argument("--python", default=None, help="a Python with Playwright (default: the venv of make deps-webkit)")
    a = p.parse_args()
    if not in_venv():
        py = a.python or os.path.join(VENV, "bin", "python")
        if not os.path.exists(py):
            print("test-web-webkit: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    from playwright.sync_api import sync_playwright

    ws = web_serve()
    missing = ws.missing_build(a.dir, ["ocw-gpu.js", "ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-webkit-" + stamp)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    checks, lines = [], []
    t0 = time.time()

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:600]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:200]}", flush=True)

    state_js = """() => JSON.stringify({gpu: window.ocwGpu || null, frames: window.ocwFrames,
        failure: window.ocwGpuFailure || null, adapterAsked: window.__adapterAsked || 0,
        status: document.getElementById('ocw-status').hidden ? '' : document.getElementById('ocw-status').textContent,
        canvas: [document.getElementById('ocw-canvas').width, document.getElementById('ocw-canvas').height,
                 document.getElementById('ocw-canvas').clientWidth, document.getElementById('ocw-canvas').clientHeight],
        viewport: [innerWidth, innerHeight, devicePixelRatio],
        ua: navigator.userAgent, vendor: navigator.vendor})"""

    def ride(pg, label):
        pg.goto(root + "?route=hills&bots=1")
        try:
            pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                 " && window.ocwFrames > 20", timeout=120000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{label}] wait: {e}")
        pg.wait_for_timeout(int(a.seconds * 1000))
        st = json.loads(pg.evaluate(state_js))
        shot = f"{base}-{label}.png"
        pg.screenshot(path=shot)
        return st, shot

    summary = {"ok": False}
    with sync_playwright() as pw:
        browser = pw.webkit.launch()
        summary["webkit"] = browser.version
        try:
            for label in ("iphone", "mac"):
                if label == "iphone":
                    ctx = browser.new_context(**pw.devices["iPhone 15"])
                else:
                    ctx = browser.new_context(viewport={"width": 1280, "height": 720})
                ctx.add_init_script(FAKE_GPU)
                pg = ctx.new_page()
                pg.on("console", lambda m, label=label: lines.append(
                    f"[{label}] {time.time() - t0:7.2f} s  {m.type}: {m.text[:600]}"))
                pg.on("pageerror", lambda e, label=label: lines.append(f"[{label}] PAGEERROR {e}"))
                if label == "iphone":
                    # The preview's start page with its checkbox.
                    pg.route("**/config.json", lambda route: route.fulfill(
                        status=200, content_type="application/json", body=PREVIEW_CONFIG))
                    pg.goto(root)
                    pg.wait_for_selector("#ocw-start:not([hidden])", timeout=30000)
                    ticked = pg.evaluate("document.getElementById('ocw-start-webgl2').checked")
                    href = pg.evaluate("document.querySelector('#ocw-start a.ride').getAttribute('href')")
                    box = pg.evaluate("(() => { const r = document.querySelector('#ocw-start .opts label').getBoundingClientRect();"
                                      " return [r.width, r.height]; })()")
                    pg.screenshot(path=f"{base}-iphone-start.png")
                    check("iPhone: the start page's WebGL 2 box is ticked, the ride link takes WebGL 2 low",
                          ticked and "gpu=webgl2" in href and "quality=low" in href, href)
                    check("iPhone: the box's label is a finger-sized target (≥ 44 px high)", box[1] >= 44, box)
                    pg.unroute("**/config.json")
                st, shot = ride(pg, label)
                drawn, how = image_drawn(shot)
                gpu = st.get("gpu") or {}
                check(f"{label}: Safari with WebGPU takes WebGL 2", gpu.get("api") == "webgl2"
                      and "Safari" in gpu.get("reason", "") and st.get("adapterAsked") == 0, gpu)
                check(f"{label}: rides and draws", st.get("frames", 0) > 20 and not st.get("status")
                      and not st.get("failure") and drawn, f"{how}; frames {st.get('frames')}")
                cw, vp = st["canvas"], st["viewport"]
                check(f"{label}: the canvas fills the viewport", abs(cw[2] - vp[0]) <= 1 and abs(cw[3] - vp[1]) <= 1,
                      {"canvas": cw, "viewport": vp})
                if label == "iphone":
                    # WebKit has no devicePixelContentBoxSize: winit keeps the
                    # drawing buffer at the device's pixels, and Low's pixel-
                    # ratio cap would only shrink the HUD to a third – so the
                    # app leaves it off there (the HUD keeps its size; see
                    # the screenshot).
                    check("iphone: no pixel-ratio cap in WebKit (buffer = CSS × ratio, the HUD unshrunk)",
                          abs(cw[0] - cw[2] * vp[2]) <= 3 and abs(cw[1] - cw[3] * vp[2]) <= 3,
                          {"canvas": cw, "viewport": vp})
                summary[label] = st
                ctx.close()
            summary["ok"] = all(c["ok"] for c in checks)
        except Exception as e:  # noqa: BLE001
            summary["error"] = repr(e)
            print("ERROR", repr(e))
        finally:
            browser.close()
            httpd.shutdown()
    summary["checks"] = checks
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    failed = [c for c in checks if not c["ok"]]
    print(f"{len(checks) - len(failed)} of {len(checks)} checks passed; log: {base}.log")
    sys.exit(0 if summary["ok"] and checks else 1)


if __name__ == "__main__":
    main()
