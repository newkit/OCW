#!/usr/bin/env python3
"""Only S … S goes to Strava, in the browser (spec 2.8.1 Q6; §11.4).

    scripts/test-web-strava-q6.py [--out test-output/strava-q6] [--dir target/web]
                                  [--chromium chromium] [--size 1280x720]

One headless Chromium (WebGPU) rides the Hills test route: a Strava
recording (`strava.record start` … `stop`), the ride saved (`ride.save`),
a few seconds more without S, saved again. Then the training history's
pages of both rides: the one with the recording has *Upload to Strava (in
your browser)* – "your Strava recording" – and its upload downloads the
recording (not the ride); the one without has only the line *no Strava
recording – press S while riding*.

  OUT/web-strava-q6-<time>-ride-with-s.jpg      the ride with its recording
  OUT/web-strava-q6-<time>-ride-without-s.jpg   the ride without
  OUT/web-strava-q6-<time>.json / .log          the menu rows, checks

Exits 0 when both pages show what they should.
"""

import argparse
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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

HERE = os.path.dirname(os.path.abspath(__file__))
NO_RECORDING = "no Strava recording – press S while riding"


def test_web():
    """scripts/test-web.py as a module (Cdp, the server, console lines)."""
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/strava-q6")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    sys.path.insert(0, HERE)
    import web_ui_test

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-strava-q6-" + stamp)
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?bots=0&power=250"
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = tw.chromium_flags(profile, w, h, "webgpu", False)
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(a.chromium, flags, log)
    lines = []
    summary = {"ok": False, "browser": web_browser.describe(a.chromium), "url": url}
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
        ui = web_ui_test.UiTest(cdp, base, lines, a.out)
        # Until the local server ticks.
        end = time.time() + 90
        while time.time() < end and not any("ticks" in l and "local server running" in l
                                            for l in lines):
            ui.pump(0.3)
        for line in ("weather.transition instant", "weather clear", "time 2026-06-21 17:30"):
            ui.console(line)
        ui.pump(3.0)
        ui.console("strava.record start")
        ui.pump(6.0)
        ui.console("strava.record stop")
        ui.pump(2.0)
        ui.console("ride.save")
        ui.pump(4.0)
        ui.console("ride.save")
        ui.pump(1.0)
        # The older ride (2) has the recording, the newest (1) none.
        ui.console("menu ride 2")
        with_s = ui.wait_menu(lambda m: m.get("open") and any(
            i.startswith("Upload to Strava") for i in m.get("items", [])), timeout=5.0)
        ui.shot("ride-with-s")
        ui.console("menu ride 1")
        without = ui.wait_menu(lambda m: m.get("open") and any(
            NO_RECORDING in i for i in m.get("items", [])), timeout=5.0)
        ui.shot("ride-without-s")
        ui.close_menu()
        items_with = with_s.get("items", [])
        items_without = without.get("items", [])
        ui.check("the ride with S offers its recording",
                 any(i.startswith("Upload to Strava") and "your Strava recording" in i
                     for i in items_with), items_with)
        ui.check("the ride without S has no upload, the line to press S",
                 any(NO_RECORDING in i for i in items_without)
                 and not any(i.startswith("Upload to Strava") for i in items_without), items_without)
        summary.update({"with_s": items_with, "without_s": items_without,
                        "checks": ui.checks, "shots": ui.shots})
        summary["ok"] = all(c["ok"] for c in ui.checks)
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
    log.write("\n--- summary ---\n" + json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2, ensure_ascii=False)
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
