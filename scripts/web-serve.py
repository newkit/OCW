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
"""

import argparse
import functools
import http.server
import os
import re


class Handler(http.server.SimpleHTTPRequestHandler):
    """Static files with COOP / COEP and single byte ranges."""

    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
        ".json": "application/json",
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


def server(directory, bind="127.0.0.1", port=0, stats=None):
    """A threading HTTP server for `directory` (port 0: a free one)."""
    handler = type("CountingHandler", (Handler,), {"stats": stats})
    return http.server.ThreadingHTTPServer(
        (bind, port), functools.partial(handler, directory=directory))


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--port", type=int, default=8080)
    p.add_argument("--dir", default="target/web")
    p.add_argument("--bind", default="127.0.0.1")
    a = p.parse_args()
    with server(a.dir, a.bind, a.port) as httpd:
        print(f"serving {a.dir} on http://{a.bind}:{a.port}/", flush=True)
        worlds = os.path.join(a.dir, "worlds")
        if os.path.isdir(worlds):
            ids = sorted(os.path.relpath(r, worlds) for r, _, fs in os.walk(worlds) if "latest" in fs)
            root = os.path.join("target", "web-keys", "root.hex")
            trust = ""
            if os.path.isfile(root):
                trust = "&trust-root=" + open(root).read().strip()
            for i in ids:
                print(f"  world {i}: http://{a.bind}:{a.port}/?world={i}{trust}", flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    main()
