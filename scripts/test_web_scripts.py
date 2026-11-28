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
pwi = load("preview_worlds_index", "preview_worlds_index.py")
tw = load("test_web", "test-web.py")
wf = load("wasm_features", "wasm_features.py")
KEY = "ab" * 32


class WasmLimit(unittest.TestCase):
    """spec §14 Phase 8 T8: a world ride fails above the wasm memory limit."""

    def test_world_rides_have_a_default_limit(self):
        from types import SimpleNamespace as NS
        self.assertEqual(tw.wasm_limit(NS(max_wasm_mb=None), True), tw.WORLD_WASM_LIMIT_MB)
        self.assertEqual(tw.wasm_limit(NS(max_wasm_mb=None), False), 0.0)
        self.assertEqual(tw.wasm_limit(NS(max_wasm_mb=800), False), 800.0)
        self.assertEqual(tw.wasm_limit(NS(max_wasm_mb=0), True), 0.0)
        # The preview test's ride arguments have no such option.
        self.assertEqual(tw.wasm_limit(NS(), True), tw.WORLD_WASM_LIMIT_MB)

    def test_the_regression_fails(self):
        # The Liechtenstein ride of v3.49 (2 705 MB) fails, a fixed one not.
        self.assertIn("2705 MB", tw.wasm_error(2705.0, 1500.0))
        self.assertIsNone(tw.wasm_error(320.0, 1500.0))
        self.assertIsNone(tw.wasm_error(None, 1500.0))
        self.assertIsNone(tw.wasm_error(2705.0, 0.0))

    def test_the_status_line_gives_heap_and_gpu(self):
        # spec §14 Phase 10 B2: the memory per resident tile.
        s = tw.streaming_line(
            "world streaming: 49 of 2265 tiles resident (3.1 MB, peak 4.2 MB), 52 loaded, "
            "3 dropped, 0 pending; tiles from network 14.2 MB, from cache 0 B, wasm memory 1377.0 MB, "
            "heap 612 MB (peak 840 MB), gpu meshes 120 MB in 2345 (buffers 160 MB), textures 300 MB in 410")
        self.assertEqual((s["resident"], s["tiles"], s["dropped"]), (49, 2265, 3))
        self.assertEqual(s["wasm_mb"], 1377.0)
        self.assertEqual((s["heap_mb"], s["heap_peak_mb"]), (612.0, 840.0))
        self.assertEqual((s["gpu_meshes_mb"], s["meshes"], s["gpu_buffers_mb"], s["gpu_textures_mb"]),
                         (120.0, 2345, 160.0, 300.0))
        # An older app's line (no heap, no GPU) and other lines.
        old = tw.streaming_line("world streaming: 9 of 510 tiles resident (1.0 MB, peak 1.2 MB), "
                                "9 loaded, 0 dropped, 0 pending; wasm memory 430.0 MB")
        self.assertEqual(old["wasm_mb"], 430.0)
        self.assertNotIn("heap_mb", old)
        self.assertIsNone(tw.streaming_line("world ready"))


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


