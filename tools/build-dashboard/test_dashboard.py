"""Unit tests for the build dashboard parsers (no network).

    python3 -m unittest discover tools/build-dashboard
"""

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import dashboard as d  # noqa: E402

BROKEN_TOML = r'''
[defaults]
port = 22
ssh_dir = "ssh"   # comment
connect_timeout_s = 10

[[machine]]
name = "leela"
os = "linux"
roles = ["build", "test", "bench"]
host = "10.1.2.3"
user = "builder"
identity_file = "ocw_linux_ed25519"
repo_path = "~/src/OCW"

[[machine]]
name = "Win11Builder"
os = "windows"
host = "win.example.lan"
user = "claude"
repo_path = "B:\\src\\OCW"
#gpu = "AMD Radeon RX 6600"

[[machine]]
name = "macos"
os = "macos"
roles = ["build", "test". "bench"]
host = "mac.example.lan"
port = 2222
user = "ocw"
vram_gb = ?
cpu = ?
display = "aqua"
notes = "Has a # inside"
'''


class ConfigTest(unittest.TestCase):
    def test_tolerant_parser_handles_broken_macos_entry(self):
        defaults, machines = d.parse_machines(BROKEN_TOML)
        self.assertEqual(defaults, {"port": 22, "ssh_dir": "ssh", "connect_timeout_s": 10})
        self.assertEqual([m["name"] for m in machines], ["leela", "Win11Builder", "macos"])
        mac = machines[2]
        self.assertEqual(mac["roles"], ["build", "test", "bench"])
        self.assertEqual(mac["port"], 2222)
        self.assertNotIn("vram_gb", mac)
        self.assertNotIn("cpu", mac)
        self.assertEqual(mac["notes"], "Has a # inside")
        self.assertNotIn("gpu", machines[1])

    def test_windows_path_unescaped(self):
        _, machines = d.parse_machines(BROKEN_TOML)
        self.assertEqual(machines[1]["repo_path"], "B:\\src\\OCW")

    def test_machine_defaults_and_paths(self):
        defaults, machines = d.parse_machines(BROKEN_TOML)
        m = d.Machine(machines[0], defaults, Path("/cfg"))
        self.assertEqual(m.port, 22)
        self.assertEqual(m.identity, "/cfg/ssh/ocw_linux_ed25519")
        self.assertEqual(m.known_hosts, "/cfg/ssh/known_hosts")
        argv = d.ssh_argv(m, "/run/ctl")
        for opt in ("IdentitiesOnly=yes", "BatchMode=yes", "StrictHostKeyChecking=yes",
                    "UserKnownHostsFile=/cfg/ssh/known_hosts", "ConnectTimeout=10"):
            self.assertIn(opt, argv)
        self.assertIn("ControlPath=/run/ctl/%C", argv)
        self.assertEqual(argv[-1], "builder@10.1.2.3")

    def test_slot_counts_like_remote_sh(self):
        # No `slots` keys: OS defaults linux 2, windows 1, macos 1.
        defaults, machines = d.parse_machines(BROKEN_TOML)
        ms = [d.Machine(m, defaults, Path("/cfg")) for m in machines]
        self.assertEqual([m.slots for m in ms], [2, 1, 1])
        # A machine's `slots` wins over [defaults] slots, which wins over the OS default.
        text = BROKEN_TOML.replace("connect_timeout_s = 10", "connect_timeout_s = 10\nslots = 3")
        text = text.replace('name = "Win11Builder"', 'name = "Win11Builder"\nslots = 2  # 16 GB now')
        defaults, machines = d.parse_machines(text)
        self.assertEqual([d.Machine(m, defaults, Path("/cfg")).slots for m in machines], [3, 2, 3])
        self.assertEqual(d.slots_setting("0", "linux"), 0)
        self.assertEqual(d.slots_setting("x", "windows"), 1)
        self.assertEqual(d.slots_setting(-1, "macos"), 1)
        self.assertEqual(d.default_slots("freebsd"), 2)


class MaskTest(unittest.TestCase):
    def test_masks_hosts_users_addresses(self):
        mk = d.Masker(["win.example.lan", "claude", "10.1.2.3"], ["/home/claude"])
        text = ("ssh: connect to host win.example.lan port 22: Connection refused; "
                "10.1.2.3 192.168.0.7:22 fe80::1%eth0 2001:db8:0:0:0:0:0:1 "
                "builder@other.lan /home/claude/src/OCW /home/someuser/x C:\\Users\\Someone\\.cargo "
                "AA:BB:CC:DD:EE:FF at 21:32:41")
        out = mk(text)
        for leak in ("win.example.lan", "claude", "10.1.2.3", "192.168", "fe80", "2001:db8",
                     "builder@", "someuser", "Someone", "AA:BB", "port 22"):
            self.assertNotIn(leak, out)
        self.assertIn("~/src/OCW", out)
        self.assertIn("21:32:41", out)  # times are not addresses

    def test_deep(self):
        mk = d.Masker(["secret.lan"])
        self.assertEqual(mk.deep({"a": ["x secret.lan"], "b": 3}), {"a": ["x <masked>"], "b": 3})

    def test_classify_ssh_error(self):
        self.assertEqual(d.classify_ssh_error("ssh: connect to host h port 22: Connection refused", 255), "connection refused")
        self.assertEqual(d.classify_ssh_error("ssh: connect to host h port 22: Connection timed out", 255), "timeout")
        self.assertEqual(d.classify_ssh_error("ssh: connect to host h port 22: No route to host", 255), "no route to host")
        self.assertEqual(d.classify_ssh_error("Host key verification failed.", 255), "host key mismatch")
        self.assertEqual(d.classify_ssh_error("", None, timed_out=True), "timeout")


