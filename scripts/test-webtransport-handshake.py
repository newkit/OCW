#!/usr/bin/env python3
"""WebTransport handshakes from headless Chromium to a hosted ocw-server
(spec 11.5, the Fly.io server's browser path):

    scripts/test-webtransport-handshake.py --url https://IPV4:47801 --hash HEX
        [--attempts 4] [--parallel 1] [--pause 20] [--timeout 15] [--net-log FILE]
        [--chromium-flag=--disable-features=PostQuantumKyber ...]

Serves an empty page on localhost (a secure context; --page: any page, e.g.
the public preview), opens Chromium once
and makes ATTEMPTS handshakes one after the other (each a new connection,
PAUSE s apart: many new UDP flows in a row made Fly's path go quiet), then
prints one line per attempt (ok / the error, milliseconds) and a JSON
summary. --net-log writes Chromium's NetLog with every QUIC packet sent and
received, for comparing with a capture on the server. Exits 0 when every
attempt succeeded.
"""

import argparse
import http.server
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
sys.path.insert(0, HERE)
import web_browser  # noqa: E402


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


PAGE = b"<!doctype html><meta charset=utf-8><title>wt</title><body>wt</body>"

# One handshake: resolves to {ok, ms, error}.
ATTEMPT = """
(async () => {
  const hex = %(hash)s;
  const value = new Uint8Array(hex.match(/../g).map(h => parseInt(h, 16)));
  const t0 = performance.now();
  let wt;
  try {
    wt = new WebTransport(%(url)s, {serverCertificateHashes: [{algorithm: "sha-256", value}]});
    const timeout = new Promise((_, no) => setTimeout(() => no(new Error("test timeout")), %(timeout)d));
    await Promise.race([wt.ready, timeout]);
    const ms = performance.now() - t0;
    wt.close();
    return {ok: true, ms};
  } catch (e) {
    try { wt && wt.close(); } catch (_) {}
    return {ok: false, ms: performance.now() - t0, error: String(e && (e.message || e))};
  }
})()
"""


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--url", required=True)
    p.add_argument("--hash", required=True, help="the certificate's SHA-256 (hex)")
    p.add_argument("--attempts", type=int, default=4)
    p.add_argument("--pause", type=float, default=20.0)
    p.add_argument("--timeout", type=float, default=15.0)
    p.add_argument("--parallel", type=int, default=1,
                   help="handshakes started at the same moment per attempt (each its own UDP flow)")
    p.add_argument("--page", default=None,
                   help="make the handshakes from this page instead of an empty localhost one, e.g. the"
                        " public preview https://newkit.github.io/OCW/ (its origin, its CSP)")
    p.add_argument("--net-log", default=None)
    p.add_argument("--chromium", default=None)
    p.add_argument("--chromium-flag", action="append", default=[])
    a = p.parse_args()
    exe = a.chromium or web_browser.find()
    tw = module("test_web", "test-web.py")

    class Quiet(http.server.BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(PAGE)

        def log_message(self, *args):
            pass

    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Quiet)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    profile = tempfile.mkdtemp(prefix="ocw-wt-")
    flags = ["--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
             "--no-first-run", "--no-default-browser-check", "--disable-extensions",
             "--no-proxy-server", "--password-store=basic"] + a.chromium_flag
    if a.net_log:
        flags += [f"--log-net-log={os.path.abspath(a.net_log)}", "--net-log-capture-mode=Everything"]
    log = open(os.devnull, "w")
    proc, cdp = tw.start_browser(exe, flags, log)
    results = []
    try:
        targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        cdp.session = cdp.call("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True},
                               session=False)["sessionId"]
        cdp.call("Page.enable")
        cdp.call("Page.navigate", {"url": a.page or f"http://127.0.0.1:{httpd.server_address[1]}/"})
        time.sleep(5.0 if a.page else 1.5)
        print(f"page {cdp.eval('location.origin')}, secure context {cdp.eval('window.isSecureContext')}",
              flush=True)
        one = ATTEMPT.strip() % {"hash": json.dumps(a.hash), "url": json.dumps(a.url), "timeout": int(a.timeout * 1000)}
        js = f"Promise.all([{', '.join([one] * a.parallel)}])"
        for i in range(a.attempts):
            if i:
                time.sleep(a.pause)
            want = cdp.send("Runtime.evaluate", {"expression": js, "awaitPromise": True, "returnByValue": True})
            res = None
            deadline = time.time() + a.timeout + 10
            while res is None and time.time() < deadline:
                for m in cdp.pump(0.2):
                    if m.get("id") == want:
                        res = m.get("result", {}).get("result", {}).get("value")
            at = time.strftime("%H:%M:%S")
            for j, one_res in enumerate(res or [{"ok": False, "error": "no answer from the page"}] * a.parallel):
                one_res["at"] = at
                results.append(one_res)
                print(f"attempt {i + 1}.{j + 1}: {'ok' if one_res['ok'] else 'FAIL'} "
                      f"{one_res.get('ms', 0):.0f} ms {one_res.get('error', '')} ({at})", flush=True)
    finally:
        try:
            cdp.call("Browser.close", session=False, timeout=5)
        except Exception:  # noqa: BLE001
            pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
    ok = sum(1 for r in results if r["ok"])
    print(json.dumps({"ok": ok, "attempts": len(results), "flags": a.chromium_flag, "results": results}))
    sys.exit(0 if ok == len(results) else 1)


if __name__ == "__main__":
    main()
