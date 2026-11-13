"""Tests of answering the open questions on the dashboard (answers.py and
the HTTP API of dashboard.py; no network beyond a loopback server)."""

import json
import os
import shutil
import stat
import subprocess
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import answers
import dashboard as d
import spec_questions

HERE = Path(__file__).resolve().parent
EVIL = '<img src=x onerror="alert(1)"><script>alert(2)</script>'
SPEC = ("Version 2.63 – November 2026\n\n### Phase 12 – Collisions\n\n#### 12J – Open questions (for the user)\n\n"
        "- **Q1 – Defaults:** collisions on by default (proposal), or both off?\n"
        "- **Q2 – Bots " + EVIL + ":** perfect " + EVIL + ", or chaotic?\n"
        "- **Q3 – Falls: decided (2026-11-13):** scripted.\n")


class StoreTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.t = [1_800_000_000.0]
        self.store = answers.AnswerStore(self.dir / "answers", clock=lambda: self.t[0])
        self.qs = {q["id"]: q for q in spec_questions.parse_questions(SPEC)}

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_round_trip_with_time_id_and_exact_question(self):
        q = self.qs["Phase 12 Q1"]
        e = self.store.add(q, "  Accept proposal: collisions on by default  ", "collisions on by default", "2.63")
        self.assertEqual((e["id"], e["answer"], e["question"], e["spec_version"]),
                         ("Phase 12 Q1", "Accept proposal: collisions on by default", q["text"], "2.63"))
        self.assertRegex(e["time"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$")
        lines = (self.dir / "answers" / "answers.jsonl").read_text().splitlines()
        self.assertEqual(json.loads(lines[0])["answer"], e["answer"])
        inbox = (self.dir / "answers" / "inbox.log").read_text().splitlines()
        self.assertEqual(len(inbox), 1)
        self.assertIn("Phase 12 Q1 (Defaults): Accept proposal", inbox[0])

    def test_a_new_answer_supersedes_and_since_filters(self):
        q = self.qs["Phase 12 Q1"]
        self.store.add(q, "on")
        self.t[0] += 60
        e2 = self.store.add(q, "off\nafter all")
        self.assertEqual(self.store.latest()["Phase 12 Q1"]["answer"], "off\nafter all")
        self.assertIsNotNone(e2["supersedes"])
        self.assertEqual(len(self.store.all()), 2, "append-only")
        self.assertEqual([x["answer"] for x in self.store.since(self.t[0] - 30)], ["off\nafter all"])
        inbox = (self.dir / "answers" / "inbox.log").read_text().splitlines()
        self.assertEqual(len(inbox), 2)
        self.assertIn("off / after all [replaces the answer of", inbox[1], "one line per answer")

    def test_bad_answers(self):
        q = self.qs["Phase 12 Q1"]
        for bad in ("", "   ", None, 5, "x" * (answers.MAX_ANSWER + 1)):
            with self.assertRaises(ValueError):
                self.store.add(q, bad)
        self.assertEqual(answers.clean_answer("a\x00b\x1b[31m\r\nc"), "ab[31m\nc")

    def test_states_answered_then_decided(self):
        text = SPEC
        self.store.add(self.qs["Phase 12 Q1"], "on")
        s = answers.annotate(spec_questions.summary(spec_questions.parse_questions(text)), self.store.latest())
        st = {it["id"]: it["answer_state"] for g in s["groups"] for it in g["items"]}
        self.assertEqual(st, {"Phase 12 Q1": "answered", "Phase 12 Q2": "open", "Phase 12 Q3": "decided"})
        self.assertEqual(s["answered"], 1)
        # The coordinator records it in the spec: the parser shows it decided.
        text = text.replace("- **Q1 – Defaults:**", "- **Q1 – Defaults: decided (2026-11-14):**")
        s = answers.annotate(spec_questions.summary(spec_questions.parse_questions(text)), self.store.latest())
        it = s["groups"][0]["items"][0]
        self.assertEqual((it["answer_state"], it["answer"]["answer"], s["answered"]), ("decided", "on", 0))

    def test_token_file_created_0600_and_kept(self):
        p = self.dir / "config" / "dashboard-token"
        t1 = answers.load_token(p)
        self.assertGreaterEqual(len(t1), 32)
        self.assertEqual(stat.S_IMODE(os.stat(p).st_mode), 0o600)
        self.assertEqual(answers.load_token(p), t1, "not replaced on the next start")
        self.assertTrue(answers.token_ok(t1, t1))
        self.assertFalse(answers.token_ok(t1, None))
        self.assertFalse(answers.token_ok(t1, t1[:-1]))
        self.assertFalse(answers.token_ok("", ""))

    def test_options(self):
        self.assertEqual(self.qs["Phase 12 Q1"]["options"], ["collisions on by default", "both off"])
        self.assertEqual(self.qs["Phase 12 Q3"]["options"], [])


class HttpTest(unittest.TestCase):
    """The API on a loopback server: POST needs the token, GET stays open."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        (self.dir / "SPECIFICATION.md").write_text(SPEC, encoding="utf-8")
        dash = d.Dashboard([], use_control=False)
        dash.questions = spec_questions.SpecQuestions(self.dir / "SPECIFICATION.md")
        dash.answers = answers.AnswerStore(self.dir / "answers")
        dash.token = answers.load_token(self.dir / "dashboard-token")
        self.dash = dash
        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), d.make_handler(dash))
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.base = "http://127.0.0.1:%d" % self.srv.server_address[1]

    def tearDown(self):
        self.srv.shutdown()
        self.srv.server_close()
        shutil.rmtree(self.dir)

    def req(self, path, body=None, token=None):
        headers = {"Content-Type": "application/json"}
        if token is not None:
            headers[answers.TOKEN_HEADER] = token
        r = urllib.request.Request(self.base + path, data=json.dumps(body).encode() if body is not None else None,
                                   headers=headers, method="POST" if body is not None else "GET")
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    def test_token_required(self):
        body = {"id": "Phase 12 Q1", "answer": "on"}
        self.assertEqual(self.req("/api/answers", body)[0], 403)
        self.assertEqual(self.req("/api/answers", body, token="wrong")[0], 403)
        self.assertEqual(self.dash.answers.all(), [], "nothing stored without the token")
        code, res = self.req("/api/answers", body, token=self.dash.token)
        self.assertEqual((code, res["answer"]["id"]), (200, "Phase 12 Q1"))
        self.assertNotIn(self.dash.token, (self.dir / "answers" / "answers.jsonl").read_text())
        self.assertNotIn(self.dash.token, (self.dir / "answers" / "inbox.log").read_text())

    def test_unknown_question_and_bad_body(self):
        self.assertEqual(self.req("/api/answers", {"id": "Phase 99 Q1", "answer": "x"}, self.dash.token)[0], 400)
        self.assertEqual(self.req("/api/answers", {"id": "Phase 12 Q1", "answer": ""}, self.dash.token)[0], 400)
        self.assertEqual(self.req("/api/answers", ["not", "an", "object"], self.dash.token)[0], 400)

    def test_round_trip_since_and_status(self):
        code, first = self.req("/api/answers", {"id": "Phase 12 Q2", "answer": EVIL}, self.dash.token)
        self.assertEqual(code, 200)
        code, res = self.req("/api/answers?since=0")
        self.assertEqual((code, [a["answer"] for a in res["answers"]]), (200, [EVIL]))
        self.assertEqual(self.req("/api/answers?since=%f" % (first["answer"]["epoch"] + 1))[1]["answers"], [])
        self.assertEqual(self.req("/api/answers?since=abc")[0], 400)
        st = self.dash.snapshot()
        it = st["questions"]["groups"][0]["items"][1]
        self.assertEqual((it["answer_state"], it["answer"]["answer"]), ("answered", EVIL))
        self.assertTrue(st["questions"]["answers_enabled"])
        self.assertNotIn("token", json.dumps(st).lower().replace("dashboard-token", ""))


@unittest.skipUnless(shutil.which("node"), "node not installed")
class XssTest(unittest.TestCase):
    """Renders the page's questions panel in node with a fake DOM: question
    and answer text containing HTML must come out escaped."""

    def test_question_and_answer_text_escaped(self):
        qs = answers.annotate(spec_questions.summary(spec_questions.parse_questions(SPEC)),
                              {"Phase 12 Q2": {"time": "t", "epoch": 1, "answer": EVIL, "choice": None}})
        qs.update(answers_enabled=True, mtime=1)
        page = (HERE / "index.html").read_text(encoding="utf-8")
        script = page.split("<script>", 1)[1].split("</script>", 1)[0]
        harness = """
const els = {};
global.document = { getElementById: (id) => (els[id] ||= { innerHTML: "", textContent: "", style: {}, addEventListener() {} }) };
global.localStorage = { getItem: (k) => k === "ocw-answer-token" ? "tok" : null, setItem() {}, removeItem() {} };
global.setInterval = () => 0; global.fetch = async () => ({ json: async () => ({}) });
""" + script + "\nrenderQuestions(%s, true); process.stdout.write(els.questions.innerHTML);\n" % json.dumps(qs)
        with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
            f.write(harness)
        try:
            out = subprocess.run(["node", f.name], capture_output=True, text=True, timeout=30)
        finally:
            os.unlink(f.name)
        self.assertEqual(out.returncode, 0, out.stderr[-2000:])
        html = out.stdout
        self.assertIn("answered – waiting for the coordinator", html)
        self.assertIn("&lt;script&gt;", html)
        self.assertNotIn("<script", html)
        self.assertNotIn("<img", html)
        self.assertIn('data-act="send"', html)


if __name__ == "__main__":
    unittest.main()
