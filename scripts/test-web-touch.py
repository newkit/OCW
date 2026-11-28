#!/usr/bin/env python3
"""The on-screen touch controls of the browser app (spec 3.2.3 *The on-screen controls*; 11.4).

    scripts/test-web-touch.py [--out test-output/web-touch] [--dir target/web]
                              [--engines webkit,chromium] [--cases phone,phone-landscape,tablet,tablet-landscape]
                              [--software] [--python PYTHON]

Playwright (`make deps-webkit`) on target/web/ (`make web`), with touch
emulation, the Hills ride on WebGL 2 at Low on the virtual trainer:

  webkit    Safari's engine as an iPhone (430 x 932 at 3, the notch's
            insets through ?safe-insets=) and an iPad (820 x 1180 at 2);
  chromium  the pinned Chromium as an Android phone and tablet of the same
            sizes.

Each engine rides the cases

  phone             430 x 932, insets 59 / 0 / 34 / 0 (the full run, with
                    a screen video of the touch session);
  phone-landscape   932 x 430, insets 0 / 59 / 21 / 59;
  tablet            820 x 1180, insets 24 / 0 / 20 / 0;
  tablet-landscape  1180 x 820, insets 24 / 0 / 20 / 0;

and checks in each (every tap is a real touch: Playwright's touchscreen;
holds and drags are touch pointers on the canvas):

- the layout: the menu button and the hide button (the eye) show, 44 px or
  more, inside the safe area, on no HUD box, chevron or page button; no
  stop / - / + button anywhere; the faster / slower zones are the top and
  the bottom quarter of the safe area;
- a tap at the top is faster (+25 W, "▲ … W" at the top edge), a tap at
  the bottom slower (−10 W, "▼ … W"), neither steers; held, the top ramps
  the power up and the bottom down (to 0, without the brake); with no
  power left a tap at the bottom stops ("▼ stop", the brake), the top
  rides on; a tap in the half between steers and changes no power;
- no accidental steps: a drag across the top, two fingers (a pinch), taps
  behind the notch and on the home indicator;
- the hide button: the chevrons, the zone marks and the tips are not drawn
  (the page's Devices button neither), a note says "Controls hidden – they
  still work" the first time, and every touch area still works – faster,
  slower, steering; the state survives a reload; a second tap shows them;
- the menu by touch: ☰ opens the menu the Esc key opens, the zones are off
  under it, a tap on *Settings* opens it, on into *Controls* and *Touch
  and unicorn mode*, the › / ‹ of *Touch controls* hides and shows the
  controls, *Back* goes up page by page, ☰ closes the menu;
- (phone) a text field: Ride → Join's link field starts typing, the page's
  text bar shows, a tap into it and typed text reach the field (the
  on-screen keyboard's path), Backspace and Done work;
- (phone) the page log's panel open: the zones are off, steering stays;
- no page error.

  OUT/web-touch-<time>-<engine>-<case>-<what>.png   screenshots: controls
        shown, the faster and slower feedback, controls hidden (with the
        note and without), the menu opened by the button, Settings
  OUT/web-touch-<time>-<engine>-phone-session.webm  the phone's touch session
  OUT/web-touch-<time>.json / .log                  checks and the console

Without Playwright (no `make deps-webkit`) it says so and exits 2.
Exits 0 when every check passed.
"""

import argparse
import datetime
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
VENV = os.path.join(os.path.expanduser("~"), ".cache", "ocw", "playwright-venv")
CONFIG = json.dumps({"worlds": "worlds/", "server": "", "voice": "", "signin": "", "build": "test-web-touch"})
IPHONE_UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 "
             "(KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1")
IPAD_UA = ("Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 "
           "(KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1")
ANDROID_UA = ("Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) "
              "Chrome/131.0.0.0 Mobile Safari/537.36")
ANDROID_TABLET_UA = ("Mozilla/5.0 (Linux; Android 14; Pixel Tablet) AppleWebKit/537.36 (KHTML, like Gecko) "
                     "Chrome/131.0.0.0 Safari/537.36")
