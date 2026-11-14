#!/usr/bin/env python3
"""Serves the browser app (target/web/, `make web`) for testing.

    scripts/web-serve.py [--port 8080] [--dir target/web] [--bind 127.0.0.1]

Sends the headers the hosting will send (spec §14 Phase 10, 10B / B8):
COOP / COEP for cross-origin isolation (wasm threads later), the wasm MIME
type (streaming compilation), and no caching (fresh builds while testing).
Development only: no TLS (WebGPU and WebTransport need a secure context,
which http://localhost and 127.0.0.1 are).
"""

import argparse
import functools
import http.server


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--port", type=int, default=8080)
    p.add_argument("--dir", default="target/web")
    p.add_argument("--bind", default="127.0.0.1")
    a = p.parse_args()
    handler = functools.partial(Handler, directory=a.dir)
    with http.server.ThreadingHTTPServer((a.bind, a.port), handler) as httpd:
        print(f"serving {a.dir} on http://{a.bind}:{a.port}/", flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    main()
