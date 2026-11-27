#!/usr/bin/env python3
"""Headless browser test of ANT+ over WebUSB (spec 14 Phase 17 A5, 17B; 11.4).

    scripts/test-web-ant.py [--out test-output/web-ant] [--engine chromium|webkit]
                            [--dir target/web] [--chromium chromium]

Serves target/web/ (`make web`) and drives the page without hardware.

--engine chromium (default): headless Chromium over the DevTools protocol
  (scripts/test-web.py's Cdp) with a fake `navigator.usb`
  (scripts/web-usb-fake.js, injected before the page's scripts): one ANT
  USB-m stick whose bytes come from the page's simulated stick
  (`ocw_devices::ant::sim`, `window.ocw.antSim*`) with a heart-rate strap
  (1234, 141 bpm) and a power meter (5678, 247 W). Checks: Settings ->
  Trainer -> Devices shows *Add an ANT+ stick*; Enter on it opens the
  chooser in the key press's user gesture with the filters 0fcf:1008 /
  0fcf:1009; Devices -> ANT+ -> Enter the network key: the made-up test key
  typed key by key, the row masked; it is kept in this page's storage
  (`ocw.ant.key`) only; *Find ANT+ sensors* finds both; both connect: the
  heart rate and the power (not the virtual trainer's) reach the ride; the
  stick unplugged (`disconnect`) and plugged in again (`connect`): both
  come back by themselves; a reload: the stick (granted, `getDevices`), the
  key and the sensors come back without the chooser; the key in no network
  request (URLs, bodies, WebSocket frames), in no console line, the page
  log or the menu's text; `#nousb` (no WebUSB): the row says why and its
  page explains; the Devices panel's *+ ANT+ stick* button only with WebUSB.

--engine webkit: Safari's engine (Playwright, `make deps-webkit`), which has
  no WebUSB: the Devices page's *Add an ANT+ stick* says "no WebUSB here"
  and opens the page that says why (Firefox, Safari, iOS; Chrome or Edge;
  the sensor bridge); the panel's button stays hidden.

Saves OUT/web-ant-<engine>-<time>-<step>.jpg / .png, .log and .json. Exits
0 when every check passed.
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
sys.path.insert(0, HERE)

# The made-up key the simulated stick hears with (`NetworkKey::TEST`) –
# not the ANT+ network key, which is never in the repository.
TEST_KEY = "0A1B2C3D4E5F6071"
KEY_FORMS = [TEST_KEY, TEST_KEY.lower(), "0A 1B 2C 3D", "0a 1b 2c 3d", "0x0A, 0x1B", "1B2C3D4E5F"]
HR_ID, PM_ID = "ant:120:1234:1", "ant:11:5678:5"
ADD_ROW = "Add an ANT+ stick"


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def has_key(text):
    t = str(text)
    return any(f in t for f in KEY_FORMS)


# ---------------------------------------------------------------- Chromium

def chromium(a, base):
    import shutil
    import tempfile

    import web_browser  # noqa: E402  (the pinned browser)
    from web_ui_test import KEYS

    tw = module("test_web", "test-web.py")
    ble = module("test_web_ble", "test-web-ble.py")
    a.chromium = a.chromium or web_browser.find()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)

    r = ble.Run(a, base)
    r.summary = {"engine": "chromium", "browser": web_browser.describe(a.chromium)}
    r.exceptions = []
    requests = []

    def network(e):
        m, p = e["method"], e.get("params", {})
        if m == "Network.requestWillBeSent":
            req = p.get("request", {})
            requests.append({"url": req.get("url", ""), "body": req.get("postData", "")})
        elif m == "Network.webSocketFrameSent":
            requests.append({"url": "websocket", "body": p.get("response", {}).get("payloadData", "")})
        elif m == "Network.webTransportCreated":
            requests.append({"url": p.get("url", ""), "body": ""})

    r.handlers.append(network)

    def key(name, times=1):
        code, vk, text = KEYS[name]
        for _ in range(times):
            down = {"type": "keyDown", "key": name, "code": code,
                    "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}
            if text:
                down["text"] = text
            r.cdp.call("Input.dispatchKeyEvent", down)
            r.pump(0.12)
            r.cdp.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": name, "code": code,
                                                  "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
            r.pump(0.12)

    def type_char(c):
        code = f"Digit{c}" if c.isdigit() else f"Key{c.upper()}"
        vk = ord(c.upper())
        r.cdp.call("Input.dispatchKeyEvent", {"type": "keyDown", "key": c, "code": code, "text": c,
                                              "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
        r.pump(0.06)
        r.cdp.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": c, "code": code,
                                              "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
        r.pump(0.06)

    def menu():
        raw = r.cdp.eval("window.ocw && window.ocw.menu ? window.ocw.menu() : ''")
        try:
            return json.loads(raw) if raw else {}
        except ValueError:
            return {}

    def wait_menu(pred, timeout=4.0):
        end = time.time() + timeout
        m = menu()
        while time.time() < end and not pred(m):
            r.pump(0.1)
            m = menu()
        return m

    def focus(prefix):
        """Focuses the first item starting with `prefix` (paging with Next)."""
        for _ in range(4):
            m = menu()
            items = m.get("items", [])
            at = next((i for i, it in enumerate(items) if it.startswith(prefix)), None)
            if at is not None:
                key("ArrowDown", (at - m.get("focused", 0)) % len(items))
                return True
            nxt = next((i for i, it in enumerate(items) if it.startswith("Next")), None)
            if nxt is None:
                return False
            key("ArrowDown", (nxt - m.get("focused", 0)) % len(items))
            key("Enter")
        return False

    def select(prefix):
        ok = focus(prefix)
        r.check(f"menu: {prefix!r} is there", ok, json.dumps(menu().get("items")))
        if ok:
            key("Enter")
        return ok

    def ant():
        j = r.devices()
        return (j or {}).get("ant") or {}

    def device(j, id_):
        return next((d for d in (j or {}).get("devices", []) if d["id"] == id_), None)

    stats = {}
    httpd = tw.web_serve().server(a.dir, stats=stats)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    url = root + "?bots=1&power=120&name=Ada+Rider"
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-", dir=os.environ.get("TMPDIR"))
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        "--enable-unsafe-webgpu", "--ignore-gpu-blocklist",
        "--use-angle=vulkan", "--enable-gpu-rasterization",
        "--enable-features=Vulkan,WebGPUService",
    ]
    cmd_r, cmd_w = os.pipe()
    res_r, res_w = os.pipe()
    os.set_inheritable(cmd_r, True)
    os.set_inheritable(res_w, True)

    def child_fds():
        os.dup2(cmd_r, 3)
        os.dup2(res_w, 4)

    browser_log = open(base + ".browser.log", "w")
    proc = subprocess.Popen([a.chromium, *flags, "about:blank"], preexec_fn=child_fds,
                            close_fds=False, stdout=browser_log, stderr=subprocess.STDOUT)
    os.close(cmd_r)
    os.close(res_w)
    r.cdp = tw.Cdp(proc, cmd_w, res_r)
    r.summary.update({"url": url})
    try:
        targets = r.cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        r.cdp.session = r.cdp.call("Target.attachToTarget",
                                   {"targetId": page["targetId"], "flatten": True}, session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable", "Network.enable"):
            r.cdp.call(m)
        r.cdp.call("Emulation.setDeviceMetricsOverride",
                   {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
        with open(os.path.join(HERE, "web-usb-fake.js")) as f:
            r.cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": f.read()})
        if not r.navigate(url):
            raise RuntimeError("the ride did not start")
        r.console("time noon")
        j = r.wait("WebUSB seen", lambda j: j["ant"]["supported"], 15)
        r.check("WebUSB seen, no key yet, no stick", j is not None and not j["ant"]["key_set"]
                and not j["ant"]["granted"], json.dumps(j and j["ant"]))
        panel = json.loads(r.cdp.eval("""JSON.stringify({hidden: document.querySelector('button[data-ant]').hidden,
            text: document.querySelector('button[data-ant]').textContent})"""))
        r.check("the Devices panel has + ANT+ stick (WebUSB)", not panel["hidden"], json.dumps(panel))

        # Settings -> Trainer -> Devices: Add an ANT+ stick (Enter: the gesture).
        r.console("menu devices")
        wait_menu(lambda m: m.get("title") == "Devices")
        ok = focus(ADD_ROW)
        r.check("Devices page with Add an ANT+ stick", ok, json.dumps(menu().get("items")))
        r.shot("devices")
        key("Enter")
        r.pump(2)
        reqs = json.loads(r.cdp.eval("JSON.stringify(window.__ocwFakeUsb.requests)") or "[]")
        r.check("requestDevice in the key press's gesture, filters 0fcf:1008 / 1009",
                len(reqs) == 1 and reqs[0]["gesture"]
                and reqs[0]["options"] == {"filters": [{"vendorId": 4047, "productId": 4104},
                                                       {"vendorId": 4047, "productId": 4105}]},
                json.dumps(reqs))
        r.check("the stick is added (said on the page)",
                any("ANT USB-m stick added" in l for l in r.lines), "")

        # Devices -> ANT+ -> Enter the network key (typed, masked).
        r.console("menu ant")
        wait_menu(lambda m: m.get("title") == "ANT+")
        select("Enter the network key")
        for c in TEST_KEY[:10]:
            type_char(c)
        m = wait_menu(lambda m: any("10 of 16" in i for i in m.get("items", [])))
        field = next((i for i in m.get("items", []) if i.startswith("Network key")), "")
        r.check("the key's row is masked", "10 of 16" in field and not has_key(field), field)
        r.shot("key-typing")
        for c in TEST_KEY[10:]:
            type_char(c)
        key("Enter")
        j = r.wait("the key set, the stick found", lambda j: j["ant"]["key_set"] and j["ant"]["granted"], 8)
        stored = r.cdp.eval("localStorage.getItem('ocw.ant.key')")
        r.check("the key kept in this page's storage; the stick found (not opened: nothing needs it yet)",
                j is not None and stored == TEST_KEY and "ready" in (j["ant"]["status"] or ""),
                f"key_set {j and j['ant']['key_set']}, stored {'the key' if stored == TEST_KEY else stored!r}")
        others = json.loads(r.cdp.eval("""JSON.stringify(Object.keys(localStorage).filter(
            k => k !== 'ocw.ant.key' && (localStorage.getItem(k) || '').toUpperCase().includes('1B2C3D4E')))"""))
        r.check("the key in no other storage entry (settings, devices)", not others, json.dumps(others))
        m = menu()
        r.shot("ant-page")

        # Find ANT+ sensors.
        select("Find ANT+ sensors")
        j = r.wait("both sensors found", lambda j: set(j["ant"]["found"]) >= {HR_ID, PM_ID}, 20)
        r.check("the search finds the strap and the power meter", j is not None,
                json.dumps(j and j["ant"]))
        r.console("menu devices")
        m = wait_menu(lambda m: m.get("title") == "Devices")
        r.shot("found")
        focus("ANT+ heart rate 1234")
        r.shot("found-2")

        # Both connect (the device page's Connect runs devices.connect ID).
        key("Enter")
        wait_menu(lambda m: m.get("title") == "Device")
        select("Connect")
        r.console(f"devices.connect {PM_ID}")
        j = r.wait("both connected, power and heart rate in the ride",
                   lambda j: (device(j, HR_ID) or {}).get("state") == "connected"
                   and (device(j, PM_ID) or {}).get("state") == "connected"
                   and abs(j["rider_power_w"] - 247) < 3 and not j["virtual_sample"]
                   and (device(j, HR_ID) or {}).get("heart_rate_bpm") == 141, 20)
        r.check("the ANT+ strap and power meter ride (247 W, not the virtual 120 W; 141 bpm)",
                j is not None, json.dumps(j and j["devices"]))
        r.console("menu devices")
        wait_menu(lambda m: m.get("title") == "Devices")
        r.shot("connected")
        r.console("menu close")
        r.pump(2)
        r.shot("ride")

        # Unplugged and plugged in again.
        r.cdp.eval("window.__ocwFakeUsb.unplug()")
        j = r.wait("the sensors lost", lambda j: (device(j, HR_ID) or {}).get("state") != "connected"
                   and (device(j, PM_ID) or {}).get("state") != "connected", 10)
        r.check("unplugged: both lose their link", j is not None, json.dumps(j and j["devices"]))
        r.cdp.eval("window.__ocwFakeUsb.plug()")
        j = r.wait("back after the replug", lambda j: (device(j, HR_ID) or {}).get("state") == "connected"
                   and (device(j, PM_ID) or {}).get("state") == "connected"
                   and abs(j["rider_power_w"] - 247) < 3, 20)
        r.check("plugged in again: both come back by themselves", j is not None,
                json.dumps(j and j["devices"]))

        # A reload: the stick (getDevices), the key, the sensors – no chooser.
        if not r.navigate(url):
            raise RuntimeError("the ride did not start after the reload")
        j = r.wait("back after the reload", lambda j: (device(j, HR_ID) or {}).get("state") == "connected"
                   and (device(j, PM_ID) or {}).get("state") == "connected"
                   and abs(j["rider_power_w"] - 247) < 3, 30)
        reqs = json.loads(r.cdp.eval("JSON.stringify(window.__ocwFakeUsb.requests)") or "[]")
        r.check("after a reload the sensors ride again without the chooser",
                j is not None and not reqs, json.dumps(j and j["devices"]))
        r.shot("reload")

        # The key: never sent, never logged, never shown.
        log_text = r.cdp.eval(
            "window.ocwLog && window.ocwLog.lines ? window.ocwLog.lines().join('\\n') : ''") or ""
        leaks = [q["url"] for q in requests if has_key(q["url"]) or has_key(q["body"])]
        r.check("the key is in no network request", not leaks and len(requests) > 3,
                f"{len(requests)} requests, leaks {leaks[:3]}")
        r.check("the key is in no console line and not the page log",
                not any(has_key(l) for l in r.lines) and not has_key(log_text),
                f"{len(r.lines)} lines, page log {len(log_text)} chars")
        r.check("the key is not in the devices JSON or the menu", not has_key(json.dumps(r.devices()))
                and not has_key(json.dumps(menu())))
        r.summary["requests"] = len(requests)
        r.summary["fake_usb"] = json.loads(r.cdp.eval("JSON.stringify(window.__ocwFakeUsb)") or "{}")

        # A browser without WebUSB.
        r.cdp.call("Page.navigate", {"url": url + "&nousb=1#nousb"})
        r.wait("the ride without WebUSB", lambda j: j.get("joined") and not j["ant"]["supported"], 60)
        panel = json.loads(r.cdp.eval("JSON.stringify({hidden: document.querySelector('button[data-ant]').hidden})"))
        r.check("no WebUSB: the panel's button stays hidden", panel["hidden"], json.dumps(panel))
        r.console("menu devices")
        wait_menu(lambda m: m.get("title") == "Devices")
        focus(ADD_ROW)
        m = menu()
        row = next((i for i in m.get("items", []) if i.startswith(ADD_ROW)), "")
        r.check("no WebUSB: the row says so", "no WebUSB here" in row, row)
        r.shot("nousb-devices")
        key("Enter")
        m = wait_menu(lambda m: m.get("title") == "ANT+")
        r.check("no WebUSB: its page says why",
                any(i.startswith("Firefox, Safari, iOS") for i in m.get("items", []))
                and any(i.startswith("Use Chrome or Edge") for i in m.get("items", [])),
                json.dumps(m.get("items")))
        r.shot("nousb-ant")
        r.check("no uncaught exceptions", not r.exceptions, "; ".join(r.exceptions[:3]))
    except Exception as e:  # noqa: BLE001 – reported in the summary
        r.check("the test ran", False, repr(e))
    finally:
        try:
            r.cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
        browser_log.close()
    return r.checks, r.shots, r.lines, r.summary


# ---------------------------------------------------------------- WebKit

def webkit(a, base):
    venv = os.path.join(os.path.expanduser("~"), ".cache", "ocw", "playwright-venv")
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        py = a.python or os.path.join(venv, "bin", "python")
        if not os.path.exists(py):
            print("test-web-ant --engine webkit: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    ws = module("web_serve", "web-serve.py")
    missing = ws.missing_build(a.dir, ["ocw_web_webgl2_bg.wasm"])
    if missing:
        sys.exit(missing)
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    checks, shots, lines = [], [], []
    t0 = time.time()

    def log(t):
        line = f"{time.time() - t0:7.2f} s  {t}"
        lines.append(line)
        print(line, flush=True)

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:600]})
        log(f"{'PASS' if ok else 'FAIL'} {name} {str(detail)[:200]}")

    def menu(pg):
        raw = pg.evaluate("() => window.ocw && window.ocw.menu ? window.ocw.menu() : ''")
        return json.loads(raw) if raw else {}

    def wait(pg, pred, timeout):
        end = time.time() + timeout
        while time.time() < end:
            try:
                if pred():
                    return True
            except Exception:  # noqa: BLE001 – not ready yet
                pass
            pg.wait_for_timeout(250)
        return False

    summary = {"engine": "webkit"}
    errors = []
    with sync_playwright() as pw:
        browser = pw.webkit.launch()
        pg = browser.new_page(viewport={"width": 1280, "height": 720})
        pg.on("console", lambda m: log(f"console {m.type:7} {m.text}"))
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.goto(root + "?route=hills&bots=1&name=Bea+Climber")
        ok = wait(pg, lambda: json.loads(pg.evaluate("() => window.ocw.devices()"))["joined"], 90)
        check("WebKit: the ride starts (WebGL 2)", ok)
        has_usb = pg.evaluate("() => !!navigator.usb")
        check("WebKit has no navigator.usb", not has_usb)
        hidden = pg.evaluate("() => document.querySelector('button[data-ant]').hidden")
        check("WebKit: the panel's + ANT+ stick button is hidden", hidden)
        pg.evaluate("() => window.ocw.console('menu devices')")
        wait(pg, lambda: menu(pg).get("title") == "Devices", 5)
        m = menu(pg)
        row = next((i for i in m.get("items", []) if i.startswith(ADD_ROW)), "")
        check("WebKit: Add an ANT+ stick says no WebUSB here", "no WebUSB here" in row, json.dumps(m.get("items")))
        path = f"{base}-devices.png"
        pg.screenshot(path=path)
        shots.append(path)
        at = m["items"].index(row)
        for _ in range((at - m.get("focused", 0)) % len(m["items"])):
            pg.keyboard.press("ArrowDown")
            pg.wait_for_timeout(150)
        pg.keyboard.press("Enter")
        wait(pg, lambda: menu(pg).get("title") == "ANT+", 5)
        m = menu(pg)
        check("WebKit: its page says why (Firefox, Safari, iOS; Chrome or Edge; the bridge)",
              any(i.startswith("Firefox, Safari, iOS") for i in m.get("items", []))
              and any(i.startswith("Or the desktop app") for i in m.get("items", [])),
              json.dumps(m.get("items")))
        path = f"{base}-ant.png"
        pg.screenshot(path=path)
        shots.append(path)
        check("WebKit: no page errors", not errors, "; ".join(errors[:3]))
        browser.close()
    httpd.shutdown()
    return checks, shots, lines, summary


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-ant")
    p.add_argument("--engine", choices=["chromium", "webkit"], default="chromium")
    p.add_argument("--chromium", default=None, help="the browser (default: scripts/web_browser.py's)")
    p.add_argument("--python", default=None, help="--engine webkit: a Python with Playwright")
    p.add_argument("--size", default="1280x720")
    a = p.parse_args()
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"web-ant-{a.engine}-{stamp}")
    checks, shots, lines, summary = (chromium if a.engine == "chromium" else webkit)(a, base)
    ok = bool(checks) and all(c["ok"] for c in checks)
    summary.update({"ok": ok, "checks": checks, "screenshots": shots})
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    print(json.dumps({"ok": ok, "failed": [c for c in checks if not c["ok"]], "screenshots": shots}, indent=2))
    print(f"log: {base}.log")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