# Safari 26's navigator.gpu (WebKit on Linux has none): the page takes WebGL 2.
FAKE_GPU = """
if (!navigator.gpu) {
  Object.defineProperty(Navigator.prototype, "gpu", {get: () => ({
    requestAdapter: async () => ({features: new Set(), limits: {}}),
    getPreferredCanvasFormat: () => "bgra8unorm",
  }), configurable: true});
}
"""
# The iPhone's Safari has no element full screen and no Web Bluetooth.
NO_FULLSCREEN = """
for (const k of ["fullscreenEnabled", "webkitFullscreenEnabled"]) {
  Object.defineProperty(Document.prototype, k, {get: () => false, configurable: true});
}
"""
# The cases: CSS size, the safe-area insets (top, right, bottom, left),
# the device pixel ratio, a phone or a tablet, and the rider of the
# pictures (made-up names, a different look in each: spec 11.4).
CASES = {
    "phone": ((430, 932), (59, 0, 34, 0), 3, "phone", "Ada Rider",
              "character --body b --hair ponytail --hair-colour #2B1B14 --skin 7 --glasses 2"),
    "phone-landscape": ((932, 430), (0, 59, 21, 59), 3, "phone", "Bea Climber",
                        "character --body a --hair short --hair-colour #C9A227 --skin 2"),
    "tablet": ((820, 1180), (24, 0, 20, 0), 2, "tablet", "Cleo Sprinter",
               "character --body c --hair long --hair-colour auburn --skin 4 --glasses 1"),
    "tablet-landscape": ((1180, 820), (24, 0, 20, 0), 2, "tablet", "Dana Rouleur",
                         "character --body a --hair curly --hair-colour #1A1A1A --skin 10"),
}
# A touch pointer on the canvas (winit reads pointer events; Playwright's
# touchscreen can only tap).
POINTER_JS = """([type, x, y, id]) => {
  const c = document.getElementById('ocw-canvas');
  const up = type === 'pointerup' || type === 'pointercancel';
  c.dispatchEvent(new PointerEvent(type, {pointerId: id || 41, pointerType: 'touch', isPrimary: (id || 41) === 41,
    clientX: x, clientY: y, screenX: x, screenY: y, bubbles: true, cancelable: true, composed: true,
    pressure: up ? 0 : 0.5, buttons: up ? 0 : 1, button: up || type === 'pointerdown' ? 0 : -1,
    width: 20, height: 20}));
}"""


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


