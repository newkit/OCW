"""Tests of the agent limit on the coordinator (dashboard 1.5.0): the
`[coordinator] max_agents` setting of build-machines.toml (read, write,
defaults, range, file round trip next to the machines), the counting of
running agents from agents.jsonl, the `agents` field of /api/status and
POST /api/coordinator (token, validation), the page's "agents N of M", and
scripts/agent-slots.sh's output and exit codes with fixture files (no
network beyond a loopback server)."""

import json
import os
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import agent_slots
import answers
import build_machines as bm
import claude_tasks as ct
import dashboard as d
from test_build_machines import MACHINES, machines

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent.parent / "scripts" / "agent-slots.sh"


def tmpdir() -> Path:
    # Never deb's /tmp (a tmpfs): TMPDIR points to disk there (CLAUDE.md).
    return Path(tempfile.mkdtemp(prefix="agent-slots-test."))


def write_agents(log: Path, events: list, now: float | None = None):
    """Fixture agents.jsonl of today: (agent id, event) pairs, one minute apart."""
    now = time.time() if now is None else now
    day = log / time.strftime("%Y-%m-%d", time.localtime(now))
    day.mkdir(parents=True, exist_ok=True)
    lines = []
    for i, (aid, ev) in enumerate(events):
        t = time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(now - 3600 + 60 * i))
        lines.append(json.dumps({"time": t[:-2] + ":" + t[-2:], "agent_id_short": aid, "task": "Fixture task " + aid,
                                 "branch": "worktree-agent-" + aid, "event": ev}))
    (day / "agents.jsonl").write_text("\n".join(lines) + "\n")


