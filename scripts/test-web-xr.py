#!/usr/bin/env python3
"""The browser app in VR, in an emulated headset (spec §14 Phase 14, 14I W1; §11.4).

    scripts/test-web-xr.py [--out test-output/web-xr] [--dir target/web]
                           [--chromium chromium] [--size 1280x720]

Needs the WebXR build (`make web-xr`: target/web/ocw_web_webxr.js). Headless
Chromium has no headset, so the page gets the **Immersive Web Emulation
Runtime** (iwer 2.7.0, MIT, Meta – the runtime inside the Immersive Web
Emulator extension 2.x; `scripts/fetch-iwer.sh`) injected before it loads,
with its Oculus Quest 1 profile in stereo. The extension itself is not used:
its controls are a DevTools panel, which a headless browser has not; the
runtime is driven from here instead (`window.ocwXrDevice`: the headset's
position, orientation and visibility). The emulator's device frames run on
the display's clock (the real requestAnimationFrame), as a headset's
compositor does – never on the page's, which the app takes over during the
session.

On the test route (no bots, 150 W, noon, clear) it checks:

  1. the page offers Enter VR (isSessionSupported) – `-flat.jpg` – and the
     sensor bridge's pairing panel (14I Q7 (b)): a code typed and Pair
     pressed starts the bridge (`-pair.jpg`; no desktop app runs here, so
     it stays connecting);
  2. a click on it starts the session: two eye cameras side by side with
     the views' projections and poses, the app stepped from the XR frames,
     the canvas copied into a framebuffer of our own (`xrcopy=fbo`, the
     headset's opaque framebuffer stands in) – `-xr-view.jpg` (the emulated
     view: both eyes), `-xr-left.jpg`, `-xr-right.jpg`, `-xr-copy.png` (the
     copy read back);
  3. head tracking: the headset turned 40° left and tilted 25° down – the
     eyes follow (the app's eye poses) and the picture changes –
     `-xr-turned.jpg`, `-xr-down.jpg`; a short sweep as `-xr-sweep.mp4`
     (ffmpeg) with its frames;
  4. the frame timing in the emulator (a sanity number only: the headset's
     comes in W4); the beacon queued its report for the desktop;
  4b. the reference space (14I Q8 (a)): `local-floor`; the eyes as high
     above the road as the emulated head above the floor (seated 1.45 m,
     then 1.70 m out of the saddle – `-xr-standing.jpg`), across the ground
     at the rider's eye point; the saddle and eye heights of the profile;
  5. visible-blurred and back; exit: the flat view again, the window's
     frames run on – `-after.jpg`;
  6. without the emulator the page says why there is no VR and shows no
     button – `-unsupported.jpg`.

OUT/web-xr-<time>.json / .log: the summary, the console. Exits 0 when all
of it holds.
"""

import argparse
import base64
import datetime
import importlib.util
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

HERE = os.path.dirname(os.path.abspath(__file__))
RENDER_ERRORS = ("Caught rendering error", "Quitting the application", "panicked at")


def test_web():
    """scripts/test-web.py as a module (Cdp, the server, console lines)."""
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def emulator_script(iwer_path, eye_height):
    """The init script: the display's clock for the emulator, iwer with the
    Quest 1 profile in stereo, the headset at `eye_height` m."""
    src = open(iwer_path, encoding="utf-8").read()
    for a, b in (("globalThis.requestAnimationFrame(", "globalThis.ocwDisplayRaf("),
                 ("globalThis.cancelAnimationFrame(", "globalThis.ocwDisplayCaf(")):
        if src.count(a) != 1:
            sys.exit(f"iwer: expected one {a} (another version? see scripts/fetch-iwer.sh)")
        src = src.replace(a, b)
    return (
        "window.ocwDisplayRaf = window.requestAnimationFrame.bind(window);\n"
        "window.ocwDisplayCaf = window.cancelAnimationFrame.bind(window);\n"
        + src + "\n"
        "window.ocwXrDevice = new IWER.XRDevice(IWER.oculusQuest1, {stereoEnabled: true});\n"
        "window.ocwXrDevice.installRuntime({forceInstall: true});\n"
        f"window.ocwXrDevice.position.set(0, {eye_height}, 0);\n"
    )


