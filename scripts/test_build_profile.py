#!/usr/bin/env python3
"""Tests of scripts/build_profile.py: python3 scripts/test_build_profile.py"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True  # no __pycache__ in scripts/
sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_profile as bp  # noqa: E402

LOG = """remote.sh: waiting for a free build slot
remote.sh: got build slot 2 after 95 s
   Compiling proc-macro2 v1.0.101
   Compiling ocw-world v0.2.0 (/home/x/src/OCW/crates/ocw-world)
    Checking ocw-sim v0.2.0 (/home/x/src/OCW/crates/ocw-sim)
    Finished `dev` profile [optimized + debuginfo] target(s) in 1m 23s
test result: ok. 84 passed; 0 failed
    Finished `test` profile [optimized + debuginfo] target(s) in 4.56s
     Running unittests src/lib.rs
"""

HTML = """<html><script>
const UNIT_DATA = [
  {"i":0,"name":"bevy_render","version":"0.20.1","mode":"todo","target":"","start":1.0,"duration":40.5},
  {"i":1,"name":"webrtc-sys","version":"0.3","mode":"run-custom-build","target":" build script (run)","start":0.5,"duration":12.0},
  {"i":2,"name":"ocw-render","version":"0.2.0","mode":"todo","target":"","start":50.0,"duration":20.25},
  {"i":3,"name":"ocw-render","version":"0.2.0","mode":"todo","target":"","start":80.0,"duration":0.75}
];
const CONCURRENCY_DATA = [];
</script></html>"""


class CargoLog(unittest.TestCase):
    def test_finished_compiling_and_wait(self):
        r = bp.cargo_log(LOG)
        self.assertEqual(r, {"cargo_s": 87.6, "cargo_builds": 2, "compiled": 3, "wait_s": 95})

    def test_quiet_output_gives_nulls(self):
        self.assertEqual(bp.cargo_log("test result: ok\n"),
                         {"cargo_s": None, "cargo_builds": 0, "compiled": 0, "wait_s": None})

    def test_json_line_for_remote_sh(self):
        # remote.sh strips the braces and appends the fields to runs.jsonl.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "run.log"
            p.write_text(LOG)
            from io import StringIO
            from contextlib import redirect_stdout
            out = StringIO()
            with redirect_stdout(out):
                self.assertEqual(bp.main(["cargo-log", str(p)]), 0)
            line = out.getvalue().strip()
            self.assertTrue(line.startswith("{") and line.endswith("}"))
            row = json.loads('{"seconds":1,' + line[1:])
            self.assertEqual(row["compiled"], 3)


class Timings(unittest.TestCase):
    def test_units_and_slowest(self):
        us = bp.units(HTML)
        self.assertEqual(len(us), 4)
        top = bp.slowest(us, 2)
        self.assertEqual(top[0], ("bevy_render", 40.5))
        # The same crate's units are summed.
        self.assertEqual(top[1], ("ocw-render", 21.0))
        self.assertEqual(bp.units("<html>no data</html>"), [])

    def test_durations(self):
        self.assertEqual(bp.fmt_s(4.0), "4.0s")
        self.assertEqual(bp.fmt_s(83.0), "1m 23.0s")
        self.assertEqual(bp.fmt_s(None), "–")


class Reports(unittest.TestCase):
    def write(self, d: Path, rows):
        d.mkdir(parents=True)
        (d / "results.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))

    def test_summary_and_compare(self):
        with tempfile.TemporaryDirectory() as t:
            a, b = Path(t) / "before", Path(t) / "after"
            row = {"label": "before", "command": "build", "step": "clean", "seconds": 600.0,
                   "compiled": 700, "exit": 0, "load": 1.5, "target_mb": 20480}
            self.write(a, [row, dict(row, step="noop", seconds=2.0, compiled=0)])
            self.write(b, [dict(row, label="after", seconds=450.0)])
            (a / "build-clean.html").write_text(HTML)
            s = bp.summary(a)
            self.assertIn("# Build profile: before", s)
            self.assertIn("| build | clean | 10m 00.0s | 700 | 0 | 1.5 | 20.0 GB |", s)
            self.assertIn("| build | noop | 2.0s | 0 | 0 | 1.5 | 20.0 GB |", s)
            self.assertIn("- bevy_render: 40.5s", s)
            c = bp.compare(a, b)
            self.assertIn("| build | clean | 10m 00.0s | 7m 30.0s | -25% |", c)
            # Only steps measured in both.
            self.assertNotIn("noop", c)


if __name__ == "__main__":
    unittest.main()
