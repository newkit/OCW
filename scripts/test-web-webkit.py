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
           the phone page of spec 3.2.3 TS4: the web app manifest (display
           fullscreen, its icons served), the Apple touch icon and meta
           tags, viewport-fit=cover; the ride with the touch hints and a
           notch's insets (?safe-insets=59,0,34,0 stands in for Safari's
           env(safe-area-inset-*): WebKit on Linux has no notch): the
           insets reach the app, a tap on each half steers, a tap on the
           home bar does not, html / body have touch-action none, no
           element full screen (as on the iPhone: the first-ride tip says
           Add to Home Screen), no wake lock (removed: tolerated);
  iphone-landscape  the phone turned (852 × 393, insets 0,59,21,59): taps
           behind the notch do not steer, each half does, a held right side
           (touch pointers) steers and shows its glow; a wake lock that is
           there is held and asked again when the page is shown again;
  both phone cases also check the phone layout (spec 3.2 *Phone layout*,
           3.2.3 TS5): the compact HUD (scale < 1, at most 2 big and 5
           lines, in the top quarter / third, no key line), nothing over
           the road in the middle, the HUD's boxes apart and on screen, the
           Devices panel collapsed, the power buttons (+ adds power and
           does not steer, − takes some away, stop brakes, + rides on), a
           held right side in portrait too;
  layout-* the first ride's touch layout (spec 3.2.3) at iPhone 15 and
           small Android sizes, portrait and landscape (390 × 844, 844 × 390
           with the notch's insets; 360 × 640, 640 × 360): the first-ride
           tip, the screen tip's banner (Add to Home Screen on the iPhone,
           Full screen on Android), the power buttons, the chevrons and the
           HUD's boxes – no two overlap, all inside the safe area, the
           first-ride tip above the rider;
  mac      Safari on macOS (1280 × 720): the Hills ride on WebGL 2, draws;
           the desktop HUD unchanged (not compact, scale 1, keys), the
           Devices panel open, no power buttons.

  No check may see a page error (an uncaught exception) in any case.

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
# The iPhone's Safari: no element full screen (WebKit on Linux has it).
NO_FULLSCREEN = """
for (const k of ["fullscreenEnabled", "webkitFullscreenEnabled"]) {
  Object.defineProperty(Document.prototype, k, {get: () => false, configurable: true});
}
"""
# No Screen Wake Lock API (Safari before 16.4): the page must not throw.
NO_WAKE_LOCK = """
Object.defineProperty(Navigator.prototype, "wakeLock", {get: () => undefined, configurable: true});
"""
# A wake lock that counts its requests (Chrome, Safari 16.4+).
FAKE_WAKE_LOCK = """
window.__wakeRequests = 0;
window.__wakeSentinels = [];
const __wakeLock = {
  request: async (type) => {
    window.__wakeRequests++;
    const s = new EventTarget();
    s.type = type;
    s.released = false;
    s.release = async () => { s.released = true; s.dispatchEvent(new Event("release")); };
    window.__wakeSentinels.push(s);
    return s;
  },
};
Object.defineProperty(Navigator.prototype, "wakeLock", {get: () => __wakeLock, configurable: true});
"""
# The phone cases: the device, the insets (top, right, bottom, left) and
# the page's APIs.
PHONES = {
    "iphone": ("iPhone 15", (59, 0, 34, 0), NO_FULLSCREEN + NO_WAKE_LOCK),
    "iphone-landscape": ("iPhone 15 landscape", (0, 59, 21, 59), NO_FULLSCREEN + FAKE_WAKE_LOCK),
}
# The layout cases (spec 3.2.3): CSS size, insets (top, right, bottom,
# left), device pixel ratio, the page's APIs, the user agent.
IPHONE_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
             "(KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1")
ANDROID_UA = ("Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) "
              "Chrome/131.0.0.0 Mobile Safari/537.36")
