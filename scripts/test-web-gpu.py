#!/usr/bin/env python3
"""The page's GPU choice and the WebGPU fallback (spec §14 Phase 10 B7, B8a; §11.4).

    scripts/test-web-gpu.py [--out test-output/web] [--dir target/web]
                            [--chromium chromium] [--part logic|fallback|all]

The regression test of the iPhone report of 2026-11-15 (iOS 26, Safari's
WebGPU on by default: sound, but no image). Headless Chromium with WebGPU
(the pinned browser, scripts/web_browser.py) on target/web/ (`make web`,
both builds):

  logic     apps/web/web/ocw-gpu.js's rules as pure functions in the page:
            the GPU choice (Chrome, Safari on macOS / iPhone / iPad, Chrome
            on iOS, ?gpu=, a remembered failure and an updated browser, no
            adapter, no WebGL 2, a storage that throws), the fallback URL,
            the start page's checkbox, the one-colour check;
  fallback  the real app with a WebGPU failure injected before the page's
            scripts (DevTools Page.addScriptToEvaluateOnNewDocument):
              uncaptured  a validation error 6 s after the device exists
                          (also Bevy's renderer: "renderer stopped", never
                          "Quitting the application"),
              device      requestDevice rejected,
              noframe     the canvas' configure throws (wgpu reports the
                          surface lost: no frame is ever drawn);
            each must reload the ride on WebGL 2 (gpu=webgl2&quality=low&
            webgpu-failed=1) with the note, remember the failure and draw.
            Then: the remembered failure takes WebGL 2 without ?gpu=;
            ?gpu=webgpu forces WebGPU, which draws and forgets the failure;
            a healthy WebGPU ride stays on WebGPU (no false alarm); Safari
            on an iPhone 15 (its user agent, navigator.vendor, touch, a
            device pixel ratio of 3; Chromium's WebGPU adapter present)
            rides on WebGL 2 by default and draws.

  OUT/web-gpu-<time>-<case>.jpg   a screenshot per ride
  OUT/web-gpu-<time>.json / .log  the checks, the console

Exits 0 when every check passed.
"""

import argparse
import datetime
import importlib.util
import json
import os
import shutil
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

IPHONE_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 "
             "(KHTML, like Gecko) Version/26.1 Mobile/15E148 Safari/604.1")
SAFARI_UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 "
             "(KHTML, like Gecko) Version/26.6 Safari/605.1.15")
CHROME_UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
             "Chrome/157.0.0.0 Safari/537.36")
IOS_CHROME_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 "
                 "(KHTML, like Gecko) CriOS/141.0 Mobile/15E148 Safari/604.1")
# Firefox and Edge on iOS (WebKit too; the user's restarts of 2026-11-22
# were in Safari and Firefox on an iPhone 15).
IOS_FIREFOX_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 "
                  "(KHTML, like Gecko) FxiOS/145.0 Mobile/15E148 Safari/605.1.15")
IOS_EDGE_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 "
               "(KHTML, like Gecko) Version/26.0 EdgiOS/142.0 Mobile/15E148 Safari/605.1.15")

