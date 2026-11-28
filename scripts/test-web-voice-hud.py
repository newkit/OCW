#!/usr/bin/env python3
"""The voice status panel of the HUD in the browser (spec 7.1.13; 11.4).

    scripts/test-web-voice-hud.py [--out test-output/voice-hud] [--dir target/web]
                                  [--chromium chromium] [--software]

Headless Chromium (WebGPU) rides the Hills test route three times – a
desktop window (1280x720), an iPhone 15 in portrait (393x852 CSS px, touch)
and the same with the safe areas of the notch and the home indicator
(`?safe-insets=59,0,34,0`) – with a varied rider and bots. Without a voice server the panel is hidden; `dev.voice_hud STATE`
then shows it in made-up states (made-up names), and each is checked from
`window.ocw.hudLayout()` (the boxes Bevy laid out, CSS px) and screenshot:

- hidden without voice; one dim line "not in a call";
- connected with the microphone off (the X mouth, the key hint), on and
  speaking, others speaking (three names and "+2"), I hear nobody
  (headphones), failed, reconnecting;
- the panel lies directly under the top-right column's printed lines, its
  right edge on theirs, with one line (the time stamp), two (the FPS
  counter), three (the Strava badge) and a long notice (the Strava
  recording's stop): never over a line, never outside the window or
  (phone) the safe area;
- `hud.voice off | compact | full` (setting `display.voice_hud`): off hides
  it, compact folds to icons after a few quiet seconds, full keeps the rows;
- `hud off` hides it with the HUD;
- the page's own Devices panel (open at the right edge on a desktop
  browser until a device connects) collapses to its button when the voice
  panel first reaches it and, opened again, stands left of it.

  OUT/web-voice-hud-<time>-<desktop|phone|phone-safe>-<state>.jpg
  OUT/web-voice-hud-<time>.json / .log     the layouts, checks

Exits 0 when every check holds.
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

# What each made-up state must show (substrings of the panel's rows).
STATES = [
    ("idle", "idle", ["voice: not in a call"]),
    ("mic-off", "rows", ["connected (4)", "off (M) - everybody", "everybody"]),
    ("speaking", "rows", ["connected (4)", "on - selected 2", "selected 2 (1 here)"]),
    ("others", "rows", ["connected (6)", "on - everybody", "Bea Climber", "Cleo Sprinter",
                        "Dana Rouleur", "+2"]),
    ("nobody", "rows", ["connected (4)", "nobody"]),
    ("failed", "rows", ["failed: server unreachable, retrying"]),
    ("reconnecting", "rows", ["reconnecting"]),
]

# A varied rider per view (CLAUDE.md: vary the riders in pictures).
LOOKS = {
    "desktop": "character --body b --age 30s --hair-colour #6D4C41",
    "phone": "character --body a --age 50s --hair-colour #F9A825",
    "phone-safe": "character --body c --age 40s --hair-colour #212121",
}


def test_web():
    """scripts/test-web.py as a module (Cdp, the server, console lines)."""
    spec = importlib.util.spec_from_file_location("test_web", os.path.join(HERE, "test-web.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def overlap(a, b):
    """Whether two [left, top, width, height] boxes overlap by more than 1 px."""
    return (min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0]) > 1
            and min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1]) > 1)


def layout_problem(hud, size, insets):
    """What is wrong with the panel's place (None: nothing)."""
    panel, lines, column = hud.get("voice_panel"), hud.get("top_right_lines") or [], hud.get("top_right")
    if not panel:
        return "the panel is not shown"
    if hud.get("voice_clear"):
        return hud["voice_clear"]
    for l in lines:
        if overlap(panel, l):
            return f"the panel {panel} overlaps the line {l}"
        if panel[1] + 1 < l[1] + l[3]:
            return f"the panel {panel} is not below the line {l}"
        if abs((l[0] + l[2]) - (panel[0] + panel[2])) > 1.5:
            return f"the panel's right edge differs from the line's: {panel} / {l}"
    if lines:
        gap = panel[1] - max(l[1] + l[3] for l in lines)
        if gap > 12:
            return f"the panel is {gap:.0f} px below the last line (not directly under it)"
    w, h = size
    top, right, bottom, left = insets
    if (panel[0] < left - 1 or panel[1] < top - 1 or panel[0] + panel[2] > w - right + 1
            or panel[1] + panel[3] > h - bottom + 1):
        return f"the panel {panel} leaves the window's safe area"
    if w - right - (panel[0] + panel[2]) > 24:
        return f"the panel {panel} is not at the right edge"
    if column and not (column[1] - 1 <= panel[1] and panel[1] + panel[3] <= column[1] + column[3] + 1):
        return f"the panel {panel} is outside the column {column}"
    for name in ("top_panel", "profile"):
        other = hud.get(name)
        if other and overlap(panel, other):
            return f"the panel {panel} overlaps {name} {other}"
    return None


