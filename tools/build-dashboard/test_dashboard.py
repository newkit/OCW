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


class MaskTest(unittest.TestCase):
    def test_masks_hosts_users_addresses(self):
        mk = d.Masker(["win.example.lan", "claude", "10.1.2.3"], ["/home/claude"])
        text = ("ssh: connect to host win.example.lan port 22: Connection refused; "
                "10.1.2.3 192.168.0.7:22 fe80::1%eth0 2001:db8:0:0:0:0:0:1 "
                "builder@other.lan /home/claude/src/OCW /home/nils/x C:\\Users\\Someone\\.cargo "
                "AA:BB:CC:DD:EE:FF at 21:32:41")
        out = mk(text)
        for leak in ("win.example.lan", "claude", "10.1.2.3", "192.168", "fe80", "2001:db8",
                     "builder@", "nils", "Someone", "AA:BB", "port 22"):
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


if __name__ == "__main__":
    unittest.main()