# ocw-gpu.js's rules, run in the page; returns [{check, ok, detail}].
LOGIC = r"""
(async () => {
  const G = window.ocwGpuChoice;
  const out = [];
  const check = (name, ok, detail) => out.push({check: name, ok: !!ok, detail: String(detail)});
  const mem = (init) => {
    const m = new Map(Object.entries(init || {}));
    return {getItem: (k) => m.has(k) ? m.get(k) : null, setItem: (k, v) => m.set(k, String(v)),
            removeItem: (k) => m.delete(k), map: m};
  };
  const broken = {getItem() { throw new Error("SecurityError"); }, setItem() { throw new Error("QuotaExceeded"); },
                  removeItem() { throw new Error("SecurityError"); }};
  const gpuOk = {requestAdapter: async () => ({})};
  const gpuNone = {requestAdapter: async () => null};
  const gpuThrows = {requestAdapter: async () => { throw new Error("boom"); }};
  const win = (coarse) => ({matchMedia: (q) => ({matches: coarse && q === "(pointer: coarse)"})});
  const F = {
    chrome: G.facts({userAgent: CHROME_UA, vendor: "Google Inc.", platform: "Linux x86_64", maxTouchPoints: 0}, win(false)),
    safari: G.facts({userAgent: SAFARI_UA, vendor: "Apple Computer, Inc.", platform: "MacIntel", maxTouchPoints: 0}, win(false)),
    iphone: G.facts({userAgent: IPHONE_UA, vendor: "Apple Computer, Inc.", platform: "iPhone", maxTouchPoints: 5}, win(true)),
    ipad: G.facts({userAgent: SAFARI_UA, vendor: "Apple Computer, Inc.", platform: "MacIntel", maxTouchPoints: 5}, win(true)),
    ioschrome: G.facts({userAgent: IOS_CHROME_UA, vendor: "Apple Computer, Inc.", platform: "iPhone", maxTouchPoints: 5}, win(true)),
    iosfirefox: G.facts({userAgent: IOS_FIREFOX_UA, vendor: "Apple Computer, Inc.", platform: "iPhone", maxTouchPoints: 5}, win(true)),
    iosedge: G.facts({userAgent: IOS_EDGE_UA, vendor: "", platform: "iPhone", maxTouchPoints: 5}, win(true)),
    android: G.facts({userAgent: CHROME_UA.replace("X11; Linux x86_64", "Linux; Android 15; Pixel 9") + " Mobile",
                      vendor: "Google Inc.", platform: "Linux armv8l", maxTouchPoints: 5,
                      userAgentData: {mobile: true}}, win(true)),
  };
  // WebAssembly SIMD (Q10 (a)): this Chromium has it.
  check("simd: Chromium validates the SIMD test module; a refusing, missing or throwing WebAssembly → too old",
        G.simd(WebAssembly) && !G.simd({validate: () => false}) && !G.simd(null) && !G.simd({})
        && !G.simd({validate: () => { throw new Error("x"); }}) && G.TOO_OLD.includes("Safari 16.4 / Chrome 91 / Firefox 89"), "");
  // () -> () { i32.const 0; drop }: the same module without SIMD.
  const noSimd = [0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 10, 7, 1, 5, 0, 65, 0, 26, 11];
  check("simd: the same module without its v128 parts validates (only SIMD can refuse the test)",
        WebAssembly.validate(new Uint8Array(noSimd)), JSON.stringify(noSimd));
  check("facts: Chrome is no WebKit, no phone", !F.chrome.webkit && !F.chrome.phone && !F.chrome.ios, JSON.stringify(F.chrome));
  check("facts: Safari on macOS is WebKit, no phone", F.safari.webkit && !F.safari.ios && !F.safari.phone, JSON.stringify(F.safari));
  check("facts: an iPhone is WebKit, iOS, a phone", F.iphone.webkit && F.iphone.ios && F.iphone.phone, JSON.stringify(F.iphone));
  check("facts: an iPad asking for desktop pages is iOS", F.ipad.ios && F.ipad.webkit && F.ipad.phone, JSON.stringify(F.ipad));
  check("facts: Chrome on iOS is WebKit", F.ioschrome.webkit && F.ioschrome.ios, JSON.stringify(F.ioschrome));
  check("facts: Firefox (FxiOS) and Edge (EdgiOS) on iOS are WebKit, iOS, phones (whatever the vendor says)",
        F.iosfirefox.webkit && F.iosfirefox.ios && F.iosfirefox.phone && F.iosedge.webkit && F.iosedge.ios,
        JSON.stringify([F.iosfirefox, F.iosedge]));
  check("facts: Android is a phone, no WebKit", F.android.phone && !F.android.webkit, JSON.stringify(F.android));
  const choose = (f, q, extra) => G.choose(Object.assign({query: new URLSearchParams(q || ""), facts: f, gpu: gpuOk,
                                                          webgl2: () => true, storage: mem()}, extra || {}));
  let c = await choose(F.chrome);
  check("Chrome with an adapter: WebGPU", c.api === "webgpu" && !c.forced && !c.note, JSON.stringify(c));
  c = await choose(F.iphone);
  check("iPhone Safari 26 with WebGPU: WebGL 2", c.api === "webgl2" && /Safari/.test(c.reason) && /iOS/.test(c.reason), JSON.stringify(c));
  c = await choose(F.safari);
  check("Safari on macOS with WebGPU: WebGL 2", c.api === "webgl2" && /WebKit/.test(c.reason), JSON.stringify(c));
  c = await choose(F.ipad);
  check("iPadOS: WebGL 2", c.api === "webgl2", JSON.stringify(c));
  c = await choose(F.ioschrome);
  check("Chrome on iOS (WebKit): WebGL 2", c.api === "webgl2", JSON.stringify(c));
  for (const k of ["iosfirefox", "iosedge"]) {
    c = await choose(F[k]);
    check(`${k} with WebGPU: WebGL 2`, c.api === "webgl2" && /iOS/.test(c.reason), JSON.stringify(c));
    c = await choose(F[k], "", {gpu: undefined});
    check(`${k} without navigator.gpu: WebGL 2`, c.api === "webgl2", JSON.stringify(c));
  }
  c = await choose(F.iphone, "gpu=webgpu");
  check("gpu=webgpu on an iPhone: WebGPU, forced", c.api === "webgpu" && c.forced, JSON.stringify(c));
  c = await choose(F.iphone, "gpu=webgpu", {gpu: gpuNone});
  check("gpu=webgpu without an adapter: none, says why", c.api === null && /gpu=webgpu/.test(c.reason), JSON.stringify(c));
  c = await choose(F.chrome, "gpu=webgl2");
  check("gpu=webgl2: WebGL 2, forced", c.api === "webgl2" && c.forced, JSON.stringify(c));
  c = await choose(F.chrome, "gpu=webgl2&quality=low&webgpu-failed=1");
  check("the fallback's reload: WebGL 2 with the note", c.api === "webgl2" && /WebGPU failed/.test(c.note || ""), JSON.stringify(c));
  const s = mem();
  G.remember(s, F.chrome.ua, "WebGPU device lost: test", "2026-11-15", "abc");
  c = await choose(F.chrome, "", {storage: s});
  check("a remembered failure: WebGL 2 with the note", c.api === "webgl2" && !c.forced && /device lost/.test(c.reason)
        && /device lost/.test(c.note || ""), JSON.stringify(c));
  c = await choose(Object.assign({}, F.chrome, {ua: F.chrome.ua.replace("157", "158")}), "", {storage: s});
  check("an updated browser tries WebGPU again", c.api === "webgpu", JSON.stringify(c));
  c = await choose(F.chrome, "gpu=webgpu", {storage: s});
  check("gpu=webgpu over a remembered failure: WebGPU", c.api === "webgpu" && c.forced, JSON.stringify(c));
  G.forget(s);
  check("forget clears it", G.recall(s, F.chrome.ua) === null, JSON.stringify([...s.map]));
  s.setItem(G.KEY, "{not json");
  check("a damaged entry is ignored", G.recall(s, F.chrome.ua) === null, "");
  c = await choose(F.chrome, "", {storage: broken});
  check("a storage that throws: WebGPU, no exception", c.api === "webgpu", JSON.stringify(c));
  check("remember on a storage that throws: false", G.remember(broken, "x", "y", "z") === false, "");
  c = await choose(F.chrome, "", {gpu: gpuNone});
  check("no adapter: WebGL 2", c.api === "webgl2" && /without an adapter/.test(c.reason), JSON.stringify(c));
  c = await choose(F.chrome, "", {gpu: gpuThrows});
  check("requestAdapter throws: WebGL 2", c.api === "webgl2" && /boom/.test(c.reason), JSON.stringify(c));
  c = await choose(F.chrome, "", {gpu: undefined});
  check("no navigator.gpu: WebGL 2", c.api === "webgl2" && /navigator.gpu/.test(c.reason), JSON.stringify(c));
  c = await choose(F.chrome, "", {gpu: undefined, webgl2: () => false});
  check("neither: none, says why", c.api === null && /no WebGL 2/.test(c.reason), JSON.stringify(c));
  c = await choose(F.safari, "", {webgl2: () => false});
  check("Safari without WebGL 2: WebGPU (the only chance)", c.api === "webgpu", JSON.stringify(c));
  c = await choose(F.chrome, "", {webgl2: () => { throw new Error("x"); }});
  check("a WebGL 2 check that throws counts as none", c.api === "webgpu", JSON.stringify(c));
  const u = new URL(G.fallbackUrl("https://newkit.github.io/OCW/?world=liechtenstein&route=vaduz-malbun&gpu=webgpu&quality=high"));
  check("fallback URL keeps the ride, takes WebGL 2 low",
        u.searchParams.get("world") === "liechtenstein" && u.searchParams.get("route") === "vaduz-malbun"
        && u.searchParams.getAll("gpu").join() === "webgl2" && u.searchParams.get("quality") === "low"
        && u.searchParams.get("webgpu-failed") === "1" && u.pathname === "/OCW/", u.toString());
  // The fallback never loops (the iPhone's restarts, 2026-11-22).
  const href = "https://newkit.github.io/OCW/?world=liechtenstein";
  const wgpu = {api: "webgpu", forced: false};
  check("fallback: a chosen WebGPU falls back once", G.mayFallBack(wgpu, href, true), "");
  check("fallback: never from WebGL 2", !G.mayFallBack({api: "webgl2", forced: false}, href, true)
        && !G.mayFallBack({api: "webgl2", forced: true}, href, true), "");
  check("fallback: never from a forced WebGPU or without WebGL 2",
        !G.mayFallBack({api: "webgpu", forced: true}, href, true) && !G.mayFallBack(wgpu, href, false), "");
  check("fallback: never from a page that already is the fallback",
        !G.mayFallBack(wgpu, G.fallbackUrl(href), true) && !G.mayFallBack(wgpu, href + "&gpu=webgl2", true), "");
  const tab = mem();
  const t = 1763800000000;
  check("reload: the first one is allowed and recorded", G.mayReload(tab, t, "x") && tab.map.has(G.RELOAD_KEY), "");
  check("reload: a second within ten minutes is not",
        !G.mayReload(tab, t + 1000, "x") && !G.mayReload(tab, t + G.RELOAD_WINDOW_MS - 1, "x"), "");
  check("reload: allowed again after ten minutes", G.mayReload(tab, t + G.RELOAD_WINDOW_MS, "x"), "");
  check("reload: never without a storage (nothing could stop a loop)",
        !G.mayReload(null, t, "x") && !G.mayReload(broken, t, "x"), "");
  const loop = mem();
  let reloads = 0;
  for (let i = 0; i < 50; i++) if (G.mayReload(loop, t + i * 2000, "the same failure")) reloads++;
  check("reload: a failure that comes back every 2 s reloads once in 100 s", reloads === 1, reloads);
  const rides = mem();
  check("ride: none open at the first start", G.lastRideDied(rides, href).died === 0, "");
  G.rideStarted(rides, href, 0);
  let d = G.lastRideDied(rides, href);
  check("ride: a page that died in the ride is noticed at the next start", d.died === 1 && d.same, JSON.stringify(d));
  G.rideStarted(rides, G.saverUrl(href), 1);
  d = G.lastRideDied(rides, G.saverUrl(href));
  check("ride: … the second time counts 2", d.died === 2, JSON.stringify(d));
  G.rideClosed(rides);
  check("ride: a closed page (pagehide) is no death", G.lastRideDied(rides, href).died === 0, "");
  check("ride: a storage that throws: no death, no exception", G.lastRideDied(broken, href).died === 0
        && G.rideStarted(broken, href, 0) === false, "");
  const su = new URL(G.saverUrl(href + "&quality=high&route=vaduz-malbun"));
  check("saver URL keeps the ride, Low, memory=saver", su.searchParams.get("world") === "liechtenstein"
        && su.searchParams.get("route") === "vaduz-malbun" && su.searchParams.get("quality") === "low"
        && su.searchParams.get("memory") === "saver", su.toString());
  // Each death one step further: saver, minimal, then none (v5.42).
  const s1 = G.nextMemoryStep(href), s2 = s1 && G.nextMemoryStep(s1.href), s3 = s2 && G.nextMemoryStep(s2.href);
  check("deaths: saver, then minimal, then stop (no loop)", s1 && s1.mode === "saver" && s2 && s2.mode === "minimal"
        && s3 === null && new URL(s2.href).searchParams.get("quality") === "low"
        && new URL(s2.href).searchParams.get("world") === "liechtenstein", JSON.stringify([s1, s2, s3]));
  const qs = (s) => new URLSearchParams(s);
  check("pixel ratio: an iPhone in WebKit draws at 1.5, at 1 in the saver modes",
        G.pixelRatioCap(F.iphone, qs(""), 3, false) === 1.5 && G.pixelRatioCap(F.iphone, qs("memory=saver"), 3, false) === 1
        && G.pixelRatioCap(F.iphone, qs("memory=minimal"), 3, false) === 1, "");
  check("pixel ratio: desktops, Chromium phones (devicePixelContentBoxSize) and a ratio within the cap unchanged",
        G.pixelRatioCap(F.chrome, qs(""), 2, false) === null && G.pixelRatioCap(F.android, qs(""), 3, true) === null
        && G.pixelRatioCap(F.iphone, qs(""), 1, false) === null, "");
  check("pixel ratio: ?pixel-ratio= chooses (0.5 – 4)", G.pixelRatioCap(F.chrome, qs("pixel-ratio=1.25"), 2, true) === 1.25
        && G.pixelRatioCap(F.iphone, qs("pixel-ratio=9"), 3, false) === 1.5, "");
  let sp = G.startPage(F.chrome, null, true);
  check("start page, desktop Chrome: unticked; ticked adds gpu=webgl2&quality=low",
        !sp.ticked && sp.query(true) === "&gpu=webgl2&quality=low" && sp.query(false) === "", JSON.stringify(sp));
  sp = G.startPage(F.iphone, null, true);
  check("start page, iPhone: ticked; unticked tries WebGPU",
        sp.ticked && sp.query(false) === "&gpu=webgpu" && /not used in Safari/.test(sp.webgpuLabel), JSON.stringify(sp));
  sp = G.startPage(F.android, null, true);
  check("start page, Android phone: ticked; unticked: the default (WebGPU)", sp.ticked && sp.query(false) === "", JSON.stringify(sp));
  sp = G.startPage(F.chrome, {reason: "x"}, true);
  check("start page after a failure: ticked", sp.ticked && sp.query(false) === "&gpu=webgpu", JSON.stringify(sp));
  sp = G.startPage(F.chrome, null, false);
  check("start page without WebGPU: ticked, unticked adds nothing", sp.ticked && sp.query(false) === "", JSON.stringify(sp));
  const px = (rgb, n) => { const a = new Uint8ClampedArray(n * 4); for (let i = 0; i < n; i++) a.set([...rgb, 255], i * 4); return a; };
  check("one colour is uniform", G.uniform(px([0, 0, 0], 64)) && G.uniform(px([16, 20, 26], 64)), "");
  const drawn = px([90, 140, 200], 64); drawn.set([40, 60, 30, 255], 40 * 4);
  check("a drawn frame is not", !G.uniform(drawn) && !G.uniform(null), "");
  const near = px([10, 10, 10], 64); near.set([12, 11, 10, 255], 8);
  check("noise within 2 still uniform", G.uniform(near), "");
  return out;
})()
""".replace("IOS_FIREFOX_UA", json.dumps(IOS_FIREFOX_UA)).replace("IOS_EDGE_UA", json.dumps(IOS_EDGE_UA)) \
   .replace("IOS_CHROME_UA", json.dumps(IOS_CHROME_UA)).replace("CHROME_UA", json.dumps(CHROME_UA)) \
   .replace("SAFARI_UA", json.dumps(SAFARI_UA)).replace("IPHONE_UA", json.dumps(IPHONE_UA))

