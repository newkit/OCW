#!/usr/bin/env python3
"""Headless browser test of the trust dialog (spec §4.7 *User keys*, v3.63).

    scripts/test-web-trust.py [--dir target/web] [--out test-output/web]
                              [--package data/pilot/liechtenstein.ocwpkg]
                              [--route vaduz-malbun] [--chromium chromium]

Makes a throwaway user key "Ada Rider" in a temporary directory
(`world-builder keys init-user`; deleted afterwards, never committed),
signs a copy of the package with it as `user/ada/<name>` and publishes it
into DIR/worlds/. Serves DIR on a free local port and opens the page with
`?world=user/ada/<name>` in headless Chromium (WebGPU), then checks:

  dialog    the stream waits after the manifest and the trust dialog is
            open: its title, the items (Trust this key, Load once, Cancel),
            Cancel focused, the full fingerprint (the one `world-builder keys
            fingerprint` prints) drawn large, the claimed name, the warning
            -> OUT/web-trust-<time>-dialog.jpg
  cancel    Enter on the focused Cancel declines: the world is not loaded
            and the page says why                     -> -declined.jpg
  trust     after a reload the dialog asks again; Up, Up, Enter trusts the
            key: the world streams and the rider rides; the key is in
            localStorage (`ocw-trusted-keys`)          -> -trusted.jpg
  again     a reload loads the world without asking

Writes OUT/web-trust-<time>.log and .json; exits 0 when every check passed.
"""

import argparse
import datetime
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tw = load("test_web", "test-web.py")
ws = load("web_serve", "web-serve.py")
sys.path.insert(0, HERE)
import web_browser  # noqa: E402
import web_ui_test  # noqa: E402

TITLE = "Trust this world's key?"
WARNING = "This world was not made by Open Cycling World."


def world_builder():
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "world-builder"], check=True)
    wb = "target/release/world-builder"
    return wb if os.path.exists(wb) else wb + ".exe"


