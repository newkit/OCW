#!/usr/bin/env python3
"""The browser's sign-in end to end (spec §9.1, §11.4): headless Chromium,
the web app (target/web, `make web`), `ocw-server --auth-dev` (the accounts
backend with in-process fake Google and Strava; nothing leaves 127.0.0.1).

    scripts/test-web-account.py [--out test-output/web] [--server target/debug/ocw-server]

Loads the page with `auth=<backend>`, signs in with Google through the
redirect (the page goes to the backend, the fake Google approves, the
backend's redirect brings the page back with the code; the page exchanges
it with fetch() over CORS), checks that the code left the address bar and
the ride's options came back, opens Settings → Account (screenshot
OUT/web-account-<time>-signed-in.jpg), links Strava (the link ticket over
CORS with the bearer token, a second redirect), signs out. Exits 0 when
every check passed; the checks and the console go to OUT/web-account-<time>.log.
"""

import argparse
import datetime
import importlib.util
import json
import os
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


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--server", default="target/debug/ocw-server")
    p.add_argument("--chromium", default=None)
    p.add_argument("--timeout", type=float, default=90.0)
    a = p.parse_args()
    sys.path.insert(0, HERE)
    tw = load("test_web", "test-web.py")
    import web_browser
    chromium = a.chromium or web_browser.find()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    base = os.path.join(a.out, "web-account-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    httpd = tw.web_serve().server(a.dir, stats={})
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    page = f"http://127.0.0.1:{httpd.server_address[1]}/"
    info = tempfile.mktemp(suffix=".json")
    srv_log = open(base + "-server.log", "w")
    server = subprocess.Popen([a.server, "--auth-dev", "127.0.0.1:0", "--auth-dev-web", page,
                               "--auth-info-file", info, "--bots", "0"],
                              stdout=srv_log, stderr=subprocess.STDOUT)
    for _ in range(300):
        if os.path.exists(info) and os.path.getsize(info) > 0:
            break
        time.sleep(0.1)
    issuer = json.load(open(info))["issuer"]
    query = f"?auth={issuer}&bots=2&name=Web"
    url = page + query
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    log = open(base + ".log", "w")
    proc, cdp = tw.start_browser(chromium, tw.chromium_flags(profile, 1280, 720), log)
    lines, checks = [], []

    def check(name, ok, detail=None):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})
        lines.append(f"check {'ok ' if ok else 'FAIL'} {name}: {detail}")

    seen = [0]

    def said(text, timeout):
        """Waits for a console line containing `text`."""
        end = time.time() + timeout
        while time.time() < end:
            cdp.pump(0.2)
            for e in cdp.events[seen[0]:]:
                if e["method"] == "Runtime.consoleAPICalled":
                    t = tw.console_text(e)
                    lines.append(t)
                    if text in t:
                        seen[0] = cdp.events.index(e) + 1
                        return t
            seen[0] = len(cdp.events)
        return None

    def menu():
        raw = cdp.eval("window.ocw && window.ocw.menu ? window.ocw.menu() : ''")
        return json.loads(raw) if raw else {}

    def console(line):
        cdp.eval(f"window.ocw && window.ocw.console({json.dumps(line)})")

    def account_page():
        for line in ["menu close", "menu open"] + ["menu down"] * 5 + ["menu select", "menu select"]:
            console(line)
            cdp.pump(0.25)
        end = time.time() + 5
        m = menu()
        while time.time() < end and m.get("title") != "Account":
            cdp.pump(0.2)
            m = menu()
        return m

    ok = False
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        target = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget", {"targetId": target["targetId"], "flatten": True},
                               session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            cdp.call(m)
        cdp.call("Emulation.setDeviceMetricsOverride",
                 {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False})
        cdp.call("Page.navigate", {"url": url})
        check("page runs", said("local server running", a.timeout) is not None)
        m = account_page()
        check("Account page signed out", "anonymously" in m.get("status", ""), m.get("status"))
        cdp.screenshot(base + "-signed-out.jpg")
        console("menu close")
        console("account signin google")
        t = said("signed in as", a.timeout)
        check("signed in through the redirect", t is not None and "Marianne Vos" in t, t)
        said("local server running", 30)
        search = cdp.eval("location.search")
        check("the code left the address bar, the options came back", search == query, search)
        m = account_page()
        check("Account page signed in", m.get("status") == "Signed in as Marianne Vos (Google)", m.get("status"))
        check("Link Strava offered", "Link Strava" in m.get("items", []), m.get("items"))
        cdp.screenshot(base + "-signed-in.jpg")
        console("menu close")
        console("account link strava")
        t = said("signed in as", a.timeout)
        said("local server running", 30)
        m = account_page()
        check("Strava linked", m.get("status") == "Signed in as Marianne Vos (Google, Strava)", m.get("status"))
        cdp.screenshot(base + "-linked.jpg")
        console("menu close")
        stored = cdp.eval("localStorage.getItem('ocw.account.session') || ''")
        check("localStorage keeps no access token", '"access_token":""' in stored, len(stored))
        console("account signout")
        cdp.pump(1.0)
        m = account_page()
        check("signed out", "anonymously" in m.get("status", ""), m.get("status"))
        check("localStorage cleared", not cdp.eval("localStorage.getItem('ocw.account.session')"), None)
        ok = all(c["ok"] for c in checks)
    except Exception as e:  # noqa: BLE001 – reported
        check("no exception", False, repr(e))
    finally:
        try:
            cdp.send("Browser.close", session=False)
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            proc.kill()
        server.kill()
        httpd.shutdown()
    log.write("\n".join(lines) + "\n" + json.dumps(checks, indent=2) + "\n")
    log.close()
    for c in checks:
        print(f"{'ok  ' if c['ok'] else 'FAIL'} {c['check']}: {c['detail']}")
    print(f"log: {base}.log  screenshots: {base}-signed-out.jpg {base}-signed-in.jpg {base}-linked.jpg")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
