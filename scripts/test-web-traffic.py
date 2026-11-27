#!/usr/bin/env python3
"""Settings → Traffic in the browser (spec 14 Phase 18, T1; Q9; 11.4).

    scripts/test-web-traffic.py [--out test-output/web-traffic] [--dir target/web]
                                [--chromium chromium] [--size 1280x720]

One headless Chromium (WebGPU) rides the Hills test route with a varied
rider (`character …`), opens Settings → Traffic (`menu traffic`) and its
*Sound and live data* page. The browser has no road traffic (Q9 (b)): no
*Road traffic*, *Driver manners* or *Live traffic data* rows; *Traffic*,
*Aircraft*, *UFOs* and the dimmed *Aircraft: live* are there, and the
`traffic` command says road traffic is the desktop's.

  OUT/web-traffic-<time>-traffic.jpg        Settings → Traffic
  OUT/web-traffic-<time>-traffic-more.jpg   … → Sound and live data
  OUT/web-traffic-<time>.json / .log        the menu rows, checks

Exits 0 when both pages show what they should.
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
    p.add_argument("--out", default="test-output/web-traffic")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    import web_ui_test

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-traffic-" + stamp)
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?bots=2&power=220"
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
        end = time.time() + 90
        while time.time() < end and not any("ticks" in l and "local server running" in l
                                            for l in lines):
            ui.pump(0.3)
        # Another look than the default rider (CLAUDE.md: vary the riders).
        for line in ("weather.transition instant", "weather clear", "time 2026-06-21 18:30",
                     "character --hair-colour #B71C1C --age 48",
                     "set --temp traffic.ufo rare"):
            ui.console(line)
        ui.pump(3.0)
        ui.console("traffic road busy")
        ui.pump(0.5)
        ui.console("traffic")
        ui.pump(0.5)
        log_lines = [str(x) for x in (ui.console_state().get("log") or [])]
        ui.console("menu traffic")
        main_page = ui.wait_menu(lambda m: m.get("open") and any(
            i.startswith("UFOs") for i in m.get("items", [])), timeout=5.0)
        ui.shot("traffic")
        for _ in range(3):
            ui.console("menu down")
        ui.console("menu select")
        more = ui.wait_menu(lambda m: m.get("open") and any(
            i.startswith("Aircraft: live") for i in m.get("items", [])), timeout=5.0)
        ui.shot("traffic-more")
        ui.close_menu()
        items = main_page.get("items", [])
        items_more = more.get("items", [])
        starts = lambda xs, p: any(i.startswith(p) for i in xs)  # noqa: E731
        ui.check("the browser's Traffic page: master switch, aircraft, UFOs, more",
                 all(starts(items, p) for p in ("Traffic", "Aircraft", "UFOs", "Sound and live data")),
                 items)
        ui.check("no road traffic in the browser (Q9 (b))",
                 not starts(items, "Road traffic") and not starts(items, "Driver manners"), items)
        ui.check("UFOs rare from the setting", any(i.startswith("UFOs") and "rare" in i for i in items),
                 items)
        ui.check("Sound and the dimmed Aircraft: live; no live road data row",
                 starts(items_more, "Sound") and starts(items_more, "Aircraft: live")
                 and not starts(items_more, "Live traffic data"), items_more)
        ui.check("traffic road is refused in the browser, `traffic` says so",
                 any("desktop app only" in l for l in log_lines + lines), log_lines[-12:])
        summary.update({"traffic": items, "traffic_more": items_more, "console": log_lines[-12:],
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