def quarter_zones(size, insets):
    """Where the faster zone ends and the slower zone starts (CSS px from
    the top): a quarter of the safe area's height from its top and bottom."""
    (_, h), (top, _, bottom, _) = size, insets
    zone = (h - top - bottom) / 4
    return top + zone, h - bottom - zone


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-touch")
    p.add_argument("--engines", default="webkit,chromium")
    p.add_argument("--cases", default=",".join(CASES))
    p.add_argument("--software", action="store_true", help="Chromium's WebGL 2 on SwiftShader")
    p.add_argument("--python", default=None)
    a = p.parse_args()
    if not in_venv():
        py = a.python or os.path.join(VENV, "bin", "python")
        if not os.path.exists(py):
            print("test-web-touch: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    from playwright.sync_api import sync_playwright

    ws = load("web_serve", "web-serve.py")
    missing = ws.missing_build(a.dir, ["ocw-gpu.js", "ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-touch-" + stamp)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    checks, lines, shots, videos = [], [], [], []
    t0 = time.time()
    summary = {"ok": False}

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:700]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:220]}", flush=True)

    def run(engine, case, browser):
        size, insets, dpr, kind, name, look = CASES[case]
        w, h = size
        it, ir, ib, il = insets
        label = f"{engine} {case}"
        full = case == "phone"
        if engine == "webkit":
            ua = IPHONE_UA if kind == "phone" else IPAD_UA
        else:
            ua = ANDROID_UA if kind == "phone" else ANDROID_TABLET_UA
        video_dir = os.path.join(a.out, f".video-{stamp}-{engine}-{case}") if full else None
        ctx = browser.new_context(viewport={"width": w, "height": h}, screen={"width": w, "height": h},
                                  device_scale_factor=dpr, is_mobile=True, has_touch=True, user_agent=ua,
                                  **({"record_video_dir": video_dir, "record_video_size": {"width": w, "height": h}}
                                     if video_dir else {}))
        ctx.add_init_script(FAKE_GPU + (NO_FULLSCREEN if engine == "webkit" and kind == "phone" else ""))
        ctx.route("**/config.json", lambda route: route.fulfill(status=200, content_type="application/json", body=CONFIG))
        pg = ctx.new_page()
        errors = []
        pg.on("console", lambda m: lines.append(f"[{label}] {time.time() - t0:7.2f} s  {m.type}: {m.text[:400]}"))
        pg.on("pageerror", lambda e: (lines.append(f"[{label}] PAGEERROR {e}"), errors.append(str(e)[:300])))
        url = (root + f"?route=hills&bots=3&touch=on&gpu=webgl2&quality=low&name={name.replace(' ', '%20')}"
               + "&safe-insets=" + ",".join(str(v) for v in insets))

        def steer():
            return json.loads(pg.evaluate("window.ocw.steer()") or "{}")

        def menu():
            return json.loads(pg.evaluate("window.ocw.menu()") or "{}")

        def hud():
            return json.loads(pg.evaluate("window.ocw.hudLayout()") or "{}")

        def screen():
            return json.loads(pg.evaluate("window.ocwScreen.state()") or "{}")

        def watts():
            return float(steer().get("key_power_w") or 0)

        def line():
            v = steer().get("line_m")
            return float(v) if v is not None else float("nan")

        def tap(x, y, wait=350):
            pg.touchscreen.tap(x, y)
            pg.wait_for_timeout(wait)

        def pointer(kind_, x, y, pid=41):
            pg.evaluate(POINTER_JS, [kind_, x, y, pid])

        def hold(x, y, seconds):
            """A finger held at (x, y); returns the state while it is held."""
            pointer("pointerdown", x, y)
            for _ in range(int(seconds / 0.1)):
                pg.wait_for_timeout(100)
                pointer("pointermove", x, y)
            st = steer()
            pointer("pointerup", x, y)
            pg.wait_for_timeout(300)
            return st

        def shot(what):
            path = f"{base}-{engine}-{case}-{what}.png"
            pg.screenshot(path=path)
            shots.append(path)
            return path

        def wait_ride():
            try:
                pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                     " && window.ocwFrames > 20 && window.ocw && window.ocw.steer"
                                     " && JSON.parse(window.ocw.steer() || '{}').riding", timeout=180000)
            except Exception as e:  # noqa: BLE001
                lines.append(f"[{label}] wait: {e}")
            pg.wait_for_timeout(2500)

        def centre(r):
            return ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2)

        def inside_safe(b):
            return (b[0] >= il - 0.5 and b[1] >= it - 0.5 and b[0] + b[2] <= w - ir + 0.5
                    and b[1] + b[3] <= h - ib + 0.5)

        def overlap(x, y):
            return (x[0] < y[0] + y[2] - 0.5 and y[0] < x[0] + x[2] - 0.5
                    and x[1] < y[1] + y[3] - 0.5 and y[1] < x[1] + x[3] - 0.5)

        def dom_box(sel):
            return pg.evaluate(f"""(() => {{ const e = document.querySelector('{sel}');
              if (!e || e.hidden || getComputedStyle(e).display === 'none') return null;
              const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }})()""")

        def tap_item(prefix, what="row"):
            """Taps the menu item whose label starts with `prefix`."""
            m = menu()
            items = m.get("items") or []
            i = next((i for i, t in enumerate(items) if t.strip().startswith(prefix)), None)
            rows = m.get("rows") or []
            if i is None or i >= len(rows) or not rows[i]:
                return None, m
            r = rows[i]
            tap(r[0] + r[2] / 2, r[1] + r[3] / 2, wait=500)
            return r, menu()

        pg.goto(url)
        wait_ride()
        # Chromium has Web Bluetooth: the Devices panel starts open over the
        # bottom of a phone's screen (its buttons take taps). Collapsed to
        # its button, as on the iPhone and once a device is connected.
        pg.evaluate("(() => { const p = document.getElementById('ocw-devices');"
                    " if (p && !p.hidden) document.getElementById('ocw-devices-hide').click(); })()")
        pg.evaluate(f"window.ocw.console({json.dumps(look)})")
        pg.wait_for_timeout(1200)
        st, lay, sc = steer(), hud(), screen()
        summary[label] = {"steer": st, "hud": {k: lay.get(k) for k in ("ui_scale", "top_panel", "buttons", "chevrons")}}

        # ---- the layout --------------------------------------------------
        btn = lay.get("buttons") or []
        check(f"{label}: the menu and hide buttons show, 44 px or more, inside the safe area",
              st.get("buttons") and len(btn) == 2 and all(b[2] >= 44 and b[3] >= 44 and inside_safe(b) for b in btn),
              {"buttons": btn, "insets": insets})
        boxes = {"top panel": lay.get("top_panel"), "top-right column": lay.get("top_right"),
                 "profile": lay.get("profile"), "first-ride tip": lay.get("touch_tip"),
                 "screen tip": sc.get("tip_rect"), "Devices button": dom_box("#ocw-devices-show")}
        for i, c in enumerate(lay.get("chevrons") or []):
            boxes[f"chevron {i}"] = c
        boxes = {k: v for k, v in boxes.items() if v}
        on_buttons = [(k, n) for k, v in boxes.items() for n, b in zip(("menu", "hide"), btn) if overlap(v, b)]
        check(f"{label}: the two buttons cover no HUD box, chevron, tip or page button", len(btn) == 2 and not on_buttons,
              {"overlaps": on_buttons, "boxes": boxes, "buttons": btn})
        centre_road = [w * 0.35, h * 0.4, w * 0.3, h * 0.35]
        check(f"{label}: … and leave the road in the middle free", not any(overlap(b, centre_road) for b in btn), btn)
        texts = pg.evaluate("Array.from(document.querySelectorAll('button')).filter((b) => !b.hidden"
                            " && getComputedStyle(b).display !== 'none').map((b) => b.textContent.trim())")
        check(f"{label}: no stop / − / + button (the app's or the page's)",
              not st.get("power_buttons") and not any(t in ("stop", "+", "−", "-") for t in texts), texts)
        want = quarter_zones(size, insets)
        zones = st.get("power_zones") or [0, 0]
        check(f"{label}: faster is the top quarter of the safe area, slower the bottom quarter",
              st.get("power") and abs(zones[0] - want[0]) < 1 and abs(zones[1] - want[1]) < 1,
              {"zones": zones, "want": want})
        check(f"{label}: the hints are drawn (chevrons, zone marks)", st.get("shown") and st.get("hints_drawn")
              and len(lay.get("chevrons") or []) == 2, st)
        shot("controls-shown")

        # Clear of the two buttons, the Devices button and the tips.
        top = (w * 0.62, it + (want[0] - it) * 0.72)
        bottom = (w * 0.3, want[1] + (h - ib - want[1]) * 0.45)
        mid_right = (w * 0.78, h * 0.55)
        mid_left = (w * 0.22, h * 0.55)

        # ---- faster / slower ---------------------------------------------
        l0, p0 = line(), watts()
        pg.touchscreen.tap(*top)
        pg.wait_for_timeout(250)
        st1 = steer()
        shot("faster")
        p1, l1 = watts(), line()
        check(f"{label}: a tap at the top is one step faster (+25 W) and does not steer",
              abs(p1 - p0 - 25) < 0.6 and abs(l1 - l0) < 1e-3, f"{p0} → {p1} W, line {l0:.3f} → {l1:.3f}")
        flash = st1.get("power_flash") or ""
        check(f"{label}: … with its feedback at the top edge (▲ … W)", flash.startswith("▲") and flash.endswith(" W"), flash)
        pg.wait_for_timeout(1200)
        check(f"{label}: … which fades", not steer().get("power_flash"), steer().get("power_flash"))
        pg.touchscreen.tap(*bottom)
        pg.wait_for_timeout(250)
        st2 = steer()
        shot("slower")
        p2 = watts()
        check(f"{label}: a tap at the bottom is one step slower (−10 W) and does not steer",
              abs(p1 - p2 - 10) < 0.6 and abs(line() - l0) < 1e-3, f"{p1} → {p2} W")
        flash = st2.get("power_flash") or ""
        check(f"{label}: … with its feedback at the bottom edge (▼ … W)", flash.startswith("▼") and flash.endswith(" W"), flash)
        # Held: the ramp (250 W/s up, down to 0 without the brake).
        held = hold(top[0], top[1], 1.2)
        p3 = watts()
        check(f"{label}: holding the top repeats: the power ramps up", p3 - p2 > 120 and held.get("power_held") == "faster",
              f"{p2} → {p3} W, held {held.get('power_held')}")
        held = hold(bottom[0], bottom[1], 0.7)
        p4 = watts()
        check(f"{label}: holding the bottom repeats: the power ramps down", p3 - p4 > 60 and held.get("power_held") == "slower",
              f"{p3} → {p4} W, held {held.get('power_held')}")
        hold(bottom[0], bottom[1], 2.2)
        st5 = steer()
        check(f"{label}: … to 0, and a hold never brakes", watts() == 0.0 and not st5.get("braking"), st5)
        # The half between: steering, no power.
        tap(*mid_right)
        l2 = line()
        check(f"{label}: a tap in the half between steers (+0.5 m) and changes no power",
              abs(l2 - l0 - 0.5) < 0.03 and watts() == 0.0, f"line {l0:.3f} → {l2:.3f}, {watts()} W")
        tap(*mid_left)
        check(f"{label}: … and back on the left", abs(line() - l0) < 0.03, f"{line():.3f}")
        if full:
            # No power left: slower is a stop, the top rides on.
            tap(*bottom)
            st6 = steer()
            check(f"{label}: slower with no power left stops: the brake, \"▼ stop\"",
                  st6.get("braking") and "stop" in (st6.get("power_flash") or ""), st6)
            shot("stopped")
            tap(*top)
            st7 = steer()
            check(f"{label}: the top rides on after the stop", not st7.get("braking") and watts() > 0, st7)
            tap(*bottom)
            tap(*bottom)
            tap(*bottom)
            pg.wait_for_timeout(600)

            # ---- no accidental steps ---------------------------------------
            pa, la = watts(), line()
            x, y = top
            pointer("pointerdown", x, y)
            for k in range(1, 9):
                pg.wait_for_timeout(40)
                pointer("pointermove", x - 12 * k, y)
            pointer("pointerup", x - 96, y)
            pg.wait_for_timeout(300)
            check(f"{label}: a drag across the top is no step", watts() == pa, f"{pa} → {watts()} W")
            # Two fingers (a pinch): one at the top, one at the bottom.
            pointer("pointerdown", top[0], top[1], 41)
            pg.wait_for_timeout(60)
            pointer("pointerdown", bottom[0], bottom[1], 42)
            for k in range(1, 9):
                pg.wait_for_timeout(40)
                pointer("pointermove", top[0], top[1] + 6 * k, 41)
                pointer("pointermove", bottom[0], bottom[1] - 6 * k, 42)
            pg.wait_for_timeout(400)
            pointer("pointerup", top[0], top[1] + 48, 41)
            pointer("pointerup", bottom[0], bottom[1] - 48, 42)
            pg.wait_for_timeout(300)
            page = pg.evaluate("[window.visualViewport ? visualViewport.scale : 1, scrollX, scrollY]")
            check(f"{label}: two fingers (a pinch) are no step, no steer, and the page does not move",
                  watts() == pa and abs(line() - la) < 1e-3 and page == [1, 0, 0],
                  f"{pa} → {watts()} W, line {la:.3f} → {line():.3f}, page {page}")
            tap(w * 0.5, it * 0.5)
            tap(w * 0.5, h - ib * 0.4)
            check(f"{label}: taps behind the notch and on the home indicator do nothing",
                  watts() == pa and abs(line() - la) < 1e-3, f"{watts()} W")

        # ---- the hide button ---------------------------------------------
        st = steer()
        hide_at = centre(st["hide_button"])
        tap(*hide_at, wait=700)
        st, sc = steer(), screen()
        check(f"{label}: the hide button hides the controls: nothing but the two buttons is drawn",
              st.get("controls_hidden") and not st.get("hints_drawn") and st.get("buttons")
              and not (hud().get("chevrons") or []) and sc.get("controls_hidden"),
              {k: st.get(k) for k in ("controls_hidden", "hints_drawn", "buttons", "shown")})
        check(f"{label}: … with the note the first time", st.get("toast") == "Controls hidden – they still work", st.get("toast"))
        opacity = pg.evaluate("getComputedStyle(document.getElementById('ocw-devices-show')).opacity")
        check(f"{label}: … the page's Devices button is not drawn either, no screen tip",
              opacity == "0" and not sc.get("tip"), {"opacity": opacity, "tip": sc.get("tip")})
        check(f"{label}: … and the zones are still on", st.get("power") and st.get("active"), st)
        shot("controls-hidden-note")
        pg.wait_for_timeout(4300)
        check(f"{label}: the note goes after a few seconds", not steer().get("toast"), steer().get("toast"))
        shot("controls-hidden")
        pb, lb = watts(), line()
        pg.touchscreen.tap(*top)
        pg.wait_for_timeout(250)
        stf = steer()
        shot("controls-hidden-faster")
        check(f"{label}: hidden controls still work: the top is faster (with its feedback)",
              abs(watts() - pb - 25) < 0.6 and (stf.get("power_flash") or "").startswith("▲"),
              f"{pb} → {watts()} W, {stf.get('power_flash')}")
        tap(*bottom)
        check(f"{label}: … the bottom slower", abs(watts() - pb - 15) < 0.6, f"{watts()} W")
        tap(*mid_right)
        check(f"{label}: … the right side steers", abs(line() - lb - 0.5) < 0.03, f"{lb:.3f} → {line():.3f}")
        tap(*mid_left)
        if full:
            # Remembered: a reload starts hidden.
            pg.reload()
            wait_ride()
            st = steer()
            check(f"{label}: the hidden state is remembered over a reload", st.get("controls_hidden")
                  and not st.get("hints_drawn") and st.get("buttons") and not st.get("toast"), st)
            hide_at = centre(st["hide_button"])
        tap(*hide_at, wait=700)
        st = steer()
        check(f"{label}: a second tap shows the controls again", not st.get("controls_hidden") and st.get("hints_drawn"), st)
        if full:
            tap(*hide_at, wait=700)
            check(f"{label}: hidden again: no second note", steer().get("controls_hidden") and not steer().get("toast"),
                  steer().get("toast"))
            tap(*hide_at, wait=700)

        # ---- the menu by touch -------------------------------------------
        st = steer()
        menu_at = centre(st["menu_button"])
        pm = watts()
        tap(*menu_at, wait=900)
        m = menu()
        check(f"{label}: a tap on the menu button opens the menu", m.get("open") and any(
            t.strip().startswith("Settings") for t in m.get("items") or []), m.get("items"))
        top_title, top_items = m.get("title"), m.get("items")
        shot("menu")
        rows = [r for r in (m.get("rows") or []) if r]
        # The menu is sized by the window's height (spec 3.2): its rows
        # are about 7 % of it – 60 px on a phone in portrait, 30 px turned.
        low = min((r[3] for r in rows), default=0)
        summary[label]["menu_row_px"] = round(low, 1)
        check(f"{label}: every row is a touch target on the screen ({low:.0f} px high)",
              len(rows) == len(m.get("items") or []) and low >= (44 if h >= 640 else 26) and all(
                  r[0] >= -0.5 and r[1] >= -0.5 and r[0] + r[2] <= w + 0.5 and r[1] + r[3] <= h + 0.5
                  for r in rows), [[round(v) for v in r] for r in rows])
        # A tap in the top zone beside the menu's rows.
        free = next((c for c in ((w * 0.62, top[1]), (w * 0.93, it + 24), (w * 0.5, it + 20), (w * 0.93, want[0] - 6))
                     if not any(r[0] - 4 <= c[0] <= r[0] + r[2] + 4 and r[1] - 4 <= c[1] <= r[1] + r[3] + 4 for r in rows)),
                    (w * 0.93, it + 24))
        tap(*free)
        check(f"{label}: under the menu the zones are off", watts() == pm and menu().get("open")
              and menu().get("title") == top_title, f"{pm} → {watts()} W at {free}")
        r, m = tap_item("Settings")
        check(f"{label}: a tap on Settings opens it", r and m.get("open") and m.get("title") != top_title
              and "Settings" in (m.get("title") or ""), {"title": m.get("title"), "items": m.get("items")})
        shot("menu-settings")
        r, m = tap_item("Controls")
        check(f"{label}: … on into Controls", r and "Controls" in (m.get("title") or ""),
              {"title": m.get("title"), "items": m.get("items")})
        r, m = tap_item("Touch and unicorn")
        check(f"{label}: … and Touch and unicorn mode", r and "Touch" in (m.get("title") or ""),
              {"title": m.get("title"), "items": m.get("items")})
        items = m.get("items") or []
        i = next((i for i, t in enumerate(items) if t.strip().startswith("Touch controls")), None)
        arrows = {(x[0], x[1]): x[2:] for x in m.get("arrows") or []}
        check(f"{label}: the Touch controls row has its ‹ › arrows", i is not None and (i, 1) in arrows and (i, -1) in arrows,
              {"items": items, "arrows": m.get("arrows")})
        if i is not None and (i, 1) in arrows:
            x = arrows[(i, 1)]
            tap(x[0] + x[2] / 2, x[1] + x[3] / 2, wait=600)
            m2 = menu()
            check(f"{label}: a tap on › changes the value: the controls are hidden",
                  steer().get("controls_hidden") and "hidden" in (m2.get("items") or [""] * (i + 1))[i],
                  (m2.get("items") or [])[i:i + 1])
            shot("menu-touch-controls")
            x = {(y[0], y[1]): y[2:] for y in m2.get("arrows") or []}.get((i, -1)) or arrows[(i, -1)]
            tap(x[0] + x[2] / 2, x[1] + x[3] / 2, wait=600)
            check(f"{label}: … and ‹ shows them again", not steer().get("controls_hidden"), steer().get("controls_hidden"))
        # Back, page by page, to the top.
        trail = []
        for _ in range(4):
            m = menu()
            if m.get("title") == top_title or not m.get("open"):
                break
            pg.wait_for_timeout(300)
            r, m = tap_item("Back")
            trail.append(m.get("title"))
            if not r:
                break
        m = menu()
        check(f"{label}: Back goes up page by page to the top of the menu", m.get("open") and m.get("title") == top_title
              and m.get("items") == top_items and len(trail) == 3, trail)
        if full:
            # A text field: the page's text bar (the on-screen keyboard).
            r, m = tap_item("Ride")
            r2, m = tap_item("Join")
            entered = bool(r and r2)
            if entered:
                items = m.get("items") or []
                link = next((t.strip().split(":")[0] for t in items if t.strip().lower().startswith(("link", "join link", "address"))), None)
                r3, m = tap_item(link) if link else (None, m)
            if not entered or not r3 or not steer().get("typing"):
                # The same field through its command (the page is the same).
                pg.evaluate("window.ocw.console('join.edit link')")
            pg.wait_for_timeout(900)
            sc = screen()
            check(f"{label}: typing into a text field shows the page's text bar", steer().get("typing") and sc.get("type_bar"),
                  {"typing": steer().get("typing"), "bar": sc.get("type_bar"), "by menu": entered, "items": m.get("items")})
            bar = dom_box("#ocw-type-input")
            check(f"{label}: … a field of 44 px inside the safe area", bar and bar[3] >= 44 and inside_safe(bar), bar)
            if bar:
                tap(bar[0] + bar[2] / 2, bar[1] + bar[3] / 2)
                focused = pg.evaluate("document.activeElement && document.activeElement.id")
                pg.keyboard.type("ocw.example/r1x", delay=30)
                pg.wait_for_timeout(500)
                m = menu()
                check(f"{label}: … a tap focuses it (the keyboard's field) and the typed text reaches the menu's field",
                      focused == "ocw-type-input" and any("ocw.example/r1x" in t for t in m.get("items") or []),
                      {"focused": focused, "items": m.get("items"), "typed": screen().get("typed")})
                shot("menu-text-field")
                pg.keyboard.press("Backspace")
                pg.keyboard.press("Backspace")
                pg.wait_for_timeout(400)
                m = menu()
                check(f"{label}: … Backspace deletes", any("ocw.example/r" in t and "r1x" not in t for t in m.get("items") or []),
                      m.get("items"))
                check(f"{label}: … and the zones' page check sees the text field", screen().get("overlay") == "text",
                      screen().get("overlay"))
                done = dom_box("#ocw-type-done")
                tap(done[0] + done[2] / 2, done[1] + done[3] / 2, wait=900)
                check(f"{label}: Done ends the typing and the bar goes", not steer().get("typing") and not screen().get("type_bar"),
                      {"typing": steer().get("typing"), "bar": screen().get("type_bar")})
            pg.evaluate("window.ocw.console('menu close')")
            pg.wait_for_timeout(500)
            tap(*menu_at, wait=900)
        st = steer()
        check(f"{label}: the menu button stays over the open menu", st.get("buttons") and menu().get("open"), st.get("buttons"))
        tap(*menu_at, wait=900)
        check(f"{label}: a tap on the menu button closes the menu", not menu().get("open") and steer().get("active"), menu().get("title"))
        check(f"{label}: nothing in the menu changed the power or stopped the ride", watts() == pm and not steer().get("braking"),
              f"{pm} → {watts()} W")

        if full:
            # ---- the page log's panel --------------------------------------
            pg.evaluate("window.ocwLog && window.ocwLog.show()")
            pg.wait_for_timeout(600)
            st, sc = steer(), screen()
            check(f"{label}: with the page log open the zones are off (steering stays)",
                  sc.get("overlay") == "log" and not st.get("power") and st.get("active"),
                  {"overlay": sc.get("overlay"), "power": st.get("power")})
            pg.evaluate("window.ocwLog.hide()")
            pg.wait_for_timeout(600)
            check(f"{label}: … and on again when it closes", steer().get("power") and screen().get("overlay") == "",
                  steer().get("power"))
            pl = watts()
            tap(*top)
            check(f"{label}: … the top is faster again", abs(watts() - pl - 25) < 0.6, f"{pl} → {watts()} W")
            # A last look at the ride for the video: steer, faster, slower.
            hold(mid_right[0], mid_right[1], 0.8)
            hold(top[0], top[1], 0.9)
            hold(mid_left[0], mid_left[1], 0.8)
            hold(bottom[0], bottom[1], 0.6)
            pg.wait_for_timeout(1200)
        shot("end")
        check(f"{label}: no page errors", not errors, errors[:5])
        video = pg.video
        ctx.close()
        if video_dir and video:
            try:
                out = f"{base}-{engine}-{case}-session.webm"
                shutil.move(video.path(), out)
                shutil.rmtree(video_dir, ignore_errors=True)
                videos.append(out)
                check(f"{label}: the touch session's video", os.path.getsize(out) > 20000, f"{out} ({os.path.getsize(out)} bytes)")
            except Exception as e:  # noqa: BLE001
                check(f"{label}: the touch session's video", False, repr(e))

    engines = [e.strip() for e in a.engines.split(",") if e.strip()]
    cases = [c.strip() for c in a.cases.split(",") if c.strip()]
    with sync_playwright() as pw:
        for engine in engines:
            try:
                if engine == "webkit":
                    browser = pw.webkit.launch()
                else:
                    web_browser = load("web_browser", "web_browser.py")
                    exe = web_browser.find()
                    args = ["--ignore-gpu-blocklist", "--disable-features=WebGPU,WebGPUService",
                            "--disable-blink-features=WebGPU", "--no-proxy-server", "--password-store=basic"]
                    args += (["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] if a.software
                             else ["--enable-features=Vulkan", "--use-angle=vulkan"])
                    browser = pw.chromium.launch(executable_path=exe, args=args)
                summary[engine] = browser.version
            except Exception as e:  # noqa: BLE001
                summary.setdefault("errors", []).append(f"{engine}: {e!r}")
                check(f"{engine}: the browser starts", False, repr(e))
                continue
            for case in cases:
                try:
                    run(engine, case, browser)
                except Exception as e:  # noqa: BLE001
                    summary.setdefault("errors", []).append(f"{engine} {case}: {e!r}")
                    check(f"{engine} {case}: the run finished", False, repr(e))
            browser.close()
        summary["ok"] = bool(checks) and all(c["ok"] for c in checks)
    httpd.shutdown()
    summary["checks"] = checks
    summary["screenshots"] = shots
    summary["videos"] = videos
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    failed = [c for c in checks if not c["ok"]]
    print(f"{len(checks) - len(failed)} of {len(checks)} checks passed; log: {base}.log")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
