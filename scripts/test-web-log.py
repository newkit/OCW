#!/usr/bin/env python3
"""The page log on phones (spec 14 Phase 10, 10G *The page log*; 11.4).

    scripts/test-web-log.py [--out test-output/web-log] [--dir target/web]
                            [--cases iphone,android] [--software] [--python PYTHON]

Playwright (the venv of `make deps-webkit`) on target/web/ (`make web`):
WebKit with an iPhone 15 (Safari's engine, 393 × 659, pixel ratio 3, touch)
and the system's Chromium (scripts/web_browser.py) as an Android phone
(Pixel 7: 412 × 915, pixel ratio 2.625, touch, Chrome's Android user agent;
WebGL 2 – headless WebGPU is not a phone's). For each phone:

  ride      ?log=1 on the Hills ride: the panel is there at once (before the
            wasm), shows the environment header (user agent, screen and
            pixel ratio, WebGPU / WebGL 2, SIMD and threads, memory, the
            page build from config.json, the chosen backend, the address
            with the rider's name hidden) and the lines: the page's
            `ocw-web:` lines and the app's Rust lines ("rs …"); a
            console.error, an uncaught error (with file:line) and an
            unhandled rejection arrive; Copy (Chromium: read back from the
            clipboard; WebKit: copied or the text selected), Download (a
            .txt with the same text), the ring buffer keeps 2,000 lines;
            Hide hides it, a reload keeps it hidden;
  previous  a reload shows the load before ("closed normally"); a load
            whose pagehide never comes (as when the browser kills the page)
            shows as ended without pagehide, with its last line;
  taps      without ?log=1 five taps on the version stamp (bottom right)
            open it;
  too-old   no WebAssembly SIMD: the "too old" screen offers Show log, and
            the log has the cause;
  no-wasm   the .wasm blocked: "Start failed" offers Show log with the
            failed load in the log;
  no-gpu    (Android) neither WebGPU nor WebGL 2: the error offers Show log;
  retry     the module's download (spec 10G *Module download*): the first
            request of the .wasm aborted (Safari's "Load failed"), and once
            cut to half its bytes (a partial copy): the page shows the
            progress ("Downloading the app: x / y MB"), says "the download
            broke off – trying again (2/3)" (in the log too), downloads it
            again past the HTTP cache and rides; with every request aborted
            (no-wasm) "Start failed" comes after three tries, with Try again.

  OUT/web-log-<time>-<phone>-<case>.png   screenshots (phone size)
  OUT/web-log-<time>-<phone>-<case>.txt   the log text as Copy / Download give it
  OUT/web-log-<time>.json / .log          checks and the console

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
CONFIG = json.dumps({"worlds": "worlds/", "server": "", "voice": "", "signin": "", "build": "test-web-log"})
ANDROID_UA = ("Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) "
              "Chrome/131.0.0.0 Mobile Safari/537.36")
# Safari 26's navigator.gpu (WebKit on Linux has none): the page takes WebGL 2.
FAKE_GPU = """
if (!navigator.gpu) {
  Object.defineProperty(Navigator.prototype, "gpu", {get: () => ({
    requestAdapter: async () => ({features: new Set(), limits: {maxTextureDimension2D: 8192, maxBufferSize: 268435456},
      info: {vendor: "apple", architecture: "common-3"}}),
    getPreferredCanvasFormat: () => "bgra8unorm",
  }), configurable: true});
}
"""
NO_SIMD = "WebAssembly.validate = () => false;\n"
NO_GPU = """
Object.defineProperty(Navigator.prototype, "gpu", {get: () => undefined, configurable: true});
const __getContext = HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext = function (kind, ...rest) {
  return kind === "webgl2" ? null : __getContext.call(this, kind, ...rest);
};
"""


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
    p.add_argument("--out", default="test-output/web-log")
    p.add_argument("--cases", default="iphone,android")
    p.add_argument("--software", action="store_true", help="Chromium's WebGL 2 on SwiftShader")
    p.add_argument("--python", default=None)
    a = p.parse_args()
    if not in_venv():
        py = a.python or os.path.join(VENV, "bin", "python")
        if not os.path.exists(py):
            print("test-web-log: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    from playwright.sync_api import sync_playwright

    ws = load("web_serve", "web-serve.py")
    missing = ws.missing_build(a.dir, ["ocw-log.js", "ocw-gpu.js", "ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-log-" + stamp)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    checks, lines, shots = [], [], []
    t0 = time.time()
    summary = {"ok": False}

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:600]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:200]}", flush=True)

    def shot(pg, phone, case):
        path = f"{base}-{phone}-{case}.png"
        pg.screenshot(path=path)
        shots.append(path)
        return path

    def state(pg):
        return json.loads(pg.evaluate("window.ocwLog.state()"))

    def report(pg):
        return pg.evaluate("window.ocwLog.report()")

    def save_text(phone, case, text):
        with open(f"{base}-{phone}-{case}.txt", "w") as f:
            f.write(text)

    def panel_text(pg):
        return pg.evaluate("(() => { const p = document.getElementById('ocw-log');"
                           " return p && !p.hidden ? p.innerText : ''; })()")

    def wait_ride(pg, label):
        try:
            pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                 " && window.ocwFrames > 20", timeout=120000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{label}] wait: {e}")
        pg.wait_for_timeout(4000)

    def context(browser, pw, phone, init=""):
        if phone == "iphone":
            ctx = browser.new_context(**pw.devices["iPhone 15"], accept_downloads=True)
            ctx.add_init_script(FAKE_GPU)
        else:
            ctx = browser.new_context(viewport={"width": 412, "height": 915}, device_scale_factor=2.625,
                                      is_mobile=True, has_touch=True, user_agent=ANDROID_UA,
                                      accept_downloads=True)
            try:
                ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=root.rstrip("/"))
            except Exception as e:  # noqa: BLE001
                lines.append(f"[{phone}] clipboard permissions: {e}")
        if init:
            ctx.add_init_script(init)
        # window.__noPagehide = true: this load's pagehide never reaches the
        # page (as when the browser kills the page).
        ctx.add_init_script("addEventListener('pagehide', (e) => { if (window.__noPagehide) e.stopImmediatePropagation(); }, true);")
        ctx.route("**/config.json", lambda route: route.fulfill(status=200, content_type="application/json", body=CONFIG))
        pg = ctx.new_page()
        pg.on("console", lambda m: lines.append(f"[{phone}] {time.time() - t0:7.2f} s  {m.type}: {m.text[:400]}"))
        pg.on("pageerror", lambda e: lines.append(f"[{phone}] PAGEERROR {e}"))
        return ctx, pg

    def ride_case(browser, pw, phone):
        ctx, pg = context(browser, pw, phone)
        url = root + "?route=hills&bots=1&touch=on&log=1&name=Ada%20Rider&gpu=webgl2&quality=low"
        pg.goto(url)
        # Before the wasm: the panel and the page's first lines.
        pg.wait_for_selector("#ocw-log:not([hidden])", timeout=15000)
        early = state(pg)
        check(f"{phone}: ?log=1 shows the panel at once, before the app started",
              early["shown"] and early["built"] and early["lines"] >= 1, early)
        wait_ride(pg, phone)
        pg.evaluate("""() => { console.error('ocw-test: an error line for the log');
            const s = document.createElement('script');
            s.src = URL.createObjectURL(new Blob(["throw new Error('ocw-test: an uncaught error');"], {type: 'text/javascript'}));
            document.head.appendChild(s);
            Promise.reject(new Error('ocw-test: an unhandled rejection')); }""")
        pg.wait_for_timeout(1500)
        text = report(pg)
        save_text(phone, "ride", text)
        head = text.split("\n--- ")[0]
        for key in ("user agent:", "screen:", "pixel ratio", "WebGPU:", "WebGL 2: yes", "SIMD yes", "threads",
                    "memory:", "page build: test-web-log", "backend: webgl2"):
            check(f"{phone}: the header has {key!r}", key in head, head[:400])
        address = [ln for ln in head.split("\n") if ln.startswith("address")]
        check(f"{phone}: the address in the header hides the rider's name", address and "name=(hidden)" in address[0]
              and "Ada" not in address[0], address)
        check(f"{phone}: the header has the app's build id", "app build: (the app has not started)" not in head
              and "app build: " in head, [ln for ln in head.split("\n") if ln.startswith("app build")])
        body = text.split("--- this page load")[-1]
        check(f"{phone}: the page's own lines are captured (ocw-web: GPU API …)", "ocw-web: GPU API webgl2" in body)
        check(f"{phone}: the app's Rust lines are captured, without the %c styles",
              " rs " in body and "Open Cycling World" in body and "%c" not in body and "color:" not in body,
              [ln for ln in body.split("\n") if " rs " in ln][:3])
        check(f"{phone}: no home folder in the log (the app's source paths shortened: cargo:…, ~/…)",
              "/home/" not in text and "/Users/" not in text and "cargo:" in text,
              [ln for ln in text.split("\n") if "/home/" in ln][:2])
        # The GPU memory estimate (10G, v6.77): the renderer's allocations
        # and the memory samples with the GPU.
        check(f"{phone}: the log has the renderer's GPU allocations",
              re.search(r"gpu allocated \d+ MB \(textures \d+ MB in \d+, buffers \d+ MB in \d+; peak \d+ MB\)", body)
              and re.search(r" P memory: wasm \d+ MB.*gpu ~\d+ MB \(renderer \d+ MB", body),
              [ln for ln in body.split("\n") if "gpu " in ln][:2])
        ios_off = "shadows off on iOS WebKit with WebGL 2" in body
        check(f"{phone}: the iOS shadow default only on the iPhone", ios_off == (phone == "iphone"),
              [ln for ln in body.split("\n") if "shadows" in ln][:2])
        photos_off = "ride photos off on iOS WebKit with WebGL 2: taking the first photo closed the page" in body
        check(f"{phone}: the iOS ride-photo default only on the iPhone (10G, v6.86)", photos_off == (phone == "iphone"),
              [ln for ln in body.split("\n") if "ride photo" in ln][:2])
        check(f"{phone}: console.error arrives as E", " E ocw-test: an error line for the log" in body)
        uncaught = [ln for ln in body.split("\n") if "ocw-test: an uncaught error" in ln]
        check(f"{phone}: an uncaught error arrives with file:line", uncaught and " ! uncaught" in uncaught[0]
              and ":" in uncaught[0].split("(")[-1], uncaught[:1])
        check(f"{phone}: an unhandled rejection arrives", "unhandled promise rejection: Error: ocw-test: an unhandled rejection"
              in body)
        check(f"{phone}: timestamps relative to the page's start (+s.mmm)", body.strip().split("\n")[1].startswith("+"),
              body.strip().split("\n")[1:3])
        shown = panel_text(pg)
        check(f"{phone}: the panel shows the header and the lines", "user agent:" in shown and "ocw-web: GPU API" in shown,
              shown[:200])
        box = pg.evaluate("(() => { const r = document.getElementById('ocw-log').getBoundingClientRect();"
                          " return [r.left, r.top, r.width, r.height, innerWidth, innerHeight]; })()")
        check(f"{phone}: the panel fits the phone's screen", box[0] >= 0 and box[1] >= 0 and box[0] + box[2] <= box[4] + 0.5
              and box[1] + box[3] <= box[5] + 0.5, box)
        btn = pg.evaluate("(() => { const r = document.querySelector('#ocw-log button[data-log=copy]').getBoundingClientRect();"
                          " return [r.width, r.height]; })()")
        check(f"{phone}: the buttons are finger-sized (≥ 40 px high)", btn[1] >= 40, btn)
        shot(pg, phone, "ride")
        # Copy.
        pg.click("#ocw-log button[data-log=copy]")
        pg.wait_for_timeout(1000)
        note = pg.evaluate("document.querySelector('#ocw-log .note').textContent")
        if phone == "android":
            try:
                clip = pg.evaluate("navigator.clipboard.readText()")
            except Exception as e:  # noqa: BLE001
                clip = f"(read failed: {e})"
            check("android: Copy puts the log on the clipboard", "user agent:" in clip and "--- this page load" in clip,
                  note + " | " + clip[:120])
        else:
            check("iphone: Copy copies (or selects the text for the phone's Copy)",
                  note.startswith("Copied") or "selected" in note, note)
        shot(pg, phone, "copy")
        # Download.
        # The log's .txt (a ride photo may download meanwhile on Android:
        # the ride photographer's JPEG; the iPhone takes none, v6.86).
        with pg.expect_download(predicate=lambda d: d.suggested_filename.endswith(".txt"), timeout=15000) as dl:
            pg.click("#ocw-log button[data-log=download]")
        path = dl.value.path()
        got = open(path, encoding="utf-8").read() if path else ""
        check(f"{phone}: Download gives a .txt with the log", dl.value.suggested_filename.endswith(".txt")
              and "user agent:" in got and "ocw-web: GPU API" in got, dl.value.suggested_filename)
        share = pg.evaluate("!!navigator.share")
        hidden = pg.evaluate("document.querySelector('#ocw-log button[data-log=share]').hidden")
        check(f"{phone}: Share is offered exactly where the browser can share", hidden == (not share),
              {"navigator.share": share, "button hidden": hidden})
        # The ring buffer.
        pg.evaluate("(() => { for (let i = 0; i < 2600; i++) window.ocwLog.add('ocw-test: filler ' + i); })()")
        st = state(pg)
        check(f"{phone}: the ring buffer keeps the last ~2,000 lines", 2000 <= st["lines"] <= 2200 and st["dropped"] > 0
              and any(ln.endswith("filler 2599") for ln in pg.evaluate("window.ocwLog.lines().slice(-30)")), st)
        pg.wait_for_timeout(300)
        st = state(pg)
        check(f"{phone}: the panel follows the buffer (its lines stay bounded)", st["rendered"] <= 2400, st)
        # Previous load.
        pg.wait_for_timeout(2000)
        pg.reload()
        pg.wait_for_selector("#ocw-log:not([hidden])", timeout=15000)
        prev = pg.evaluate("window.ocwLog.previous()") or {}
        check(f"{phone}: a reload shows the load before (closed normally)", len(prev.get("lines", [])) >= 1000
              and "closed normally" in (prev.get("ended") or ""), {k: prev.get(k) for k in ("started", "ended")})
        check(f"{phone}: the panel shows the 'page load before' section", "the page load before" in panel_text(pg))
        pg.click("#ocw-log button[data-log=clear]")
        pg.wait_for_timeout(300)
        st = state(pg)
        cleared = pg.evaluate("window.ocwLog.lines().filter((l) => l.includes('page log started')).length")
        check(f"{phone}: Clear empties both loads", st["previous"] == 0 and cleared == 0, {"state": st, "start lines": cleared})
        wait_ride(pg, phone)
        shot(pg, phone, "reload")
        pg.click("#ocw-log button[data-log=hide]")
        st = state(pg)
        check(f"{phone}: Hide hides the panel", not st["shown"] and pg.evaluate("document.getElementById('ocw-log').hidden"), st)
        pg.goto(root + "?route=hills&bots=1&touch=on&gpu=webgl2&quality=low")
        pg.wait_for_timeout(2000)
        st = state(pg)
        check(f"{phone}: after Hide a load without ?log=1 keeps it hidden and unbuilt", not st["shown"] and not st["built"], st)
        # A page the browser killed (iOS, out of memory) ends without
        # pagehide: the test's init script swallows this load's pagehide
        # (Playwright cannot go on with a crashed page).
        pg.goto(root + "?route=hills&bots=1&touch=on&gpu=webgl2&quality=low")
        pg.wait_for_timeout(1500)
        pg.evaluate("window.ocwLog.add('ocw-test: the last line before the kill'); window.__noPagehide = true")
        pg.wait_for_timeout(2500)
        pg.reload()
        pg.wait_for_timeout(2000)
        prev = pg.evaluate("window.ocwLog.previous()") or {}
        check(f"{phone}: a load killed without pagehide: the next load says so and has its last line",
              not prev.get("ended") and any("the last line before the kill" in ln for ln in prev.get("lines", [])),
              {"ended": prev.get("ended"), "lines": len(prev.get("lines", []))})
        pg.evaluate("window.ocwLog.show()")
        pg.wait_for_timeout(800)
        pg.evaluate("(() => { const b = document.querySelector('#ocw-log .body'); const s = b.querySelector('.sec');"
                    " b.scrollTop = Math.max(0, s.offsetTop - 8); })()")
        pg.wait_for_timeout(300)
        check(f"{phone}: the panel says the load before ended without pagehide",
              "ended without pagehide" in panel_text(pg))
        shot(pg, phone, "killed")
        ctx.close()

    def taps_case(browser, pw, phone):
        ctx, pg = context(browser, pw, phone)
        pg.goto(root + "?route=hills&bots=1&touch=on&gpu=webgl2&quality=low")
        wait_ride(pg, phone)
        st = state(pg)
        check(f"{phone}: without ?log=1 the panel is not built (no cost)", not st["built"] and st["lines"] > 0, st)
        w, h = pg.evaluate("[innerWidth, innerHeight]")
        for _ in range(5):
            pg.touchscreen.tap(w - 40, h - 8)
            pg.wait_for_timeout(150)
        pg.wait_for_timeout(500)
        st = state(pg)
        check(f"{phone}: five taps on the version stamp open the log", st["shown"], st)
        shot(pg, phone, "taps")
        ctx.close()

    def offer_case(browser, pw, phone, case, init, url, cause, block_wasm=False):
        ctx, pg = context(browser, pw, phone, init)
        if block_wasm:
            pg.route("**/*.wasm", lambda route: route.abort())
        pg.goto(url)
        try:
            pg.wait_for_selector("#ocw-log-offer:not([hidden])", timeout=60000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{phone} {case}] offer: {e}")
        st = state(pg)
        status = pg.evaluate("document.getElementById('ocw-status').innerText")
        check(f"{phone} {case}: the error screen offers Show log without ?log=1", st["offered"] and not st["shown"],
              {"state": st, "status": status[:200]})
        shot(pg, phone, case)
        if st["offered"]:
            pg.click("#ocw-log-offer")
            pg.wait_for_timeout(1500)
        text = report(pg)
        save_text(phone, case, text)
        check(f"{phone} {case}: the log has the cause ({cause})", cause in text,
              [ln for ln in text.split("\n") if " E " in ln or " W " in ln or " ! " in ln][:4])
        shot(pg, phone, case + "-log")
        ctx.close()

    # Every text the loading screen showed (sampled every 50 ms).
    SEEN = """
