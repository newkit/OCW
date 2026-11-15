"""Tests of the coordinator (deb) card: parsers, alerts, the history ring,
and that no command line or environment reaches the status (fixture
text and a fake /proc in a temp directory; never the real /proc).

    python3 -m unittest discover tools/build-dashboard
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))

import coordinator as c  # noqa: E402
import dashboard as d  # noqa: E402

GIB = 1024 ** 3

MEMINFO = """MemTotal:       16383264 kB
MemFree:          613460 kB
MemAvailable:   13483468 kB
Buffers:           97460 kB
Cached:         12963244 kB
Active(anon):    1469084 kB
Shmem:           4283868 kB
SwapTotal:       4194300 kB
SwapFree:         205360 kB
"""

PSI = """some avg10=0.10 avg60=12.43 avg300=2.08 total=3388213342
full avg10=0.00 avg60=1.50 avg300=0.00 total=0
"""

MOUNTS = r"""sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
/dev/mapper/deb9--vg-root / ext4 rw,relatime,errors=remount-ro 0 0
tmpfs /run tmpfs rw,nosuid,nodev 0 0
/dev/mapper/deb9--vg-root /home ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev,nr_inodes=1048576,inode64 0 0
/dev/vdc /home/claude ext4 rw,relatime,errors=remount-ro 0 0
/dev/vdc /home/claude ext4 rw,relatime,errors=remount-ro 0 0
/dev/sdb1 /media/with\040space ext4 rw 0 0
"""

JOURNAL = [
    json.dumps({"MESSAGE": "ocw-build-dashboard.service: A process of this unit has been killed by the OOM killer.",
                "__REALTIME_TIMESTAMP": "1794699558000000"}),
    json.dumps({"MESSAGE": "ocw-build-dashboard.service: Failed with result 'oom-kill'.",
                "__REALTIME_TIMESTAMP": "1794699558100000"}),
    json.dumps({"MESSAGE": "tmux-spawn-75bb77d1-0461-4850-87c0-3f340ccdf0f3.scope: A process of this unit has "
                           "been killed by the OOM killer.", "__REALTIME_TIMESTAMP": "1794699587000000"}),
    json.dumps({"MESSAGE": "Out of memory: Killed process 4242 (claude) total-vm:1234kB, anon-rss:999kB",
                "__REALTIME_TIMESTAMP": "1794699557000000"}),
    "not json",
    json.dumps({"MESSAGE": "unrelated", "__REALTIME_TIMESTAMP": "1"}),
]

DMESG = """[ 1234.567890] claude invoked oom-killer: gfp_mask=0x140cca(GFP_HIGHUSER_MOVABLE|__GFP_COMP), order=0
[ 1234.600000] Out of memory: Killed process 4242 (claude) total-vm:21474836kB, anon-rss:6553600kB
[ 2000.000000] Memory cgroup out of memory: Killed process 77 (ffmpeg) total-vm:1kB
"""


def statvfs_fixture(table):
    def f(path):
        blocks, bfree, bavail = table[path]
        return SimpleNamespace(f_blocks=blocks, f_bfree=bfree, f_bavail=bavail, f_frsize=4096)
    return f


class Parsers(unittest.TestCase):
    def test_meminfo(self):
        m = c.parse_meminfo(MEMINFO)
        self.assertEqual(m["total"], 16383264 * 1024)
        self.assertEqual(m["available"], 13483468 * 1024)
        self.assertEqual(m["used"], (16383264 - 13483468) * 1024)
        self.assertEqual(m["shmem"], 4283868 * 1024)
        self.assertEqual(m["swap_used"], (4194300 - 205360) * 1024)
        self.assertAlmostEqual(m["available_pct"], 82.3, places=1)
        self.assertAlmostEqual(m["swap_pct"], 95.1, places=1)
        self.assertEqual(c.parse_meminfo("garbage"), {})

    def test_meminfo_without_swap(self):
        m = c.parse_meminfo("MemTotal: 1000 kB\nMemAvailable: 500 kB\n")
        self.assertEqual(m["swap_total"], 0)
        self.assertEqual(m["swap_pct"], 0.0)

    def test_psi(self):
        p = c.parse_psi(PSI)
        self.assertEqual(p["some"]["avg60"], 12.43)
        self.assertEqual(p["full"]["avg10"], 0.0)
        self.assertEqual(c.parse_psi(""), {})

    def test_loadavg(self):
        self.assertEqual(c.parse_loadavg("0.50 0.89 0.75 1/991 4070463\n"), [0.5, 0.89, 0.75])
        self.assertIsNone(c.parse_loadavg(""))

    def test_mounts_own_mounts_only_deduped_by_device(self):
        ms = c.parse_mounts(MOUNTS)
        self.assertEqual([m["mount"] for m in ms], ["/", "/home/claude", "/tmp"])  # /home: same device as /
        self.assertEqual(ms[2]["fstype"], "tmpfs")

    def test_mounts_separate_home_and_octal_escapes(self):
        text = "/dev/sda1 / ext4 rw 0 0\n/dev/sda2 /home ext4 rw 0 0\n/dev/sdb1 /media/with\\040space ext4 rw 0 0\n"
        self.assertEqual([m["mount"] for m in c.parse_mounts(text)], ["/", "/home"])
        self.assertEqual(c.parse_mounts(text, ("/media/with space",))[0]["mount"], "/media/with space")

    def test_mounts_two_tmpfs_are_not_merged(self):
        text = "tmpfs /tmp tmpfs rw 0 0\ntmpfs /home tmpfs rw 0 0\n/dev/x / ext4 rw 0 0\n"
        self.assertEqual(len(c.parse_mounts(text)), 3)

    def test_disk_usage_like_df(self):
        f = statvfs_fixture({"/": (1000, 100, 50)})
        u = c.disk_usage({"mount": "/", "fstype": "ext4"}, f)
        self.assertEqual(u["total"], 1000 * 4096)
        self.assertEqual(u["used"], 900 * 4096)
        self.assertEqual(u["free"], 50 * 4096)
        self.assertAlmostEqual(u["pct"], 100 * 900 / 950, places=1)
        self.assertFalse(u["in_ram"])
        self.assertTrue(c.disk_usage({"mount": "/", "fstype": "tmpfs"}, f)["in_ram"])

    def test_stat_with_odd_names(self):
        st = "1234 (Web (Content) x) S 99 1234 1234 0 -1 4194560 " + " ".join(["0"] * 8) + " 20 0 1 0 100 4096 777 0"
        self.assertEqual(c.parse_stat(st), (1234, "Web (Content) x", 99, 777))
        self.assertIsNone(c.parse_stat("garbage"))

    def test_process_kinds(self):
        k = c.process_kind
        self.assertEqual(k("claude"), "claude")
        self.assertEqual(k("2.1.325", "/home/u/.local/share/claude/versions/2.1.325"), "claude")
        self.assertEqual(k("2.1.325", "/usr/bin/other"), "other")
        self.assertEqual(k("rustc"), "cargo/rustc")
        self.assertEqual(k("cargo-clippy"), "cargo/rustc")
        self.assertEqual(k("ffmpeg"), "ffmpeg")
        self.assertEqual(k("python3"), "python")
        self.assertEqual(k("python3.13"), "python")
        self.assertEqual(k("qemu-system-x86"), "qemu")
        self.assertEqual(k("qemu-ga"), "other")  # the guest agent is no VM
        self.assertEqual(k("chrome"), "browsers")
        self.assertEqual(k("headless_shell"), "browsers")
        self.assertEqual(k("influxd"), "other")

    def test_summarize_processes(self):
        procs = [(1, "bash", 0, 100, "other"), (10, "claude", 1, 1000, "claude"),
                 (11, "claude", 10, 300, "claude"), (12, "2.1.325", 11, 400, "claude"),
                 (20, "claude", 1, 500, "claude"), (30, "ffmpeg", 1, 50, "ffmpeg"),
                 (31, "qemu-system-x86", 1, 2000, "qemu"), (40, "kthreadd", 0, 0, "other")]
        s = c.summarize_procs(procs, page=1)
        self.assertEqual(s["claude_sessions"], 2)  # 10 and 20: parents are no claude
        self.assertEqual(sorted(s["claude_session_rss"]), [500, 1000])
        self.assertEqual([x["name"] for x in s["top"]], ["qemu-system-x86", "claude", "claude", "claude 2.1.325", "claude"])
        self.assertEqual(s["groups"]["claude"], {"count": 4, "rss": 2200})
        self.assertEqual(s["ffmpeg"], 1)
        self.assertEqual(s["qemu"], 1)
        self.assertEqual(len(s["top"]), c.TOP_N)

    def test_journal_oom_events(self):
        ev = c.parse_journal_oom(JOURNAL)
        self.assertEqual([e["name"] for e in ev], ["ocw-build-dashboard.service", "tmux-spawn-75bb77d1….scope", "claude"])
        self.assertEqual(ev[0]["t"], 1794699558.0)
        merged = c.merge_oom(ev + [{"t": 1794699558.5, "name": "ocw-build-dashboard.service", "source": "x"}])
        self.assertEqual(len(merged), 3)  # the same unit from two sources once, newest first
        self.assertEqual(merged[0]["name"], "tmux-spawn-75bb77d1….scope")

    def test_dmesg_oom_events(self):
        ev = c.parse_dmesg_oom(DMESG, boot_time=1000)
        self.assertEqual([(e["name"], e["t"]) for e in ev], [("claude", 2234.6), ("ffmpeg", 3000.0)])

    def test_vmstat_systemctl_sizes(self):
        self.assertEqual(c.parse_vmstat_oom("pgfault 1\noom_kill 2\nfoo 3\n"), 2)
        self.assertIsNone(c.parse_vmstat_oom(""))
        kv = c.parse_systemctl_show("ActiveState=active\nNextElapseUSecRealtime=Sun 2026-11-15 15:00:41 CET\n")
        self.assertEqual(kv["ActiveState"], "active")
        self.assertEqual(c.parse_memory_size("1G"), GIB)
        self.assertEqual(c.parse_memory_size("768M"), 768 * 1024 ** 2)
        self.assertEqual(c.parse_memory_size("1073741824\n"), GIB)
        self.assertIsNone(c.parse_memory_size("max"))
        self.assertEqual(c.unit_memory_limits("[Service]\nMemoryHigh=768M\nMemoryMax=1G\n"),
                         {"MemoryHigh": 768 * 1024 ** 2, "MemoryMax": GIB})

    def test_the_unit_file_has_the_limits(self):
        lim = c.unit_memory_limits(c.UNIT_FILE.read_text())
        self.assertEqual(lim["MemoryMax"], GIB)

    def test_home_path(self):
        self.assertEqual(c.home_path("/home/u/data/x", "/home/u"), "~/data/x")
        self.assertEqual(c.home_path("/home/user2/x", "/home/u"), "/home/user2/x")
        self.assertEqual(c.home_path("/tmp", "/home/u"), "/tmp")


def status(mem_avail_pct=80.0, total=16 * GIB, swap_pct=10.0, root_pct=50.0, oom_age=None, now=1e9,
           psi=0.0, load=0.5, cargo=0, dash_used=300 * 1024 ** 2):
    st = {"memory": {"total": total, "available": int(total * mem_avail_pct / 100), "available_pct": mem_avail_pct,
                     "swap_total": 4 * GIB, "swap_used": int(4 * GIB * swap_pct / 100), "swap_pct": swap_pct},
          "disks": [{"mount": "/", "pct": root_pct, "free": 10 * GIB}],
          "pressure": {"memory": {"some": {"avg60": psi}}}, "load": [load, 0, 0], "ncpu": 4,
          "dashboard": {"used": dash_used, "max": GIB},
          "processes": {"groups": {"cargo/rustc": {"count": cargo, "rss": 0}}},
          "oom": {"events": [{"t": now - oom_age, "name": "claude"}] if oom_age is not None else []}}
    return st


def levels(st, now=1e9):
    return {a["kind"]: a["level"] for a in c.evaluate_alerts(st, now)}


class Alerts(unittest.TestCase):
    def test_quiet_when_fine(self):
        self.assertEqual(c.evaluate_alerts(status(), 1e9), [])

    def test_memory(self):
        self.assertEqual(levels(status(mem_avail_pct=14))["memory"], "bad")
        self.assertEqual(levels(status(mem_avail_pct=20))["memory"], "warn")
        # Below 2 GiB is red even when that is > 15 % of a small machine.
        self.assertEqual(levels(status(mem_avail_pct=20, total=8 * GIB))["memory"], "bad")
        self.assertEqual(levels(status(mem_avail_pct=40, total=8 * GIB))["memory"], "warn")  # 3.2 GiB < 4 GiB

    def test_swap_disk_oom(self):
        self.assertEqual(levels(status(swap_pct=95))["swap"], "bad")
        self.assertEqual(levels(status(swap_pct=60))["swap"], "warn")
        self.assertEqual(levels(status(root_pct=94))["disk"], "bad")
        self.assertEqual(levels(status(root_pct=88))["disk"], "warn")
        self.assertNotIn("disk", levels(status(root_pct=85)))
        self.assertEqual(levels(status(oom_age=3600))["oom"], "bad")
        self.assertEqual(levels(status(oom_age=3 * 86400))["oom"], "warn")
        self.assertNotIn("oom", levels(status(oom_age=8 * 86400)))

    def test_pressure_load_dashboard_cargo(self):
        self.assertEqual(levels(status(psi=12))["pressure"], "warn")
        self.assertEqual(levels(status(load=7))["load"], "warn")
        self.assertEqual(levels(status(load=13))["load"], "bad")
        self.assertEqual(levels(status(dash_used=int(0.95 * GIB)))["dashboard"], "bad")
        self.assertEqual(levels(status(cargo=2))["cargo"], "warn")

    def test_alert_text_names_the_coordinator(self):
        a = c.evaluate_alerts(status(root_pct=94), 1e9)[0]
        self.assertEqual(a["machine"], c.NAME)
        self.assertTrue(a["text"].startswith("deb: disk / 94 %"))

    def test_thresholds_in_one_place(self):
        for k, (warn, bad) in c.THRESHOLDS.items():
            if k.endswith("_below") or k == "oom_age_s":
                self.assertGreater(warn, bad, k)
            else:
                self.assertLess(warn, bad, k)


class HistoryRing(unittest.TestCase):
    def line(self, t, avail=8 * GIB, load=1.0, free=10 * GIB):
        return {"t": t, "time": "x", "memory": {"available": avail, "total": 16 * GIB, "swap_used": 0},
                "load": [load, 0, 0], "disks": [{"mount": "/", "free": free, "pct": 50}]}

    def test_writes_lines_and_reads_them_back(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            now = 1_800_000_000.0
            h = c.History(root)
            for k in range(5):
                h.add(self.line(now - 300 + 60 * k))
            f = h.path(now)
            self.assertEqual(f.name, "deb.jsonl")
            lines = f.read_text().splitlines()
            self.assertEqual(len(lines), 5)
            self.assertEqual(json.loads(lines[0])["t"], now - 300)
            with open(f, "a") as fh:
                fh.write("{broken\n")
            h2 = c.History(root)
            self.assertEqual(h2.load(now), 5)  # the broken line is skipped
            self.assertEqual(len(h2.ring), 5)

    def test_ring_drops_old_samples(self):
        h = c.History(None, keep_s=600)
        for k in range(30):
            h.add(self.line(1000.0 + 60 * k))
        self.assertTrue(all(p[0] >= 1000 + 60 * 29 - 600 for p in h.ring))
        self.assertLessEqual(len(h.ring), h.ring.maxlen)

    def test_series_keeps_the_worst_in_a_bucket(self):
        h = c.History(None)
        now = 100_000.0
        h.add(self.line(now - 100, avail=5 * GIB, load=1))
        h.add(self.line(now - 90, avail=3 * GIB, load=4))
        h.add(self.line(now - 4 * 3600, avail=9 * GIB))
        s = h.series(now, span=3600, points=10)
        self.assertEqual(s["mem_avail"], [3 * GIB])  # only the last hour, the minimum
        self.assertEqual(s["load1"], [4])
        self.assertEqual(h.series(now, span=6 * 3600, points=10)["mem_avail"], [9 * GIB, 3 * GIB])

    def test_empty(self):
        self.assertEqual(c.History(None).series(1.0)["t"], [])


def fake_proc(root: Path):
    """A /proc with three processes; cmdline and environ hold secrets."""
    (root / "pressure").mkdir(parents=True)
    (root / "meminfo").write_text(MEMINFO)
    (root / "loadavg").write_text("0.50 0.89 0.75 1/991 4070463\n")
    for k in ("cpu", "memory", "io"):
        (root / "pressure" / k).write_text(PSI)
    (root / "mounts").write_text("")  # no disks (statvfs of the real ones is not wanted)
    (root / "vmstat").write_text("oom_kill 3\n")
    (root / "stat").write_text("cpu 1 2 3\nbtime 1794600000\n")
    (root / "self").mkdir()
    (root / "self" / "status").write_text("VmRSS:\t  12345 kB\n")
    (root / "self" / "cgroup").write_text("0::/user.slice/session.scope\n")
    tail = " ".join(["0"] * 8) + " 20 0 1 0 100 4096 %d 0"
    for pid, comm, ppid, rss in ((1, "systemd", 0, 10), (100, "claude", 1, 50000), (200, "ffmpeg", 100, 900)):
        p = root / str(pid)
        p.mkdir()
        (p / "stat").write_text("%d (%s) S %d 1 1 0 -1 4194560 %s" % (pid, comm, ppid, tail % rss))
        (p / "cmdline").write_text("claude\0--api-key=SECRET-CMDLINE-TOKEN\0/home/u/private/plan.txt\0")
        (p / "environ").write_text("ANTHROPIC_API_KEY=SECRET-ENV-TOKEN\0")


class NoLeaks(unittest.TestCase):
    def test_no_command_line_or_environment_in_the_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "proc"
            fake_proc(root)
            repo = Path(tmp) / "repo"
            repo.mkdir()
            co = c.Coordinator(repo, None, None, proc_root=str(root), home=Path(tmp))
            opened = []
            real_open = open

            def spy(path, *a, **k):
                opened.append(str(path))
                return real_open(path, *a, **k)

            import builtins
            builtins.open, saved = spy, builtins.open
            try:
                fast = co.read_fast()
            finally:
                builtins.open = saved
            co.latest = fast
            co.dirs = {"at": 1.0, "items": []}
            st = co.status(with_history=False)
            text = json.dumps(st)
            self.assertNotIn("SECRET", text)
            self.assertNotIn("private", text)
            self.assertFalse([p for p in opened if p.endswith(("cmdline", "environ"))], opened)
            self.assertEqual(st["processes"]["claude_sessions"], 1)
            self.assertEqual(st["processes"]["groups"]["ffmpeg"]["count"], 1)
            self.assertEqual(st["oom_kill_since_boot"], 3)
            self.assertEqual(st["dashboard"]["rss"], 12345 * 1024)
            self.assertFalse(st["dashboard"]["in_unit"])
            self.assertEqual(st["dashboard"]["max"], GIB)  # from the unit file
            line = json.dumps(co.sample_line(1794700000.0))
            self.assertNotIn("SECRET", line)
            lines = c.text_lines(st)
            self.assertTrue(lines[0].startswith(("Coordinator", "ALERT", "WARN")))
            self.assertNotIn("SECRET", "\n".join(lines))

    def test_dashboard_adds_the_card_and_alerts(self):
        class FakeCoord:
            tasks = None

            def status(self):
                return {"name": "deb", "available": True,
                        "alerts": [{"machine": "deb", "level": "warn", "kind": "swap", "text": "deb: swap 60 % used"}]}

        st = d.add_coordinator({"alerts": [{"machine": "marge", "text": "marge: disk 93 %"}], "machines": []},
                               FakeCoord())
        self.assertEqual(st["coordinator"]["name"], "deb")
        self.assertEqual([a.get("level") for a in st["alerts"]], [None, "warn"])
        text = d.format_text(dict(st, overview={"online": 0, "machines": 0, "tasks": 0, "waiting": 0, "slots_used": 0,
                                                "slots_total": 0}, machines=[]))
        self.assertIn("WARN deb: swap 60 % used", text)
        self.assertIn("ALERT marge: disk 93 %", text)


if __name__ == "__main__":
    unittest.main()
