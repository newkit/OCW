#!/usr/bin/env python3
"""The bisect switches and the early page log (spec 14 Phase 10, 10G
*Bisecting a phone's crash*; 11.4).

    scripts/test-web-bisect.py [--out test-output/web-bisect] [--dir target/web]
                               [--cases iphone,android] [--switches all|nosee,ios=safe,…]
                               [--seconds 12] [--software] [--python PYTHON]

The iPhone of 2026-11-27 (Firefox for iOS, WebKit, 430 × 932 at 3) lost the
page within 2 s of the first frames on ?route=hills&gpu=webgl2&quality=low.
Playwright (the venv of `make deps-webkit`) on target/web/ (`make web`):
WebKit as an iPhone of that size (Safari's engine, touch, pixel ratio 3)
and the system's Chromium as an Android phone (WebGL 2). Each switch is one
ride of the Hills route with ?log=1&gpu=webgl2&quality=low&SWITCH:

  every ride   rides and draws (frames counted), no page error; the early
               log: the app's pipeline lines ("pipeline N ok in M ms: …"),
               the frame times of the first frames, the memory samples
               (wasm, frames), each line in sessionStorage at once (the
               stored lines are this load's lines); LiveKit's SDK is never
               fetched (a solo ride; the page's config names a voice
               server); a "Script error." without a file gets its note;
  base         (no switch) "render switches: 4 pipeline(s) per frame" (WebGL
               2's budget), the trees' see_through.wesl compiles;
  nosee, noarrays, plain   their shaders never compile;
  noshadow, nomsaa, noaudio, novoice, pipelines=serial   named in the log
               ("bisect switches: …");
  nosky        no clouds.wesl pipeline;  noegui   no egui pipeline;
  frames=30 / frames=0   "the cameras stop drawing", the page lives on;
  ios=safe     all of them, the ride still draws;
  the shadows  (v6.77) the iPhone (iOS WebKit, WebGL 2) rides without
               shadows by default ("shadows off on iOS WebKit with WebGL 2"
               in the log, the quality line "shadows off (iOS WebKit …)")
               in the saver memory mode; noshadow=0 turns them on (1024²),
               shadowmap=512&noshadow=0 at 512², shadowfix=hw2x2,near
               names the fix; Android keeps them (1024², no iOS line);
               every ride logs the renderer's GPU allocations ("gpu
               allocated N MB", "gpu ~N MB" in the memory samples);
  killed       a load whose pagehide never comes (the browser killed it):
               the next load names its last sign of life (the pulse: time,
               wasm, frames).

The riders' looks differ per ride (kit.design, workshop.paint).

  OUT/web-bisect-<time>-<phone>-<case>.png   the log panel over the ride
  OUT/web-bisect-<time>-<phone>-<case>.txt   the log text (ocwLog.report())
  OUT/web-bisect-<time>.json / .log          checks and the console

Exits 0 when every check passed, 2 without Playwright.
"""

import argparse
import datetime
import importlib.util
import json
import os
import re
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
VENV = os.path.join(os.path.expanduser("~"), ".cache", "ocw", "playwright-venv")
# A voice server named: the page must still not fetch LiveKit's SDK for a
# solo ride (it is loaded when a room is joined).
CONFIG = json.dumps({"worlds": "worlds/", "server": "", "voice": "wss://lk.example.invalid", "signin": "",
                     "build": "test-web-bisect"})
ANDROID_UA = ("Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) "
              "Chrome/131.0.0.0 Mobile Safari/537.36")
IPHONE_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) "
             "FxiOS/157.1 Mobile/15E148 Safari/605.1.15")
FAKE_GPU = """
if (!navigator.gpu) {
  Object.defineProperty(Navigator.prototype, "gpu", {get: () => ({
    requestAdapter: async () => ({features: new Set(), limits: {maxTextureDimension2D: 8192, maxBufferSize: 268435456},
      info: {vendor: "apple", architecture: "common-3"}}),
    getPreferredCanvasFormat: () => "bgra8unorm",
  }), configurable: true});
}
"""
ALL = ["base", "nosee", "noarrays", "plain", "noshadow", "nomsaa", "nosky", "noegui", "novoice", "noaudio",
       "pipelines=serial", "frames=30", "frames=0", "ios=safe", "killed",
       "noshadow=0", "shadowmap=512&noshadow=0", "shadowmap=1024", "shadowfix=hw2x2,near&noshadow=0"]