class SettingTest(unittest.TestCase):
    def setUp(self):
        self.dir = tmpdir()
        self.t = [1_800_000_000.0]
        self.bm = bm.BuildMachines(self.dir / "config" / bm.FILE_NAME, self.dir / "build-log", clock=lambda: self.t[0])
        self.ms = {m.name: m for m in machines()}

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_default_without_a_file_or_table(self):
        c = self.bm.coordinator()
        self.assertEqual((c["max_agents"], c["source"], c["min"], c["max"]), (5, "default", 1, 16))
        self.bm.update(self.ms["marge"], slots=3)
        self.assertNotIn("\n[coordinator]\n", self.bm.path.read_text(), "no table until the limit is set")
        self.assertEqual(self.bm.coordinator()["source"], "default")

    def test_write_read_and_log(self):
        c = self.bm.set_max_agents(7, by="tester")
        self.assertEqual((c["max_agents"], c["source"], c["changed_by"]), (7, "dashboard", "tester"))
        self.assertTrue(c["changed_at"].startswith("2027-01-15T"))
        self.assertEqual(bm.parse_coordinator(self.bm.path.read_text())["max_agents"], 7, "round trip")
        self.bm.set_max_agents(7)
        self.bm.set_max_agents(3)
        log = self.dir / "build-log" / "2027-01-15" / bm.LOG_NAME
        lines = [json.loads(x) for x in log.read_text().splitlines()]
        self.assertEqual([(x["machine"], x["changes"], x["max_agents"]) for x in lines],
                         [("coordinator", {"max_agents": [5, 7]}, 7), ("coordinator", {}, 7),
                          ("coordinator", {"max_agents": [7, 3]}, 3)])

    def test_range_and_types(self):
        for bad in (0, 17, -1, "5", 5.0, True, None, [5]):
            with self.assertRaises(ValueError, msg=bad):
                self.bm.set_max_agents(bad)
        self.assertFalse(self.bm.path.exists(), "nothing written on a bad value")
        self.assertEqual(self.bm.set_max_agents(1)["max_agents"], 1)
        self.assertEqual(self.bm.set_max_agents(16)["max_agents"], 16)

    def test_round_trip_with_machines_keeps_both(self):
        self.bm.update(self.ms["leela"], enabled=False, reason="kiosk # only", note="Tenerife")
        self.bm.set_max_agents(4)
        self.bm.update(self.ms["marge"], slots=3)
        text = self.bm.path.read_text()
        self.assertTrue(text.startswith(bm.HEADER), "the header comments stay")
        self.assertIn("max_agents", bm.HEADER, "the header explains the table")
        self.assertLess(text.index("\n[coordinator]\n"), text.index("[[machine]]"), "the table comes before the machines")
        self.assertEqual(bm.parse_coordinator(text)["max_agents"], 4, "a machine change keeps the limit")
        e = bm.parse(text)
        self.assertEqual((e["leela"]["enabled"], e["leela"]["reason"], e["leela"]["note"], e["marge"]["slots"]),
                         (False, "kiosk # only", "Tenerife", 3), "the limit change kept the machines")
        import tomllib
        data = tomllib.loads(text)
        self.assertEqual((data["coordinator"]["max_agents"], len(data["machine"])), (4, 2), "valid TOML")

    def test_tolerant_parser_and_hand_edits(self):
        self.bm.set_max_agents(6)
        # A broken line elsewhere makes tomllib fail: the line parser reads the table.
        self.bm.path.write_text(self.bm.path.read_text() + "\n[[machine]]\nname = broken\n")
        self.assertEqual(self.bm.coordinator()["max_agents"], 6)
        for raw, want in (("max_agents = 9", 9), ("max_agents = 0", 5), ("max_agents = 99", 5),
                          ('max_agents = "8"', 5), ("max_agents = true", 5)):
            self.bm.path.write_text("[coordinator]\n%s\n" % raw)
            os.utime(self.bm.path, ns=(time.time_ns(), time.time_ns() + len(raw)))
            self.assertEqual(self.bm.coordinator()["max_agents"], want, raw)
        self.bm.path.write_text("[defaults]\nmax_agents = 9\n[[machine]]\nname = \"marge\"\nmax_agents = 9\n")
        self.assertEqual(self.bm.coordinator()["max_agents"], 5, "only the [coordinator] table counts")

    def test_remote_sh_lookup_skips_the_table(self):
        """remote.sh's awk `field` lookup still finds the machines' keys."""
        self.bm.update(self.ms["leela"], enabled=False, slots=2)
        self.bm.set_max_agents(4)
        awk = r'''
          /^\[\[machine\]\]/ { inm=1; ind=0; cur=""; next }
          /^\[defaults\]/   { ind=1; inm=0; next }
          /^\[/             { inm=0; ind=0 }
          { line=$0; sub(/#.*/, "", line)
            if (match(line, /^[ \t]*[a-z_]+[ \t]*=/)) {
              k=line; sub(/[ \t]*=.*/, "", k); gsub(/[ \t]/, "", k)
              v=line; sub(/^[^=]*=[ \t]*/, "", v); gsub(/^"|"[ \t]*$/, "", v)
              if (inm && k=="name") cur=v
              if (inm && k==key) vals[cur]=v
              if (ind && k==key) def=v } }
          END { if (want in vals) print vals[want]; else print def }'''
        src = (HERE.parent.parent / "scripts" / "remote.sh").read_text()
        self.assertIn("/^\\[\\[machine\\]\\]/ { inm=1; ind=0; cur=\"\"; next }", src, "same lookup as remote.sh")
        look = lambda m, k: subprocess.run(["awk", "-v", "want=" + m, "-v", "key=" + k, awk, str(self.bm.path)],  # noqa: E731
                                           capture_output=True, text=True).stdout.strip()
        self.assertEqual((look("leela", "enabled"), look("leela", "slots"), look("leela", "max_agents")), ("false", "2", ""))

    def test_cli(self):
        cfg = self.dir / "config"
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "machines.toml").write_text(MACHINES)
        run = lambda *a: subprocess.run(["python3", str(HERE / "build_machines.py"), "--config", str(cfg), "--log",  # noqa: E731
                                         str(self.dir / "log")] + list(a), capture_output=True, text=True, timeout=60)
        r = run("agents")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "agents on the coordinator: at most 5 (default)"))
        r = run("agents", "8")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "agents on the coordinator: at most 8 (set in the dashboard)"))
        self.assertEqual(run("agents", "17").returncode, 2)
        self.assertIn("at most 8 agents", run("list").stdout)


class CountTest(unittest.TestCase):
    def test_levels(self):
        self.assertEqual([ct.agent_slots(n, 5)["level"] for n in (0, 4, 5, 6)], ["ok", "ok", "warn", "bad"])
        self.assertEqual(ct.agent_slots(3, 5), {"running": 3, "limit": 5, "free": 2, "level": "ok", "text": "agents 3 of 5"})
        self.assertEqual(ct.agent_slots(7, 5)["free"], 0)
        self.assertEqual(ct.agent_slots(None, 5)["text"], "agents ? of 5")

    def test_counts_running_not_interrupted_or_finished(self):
        log = tmpdir()
        try:
            write_agents(log, [("a1", "started"), ("a2", "started"), ("a3", "started"), ("a2", "reported"),
                               ("a4", "started"), ("a4", "interrupted"), ("a5", "resumed"), ("a6", "merged")])
            s = ct.read_agent_slots(log, 3)
            self.assertEqual((s["running"], s["free"], s["level"]), (3, 0, "warn"), "a1, a3, a5")
            self.assertEqual(ct.read_agent_slots(log / "nothing", 3)["running"], 0)
        finally:
            shutil.rmtree(log)