PS = """\
      1       0     0  9000   1000 /sbin/init
    100       1     0  9000   1000 sshd: /usr/sbin/sshd
    200     100  1002   400   3000 bash -c ocw_slot() { ...; }; ocw_slot; cd ~/src/OCW-worktree-agent-a08d34a9817282b1d && . ~/.cargo/env && export CARGO_BUILD_JOBS=4 && cargo test --workspace
    201     200  1002   390  20000 /home/claude/.cargo/bin/cargo test --workspace
    202     201  1002   380  20000 /home/claude/.rustup/toolchains/x/bin/cargo test --workspace
    203     202  1002    20 500000 /home/claude/.rustup/toolchains/x/bin/rustc --crate-name bevy_render --edition=2021 /home/claude/.cargo/registry/src/bevy_render/src/lib.rs
    204     202  1002    10 400000 /home/claude/.rustup/toolchains/x/bin/rustc --crate-name ocw_sim crates/ocw-sim/src/lib.rs
    205     202  1002     5  90000 cc -m64 /tmp/rustcXYZ/symbols.o -o /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d/target/debug/deps/ocw_sim-0123456789abcdef
    206     202  1002     3  80000 /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d/target/debug/deps/ocw_core-0123456789abcdef
    300     100  1002   100   3000 bash -c ocw_slot() { ...; }; ocw_slot; cd ~/src/OCW && . ~/.cargo/env && export OCW_REMOTE=1 && cargo build --release
    301     300  1002    99   1000 flock 9
    400     100  1002    50   3000 bash -c ocw_slot() { ...; }; ocw_slot; cd ~/src/OCW-fix-x && . ~/.cargo/env && export CARGO_BUILD_JOBS=4 && cargo clippy
    401     400  1002    49   1000 flock 9
    500       1  1000   838   4048 bash /var/tmp/ocw/ocw-kiosk.sh run -- --monitor 2560x1600
    501     500  1000   439 336272 /var/tmp/ocw/ocw-desktop --monitor 2560x1600
    600     100  1002     1   2000 bash -c export LC_ALL=C
    601     600  1002     0   1000 ps -eo pid=,ppid=,uid=,etimes=,rss=,args=
    700       1  1002    60  10000 rust-analyzer
"""

CWD = """\
/proc/200 /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d
/proc/201 /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d
/proc/202 /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d
/proc/203 /home/claude/.cargo/registry/src/bevy_render
/proc/204 /home/claude/src/OCW-worktree-agent-a08d34a9817282b1d/crates/ocw-sim
/proc/300 /home/claude
/proc/301 /home/claude
/proc/400 /home/claude
/proc/401 /home/claude
/proc/600 /home/claude
"""

LINUX_OUTPUT = (
    "@@self\n600\n@@home\n/home/claude\n1002\n@@nproc\n12\n"
    "@@loadavg\n1.71 2.05 2.20 3/900 12345\n@@uptime\n1013.50 9000.00\n"
    "@@meminfo\nMemTotal:       32000000 kB\nMemAvailable:   28000000 kB\nSwapTotal:       1000000 kB\nSwapFree:         750000 kB\n"
    "@@df\n/dev/nvme0n1p2 400000000 280000000 115000000 71% /\n"
    "@@stat\ncpu  100 0 100 700 100 0 0 0 0 0\ncpu  150 0 150 800 100 0 0 0 0 0\n"
    "@@ps\n" + PS + "@@cwd\n" + CWD +
    "@@slots\nocw-build-slot-1.lock 4444 : 200 201 202 300 400\nocw-build-slot-2.lock 4445 :\n"
    "@@locks\n8: FLOCK  ADVISORY  WRITE 1439 fe:00:1085944 0 EOF\n9: FLOCK  ADVISORY  WRITE 210 00:19:4444 0 EOF\n"
    "9: -> FLOCK  ADVISORY  WRITE 301 00:19:4444 0 EOF\n9: -> FLOCK  ADVISORY  WRITE 401 00:19:4444 0 EOF\n"
    "@@kiosk\na811b99 2026-11-11 21:32:41\n"
    "@@bt\nDevice F1:22:33:44:55:66 Wahoo KICKR A208\n\x1b[0;94m[bluetooth]\x1b[0m# \n@@end\n")


