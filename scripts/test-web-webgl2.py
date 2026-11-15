#!/usr/bin/env python3
"""WebGPU High against WebGL 2 Low in the browser (spec §14 Phase 10, B7; §11.4).

    scripts/test-web-webgl2.py [--out test-output/web] [--dir target/web]
                               [--chromium chromium] [--size 1280x720]

The other half of `make test-web-webgl2` (the first is `scripts/test-web.py
--gpu webgl2`: Chromium without WebGPU, the page falls back to the WebGL 2
build, rides, the menus, the frame rate). Here one headless Chromium with
WebGPU loads the same view twice – the page option `gpu=` chooses the build:

  1. `gpu=webgpu&quality=high`, 2. `gpu=webgl2&quality=low`

on the Hills route at km 1.2, no bots, at 0 W (coasting), at midsummer noon
(the clock paused) in clear weather; after the light has settled it measures the frame
rate (requestAnimationFrame over 6 s), asks the console's `gpu` and saves
a screenshot of each. On WebGL 2 it then opens Settings → Display →
Graphics (the quality preset's page) and saves that too.

  OUT/web-compare-<time>-webgpu-high.jpg   the WebGPU build, High
  OUT/web-compare-<time>-webgl2-low.jpg    the WebGL 2 build, Low
  OUT/web-compare-<time>.jpg               both side by side (labelled)
  OUT/web-compare-<time>-webgl2-graphics.jpg   the menu on WebGL 2
  OUT/web-compare-<time>.json / .log       fps, the `gpu` lines, errors

Exits 0 when both builds rode with the API and level asked for, without a
rendering error, and the Graphics page opened on WebGL 2.
"""

import argparse
import datetime
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

HERE = os.path.dirname(os.path.abspath(__file__))


def test_web():
    """scripts/test-web.py as a module (Cdp, the server, console lines)."""
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


RENDER_ERRORS = ("Caught rendering error", "Quitting the application", "panicked at")


def ride(tw, cdp, url, label, lines, timeout=90.0):
    """Loads `url`, waits until the local server ticks, sets the scene,
    measures the frame rate and the `gpu` answer; the summary."""
    seen = len(cdp.events)
    t0 = time.time()
    cdp.call("Page.navigate", {"url": url})
    r = {"label": label, "url": url, "errors": [], "gpu_lines": []}
    ticking = None
    scene_set = False

    def drain():
        nonlocal seen, ticking
        for e in cdp.events[seen:]:
            now = time.time() - t0
            if e["method"] == "Runtime.consoleAPICalled":
                text = tw.console_text(e)
                lines.append(f"[{label}] {now:7.2f} s  {text}")
                if ticking is None and re.search(r"local server running.*?ticks\s*=\s*[1-9]", text):
                    ticking = now
                if text.startswith("gpu:") or "gpu: " in text or "quality: " in text:
                    r["gpu_lines"].append(text)
                if any(k in text for k in RENDER_ERRORS):
                    r["errors"].append(text[:400])
            elif e["method"] == "Runtime.exceptionThrown":
                d = e["params"]["exceptionDetails"]
                text = d.get("exception", {}).get("description") or d.get("text", "")
                lines.append(f"[{label}] {now:7.2f} s  EXCEPTION {text}")
                if "Using exceptions for control flow" not in text:
                    r["errors"].append(text[:400])
        seen = len(cdp.events)

    def wait(seconds):
        end = time.time() + seconds
        while time.time() < end:
            cdp.pump(0.1)
            drain()

    while time.time() - t0 < timeout and ticking is None and not r["errors"]:
        wait(0.2)
    if ticking is None:
        r["errors"].append("the local server never ticked")
        return r
    # The same light in both: midsummer noon, the clock paused, clear sky.
    for line in ("sim.power 0", "weather.transition instant", "weather clear",
                 "time 2026-06-21 12:00", "time pause"):
        cdp.eval(f"window.ocw.console({json.dumps(line)})")
    scene_set = True
    # The exposure eases in a few seconds; shaders compile on first use.
    wait(6.0)
    f0 = cdp.eval("[window.ocwFrames, performance.now()]")
    wait(6.0)
    f1 = cdp.eval("[window.ocwFrames, performance.now()]")
    r["fps"] = round((f1[0] - f0[0]) * 1000.0 / max(1.0, f1[1] - f0[1]), 1)
    cdp.eval("window.ocw.console('gpu')")
    wait(1.0)
    r["chosen"] = json.loads(cdp.eval("JSON.stringify(window.ocwGpu || null)") or "null")
    r["scene_set"] = scene_set
    r["ticking_s"] = round(ticking, 2)
    return r