def quat(yaw_deg, pitch_deg):
    """The head turned by yaw (left positive, about +Y), then pitched (up
    positive, about the turned +X): [x, y, z, w]."""
    y, p = math.radians(yaw_deg) / 2, math.radians(pitch_deg) / 2
    qy = (0.0, math.sin(y), 0.0, math.cos(y))
    qx = (math.sin(p), 0.0, 0.0, math.cos(p))
    ax, ay, az, aw = qy
    bx, by, bz, bw = qx
    return (aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz)


def pairing_code(ip, slot, secret):
    """A sensor-bridge pairing code (`ocw_bridge::code::PairingCode::compact`:
    the address form, the address, the port slot, the 30-bit secret in
    Crockford base32, a SHA-256 check symbol)."""
    import hashlib
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    a, b, c, d = (int(x) for x in ip.split("."))
    if (a, b) == (192, 168):
        form, addr, n = 0, (c << 8) | d, 16
    elif a == 10:
        form, addr, n = 1, (b << 16) | (c << 8) | d, 24
    elif a == 172 and 16 <= b <= 31:
        form, addr, n = 2, ((b - 16) << 16) | (c << 8) | d, 20
    else:
        form, addr, n = 3, (a << 24) | (b << 16) | (c << 8) | d, 32
    bits = []
    for value, k in ((form, 2), (addr, n), (slot, 2), (secret, 30)):
        bits += [(value >> i) & 1 for i in range(k - 1, -1, -1)]
    symbols = []
    for i in range(0, len(bits), 5):
        chunk = bits[i:i + 5] + [0] * (5 - len(bits[i:i + 5]))
        symbols.append(int("".join(map(str, chunk)), 2))
    symbols.append(hashlib.sha256(b"ocw-bridge-code" + bytes(symbols)).digest()[0] >> 3)
    s = "".join(alphabet[v] for v in symbols)
    return "-".join(s[i:i + 4] for i in range(0, len(s), 4))


def write_png(path, w, h, rgba_bottom_up):
    """An RGBA PNG from OpenGL's rows (bottom-up); no PIL needed."""
    stride = w * 4
    rows = [rgba_bottom_up[i * stride:(i + 1) * stride] for i in range(h)][::-1]
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(kind, data):
        c = struct.pack(">I", len(data)) + kind + data
        return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return path


def picture_stats(rgba, w, h):
    """Mean brightness of each half (left / right eye) and how many
    distinct colours there are (a frame of one colour has none to show)."""
    halves = [0.0, 0.0]
    counts = [0, 0]
    colours = set()
    for y in range(0, h, 2):
        for x in range(0, w, 2):
            i = (y * w + x) * 4
            r, g, b = rgba[i], rgba[i + 1], rgba[i + 2]
            k = 0 if x < w // 2 else 1
            halves[k] += (r + g + b) / 3
            counts[k] += 1
            colours.add((r >> 4, g >> 4, b >> 4))
    return {"mean": [round(halves[i] / max(1, counts[i]), 1) for i in (0, 1)], "colours": len(colours)}


def difference(a, b):
    """Mean absolute difference (0 … 255) of two equal-sized pictures."""
    if not a or not b or len(a) != len(b):
        return None
    step = 16
    s = sum(abs(a[i] - b[i]) for i in range(0, len(a), step))
    return round(s / (len(a) / step), 2)