class StatusTest(unittest.TestCase):
    def setUp(self):
        self.dir = tmpdir()
        self.bm = bm.BuildMachines(self.dir / bm.FILE_NAME, self.dir / "build-log")

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_agents_field(self):
        running = [{"running": True}, {"running": True}, {"running": False, "state": "interrupted"}]
        st = {"claude_tasks": {"running": running}, "coordinator": {"memory": {"available": 6 << 30, "total": 15 << 30}}}
        a = d.add_agents(st, self.bm)["agents"]
        self.assertEqual((a["running"], a["max_agents"], a["free"], a["level"], a["source"], a["mem_available"]),
                         (2, 5, 3, "ok", "default", 6 << 30))
        self.bm.set_max_agents(2)
        a = d.add_agents(st, self.bm)["agents"]
        self.assertEqual((a["text"], a["level"], a["source"]), ("agents 2 of 2", "warn", "dashboard"))
        self.bm.set_max_agents(1)
        self.assertEqual(d.add_agents(st, self.bm)["agents"]["level"], "bad")
        self.assertIn("Agents on the coordinator: agents 2 of 1 (set in the dashboard); 0 free, ABOVE THE LIMIT, 6.0 GiB",
                      d.agents_text(st["agents"]))

    def test_without_tasks_or_settings(self):
        a = d.add_agents({}, None)["agents"]
        self.assertEqual((a["running"], a["max_agents"], a["level"], a["mem_available"]), (None, 5, "", None))


class HttpTest(unittest.TestCase):
    def setUp(self):
        self.dir = tmpdir()
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
        r = urllib.request.Request(self.base + "/api/coordinator", headers=headers, method="POST",
                                   data=raw if raw is not None else json.dumps(body).encode())
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    def status(self):
        with urllib.request.urlopen(self.base + "/api/status", timeout=10) as resp:
            return json.loads(resp.read())

    def test_status_has_the_limit(self):
        a = self.status()["agents"]
        self.assertEqual((a["max_agents"], a["source"], a["min"], a["max"]), (5, "default", 1, 16))

    def test_token_and_validation(self):
        self.assertEqual(self.post({"max_agents": 3})[0], 403)
        self.assertEqual(self.post({"max_agents": 3}, "wrong")[0], 403)
        t = self.dash.token
        for body in ({"max_agents": 0}, {"max_agents": 17}, {"max_agents": "3"}, {"max_agents": 3.5},
                     {"max_agents": True}, {}, ["max_agents"]):
            self.assertEqual(self.post(body, t)[0], 400, body)
        self.assertEqual(self.post(None, t, raw=b"{nope")[0], 400)
        self.assertFalse(self.dash.build_machines.path.exists(), "nothing written")
        code, res = self.post({"max_agents": 3}, t)
        self.assertEqual((code, res["settings"]["max_agents"]), (200, 3))
        self.assertNotIn(t, self.dash.build_machines.path.read_text())
        a = self.status()["agents"]
        self.assertEqual((a["max_agents"], a["source"]), (3, "dashboard"))

    def test_read_only(self):
        self.dash.build_machines.read_only = True
        self.assertEqual(self.post({"max_agents": 3}, self.dash.token)[0], 404)


