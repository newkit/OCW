#!/usr/bin/env python3
"""A browser rider on a hosted ocw-server (spec 11.5; part of
scripts/test-remote-server.sh):

    scripts/test-remote-web.py --query 'server=https://IPV4:47801&cert-hash=…'
        [--world liechtenstein] [--seconds 55] [--name Dora] [--out DIR] [--https]

Serves target/web/ (`make web`, the world from target/web/worlds/, `make
web-worlds` of the same package the server rides) on localhost and opens
headless Chromium (WebGPU, a fake microphone playing a voice-like tone) with
the server's page options. The page joins the game over WebTransport (UDP
47801), streams the world, then joins the server's voice room with the real
livekit-client (ws://IPV4:7880, media UDP 7882 / TCP 7881), turns its voice
on for everybody, rides, and must see other participants in the room.
With --https the page is served over https:// (a throwaway self-signed
certificate, Chromium told to accept it), as the public preview is: then
mixed content rules apply, so the voice grant must be wss:// (spec 11.5,
--voice-public-url). Screenshots OUT/remote-web-<time>-*.jpg, the log and summary next to them.
Exits 0 when it joined, rode, connected to voice and saw others there.

    scripts/test-remote-web.py --together --dir target/web-preview-site --https [--name Ada]

rides together as the public preview does (spec 11.5 *Join info*): the
site's own config.json names the server's join info (web-preview-site.py
PUBLIC_SERVER), the page opens with ?together=1 only and fetches the
server's WebTransport URL, certificate hashes and world itself. A preview
site trusts only the preview's signing root (no trust-root=).

    scripts/test-remote-web.py --together --url https://newkit.github.io/OCW/ [--name Ada]

rides together from the live public page itself (nothing served locally):
the quick check after a server deploy (spec 11.5 *Status page*).
"""

import argparse
import datetime
import importlib.util
import json
import os
import ssl
import subprocess
import sys
import tempfile
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


wa = module("test_web_audio", "test-web-audio.py")