class Run:
    def __init__(self, tw, cdp, lines):
        self.tw, self.cdp, self.lines = tw, cdp, lines
        self.seen = 0
        self.t0 = time.time()
        self.errors = []
        self.ticking = False

    def drain(self):
        for e in self.cdp.events[self.seen:]:
            now = time.time() - self.t0
            if e["method"] == "Runtime.consoleAPICalled":
                text = self.tw.console_text(e)
                self.lines.append(f"{now:7.2f} s  {text}")
                if "local server running" in text:
                    self.ticking = True
                if any(k in text for k in RENDER_ERRORS):
                    self.errors.append(text[:400])
            elif e["method"] == "Runtime.exceptionThrown":
                d = e["params"]["exceptionDetails"]
                text = d.get("exception", {}).get("description") or d.get("text", "")
                self.lines.append(f"{now:7.2f} s  EXCEPTION {text}")
                if "Using exceptions for control flow" not in text:
                    self.errors.append(text[:400])
        self.seen = len(self.cdp.events)

    def wait(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            self.cdp.pump(0.05)
            self.drain()

    def until(self, what, timeout):
        end = time.time() + timeout
        while time.time() < end:
            v = what()
            if v:
                return v
            self.wait(0.2)
        return None

    def js(self, expr):
        return self.cdp.eval(expr)

    def json(self, expr):
        v = self.js(f"JSON.stringify({expr})")
        return json.loads(v) if v else None

    def xr(self):
        return self.json("window.ocw && window.ocw.xr ? JSON.parse(window.ocw.xr() || 'null') : null") or {}

    def page_xr(self):
        return self.json("window.ocwXr ? window.ocwXr.state() : null") or {}

    def head(self, yaw=0.0, pitch=0.0, height=None):
        x, y, z, w = quat(yaw, pitch)
        js = f"window.ocwXrDevice.quaternion.set({x}, {y}, {z}, {w});"
        if height is not None:
            js += f"window.ocwXrDevice.position.set(0, {height}, 0);"
        self.js(js)

    def click(self, element_id):
        r = self.json(f"(() => {{ const b = document.getElementById('{element_id}');"
                      " if (!b || b.hidden) return null; const r = b.getBoundingClientRect();"
                      " return [r.x + r.width / 2, r.y + r.height / 2]; })()")
        if not r:
            return False
        for kind in ("mousePressed", "mouseReleased"):
            self.cdp.call("Input.dispatchMouseEvent",
                          {"type": kind, "x": r[0], "y": r[1], "button": "left", "clickCount": 1})
        return True

    def click_selector(self, selector):
        r = self.json(f"(() => {{ const b = document.querySelector({json.dumps(selector)});"
                      " if (!b) return null; const r = b.getBoundingClientRect();"
                      " return [r.x + r.width / 2, r.y + r.height / 2]; })()")
        if not r:
            return False
        for kind in ("mousePressed", "mouseReleased"):
            self.cdp.call("Input.dispatchMouseEvent",
                          {"type": kind, "x": r[0], "y": r[1], "button": "left", "clickCount": 1})
        return True

    def readback(self, width=480):
        r = self.json(f"window.ocwXr.readback({width})")
        if not r:
            return None, 0, 0
        return base64.b64decode(r["rgba"]), r["w"], r["h"]

    def shot(self, path, clip=None):
        params = {"format": "jpeg", "quality": 90}
        if clip:
            params["clip"] = dict(clip, scale=1)
        data = self.cdp.call("Page.captureScreenshot", params)["data"]
        with open(path, "wb") as f:
            f.write(base64.b64decode(data))
        return path


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-xr")
    p.add_argument("--chromium", default=None,
                   help="the browser (default: OCW_BROWSER, else the pinned Chromium of `make deps-browser`,"
                        " else the system's chromium; scripts/web_browser.py)")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--eye-height", type=float, default=1.45,
                   help="the emulated head above the floor (m; a rider on the saddle)")
    p.add_argument("--query", default="bots=0&power=150&start=1.2")
    p.add_argument("--bridge-code", default=None,
                   help="a pairing code typed into the page's panel (default: one for 192.168.0.250, where"
                        " no desktop app answers)")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    a.bridge_code = a.bridge_code or pairing_code("192.168.0.250", 0, 0x2A5F1C3)
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_webxr_bg.wasm", "ocw-xr.js"])
    if missing:
        sys.exit(missing + "\n  (the WebXR build: make web-xr)")
    iwer = subprocess.run([os.path.join(HERE, "fetch-iwer.sh")], check=True,
                          capture_output=True, text=True).stdout.strip().splitlines()[-1]

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-xr-" + stamp)
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    url = f"{root}?xr=1&xrcopy=fbo&{a.query}"
    w, h = (int(v) for v in a.size.split("x"))
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    # WebGL 2 through ANGLE on Vulkan (the GPU); no WebGPU needed.
    flags = tw.chromium_flags(profile, w, h, gpu="webgl2")
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(a.chromium, flags, log)
    lines = []
    s = {"url": url, "ok": False, "browser": web_browser.describe(a.chromium), "checks": {},
         "emulator": "iwer 2.7.0 (Immersive Web Emulation Runtime), oculusQuest1, stereo", "shots": {}}
    checks = s["checks"]
    run = Run(tw, cdp, lines)
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": w, "height": h, "deviceScaleFactor": 1, "mobile": False})
        script = cdp.call("Page.addScriptToEvaluateOnNewDocument",
                          {"source": emulator_script(iwer, a.eye_height)})["identifier"]
        cdp.call("Page.navigate", {"url": url})
        run.until(lambda: run.ticking, 120)
        checks["ticking"] = run.ticking
        for line in ("weather.transition instant", "weather clear", "time 2026-06-21 12:00", "time pause"):
            run.js(f"window.ocw.console({json.dumps(line)})")
        offered = run.until(lambda: run.page_xr().get("state") == "offered", 30)
        checks["offered"] = bool(offered) and run.xr().get("state") == "offered"
        run.wait(4.0)
        s["shots"]["flat"] = run.shot(base + "-flat.jpg")

        # 1b. The sensor bridge's panel before Enter VR (14I Q7 (b)): type
        # the desktop's code, press Pair (real input events).
        panel = run.js("(() => { const p = document.getElementById('ocw-xr-pair'); return !!p && !p.hidden; })()")
        run.js("document.getElementById('ocw-xr-code').focus()")
        cdp.call("Input.insertText", {"text": a.bridge_code})
        run.click_selector("#ocw-xr-pair button")
        paired = run.until(lambda: (run.json("JSON.parse(window.ocw.devices() || 'null')") or {})
                           .get("bridge", {}).get("code"), 15)
        run.wait(1.5)
        line = run.js("document.getElementById('ocw-xr-pair-state').textContent")
        s["bridge"] = {"panel": panel, "code": paired, "line": line,
                       "devices": (run.json("JSON.parse(window.ocw.devices() || 'null')") or {}).get("bridge")}
        s["shots"]["pair"] = run.shot(base + "-pair.jpg")
        want = a.bridge_code.replace("-", "").upper()
        checks["bridge_pairing_on_the_page"] = bool(panel) and bool(paired) \
            and paired.replace("-", "") == want and want[:4] in (line or "").replace("-", "")

        # 2. Enter VR by a click (the user gesture).
        clicked = run.click("ocw-xr-enter")
        running = run.until(lambda: run.xr().get("in_headset") and run.xr().get("placed", 0) > 10, 30)
        x = run.xr()
        s["entered"] = {"clicked": clicked, "app": x, "page": run.page_xr()}
        eyes = x.get("eyes") or []
        side_by_side = (len(eyes) == 2 and eyes[0]["viewport"][0] == 0
                        and eyes[1]["viewport"][0] == eyes[0]["viewport"][2])
        ipd = math.dist(eyes[0]["pos"], eyes[1]["pos"]) if len(eyes) == 2 else None
        s["ipd_m"] = ipd
        checks["running"] = bool(running) and x.get("state") == "running"
        checks["two_eyes_side_by_side"] = side_by_side
        checks["eyes_an_ipd_apart"] = ipd is not None and abs(ipd - 0.063) < 0.005
        checks["copy_into_framebuffer"] = run.page_xr().get("copy") == "fbo" and run.page_xr().get("copies", 0) > 10
        run.wait(3.0)
        s["shots"]["view"] = run.shot(base + "-xr-view.jpg")
        s["shots"]["left"] = run.shot(base + "-xr-left.jpg", {"x": 0, "y": 0, "width": w // 2, "height": h})
        s["shots"]["right"] = run.shot(base + "-xr-right.jpg", {"x": w // 2, "y": 0, "width": w - w // 2, "height": h})
        ahead, rw, rh = run.readback()
        if ahead:
            s["shots"]["copy"] = write_png(base + "-xr-copy.png", rw, rh, ahead)
            s["copy_picture"] = picture_stats(ahead, rw, rh)
        checks["copy_has_a_picture"] = bool(ahead) and s.get("copy_picture", {}).get("colours", 0) > 20

        # 3. Head tracking.
        yaw0 = (eyes[0]["yaw_deg"] if eyes else None)
        run.head(yaw=40.0)
        run.wait(1.5)
        turned = run.xr().get("eyes") or []
        s["shots"]["turned"] = run.shot(base + "-xr-turned.jpg")
        left_pic, _, _ = run.readback()
        run.head(yaw=0.0, pitch=-25.0)
        run.wait(1.5)
        down = run.xr().get("eyes") or []
        s["shots"]["down"] = run.shot(base + "-xr-down.jpg")
        down_pic, _, _ = run.readback()
        dyaw = None
        if yaw0 is not None and turned:
            dyaw = (turned[0]["yaw_deg"] - yaw0 + 180) % 360 - 180
        s["head"] = {"yaw0_deg": yaw0, "turned_yaw_change_deg": dyaw,
                     "down_pitch_deg": down[0]["pitch_deg"] if down else None,
                     "picture_change_turned": difference(ahead, left_pic),
                     "picture_change_down": difference(ahead, down_pic)}
        checks["eyes_follow_the_turn"] = dyaw is not None and abs(dyaw - 40.0) < 3.0
        checks["eyes_follow_the_tilt"] = bool(down) and abs(down[0]["pitch_deg"] + 25.0) < 3.0
        checks["picture_changes"] = (s["head"]["picture_change_turned"] or 0) > 3 \
            and (s["head"]["picture_change_down"] or 0) > 3
        # The sweep: left to right and back, as a short video.
        frames_dir = base + "-xr-sweep"
        os.makedirs(frames_dir, exist_ok=True)
        n = 48
        for i in range(n):
            run.head(yaw=50.0 * math.sin(2 * math.pi * i / n), pitch=-8.0 * math.sin(4 * math.pi * i / n))
            run.wait(0.08)
            run.shot(os.path.join(frames_dir, f"{i:03d}.jpg"))
        run.head()
        ffmpeg = shutil.which("ffmpeg")
        if ffmpeg:
            mp4 = base + "-xr-sweep.mp4"
            r = subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-framerate", "12",
                                "-i", os.path.join(frames_dir, "%03d.jpg"), "-c:v", "libx264",
                                "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", mp4])
            if r.returncode == 0:
                s["shots"]["sweep"] = mp4

        # 4. The timing in the emulator (sanity only).
        run.wait(6.0)
        x = run.xr()
        s["timing"] = {k: x.get(k) for k in ("fps", "gap_ms_p50", "gap_ms_p95", "work_ms_p50", "work_ms_p95",
                                             "longest_gap_ms", "late", "frames", "framebuffer")}
        s["timing"]["page"] = run.page_xr()
        checks["frames_counted"] = (x.get("frames") or 0) > 100 and x.get("fps") is not None
        beacon = run.page_xr().get("beacon") or {}
        s["beacon"] = beacon
        checks["beacon_queued"] = beacon.get("sent", 0) >= 1 and not beacon.get("error")

        # 4b. The reference space and the eye height (14I Q8 (a)).
        seated = run.xr()
        run.head(height=1.70)
        run.wait(1.5)
        standing = run.xr()
        s["shots"]["standing"] = run.shot(base + "-xr-standing.jpg")
        s["shots"]["standing_left"] = run.shot(base + "-xr-standing-left.jpg",
                                               {"x": 0, "y": 0, "width": w // 2, "height": h})
        run.head(height=a.eye_height)
        run.wait(1.0)
        keys = ("space", "saddle_m", "eye_seated_m", "eye_standing_m", "head_m", "eye_above_road_m",
                "eye_from_point_m")
        s["floor"] = {"seated": {k: seated.get(k) for k in keys}, "standing": {k: standing.get(k) for k in keys}}
        near = lambda v, want, tol: v is not None and abs(v - want) < tol  # noqa: E731
        checks["local_floor"] = seated.get("space") == "local-floor"
        checks["eye_height_is_the_real_head"] = near(seated.get("eye_above_road_m"), a.eye_height, 0.02) \
            and near(standing.get("eye_above_road_m"), 1.70, 0.02)
        checks["eyes_at_the_rider_eye_point"] = near(seated.get("eye_from_point_m"), 0.0, 0.02) \
            and near(standing.get("eye_from_point_m"), 0.0, 0.02)
        checks["profile_heights"] = (seated.get("saddle_m") or 0) > 0.6 \
            and (seated.get("eye_seated_m") or 0) > (seated.get("saddle_m") or 9) \
            and (seated.get("eye_standing_m") or 0) > (seated.get("eye_seated_m") or 9)

        # 5. Visibility and exit.
        run.js("window.ocwXrDevice.updateVisibilityState('visible-blurred')")
        blurred = run.until(lambda: run.xr().get("state") == "visible-blurred", 10)
        run.js("window.ocwXrDevice.updateVisibilityState('visible')")
        back = run.until(lambda: run.xr().get("state") == "running", 10)
        checks["blurred_and_back"] = bool(blurred) and bool(back)
        run.js("window.ocwXr.exit()")
        ended = run.until(lambda: run.xr().get("state") == "ended", 10)
        run.wait(2.0)
        f0 = run.js("window.ocwFrames")
        run.wait(2.0)
        f1 = run.js("window.ocwFrames")
        s["after"] = {"frames_in_2s": (f1 or 0) - (f0 or 0), "page": run.page_xr(),
                      "button": run.js("document.getElementById('ocw-xr-enter').textContent")}
        s["shots"]["after"] = run.shot(base + "-after.jpg")
        checks["exit_back_to_flat"] = bool(ended) and s["after"]["frames_in_2s"] > 10 \
            and s["after"]["button"] == "Enter VR" and any("the flat view again" in t for t in lines)

        # 6. Without the emulator.
        cdp.call("Page.removeScriptToEvaluateOnNewDocument", {"identifier": script})
        run.ticking = False
        cdp.call("Page.navigate", {"url": url})
        run.until(lambda: run.ticking, 120)
        unsupported = run.until(lambda: run.page_xr().get("state") == "unsupported", 30)
        run.wait(2.0)
        note = run.js("(() => { const n = document.getElementById('ocw-xr-note'); return n && !n.hidden ? n.textContent : ''; })()")
        button_hidden = run.js("(() => { const b = document.getElementById('ocw-xr-enter'); return !b || b.hidden; })()")
        s["unsupported"] = {"note": note, "button_hidden": button_hidden, "app": run.xr().get("state")}
        s["shots"]["unsupported"] = run.shot(base + "-unsupported.jpg")
        checks["unsupported_says_why"] = bool(unsupported) and bool(note) and bool(button_hidden) \
            and run.xr().get("state") == "unsupported"
        checks["no_errors"] = not run.errors
        s["errors"] = run.errors
        s["ok"] = all(checks.values())
    except Exception as e:  # noqa: BLE001 – reported in the summary
        s["error"] = repr(e)
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
    log.write("\n--- console ---\n" + "\n".join(lines) + "\n")
    log.write("\n--- summary ---\n" + json.dumps(s, indent=2) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(s, f, indent=2)
    print(json.dumps({k: s.get(k) for k in ("ok", "checks", "timing", "head", "ipd_m", "shots", "error",
                                           "errors")}, indent=2))
    sys.exit(0 if s["ok"] else 1)


if __name__ == "__main__":
    main()
