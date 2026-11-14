#!/usr/bin/env python3
"""Headless browser test of the web app's devices (spec §14 Phase 10 B4; §11.4).

    scripts/test-web-ble.py [--out test-output/web] [--mode fake|emulation|real]
                            [--device KICKR] [--seconds 12] [--chromium chromium]

Serves target/web/ (`make web`), starts headless Chromium on WebGPU and
drives the page over the DevTools protocol (scripts/test-web.py's Cdp).
The device buttons are clicked with real mouse events (Web Bluetooth
needs the user gesture).

--mode fake (default, `make test-web-ble`): a fake `navigator.bluetooth`
  (scripts/web-ble-fake.js, injected before the page's scripts) with an
  FTMS trainer and a heart-rate strap. Checks: the virtual power first;
  "+ Trainer" connects, the rider's power comes from the device (321 W,
  not the virtual 120 W); SIM writes leave with the hills' gradients;
  "+ Heart rate"; ERG (`erg.target 180`: the write, and the ideal fake
  trainer holds 180 W); a drop (out of range 3 s): reconnecting, then
  connected again with control, start and the ERG target re-sent; SIM
  again; a reload: the devices come back without the chooser (permitted
  devices + this origin's memory); a browser without Web Bluetooth
  (`#nobt`): virtual power and the message.

--mode emulation: Chromium's own Web Bluetooth stack with a simulated
  peripheral (DevTools `BluetoothEmulation`; the chooser answered with
  `DeviceAccess.selectPrompt`): a KICKR-like device (Cycling Power + Wahoo's
  control point). The emulation cannot send notifications, so no power
  arrives and no write is answered; the check is the path: chooser,
  connection, discovery, subscriptions and the Wahoo control writes (unlock,
  SIM mode, grade) arriving at the "device".

--mode real: the machine's Bluetooth (BlueZ): the chooser's list is read
  with `DeviceAccess` and the first device whose name contains --device is
  picked; rides --seconds in SIM, 10 s ERG 100 W, SIM again.

Saves OUT/web-ble-<time>[-<step>].jpg, .log (console, fake device log) and
.json (summary). Exits 0 when every check of the mode passed.
"""

import argparse
import base64
import datetime
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tw = module("test_web", "test-web.py")

SIG = "0000{:04x}-0000-1000-8000-00805f9b34fb".format
WAHOO_CONTROL = "a026e005-0a7d-4ab3-97fa-f1500f9feb8b"


class Run:
    """The browser, its page session and the log."""

    def __init__(self, a, base):
        self.a, self.base = a, base
        self.lines, self.checks, self.shots = [], [], []
        self.t0 = time.time()
        self.seen = 0
        self.handlers = []

    def log(self, text):
        line = f"{time.time() - self.t0:7.2f} s  {text}"
        self.lines.append(line)
        print(line, flush=True)

    def check(self, name, ok, detail=""):
        self.checks.append({"check": name, "ok": bool(ok), "detail": detail})
        self.log(f"{'PASS' if ok else 'FAIL'} {name} {detail}")
        return ok

    def pump(self, seconds):
        """Reads events for `seconds`: console lines, exceptions, handlers."""
        end = time.time() + seconds
        while True:
            self.cdp.pump(min(0.1, max(0.0, end - time.time())))
            for e in self.cdp.events[self.seen:]:
                m = e["method"]
                if m == "Runtime.consoleAPICalled":
                    self.log(f"console {e['params'].get('type', 'log'):7} {tw.console_text(e)}")
                elif m == "Runtime.exceptionThrown":
                    d = e["params"]["exceptionDetails"]
                    text = d.get("exception", {}).get("description") or d.get("text", "")
                    if "Using exceptions for control flow" not in text:
                        self.log(f"EXCEPTION {text}")
                        self.exceptions.append(text)
                elif m == "Log.entryAdded":
                    entry = e["params"]["entry"]
                    self.log(f"browser {entry['level']} [{entry['source']}] {entry['text']}")
                for h in self.handlers:
                    h(e)
            self.seen = len(self.cdp.events)
            if time.time() >= end:
                return

    def devices(self):
        text = self.cdp.eval("window.ocw && window.ocw.devices ? window.ocw.devices() : ''")
        try:
            return json.loads(text) if text else None
        except ValueError:
            return None

    def wait(self, what, pred, timeout):
        """Pumps until pred(devices JSON) holds; returns the JSON (or None)."""
        end = time.time() + timeout
        last = None
        while time.time() < end:
            self.pump(0.25)
            last = self.devices()
            try:
                if last and pred(last):
                    return last
            except (KeyError, IndexError, TypeError):
                pass
        self.log(f"timeout waiting for {what}: {json.dumps(last)[:400]}")
        return None

    def console(self, line):
        self.cdp.eval(f"window.ocw.console({json.dumps(line)})")
        self.log(f"> {line}")

    def click(self, selector):
        """A real mouse click (a user gesture) on the element."""
        box = self.cdp.eval(f"""(() => {{
            const r = document.querySelector({json.dumps(selector)}).getBoundingClientRect();
            return [r.x + r.width / 2, r.y + r.height / 2];
        }})()""")
        for t in ("mousePressed", "mouseReleased"):
            self.cdp.call("Input.dispatchMouseEvent", {
                "type": t, "x": box[0], "y": box[1], "button": "left", "clickCount": 1})
        self.log(f"click {selector}")

    def shot(self, label):
        path = f"{self.base}-{label}.jpg" if label else f"{self.base}.jpg"
        self.cdp.screenshot(path)
        self.shots.append(path)
        self.log(f"screenshot {path}")
        return path

    def navigate(self, url):
        self.exceptions = []
        self.cdp.call("Page.navigate", {"url": url})
        self.log(f"navigate {url}")
        ok = self.wait("the ride", lambda j: j.get("joined"), 60)
        return ok is not None

    def fake_writes(self, key="trainer"):
        return self.cdp.eval(f"JSON.stringify(window.__ocwFakeBle.writesOf('{key}'))")