def run_view(tw, web_ui_test, a, base, httpd, label, size, phone, insets, summary, log):
    w, h = size
    opts = ["bots=3", "power=230"]
    if phone:
        opts += ["touch=on"]
    if any(insets):
        opts += ["safe-insets=" + ",".join(str(i) for i in insets)]
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?" + "&".join(opts)
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = tw.chromium_flags(profile, w, h, "webgpu", a.software)
    proc, cdp = tw.start_browser(a.chromium, flags, log)
    lines = []
    out = {"url": url, "states": {}}
    summary[label] = out
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget",
                               {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": w, "height": h, "deviceScaleFactor": 1, "mobile": phone})
        if phone:
            cdp.call("Emulation.setTouchEmulationEnabled", {"enabled": True, "maxTouchPoints": 5})
        cdp.call("Page.navigate", {"url": url})
        ui = web_ui_test.UiTest(cdp, f"{base}-{label}", lines, a.out)
        end = time.time() + 120
        while time.time() < end and not any("ticks" in l and "local server running" in l
                                            for l in lines):
            ui.pump(0.3)

        def hud():
            try:
                return json.loads(cdp.eval("window.ocw.hudLayout ? window.ocw.hudLayout() : '{}'") or "{}")
            except Exception:  # noqa: BLE001
                return {}

        def wait_hud(pred, timeout=6.0):
            end = time.time() + timeout
            got = hud()
            while time.time() < end and not pred(got):
                ui.pump(0.2)
                got = hud()
            return got

        def check(name, ok, detail=None):
            ui.check(f"{label}: {name}", ok, detail)

        def devices_box():
            """The page's open Devices panel (CSS px), None while collapsed."""
            try:
                return json.loads(cdp.eval(
                    "(() => { const e = document.getElementById('ocw-devices');"
                    " if (!e || e.hidden || getComputedStyle(e).visibility === 'hidden') return 'null';"
                    " const r = e.getBoundingClientRect();"
                    " return JSON.stringify([r.left, r.top, r.width, r.height]); })()") or "null")
            except Exception:  # noqa: BLE001
                return None

        for line in ("weather.transition instant", "weather clear", "time 2026-06-21 17:10",
                     LOOKS[label], "hud.voice full"):
            ui.console(line)
        ui.pump(3.0)
        # A solo ride on the page's own server: no voice, no panel.
        got = wait_hud(lambda x: "voice_shape" in x)
        check("hidden without voice", got.get("voice_shape") == "hidden" and not got.get("voice_panel"),
              {k: got.get(k) for k in ("voice_shape", "voice_panel")})
        ui.shot("no-voice")
        for state, shape, rows in STATES:
            ui.console(f"dev.voice_hud {state}")
            got = wait_hud(lambda x: x.get("voice_shape") == shape and x.get("voice_panel")
                           and all(any(r in row for row in x.get("voice_rows", [])) for r in rows))
            shown = got.get("voice_rows", [])
            check(f"{state}: the rows", got.get("voice_shape") == shape
                  and all(any(r in row for row in shown) for r in rows), shown)
            problem = layout_problem(got, (w, h), insets)
            check(f"{state}: under the top-right lines, right-aligned, inside the window",
                  problem is None, problem or got.get("voice_panel"))
            ui.pump(1.2)
            dev = devices_box()
            check(f"{state}: the page's Devices panel keeps clear of it",
                  not dev or not got.get("voice_panel") or not overlap(dev, hud().get("voice_panel") or [0, 0, 0, 0]),
                  {"devices": dev, "panel": got.get("voice_panel")})
            out["states"][state] = {"rows": shown, "panel": got.get("voice_panel"),
                                    "lines": got.get("top_right_lines")}
            ui.shot(state)
        # The page's Devices panel: collapsed to its button when it first
        # met the voice panel; opened again it steps left of it.
        if not phone:
            ui.console("dev.voice_hud others")
            ui.pump(1.2)
            check("the page's Devices panel collapsed when the voice panel reached it",
                  devices_box() is None, devices_box())
            cdp.eval("document.getElementById('ocw-devices-show').click()")
            ui.pump(1.5)
            dev, got = devices_box(), hud()
            check("opened again, the Devices panel stands left of the voice panel",
                  bool(dev) and bool(got.get("voice_panel")) and not overlap(dev, got["voice_panel"])
                  and dev[0] + dev[2] <= got["voice_panel"][0] + 1,
                  {"devices": dev, "panel": got.get("voice_panel")})
            ui.shot("devices-open")
            cdp.eval("document.getElementById('ocw-devices-hide').click()")
            ui.pump(0.6)
        # The lines above grow: the FPS counter, the Strava badge, its
        # notice. The panel moves down, never over them.
        ui.console("dev.voice_hud others")
        one = wait_hud(lambda x: x.get("voice_panel"))
        n1 = len(one.get("top_right_lines") or [])
        ui.console("fps on")
        two = wait_hud(lambda x: len(x.get("top_right_lines") or []) > n1)
        n2 = len(two.get("top_right_lines") or [])
        problem = layout_problem(two, (w, h), insets)
        check("with the FPS counter: one line more, the panel below it",
              n2 == n1 + 1 and problem is None and two["voice_panel"][1] > one["voice_panel"][1],
              problem or {"lines": two.get("top_right_lines"), "panel": two.get("voice_panel")})
        ui.shot("lines-fps")
        # The Strava badge: a third line.
        ui.console("strava.record start")
        many = wait_hud(lambda x: len(x.get("top_right_lines") or []) > n2, timeout=8.0)
        n3 = len(many.get("top_right_lines") or [])
        problem = layout_problem(many, (w, h), insets)
        check("with the Strava badge too: the panel below every line",
              n3 > n2 and problem is None and many["voice_panel"][1] > two["voice_panel"][1],
              problem or {"lines": many.get("top_right_lines"), "panel": many.get("voice_panel")})
        ui.shot("lines-several")
        # Its notice at the stop (a long text in a box that wraps).
        ui.console("strava.record stop")
        ui.pump(0.8)
        note = hud()
        tall = max((l[3] for l in note.get("top_right_lines") or []), default=0)
        problem = layout_problem(note, (w, h), insets)
        check("with a long notice: the panel below it",
              len(note.get("top_right_lines") or []) > n2 and problem is None,
              problem or {"lines": note.get("top_right_lines"), "panel": note.get("voice_panel")})
        ui.shot("lines-notice")
        out["lines"] = {"one": one.get("top_right_lines"), "two": two.get("top_right_lines"),
                        "several": many.get("top_right_lines"), "notice": note.get("top_right_lines"),
                        "tallest_notice_line": tall,
                        "panels": [one.get("voice_panel"), two.get("voice_panel"),
                                   many.get("voice_panel"), note.get("voice_panel")]}
        ui.console("fps off")
        # The setting: off hides, compact folds to icons, full keeps rows.
        ui.console("dev.voice_hud mic-off")
        ui.console("set display.voice_hud off")
        got = wait_hud(lambda x: x.get("voice_shape") == "hidden")
        check("display.voice_hud off hides the panel",
              got.get("voice_shape") == "hidden" and not got.get("voice_panel"), got.get("voice_shape"))
        ui.console("set display.voice_hud compact")
        got = wait_hud(lambda x: x.get("voice_shape") == "rows")
        check("compact: the rows after a change", got.get("voice_shape") == "rows", got.get("voice_shape"))
        got = wait_hud(lambda x: x.get("voice_shape") == "icons", timeout=12.0)
        problem = layout_problem(got, (w, h), insets)
        check("compact: icons after a few quiet seconds (the riders in the call)",
              got.get("voice_shape") == "icons" and got.get("voice_rows") == ["4"] and problem is None,
              problem or got.get("voice_rows"))
        ui.shot("compact-icons")
        ui.console("dev.voice_hud others")
        got = wait_hud(lambda x: x.get("voice_shape") == "rows")
        check("compact: speech opens the rows again", got.get("voice_shape") == "rows",
              got.get("voice_shape"))
        setting = ui.setting("display.voice_hud")
        check("the setting is kept", "compact" in json.dumps(setting), setting)
        ui.console("set display.voice_hud full")
        ui.console("hud off")
        got = wait_hud(lambda x: not x.get("voice_panel"))
        check("hud off hides the panel with the HUD", not got.get("voice_panel"), got.get("voice_panel"))
        ui.console("hud on")
        ui.console("dev.voice_hud none")
        got = wait_hud(lambda x: x.get("voice_shape") == "hidden")
        check("dev.voice_hud none: the call's own again (hidden)", got.get("voice_shape") == "hidden",
              got.get("voice_shape"))
        out["checks"] = ui.checks
        out["shots"] = ui.shots
    except Exception as e:  # noqa: BLE001 – reported in the summary
        out["error"] = repr(e)
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        shutil.rmtree(profile, ignore_errors=True)
    log.write(f"\n--- console ({label}) ---\n" + "\n".join(lines) + "\n")
    return "error" not in out and bool(out.get("checks")) and all(c["ok"] for c in out["checks"])


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/voice-hud")
    p.add_argument("--chromium", default=None)
    p.add_argument("--software", action="store_true",
                   help="WebGPU on SwiftShader (a machine whose GPU gives broken headless captures)")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    tw = test_web()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    import web_ui_test

    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-voice-hud-" + stamp)
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    log = open(base + ".log", "w")
    summary = {"browser": web_browser.describe(a.chromium)}
    ok = True
    try:
        for label, size, phone, insets in (("desktop", (1280, 720), False, (0, 0, 0, 0)),
                                           ("phone", (393, 852), True, (0, 0, 0, 0)),
                                           ("phone-safe", (393, 852), True, (59, 0, 34, 0))):
            ok = run_view(tw, web_ui_test, a, base, httpd, label, size, phone, insets, summary, log) and ok
    finally:
        httpd.shutdown()
    summary["ok"] = ok
    log.write("\n--- summary ---\n" + json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2, ensure_ascii=False)
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