INJECT = {
    "uncaptured": r"""(() => {
      if (!window.GPUAdapter) return;
      const rd = GPUAdapter.prototype.requestDevice;
      GPUAdapter.prototype.requestDevice = async function (d) {
        const dev = await rd.call(this, d);
        setTimeout(() => dev.dispatchEvent(new GPUUncapturedErrorEvent("uncapturederror",
          {error: new GPUValidationError("injected by test-web-gpu: a validation error only WebKit reports")})), 6000);
        return dev;
      };
    })();""",
    "device": r"""(() => {
      if (!window.GPUAdapter) return;
      GPUAdapter.prototype.requestDevice = function () {
        return Promise.reject(new DOMException("injected by test-web-gpu", "OperationError"));
      };
    })();""",
    "noframe": r"""(() => {
      if (!window.GPUCanvasContext) return;
      GPUCanvasContext.prototype.configure = function () {
        throw new TypeError("injected by test-web-gpu: canvas configuration refused");
      };
    })();""",
}
SAFARI_VENDOR = """Object.defineProperty(Navigator.prototype, "vendor", {get: () => "Apple Computer, Inc."});"""


def test_web():
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def image_drawn(path):
    """(drawn, detail): a screenshot of more than one colour."""
    try:
        from PIL import Image, ImageStat
    except ImportError:
        size = os.path.getsize(path)
        return size > 40000, f"{size} bytes (no PIL)"
    img = Image.open(path).convert("RGB")
    st = ImageStat.Stat(img)
    spread = max(st.stddev)
    return spread > 8.0, f"stddev {spread:.1f}"