LAYOUTS = {
    "layout-iphone-390x844": ((390, 844), (59, 0, 34, 0), 3, NO_FULLSCREEN, IPHONE_UA),
    "layout-iphone-844x390": ((844, 390), (0, 59, 21, 59), 3, NO_FULLSCREEN, IPHONE_UA),
    "layout-android-360x640": ((360, 640), (0, 0, 0, 0), 3, "", ANDROID_UA),
    "layout-android-640x360": ((640, 360), (0, 0, 0, 0), 3, "", ANDROID_UA),
}
# A touch pointer on the canvas (winit reads pointer events; Playwright's
# WebKit can only tap, not hold).
POINTER_JS = """([type, x, y]) => {
  const c = document.getElementById('ocw-canvas');
  const up = type === 'pointerup';
  c.dispatchEvent(new PointerEvent(type, {pointerId: 41, pointerType: 'touch', isPrimary: true,
    clientX: x, clientY: y, screenX: x, screenY: y, bubbles: true, cancelable: true, composed: true,
    pressure: up ? 0 : 0.5, buttons: up ? 0 : 1, button: up || type === 'pointerdown' ? 0 : -1,
    width: 20, height: 20}));
}"""
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

    def ride(pg, label, seconds=None):
        extra = ""
        if label in PHONES:
            extra = "&touch=on&safe-insets=" + ",".join(str(v) for v in PHONES[label][1])
        elif label in LAYOUTS:
            extra = "&touch=on&safe-insets=" + ",".join(str(v) for v in LAYOUTS[label][1])
        pg.goto(root + "?route=hills&bots=1" + extra)
        try:
            pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                 " && window.ocwFrames > 20", timeout=120000)
        except Exception as e:  # noqa: BLE001
            lines.append(f"[{label}] wait: {e}")
        pg.wait_for_timeout(int((a.seconds if seconds is None else seconds) * 1000))
        st = json.loads(pg.evaluate(state_js))
        shot = f"{base}-{label}.png"
        pg.screenshot(path=shot)
        return st, shot

    def steer(pg):
        return json.loads(pg.evaluate("window.ocw.steer()") or "{}")

    def screen(pg):
        return json.loads(pg.evaluate("window.ocwScreen.state()") or "{}")

    def line(pg):
        v = steer(pg).get("line_m")
        return float(v) if v is not None else float("nan")

    def tap(pg, x, y):
        pg.touchscreen.tap(x, y)
        pg.wait_for_timeout(400)

    def page_files(pg):
        """The phone page's head and the files Add to Home Screen needs."""
        return pg.evaluate("""async () => {
          const q = (s) => document.querySelector(s);
          const attr = (s, a) => (q(s) && q(s).getAttribute(a)) || null;
          const out = {viewport: attr('meta[name=viewport]', 'content'),
            manifest_link: attr('link[rel=manifest]', 'href'),
            touch_icon: attr('link[rel=apple-touch-icon]', 'href'),
            capable: attr('meta[name=apple-mobile-web-app-capable]', 'content'),
            theme: attr('meta[name=theme-color]', 'content'),
            device_info: JSON.parse(window.ocwDeviceInfo || '{}').safe_insets || null};
          const get = async (u) => { try { const r = await fetch(u, {cache: 'no-cache'});
            return {status: r.status, type: r.headers.get('content-type'), bytes: (await r.arrayBuffer()).byteLength};
          } catch (e) { return {status: 0, error: String(e)}; } };
          try {
            const r = await fetch(out.manifest_link || 'manifest.webmanifest', {cache: 'no-cache'});
            out.manifest_status = r.status;
            out.manifest_type = r.headers.get('content-type');
            out.manifest = await r.json();
          } catch (e) { out.manifest_error = String(e); }
          out.icons = [];
          for (const i of ((out.manifest && out.manifest.icons) || [])) out.icons.push({src: i.src, ...(await get(i.src))});
          out.touch_icon_file = out.touch_icon ? await get(out.touch_icon) : null;
          return out;
        }""")

    def hud_layout(pg):
        return json.loads(pg.evaluate("window.ocw.hudLayout ? window.ocw.hudLayout() : '{}'") or "{}")

    def dom_box(pg, sel):
        return pg.evaluate(f"""(() => {{ const e = document.querySelector('{sel}');
          if (!e || e.hidden || getComputedStyle(e).display === 'none') return null;
          const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }})()""")

    def overlap(a, b):
        return (a[0] < b[0] + b[2] - 0.5 and b[0] < a[0] + a[2] - 0.5
                and a[1] < b[1] + b[3] - 0.5 and b[1] < a[1] + a[3] - 0.5)

    def phone_layout(pg, label, w, h):
        """The phone layout (spec 3.2 *Phone layout*, 3.2.3 TS5): a compact
        HUD, the road's centre clear, nothing on top of another, the Devices
        panel collapsed, the power buttons change the power."""
        hud = hud_layout(pg)
        st = steer(pg)
        portrait = label == "iphone"
        check(f"{label}: the compact HUD (scale follows the CSS size: {hud.get('ui_scale')})",
              hud.get("compact") and hud.get("narrow") == portrait and 0.7 <= hud.get("ui_scale", 0) < 1.0, hud)
        lines = hud.get("big_lines", 99) + hud.get("small_lines", 99)
        check(f"{label}: the top panel has at most 2 big lines and 5 in all", hud.get("big_lines", 99) <= 2
              and lines <= 5, hud.get("text"))
        tp = hud.get("top_panel") or [0, 0, 0, h]
        check(f"{label}: the top panel stays in the top {'quarter' if portrait else 'third'}",
              tp[1] + tp[3] <= h * (0.25 if portrait else 0.36) + (59 if portrait else 0), tp)
        check(f"{label}: no key line while the touch hints show", "up: faster" not in hud.get("text", "")
              and "F12" not in hud.get("text", ""), hud.get("text"))
        dev = json.loads(pg.evaluate("window.ocwDevicesPanel()") or "{}")
        check(f"{label}: the Devices panel starts collapsed (no Web Bluetooth on a touch phone)",
              dev.get("collapsed") and not dev.get("expanded"), dev)
        mp = json.loads(pg.evaluate("window.ocw.hudMap()") or "{}")
        boxes = {"top panel": hud.get("top_panel"), "top-right column": hud.get("top_right"),
                 "profile": hud.get("profile"), "map": mp.get("rect") if mp.get("shown") else None,
                 "Devices button": dom_box(pg, "#ocw-devices-show")}
        for i, b in enumerate(st.get("power_buttons") or []):
            boxes[f"power button {i}"] = [b[0], b[1], b[2] - b[0], b[3] - b[1]]
        boxes = {k: v for k, v in boxes.items() if v}
        centre = [w * 0.35, h * 0.4, w * 0.3, h * 0.35]
        on_road = [k for k, v in boxes.items() if overlap(v, centre)]
        check(f"{label}: nothing covers the road in the middle of the screen", not on_road,
              {"centre": centre, "on it": on_road, "boxes": boxes})
        names = list(boxes)
        clashes = [(a, b) for i, a in enumerate(names) for b in names[i + 1:]
                   if not (a.startswith("power") and b.startswith("power")) and overlap(boxes[a], boxes[b])]
        check(f"{label}: the HUD's boxes do not overlap", not clashes, {"clashes": clashes, "boxes": boxes})
        off = [k for k, v in boxes.items() if v[0] < -0.5 or v[1] < -0.5 or v[0] + v[2] > w + 0.5
               or v[1] + v[3] > h + 0.5]
        check(f"{label}: every box is on the screen", not off, {"off": off, "boxes": boxes})
        # TS5: the power buttons (stop, −, +) while riding on virtual power.
        pb = st.get("power_buttons") or []
        check(f"{label}: the touch power buttons show (virtual power)", st.get("power") and len(pb) == 3, st)
        if len(pb) == 3:
            def mid(b):
                return ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2)

            def watts():
                return float(steer(pg).get("key_power_w") or 0)

            l0, p0 = line(pg), watts()
            tap(pg, *mid(pb[2]))
            p1, l1 = watts(), line(pg)
            check(f"{label}: a tap on + adds power and does not steer", p1 > p0 and abs(l1 - l0) < 1e-3,
                  f"{p0} → {p1} W, line {l0:.3f} → {l1:.3f}")
            tap(pg, *mid(pb[1]))
            p2 = watts()
            check(f"{label}: a tap on − takes some away", p2 < p1, f"{p1} → {p2} W")
            tap(pg, *mid(pb[0]))
            pg.wait_for_timeout(300)
            p3 = watts()
            check(f"{label}: stop: no extra power, the brake on", p3 == 0.0, f"{p2} → {p3} W")
            pg.screenshot(path=f"{base}-{label}-stopped.png")
            tap(pg, *mid(pb[2]))
            pg.wait_for_timeout(300)
            check(f"{label}: + rides on after stop", watts() > 0, steer(pg))
            pg.wait_for_timeout(3000)
            pg.screenshot(path=f"{base}-{label}-riding.png")

    def touch_layout(pg, label, errors):
        """The first ride's touch layout (spec 3.2.3): the first-ride tip,
        the screen tip's banner, the power buttons, the chevrons and the
        HUD's boxes apart, inside the safe area, the tip above the rider."""
        (w, h), (it, ir, ib, il), _, _, _ = LAYOUTS[label]
        pg.wait_for_timeout(1200)
        hud, st, sc = hud_layout(pg), steer(pg), screen(pg)
        boxes = {"first-ride tip": hud.get("touch_tip"), "screen tip": sc.get("tip_rect"),
                 "top panel": hud.get("top_panel"), "top-right column": hud.get("top_right"),
                 "profile": hud.get("profile"), "Devices button": dom_box(pg, "#ocw-devices-show")}
        for i, c in enumerate(hud.get("chevrons") or []):
            boxes[f"chevron {i}"] = c
        for i, b in enumerate(st.get("power_buttons") or []):
            boxes[f"power button {i}"] = [b[0], b[1], b[2] - b[0], b[3] - b[1]]
        shown = {k: v for k, v in boxes.items() if v}
        check(f"{label}: the first-ride tip, the screen tip and the power buttons show",
              st.get("tip") and boxes["first-ride tip"] and boxes["screen tip"] and st.get("power")
              and len(st.get("power_buttons") or []) == 3,
              {"steer": {k: st.get(k) for k in ("shown", "tip", "power")}, "screen": sc.get("tip"), "boxes": boxes})
        names = list(shown)
        clashes = [(x, y) for i, x in enumerate(names) for y in names[i + 1:] if overlap(shown[x], shown[y])]
        check(f"{label}: nothing overlaps (tips, power buttons, chevrons, HUD)", not clashes,
              {"clashes": clashes, "boxes": shown})
        off = [k for k, v in shown.items() if v[0] < il - 0.5 or v[1] < it - 0.5 or v[0] + v[2] > w - ir + 0.5
               or v[1] + v[3] > h - ib + 0.5]
        check(f"{label}: every box inside the safe area", not off, {"off": off, "boxes": shown})
        tip = boxes["first-ride tip"] or [0, h, 0, 0]
        check(f"{label}: the first-ride tip above the rider (its bottom in the upper half)",
              tip[1] + tip[3] <= h * 0.5 + 0.5, tip)
        pg.screenshot(path=f"{base}-{label}.png")
        check(f"{label}: no page errors", not errors, errors[:5])

    def phone(pg, label, errors):
        """The phone page of spec 3.2.3 TS4 (after the ride shows)."""
        _, insets, _ = PHONES[label]
        files = page_files(pg)
        man = files.get("manifest") or {}
        icons_ok = bool(files["icons"]) and all(i.get("status") == 200 and i.get("bytes", 0) > 100
                                                for i in files["icons"])
        if label == "iphone":
            check("iphone: viewport-fit=cover", "viewport-fit=cover" in (files.get("viewport") or ""), files.get("viewport"))
            check("iphone: the web app manifest is served and valid (display fullscreen, name, start_url, theme)",
                  files.get("manifest_status") == 200 and man.get("display") == "fullscreen"
                  and man.get("name") == "Open Cycling World" and man.get("start_url")
                  and man.get("theme_color") == files.get("theme"),
                  {k: files.get(k) for k in ("manifest_status", "manifest_type", "manifest", "theme")})
            check("iphone: the manifest's icons are served (PNG 256 and 512, the SVG)", icons_ok
                  and {"256x256", "512x512"} <= {i.get("sizes") for i in man.get("icons", [])}, files["icons"])
            tf = files.get("touch_icon_file") or {}
            check("iphone: an Apple touch icon and apple-mobile-web-app-capable",
                  tf.get("status") == 200 and "png" in (tf.get("type") or "") and files.get("capable") == "yes",
                  {"touch_icon": files.get("touch_icon"), "file": tf, "capable": files.get("capable")})
        want = list(insets)
        st, sc = steer(pg), screen(pg)
        check(f"{label}: the page passes the safe-area insets to the app",
              [float(v) for v in st.get("insets", [])] == [float(v) for v in want]
              and [sc.get("insets", {}).get(k) for k in ("top", "right", "bottom", "left")] == want,
              {"app": st.get("insets"), "page": sc.get("insets"), "device_info": files.get("device_info")})
        check(f"{label}: html and body take no pan or pinch while riding (touch-action none)",
              sc.get("riding") and sc.get("touch_action") == "none", sc)
        # The hints and the first-ride tip with Add to Home Screen.
        pg.wait_for_timeout(700)
        sc = screen(pg)
        check(f"{label}: the hints and the first-ride tip show (touch=on)", st.get("shown") and st.get("tip"), st)
        check(f"{label}: no element full screen: the tip says Add to Home Screen",
              sc.get("tip") == "fullscreen" and "Add to Home Screen" in sc.get("tip_text", "")
              and not sc.get("fullscreen_api"), sc)
        said = pg.evaluate("window.ocwScreen.fullscreen('toggle')")
        check(f"{label}: Full screen (the menu's command) says how to add to the home screen",
              "Add to Home Screen" in said, said)
        pg.screenshot(path=f"{base}-{label}-hints.png")
        w, h = pg.evaluate("[innerWidth, innerHeight]")
        # Taps outside the safe area (notch / home bar) do not steer.
        l0 = line(pg)
        if label == "iphone":
            tap(pg, w * 0.25, h - 12)
            where = "on the home bar"
        else:
            tap(pg, 30, h * 0.6)
            tap(pg, w - 30, h * 0.6)
            where = "behind the notch"
        l1 = line(pg)
        check(f"{label}: taps {where} do not steer", abs(l1 - l0) < 1e-3, f"{l0:.3f} → {l1:.3f}")
        tap(pg, w * 0.75, h * 0.75)
        l2 = line(pg)
        check(f"{label}: a tap on the right half moves the line +0.5 m", abs(l2 - l1 - 0.5) < 0.02,
              f"{l1:.3f} → {l2:.3f}")
        tap(pg, w * 0.25, h * 0.75)
        l3 = line(pg)
        check(f"{label}: a tap on the left half moves it back", abs(l3 - l2 + 0.5) < 0.02, f"{l2:.3f} → {l3:.3f}")
        sc = screen(pg)
        # A held right side (touch pointers on the canvas), clear of the
        # power buttons.
        x, y = w * 0.7, h * 0.72
        pg.evaluate(POINTER_JS, ["pointerdown", x, y])
        for _ in range(6):
            pg.wait_for_timeout(150)
            pg.evaluate(POINTER_JS, ["pointermove", x, y])
        held = steer(pg)
        pg.screenshot(path=f"{base}-{label}-hold.png")
        pg.evaluate(POINTER_JS, ["pointerup", x, y])
        pg.wait_for_timeout(300)
        l4 = line(pg)
        check(f"{label}: a held right side steers right while held (glow)", held.get("right_held"), held)
        check(f"{label}: … about 2 m/s", 0.8 < l4 - l3 < 3.6, f"{l3:.3f} → {l4:.3f}")
        phone_layout(pg, label, w, h)
        if label == "iphone":
            check("iphone: no wake lock API: tolerated (no error)", sc.get("wake_lock") == "unsupported"
                  and not errors, sc)
        else:
            n0 = pg.evaluate("window.__wakeRequests")
            check(f"{label}: the wake lock is held while riding", sc.get("wake_lock") == "held" and n0 >= 1,
                  {"state": sc.get("wake_lock"), "requests": n0})
            # The system lets it go (the page hidden); shown again: asked again.
            pg.evaluate("window.__wakeSentinels[window.__wakeSentinels.length - 1].release()")
            pg.wait_for_timeout(100)
            released = screen(pg).get("wake_lock")
            pg.evaluate("document.dispatchEvent(new Event('visibilitychange'))")
            pg.wait_for_timeout(300)
            n1 = pg.evaluate("window.__wakeRequests")
            check(f"{label}: the wake lock is asked again after visibilitychange",
                  released == "released" and n1 == n0 + 1 and screen(pg).get("wake_lock") == "held",
                  {"released": released, "requests": [n0, n1]})
        check(f"{label}: no page errors", not errors, errors[:5])

    summary = {"ok": False}
    with sync_playwright() as pw:
        browser = pw.webkit.launch()
        summary["webkit"] = browser.version
        try:
            for label in ("iphone", "iphone-landscape", "mac", *LAYOUTS):
                if label in LAYOUTS:
                    (lw, lh), _, dpr, js, ua = LAYOUTS[label]
                    ctx = browser.new_context(viewport={"width": lw, "height": lh}, device_scale_factor=dpr,
                                              is_mobile=True, has_touch=True, user_agent=ua)
                    if js:
                        ctx.add_init_script(js)
                elif label in PHONES:
                    ctx = browser.new_context(**pw.devices[PHONES[label][0]])
                    ctx.add_init_script(PHONES[label][2])
                else:
                    ctx = browser.new_context(viewport={"width": 1280, "height": 720})
                ctx.add_init_script(FAKE_GPU)
                pg = ctx.new_page()
                errors = []
                pg.on("console", lambda m, label=label: lines.append(
                    f"[{label}] {time.time() - t0:7.2f} s  {m.type}: {m.text[:600]}"))
                pg.on("pageerror", lambda e, label=label, errors=errors: (
                    lines.append(f"[{label}] PAGEERROR {e}"), errors.append(str(e)[:300])))
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
                if label in LAYOUTS:
                    st, _ = ride(pg, label, seconds=6)
                    touch_layout(pg, label, errors)
                    summary[label] = st
                    ctx.close()
                    continue
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
                if label in PHONES:
                    phone(pg, label, errors)
                else:
                    # A desktop browser keeps its HUD (spec 3.2 *Phone layout*).
                    hud = hud_layout(pg)
                    dev = json.loads(pg.evaluate("window.ocwDevicesPanel()") or "{}")
                    check(f"{label}: the desktop HUD is unchanged (not compact, scale 1, the keys shown)",
                          hud.get("compact") is False and hud.get("ui_scale") == 1.0
                          and "up: faster" in hud.get("text", ""), hud)
                    check(f"{label}: the Devices panel starts open on a desktop", dev.get("expanded"), dev)
                    check(f"{label}: no touch power buttons without touch", not steer(pg).get("power"), steer(pg))
                    check(f"{label}: no page errors", not errors, errors[:5])
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
