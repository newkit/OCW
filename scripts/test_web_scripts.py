#!/usr/bin/env python3
"""Tests of the browser app's scripts (no network, no build, no browser):
scripts/web-serve.py, scripts/web_browser.py, scripts/test-web-readme.py.

    python3 scripts/test_web_scripts.py
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True  # no __pycache__ in scripts/
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import web_browser as wb  # noqa: E402


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ws = load("web_serve", "web-serve.py")
rd = load("test_web_readme", "test-web-readme.py")
site = load("web_preview_site", "web-preview-site.py")
KEY = "ab" * 32


class WebServe(unittest.TestCase):
    def test_missing_dir(self):
        with tempfile.TemporaryDirectory() as d:
            msg = ws.missing_build(os.path.join(d, "web"))
            self.assertIn("does not exist", msg)
            self.assertIn("make web-remote MACHINE=", msg)
            self.assertIn("make web ", msg)

    def test_empty_dir_and_modules(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertIn("has no index.html", ws.missing_build(d))
            Path(d, "index.html").write_text("<html>")
            self.assertIsNone(ws.missing_build(d))
            self.assertIn("ocw_web_bg.wasm", ws.missing_build(d, ["ocw_web_bg.wasm"]))

    def test_main_exits_with_the_message(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([sys.executable, str(HERE / "web-serve.py"), "--dir",
                                os.path.join(d, "web"), "--port", "0"],
                               capture_output=True, text=True, timeout=30)
            self.assertEqual(p.returncode, 2)
            self.assertIn("make web-remote", p.stderr)
            self.assertNotIn("serving", p.stdout)

    def test_loopback(self):
        for b in ("127.0.0.1", "127.0.1.1", "::1", "localhost"):
            self.assertTrue(ws.is_loopback(b), b)
        for b in ("0.0.0.0", "192.168.1.5", "::", "example"):
            self.assertFalse(ws.is_loopback(b), b)

    def test_lan_addresses(self):
        ip = ("1: lo    inet 127.0.0.1/8 scope host lo\n"
              "2: enp3s0    inet 192.168.1.5/24 brd 192.168.1.255 scope global enp3s0\n"
              "3: wlan0    inet 10.0.0.2/24 scope global wlan0\n"
              "4: docker0    inet 172.17.0.1/16 scope global docker0\n"
              "5: br-1a2b    inet 172.18.0.1/16 scope global br-1a2b\n")
        self.assertEqual(ws.lan_addresses("0.0.0.0", ip, primary="10.0.0.2"), ["10.0.0.2", "192.168.1.5"])
        # Without `ip`: hostname -I.
        self.assertEqual(ws.lan_addresses("0.0.0.0", "", "192.168.1.5 fe80::1 127.0.1.1 192.168.1.5\n",
                                          primary=None), ["192.168.1.5"])
        self.assertEqual(ws.lan_addresses("10.1.2.3", ""), ["10.1.2.3"])

    def test_lan_warning(self):
        w = ws.lan_warning("0.0.0.0", 8080, ["192.168.1.5"])
        for want in ("not a secure context", "WebGL 2", "Web Bluetooth", "WebTransport",
                     "trust", "ssh -N -L 8080:127.0.0.1:8080", "http://192.168.1.5:8080/?route=hills"):
            self.assertIn(want, w)
        self.assertIn("no LAN address", ws.lan_warning("0.0.0.0", 8080, []))


def make_zip(path, members):
    """members: (name, data, mode) – mode with stat.S_IFLNK for a symlink."""
    with zipfile.ZipFile(path, "w") as z:
        for name, data, mode in members:
            info = zipfile.ZipInfo(name)
            info.external_attr = mode << 16
            z.writestr(info, data)


class WebBrowser(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = Path(self.tmp.name)
        self.old_env = os.environ.pop("OCW_BROWSER", None)

    def tearDown(self):
        if self.old_env is not None:
            os.environ["OCW_BROWSER"] = self.old_env
        self.tmp.cleanup()

    def pin(self, archive, sha=None):
        sha = sha or hashlib.sha256(archive.read_bytes()).hexdigest()
        p = {"name": "chromium", "version": "1.2.3",
             "platforms": {"linux-x64": {"url": archive.as_uri(), "sha256": sha,
                                         "exe": "chrome-linux/chrome"}}}
        return wb.pin_for(p, "linux-x64"), p

    def test_pins_file(self):
        p = wb.pins()
        self.assertEqual(p["name"], "chromium")
        for key in ("linux-x64", "mac-arm64", "win-x64"):
            pin = wb.pin_for(p, key)
            self.assertRegex(pin["sha256"], r"^[0-9a-f]{64}$")
            self.assertTrue(pin["url"].startswith("https://"))
            self.assertIn(str(pin["revision"]), pin["url"])

    def test_platform_key(self):
        self.assertEqual(wb.platform_key("Linux", "x86_64"), "linux-x64")
        self.assertEqual(wb.platform_key("Darwin", "arm64"), "mac-arm64")
        self.assertEqual(wb.platform_key("Windows", "AMD64"), "win-x64")
        self.assertIsNone(wb.platform_key("SunOS", "sparc"))

    def test_fetch_checks_and_unpacks(self):
        z = self.d / "src" / "b.zip"
        z.parent.mkdir()
        make_zip(z, [("chrome-linux/chrome", "#!/bin/sh\necho Chromium 1.2.3\n", stat.S_IFREG | 0o755),
                     ("chrome-linux/lib.so", "x", stat.S_IFREG | 0o644),
                     ("chrome-linux/link", "lib.so", stat.S_IFLNK | 0o777)])
        pin, _ = self.pin(z)
        root = self.d / "cache"
        with contextlib.redirect_stderr(io.StringIO()):
            exe = wb.fetch(pin, str(root), progress=False)
        self.assertEqual(exe, str(root / "chromium-1.2.3" / "chrome-linux" / "chrome"))
        self.assertTrue(os.access(exe, os.X_OK))
        self.assertTrue(os.path.islink(root / "chromium-1.2.3" / "chrome-linux" / "link"))
        self.assertEqual(wb.installed(pin, str(root)), exe)
        self.assertEqual(json.loads((root / "chromium-1.2.3" / wb.MARKER).read_text())["sha256"],
                         pin["sha256"])
        # A second fetch finds it; a new checksum is a new install.
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(wb.fetch(pin, str(root)), exe)
        self.assertIsNone(wb.installed(dict(pin, sha256="0" * 64), str(root)))

    def test_fetch_rejects_a_wrong_checksum(self):
        z = self.d / "b.zip"
        make_zip(z, [("chrome-linux/chrome", "x", stat.S_IFREG | 0o755)])
        pin, _ = self.pin(z, sha="0" * 64)
        root = self.d / "cache"
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "SHA-256"):
            wb.fetch(pin, str(root), progress=False)
        self.assertFalse((root / "chromium-1.2.3").exists())
        self.assertEqual([p.name for p in root.iterdir()], [])  # no work directory left

    def test_unzip_rejects_escapes(self):
        for members in ([("../evil", "x", stat.S_IFREG | 0o644)],
                        [("/abs", "x", stat.S_IFREG | 0o644)],
                        [("a/link", "../../etc", stat.S_IFLNK | 0o777)]):
            z = self.d / "e.zip"
            make_zip(z, members)
            out = self.d / "out"
            out.mkdir(exist_ok=True)
            with self.assertRaisesRegex(ValueError, "unsafe"):
                wb.unzip(z, out)

    def test_find_order(self):
        z = self.d / "b.zip"
        make_zip(z, [("chrome-linux/chrome", "x", stat.S_IFREG | 0o755)])
        pin, pins = self.pin(z)
        root = str(self.d / "cache")
        which = {"chromium": "/usr/bin/chromium"}.get
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            # Nothing installed: the system's, with a warning.
            self.assertEqual(wb.find(p=pins, key="linux-x64", root=root, which=which), "/usr/bin/chromium")
            self.assertIn("make deps-browser", err.getvalue())
            # Nothing at all: the plain name.
            self.assertEqual(wb.find(p=pins, key="linux-x64", root=root, which=lambda n: None), "chromium")
            wb.fetch(pin, root, progress=False)
        exe = wb.installed(pin, root)
        self.assertEqual(wb.find(p=pins, key="linux-x64", root=root, which=which), exe)
        os.environ["OCW_BROWSER"] = "/opt/my/chrome"
        self.assertEqual(wb.find(p=pins, key="linux-x64", root=root, which=lambda n: None), "/opt/my/chrome")


class ReadmeWalkthrough(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sec = rd.section((HERE.parent / "README.md").read_text())

    def test_steps_follow_the_readme(self):
        steps = rd.make_steps(self.sec)
        for cmd in ("make deps-web", "make web", "make web-worlds", "make web-serve"):
            self.assertIn(cmd, steps)
        self.assertLess(steps.index("make deps-web"), steps.index("make web"))
        self.assertLess(steps.index("make web"), steps.index("make web-worlds"))
        self.assertFalse([c for c in steps if c.startswith("make web-remote")])

    def test_urls_of_the_readme(self):
        us = rd.urls(self.sec)
        readme = [u["readme"] for u in us]
        self.assertIn("http://127.0.0.1:8080/?route=hills", readme)
        self.assertIn("http://127.0.0.1:8080/?route=hills&gpu=webgl2&quality=low", readme)
        self.assertTrue(any("trust-root=<" in u for u in readme), readme)
        self.assertTrue(all(u.get("skip") for u in us if "server=" in u["readme"]))
        self.assertTrue(any(not u.get("skip") for u in us))
        self.assertEqual(len(readme), len(set(readme)))
        self.assertFalse([u for u in readme if "$" in u], readme)
        self.assertIsNotNone(rd.oneliner(self.sec))

    def test_concrete(self):
        u = rd.concrete("http://127.0.0.1:8080/?world=x&trust-root=<the key of target/web-keys/root.hex>",
                        KEY, 9001)
        self.assertEqual(u, f"http://127.0.0.1:9001/?world=x&trust-root={KEY}")
        u = rd.concrete("http://localhost:8080/?world=x&trust-root=" + "3f" * 32 + "&a=1", KEY, 9001)
        self.assertEqual(u, f"http://localhost:9001/?world=x&trust-root={KEY}&a=1")

    def test_expects_and_slug(self):
        self.assertEqual(rd.expects("http://h/?route=hills&gpu=webgl2"), {"world": False, "gpu": "webgl2"})
        self.assertEqual(rd.expects(f"http://h/?world=x&trust-root={KEY}"), {"world": True, "gpu": "webgpu"})
        self.assertEqual(rd.slug(f"http://h/?world=x&free=1&trust-root={KEY}"), "world-x-free-1-key")
        self.assertEqual(rd.slug("http://h/"), "root")

    def test_errors(self):
        ev = [{"method": "Network.responseReceived",
               "params": {"type": "Document", "response": {"status": 200, "url": "http://h/"}}},
              {"method": "Network.responseReceived",
               "params": {"type": "Other", "response": {"status": 404, "url": "http://h/x.js"}}},
              {"method": "Network.loadingFailed", "params": {"canceled": True, "requestId": "1"}}]
        self.assertEqual(rd.document_status(ev), 200)
        self.assertEqual(rd.http_errors(ev), [(404, "http://h/x.js")])
        self.assertEqual(rd.http_errors(ev, 2), [])
        log = '127.0.0.1 - - [x] "GET / HTTP/1.1" 200 -\n127.0.0.1 - - [x] "GET /y HTTP/1.1" 404 -\n'
        self.assertEqual(len(rd.server_errors(log)), 1)


class PreviewRideSizes(unittest.TestCase):
    """The start page tells the download size and load time of each ride."""

    def test_load_time(self):
        self.assertEqual(site.load_time(1e5), "under 1 s")
        self.assertEqual(site.load_time(100e6), "about 16 s")  # 800 Mbit at 50 Mbit/s
        self.assertEqual(site.load_time(500e6), "about 80 s")
        self.assertEqual(site.load_time(600e6), "about 2 min")

    def test_every_ride_has_a_size(self):
        found = [{"id": "europe/spain/tenerife", "name": "Tenerife", "bytes": 500_000_000,
                  "routes": [{"name": "Puerto - Teide", "km": 42.04, "slug": "puerto-teide"}]}]
        rides = site.start_rides(found, app=15_000_000)
        self.assertEqual(len(rides), len(site.TEST_RIDES) + 2)
        for r in rides:
            self.assertIn("MB", r["size"])
        self.assertIn("no world data", rides[0]["size"])
        self.assertEqual(rides[2]["size"], "Download: 15 MB app + up to 500 MB world, streamed while you ride"
                         " (all of it: about 82 s at 50 Mbit/s); cached for the next visit.")
        self.assertEqual(rides[2]["text"], "42.0 km on real roads.")
        self.assertNotIn("size", site.TEST_RIDES[0])  # the constant stays untouched

    def test_app_bytes(self):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "ocw_web_bg.wasm").write_bytes(os.urandom(4000))
            Path(d, "ocw_web.js").write_bytes(b"x" * 100)
            n = site.app_bytes(d)
            self.assertGreater(n, 4000)  # random bytes do not compress
            self.assertEqual(site.app_bytes(os.path.join(d, "none")), 0)


if __name__ == "__main__":
    unittest.main()