class Run:
    def __init__(self, tw, cdp, base, lines):
        self.tw, self.cdp, self.base, self.lines = tw, cdp, base, lines
        self.seen = 0
        self.checks = []
        self.t0 = time.time()

    def check(self, name, ok, detail=""):
        self.checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:600]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:200]}", flush=True)
        return ok

    def drain(self):
        said = []
        for e in self.cdp.events[self.seen:]:
            if e["method"] == "Runtime.consoleAPICalled":
                text = self.tw.console_text(e)
                said.append(text)
                self.lines.append(f"{time.time() - self.t0:7.2f} s  {text[:600]}")
            elif e["method"] == "Runtime.exceptionThrown":
                d = e["params"]["exceptionDetails"]
                text = d.get("exception", {}).get("description") or d.get("text", "")
                self.lines.append(f"{time.time() - self.t0:7.2f} s  EXCEPTION {text[:600]}")
        self.seen = len(self.cdp.events)
        return said

    def wait(self, expr, timeout):
        """Pumps until the page's `expr` is truthy; its value or None."""
        end = time.time() + timeout
        while time.time() < end:
            self.cdp.pump(0.2)
            self.drain()
            try:
                v = self.cdp.eval(expr)
            except Exception:  # noqa: BLE001 – a navigation in between
                v = None
            if v:
                return v
        return None

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            self.cdp.pump(0.2)
            self.drain()

    def state(self):
        return json.loads(self.cdp.eval("""JSON.stringify({gpu: window.ocwGpu || null, href: location.href,
            failure: window.ocwGpuFailure || null, note: document.getElementById('ocw-gpu-note')?.hidden === false
              ? document.getElementById('ocw-gpu-note').textContent : null,
            watch: window.ocwGpuWatch ? {frames: window.ocwGpuWatch.frames, ok: window.ocwGpuWatch.ok,
              failed: window.ocwGpuWatch.failed, samples: window.ocwGpuWatch.samples} : null,
            remembered: (() => { try { return localStorage.getItem('ocw.gpu.webgpu-failed'); } catch (e) { return 'error ' + e; } })(),
            canvas: [document.getElementById('ocw-canvas').width, document.getElementById('ocw-canvas').height,
                     document.getElementById('ocw-canvas').clientWidth, devicePixelRatio]})""") or "{}")

    def ticking(self, timeout=90):
        return self.wait("(() => { try { return window.ocw && JSON.parse(window.ocw.menu() || '{}') && "
                         "document.getElementById('ocw-status').hidden && window.ocwFrames > 30; } catch (e) { return false; } })()",
                         timeout)

    def shot(self, name):
        path = f"{self.base}-{name}.jpg"
        self.cdp.screenshot(path)
        return path