class ProcTest(unittest.TestCase):
    def test_cpu_busy(self):
        self.assertEqual(d.parse_cpu_busy(["cpu  100 0 100 700 100", "cpu  150 0 150 800 100"]), 50.0)
        self.assertIsNone(d.parse_cpu_busy(["cpu  1 2 3 4"]))

    def test_meminfo_df(self):
        m = d.parse_meminfo(["MemTotal: 1000 kB", "MemAvailable: 250 kB", "SwapTotal: 0 kB", "SwapFree: 0 kB"])
        self.assertEqual(m, {"mem_total": 1024000, "mem_used": 768000})
        self.assertEqual(d.parse_df(["/dev/x 100 60 40 60% /"])["disk_free"], 40 * 1024)

    def test_proc_locks(self):
        locks = d.parse_proc_locks([
            "1: POSIX  ADVISORY  WRITE 3924 fe:00:6039789 1073741826 1073742335",
            "9: FLOCK  ADVISORY  WRITE 1484 00:19:4444 0 EOF",
            "9: -> FLOCK  ADVISORY  WRITE 1500 00:19:4444 0 EOF",
        ])
        self.assertEqual(locks, {4444: {"held": True, "waiting": 1}})

    def test_etime(self):
        self.assertEqual(d.parse_etime("05:03"), 303)
        self.assertEqual(d.parse_etime("1:00:00"), 3600)
        self.assertEqual(d.parse_etime("2-01:00:00"), 2 * 86400 + 3600)

    def test_bluetooth_names_only(self):
        self.assertEqual(d.parse_bluetooth(["Device AA:BB:CC:DD:EE:FF Wahoo KICKR A208", "Agent registered"]),
                         ["Wahoo KICKR A208"])


class GroupingTest(unittest.TestCase):
    def test_checkout_labels(self):
        h = "/home/u"
        self.assertEqual(d.checkout_label("/home/u/src/OCW", "~/src/OCW", h), "main")
        self.assertEqual(d.checkout_label("/home/u/src/OCW/crates/x", "~/src/OCW", h), "main")
        self.assertEqual(d.checkout_label("/home/u/src/OCW-worktree-agent-a08d34a9817282b1d/target", "~/src/OCW", h),
                         "agent a08d34a9")
        self.assertEqual(d.checkout_label("/home/u/src/OCW-feature-x", "~/src/OCW", h), "feature-x")
        self.assertEqual(d.checkout_label("/var/tmp/ocw/ocw-desktop", "~/src/OCW", h), "kiosk")
        self.assertIsNone(d.checkout_label("/home/u/.cargo/registry/src/x", "~/src/OCW", h))
        self.assertEqual(d.checkout_label("b:\\SRC\\OCW-main2\\target\\debug\\x.exe", "B:\\src\\OCW"), "main2")

    def test_classify(self):
        c = d.classify_process
        self.assertEqual(c("/x/cargo +stable test -p ocw-sim"), "cargo test")
        self.assertEqual(c("rustc --crate-name x"), "rustc")
        self.assertEqual(c("/usr/bin/ld.lld -o x"), "link")
        self.assertEqual(c("/a/target/debug/deps/ocw_sim-0123456789abcdef --nocapture"), "test")
        self.assertEqual(c("/a/target/debug/build/foo-1234/build-script-build"), "build script")
        self.assertEqual(c("flock 9"), "waiting for slot")
        self.assertEqual(c("B:\\src\\OCW\\target\\debug\\deps\\ocw_sim-0123456789abcdef.exe"), "test")
        self.assertEqual(c("C:\\Users\\x\\.cargo\\bin\\cargo.exe"), "cargo")
        self.assertIsNone(c("rust-analyzer"))
        self.assertIsNone(c("/usr/bin/vim cargo.toml"))

    def test_linux_full_parse(self):
        out = d.parse_linux(LINUX_OUTPUT, "~/src/OCW")
        self.assertEqual(out["ncpu"], 12)
        self.assertEqual(out["load"], [1.71, 2.05, 2.2])
        self.assertEqual(out["cpu_busy"], 50.0)
        self.assertEqual(out["swap_used"], 250000 * 1024)
        groups = {g["checkout"]: g for g in out["tasks"]}
        self.assertEqual(set(groups), {"agent a08d34a9"})  # waiting runs are not tasks; ps is skipped
        g = groups["agent a08d34a9"]
        self.assertEqual(g["command"], "cargo test --workspace")
        kinds = {k["kind"]: k for k in g["kinds"]}
        self.assertEqual(kinds["cargo test"]["count"], 1)  # rustup proxy counted once
        self.assertEqual(kinds["rustc"]["count"], 2)  # registry crate inherits the checkout
        self.assertEqual(kinds["link"]["count"], 1)
        self.assertEqual(kinds["test"]["count"], 1)
        q = out["queue"]
        self.assertEqual((q["used"], q["total"], q["waiting"]), (1, 2, 2))
        self.assertEqual(q["slots"][0]["holder"], "agent a08d34a9")
        self.assertEqual(sorted(q["waiting_checkouts"]), ["fix-x", "main"])
        # Waiters of an older remote.sh (blocked in `flock 9`): waiting time from the flock process.
        self.assertEqual(sorted((w["checkout"], w["elapsed"], w["command"]) for w in q["queued"]),
                         [("fix-x", 49, None), ("main", 99, None)])
        k = out["kiosk"]
        self.assertTrue(k["running"])
        self.assertTrue(k["other_user"])
        self.assertEqual(k["version"], "a811b99 2026-11-11 21:32:41")
        self.assertEqual(k["bluetooth"], ["Wahoo KICKR A208"])

    def test_macos_parse(self):
        text = ("@@self\n50\n@@home\n/Users/ocw\n501\n@@sysctl\n8\n17179869184\n{ 1.45 1.60 1.70 }\n"
                "{ sec = 1000, usec = 0 } Thu Jan  1 00:16:40 1970\ntotal = 2048.00M  used = 512.00M  free = 1536.00M  (encrypted)\n"
                "@@now\n43360\n@@vmstat\nMach Virtual Memory Statistics: (page size of 16384 bytes)\n"
                "Pages free:                               100000.\nPages active:  300000.\nPages inactive:                           200000.\n"
                "Pages speculative:                         10000.\n"
                "@@df\n/dev/disk3s5 488000000 300000000 140000000 69% /System/Volumes/Data\n"
                "@@cpu\nCPU usage: 3.10% user, 5.00% sys, 91.90% idle\n"
                "@@ps\n  10 1 501 01:02:03 20000 /Users/ocw/.cargo/bin/cargo build --release\n"
                "  11 10 501 05:00 400000 /Users/ocw/.rustup/toolchains/x/bin/rustc --crate-name y\n"
                "@@cwd\np10\nfcwd\nn/Users/ocw/src/OCW\np11\nfcwd\nn/Users/ocw/.cargo/registry/src/y\n@@end\n")
        out = d.parse_macos(text, "~/src/OCW")
        self.assertEqual(out["ncpu"], 8)
        self.assertEqual(out["load"], [1.45, 1.6, 1.7])
        self.assertEqual(out["uptime"], 42360.0)
        self.assertEqual(out["mem_used"], 17179869184 - 310000 * 16384)
        self.assertEqual(out["cpu_busy"], 8.1)
        self.assertEqual(out["swap_used"], 512 * 1024 * 1024)
        g = out["tasks"][0]
        self.assertEqual(g["checkout"], "main")
        self.assertEqual({k["kind"]: k["count"] for k in g["kinds"]}, {"cargo build": 1, "rustc": 1})
        self.assertEqual(g["elapsed"], 3723)

    def test_windows_parse(self):
        data = {"ncpu": 6, "uptime": 4731.4, "home": "C:\\Users\\claude", "mem_total": 8000, "mem_avail": 3000,
                "disks": [{"name": "B", "used": 10, "free": 90}, {"name": "C", "used": 5, "free": 5}],
                "procs": {"pid": 7, "ppid": None, "name": "rustc",
                          "args": "B:\\src\\OCW-worktree-agent-abcdef12345\\target\\debug\\deps\\ocw_sim-0123456789abcdef.exe",
                          "elapsed": 12.5, "rss": 1000},
                "self": 99}
        out = d.parse_windows("\ufeff" + json.dumps(data), "B:\\src\\OCW")
        self.assertEqual(out["mem_used"], 5000)
        self.assertEqual((out["disk_name"], out["disk_free"], out["disk_total"]), ("B:", 90, 100))
        self.assertIsNone(out["cpu_busy"])
        self.assertEqual(out["tasks"][0]["checkout"], "agent abcdef12")
        self.assertEqual(out["tasks"][0]["kinds"][0]["kind"], "test")

    def test_windows_command_is_encoded(self):
        cmd = d.windows_command("Write-Output 1")
        self.assertTrue(cmd.startswith("powershell -NoProfile -NonInteractive -EncodedCommand "))

    def test_summary_and_text(self):
        out = d.parse_linux(LINUX_OUTPUT, "~/src/OCW")
        st = d.summarize([dict(name="marge", os="linux", online=True, polled_at=0, **out),
                          dict(name="leela", os="linux", online=False, error="timeout")])
        self.assertEqual(st["overview"]["online"], 1)
        self.assertEqual(st["overview"]["waiting"], 2)
        text = d.format_text(st)
        self.assertIn("leela", text)
        self.assertIn("UNREACHABLE (timeout)", text)
        self.assertIn("Wahoo KICKR A208", text)


