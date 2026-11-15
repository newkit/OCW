"""Unit tests of the art project's tool (art/ART-PROJECT.md): masking of the
tracked text files, check-masked, the collector's days, the lock, the status
file and scripts/art-collect.sh. A small fixture archive in a temporary
directory; no real archive, no network, no ffmpeg.

    /usr/bin/python3 -m unittest discover -s art -p 'test_*.py'   (make test-art)
"""

from __future__ import annotations

import datetime as dt
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import diary  # noqa: E402

# Fake build machines: (value, replacement) as diary._secrets() returns them.
SECRETS = [("buildhost.example.lan", "marge"), ("10.20.30.40", "leela"), ("builder", "user"),
           ("/home/devuser", "~")]
PRIVATE = ["buildhost.example.lan", "10.20.30.40", "builder@", "/home/devuser", "/home/builder",
           "dev@mail.example.org", "192.168.7.9"]
DAY = "2026-11-14"


def tree_state(root: Path) -> dict:
    """{relative path: (size, mtime_ns)} of every file and folder below root."""
    if not root.exists():
        return {}
    out = {}
    for p in [root, *root.rglob("*")]:
        st = p.lstat()
        out[p.relative_to(root).as_posix()] = (st.st_size if p.is_file() else None, st.st_mtime_ns)
    return out


# The real archive at the default place (the main checkout has one): the tests
# must never touch it. They hold its lock meanwhile (the hourly collector
# waits), and compare every file's size and mtime before and after.
REAL_HIST = diary.ROOT / "art" / "build-history"
_real = {"lock": None, "state": None}


