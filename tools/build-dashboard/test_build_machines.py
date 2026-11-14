"""Tests of the build machine settings (build_machines.py: parsing,
defaults, writing, the change log), the POST /api/machines endpoint
(token, validation) and the cards of the page (escaped reason and note;
no network beyond a loopback server)."""

import json
import os
import shutil
import subprocess
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import answers
import build_machines as bm
import dashboard as d

HERE = Path(__file__).resolve().parent
EVIL = '<img src=x onerror="alert(1)"># "quoted" \\ back\nslash'
MACHINES = """\
[defaults]
port = 22

[[machine]]
name = "marge"
os = "linux"
host = "marge.example.lan"

[[machine]]
name = "leela"
os = "linux"
slots = 3
host = "leela.example.lan"

[[machine]]
name = "Win11Builder"
os = "windows"
host = "win.example.lan"

[[machine]]
name = "macos"
os = "macos"
host = "mac.example.lan"
"""


def machines():
    defaults, ms = d.parse_machines(MACHINES)
    return [d.Machine(m, defaults, Path("/cfg")) for m in ms]


class SettingsTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.t = [1_800_000_000.0]
        self.bm = bm.BuildMachines(self.dir / "config" / bm.FILE_NAME, self.dir / "build-log", clock=lambda: self.t[0])
        self.ms = {m.name: m for m in machines()}

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_defaults_without_a_file(self):
        e = {n: self.bm.effective(m) for n, m in self.ms.items()}
        self.assertTrue(all(x["enabled"] for x in e.values()))
        self.assertEqual({n: (x["slots"], x["slots_source"]) for n, x in e.items()},
                         {"marge": (2, "default"), "leela": (3, "machines.toml"),
                          "Win11Builder": (1, "default"), "macos": (1, "default")})

    def test_parse_tolerates_junk(self):
        text = ('[[machine]]\nname = "marge"\nenabled = false\nslots = 99\nreason = "r\\u0023 1"\n'
                '[[machine]]\nname = "macos"\nslots = "x"\n[[machine]]\nenabled = false\n')
        e = bm.parse(text)
        self.assertEqual(sorted(e), ["macos", "marge"])
        self.assertEqual((e["marge"]["enabled"], e["marge"]["slots"], e["marge"]["reason"]), (False, None, "r# 1"))
        self.assertEqual((e["macos"]["enabled"], e["macos"]["slots"]), (True, None))

    def test_tolerant_parser_reads_the_escapes_too(self):
        self.assertEqual(d._unescape_basic('a\\u0023b \\"q\\" \\\\'), 'a#b "q" \\')

    def test_update_writes_file_and_log(self):
        m = self.ms["leela"]
        e = self.bm.update(m, enabled=False, reason=EVIL, by="tester")
        self.assertEqual((e["enabled"], e["slots"], e["slots_source"]), (False, 3, "dashboard"),
                         "a new entry starts with the current slot count")
        self.assertEqual(e["reason"], EVIL.replace("\n", " "), "control characters dropped")
        text = self.bm.path.read_text()
        self.assertNotIn("#", text.split("[[machine]]", 1)[1], "no # (a comment for remote.sh) in values")
        self.assertEqual(bm.parse(text)["leela"]["reason"], e["reason"], "round trip")
        e = self.bm.update(self.ms["marge"], slots=4, note="main Linux box")
        self.assertEqual((e["enabled"], e["slots"], e["note"]), (True, 4, "main Linux box"))
        e = self.bm.update(m, enabled=True)
        self.assertEqual((e["enabled"], e["reason"]), (True, ""), "enabling clears the reason")
        log = self.dir / "build-log" / "2027-01-15" / bm.LOG_NAME
        lines = [json.loads(x) for x in log.read_text().splitlines()]
        self.assertEqual([(x["machine"], sorted(x["changes"])) for x in lines],
                         [("leela", ["enabled", "reason"]), ("marge", ["note", "slots"]),
                          ("leela", ["enabled", "reason"])])
        self.assertEqual((lines[0]["by"], lines[0]["source"], lines[0]["changes"]["enabled"]), ("tester", "dashboard", [True, False]))
        self.assertTrue(lines[0]["time"].startswith("2027-01-15T"))

    def test_validation(self):
        m = self.ms["marge"]
        for bad in (dict(slots=0), dict(slots=9), dict(slots="2"), dict(slots=True), dict(slots=2.5),
                    dict(enabled="no"), dict()):
            with self.assertRaises(ValueError, msg=bad):
                self.bm.update(m, **bad)
        self.assertFalse(self.bm.path.exists(), "nothing written on a bad request")

    def test_hand_edits_are_picked_up(self):
        self.bm.update(self.ms["marge"], slots=2)
        self.assertTrue(self.bm.effective(self.ms["marge"])["enabled"])
        text = self.bm.path.read_text().replace("enabled = true", "enabled = false")
        self.bm.path.write_text(text + "\n")
        self.assertFalse(self.bm.effective(self.ms["marge"])["enabled"])

    def test_status_and_text(self):
        self.bm.update(self.ms["leela"], enabled=False, reason="excluded by the user")
        states = [dict(name=n, os=m.os, online=False, error="timeout", slots=m.slots) for n, m in self.ms.items()]
        d.apply_settings(states, list(self.ms.values()), self.bm)
        st = d.summarize(states)
        self.assertEqual(st["overview"]["disabled"], 1)
        self.assertEqual(st["overview"]["slots_total"], 2 + 1 + 1, "disabled machines' slots not counted")
        leela = next(x for x in st["machines"] if x["name"] == "leela")
        self.assertEqual((leela["enabled"], leela["disabled_reason"], leela["slots"]), (False, "excluded by the user", 3))
        text = d.format_text(st)
        self.assertIn("build: DISABLED (excluded by the user)", text)
        self.assertIn("build: enabled, 2 slots (OS default)", text)

    def test_cli(self):
        cfg = self.dir / "config"
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "machines.toml").write_text(MACHINES)
        run = lambda *a: subprocess.run(["python3", str(HERE / "build_machines.py"), "--config", str(cfg), "--log", str(self.dir / "log")] + list(a),  # noqa: E731
                                        capture_output=True, text=True, timeout=60)
        r = run("set", "leela", "--disable", "--reason", "kiosk only")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("DISABLED", next(l for l in r.stdout.splitlines() if l.startswith("leela")))
        self.assertEqual(run("set", "nosuch", "--enable").returncode, 2)
        self.assertEqual(run("set", "marge", "--slots", "12").returncode, 2)
        self.assertNotIn("example.lan", run("list").stdout)


class HttpTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        dash = d.Dashboard(machines(), use_control=False)
        dash.build_machines = bm.BuildMachines(self.dir / bm.FILE_NAME, self.dir / "build-log")
        dash.token = answers.load_token(self.dir / "dashboard-token")
        self.dash = dash
        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), d.make_handler(dash))
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.base = "http://127.0.0.1:%d" % self.srv.server_address[1]

    def tearDown(self):
        self.srv.shutdown()
        self.srv.server_close()
        shutil.rmtree(self.dir)

    def post(self, body, token=None, raw=None):
        headers = {"Content-Type": "application/json"}
        if token is not None:
            headers[answers.TOKEN_HEADER] = token
        r = urllib.request.Request(self.base + "/api/machines", data=raw if raw is not None else json.dumps(body).encode(),
                                   headers=headers, method="POST")
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    def test_token_required(self):
        body = {"machine": "leela", "enabled": False}
        self.assertEqual(self.post(body)[0], 403)
        self.assertEqual(self.post(body, "wrong")[0], 403)
        self.assertFalse(self.dash.build_machines.path.exists())
        code, res = self.post(dict(body, reason="kiosk only"), self.dash.token)
        self.assertEqual((code, res["settings"]["enabled"], res["settings"]["reason"]), (200, False, "kiosk only"))
        self.assertNotIn(self.dash.token, self.dash.build_machines.path.read_text())
        log = next((self.dir / "build-log").glob("*/" + bm.LOG_NAME)).read_text()
        self.assertNotIn(self.dash.token, log)
        st = self.dash.snapshot()
        self.assertTrue(st["machine_settings"])
        self.assertEqual(next(m for m in st["machines"] if m["name"] == "leela")["enabled"], False)

    def test_validation_and_injection(self):
        t = self.dash.token
        for body in ({"machine": "../../etc/passwd", "enabled": False}, {"machine": "nosuch", "slots": 2},
                     {"machine": ["marge"], "slots": 2}, {"machine": "marge", "slots": 0},
                     {"machine": "marge", "slots": 9}, {"machine": "marge", "slots": "3; rm -rf /"},
                     {"machine": "marge", "enabled": "false"}, {"machine": "marge"}, ["marge"]):
            self.assertEqual(self.post(body, t)[0], 400, body)
        self.assertEqual(self.post(None, t, raw=b"{not json")[0], 400)
        self.assertFalse(self.dash.build_machines.path.exists(), "nothing written")
        self.assertEqual(list(self.dir.glob("**/passwd")), [])
        code, res = self.post({"machine": "marge", "slots": 8, "note": EVIL}, t)
        self.assertEqual((code, res["settings"]["slots"]), (200, 8))
        self.assertEqual(bm.parse(self.dash.build_machines.path.read_text())["marge"]["note"], EVIL.replace("\n", " "))

    def test_read_only(self):
        self.dash.build_machines.read_only = True
        self.assertEqual(self.post({"machine": "marge", "slots": 2}, self.dash.token)[0], 404)
        self.assertFalse(self.dash.snapshot()["machine_settings"])