window.__ocwSeen = [];
setInterval(() => {
  const s = document.getElementById("ocw-status");
  const t = s && !s.hidden ? s.textContent : "";
  if (t && window.__ocwSeen[window.__ocwSeen.length - 1] !== t) window.__ocwSeen.push(t);
}, 50);
// The cache mode of each request of the module (the retries go past the
// HTTP cache: "reload").
window.__ocwWasmCache = [];
const __fetch = window.fetch;
window.fetch = function (input, init) {
  if (String(input).endsWith(".wasm")) window.__ocwWasmCache.push((init && init.cache) || "default");
  return __fetch.call(this, input, init);
};
"""

    def retry_case(browser, pw, phone, how):
        """The first .wasm request broken (`abort`: a network error; `half`:
        half the bytes, a damaged copy): the page tries again and rides."""
        ctx, pg = context(browser, pw, phone, SEEN)
        asked = []

        def wasm(route):
            asked.append(route.request.url)
            if len(asked) > 1:
                route.continue_()
            elif how == "abort":
                route.abort()
            else:
                body = route.fetch().body()
                route.fulfill(status=200, headers={"content-type": "application/wasm"}, body=body[:len(body) // 2])

        pg.route("**/*.wasm", wasm)
        pg.goto(root + "?route=hills&bots=1&gpu=webgl2&quality=low")
        wait_ride(pg, f"{phone} retry-{how}")
        seen = pg.evaluate("window.__ocwSeen")
        text = report(pg)
        save_text(phone, "retry-" + how, text)
        started = pg.evaluate("window.ocwFrames > 20 && document.getElementById('ocw-status').hidden")
        check(f"{phone} retry-{how}: the ride starts after a broken download of the module", started,
              {"seen": seen[-6:], "requests": len(asked)})
        modes = pg.evaluate("window.__ocwWasmCache")
        check(f"{phone} retry-{how}: the module was asked twice, the second time past the cache",
              len(asked) == 2 and modes == ["default", "reload"], {"requests": len(asked), "cache": modes})
        check(f"{phone} retry-{how}: the loading screen said it tries again (2/3)",
              any("The download broke off – trying again (2/3)" in t for t in seen), seen[:8])
        check(f"{phone} retry-{how}: the log says so", "trying again (2/3)" in text,
              [ln for ln in text.split("\n") if "download" in ln][:4])
        progress = [t for t in seen if re.search(r"Downloading the app: \d+\.\d / \d+\.\d MB", t)]
        check(f"{phone} retry-{how}: the download's progress is shown (x / y MB)", progress, seen[:8])
        check(f"{phone} retry-{how}: no Start failed", not any("Start failed" in t for t in seen), seen[-4:])
        shot(pg, phone, "retry-" + how)
        ctx.close()

    def try_again_case(browser, pw, phone):
        """Every .wasm request aborted: three tries, then "Start failed" with
        Try again (a new load of the page)."""
        ctx, pg = context(browser, pw, phone, SEEN)
        asked = []

        def wasm(route):
            asked.append(1)
            route.abort()

        pg.route("**/*.wasm", wasm)
        pg.goto(root + "?route=hills&bots=1&gpu=webgl2&quality=low")
        try:
            pg.wait_for_selector("#ocw-try-again", timeout=60000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{phone} try-again] wait: {e}")
        seen = pg.evaluate("window.__ocwSeen")
        status = pg.evaluate("document.getElementById('ocw-status').innerText")
        check(f"{phone} try-again: three tries, then Start failed with Try again",
              len(asked) == 3 and "Start failed" in status and "Try again" in status
              and any("(3/3)" in t for t in seen), {"requests": len(asked), "status": status[:200]})
        shot(pg, phone, "try-again")
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
                ride_case(browser, pw, phone)
                taps_case(browser, pw, phone)
                offer_case(browser, pw, phone, "too-old", NO_SIMD,
                           root + "?route=hills&bots=1", "no WebAssembly SIMD")
                offer_case(browser, pw, phone, "no-wasm", "", root + "?route=hills&bots=1&gpu=webgl2&quality=low",
                           "Start failed", block_wasm=True)
                retry_case(browser, pw, phone, "abort")
                retry_case(browser, pw, phone, "half")
                try_again_case(browser, pw, phone)
                if phone == "android":
                    offer_case(browser, pw, phone, "no-gpu", NO_GPU, root + "?route=hills&bots=1",
                               "neither WebGPU nor WebGL 2")
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