def publish_user_world(wb, package, worlds):
    """A copy of `package` signed by a throwaway user key, published into
    `worlds`; returns (id, fingerprint)."""
    tmp = tempfile.mkdtemp(prefix="ocw-trust-")
    try:
        keys = os.path.join(tmp, "keys")
        subprocess.run([wb, "keys", "init-user", "--name", "Ada Rider", "--dir", keys],
                       check=True, stdout=subprocess.DEVNULL)
        stem = os.path.splitext(os.path.basename(package))[0]
        staged = os.path.join(tmp, f"{stem}.ocwpkg")
        try:
            os.link(package, staged)
        except OSError:
            shutil.copy(package, staged)
        wid = f"user/ada/{stem}"
        subprocess.run([wb, "sign", staged, "--key", os.path.join(keys, "user.key"), "--id", wid],
                       check=True, stdout=subprocess.DEVNULL)
        fp = subprocess.run([wb, "keys", "fingerprint", staged], check=True,
                            capture_output=True, text=True).stdout
        fingerprint = re.search(r"fingerprint:\s*([0-9A-F ]+)", fp).group(1).strip()
        subprocess.run([wb, "publish", staged, "--out", worlds], check=True,
                       stdout=subprocess.DEVNULL)
        return wid, fingerprint
    finally:
        # The throwaway key never outlives the test.
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--package", default="data/pilot/liechtenstein.ocwpkg")
    p.add_argument("--route", default="vaduz-malbun")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    p.add_argument("--timeout", type=float, default=120.0)
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    missing = ws.missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    if not os.path.isfile(a.package):
        sys.exit(f"{a.package}: no such package")
    wid, fingerprint = publish_user_world(world_builder(), a.package, os.path.join(a.dir, "worlds"))
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"web-trust-{stamp}")
    httpd = ws.server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{httpd.server_address[1]}/?world={wid}&route={a.route}"
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(a.chromium, tw.chromium_flags(profile, w, h), log)
    lines, checks, shots = [], [], {}
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
        ui = web_ui_test.UiTest(cdp, base, lines, a.out)

        def check(name, ok, detail=""):
            checks.append({"name": name, "ok": bool(ok), "detail": str(detail)[:300]})
            print(("ok   " if ok else "FAIL ") + name + (f": {detail}" if not ok else ""))

        def said(pattern, since):
            return any(e["method"] == "Runtime.consoleAPICalled"
                       and re.search(pattern, tw.console_text(e))
                       for e in cdp.events[since:])

        def wait(pred, timeout):
            end = time.time() + timeout
            while time.time() < end:
                if pred():
                    return True
                ui.pump(0.2)
            return pred()

        def open_page(navigate=True):
            mark = len(cdp.events)
            if navigate:
                cdp.call("Page.navigate", {"url": url})
            else:
                cdp.call("Page.reload", {})
            return mark

        def shot(name):
            path = cdp.screenshot(f"{base}-{name}.jpg")
            shots[name] = path
            return path

        # 1. The dialog.
        open_page()
        wait(lambda: ui.menu().get("title") == TITLE, a.timeout)
        m = ui.menu()
        check("dialog: the trust dialog opens for the user-signed world", m.get("title") == TITLE, m)
        check("dialog: Trust this key, Load once, Cancel",
              m.get("items") == ["Trust this key", "Load once", "Cancel"], m.get("items"))
        check("dialog: Cancel is focused", m.get("focused") == 2, m.get("focused"))
        check("dialog: the full fingerprint, large",
              " ".join(m.get("emphasis") or []) == fingerprint, (m.get("emphasis"), fingerprint))
        text = " ".join(m.get("text") or [])
        check("dialog: the claimed name", "Signed by 'Ada Rider' (unverified name)" in (m.get("status") or ""),
              m.get("status"))
        check("dialog: the warning", WARNING in text, text)
        check("dialog: the data sources", "OpenStreetMap" in text, text)
        ui.pump(1.0)
        shot("dialog")
        # 2. Enter on Cancel declines.
        mark = len(cdp.events)
        ui.key("Enter")
        declined = wait(lambda: said(r"did not trust", mark), 15)
        check("cancel: Enter on the focused Cancel declines; not loaded", declined)
        check("cancel: no ride starts", not said(r"world ready", mark))
        ui.key("Escape")
        ui.pump(1.0)
        shot("declined")
        # 3. Asked again after a reload; trusted on purpose.
        mark = open_page(navigate=False)
        wait(lambda: ui.menu().get("title") == TITLE, a.timeout)
        check("trust: asked again after a reload", ui.menu().get("title") == TITLE, ui.menu())
        ui.key("ArrowUp", 2)
        check("trust: Trust this key focused after Up, Up", ui.menu().get("focused") == 0, ui.menu())
        ui.key("Enter")
        ready = wait(lambda: said(r"world ready", mark), a.timeout)
        check("trust: the world streams once the key is trusted", ready)
        riding = wait(lambda: said(r"local server running.*ticks", mark), 30)
        check("trust: the rider rides", riding)
        stored = cdp.eval("localStorage.getItem('ocw-trusted-keys') || ''") or ""
        check("trust: the key is kept in localStorage", fingerprint in stored, stored[:200])
        ui.pump(4.0)
        shot("trusted")
        # 4. Trusted: no question.
        mark = open_page(navigate=False)
        ready = wait(lambda: said(r"world ready", mark), a.timeout)
        asked = ui.menu().get("title") == TITLE
        check("again: a trusted key loads without asking", ready and not asked, ui.menu())
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        shutil.rmtree(profile, ignore_errors=True)
        httpd.shutdown()
    ok = bool(checks) and all(c["ok"] for c in checks)
    with open(base + ".json", "w") as f:
        json.dump({"url": url, "world": wid, "fingerprint": fingerprint, "ok": ok,
                   "checks": checks, "shots": shots}, f, indent=2)
    log.write("\n".join(lines) + "\n")
    log.close()
    print(f"{'OK' if ok else 'FAILED'}: {base}.json; screenshots: {', '.join(shots.values())}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