def setUpModule():
    if not REAL_HIST.is_dir():
        return
    import fcntl
    lock = REAL_HIST / ".lock"
    if lock.exists():  # (no lock file: creating one would touch the archive)
        f = open(lock, "r")
        try:
            fcntl.flock(f, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            f.close()
            return  # a collector or diary run is busy: no stable state to compare
        _real["lock"] = f
    _real["state"] = tree_state(REAL_HIST)


def tearDownModule():
    if _real["state"] is None:
        return
    try:
        after = tree_state(REAL_HIST)
        changed = sorted(k for k in set(after) | set(_real["state"]) if after.get(k) != _real["state"].get(k))
        if changed:
            raise AssertionError(f"the tests changed the real archive {REAL_HIST}: {changed[:10]}")
    finally:
        if _real["lock"] is not None:
            _real["lock"].close()


class Fixture(unittest.TestCase):
    """A temporary archive and test-output, the fake secrets."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ocw-art-test-"))
        self.hist = self.tmp / "build-history"
        self.out = self.tmp / "test-output"
        self.hist.mkdir()
        self.out.mkdir()
        self._saved = (diary.HIST, diary.TEST_OUTPUT, diary._SECRETS, diary.TRANSCRIPTS, diary.WORKTREES)
        diary.set_history(self.hist)
        diary.TEST_OUTPUT = self.out
        diary.TRANSCRIPTS = str(self.tmp / "no-transcripts")
        diary.WORKTREES = None  # never the real agent worktrees' test-output
        diary._SECRETS = sorted(SECRETS, key=lambda kv: -len(kv[0]))
        self.env = mock.patch.dict(os.environ, {"OCW_TRANSCRIPTS": str(self.tmp / "no-transcripts")})
        self.env.start()
        os.environ.pop("OCW_ART_LOCK_HELD", None)

    def tearDown(self):
        self.env.stop()
        diary.set_history(self._saved[0])
        diary.TEST_OUTPUT, diary._SECRETS, diary.TRANSCRIPTS, diary.WORKTREES = self._saved[1:]
        subprocess.run(["rm", "-rf", "--", str(self.tmp)], check=False)

    def assert_clean(self, text: str):
        for p in PRIVATE:
            self.assertNotIn(p, text)


class MaskTest(Fixture):
    def test_mask_hosts_users_homes_mails_ips(self):
        t = ("ssh builder@buildhost.example.lan; ping 10.20.30.40; cd /home/devuser/src; "
             "ls /home/builder/src/OCW; mail dev@mail.example.org; 192.168.7.9; C:\\Users\\builder\\x")
        m = diary.mask(t)
        self.assert_clean(m)
        self.assertIn("user@marge", m)
        self.assertIn("~/src/OCW", m)
        self.assertIn("<e-mail>", m)
        self.assertIn("<ip>", m)

    def test_mask_keeps_public_names(self):
        t = "~/.builder/worktrees builder.ai icon@2x.png react@18.3.1 https://x.org/home/page"
        self.assertEqual(diary.mask(t), t)
        self.assertEqual(diary.private_findings(t), [])

    def test_mask_is_idempotent(self):
        t = "builder@buildhost.example.lan /home/devuser/x 10.20.30.40"
        self.assertEqual(diary.mask(diary.mask(t)), diary.mask(t))
        self.assertEqual(diary.private_findings(diary.mask(t)), [])

    def test_tracked_files_written_masked(self):
        """journal.md, journal.json and tokens.json land masked on disk."""
        commits = [{"sha": "abc1234", "time": "10:00:00", "merge": False, "added": 3, "removed": 1,
                    "subject": "Run on buildhost.example.lan in /home/builder/src/OCW (dev@mail.example.org)"}]
        tokens = {"day": DAY, "total": {"total": 10, "messages": 1, "input": 1, "cache_write": 2,
                                        "cache_read": 3, "output": 4},
                  "tasks": {"agent: fix /home/devuser/x on 10.20.30.40": {"feature": "Other", "total": 10,
                                                                          "output": 4, "messages": 1}},
                  "features": {"Other": {"total": 10, "output": 4, "messages": 1}}, "models": {}}
        with mock.patch.object(diary, "commits_of", return_value=commits), \
                mock.patch.object(diary, "tokens_of", return_value=tokens), \
                mock.patch.object(diary, "all_days", return_value=["2026-11-11", DAY]):
            j = diary.journal(DAY)
        self.assertEqual(j["number"], 2)
        for name in diary.TRACKED_DAY:
            text = (self.hist / DAY / name).read_text()
            self.assert_clean(text)
        self.assertIn("~/src/OCW", (self.hist / DAY / "journal.md").read_text())
        json.loads((self.hist / DAY / "journal.json").read_text())  # still valid JSON
        self.assertEqual(diary.check_masked(), [])

    def test_day_number_without_commits_yet(self):
        with mock.patch.object(diary, "all_days", return_value=["2026-11-11", "2026-11-12"]):
            self.assertEqual(diary.day_number("2026-11-12"), 2)
            self.assertEqual(diary.day_number("2026-11-13"), 3)


class CheckMaskedTest(Fixture):
    def write(self, rel, text):
        p = self.hist / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
        return p

    def test_finds_private_patterns_in_tracked_files_only(self):
        self.write("index.md", "# Build history\n\nfrom /home/builder/src/OCW\n")
        self.write(f"{DAY}/journal.json", '{"x": "ssh builder@buildhost.example.lan"}\n')
        self.write(f"{DAY}/tokens.json", "{}\n")
        self.write("SHA256SUMS", "00  ocw-build-history-2026-11-14.zip\n")
        # Untracked text keeps the originals: not checked.
        self.write(f"{DAY}/logs/run.log", "10.20.30.40 /home/devuser\n")
        self.write(f"{DAY}/prompts.md", "dev@mail.example.org\n")
        found = diary.check_masked()
        self.assertIn("index.md:3: home directory", found)
        self.assertTrue(any(f.startswith(f"{DAY}/journal.json:1: ") for f in found))
        self.assertFalse(any("logs" in f or "prompts" in f for f in found))
        for f in found:  # the values are never reported
            self.assert_clean(f)

    def test_fix_masks_in_place(self):
        p = self.write("index.md", "from /home/builder/src/OCW on 10.20.30.40\n")
        log_ = self.write(f"{DAY}/logs/run.log", "10.20.30.40\n")
        self.assertTrue(diary.check_masked())
        self.assertEqual(diary.check_masked(fix=True), [])
        self.assertEqual(p.read_text(), "from ~/src/OCW on leela\n")
        self.assertEqual(log_.read_text(), "10.20.30.40\n")  # originals stay
        self.assertEqual(diary.check_masked(), [])

    def test_cli_exit_code(self):
        self.write("index.md", "cd /home/someone/x\n")
        env = {**os.environ, "OCW_ART_HISTORY": str(self.hist)}
        r = subprocess.run([sys.executable, str(HERE / "diary.py"), "check-masked"], env=env,
                           capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("index.md:1: home directory", r.stdout)
        self.assertNotIn("/home/someone", r.stdout + r.stderr)
        r = subprocess.run([sys.executable, str(HERE / "diary.py"), "check-masked", "--fix"], env=env,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)


class DaysTest(unittest.TestCase):
    def test_today_and_yesterday(self):
        self.assertEqual(diary.default_collect_days(dt.datetime(2026, 11, 14, 9, 5)),
                         ["2026-11-13", "2026-11-14"])

    def test_month_and_year_boundary(self):
        self.assertEqual(diary.default_collect_days(dt.datetime(2026, 12, 1, 0, 3)),
                         ["2026-11-30", "2026-12-01"])
        self.assertEqual(diary.default_collect_days(dt.datetime(2027, 1, 1, 0, 1)),
                         ["2026-12-31", "2027-01-01"])


class LockTest(Fixture):
    def test_second_run_waits_and_times_out(self):
        with diary.art_lock("first"):
            self.assertIn("diary.py first", diary.lock_path().read_text())
            t0 = time.monotonic()
            with self.assertRaises(TimeoutError):
                with diary.art_lock("second", wait=0.3):
                    pass
            self.assertGreaterEqual(time.monotonic() - t0, 0.3)
        with diary.art_lock("third", wait=0.3):  # free again
            pass

    def test_lock_held_by_the_collector_script(self):
        with diary.art_lock("collector"):
            with mock.patch.dict(os.environ, {"OCW_ART_LOCK_HELD": "1"}):
                with diary.art_lock("child", wait=0.1):  # no deadlock
                    pass

    def test_lock_excludes_another_process(self):
        holder = subprocess.Popen(["flock", str(diary.lock_path()), "sleep", "5"])
        try:
            deadline = time.monotonic() + 3
            while not diary.lock_path().exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            time.sleep(0.2)
            with self.assertRaises(TimeoutError):
                with diary.art_lock("x", wait=0.3):
                    pass
        finally:
            holder.kill()
            holder.wait()


class OldPlaceTest(Fixture):
    def test_refuses_while_archive_at_old_place(self):
        old = self.tmp / "artwork" / "build-history"
        (old / DAY).mkdir(parents=True)
        with mock.patch.object(diary, "OLD_HIST", old), mock.patch.dict(os.environ):
            os.environ.pop("OCW_ART_HISTORY", None)
            with self.assertRaises(SystemExit) as e:
                diary.check_old_place()
            self.assertIn("art/build-history", str(e.exception))
            self.assertIn("mv artwork/build-history art/build-history", str(e.exception))
            (self.hist / DAY).mkdir()
            with self.assertRaises(SystemExit) as e:
                diary.check_old_place()
            self.assertIn("both", str(e.exception))
            subprocess.run(["rm", "-rf", "--", str(old)], check=True)
            diary.check_old_place()  # moved: fine


class CollectTest(Fixture):
    def fake_gather(self, day):
        if day == "2026-11-13":
            raise RuntimeError("disk full at /home/builder/x")
        (self.hist / day).mkdir(parents=True, exist_ok=True)
        (self.hist / day / "120000-shot.png").write_bytes(b"x")
        (self.hist / day / "120100-clip.mp4").write_bytes(b"x")
        return 2

    def test_status_file(self):
        now = dt.datetime(2026, 11, 14, 15, 2)
        with mock.patch.object(diary, "gather", side_effect=self.fake_gather), \
                mock.patch.object(diary, "journal") as journal, \
                mock.patch.object(diary, "sheet") as sheet:
            st = diary.collect(now=now)
        self.assertEqual([c.args[0] for c in journal.call_args_list], [DAY])  # the failed day: no journal
        sheet.assert_not_called()  # not the 23:xx run
        status = json.loads((self.out / "build-log" / DAY / "art.json").read_text())
        self.assertEqual(status, st)
        self.assertEqual(status["status"], "failed")
        self.assertEqual(status["days"][DAY], {"new_files": 2, "media": 2})
        self.assertEqual(status["new_files"], 2)
        self.assertEqual(status["media"], 2)
        self.assertEqual(status["mask_check"], "ok")
        self.assertTrue(status["start"] and status["end"])
        self.assertEqual(len(status["errors"]), 1)
        self.assertIn("2026-11-13", status["errors"][0])
        self.assert_clean(json.dumps(status))  # errors are masked too
        runs = (self.out / "build-log" / DAY / "art-runs.jsonl").read_text().splitlines()
        self.assertEqual(json.loads(runs[-1])["status"], "failed")

    def test_sheets_in_the_23h_run_and_mask_failure(self):
        (self.hist / "index.md").write_text("ssh builder@buildhost.example.lan\n")
        with mock.patch.object(diary, "gather", return_value=0), \
                mock.patch.object(diary, "journal"), mock.patch.object(diary, "sheet") as sheet:
            st = diary.collect(now=dt.datetime(2026, 11, 14, 23, 3))
        sheet.assert_called_once_with(DAY)
        self.assertTrue(st["sheets"])
        self.assertEqual(st["status"], "failed")
        self.assertTrue(st["mask_check"].startswith("failed: "))


class CollectScriptTest(Fixture):
    """scripts/art-collect.sh on the fixture (OCW_ART_HISTORY, OCW_TEST_OUTPUT)."""

    SCRIPT = HERE.parent / "scripts" / "art-collect.sh"

    def script_env(self, **kw):
        return {**os.environ, "OCW_ART_HISTORY": str(self.hist), "OCW_TEST_OUTPUT": str(self.out),
                "OCW_TRANSCRIPTS": str(self.tmp / "no-transcripts"), "OCW_ART_PYTHON": sys.executable, **kw}

    def test_locked_run_gives_up(self):
        today = dt.date.today().isoformat()
        lock = self.hist / ".lock"
        holder = subprocess.Popen(["flock", str(lock), "sleep", "10"])
        try:
            time.sleep(0.3)
            r = subprocess.run([str(self.SCRIPT), "--no-sheets"], env=self.script_env(OCW_ART_LOCK_WAIT="1"),
                               capture_output=True, text=True, timeout=60)
        finally:
            holder.kill()
            holder.wait()
        self.assertEqual(r.returncode, 75, r.stderr)
        st = json.loads((self.out / "build-log" / today / "art.json").read_text())
        self.assertEqual(st["status"], "locked")

    def test_full_run(self):
        """gather + journal of a fixture day: status ok, media counted, log."""
        day = "2026-11-14"
        src = self.out / day
        src.mkdir(parents=True)
        (src / "101500-shot.jpg").write_bytes(b"\xff\xd8 fixture")
        (src / "101600-verdict.txt").write_text("ran on /home/builder/src/OCW\n")
        r = subprocess.run([str(self.SCRIPT), "--no-sheets", day], env=self.script_env(),
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        today = dt.date.today().isoformat()
        st = json.loads((self.out / "build-log" / today / "art.json").read_text())
        self.assertEqual(st["status"], "ok")
        self.assertEqual(st["days"][day]["media"], 1)
        self.assertGreaterEqual(st["new_files"], 2)
        self.assertEqual(st["mask_check"], "ok")
        self.assertIn("art-collect", (self.out / "build-log" / today / "art.log").read_text())
        self.assertTrue((self.hist / day / "101500-shot.jpg").exists())
        self.assertTrue((self.hist / day / "journal.json").exists())
        # A second run finds nothing new (sources deduplicated by SHA-256).
        r = subprocess.run([str(self.SCRIPT), "--no-sheets", day], env=self.script_env(),
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stderr)
        st = json.loads((self.out / "build-log" / today / "art.json").read_text())
        self.assertEqual(st["days"][day]["new_files"], 0)
        self.assertEqual(st["days"][day]["media"], 1)


class DefaultArchiveUntouchedTest(Fixture):
    """The collector run with OCW_ART_HISTORY / OCW_TEST_OUTPUT, in a copy of
    the repository that has a real-looking archive at the default place
    (art/build-history/), its own test-output/ and an agent worktree with
    media of the day: none of these may be read into or written by the run."""

    def make_repo(self) -> Path:
        repo = self.tmp / "repo"
        (repo / "art").mkdir(parents=True)
        (repo / "scripts").mkdir()
        for rel in ("art/diary.py", "art/music.py", "scripts/art-collect.sh"):
            (repo / rel).write_bytes((HERE.parent / rel).read_bytes())
            (repo / rel).chmod(0o755)
        env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.org",
               "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.org",
               "GIT_AUTHOR_DATE": f"{DAY}T10:00:00", "GIT_COMMITTER_DATE": f"{DAY}T10:00:00"}
        for cmd in (["init", "-q"], ["add", "art/diary.py", "art/music.py", "scripts/art-collect.sh"],
                    ["commit", "-q", "-m", "fixture"]):
            subprocess.run(["git", "-C", str(repo), *cmd], env=env, check=True, capture_output=True)
        # The real-looking archive at the default place, with its lock file.
        real = repo / "art" / "build-history"
        (real / DAY / "logs" / "build-log").mkdir(parents=True)
        for name, data in (("index.md", "# Build history\n"), (".lock", ""), ("SENTINEL", "do not touch\n"),
                           (f"{DAY}/090000-real-shot.jpg", "jpeg"), (f"{DAY}/journal.json", "{}\n"),
                           (f"{DAY}/logs/build-log/runs.jsonl", "{}\n")):
            (real / name).write_text(data)
        # Media of the day in the repository's own test-output and in a worktree.
        for leak in (repo / "test-output" / DAY, repo / ".claude" / "worktrees" / "agent-x" / "test-output" / DAY):
            leak.mkdir(parents=True)
            (leak / "110000-leak.jpg").write_bytes(b"\xff\xd8 must not be gathered")
        return repo

    def test_overrides_keep_the_default_archive_untouched(self):
        repo = self.make_repo()
        watched = {p: tree_state(p) for p in (repo / "art" / "build-history", repo / "test-output",
                                              repo / ".claude")}
        (self.out / DAY).mkdir(parents=True)
        (self.out / DAY / "101500-shot.jpg").write_bytes(b"\xff\xd8 fixture")
        env = {**os.environ, "OCW_ART_HISTORY": str(self.hist), "OCW_TEST_OUTPUT": str(self.out),
               "OCW_TRANSCRIPTS": str(self.tmp / "no-transcripts"), "OCW_ART_PYTHON": sys.executable}
        env.pop("OCW_ART_WORKTREES", None)
        time.sleep(0.05)  # an mtime change would be visible
        r = subprocess.run([str(repo / "scripts" / "art-collect.sh"), "--no-sheets", DAY], env=env,
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for p, before in watched.items():
            self.assertEqual(tree_state(p), before, f"{p} was changed")
        self.assertEqual((repo / "art" / "build-history" / "SENTINEL").read_text(), "do not touch\n")
        # Only the fixture's own media were gathered (no leak, no real files).
        self.assertEqual(sorted(f.name for f in (self.hist / DAY).iterdir() if f.suffix == ".jpg"),
                         ["101500-shot.jpg"])
        st = json.loads((self.out / "build-log" / dt.date.today().isoformat() / "art.json").read_text())
        self.assertEqual(st["days"][DAY]["media"], 1)

    def test_sources_honour_the_overrides(self):
        """In-process: with WORKTREES off only TEST_OUTPUT is a source."""
        self.assertEqual(diary.sources(), [self.out])
        wt = self.tmp / "wt" / "agent-y" / "test-output"
        wt.mkdir(parents=True)
        diary.WORKTREES = self.tmp / "wt"
        self.assertEqual(diary.sources(), [self.out, wt])


if __name__ == "__main__":
    unittest.main()