def marker(pid, start, state="running", slot=None, checkout="OCW", command="cargo test"):
    return json.dumps({"v": 1, "pid": pid, "state": state, "slot": slot, "start": start,
                       "checkout": checkout, "branch": "b", "command": command, "os": "linux"})


class MarkerTest(unittest.TestCase):
    def test_parse_marker(self):
        m = d.parse_marker(marker(200, 1000, slot=1, checkout="OCW-worktree-agent-a08d34a9817282b1d"))
        self.assertEqual((m["pid"], m["start"], m["state"], m["slot"]), (200, 1000, "running", 1))
        self.assertEqual(m["command"], "cargo test")
        self.assertEqual(d.parse_marker('{"pid": 1, "start": 5, "state": "odd"}')["state"], "running")
        for bad in ("", "not json", "[1, 2]", '{"pid": "1", "start": 5}', '{"pid": 1}'):
            self.assertIsNone(d.parse_marker(bad))
        lines = ["", "\ufeff" + marker(1, 2), "garbage", marker(3, 4, "waiting")]
        self.assertEqual([m["pid"] for m in d.parse_marker_lines(lines)], [1, 3])

    def test_stale_detection(self):
        m = d.parse_marker(marker(200, 1000))
        self.assertTrue(d.marker_stale(m, False, None))  # PID gone
        self.assertFalse(d.marker_stale(m, True, None))
        self.assertFalse(d.marker_stale(m, True, 999))  # shell started just before the marker
        self.assertTrue(d.marker_stale(m, True, 1000 + d.PID_REUSE_SLACK_S + 1))  # PID reused

    def test_live_markers_from_process_list(self):
        ms = [d.parse_marker(marker(200, 1000, checkout="OCW-worktree-agent-a08d34a9817282b1d")),
              d.parse_marker(marker(300, 1100, "waiting", checkout="OCW")),
              d.parse_marker(marker(400, 900)),  # PID gone
              d.parse_marker(marker(500, 900))]  # PID reused by a process started at 1900
        procs = {200: {"pid": 200, "elapsed": 1000}, 300: {"pid": 300, "elapsed": 900},
                 500: {"pid": 500, "elapsed": 100}}
        live, stale = d.live_markers(ms, 2000, procs, repo_path="~/src/OCW", home="/home/u")
        self.assertEqual(stale, 2)
        self.assertEqual([(m["label"], m["state"], m["elapsed"]) for m in live],
                         [("agent a08d34a9", "running", 1000), ("main", "waiting", 900)])

    def test_running_time_counts_from_getting_the_slot(self):
        m = json.loads(marker(200, 1000, slot=2))
        m["since"] = 1300  # waited 300 s for the slot
        w = json.loads(marker(300, 1500, "waiting"))
        w["since"] = 1500
        live, _ = d.live_markers([d.parse_marker(json.dumps(m)), d.parse_marker(json.dumps(w))], 2000,
                                 {200: {"pid": 200, "elapsed": 1000}, 300: {"pid": 300, "elapsed": 500}})
        self.assertEqual([x["elapsed"] for x in live], [700, 500])
        self.assertEqual(d.parse_marker(marker(1, 50))["since"], 50)  # older markers: start

    def test_live_markers_windows_alive_flags(self):
        ms = [d.parse_marker(marker(7, 1000, checkout="OCW-fix-x")), d.parse_marker(marker(8, 1000))]
        live, stale = d.live_markers(ms, 1060, alive={7: (True, 990), 8: (False, None)}, repo_path="B:\\src\\OCW")
        self.assertEqual(stale, 1)
        self.assertEqual(live[0]["label"], "fix-x")
        self.assertEqual(live[0]["elapsed"], 60)

    def test_label_unknown_dir_falls_back(self):
        self.assertEqual(d.marker_label({"dir": "scratch", "branch": "x"}, "~/src/OCW"), "scratch")
        self.assertEqual(d.marker_label({"dir": None, "branch": None}, "~/src/OCW"), "other")

    def test_merge_tasks(self):
        live = [dict(d.parse_marker(marker(1, 10, slot=2)), label="main", elapsed=50),
                dict(d.parse_marker(marker(2, 20, "waiting", command="cargo build")), label="fix-x", elapsed=40)]
        groups = [{"checkout": "main", "command": None, "elapsed": 45, "rss": 100, "cpu": 250.0,
                   "kinds": [{"kind": "rustc"}]},
                  {"checkout": "agent 1234abcd", "command": "cargo clippy", "elapsed": 5, "rss": 7, "cpu": None,
                   "kinds": []}]
        tasks = d.merge_tasks(live, groups)
        self.assertEqual([(t["checkout"], t["state"], t["source"]) for t in tasks],
                         [("main", "running", "marker"), ("fix-x", "waiting", "marker"),
                          ("agent 1234abcd", "running", "process")])
        self.assertEqual((tasks[0]["cpu"], tasks[0]["rss"], tasks[0]["slot"], tasks[0]["elapsed"]), (250.0, 100, 2, 50))
        self.assertEqual(tasks[0]["command"], "cargo test")
        self.assertEqual(tasks[1]["kinds"], [])  # a waiting run owns no processes

    def test_single_run_adopts_only_processes_it_started(self):
        live = [dict(d.parse_marker(marker(1, 10)), label="agent abcdef12", elapsed=60)]
        procs = [{"pid": 7, "elapsed": 30}, {"pid": 8, "elapsed": 2400}, {"pid": 9, "elapsed": None}]
        self.assertEqual(d.adopt_labels(procs, live), {7: "agent abcdef12"})  # 8: older than the run
        two = live + [dict(d.parse_marker(marker(2, 11)), label="main", elapsed=50)]
        self.assertEqual(d.adopt_labels(procs, two), {})  # ambiguous
        waiting = [dict(d.parse_marker(marker(3, 12, "waiting")), label="x", elapsed=5)]
        self.assertEqual(d.adopt_labels(procs, waiting), {})

    def test_queue_from_markers(self):
        live = [dict(d.parse_marker(marker(1, 10, slot=1)), label="main"),
                dict(d.parse_marker(marker(2, 11, slot=2)), label="fix-x"),
                dict(d.parse_marker(marker(3, 12, "waiting")), label="agent abcdef12")]
        q = d.queue_from_markers(live)
        self.assertEqual((q["used"], q["total"], q["waiting"]), (2, 2, 1))
        self.assertEqual(q["waiting_checkouts"], ["agent abcdef12"])
        self.assertEqual([s["holder"] for s in q["slots"]], ["main", "fix-x"])
        self.assertEqual(d.queue_from_markers([])["used"], 0)

    def test_queue_lists_waiting_runs_and_uses_configured_total(self):
        live = [dict(d.parse_marker(marker(1, 10, slot=1)), label="main", elapsed=90),
                dict(d.parse_marker(marker(3, 40, "waiting", command="sleep 60")), label="agent abcdef12", elapsed=60),
                dict(d.parse_marker(marker(4, 50, "waiting")), label="fix-x", elapsed=50)]
        q = d.queue_from_markers(live, 1)
        self.assertEqual((q["used"], q["total"], q["waiting"]), (1, 1, 2))
        self.assertEqual(q["queued"], [
            {"checkout": "agent abcdef12", "command": "sleep 60", "since": 40, "elapsed": 60},
            {"checkout": "fix-x", "command": "cargo test", "since": 50, "elapsed": 50}])
        # Slots held without a marker (older remote.sh) and waiters without one count too.
        q = d.queue_from_markers(live, 2, held={1: "other", 2: None}, extra_waiting=["kiosk"])
        self.assertEqual([(s["slot"], s["held"], s["holder"]) for s in q["slots"]],
                         [(1, True, "main"), (2, True, None)])
        self.assertEqual((q["used"], q["waiting"], q["waiting_checkouts"][-1]), (2, 3, "kiosk"))
        # A run started with OCW_REMOTE_SLOTS=3 reports it in its marker.
        m3 = json.loads(marker(5, 60, slot=3))
        m3["slots"] = 3
        q = d.queue_from_markers([dict(d.parse_marker(json.dumps(m3)), label="x", elapsed=1)], 2)
        self.assertEqual((q["total"], q["used"], q["slots"][2]["holder"]), (3, 1, "x"))

    def test_slot_dirs_macos(self):
        procs = {70: {"pid": 70, "args": "zsh -c ocw_meta=...", "cwd": "/Users/u/src/OCW-fix-x"}}
        held = d.parse_slot_dirs(["ocw-build-slot-1.d 70", "ocw-build-slot-2.d 71", "ocw-build-slot-3.d", "junk"],
                                 procs, repo_path="~/src/OCW", home="/Users/u")
        self.assertEqual(held, {1: "fix-x"})  # 2: owner gone (stale, free); 3: no PID yet

    def test_summary_totals_use_per_machine_slots(self):
        def q(total, used, waiting):
            return {"total": total, "used": used, "waiting": waiting, "slots": [], "queued": [], "waiting_checkouts": []}
        ms = [dict(name="marge", os="linux", online=True, tasks=[{"state": "running"}] * 2, queue=q(2, 2, 1)),
              dict(name="leela", os="linux", online=True, tasks=[], queue=q(2, 0, 0)),
              dict(name="Win11Builder", os="windows", online=True,
                   tasks=[{"state": "running"}, {"state": "waiting"}], queue=q(1, 1, 1)),
              dict(name="macos", os="macos", online=False, slots=1, error="timeout")]
        o = d.summarize(ms)["overview"]
        self.assertEqual((o["tasks"], o["waiting"], o["slots_used"], o["slots_total"]), (3, 2, 3, 6))

    def test_label_from_args_prefers_remote_sh_cd(self):
        args = ("bash -c ocw_meta='\"command\":\"cd /tmp && make\"'; ocw_d=/tmp/ocw-tasks; ocw_mark running 1; "
                "cd ~/src/OCW-fix-x && . ~/.cargo/env && export OCW_REMOTE=1 && cd /tmp && make")
        self.assertEqual(d._label_from_args(args, "~/src/OCW", "/home/u"), "fix-x")
        self.assertEqual(d.run_command(args), "cd /tmp && make")
        self.assertEqual(d.classify_process(args), "run")


