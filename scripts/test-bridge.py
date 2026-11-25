#!/usr/bin/env python3
"""The local sensor bridge end to end (spec 14 Phase 10, 10D.2; 11.4).

    scripts/test-bridge.py [--seconds 120] [--devices-file PATH] [--out DIR]
                           [--world liechtenstein --route vaduz-malbun]

On a build machine with the trainer in range (marge and its Wahoo KICKR):

1. the desktop app shares its devices (`--share-bridge`) – on a headless
   Wayland compositor through `drive.sh shot-wayland`, with the devices of
   `--devices-file` (the KICKR; without one the virtual trainer at 180 W),
   the code naming 127.0.0.1 (no real address in the pictures; `--lan`
   names the machine's LAN address),
   in a config folder of its own (the code is not saved in the user's);
   after the ride it opens Devices → Share with a browser (the code, the
   address, the QR code) for its screenshot;
2. the code is read from the desktop's log ("sharing the trainer … code");
3. headless Chromium opens the browser app from the dev server with
   `?world=…&route=…&bridge=CODE` – the page pairs with the desktop over
   WebTransport (`serverCertificateHashes` from the code), as the Quest
   browser would from GitHub Pages – and rides SECONDS;
4. every 250 ms `window.ocw.devices()` is read: the bridge's state, the
   readings, their age on arrival, the ping round trip, the trainer
   control's round trip, the rider's power;
5. screenshots of the browser (riding, the bridge page) and the desktop.

Writes OUT/bridge-TIME.json (the summary), OUT/bridge-TIME-browser-*.jpg,
OUT/bridge-TIME-desktop.jpg and the logs. Exit 0 when the browser paired,
got readings from the desktop, the readings' age stayed under 150 ms (p95)
and the trainer control came back applied.
"""

import argparse
import datetime
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def load(name, file):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


test_web = load("test_web", "test-web.py")
CODE_RE = re.compile(r"sharing the trainer with a browser: code ([0-9A-Z-]+)")


