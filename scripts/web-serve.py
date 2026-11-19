#!/usr/bin/env python3
"""Serves the browser app (target/web/, `make web`) for testing.

    scripts/web-serve.py [--port 8080] [--dir target/web] [--bind 127.0.0.1]

Sends the headers the hosting will send (spec §14 Phase 10, 10B / B8):
COOP / COEP for cross-origin isolation (wasm threads later), the wasm MIME
type (streaming compilation), and no caching (fresh builds while testing).

World packages (B2) are served from target/web/worlds/ (`make web-worlds`:
the `world-builder publish` layout <id>/<version>/{manifest.json, *.ocwpkg}
and <id>/latest) with HTTP range requests (`Range: bytes=a-b` → 206 with
Content-Range), as the app streams them tile by tile.

Development only: no TLS (WebGPU and WebTransport need a secure context,
which http://localhost and 127.0.0.1 are).

Without a build (no index.html in --dir) it exits with status 2 and says
what to run, instead of answering every request with 404: `make web` where
the build runs, or `make web-remote MACHINE=…` to build on a build machine
and fetch the page here.
"""

import argparse
import functools
import http.server
import ipaddress
import os
import re
import socket
import subprocess
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    """Static files with COOP / COEP and single byte ranges."""

    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
        ".json": "application/json",
        ".webmanifest": "application/manifest+json",
        ".ocwpkg": "application/octet-stream",
        "": "text/plain",
    }
    # Set by the server owner to count what was served (tests).
    stats = None

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *args):
        if os.environ.get("OCW_WEB_SERVE_LOG"):
            super().log_message(*args)

    def count(self, n):
        if self.stats is not None:
            path = self.path.split("?", 1)[0]
            key = "worlds" if path.startswith("/worlds/") else "app"
            self.stats[key + "_bytes"] = self.stats.get(key + "_bytes", 0) + n
            self.stats[key + "_requests"] = self.stats.get(key + "_requests", 0) + 1
            if self.headers.get("Range"):
                self.stats["range_requests"] = self.stats.get("range_requests", 0) + 1

    def do_GET(self):
        rng = self.headers.get("Range")
        path = self.translate_path(self.path)
        if not rng or not os.path.isfile(path):
            return self.whole()
        m = re.fullmatch(r"bytes=(\d*)-(\d*)", rng.strip())
        size = os.path.getsize(path)
        if not m or (m.group(1) == "" and m.group(2) == ""):
            return self.whole()
        if m.group(1) == "":
            start, end = max(0, size - int(m.group(2))), size - 1
        else:
            start = int(m.group(1))
            end = int(m.group(2)) if m.group(2) else size - 1
        end = min(end, size - 1)
        if start > end or start >= size:
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        length = end - start + 1
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(length))
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            left = length
            while left > 0:
                chunk = f.read(min(left, 1 << 20))
                if not chunk:
                    break
                self.wfile.write(chunk)
                left -= len(chunk)
        self.count(length)

    def whole(self):
        f = self.send_head()
        if f:
            try:
                n = 0
                while True:
                    chunk = f.read(1 << 20)
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    n += len(chunk)
                self.count(n)
            finally:
                f.close()


def missing_build(directory, modules=()):
    """None when `directory` holds a built page (index.html and `modules`),
    else the message saying what is missing and what to run."""
    want = ["index.html", *modules]
    missing = [f for f in want if not os.path.isfile(os.path.join(directory, f))]
    if not missing:
        return None
    what = (f"{directory}/ does not exist" if not os.path.isdir(directory)
            else f"{directory}/ has no {', '.join(missing)}")
    return (f"{what}: the browser app is not built here.\n"
            "  make web                      build it on this machine (scripts/build-web.sh), or\n"
            "  make web-remote MACHINE=marge build it on a build machine and fetch target/web/ here\n"
            "                                (WORLDS=1 also publishes and fetches the worlds)")


def server(directory, bind="127.0.0.1", port=0, stats=None):
    """A threading HTTP server for `directory` (port 0: a free one)."""
    handler = type("CountingHandler", (Handler,), {"stats": stats})
    return http.server.ThreadingHTTPServer(
        (bind, port), functools.partial(handler, directory=directory))


def is_loopback(bind):
    """True for 127.0.0.0/8, ::1 and localhost: pages there are a secure
    context."""
    if bind == "localhost":
        return True
    try:
        return ipaddress.ip_address(bind).is_loopback
    except ValueError:
        return False


