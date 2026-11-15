#!/usr/bin/env python3
"""Headless browser test of the browser app's public preview (spec §14 Phase 10 B8a).

    scripts/test-web-preview.py --url https://newkit.github.io/OCW/   the live site
    scripts/test-web-preview.py --site target/web-preview-site        the assembled site
        [--out test-output/web-preview] [--chromium chromium] [--seconds 10]

With --site the site is served on a free local port under a host name that
is not localhost (`ocw-preview.test`, mapped to 127.0.0.1 and treated as a
secure origin by Chromium's flags), so the page behaves as on the public
host: no dev trust options. Checks, each with its own screenshot:

  start      the start page: config.json read, the rides listed, nothing big
             fetched before a ride is chosen
  hills      the Hills test route on WebGPU: the module loads, the local
             server ticks, the rider rides
  world      Liechtenstein streamed from the site, signed by the web
             preview's root (the log's trust line), ridden
  untrusted  the page's dev trust options (trust=dev, allow-unsigned=1,
             trust-root=…) are ignored on the public page
  webgl2     Chromium without WebGPU: the page falls back to the WebGL 2
             build and rides
  credits    credits.html loads (attribution, ODbL, privacy)

Every response with status >= 400 is a failure (no 404s). Writes OUT/
preview-<time>-<check>.jpg, preview-<time>.log and preview-<time>.json;
exits 0 when every check passed.
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
import types

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tw = load("test_web", "test-web.py")
ws = load("web_serve", "web-serve.py")

HOST = "ocw-preview.test"


class Browser:
    """Headless Chromium on the DevTools pipe (as scripts/test-web.py)."""

    def __init__(self, a, gpu, origin=None):
        w, h = a.size.split("x")
        self.size = (int(w), int(h))
        self.profile = tempfile.mkdtemp(prefix="ocw-chromium-")
        flags = tw.chromium_flags(self.profile, w, h, gpu)
        if origin:
            # The local site under a public-looking name, as a secure origin.
            flags += [f"--host-resolver-rules=MAP {HOST} 127.0.0.1",
                      f"--unsafely-treat-insecure-origin-as-secure={origin}"]
        self.log = open(os.path.join(self.profile, "chromium.log"), "w")
        self.proc, self.cdp = tw.start_browser(a.chromium, flags, self.log)
        targets = self.cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        self.cdp.session = self.cdp.call("Target.attachToTarget",
                                         {"targetId": page["targetId"], "flatten": True},
                                         session=False)["sessionId"]
        for m in ("Runtime.enable", "Log.enable", "Page.enable", "Network.enable"):
            self.cdp.call(m)
        self.cdp.call("Emulation.setDeviceMetricsOverride",
                      {"width": self.size[0], "height": self.size[1], "deviceScaleFactor": 1,
                       "mobile": False})

    def close(self):
        try:
            self.cdp.send("Browser.close", session=False)
            self.proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            self.proc.kill()
        shutil.rmtree(self.profile, ignore_errors=True)


def failed_responses(cdp, since):
    out = []
    for e in cdp.events[since:]:
        if e["method"] == "Network.responseReceived":
            r = e["params"]["response"]
            if r.get("status", 200) >= 400:
                out.append(f"{r['status']} {r['url']}")
        elif e["method"] == "Network.loadingFailed" and not e["params"].get("canceled"):
            out.append(f"failed {e['params'].get('errorText')} ({e['params'].get('type')})")
    return out


def responses(cdp, since, suffix):
    """(url, status, headers) of the responses whose URL path ends in `suffix`."""
    out = []
    for e in cdp.events[since:]:
        if e["method"] == "Network.responseReceived":
            r = e["params"]["response"]
            if r["url"].split("?")[0].endswith(suffix):
                out.append((r["url"], r["status"], r.get("headers", {})))
    return out


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--url", help="the live site, e.g. https://newkit.github.io/OCW/")
    g.add_argument("--site", help="an assembled site directory, served locally")
    p.add_argument("--out", default="test-output/web-preview")
    p.add_argument("--chromium", default=None,
                   help="the browser (default: the pinned Chromium of `make deps-browser`; scripts/web_browser.py)")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--seconds", type=float, default=10.0)
    p.add_argument("--timeout", type=float, default=180.0)
    p.add_argument("--world", default="liechtenstein")
    p.add_argument("--route", default="vaduz-malbun")
    p.add_argument("--only", default="", help="comma-separated checks (default: all)")
    a = p.parse_args()
    a.chromium = a.chromium or tw.web_browser.find()
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"preview-{stamp}")
    httpd = origin = None
    if a.site:
        httpd = ws.server(a.site, stats={})
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        origin = f"http://{HOST}:{httpd.server_address[1]}"
        url = origin + "/"
    else:
        url = a.url if a.url.endswith("/") else a.url + "/"
    only = set(filter(None, a.only.split(",")))
    want = lambda c: not only or c in only  # noqa: E731
    lines, checks = [], []
    summary = {"url": url, "local_site": a.site, "checks": checks,
               "browser": tw.web_browser.describe(a.chromium)}

    def check(name, ok, detail, **extra):
        checks.append({"check": name, "ok": bool(ok), "detail": detail, **extra})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}", flush=True)

    ride_args = lambda label: types.SimpleNamespace(  # noqa: E731
        timeout=a.timeout, seconds=a.seconds, teleport=0.0, teleport_wait=0.0, time="noon")

    def ride(b, name, query, world):
        mark = len(b.cdp.events)
        r = tw.visit(b.cdp, url + "?" + query, ride_args(name), f"{base}-{name}", lines, name, world, {})
        shot = f"{base}-{name}.jpg"
        b.cdp.screenshot(shot)
        bad = failed_responses(b.cdp, mark)
        said = [tw.console_text(e) for e in b.cdp.events[mark:] if e["method"] == "Runtime.consoleAPICalled"]
        return r, shot, bad, said

    try:
        b = Browser(a, "webgpu", origin)
        try:
            if want("start"):
                mark = len(b.cdp.events)
                b.cdp.call("Page.navigate", {"url": url})
                end = time.time() + 30
                n = 0
                while time.time() < end and not n:
                    b.cdp.pump(0.3)
                    n = b.cdp.eval("document.querySelectorAll('#ocw-start a.ride').length") or 0
                b.cdp.pump(1.0)
                shot = b.cdp.screenshot(f"{base}-start.jpg")
                wasm = responses(b.cdp, mark, ".wasm")
                cfg = responses(b.cdp, mark, "config.json")
                bad = failed_responses(b.cdp, mark)
                rides = b.cdp.eval("[...document.querySelectorAll('#ocw-start a.ride b')].map(e => e.textContent)")
                check("start", n >= 3 and cfg and not wasm and not bad,
                      f"{n} rides ({rides}), config.json {cfg[0][1] if cfg else 'missing'}, "
                      f"no module fetched: {not wasm}, failed responses: {bad}", screenshot=shot)
            if want("hills"):
                r, shot, bad, said = ride(b, "hills", "route=hills", False)
                wasm = responses(b.cdp, 0, "ocw_web_bg.wasm")
                enc = wasm[-1][2].get("content-encoding", wasm[-1][2].get("Content-Encoding", "none")) if wasm else "?"
                preview = any("page: the public preview" in t for t in said)
                check("hills", r["ok"] and not bad and preview,
                      f"ticks {r['ticks']}, distance {r['last'].get('distance_m')} m, the preview's page line: "
                      f"{preview}, wasm content-encoding: {enc}, failed responses: {bad}, errors: {r['errors'][:3]}",
                      screenshot=shot, wasm_encoding=enc)
            if want("world"):
                r, shot, bad, said = ride(b, "world", f"world={a.world}&route={a.route}", True)
                trust = [t for t in said if "trust:" in t]
                ok_trust = any("web preview root" in t and "NOT TRUSTED" not in t for t in trust)
                check("world", r["ok"] and ok_trust and not bad,
                      f"ready {r.get('ready_s')} s, {r.get('network_mb')} MB from the network, "
                      f"trust: {trust[-1] if trust else 'no line'}; failed responses: {bad}; "
                      f"errors: {r['errors'][:3]}", screenshot=shot)
            if want("untrusted"):
                q = f"world={a.world}&route={a.route}&trust=dev&allow-unsigned=1&trust-root={'ab' * 32}"
                mark = len(b.cdp.events)
                b.cdp.call("Page.navigate", {"url": url + "?" + q})
                end = time.time() + 60
                ignored = []
                while time.time() < end and len(ignored) < 3:
                    b.cdp.pump(0.3)
                    said = [tw.console_text(e) for e in b.cdp.events[mark:]
                            if e["method"] == "Runtime.consoleAPICalled"]
                    ignored = [t for t in said if "page options:" in t and "ignored" in t]
                shot = b.cdp.screenshot(f"{base}-untrusted.jpg")
                check("untrusted", len(ignored) >= 3,
                      f"the dev trust options ignored on the public page: {ignored}", screenshot=shot)
        finally:
            b.close()
        if want("webgl2"):
            b = Browser(a, "webgl2", origin)
            try:
                r, shot, bad, said = ride(b, "webgl2", "route=hills", False)
                api = b.cdp.eval("JSON.stringify(window.ocwGpu || null)")
                ok = r["ok"] and not bad and api and json.loads(api).get("api") == "webgl2"
                check("webgl2", ok, f"GPU {api}, ticks {r['ticks']}, failed responses: {bad}, "
                      f"errors: {r['errors'][:3]}", screenshot=shot)
            finally:
                b.close()
        if want("credits"):
            b = Browser(a, "webgpu", origin)
            try:
                mark = len(b.cdp.events)
                b.cdp.call("Page.navigate", {"url": url + "credits.html"})
                end = time.time() + 15
                text = ""
                while time.time() < end and "Privacy" not in (text or ""):
                    b.cdp.pump(0.3)
                    text = b.cdp.eval("document.body ? document.body.innerText : ''")
                shot = b.cdp.screenshot(f"{base}-credits.jpg")
                bad = failed_responses(b.cdp, mark)
                need = ["OpenStreetMap contributors", "Open Database License", "swissALTI3D",
                        "Copernicus", "Privacy", "localStorage"]
                missing = [n for n in need if n not in (text or "")]
                check("credits", not missing and not bad,
                      f"missing: {missing}, failed responses: {bad}", screenshot=shot)
            finally:
                b.close()
    except Exception as e:  # noqa: BLE001 – reported in the summary
        summary["error"] = repr(e)
        print(f"error: {e!r}")
    finally:
        if httpd:
            httpd.shutdown()
    summary["ok"] = bool(checks) and all(c["ok"] for c in checks) and "error" not in summary
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    print(f"log: {base}.log  summary: {base}.json")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