def sim_grades(writes):
    """FTMS Set Indoor Bike Simulation writes → gradients in %."""
    out = []
    for w in writes:
        b = w["bytes"]
        if b and b[0] == 0x11 and len(b) >= 5:
            g = int.from_bytes(bytes(b[3:5]), "little", signed=True)
            out.append(g / 100.0)
    return out


def fake_mode(r, url):
    a = r.a
    with open(os.path.join(HERE, "web-ble-fake.js")) as f:
        r.cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": f.read()})
    if not r.navigate(url):
        return r.check("the ride starts", False)
    j = r.wait("virtual power", lambda j: j["virtual"] and j["rider_power_w"] == 120, 15)
    r.check("virtual power before a device", j is not None,
            f"rider {j and j['rider_power_w']} W, virtual sample {j and j['virtual_sample']}")
    r.console("time noon")

    # The trainer: the chooser (fake) answers; power from the device.
    r.click('button[data-kind="trainer"]')
    j = r.wait("the trainer's power", lambda j: j["devices"] and j["devices"][0]["state"] == "connected"
               and j["rider_power_w"] == 321 and not j["virtual_sample"], 15)
    r.check("trainer connected, power from the device", j is not None,
            json.dumps(j and j["devices"]))
    req = json.loads(r.cdp.eval("JSON.stringify(window.__ocwFakeBle.requests)") or "[]")
    r.check("requestDevice filters and optional services",
            req and SIG(0x1826) in json.dumps(req[0]["filters"]) and WAHOO_CONTROL[:4] in json.dumps(req)
            and len(req[0].get("optionalServices", [])) >= 6, json.dumps(req[:1]))
    act = r.cdp.eval("window.__ocwFakeBle.log.filter(l => l.includes('requestDevice')).join('; ')")
    r.check("the chooser opened in a user gesture", "user activation: true" in (act or ""), act)

    r.click('button[data-kind="hr"]')
    j = r.wait("heart rate", lambda j: any(d["heart_rate_bpm"] == 142 for d in j["devices"]), 15)
    r.check("heart-rate strap connected", j is not None, json.dumps(j and j["devices"]))

    # Ride: SIM writes with the hills' gradients.
    r.pump(a.seconds)
    writes = json.loads(r.fake_writes() or "[]")
    ops = [w["bytes"][0] for w in writes]
    grades = sim_grades(writes)
    r.check("control requested and started", ops[:2] == [0x00, 0x07], f"ops {ops[:6]}")
    r.check("SIM gradient writes leave", len(grades) >= 3 and len(set(grades)) >= 2,
            f"{len(grades)} SIM writes, gradients {grades[:12]} %")
    j = r.devices()
    r.check("the rider rides on the device's power", j and j["rider_power_w"] == 321 and j["distance_m"] > 30,
            f"rider {j and j['rider_power_w']} W, {j and round(j['distance_m'], 1)} m, HUD {j and j['trainer']}")
    r.shot("sim")

    # ERG: the write, and the (ideal) trainer holds the target.
    n = len(writes)
    r.console("erg.target 180")
    j = r.wait("ERG 180 W", lambda j: j["rider_power_w"] == 180, 10)
    writes = json.loads(r.fake_writes() or "[]")
    erg = [w["bytes"] for w in writes[n:] if w["bytes"][0] == 0x05]
    r.check("ERG target write", [0x05, 180, 0] in erg, f"{erg}")
    r.check("the trainer holds the ERG target", j is not None, f"rider {j and j['rider_power_w']} W")
    r.shot("erg")

    # A drop: out of range 3 s; reconnect with the ERG target again.
    n = len(writes)
    r.cdp.eval("window.__ocwFakeBle.drop('trainer', 3000)")
    j = r.wait("reconnecting", lambda j: j["devices"][0]["state"] == "reconnecting", 5)
    r.check("the drop shows as reconnecting", j is not None, json.dumps(j and j["devices"][0]))
    j = r.wait("connected again", lambda j: j["devices"][0]["state"] == "connected"
               and j["rider_power_w"] == 180, 20)
    writes = json.loads(r.fake_writes() or "[]")
    again = [w["bytes"] for w in writes[n:]]
    r.check("reconnected after the drop", j is not None,
            r.cdp.eval("window.__ocwFakeBle.log.slice(-6).join('; ')"))
    r.check("control, start and the ERG target re-sent", again[:3] == [[0x00], [0x07], [0x05, 180, 0]],
            f"{again[:4]}")

    # SIM again.
    n = len(writes)
    r.console("trainer.mode sim")
    j = r.wait("SIM again", lambda j: j["rider_power_w"] == 321 and "SIM" in j["trainer"], 10)
    writes = json.loads(r.fake_writes() or "[]")
    r.check("back to SIM", j is not None and sim_grades(writes[n:]), f"{sim_grades(writes[n:])[:5]}")
    r.shot("")

    # A reload: the permitted devices of this origin come back by themselves.
    r.cdp.call("Page.reload", {})
    r.log("reload")
    j = r.wait("devices after the reload", lambda j: len(j["devices"]) == 2
               and all(d["state"] == "connected" for d in j["devices"]) and j["rider_power_w"] == 321, 40)
    r.check("devices reconnect after a reload without the chooser", j is not None,
            json.dumps(j and [(d["name"], d["state"]) for d in j["devices"]]))
    reqs = r.cdp.eval("window.__ocwFakeBle.requests.length")
    r.check("no chooser after the reload", reqs == 0, f"{reqs} requestDevice calls")
    r.pump(2)
    r.shot("reload")

    # A browser without Web Bluetooth.
    if not r.navigate(url.replace("name=Web+BLE", "name=No+Bluetooth") + "#nobt"):
        return r.check("the ride starts without Web Bluetooth", False)
    j = r.wait("virtual power", lambda j: j["bluetooth"] == "Missing" and j["virtual"]
               and j["rider_power_w"] == 120, 15)
    msg = r.cdp.eval("document.getElementById('ocw-devices-msg').textContent")
    disabled = r.cdp.eval("[...document.querySelectorAll('#ocw-devices button[data-kind]')].every(b => b.disabled)")
    r.check("no Web Bluetooth: virtual power and the message", j is not None and "no Web Bluetooth" in msg
            and disabled, msg)
    r.pump(3)
    r.shot("nobt")