class WasmSimd(unittest.TestCase):
    """Q10 (a): both builds use WebAssembly SIMD; the preview's site refuses
    a build without it."""

    @staticmethod
    def module(feats, custom_first=True):
        def leb(n):
            out = bytearray()
            while True:
                b = n & 0x7F
                n >>= 7
                out.append(b | (0x80 if n else 0))
                if not n:
                    return bytes(out)
        body = leb(len(feats)) + b"".join(p.encode() + leb(len(f)) + f.encode() for p, f in feats)
        name = b"target_features"
        custom = b"\0" + leb(len(name) + 1 + len(body)) + leb(len(name)) + name + body
        types = b"\x01\x04\x01\x60\x00\x00"
        return b"\0asm\x01\0\0\0" + (custom + types if custom_first else types + custom)

    def test_features_of_the_linker_section(self):
        m = self.module([("+", "simd128"), ("+", "bulk-memory"), ("-", "atomics")])
        self.assertEqual(wf.features(m), {"simd128", "bulk-memory"})
        self.assertEqual(wf.features(self.module([("+", "mutable-globals")], custom_first=False)), {"mutable-globals"})
        self.assertIsNone(wf.features(b"\0asm\x01\0\0\0\x01\x04\x01\x60\x00\x00"))
        with self.assertRaises(ValueError):
            wf.features(b"not wasm")

    def test_require_exits_without_simd(self):
        with tempfile.TemporaryDirectory() as d:
            ok, old = Path(d, "ok.wasm"), Path(d, "old.wasm")
            ok.write_bytes(self.module([("+", "simd128")]))
            old.write_bytes(self.module([("+", "sign-ext")]))
            run = lambda p: subprocess.run([sys.executable, "-I", str(HERE / "wasm_features.py"), str(p),
                                            "--require", "simd128"], capture_output=True, text=True)
            self.assertEqual(run(ok).returncode, 0)
            r = run(old)
            self.assertEqual(r.returncode, 1)
            self.assertIn("without simd128", r.stderr)

    def test_the_preview_site_needs_the_simd_builds(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertFalse(site.built_with_simd(d))
            Path(d, "sizes.txt").write_text("profile: web-preview-release\n")
            self.assertFalse(site.built_with_simd(d))
            Path(d, "sizes.txt").write_text("profile: x\nwasm features: +simd128 (Q10 (a))\n")
            self.assertTrue(site.built_with_simd(d))


class PreviewXrSite(unittest.TestCase):
    """14I Q6 (a): the WebXR build is published under xr/ next to the normal
    preview, which stays byte for byte as it was."""

    def build(self, d, names):
        os.makedirs(d)
        for n in names:
            Path(d, n).write_bytes(os.urandom(3000) if n.endswith(".wasm") else n.encode())
        # Q10 (a): the builds say they use SIMD (build-web.sh's sizes.txt).
        Path(d, "sizes.txt").write_text("wasm features: +simd128\n")
        # The preview site needs the voice SDK next to the page (build-web.sh
        # --preview fetches it); a stand-in keeps these XR cases about XR.
        if d.endswith("web"):
            Path(d, "livekit-client.umd.js").write_text("// stand-in\n")

    def release(self, d):
        os.makedirs(d)
        pkg, man = b"OCWPKG02" + os.urandom(100), json.dumps({"signed": {"name": "Liechtenstein"}}).encode()
        Path(d, "li.ocwpkg").write_bytes(pkg)
        Path(d, "li.manifest.json").write_bytes(man)
        Path(d, "preview-worlds.json").write_text(json.dumps({"worlds": [{
            "id": "liechtenstein", "name": "Liechtenstein", "version": 4, "bytes": len(pkg),
            "routes": [{"name": "Vaduz loop", "km": 20.0, "slug": "vaduz-loop"}],
            "files": {"worlds/liechtenstein/4/liechtenstein.ocwpkg": "li.ocwpkg",
                      "worlds/liechtenstein/4/manifest.json": "li.manifest.json"},
            "sha256": {"li.ocwpkg": hashlib.sha256(pkg).hexdigest(),
                       "li.manifest.json": hashlib.sha256(man).hexdigest()}}]}))

    def run_site(self, root, *extra):
        out = os.path.join(root, "site" + str(len(extra)))
        r = subprocess.run([sys.executable, "-I", str(HERE / "web-preview-site.py"),
                            "--build", os.path.join(root, "web"), "--worlds", os.path.join(root, "rel"),
                            "--out", out, "--build-id", "abc123", *extra],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        return out, r.stdout

    def files(self, out):
        return {str(p.relative_to(out)): p.read_bytes() for p in Path(out).rglob("*") if p.is_file()}

    def test_an_xr_build_without_simd_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            self.build(os.path.join(root, "web"), ["index.html", "ocw_web.js", "ocw_web_bg.wasm",
                                                   "ocw_web_webgl2.js", "ocw_web_webgl2_bg.wasm"])
            self.build(os.path.join(root, "xr"), ["index.html", "ocw_web_webxr.js", "ocw_web_webxr_bg.wasm"])
            self.release(os.path.join(root, "rel"))
            Path(root, "xr", "sizes.txt").write_text("profile: web-preview-release\n")
            r = subprocess.run([sys.executable, "-I", str(HERE / "web-preview-site.py"),
                                "--build", os.path.join(root, "web"), "--worlds", os.path.join(root, "rel"),
                                "--out", os.path.join(root, "site"), "--xr-build", os.path.join(root, "xr")],
                               capture_output=True, text=True)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("not built with WebAssembly SIMD", r.stderr)

    def test_xr_next_to_the_unchanged_preview(self):
        with tempfile.TemporaryDirectory() as root:
            self.build(os.path.join(root, "web"), ["index.html", "ocw_web.js", "ocw_web_bg.wasm",
                                                   "ocw_web_webgl2.js", "ocw_web_webgl2_bg.wasm", "ocw-xr.js"])
            self.build(os.path.join(root, "xr"), ["index.html", "ocw_web_webxr.js", "ocw_web_webxr_bg.wasm",
                                                  "ocw-xr.js", "ocw-gpu.js", "config.json"])
            self.release(os.path.join(root, "rel"))
            plain, _ = self.run_site(root)
            with_xr, said = self.run_site(root, "--xr-build", os.path.join(root, "xr"))
            self.assertIn("WebXR at xr/", said)
            a, b = self.files(plain), self.files(with_xr)
            # The normal preview: the same files with the same bytes (site.txt
            # lists the xr/ files too).
            for k, v in a.items():
                if k != "site.txt":
                    self.assertEqual(b[k], v, k)
            xr = {k for k in b if k.startswith("xr/")}
            self.assertEqual(set(b) - set(a), xr)
            self.assertIn("xr/ocw_web_webxr_bg.wasm", xr)
            self.assertIn("xr/ocw-xr.js", xr)
            self.assertNotIn("xr/ocw_web_bg.wasm", xr)
            self.assertFalse(any(k.startswith("xr/worlds") for k in xr), "the worlds are shared")
            config = json.loads(b["xr/config.json"])
            self.assertEqual(config["worlds"], "../worlds/")
            self.assertEqual(config["channel"], "preview")
            self.assertTrue(config["start"])
            self.assertTrue(all(r["query"].endswith("&xr=1") for r in config["start"]))
            self.assertIn("world=liechtenstein&route=vaduz-loop&xr=1", [r["query"] for r in config["start"]])
            self.assertIn(b"../credits.html", b["xr/credits.html"])
            self.assertIn("xr/ocw_web_webxr_bg.wasm", b["site.txt"].decode())
            # The /xr/ page's worlds resolve to the site's (as the browser
            # does it).
            from urllib.parse import urljoin
            self.assertEqual(urljoin("https://newkit.github.io/OCW/xr/", config["worlds"]),
                             "https://newkit.github.io/OCW/worlds/")

    def test_no_xr_module_is_skipped(self):
        with tempfile.TemporaryDirectory() as root:
            self.build(os.path.join(root, "web"), ["index.html", "ocw_web_bg.wasm", "ocw_web_webgl2_bg.wasm"])
            self.build(os.path.join(root, "xr"), ["index.html"])
            self.release(os.path.join(root, "rel"))
            out, said = self.run_site(root, "--xr-build", os.path.join(root, "xr"))
            self.assertIn("no /xr/", said)
            self.assertFalse(os.path.exists(os.path.join(out, "xr")))

    def test_the_size_limit_counts_xr(self):
        with tempfile.TemporaryDirectory() as root:
            self.build(os.path.join(root, "web"), ["index.html", "ocw_web_bg.wasm", "ocw_web_webgl2_bg.wasm"])
            self.build(os.path.join(root, "xr"), ["index.html", "ocw_web_webxr_bg.wasm"])
            self.release(os.path.join(root, "rel"))
            r = subprocess.run([sys.executable, "-I", str(HERE / "web-preview-site.py"),
                                "--build", os.path.join(root, "web"), "--worlds", os.path.join(root, "rel"),
                                "--out", os.path.join(root, "o"), "--max-mb", "0.0105",
                                "--xr-build", os.path.join(root, "xr")], capture_output=True, text=True)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("GitHub Pages: 1 GB", r.stderr)


class PreviewWorldsIndex(unittest.TestCase):
    """The preview's release index finds worlds whose ids have several levels
    (europe/spain/tenerife; regression of 2026-11-15) as well as one level."""

    def world(self, out, wid, version, pkg):
        vdir = Path(out, "worlds", *wid.split("/"), str(version))
        vdir.mkdir(parents=True)
        Path(vdir.parent, "latest").write_text(f"{version}\n")
        Path(vdir, pkg).write_bytes(wid.encode() * 10)
        Path(vdir, "manifest.json").write_text(json.dumps(
            {"signed": {"name": wid.split("/")[-1].title(), "files": [{"path": pkg}]}}))

    def test_nested_and_flat_ids(self):
        with tempfile.TemporaryDirectory() as out:
            self.world(out, "liechtenstein", 4, "liechtenstein.ocwpkg")
            self.world(out, "europe/spain/tenerife", 8, "tenerife.ocwpkg")
            Path(out, "routes.jsonl").write_text(
                json.dumps({"package": "tenerife.ocwpkg", "name": "Puerto de la Cruz - Teide", "km": 42.0}) + "\n")
            self.assertEqual(pwi.world_ids(os.path.join(out, "worlds")),
                             ["europe/spain/tenerife", "liechtenstein"])
            index = pwi.build(out, "web-preview-worlds")
            ten = index["worlds"][0]
            self.assertEqual((ten["id"], ten["version"]), ("europe/spain/tenerife", 8))
            self.assertEqual(ten["files"]["worlds/europe/spain/tenerife/8/tenerife.ocwpkg"],
                             "europe-spain-tenerife-8.ocwpkg")
            self.assertEqual(ten["routes"], [{"name": "Puerto de la Cruz - Teide",
                                              "slug": "puerto-de-la-cruz-teide", "km": 42.0}])
            for asset in ten["files"].values():
                self.assertTrue(Path(out, "release", asset).is_file())
            sums = Path(out, "release", "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(sums), 6)
            # A second run (the same out dir) replaces the release files.
            pwi.build(out, "web-preview-worlds")


if __name__ == "__main__":
    unittest.main()