@unittest.skipUnless(shutil.which("node"), "node not installed")
class PageTest(unittest.TestCase):
    """Renders the coordinator card and the Claude tasks in node with a fake DOM."""

    def render(self, agents, settings=True):
        page = (HERE / "index.html").read_text(encoding="utf-8")
        script = page.split("<script>", 1)[1].split("</script>", 1)[0]
        st = {"generated_at": 100, "machines": [], "machine_settings": settings, "agents": agents,
              "overview": {"machines": 0, "online": 0, "tasks": 0, "waiting": 0, "slots_used": 0, "slots_total": 0},
              "coordinator": {"name": "deb", "available": True, "at": 90, "processes": {"processes": 10},
                              "memory": {"total": 15 << 30, "used": 9 << 30, "available": 6 << 30}},
              "claude_tasks": {"running": [], "recent": [], "worktrees": [], "worktree_counts": {}}}
        harness = """
const els = {};
global.document = { getElementById: (id) => (els[id] ||= { innerHTML: "", textContent: "", style: {}, addEventListener() {} }) };
global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
global.setInterval = () => 0; global.fetch = async () => ({ json: async () => ({}) });
""" + script + "\nrender(%s); process.stdout.write(els.coord.innerHTML + '\\n@@\\n' + els.ctasks.innerHTML);\n" % json.dumps(st)
        d_ = tmpdir()
        try:
            f = d_ / "page.js"
            f.write_text(harness)
            out = subprocess.run(["node", str(f)], capture_output=True, text=True, timeout=30)
        finally:
            shutil.rmtree(d_)
        self.assertEqual(out.returncode, 0, out.stderr[-2000:])
        return out.stdout.split("\n@@\n")

    def agents(self, running, limit, **kw):
        a = dict(max_agents=limit, source="dashboard", min=1, max=16, changed_at="2026-11-16T15:00:00+01:00",
                 mem_available=6 << 30, **ct.agent_slots(running, limit))
        a.update(kw)
        return a

    def test_levels_and_memory(self):
        coord, tasks = self.render(self.agents(3, 5))
        self.assertIn("agents 3 of 5 – 2 free", coord)
        self.assertIn("6.0 GiB memory available", coord)
        self.assertIn('class="mctl agents "', coord)
        self.assertIn("agents 3 of 5</span>", tasks)
        coord, tasks = self.render(self.agents(5, 5))
        self.assertIn('class="mctl agents warn"', coord)
        self.assertIn("at the limit", coord)
        self.assertIn('class="badge warn"', tasks)
        coord, tasks = self.render(self.agents(6, 5))
        self.assertIn('class="mctl agents bad"', coord)
        self.assertIn("above the limit", coord)
        self.assertIn('class="badge bad"', tasks)

    def test_stepper_with_token_controls_only(self):
        coord, _ = self.render(self.agents(1, 16))
        self.assertIn('data-act="agents-"', coord)
        self.assertIn('aria-label="More agents" disabled', coord, "16 is the maximum")
        coord, _ = self.render(self.agents(0, 1))
        self.assertIn('aria-label="Fewer agents" disabled', coord, "1 is the minimum")
        coord, _ = self.render(self.agents(1, 5), settings=False)
        self.assertNotIn('data-act="agents', coord)
        self.assertIn("agents 1 of 5", coord)


class ScriptTest(unittest.TestCase):
    """scripts/agent-slots.sh with fixture files (OCW_AGENT_CONFIG / OCW_AGENT_LOG)."""

    def setUp(self):
        self.dir = tmpdir()
        self.cfg, self.log = self.dir / "config", self.dir / "build-log"
        self.cfg.mkdir()

    def tearDown(self):
        shutil.rmtree(self.dir)

    def run_script(self, *args):
        env = dict(os.environ, OCW_AGENT_CONFIG=str(self.cfg), OCW_AGENT_LOG=str(self.log))
        return subprocess.run([str(SCRIPT)] + list(args), capture_output=True, text=True, timeout=60, env=env)

    def test_free_slot_exit_0(self):
        write_agents(self.log, [("a1", "started"), ("a2", "started"), ("a2", "reported")])
        r = self.run_script()
        self.assertEqual((r.returncode, r.stdout), (0, "limit 5 (default)\nrunning 1\nfree 4\n"), r.stderr)

    def test_full_exit_3(self):
        bm.BuildMachines(self.cfg / bm.FILE_NAME).set_max_agents(2)
        write_agents(self.log, [("a1", "started"), ("a2", "resumed")])
        r = self.run_script()
        self.assertEqual((r.returncode, r.stdout), (3, "limit 2 (dashboard)\nrunning 2\nfree 0\n"), r.stderr)
        write_agents(self.log, [("a1", "started"), ("a2", "started"), ("a3", "started")])
        r = self.run_script("-q")
        self.assertEqual((r.returncode, r.stdout), (3, ""), "above the limit, quiet")

    def test_no_files_and_usage(self):
        r = self.run_script()
        self.assertEqual((r.returncode, r.stdout), (0, "limit 5 (default)\nrunning 0\nfree 5\n"), r.stderr)
        self.assertEqual(self.run_script("--bogus").returncode, 2)

    def test_module_check_matches(self):
        bm.BuildMachines(self.cfg / bm.FILE_NAME).set_max_agents(4)
        write_agents(self.log, [("a1", "started")])
        s = agent_slots.check(self.cfg / bm.FILE_NAME, self.log)
        self.assertEqual((s["max_agents"], s["running"], s["free"], s["level"]), (4, 1, 3, "ok"))


if __name__ == "__main__":
    unittest.main()