def side_by_side(left, right, labels, out):
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        return None
    a, b = Image.open(left), Image.open(right)
    w = a.width + b.width
    img = Image.new("RGB", (w, max(a.height, b.height) + 34), (16, 20, 26))
    img.paste(a, (0, 34))
    img.paste(b, (a.width, 34))
    d = ImageDraw.Draw(img)
    for x, text in ((0, labels[0]), (a.width, labels[1])):
        d.text((x + 12, 9), text, fill=(235, 203, 139))
    img.save(out, "JPEG", quality=90)
    return out


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--chromium", default=None,
                   help="the browser (default: OCW_BROWSER, else the pinned Chromium of `make deps-browser`,"
                        " else the system's chromium; scripts/web_browser.py)")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--start", default="1.2", help="km on the Hills route")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm", "ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing + "\n  (both builds: not WEB_GPU=webgpu)")
    sys.path.insert(0, HERE)
    import web_ui_test

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-compare-" + stamp)
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        "--ignore-gpu-blocklist", "--enable-gpu-rasterization",
        # WebGPU (Dawn on Vulkan) and WebGL 2 (ANGLE on Vulkan), the GPU.
        "--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPUService", "--use-angle=vulkan",
    ]
    cmd_r, cmd_w = os.pipe()
    res_r, res_w = os.pipe()
    os.set_inheritable(cmd_r, True)
    os.set_inheritable(res_w, True)

    def child_fds():
        os.dup2(cmd_r, 3)
        os.dup2(res_w, 4)

    log = open(base + ".log", "w")
    proc = subprocess.Popen([a.chromium, *flags, "about:blank"], preexec_fn=child_fds,
                            close_fds=False, stdout=log, stderr=subprocess.STDOUT)
    os.close(cmd_r)
    os.close(res_w)
    cdp = tw.Cdp(proc, cmd_w, res_r)
    lines = []
    summary = {"ok": False, "runs": [], "browser": web_browser.describe(a.chromium)}
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
        shots = []
        for gpu, quality, api in (("webgpu", "high", "WebGPU"), ("webgl2", "low", "WebGL2")):
            url = f"{root}?gpu={gpu}&quality={quality}&bots=0&power=0&start={a.start}"
            r = ride(tw, cdp, url, f"{gpu}-{quality}", lines)
            r["shot"] = cdp.screenshot(f"{base}-{gpu}-{quality}.jpg")
            shots.append(r["shot"])
            said = " ".join(r["gpu_lines"])
            r["ok"] = (not r["errors"] and (r.get("chosen") or {}).get("api") == gpu
                       and f"gpu: {api}" in said and f"quality: {quality}" in said)
            summary["runs"].append(r)
        # The quality preset's page on WebGL 2 (the menu still runs there).
        ui = web_ui_test.UiTest(cdp, base, lines, a.out)
        ui.key("Escape")
        opened = ui.wait_menu(lambda m: m.get("open")).get("open")
        graphics = (opened and ui.open_item("Settings", "Settings")
                    and ui.open_item("Display", "Display") and ui.open_item("Graphics", "Graphics"))
        m = ui.menu()
        quality_row = next((i for i in m.get("items", []) if i.startswith("Quality")), None)
        graphics_shot = ui.shot("webgl2-graphics") if graphics else None
        ui.close_menu()
        summary["graphics_page"] = {"opened": bool(graphics), "items": m.get("items"),
                                    "quality_row": quality_row, "shot": graphics_shot}
        # Low's pixel-ratio cap: a HiDPI screen (2 device pixels per CSS
        # pixel) draws the canvas at 1.
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": int(w), "height": int(h), "deviceScaleFactor": 2, "mobile": False})
        ui.pump(2.5)
        canvas = json.loads(cdp.eval("""JSON.stringify((() => {
            const c = document.getElementById('ocw-canvas');
            return {width: c.width, css_width: c.clientWidth, ratio: devicePixelRatio};
        })())""") or "{}")
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
        summary["pixel_ratio"] = canvas
        capped = canvas.get("ratio") == 2 and abs(canvas.get("width", 0) - canvas.get("css_width", -9)) <= 2
        summary["compare"] = side_by_side(
            shots[0], shots[1],
            [f"WebGPU, quality high - {summary['runs'][0].get('fps')} fps",
             f"WebGL 2, quality low - {summary['runs'][1].get('fps')} fps"],
            base + ".jpg")
        summary["pixel_ratio"]["capped"] = capped
        summary["ok"] = all(r["ok"] for r in summary["runs"]) and bool(graphics) \
            and bool(quality_row and "low" in quality_row) and capped
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
    for r in summary["runs"]:
        print(json.dumps({k: r.get(k) for k in ("label", "ok", "fps", "chosen", "gpu_lines",
                                               "errors", "shot")}, indent=2))
    print(json.dumps({k: summary.get(k) for k in ("ok", "graphics_page", "pixel_ratio", "compare", "error")},
                     indent=2))
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