class CpuTest(unittest.TestCase):
    def test_cpu_per_pid(self):
        a = ["100.00 900.00", "1 500", "2 1000", "3 7"]
        b = ["100.50 901.00", "1 550", "2 1000", "4 9"]
        self.assertEqual(d.cpu_per_pid(a, b, 100), {1: 100.0, 2: 0.0})
        self.assertEqual(d.cpu_per_pid(a, a, 100), {})  # no time passed

    def test_windows_counters(self):
        ok = d.windows_counters({"pdh_status": 0, "proc_queue": 3.0})
        self.assertEqual((ok["ok"], ok["hint"]), (True, None))
        denied = d.windows_counters({"pdh_status": 0xC0000BC8, "perfmon_member": False})
        self.assertFalse(denied["ok"])
        self.assertEqual(denied["hint"], "n/a (add build user to Performance Monitor Users)")
        self.assertEqual(denied["status"], "0xC0000BC8")
        self.assertEqual(d.windows_counters({})["hint"], d.PERFMON_HINT)
        member = d.windows_counters({"pdh_status": 0xC0000BC8, "perfmon_member": True})
        self.assertIn("restart sshd", member["hint"])
        self.assertIn("0xC0000BC8", member["hint"])

    def test_windows_parse_with_markers_and_counters(self):
        data = {"ncpu": 6, "uptime": 10.0, "home": "C:\\Users\\claude", "now": 1060, "cpu_busy": 93.88,
                "runnable": 7, "pdh_status": 3221228488, "perfmon_member": False,
                "procs": [{"pid": 7, "name": "rustc", "args": "rustc.exe", "elapsed": 12.5, "rss": 1000, "cpu": 139.4},
                          {"pid": 8, "name": "cargo", "args": "cargo.exe", "elapsed": 50, "rss": 10, "cpu": None},
                          {"pid": 9, "name": "cargo", "args": "cargo.exe", "elapsed": 2400, "rss": 5, "cpu": 1.0}],
                "markers": [{"text": "\ufeff" + marker(40, 1000, slot=1, checkout="OCW-worktree-agent-abcdef1234"),
                             "alive": True, "pstart": 999},
                            {"text": marker(41, 1010, "waiting", checkout="OCW"), "alive": True, "pstart": 1009},
                            {"text": marker(42, 500), "alive": False, "pstart": None}]}
        out = d.parse_windows(json.dumps(data), "B:\\src\\OCW")
        self.assertEqual(out["cpu_busy"], 93.9)
        self.assertEqual(out["runnable"], 7)
        self.assertEqual(out["counters"]["hint"], d.PERFMON_HINT)
        self.assertEqual(out["stale_markers"], 1)
        t = out["tasks"]
        self.assertEqual([(x["checkout"], x["state"], x["source"]) for x in t],
                         [("agent abcdef12", "running", "marker"), ("main", "waiting", "marker"),
                          ("other", "running", "process")])
        # The run's own processes (started after it) are adopted, the older cargo is not.
        self.assertEqual((t[0]["elapsed"], t[0]["cpu"], t[0]["rss"]), (60, 139.4, 1010))
        self.assertEqual((t[2]["cpu"], t[2]["rss"]), (1.0, 5))
        q = out["queue"]
        self.assertEqual((q["used"], q["waiting"]), (1, 1))
        text = d.format_text(d.summarize([dict(name="Win11Builder", os="windows", online=True, polled_at=0, **out)]))
        self.assertIn("processor queue n/a (add build user to Performance Monitor Users)", text)
        self.assertIn("queue: [main] cargo test  (waiting 50s)", text)
        self.assertNotIn("[main] waiting", text)  # queued runs are listed in the queue, not as tasks
        self.assertIn("running (slot 1)", text)
        self.assertIn("1 stale task marker", text)

    def test_linux_polling_waiters(self):
        # New remote.sh: waiters poll with `flock -n` (no blocked flock
        # process); the queue comes from their markers. A waiter that has a
        # slot file open for a moment (fuser) is not its holder.
        ps = PS.replace("    301     300  1002    99   1000 flock 9\n", "").replace(
            "    401     400  1002    49   1000 flock 9\n", "")
        text = (LINUX_OUTPUT.replace(PS, ps)
                .replace("ocw-build-slot-2.lock 4445 :", "ocw-build-slot-2.lock 4445 : 400")
                .replace("9: -> FLOCK  ADVISORY  WRITE 301 00:19:4444 0 EOF\n9: -> FLOCK  ADVISORY  WRITE 401 00:19:4444 0 EOF\n",
                         "10: FLOCK  ADVISORY  WRITE 510 00:19:4445 0 EOF\n")
                .replace("@@end\n", "@@now\n2000\n@@tasks\n"
                         + marker(200, 1600, slot=1, checkout="OCW-worktree-agent-a08d34a9817282b1d") + "\n"
                         + marker(300, 1900, "waiting", command="cargo build --release") + "\n"
                         + marker(400, 1950, "waiting", checkout="OCW-fix-x", command="cargo clippy") + "\n@@end\n"))
        out = d.parse_linux(text, "~/src/OCW")
        # The polling shells of waiting runs are not listed as running tasks.
        self.assertEqual([(t["checkout"], t["state"]) for t in out["tasks"]],
                         [("agent a08d34a9", "running"), ("main", "waiting"), ("fix-x", "waiting")])
        q = out["queue"]
        self.assertEqual((q["used"], q["total"], q["waiting"]), (2, 2, 2))
        self.assertEqual([(s["slot"], s["holder"]) for s in q["slots"]], [(1, "agent a08d34a9"), (2, None)])
        self.assertEqual([(w["checkout"], w["command"], w["elapsed"]) for w in q["queued"]],
                         [("main", "cargo build --release", 100), ("fix-x", "cargo clippy", 50)])
        q = d.parse_linux(text, "~/src/OCW", slots=3)["queue"]
        self.assertEqual((q["used"], q["total"]), (2, 3))

    def test_macos_parse_with_slots_and_queue(self):
        text = ("@@self\n50\n@@home\n/Users/ocw\n501\n@@sysctl\n8\n17179869184\n@@now\n2000\n"
                "@@ps\n  60 1 501 05:00 3000 zsh -c ocw_meta=x; ocw_mark running $ocw_n; cd ~/src/OCW-fix-x && . ~/.cargo/env && sleep 60\n"
                "  61 1 501 00:30 3000 zsh -c ocw_meta=x; ocw_mark running $ocw_n; cd ~/src/OCW && . ~/.cargo/env && sleep 60\n"
                "@@tasks\n" + marker(60, 1700, slot=1, checkout="OCW-fix-x", command="sleep 60") + "\n"
                + marker(61, 1970, "waiting", command="sleep 60") + "\n"
                "@@slots\nocw-build-slot-1.d 60\n@@cwd\n@@end\n")
        out = d.parse_macos(text, "~/src/OCW")
        q = out["queue"]
        self.assertEqual((q["used"], q["total"], q["waiting"]), (1, 1, 1))
        self.assertEqual(q["slots"][0]["holder"], "fix-x")
        self.assertEqual(q["queued"][0]["checkout"], "main")
        text = d.format_text(d.summarize([dict(name="macos", os="macos", online=True, polled_at=0, **out)]))
        self.assertIn("1/1 machines online: 1 running, 1 waiting, build slots 1 used of 1", text)
        self.assertIn("queue: [main] sleep 60  (waiting 30s)", text)
        self.assertIn("[fix-x] running (slot 1): sleep 60", text)

    def test_linux_parse_with_markers_and_ticks(self):
        extra = ("@@now\n2000\n@@clk\n100\n@@ticks1\n1000.00 9000.00\n203 1000\n204 50\n"
                 "@@ticks2\n1000.50 9001.00\n203 1100\n204 75\n@@stat2\n"
                 "@@tasks\n" + marker(200, 1600, slot=1, checkout="OCW-worktree-agent-a08d34a9817282b1d") + "\n"
                 + marker(300, 1900, "waiting", command="cargo build --release") + "\n" + marker(999, 1000) + "\n")
        out = d.parse_linux(LINUX_OUTPUT.replace("@@end\n", extra + "@@end\n"), "~/src/OCW")
        self.assertEqual(out["cpu_busy"], 50.0)
        self.assertEqual(out["stale_markers"], 1)
        t = {x["checkout"]: x for x in out["tasks"]}
        a = t["agent a08d34a9"]
        self.assertEqual((a["state"], a["slot"], a["elapsed"], a["source"]), ("running", 1, 400, "marker"))
        self.assertEqual(a["cpu"], 250.0)  # rustc 203: 200 %, rustc 204: 50 %
        self.assertEqual(t["main"]["state"], "waiting")
        self.assertEqual(t["main"]["command"], "cargo build --release")
        st = d.summarize([dict(name="marge", os="linux", online=True, polled_at=0, **out)])
        self.assertEqual(st["overview"]["tasks"], 1)  # the waiting run is not counted as running