# Interfaces of containers and VMs on this machine, not of the LAN.
VIRTUAL_IFACES = re.compile(r"^(docker|br-|veth|virbr|vnet|lxc|lxd|cni|flannel|podman|tun|wg)")


def default_route_address():
    """The IPv4 address of the default route (no packet is sent: connect()
    on a UDP socket only picks the route)."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(("192.0.2.1", 9))
            return s.getsockname()[0]
    except OSError:
        return None


def lan_addresses(bind, ip_addr=None, hostname_i=None, primary="auto"):
    """The addresses other machines reach a server bound to `bind` by: the
    address itself, or for 0.0.0.0 / :: this machine's IPv4 addresses, the
    default route's first, without container / VM bridges (`ip -o -4 addr`
    on Linux, else `hostname -I`, else only the default route's). No
    loopback, no link-local."""
    if bind not in ("0.0.0.0", "::", ""):
        return [bind]

    def run(cmd):
        try:
            return subprocess.run(cmd, capture_output=True, text=True, timeout=5).stdout
        except (OSError, subprocess.TimeoutExpired):
            return ""

    if primary == "auto":
        primary = default_route_address()
    if ip_addr is None:
        ip_addr = run(["ip", "-o", "-4", "addr", "show", "scope", "global"])
    found = []
    for line in ip_addr.splitlines():
        m = re.match(r"\s*\d+:\s+(\S+)\s+inet\s+([\d.]+)/", line)
        if m and not VIRTUAL_IFACES.match(m.group(1)):
            found.append(m.group(2))
    if not ip_addr.strip():
        if hostname_i is None:
            hostname_i = run(["hostname", "-I"])
        found += hostname_i.split()
    if primary:
        found.insert(0, primary)
    out = []
    for a in found:
        try:
            ip = ipaddress.ip_address(a)
        except ValueError:
            continue
        if ip.version == 4 and not ip.is_loopback and not ip.is_link_local and a not in out:
            out.append(a)
    return out


def lan_warning(bind, port, addresses):
    """What a non-loopback bind means for the pages (the README's step 3)."""
    lines = [
        f"warning: serving on {bind}, not only on localhost. A page opened by a LAN address",
        "  (http://ADDRESS:PORT/) is not a secure context: no WebGPU (the WebGL 2 build loads),",
        "  no Web Bluetooth (virtual power only), no WebTransport (no multiplayer), no world cache;",
        "  and the world trust options (trust-root, trust=dev, allow-unsigned) work only on",
        "  localhost pages, so streamed worlds do not load. For all of that, tunnel the port and",
        f"  open the page on localhost: ssh -N -L {port}:127.0.0.1:{port} USER@THIS-MACHINE",
        f"  then http://localhost:{port}/ on the other machine.",
    ]
    if addresses:
        lines.append("  LAN URLs (solo rides on the test routes, WebGL 2):")
        lines += [f"    http://{a}:{port}/?route=hills" for a in addresses]
    else:
        lines.append("  (no LAN address of this machine found)")
    return "\n".join(lines)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--port", type=int, default=8080)
    p.add_argument("--dir", default="target/web")
    p.add_argument("--bind", default="127.0.0.1",
                   help="address to listen on (make web-serve WEB_BIND=0.0.0.0 for the LAN)")
    a = p.parse_args()
    msg = missing_build(a.dir)
    if msg:
        print(f"web-serve: {msg}", file=sys.stderr)
        sys.exit(2)
    with server(a.dir, a.bind, a.port) as httpd:
        print(f"serving {a.dir} on http://{a.bind}:{a.port}/", flush=True)
        # The links to open here: on localhost, a secure context.
        host = a.bind if is_loopback(a.bind) else "127.0.0.1"
        if not is_loopback(a.bind):
            print(lan_warning(a.bind, a.port, lan_addresses(a.bind)), flush=True)
            if a.bind not in ("0.0.0.0", "::", ""):
                host = None
        worlds = os.path.join(a.dir, "worlds")
        if host and os.path.isdir(worlds):
            ids = sorted(os.path.relpath(r, worlds) for r, _, fs in os.walk(worlds) if "latest" in fs)
            root = os.path.join("target", "web-keys", "root.hex")
            trust = ""
            if os.path.isfile(root):
                trust = "&trust-root=" + open(root).read().strip()
            for i in ids:
                print(f"  world {i}: http://{host}:{a.port}/?world={i}{trust}", flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    main()
