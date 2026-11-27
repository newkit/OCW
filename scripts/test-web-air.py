#!/usr/bin/env python3
"""The simulated aircraft in the browser (spec 14 Phase 18, T8; Q9 (b):
aircraft on every device; 11.4).

    scripts/test-web-air.py [--out test-output/web-air] [--dir target/web]
                            [--chromium chromium] [--size 1280x720]
                            [--backend webgpu|webgl2]

One headless Chromium rides the Hills test route (the default place's sky:
Zurich, Altenrhein, Friedrichshafen and the Alpine airways) with a varied
rider, sets a cold, humid morning with ice veils, turns into the unicorn,
looks up and takes pictures of the airliners and their contrails; then the
same with `traffic.air off`.

  OUT/web-air-<time>-contrails.jpg   the contrail sky
  OUT/web-air-<time>-off.jpg         aircraft off: an empty sky
  OUT/web-air-<time>.json / .log     the `air` output, checks

Exits 0 when `air` lists flying aircraft with persistent contrails and
with `traffic.air off` the command says they are off.
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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

HERE = os.path.dirname(os.path.abspath(__file__))


def test_web():
    """scripts/test-web.py as a module (Cdp, the server, console lines)."""
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-air")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    p.add_argument("--backend", default="webgpu", choices=["webgpu", "webgl2"])
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    import web_ui_test

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"web-air-{a.backend}-{stamp}")
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?bots=0&power=180"
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = tw.chromium_flags(profile, w, h, a.backend, False)
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(a.chromium, flags, log)
    lines = []
    summary = {"ok": False, "browser": web_browser.describe(a.chromium), "url": url,
               "backend": a.backend}
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
        end = time.time() + 90
        while time.time() < end and not any("ticks" in l and "local server running" in l
                                            for l in lines):
            ui.pump(0.3)
        # Another look than the default rider (CLAUDE.md: vary the riders).
        for line in ("weather.transition instant",
                     "weather clear clouds=0.2 type=cirrus temp=1 humidity=0.9 wind=8 dir=250",
                     "time 2026-11-27 10:30",
                     "character --hair-colour #2E7D32 --age 27",
                     "hud off", "god.flight hover", "god on"):
            ui.console(line)
            ui.pump(0.3)
        ui.pump(2.0)
        ui.console("god.pose here here 900 160 28")
        ui.pump(1.0)
        ui.console("air")
        ui.pump(4.0)
        log_on = [str(x) for x in (ui.console_state().get("log") or [])]
        ui.shot("contrails")
        ui.console("traffic air off")
        ui.pump(2.0)
        ui.console("air")
        ui.pump(0.5)
        log_off = [str(x) for x in (ui.console_state().get("log") or [])]
        ui.shot("off")
        flying = [l for l in log_on if "flying now" in l]
        ui.check("`air` lists aircraft flying near the camera",
                 any(not l.strip().startswith("0 ") for l in flying), flying)
        ui.check("cold humid upper air: persistent contrails",
                 any("contrails Persistent" in l for l in log_on), log_on[-14:])
        ui.check("traffic.air off: the command says the aircraft are off",
                 any("aircraft are off" in l for l in log_off), log_off[-4:])
        summary.update({"console": log_on[-16:] + log_off[-3:], "checks": ui.checks,
                        "shots": ui.shots})
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