if __name__ == "__main__":
    unittest.main()


class RecordTest(unittest.TestCase):
    def test_record_line_is_one_json_line_with_time(self):
        line = d.record_line({"machines": [{"name": "marge", "online": True}]}, now=0)
        self.assertNotIn("\n", line)
        rec = json.loads(line)
        self.assertRegex(rec["time"], r"^1970-01-0[12]T\d\d:\d\d:\d\d([+-]\d\d:\d\d)?$")
        self.assertEqual(rec["status"]["machines"][0]["name"], "marge")

    def test_record_path_per_local_day(self):
        p = d.record_path(Path("/x/build-log"), now=86400 * 3)
        self.assertEqual(p.name, "servers.jsonl")
        self.assertRegex(p.parent.name, r"^1970-01-0[34]$")


class PromptTest(unittest.TestCase):
    def _index(self):
        idx = d.PromptIndex(Path("/nonexistent"))
        idx.feed({"type": "user", "timestamp": "2026-11-13T05:00:00Z",
                  "message": {"content": "fix the double shadow\nsecond line"}})
        idx.feed({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "tu1", "name": "Agent", "input": {"description": "Fix shadow"}}]}})
        idx.feed({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "tu1", "content": "launched. agentId: aca62f4f05d964134 (x)"}]}})
        idx.feed({"type": "attachment", "timestamp": "2026-11-13T06:00:00Z", "attachment": {
            "type": "queued_command", "prompt": "add prompts to the dashboard", "origin": {"kind": "human"}}})
        idx.feed({"type": "user", "timestamp": "2026-11-13T06:30:00Z",
                  "message": {"content": "<task-notification>agent finished</task-notification>"}})
        return idx

    def test_not_prompts_are_ignored(self):
        self.assertIsNone(d.prompt_from_text("<task-notification>x</task-notification>"))
        self.assertIsNone(d.prompt_from_text("<system-reminder>only</system-reminder>"))
        self.assertEqual(d.prompt_from_text("<command-name>/auto-mode-setup</command-name>"), "/auto-mode-setup")

    def test_agent_job_gets_the_prompt_that_started_it(self):
        info = self._index().for_job("OCW-worktree-agent-aca62f4f05d964134", 0)
        self.assertEqual(info, {"prompt": "fix the double shadow", "agent": "Fix shadow"})

    def test_unknown_agent_gets_nothing_other_jobs_the_latest_prompt(self):
        idx = self._index()
        self.assertEqual(idx.for_job("OCW-worktree-agent-ffffffffffffffff", 0), {})
        started = d._epoch("2026-11-13T06:10:00Z")
        self.assertEqual(idx.for_job("main", started), {"prompt": "add prompts to the dashboard"})
        early = d._epoch("2026-11-13T05:30:00Z")
        self.assertEqual(idx.for_job("main", early), {"prompt": "fix the double shadow"})

    def test_short_prompt(self):
        self.assertEqual(d.short_prompt("a\nb"), "a")
        s = d.short_prompt("x" * 200, 20)
        self.assertEqual(len(s), 20)
        self.assertTrue(s.endswith("…"))

    def test_annotate_adds_prompt_and_masks(self):
        st = {"machines": [{"tasks": [{"checkout": "OCW-worktree-agent-aca62f4f05d964134", "command": "c",
                                       "state": "running", "started": 0}]}]}
        out = d.annotate_prompts(st, self._index(), mask=lambda s: s.replace("double", "<m>"))
        t = out["machines"][0]["tasks"][0]
        self.assertEqual(t["prompt"], "fix the <m> shadow")
        self.assertEqual(t["agent"], "Fix shadow")