def pct(values, p):
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))]


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--seconds", type=float, default=120.0)
    p.add_argument("--devices-file", default=None,
                   help="the desktop's devices (e.g. the KICKR); default: the virtual trainer")
    p.add_argument("--out", default="test-output/bridge")
    p.add_argument("--dir", default="target/web")
    p.add_argument("--world", default="liechtenstein")
    p.add_argument("--route", default="vaduz-malbun")
    p.add_argument("--gpu", choices=("webgpu", "webgl2"), default="webgpu")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--desktop-start", type=float, default=240.0,
                   help="seconds to wait for the desktop's code (it is built first)")
    p.add_argument("--lan", action="store_true",
                   help="the code names this machine's LAN address (its pictures then show it: "
                        "keep them out of test-output); default 127.0.0.1")
    a = p.parse_args()

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"bridge-{stamp}")
    summary = {"ok": False, "seconds": a.seconds, "devices_file": bool(a.devices_file)}

    # 1. The desktop: shares from the start, rides long enough for the
    # browser's ride, then shows its bridge page for the screenshot.
    cfg = tempfile.mkdtemp(prefix="ocw-bridge-cfg-")
    ride_s = a.seconds + 45
    desktop_args = ["--share-bridge", "--name", "Ada Rider",
                    "--at", f"{ride_s - 6:.0f}=menu open", "--at", f"{ride_s - 5:.0f}=menu bridge"]
    if a.devices_file:
        desktop_args += ["--devices-file", a.devices_file]
    else:
        desktop_args += ["--virtual", "--power", "180"]
    desktop_shot = f"{base}-desktop.jpg"
    env = dict(os.environ, OCW_CONFIG_DIR=cfg, OCW_TEST_OUTPUT="off",
               # No real address in the pictures (they reach the art
               # project): the code names loopback unless --lan.
               **({} if a.lan else {"OCW_BRIDGE_IP": "127.0.0.1"}),
               OCW_SHOT_TIMEOUT=str(int(ride_s + a.desktop_start + 120)))
    desktop_log = open(f"{base}-desktop-drive.log", "w")
    desktop = subprocess.Popen(
        [os.path.join(ROOT, ".claude/skills/run-ocw/drive.sh"), "shot-wayland", desktop_shot,
         f"{ride_s:.0f}", "--", *desktop_args],
        cwd=ROOT, env=env, stdout=desktop_log, stderr=subprocess.STDOUT)
    app_log = os.path.splitext(desktop_shot)[0] + ".log"

    # 2. The code from the desktop's log.
    code = None
    deadline = time.time() + a.desktop_start
    while time.time() < deadline and code is None:
        if desktop.poll() is not None:
            break
        if os.path.exists(app_log):
            m = CODE_RE.search(open(app_log, errors="replace").read())
            if m:
                code = m.group(1)
        time.sleep(0.5)
    if code is None:
        desktop.kill()
        summary["error"] = "the desktop app did not share (no code in its log)"
        json.dump(summary, open(f"{base}.json", "w"), indent=2)
        sys.exit(summary["error"])
    summary["code"] = code
    print(f"desktop shares: code {code}")
    # Give the trainer its connection (Bluetooth) before the ride.
    time.sleep(8)

    # 3. The browser app with the code.
    root_key = None
    if os.path.isfile("target/web-keys/root.hex"):
        root_key = open("target/web-keys/root.hex").read().strip()
    opts = [f"world={a.world}", f"route={a.route}", f"bridge={code}", "name=Bea%20Climber"]
    opts += [f"trust-root={root_key}"] if root_key else ["allow-unsigned=1"]
    stats = {}
    httpd = test_web.web_serve().server(a.dir, stats=stats)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?" + "&".join(opts)
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = test_web.chromium_flags(profile, w, h, a.gpu)
    blog = open(f"{base}-browser.log", "w")
    chromium = web_browser.find()
    proc, cdp = test_web.start_browser(chromium, flags, blog)
    summary["browser"] = web_browser.describe(chromium)
    samples, shots = [], {}
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
        cdp.call("Page.navigate", {"url": url})
        t0 = time.time()
        # Wait for the ride (the world streams first).
        while time.time() - t0 < 120:
            j = cdp.eval("window.ocw && window.ocw.devices ? window.ocw.devices() : ''")
            if j and json.loads(j).get("joined"):
                break
            cdp.pump(0.5)
        ride0 = time.time()
        summary["ride_start_s"] = round(ride0 - t0, 2)
        shot_at = {"riding": 25, "riding-late": a.seconds - 10}
        while time.time() - ride0 < a.seconds:
            cdp.pump(0.05)
            j = cdp.eval("window.ocw.devices()")
            if j:
                d = json.loads(j)
                b = d.get("bridge") or {}
                dev = next((x for x in d.get("devices", []) if x.get("id") == "bridge"), {})
                samples.append({
                    "t": round(time.time() - ride0, 2),
                    "state": b.get("state"), "paired": b.get("paired"),
                    "readings": b.get("readings"), "dropped": b.get("dropped"),
                    "age_ms": b.get("sample_age_ms"), "rtt_ms": b.get("rtt_ms"),
                    "control_rtt_ms": b.get("control_rtt_ms"),
                    "controls_applied": b.get("controls_applied"),
                    "bridge_power_w": dev.get("power_w"), "rider_power_w": d.get("rider_power_w"),
                    "virtual": d.get("virtual"), "mode": d.get("mode"),
                    "desktop_devices": b.get("desktop_devices"),
                    "last_command": d.get("last_command"),
                    "reader": b.get("reader"),
                    "reader_note": b.get("reader_note"),
                })
            for name, at in list(shot_at.items()):
                if time.time() - ride0 >= at:
                    shots[name] = cdp.screenshot(f"{base}-browser-{name}.jpg")
                    del shot_at[name]
            time.sleep(0.2)
        # The bridge page of the browser's menu.
        cdp.eval("window.ocw.console('menu bridge')")
        time.sleep(1.5)
        cdp.pump(0.1)
        shots["menu"] = cdp.screenshot(f"{base}-browser-menu.jpg")
        cdp.eval("window.ocw.console('menu close')")
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
        httpd.shutdown()

    # 4. The numbers.
    paired = [s for s in samples if s["paired"]]
    ages, seen = [], set()
    for s in samples:
        key = s["readings"]
        if s["age_ms"] is not None and key and key not in seen:
            seen.add(key)
            ages.append(s["age_ms"])
    rtts = [s["rtt_ms"] for s in samples if s["rtt_ms"] is not None]
    ctl = [s["control_rtt_ms"] for s in samples if s["control_rtt_ms"] is not None]
    last = samples[-1] if samples else {}
    powers = [s["bridge_power_w"] for s in samples if s["bridge_power_w"] is not None]
    summary.update({
        "samples": len(samples),
        "paired_after_s": paired[0]["t"] if paired else None,
        "paired_share": round(len(paired) / max(1, len(samples)), 3),
        "readings": last.get("readings"), "dropped": last.get("dropped"),
        "reading_rate_hz": round((last.get("readings") or 0) / a.seconds, 2),
        "age_ms": {"n": len(ages), "p50": pct(ages, 50), "p95": pct(ages, 95),
                   "max": max(ages) if ages else None},
        "rtt_ms": {"p50": pct(rtts, 50), "p95": pct(rtts, 95), "min": min(rtts) if rtts else None},
        "control_rtt_ms": {"p50": pct(ctl, 50), "max": max(ctl) if ctl else None},
        "controls_applied": last.get("controls_applied"),
        "bridge_power_w": {"min": min(powers) if powers else None,
                           "max": max(powers) if powers else None,
                           "mean": round(statistics.mean(powers), 1) if powers else None},
        "desktop_devices": last.get("desktop_devices"),
        "last_command": last.get("last_command"),
        # Who read the connection: the network worker, or the page's
        # thread where the browser has no worker for it (10G B3 worker).
        "reader": last.get("reader"),
        "reader_note": last.get("reader_note"),
        "browser_shots": shots,
    })
    # 5. The desktop finishes its ride and its screenshot.
    try:
        desktop.wait(timeout=ride_s + 120)
    except subprocess.TimeoutExpired:
        desktop.kill()
    summary["desktop_shot"] = desktop_shot if os.path.exists(desktop_shot) else None
    if os.path.exists(app_log):
        text = open(app_log, errors="replace").read()
        plain = re.sub(r"\x1b\[[0-9;]*m", "", text)
        # No device ids or Bluetooth addresses in the summary (it reaches
        # the art project's archive).
        summary["desktop_lines"] = [
            re.sub(r"\s+id=\S+", "", l.split(" INFO ", 1)[-1].strip())[:200]
            for l in plain.splitlines()
            if ("bridge" in l or "KICKR" in l) and "console:" in l][:30]
        fps = re.findall(r"fps[=: ]+([0-9.]+)", text)
        summary["desktop_fps_samples"] = fps[-5:]
    age_p95 = summary["age_ms"]["p95"]
    summary["ok"] = bool(paired) and (summary["readings"] or 0) > 0 \
        and age_p95 is not None and age_p95 < 150 and (summary["controls_applied"] or 0) > 0
    json.dump({**summary, "trace": samples}, open(f"{base}.json", "w"), indent=2)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