def spki(cert):
    """The certificate's SPKI hash as Chromium's
    --ignore-certificate-errors-spki-list takes it (base64 SHA-256)."""
    pub = subprocess.run(["openssl", "x509", "-in", cert, "-pubkey", "-noout"],
                         check=True, capture_output=True).stdout
    der = subprocess.run(["openssl", "pkey", "-pubin", "-outform", "der"], input=pub,
                         check=True, capture_output=True).stdout
    dig = subprocess.run(["openssl", "dgst", "-sha256", "-binary"], input=der,
                         check=True, capture_output=True).stdout
    import base64
    return base64.b64encode(dig).decode()


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--query", default=None, help="the server's page options (server=…&cert-hash=…)")
    p.add_argument("--together", action="store_true",
                   help="?together=1: the page fetches the join info its config.json names")
    p.add_argument("--world", default="liechtenstein")
    p.add_argument("--dir", default="target/web")
    p.add_argument("--url", default=None,
                   help="with --together: the live page (e.g. https://newkit.github.io/OCW/) instead of --dir")
    p.add_argument("--out", default="test-output/remote-server")
    p.add_argument("--seconds", type=float, default=55.0)
    p.add_argument("--name", default="Dora Descender")
    p.add_argument("--chromium", default=None)
    p.add_argument("--size", default="1280x720")
    p.add_argument("--software", action="store_true")
    p.add_argument("--https", action="store_true", help="serve the page over https:// (self-signed)")
    p.add_argument("--cmd", action="append", default=[],
                   help="a console command after joining (repeatable), e.g. the rider's look")
    p.add_argument("--hear", default=None,
                   help="the rider to hear: selected (I hear: selected riders), so the voice is"
                        " heard at any distance (riders who join seconds apart ride 40 m apart,"
                        " past the 10.5 m of everybody nearby)")
    p.add_argument("--chromium-flag", action="append", default=[],
                   help="one more Chromium flag (repeatable), e.g. --log-net-log=FILE")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    if a.url and not a.together:
        sys.exit("--url needs --together")
    if not a.url and not os.path.isdir(os.path.join(a.dir, "worlds")):
        sys.exit(f"{a.dir}/worlds missing: make web-worlds")
    if not a.query and not a.together:
        sys.exit("--query or --together")
    if a.together and not a.url:
        cfg = json.load(open(os.path.join(a.dir, "config.json")))
        if not str(cfg.get("server", "")).endswith(".json"):
            sys.exit(f"{a.dir}/config.json names no join info (server): build the site with web-preview-site.py")
    # A preview site trusts only the preview's root; a dev build the test key.
    root_file = "target/web-keys/root.hex"
    root = open(root_file).read().strip() if os.path.isfile(root_file) and not a.together else ""
    os.makedirs(a.out, exist_ok=True)
    base = os.path.join(a.out, "remote-web-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    httpd = None if a.url else wa.tw.web_serve().server(a.dir, stats={})
    scheme = "https" if a.url else "http"
    extra = ()
    if a.https and httpd:
        keys = tempfile.mkdtemp(prefix="ocw-https-")
        cert, key = os.path.join(keys, "cert.pem"), os.path.join(keys, "key.pem")
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                        "-nodes", "-days", "2", "-subj", "/CN=127.0.0.1", "-addext",
                        "subjectAltName=IP:127.0.0.1", "-keyout", key, "-out", cert],
                       check=True, capture_output=True)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
        scheme = "https"
        # Only the page's own throwaway certificate; the voice server's is
        # checked as usual (Fly's), and WebTransport goes by its hash.
        extra = (f"--ignore-certificate-errors-spki-list={spki(cert)}",)
    if httpd:
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
    port = httpd.server_address[1] if httpd else 0
    r = wa.Run(a, base, flags_extra=(*extra, *a.chromium_flag))
    ok = False
    try:
        name = a.name.replace(" ", "%20")
        if a.url:
            url = f"{a.url.rstrip('/')}/?together=1&name={name}&time=noon"
        elif a.together:
            url = f"{scheme}://127.0.0.1:{port}/?together=1&name={name}&time=noon"
        else:
            url = (f"{scheme}://127.0.0.1:{port}/?{a.query}&world={a.world}&trust-root={root}"
                   f"&name={name}&time=noon")
        r.summary["url"] = url.split("?", 1)[1]
        r.cdp.call("Page.navigate", {"url": url})
        rid = r.wait("joined the game server",
                     lambda: (r.json("window.ocw && window.ocw.netStats()") or {}).get("rider_id"), 180)
        r.check("joined the hosted game server over WebTransport", rid, rid)
        if a.together:
            # What the page made of the join info (the address it rides with).
            got = r.js("location.search") or ""
            r.summary["joined_query"] = got
            r.check("the page fetched the join info (server= and cert-hash= in the address)",
                    "server=" in got and "cert-hash=" in got, got)
        for c in a.cmd:
            r.console(c)
        r.pump(5.0)
        r.shot("joined")
        # A click: the autoplay gate opens and the voice call starts.
        r.click()
        call = r.wait("in the call", lambda: (r.voice() or {}).get("status") == "connected", 60)
        r.check("connected to the hosted voice room (real livekit-client)", call, (r.voice() or {}).get("status"))
        r.summary["page"] = scheme
        r.summary["voice_url"] = r.json("window.ocwVoice && window.ocwVoice.state && window.ocwVoice.state()")
        r.console("set voice.my_voice everybody")
        r.console("voice.mute-mic off")
        seen = r.wait("other participants",
                      lambda: [x.get("name") for x in (r.voice() or {}).get("participants", [])], 40)
        r.check("sees other participants in the voice room", seen, seen)
        if a.hear:
            # Selected once the rider is in the room (the selection goes by
            # the room's riders or remembered ones).
            r.wait(a.hear, lambda: a.hear in [x.get("name") for x in (r.voice() or {}).get("participants", [])], 40)
            r.console(f'voice.select "{a.hear}" on')
            r.console("set voice.i_hear selected")
        r.pump(max(0.0, a.seconds / 2))
        r.shot("riding")
        talk = r.wait("someone speaking", lambda: (r.voice() or {}).get("speaking"), 20)
        r.check("hears a voice from the room (active speaker)", talk, talk)
        r.pump(max(0.0, a.seconds / 2))
        stats = r.json("window.ocw && window.ocw.netStats()") or {}
        r.summary["net"] = stats
        r.summary["voice"] = r.voice()
        r.shot("end")
        ok = all(r.checks.values())
    except Exception as e:  # noqa: BLE001 – reported
        r.check("no test error", False, repr(e))
    finally:
        r.close()
        if httpd:
            httpd.shutdown()
    with open(base + ".log", "w") as f:
        f.write("\n".join(r.lines) + "\n")
    with open(base + ".json", "w") as f:
        json.dump({"ok": ok, "checks": r.checks, "summary": r.summary, "errors": r.errors}, f, indent=2)
    print(json.dumps({"ok": ok, "checks": r.checks}, indent=2))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