# The shadow cases (v6.77): what the quality line says about the shadows.
SHADOW_CASES = {
    "noshadow=0": "shadows 1024² × 1 over 70 m",
    "shadowmap=512&noshadow=0": "shadows 512² × 1 over 70 m",
    "shadowmap=1024": "shadows 1024² × 1 over 70 m",
    "shadowfix=hw2x2,near&noshadow=0": "shadows 1024² × 1 over 70 m",
}
IOS_OFF = "shadows off on iOS WebKit with WebGL 2: the shadow pass closed the page on an iPhone (iOS 18.7)"
ANDROID = ["base", "nosee", "noegui", "pipelines=serial", "ios=safe"]
# Our shaders and the switch that keeps them from compiling.
SHADERS = {
    "nosee": ["see_through.wesl"],
    "noarrays": ["road.wesl", "ground.wesl", "gear.wesl", "building.wesl"],
    "plain": ["see_through.wesl", "road.wesl", "ground.wesl", "gear.wesl", "building.wesl", "water.wesl", "far.wesl"],
    "nosky": ["clouds.wesl"],
    "ios=safe": ["see_through.wesl", "road.wesl", "ground.wesl", "gear.wesl", "water.wesl", "far.wesl", "clouds.wesl"],
    "frames=0": ["see_through.wesl", "road.wesl", "ground.wesl", "gear.wesl"],
}
# The riders' looks (made-up kits, one per ride).
LOOKS = [
    ("band", "#C62828", "#FDD835", "#212121"), ("sash", "#1565C0", "#FAFAFA", "#E53935"),
    ("stripes", "#2E7D32", "#00897B", "#FFFFFF"), ("chevrons", "#6A1B9A", "#F5F5F5", "#FFB300"),
    ("gradient", "#FF6F00", "#263238", "#4FC3F7"), ("plain", "#AD1457", "#F8BBD0", "#FAFAFA"),
    ("band", "#43A047", "#212121", "#43A047"), ("sash", "#F9A825", "#3949AB", "#1E1E22"),
]