def emulation_mode(r, url):
    """Chromium's Web Bluetooth with a simulated KICKR-like peripheral."""
    cdp = r.cdp
    address = "09:09:09:09:09:01"
    chars = {}
    writes = []
    ops = []
    answered = set()
    if not r.navigate(url):
        return r.check("the ride starts", False)
    # The domain lives on the page target in some Chromium builds and on
    # the browser target in others.
    on_page = True
    try:
        cdp.call("BluetoothEmulation.enable", {"state": "powered-on", "leSupported": True})
    except RuntimeError as e:
        r.log(f"BluetoothEmulation on the page: {e}; trying the browser target")
        on_page = False
        cdp.call("BluetoothEmulation.enable", {"state": "powered-on", "leSupported": True}, session=False)

    def emu(method, params):
        return cdp.call("BluetoothEmulation." + method, params, session=on_page)

    def emu_send(method, params):
        return cdp.send("BluetoothEmulation." + method, params, session=on_page)

    emu("simulatePreconnectedPeripheral", {
        "address": address, "name": "KICKR EMU", "manufacturerData": [],
        "knownServiceUuids": [SIG(0x1818)]})
    service = emu("addService", {"address": address, "serviceUuid": SIG(0x1818)})["serviceId"]
    for uuid, p in ((SIG(0x2a63), {"notify": True}), (WAHOO_CONTROL, {"write": True, "indicate": True})):
        cid = emu("addCharacteristic", {
            "serviceId": service, "characteristicUuid": uuid, "properties": p})["characteristicId"]
        chars[cid] = uuid
        # Chromium subscribes through the Client Characteristic
        # Configuration descriptor; without it: "GATT Error: Not supported".
        emu("addDescriptor", {"characteristicId": cid, "descriptorUuid": SIG(0x2902)})
    cdp.call("DeviceAccess.enable")

    def handler(e):
        m, p = e["method"], e.get("params", {})
        if m == "DeviceAccess.deviceRequestPrompted":
            r.log(f"chooser: {p['devices']}")
            # One answer per prompt: a second selectPrompt for the same
            # prompt fails a CHECK in Chromium 156 (the browser exits).
            if p["devices"] and p["id"] not in answered:
                answered.add(p["id"])
                cdp.send("DeviceAccess.selectPrompt", {"id": p["id"], "deviceId": p["devices"][0]["id"]})
        elif m == "BluetoothEmulation.gattOperationReceived":
            ops.append(p["type"])
            r.log(f"gatt {p['type']}")
            emu_send("simulateGATTOperationResponse",
                     {"address": p["address"], "type": p["type"], "code": 0})
        elif m == "BluetoothEmulation.characteristicOperationReceived":
            uuid = chars.get(p["characteristicId"], "?")
            data = list(base64.b64decode(p["data"])) if p.get("data") else []
            ops.append(f"{p['type']} {uuid[:8]}")
            r.log(f"characteristic {p['type']} {uuid} {data} {p.get('writeType', '')}")
            if p["type"] == "write":
                writes.append(data)
            resp = {"characteristicId": p["characteristicId"], "type": p["type"], "code": 0}
            if p["type"] == "read":
                resp["data"] = base64.b64encode(b"").decode()
            emu_send("simulateCharacteristicOperationResponse", resp)
        elif m == "BluetoothEmulation.descriptorOperationReceived":
            emu_send("simulateDescriptorOperationResponse", {
                "descriptorId": p["descriptorId"], "type": p["type"], "code": 0,
                **({"data": ""} if p["type"] == "read" else {})})

    r.handlers.append(handler)
    r.click('button[data-kind="trainer"]')
    j = r.wait("emulated device connected", lambda j: j["devices"] and j["devices"][0]["state"] == "connected", 20)
    r.check("chooser, connection, discovery (Chromium's stack)", j is not None, json.dumps(j and j["devices"]))
    r.pump(r.a.seconds)
    subs = [o for o in ops if o.startswith("subscribe")]
    r.check("subscriptions reach the device", len(subs) >= 2, f"{subs}")
    first = [w[0] for w in writes]
    r.check("Wahoo control writes leave: unlock, SIM mode, grade", first[:3] == [0x20, 0x43, 0x46], f"{first}")
    r.summary["emulation"] = {"ops": ops, "writes": writes}
    r.shot("emulation")