def fallback_case(r, root, case):
    """A WebGPU failure injected: the ride must come back on WebGL 2 and draw."""
    cdp = r.cdp
    cdp.eval("localStorage.clear()")
    ident = cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": INJECT[case]})["identifier"]
    mark = len(r.lines)
    cdp.call("Page.navigate", {"url": f"{root}?route=hills&bots=1"})
    back = r.wait("location.search.includes('webgpu-failed=1') && window.ocwGpu && window.ocwGpu.api === 'webgl2'", 75)
    cdp.call("Page.removeScriptToEvaluateOnNewDocument", {"identifier": ident})
    r.check(f"{case}: reloaded on WebGL 2", back, r.cdp.eval("location.href"))
    if not back:
        return
    rode = r.ticking(90)
    st = r.state()
    r.check(f"{case}: the note says WebGPU failed", st.get("note") and "WebGPU failed" in st["note"], st.get("note"))
    rem = json.loads(st.get("remembered") or "null") or {}
    r.check(f"{case}: the failure is remembered", rem.get("reason"), rem)
    r.pump(2.0)
    shot = r.shot(f"{case}-webgl2")
    drawn, how = image_drawn(shot)
    r.check(f"{case}: WebGL 2 rides and draws", rode and drawn, f"{how}; {shot}")
    said = "\n".join(r.lines[mark:])
    if case == "uncaptured":
        r.check("uncaptured: Bevy's renderer told the page (no quit)",
                "renderer stopped (Validation)" in said and "Quitting the application" not in said,
                [ln for ln in r.lines[mark:] if "renderer stopped" in ln or "Quitting" in ln][:2])


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--chromium", default=None)
    p.add_argument("--part", choices=("logic", "fallback", "all"), default="all")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    ws = tw.web_serve()
    modules = ["ocw-gpu.js"] + (["ocw_web_bg.wasm", "ocw_web_webgl2_bg.wasm"] if a.part != "logic" else [])
    missing = ws.missing_build(a.dir, modules)
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-gpu-" + stamp)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = tw.chromium_flags(profile, 1280, 720, "webgpu")
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(a.chromium, flags, log)
    lines = []
    r = Run(tw, cdp, base, lines)
    summary = {"ok": False, "browser": web_browser.describe(a.chromium)}
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False})
        if a.part in ("logic", "all"):
            # A page of the origin without the app: the script alone.
            cdp.call("Page.navigate", {"url": root + "ocw-gpu.js"})
            r.wait("document.readyState === 'complete'", 15)
            cdp.eval("fetch('ocw-gpu.js').then((r) => r.text()).then((t) => { (0, eval)(t); return true; })",
                     await_promise=True)
            res = cdp.call("Runtime.evaluate", {"expression": LOGIC, "awaitPromise": True, "returnByValue": True})
            if "exceptionDetails" in res:
                r.check("logic: the cases ran", False, res["exceptionDetails"])
            results = res.get("result", {}).get("value") or []
            r.check("logic: the cases ran", len(results) >= 30, len(results))
            for c in results:
                r.check("logic: " + c["check"], c["ok"], c["detail"])
        if a.part in ("fallback", "all"):
            for case in ("uncaptured", "device", "noframe"):
                fallback_case(r, root, case)
            # The remembered failure: WebGL 2 without ?gpu=.
            cdp.call("Page.navigate", {"url": f"{root}?route=hills&bots=1"})
            r.wait("window.ocwGpu", 30)
            st = r.state()
            r.check("remembered: WebGL 2 without gpu=", (st.get("gpu") or {}).get("api") == "webgl2"
                    and "failed in this browser before" in (st.get("gpu") or {}).get("reason", ""), st.get("gpu"))
            r.ticking(60)
            # ?gpu=webgpu forces WebGPU: it draws, the watchdog passes and
            # forgets the failure.
            cdp.call("Page.navigate", {"url": f"{root}?route=hills&bots=1&gpu=webgpu"})
            rode = r.ticking(90)
            ok = r.wait("window.ocwGpuWatch && (window.ocwGpuWatch.ok || window.ocwGpuWatch.failed)", 30)
            st = r.state()
            shot = r.shot("forced-webgpu")
            drawn, how = image_drawn(shot)
            r.check("gpu=webgpu: WebGPU rides, draws, the watchdog passes",
                    rode and ok and (st.get("watch") or {}).get("ok") and drawn
                    and (st.get("gpu") or {}).get("api") == "webgpu", f"{st.get('watch')}; {how}")
            r.check("gpu=webgpu that drew forgets the failure", st.get("remembered") is None, st.get("remembered"))
            # A healthy WebGPU ride: no false alarm.
            cdp.eval("localStorage.clear()")
            cdp.call("Page.navigate", {"url": f"{root}?route=hills&bots=1"})
            rode = r.ticking(90)
            r.wait("window.ocwGpuWatch && (window.ocwGpuWatch.ok || window.ocwGpuWatch.failed)", 30)
            r.pump(8.0)
            st = r.state()
            r.check("healthy WebGPU: stays on WebGPU, no alarm",
                    rode and (st.get("gpu") or {}).get("api") == "webgpu" and (st.get("watch") or {}).get("ok")
                    and not st.get("failure") and "webgpu-failed" not in st.get("href", ""), st)
            r.shot("webgpu")
            # Safari on an iPhone 15 (Chromium's WebGPU adapter stands in
            # for Safari 26's): WebGL 2 by default, and it draws.
            cdp.eval("localStorage.clear()")
            ident = cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": SAFARI_VENDOR})["identifier"]
            cdp.call("Emulation.setUserAgentOverride", {"userAgent": IPHONE_UA, "platform": "iPhone"})
            cdp.call("Emulation.setTouchEmulationEnabled", {"enabled": True, "maxTouchPoints": 5})
            cdp.call("Emulation.setDeviceMetricsOverride",
                     {"width": 393, "height": 659, "deviceScaleFactor": 3, "mobile": True})
            cdp.call("Page.navigate", {"url": f"{root}?route=hills&bots=1"})
            rode = r.ticking(90)
            r.pump(3.0)
            st = r.state()
            shot = r.shot("iphone-safari")
            drawn, how = image_drawn(shot)
            gpu = st.get("gpu") or {}
            r.check("iPhone Safari with WebGPU: WebGL 2 by default, rides, draws",
                    rode and gpu.get("api") == "webgl2" and "Safari" in gpu.get("reason", "") and drawn
                    and not st.get("failure"), f"{gpu}; {how}")
            cw = st.get("canvas") or [0, 0, 0, 0]
            r.check("iPhone: Low's pixel-ratio cap (canvas ≤ 1 device pixel per CSS pixel)",
                    cw[3] == 3 and 0 < cw[0] <= cw[2] + 2, cw)
            cdp.call("Page.removeScriptToEvaluateOnNewDocument", {"identifier": ident})
        summary["ok"] = bool(r.checks) and all(c["ok"] for c in r.checks)
    except Exception as e:  # noqa: BLE001 – reported in the summary
        summary["error"] = repr(e)
        print("ERROR", repr(e))
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
    summary["checks"] = r.checks
    log.write("\n--- console ---\n" + "\n".join(lines) + "\n")
    log.write("\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    failed = [c for c in r.checks if not c["ok"]]
    print(f"{len(r.checks) - len(failed)} of {len(r.checks)} checks passed; log: {base}.log")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