def in_venv():
    try:
        import playwright  # noqa: F401
        return True
    except ImportError:
        return False


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-bisect")
    p.add_argument("--cases", default="iphone,android")
    p.add_argument("--switches", default="all")
    p.add_argument("--seconds", type=float, default=12.0)
    p.add_argument("--software", action="store_true", help="Chromium's WebGL 2 on SwiftShader")
    p.add_argument("--python", default=None)
    a = p.parse_args()
    if not in_venv():
        py = a.python or os.path.join(VENV, "bin", "python")
        if not os.path.exists(py):
            print("test-web-bisect: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    from playwright.sync_api import sync_playwright

    ws = load("web_serve", "web-serve.py")
    missing = ws.missing_build(a.dir, ["ocw-log.js", "ocw-gpu.js", "ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-bisect-" + stamp)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    checks, lines, shots = [], [], []
    t0 = time.time()
    summary = {"ok": False, "cases": {}}
    look_n = [0]

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:600]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:200]}", flush=True)

    def context(browser, pw, phone):
        if phone == "iphone":
            # The reporting phone: 430 × 932 CSS px at 3 (Firefox for iOS
            # 157 on iOS 18.7: WebKit); the browser bars take ~190 px.
            ctx = browser.new_context(viewport={"width": 430, "height": 740}, screen={"width": 430, "height": 932},
                                      device_scale_factor=3, is_mobile=True, has_touch=True, user_agent=IPHONE_UA)
            ctx.add_init_script(FAKE_GPU)
        else:
            ctx = browser.new_context(viewport={"width": 412, "height": 915}, device_scale_factor=2.625,
                                      is_mobile=True, has_touch=True, user_agent=ANDROID_UA)
        ctx.add_init_script("addEventListener('pagehide', (e) => { if (window.__noPagehide) e.stopImmediatePropagation(); }, true);")
        ctx.route("**/config.json", lambda route: route.fulfill(status=200, content_type="application/json", body=CONFIG))
        fetched = []
        ctx.on("request", lambda r: fetched.append(r.url))
        pg = ctx.new_page()
        errors = []
        pg.on("console", lambda m: lines.append(f"[{phone}] {time.time() - t0:7.2f} s  {m.type}: {m.text[:400]}"))
        pg.on("pageerror", lambda e: (errors.append(str(e)), lines.append(f"[{phone}] PAGEERROR {e}")))
        return ctx, pg, fetched, errors

    def ride(pg, phone, case):
        try:
            pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                 " && window.ocwFrames > 20", timeout=120000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{phone} {case}] wait: {e}")
        # A different look per ride (the riders' variety in pictures).
        pattern, jersey, second, helmet = LOOKS[look_n[0] % len(LOOKS)]
        look_n[0] += 1
        for cmd in (f"kit.design --pattern {pattern} --jersey {jersey} --second {second} --number {look_n[0]}",
                    f"workshop.paint {helmet}", "camera.orbit 150 12 7"):
            try:
                pg.evaluate("(c) => window.ocw && window.ocw.console(c)", cmd)
            except Exception as e:  # noqa: BLE001
                lines.append(f"[{phone} {case}] {cmd}: {e}")
        pg.wait_for_timeout(int(a.seconds * 1000))

    def query(case):
        q = "route=hills&bots=1&touch=on&log=1&gpu=webgl2&quality=low"
        return q if case in ("base", "killed") else q + "&" + (case if "=" in case else case + "=1")

    def one(browser, pw, phone, case):
        ctx, pg, fetched, errors = context(browser, pw, phone)
        url = root + "?" + query(case)
        if case == "killed":
            # The first load never sees its pagehide (the browser killed it).
            pg.goto(url)
            pg.wait_for_function("() => window.ocwFrames > 20", timeout=120000)
            pg.wait_for_timeout(3000)
            pg.evaluate("window.__noPagehide = true")
            pg.goto(url)
            pg.wait_for_function("() => window.ocwLog && window.ocwFrames > 5", timeout=120000)
            st = json.loads(pg.evaluate("window.ocwLog.state()"))
            pulse = st.get("previous_pulse") or {}
            check(f"{phone} killed: the load before ended without pagehide", st.get("previous_ended") in ("", None),
                  st.get("previous_ended"))
            check(f"{phone} killed: its last sign of life is kept (time, wasm, frames)",
                  pulse.get("t", 0) >= 2000 and pulse.get("frames", 0) > 20 and pulse.get("wasm", 0) > 0, pulse)
            text = pg.evaluate("window.ocwLog.report()")
            check(f"{phone} killed: the report names it", "last sign of life at +" in text,
                  [ln for ln in text.split("\n") if ln.startswith("--- the page load before")][:1])
            pg.wait_for_timeout(1500)
            finish(pg, phone, case, text, ctx)
            return
        pg.goto(url)
        # The early log at work: every line already in sessionStorage.
        try:
            pg.wait_for_function("() => window.ocwLog && window.ocwFrames > 5", timeout=120000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{phone} {case}] early: {e}")
        stored = pg.evaluate("""() => { const r = JSON.parse(sessionStorage.getItem('ocw.log.run') || '{}');
            return {stored: (r.lines || []).length, lines: window.ocwLog.lines().length}; }""")
        check(f"{phone} {case}: early lines are in sessionStorage at once", stored["stored"] == stored["lines"] > 0, stored)
        ride(pg, phone, case)
        frames = pg.evaluate("window.ocwFrames")
        pg.evaluate("() => dispatchEvent(new ErrorEvent('error', {message: 'Script error.'}))")
        pg.wait_for_timeout(300)
        text = pg.evaluate("window.ocwLog.report()")
        log = text.split("--- this page load")[-1]
        compiled = re.findall(r"pipeline \d+ ok in \d+ ms: [^\n]*", log)
        summary["cases"][f"{phone} {case}"] = {"frames": frames, "pipelines": len(compiled)}
        check(f"{phone} {case}: rides (frames {frames}), no page error", frames > 20 and not errors, errors[:2])
        if case != "frames=0":
            check(f"{phone} {case}: the log has the pipeline compiles", len(compiled) >= 3, compiled[:3])
        check(f"{phone} {case}: the log has the first frames' times", re.search(r" P frame 1: \d+ ms", log), "")
        check(f"{phone} {case}: the log has the memory samples", re.search(r" P memory: wasm \d+ MB.*frames \d+", log), "")
        if case != "frames=0":
            check(f"{phone} {case}: the log has the GPU allocations",
                  re.search(r"gpu allocated \d+ MB \(textures \d+ MB in \d+, buffers", log)
                  and re.search(r" P memory: wasm \d+ MB.*gpu ~\d+ MB \(renderer \d+ MB", log),
                  [ln for ln in log.split("\n") if "gpu allocated" in ln][:1])
        quality = (re.findall(r"quality: low \([^\n]*", log) or [""])[0]
        ios_default = phone == "iphone" and "noshadow" not in case and "shadowmap" not in case and case != "ios=safe"
        if ios_default:
            check(f"{phone} {case}: shadows off by default on iOS WebKit (log, quality line)",
                  IOS_OFF in log and "shadows off (iOS WebKit with WebGL 2" in quality, quality[:200])
            check(f"{phone} {case}: the saver memory mode on iOS", "world memory: saver" in log, "")
        if phone == "android":
            check(f"{phone} {case}: Android keeps its shadows (no iOS default)",
                  IOS_OFF not in log and ("shadows 1024² × 1" in quality or case == "ios=safe"), quality[:200])
        if case in SHADOW_CASES:
            check(f"{phone} {case}: the shadows are on as asked", SHADOW_CASES[case] in quality and IOS_OFF not in log,
                  quality[:200])
        check(f"{phone} {case}: a 'Script error.' gets its note", "details are hidden" in log, "")
        sdk = [u for u in fetched if "livekit-client" in u]
        check(f"{phone} {case}: LiveKit's SDK is not fetched for a solo ride", not sdk, sdk)
        names = (re.findall(r"(?:bisect|render) switches: [^\n]*", log) or [""])
        joined = " | ".join(names)
        if case in SHADOW_CASES:
            if "shadowmap=" in case:
                n = re.search(r"shadowmap=(\d+)", case).group(1)
                check(f"{phone} {case}: the log names the size", f"shadow map: {n}² at most" in log, "")
            if "shadowfix" in case:
                check(f"{phone} {case}: the log names the fix", "shadowfix=hw2x2,near" in joined, joined)
        elif case == "base":
            check(f"{phone} base: WebGL 2 paces the pipelines (4 per frame)", "4 pipeline(s) per frame" in joined, joined)
            check(f"{phone} base: the trees' shader compiles", any("see_through.wesl" in c for c in compiled),
                  [c for c in compiled if "ocw_render" in c][:4])
        else:
            key = {"pipelines=serial": "1 pipeline(s) per frame", "frames=30": "frames=30", "frames=0": "frames=0",
                   "ios=safe": "nosee, noarrays, plain, noshadow, nomsaa, nosky, noegui"}.get(case, case)
            check(f"{phone} {case}: the log names the switch", key in joined, joined)
        for shader in SHADERS.get(case, []):
            check(f"{phone} {case}: {shader} never compiles", not any(shader in c for c in compiled),
                  [c for c in compiled if shader in c][:2])
        if case == "noegui":
            check(f"{phone} noegui: no egui pipeline", not any("egui" in c.lower() for c in compiled), "")
        if case.startswith("frames="):
            check(f"{phone} {case}: the cameras stop drawing", "the cameras stop drawing" in log, "")
        if case == "novoice" or case == "ios=safe":
            check(f"{phone} {case}: voice is off for this visit", "voice off for this visit" in log, "")
        finish(pg, phone, case, text, ctx)

    def finish(pg, phone, case, text, ctx):
        name = case.replace("=", "-").replace("&", "_").replace(",", "+")
        with open(f"{base}-{phone}-{name}.txt", "w") as f:
            f.write(text)
        path = f"{base}-{phone}-{name}.png"
        pg.screenshot(path=path)
        shots.append(path)
        # The early log's new lines: the panel scrolled to the first
        # pipeline compile (or the load before's title), at full height.
        scrolled = pg.evaluate("""() => { const p = document.getElementById('ocw-log');
            if (!p || p.hidden) return false;
            p.classList.add('full');
            const body = p.querySelector('.body');
            const want = /--- the page load before|pipeline \\d+ (ok|waits)|frames=\\d+: the cameras/;
            const el = [...body.childNodes].find((n) => want.test(n.textContent || ''));
            if (!el || !el.offsetTop) return false;
            body.scrollTop = el.offsetTop - body.offsetTop - 4;
            return true; }""")
        if scrolled:
            pg.wait_for_timeout(300)
            path = f"{base}-{phone}-{name}-early-log.png"
            pg.screenshot(path=path)
            shots.append(path)
        ctx.close()

    phones = [c.strip() for c in a.cases.split(",") if c.strip()]
    with sync_playwright() as pw:
        for phone in phones:
            try:
                if phone == "iphone":
                    browser = pw.webkit.launch()
                else:
                    web_browser = load("web_browser", "web_browser.py")
                    exe = web_browser.find()
                    args = ["--ignore-gpu-blocklist", "--disable-features=WebGPU,WebGPUService",
                            "--disable-blink-features=WebGPU", "--no-proxy-server", "--password-store=basic"]
                    args += (["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] if a.software
                             else ["--enable-features=Vulkan", "--use-angle=vulkan"])
                    browser = pw.chromium.launch(executable_path=exe, args=args)
                summary[phone] = browser.version
                cases = ALL if phone == "iphone" else ANDROID
                if a.switches != "all":
                    cases = [c for c in a.switches.split(",") if c]
                for case in cases:
                    try:
                        one(browser, pw, phone, case)
                    except Exception as e:  # noqa: BLE001
                        check(f"{phone} {case}: the run finished", False, repr(e))
                browser.close()
            except Exception as e:  # noqa: BLE001
                summary.setdefault("errors", []).append(f"{phone}: {e!r}")
                check(f"{phone}: the run finished", False, repr(e))
        summary["ok"] = bool(checks) and all(c["ok"] for c in checks)
    httpd.shutdown()
    summary["checks"] = checks
    summary["screenshots"] = shots
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    failed = [c for c in checks if not c["ok"]]
    print(f"{len(checks) - len(failed)} of {len(checks)} checks passed; log: {base}.log")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