def real_mode(r, url):
    """The machine's Bluetooth: pick --device from the chooser and ride."""
    cdp = r.cdp
    if not r.navigate(url):
        return r.check("the ride starts", False)
    avail = cdp.eval("navigator.bluetooth ? navigator.bluetooth.getAvailability() : 'no navigator.bluetooth'",
                     await_promise=True)
    r.log(f"getAvailability: {avail}")
    r.summary["availability"] = avail
    cdp.call("DeviceAccess.enable")
    chosen = {}

    def handler(e):
        if e["method"] != "DeviceAccess.deviceRequestPrompted" or chosen:
            return
        p = e["params"]
        r.log(f"chooser: {p['devices']}")
        for d in p["devices"]:
            if r.a.device.lower() in (d.get("name") or "").lower():
                chosen.update(d)
                cdp.send("DeviceAccess.selectPrompt", {"id": p["id"], "deviceId": d["id"]})
                r.log(f"selected {d}")
                return

    r.handlers.append(handler)
    r.click('button[data-kind="trainer"]')
    j = r.wait(f"{r.a.device} connected", lambda j: j["devices"] and j["devices"][0]["state"] == "connected", 60)
    r.check(f"{r.a.device} found in the chooser", bool(chosen), json.dumps(chosen))
    if not r.check(f"{r.a.device} connected", j is not None, json.dumps(j and j["devices"])):
        r.shot("real")
        return
    r.console("time noon")
    r.pump(r.a.seconds)
    j = r.devices()
    r.check("samples from the trainer", j["devices"][0]["samples"] > 0, json.dumps(j["devices"][0]))
    r.check("SIM commands sent", j["commands_sent"] > 0 and "sim" in json.dumps(j["last_command"]),
            f"{j['commands_sent']} commands, last {j['last_command']}, HUD {j['trainer']}")
    r.shot("real-sim")
    r.console("erg.target 100")
    r.pump(10)
    j = r.devices()
    r.check("ERG command sent", "erg_w" in json.dumps(j["last_command"]), json.dumps(j["last_command"]))
    r.shot("real-erg")
    r.console("trainer.mode sim")
    r.pump(4)
    rejected = [l for l in r.lines if "rejected" in l or "refused control" in l]
    r.check("the trainer accepted control", not rejected, "; ".join(rejected[:3]))
    r.summary["real_device"] = j["devices"][0]


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--mode", choices=["fake", "emulation", "real"], default="fake")
    p.add_argument("--device", default="KICKR", help="--mode real: a part of the device's name")
    p.add_argument("--seconds", type=float, default=12.0)
    p.add_argument("--chromium", default=shutil.which("chromium") or "chromium")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--flags", default="", help="extra Chromium flags (space separated)")
    a = p.parse_args()
    if not os.path.exists(os.path.join(a.dir, "ocw_web_bg.wasm")):
        sys.exit(f"{a.dir}/ocw_web_bg.wasm missing: run `make web` first")
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"web-ble-{a.mode}-{stamp}")
    r = Run(a, base)
    r.summary = {"mode": a.mode}
    r.exceptions = []

    stats = {}
    httpd = tw.web_serve().server(a.dir, stats=stats)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    # Few bots, a gentle virtual power (the device's must show), FTP for ERG.
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?bots=2&power=120&name=Web+BLE"

    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        "--enable-unsafe-webgpu", "--ignore-gpu-blocklist",
        "--use-angle=vulkan", "--enable-gpu-rasterization",
    ]
    features = ["Vulkan", "WebGPUService"]
    if a.mode != "fake":
        # Web Bluetooth on Linux, and getDevices() (permission persistence).
        flags.append("--enable-experimental-web-platform-features")
        features.append("WebBluetoothNewPermissionsBackend")
    flags.append("--enable-features=" + ",".join(features))
    flags += [f for f in a.flags.split() if f]
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
    r.summary.update({"url": url, "flags": flags})
    try:
        targets = r.cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        r.cdp.session = r.cdp.call("Target.attachToTarget",
                                   {"targetId": page["targetId"], "flatten": True}, session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            r.cdp.call(m)
        r.cdp.call("Emulation.setDeviceMetricsOverride",
                   {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
        {"fake": fake_mode, "emulation": emulation_mode, "real": real_mode}[a.mode](r, url)
        if a.mode == "fake":
            r.summary["fake_log"] = json.loads(
                r.cdp.eval("JSON.stringify(window.__ocwFakeBle ? window.__ocwFakeBle.log : [])") or "[]")
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
    ok = bool(r.checks) and all(c["ok"] for c in r.checks)
    r.summary.update({"ok": ok, "checks": r.checks, "screenshots": r.shots})
    with open(base + ".log", "w") as f:
        f.write("\n".join(r.lines) + "\n\n--- summary ---\n" + json.dumps(r.summary, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(r.summary, f, indent=2)
    print(json.dumps({"ok": ok, "failed": [c for c in r.checks if not c["ok"]], "screenshots": r.shots}, indent=2))
    print(f"log: {base}.log")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