@unittest.skipUnless(shutil.which("node"), "node not installed")
class CardTest(unittest.TestCase):
    """Renders machine cards in node with a fake DOM."""

    def render(self, machines, settings=True):
        page = (HERE / "index.html").read_text(encoding="utf-8")
        script = page.split("<script>", 1)[1].split("</script>", 1)[0]
        st = {"generated_at": 100, "machines": machines, "machine_settings": settings,
              "overview": {"machines": len(machines), "online": 0, "tasks": 0, "waiting": 0, "slots_used": 0,
                           "slots_total": 0, "disabled": 1}}
        harness = """
const els = {};
global.document = { getElementById: (id) => (els[id] ||= { innerHTML: "", textContent: "", style: {}, addEventListener() {} }) };
global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
global.setInterval = () => 0; global.fetch = async () => ({ json: async () => ({}) });
""" + script + "\nrender(%s); process.stdout.write(els.cards.innerHTML + '\\n@@\\n' + els.overview.innerHTML);\n" % json.dumps(st)
        with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
            f.write(harness)
        try:
            out = subprocess.run(["node", f.name], capture_output=True, text=True, timeout=30)
        finally:
            os.unlink(f.name)
        self.assertEqual(out.returncode, 0, out.stderr[-2000:])
        return out.stdout

    def machine(self, **kw):
        m = dict(name="leela", os="linux", online=False, error="timeout", enabled=False, disabled_reason=EVIL,
                 note=EVIL, slots=2, slots_source="dashboard", slots_config=3, changed_at="2026-11-13T20:00:00+01:00",
                 changed_by="dashboard")
        m.update(kw)
        return m

    def test_disabled_card_escaped_with_controls(self):
        html = self.render([self.machine(), self.machine(name="marge", enabled=True, disabled_reason="", note="",
                                                         slots=8, online=False)])
        self.assertNotIn("<img", html)
        self.assertIn("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;", html)
        self.assertIn('class="card off"', html)
        self.assertIn("Disabled for building", html)
        self.assertIn('data-act="toggle" aria-pressed="false"', html)
        self.assertIn('data-act="toggle" aria-pressed="true"', html)
        self.assertIn('aria-label="More build slots" disabled', html, "8 is the maximum")
        self.assertIn("(machines.toml: 3)", html)
        self.assertIn("disabled for building", html.split("@@")[1])

    def test_without_settings_no_buttons(self):
        html = self.render([self.machine()], settings=False)
        self.assertNotIn('data-act="toggle"', html)
        self.assertIn("disabled for building", html)


if __name__ == "__main__":
    unittest.main()
