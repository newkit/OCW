#!/usr/bin/env python3
"""Multiplayer in the browser, end to end (spec §14 Phase 10 B3; §11.4).

    scripts/test-web-multiplayer.py [--out test-output/web] [--seconds 30]
        [--bin target/release] [--riders 2] [--chromium chromium] [--software]

On one machine (marge): starts `ocw-server --listen --webtransport` (the
test route Hills, no server bots), `ocw-bot` riders over native QUIC
(`--riders`, following the browser rider so they stay in its picture) and
one `ocw-bot` rider over WebTransport (the same transport as the browser,
for the comparison), serves target/web/ (`make web`) and starts headless
Chromium on WebGPU with `?server=https://127.0.0.1:PORT&cert-hash=…&name=
Browser&route=hills`. The browser rides `--seconds` after it joined, then
it saves:

  OUT/web-mp-<time>.jpg     the browser's view with the other riders (HUD
                            with the ISO timestamp)
  OUT/web-mp-<time>.log     browser console, server and bot output
  OUT/web-mp-<time>.json    the summary: what the browser saw, what the
                            server saw, snapshot timing browser vs native

Exits 0 when the browser joined over WebTransport, saw at least --riders
other riders, rode, the server applied its inputs (the browser rider's
power on the server, `last_input_seq`, WebTransport datagrams in the
server statistics) and the bots saw the browser rider; 1 otherwise.
"""

import argparse
import datetime
import importlib.util
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tw = module("test_web", "test-web.py")


def free_udp_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Proc:
    """A child process whose output is collected line by line."""

    def __init__(self, label, cmd):
        self.label, self.cmd = label, cmd
        self.lines = []
        # Plain log lines (no ANSI colours) to parse.
        env = {**os.environ, "NO_COLOR": "1"}
        self.p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, bufsize=1, env=env)
        self.t = threading.Thread(target=self._read, daemon=True)
        self.t.start()

    def _read(self):
        for line in self.p.stdout:
            self.lines.append((time.time(), line.rstrip("\n")))

    def text(self):
        return "\n".join(l for _, l in self.lines)

    def wait_for(self, pattern, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            for _, l in self.lines:
                m = re.search(pattern, l)
                if m:
                    return m
            if self.p.poll() is not None:
                break
            time.sleep(0.05)
        return None

    def stop(self, sig=signal.SIGTERM, timeout=10):
        if self.p.poll() is None:
            self.p.send_signal(sig)
            try:
                self.p.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                self.p.kill()
                self.p.wait()
        self.t.join(timeout=2)


def start_chromium(a, log):
    w, h = a.size.split("x")
    profile = tempfile.mkdtemp(prefix="ocw-chromium-")
    flags = [
        "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={profile}",
        "--no-first-run", "--no-default-browser-check", "--disable-extensions",
        "--no-proxy-server", "--password-store=basic",
        f"--window-size={w},{h}", "--hide-scrollbars", "--mute-audio",
        "--enable-unsafe-webgpu", "--ignore-gpu-blocklist",
        "--enable-features=Vulkan,WebGPUService",
        "--use-angle=vulkan", "--enable-gpu-rasterization",
        # The test runs in the background: keep the page's timers and
        # frames at full rate.
        "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
        "--disable-backgrounding-occluded-windows",
    ]
    if a.software:
        flags += ["--use-webgpu-adapter=swiftshader", "--enable-unsafe-swiftshader"]
    cmd_r, cmd_w = os.pipe()
    res_r, res_w = os.pipe()
    os.set_inheritable(cmd_r, True)
    os.set_inheritable(res_w, True)

    def child_fds():
        os.dup2(cmd_r, 3)
        os.dup2(res_w, 4)

    proc = subprocess.Popen([a.chromium, *flags, "about:blank"], preexec_fn=child_fds,
                            close_fds=False, stdout=log, stderr=subprocess.STDOUT)
    os.close(cmd_r)
    os.close(res_w)
    cdp = tw.Cdp(proc, cmd_w, res_r)
    targets = cdp.call("Target.getTargets", session=False)["targetInfos"]
    page = next(t for t in targets if t["type"] == "page")
    cdp.session = cdp.call("Target.attachToTarget",
                           {"targetId": page["targetId"], "flatten": True},
                           session=False)["sessionId"]
    for m in ("Runtime.enable", "Log.enable", "Page.enable"):
        cdp.call(m)
    cdp.call("Emulation.setDeviceMetricsOverride",
             {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})
    return proc, cdp, profile, flags


def server_rider_lines(server, name):
    """The server's once-a-second lines of rider `name`: (power, distance km)."""
    out = []
    for _, l in server.lines:
        if f"name={name} " not in l and not l.endswith(f"name={name}"):
            continue
        p = re.search(r"power_w=([\d.]+)", l)
        d = re.search(r'distance_km="?([\d.]+)', l)
        if p and d:
            out.append((float(p.group(1)), float(d.group(1))))
    return out


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--bin", default="target/release", help="where ocw-server and ocw-bot are")
    p.add_argument("--seconds", type=float, default=30.0,
                   help="seconds the browser rides after it joined")
    p.add_argument("--riders", type=int, default=2, help="ocw-bot riders over native QUIC")
    p.add_argument("--no-wt-bot", action="store_true",
                   help="no ocw-bot rider over WebTransport (the native comparison)")
    p.add_argument("--name", default="Browser")
    p.add_argument("--desktop", action="store_true",
                   help="also a desktop client (ocw-desktop --server, headless X11 via "
                        ".claude/skills/run-ocw/drive.sh) in the session, with its screenshot")
    p.add_argument("--timeout", type=float, default=120.0)
    p.add_argument("--time", default="noon",
                   help="the time machine once the browser joined (console `time …`; the server "
                        "runs with --time-control anyone), '' for the real time")
    p.add_argument("--chromium", default=shutil.which("chromium") or "chromium")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--software", action="store_true")
    a = p.parse_args()
    for f in (os.path.join(a.dir, "ocw_web_bg.wasm"), os.path.join(a.bin, "ocw-server"),
              os.path.join(a.bin, "ocw-bot")):
        if not os.path.exists(f):
            sys.exit(f"{f} missing: run `make web` and `cargo build --release -p ocw-server -p ocw-bot`")
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, "web-mp-" + stamp)
    work = tempfile.mkdtemp(prefix="ocw-web-mp-")
    quic, wt = free_udp_port(), free_udp_port()
    stats_file = os.path.join(work, "server-stats.json")
    info_file = os.path.join(work, "web.json")
    # The bots ride until the server shuts down (then they report).
    bot_seconds = 0
    summary = {"ok": False, "seconds": a.seconds, "riders": a.riders}
    lines = []
    procs = []
    server = bots = wt_bot = None
    proc = cdp = profile = httpd = None
    log = open(base + ".log", "w")
    try:
        server = Proc("server", [
            os.path.join(a.bin, "ocw-server"), "--listen", f"127.0.0.1:{quic}",
            "--webtransport", f"127.0.0.1:{wt}", "--web-info-file", info_file,
            "--stats-file", stats_file, "--test-route", "hills", "--bots", "0",
            "--time-control", "anyone"])
        procs.append(server)
        if not server.wait_for(r"listening \(WebTransport", 20):
            raise RuntimeError("the server did not start:\n" + server.text())
        info = json.load(open(info_file))
        summary["webtransport"] = {k: info[k] for k in ("url", "cert_hash", "not_after_unix_s")}
        follow = ["--follow", a.name]
        bots = Proc("bots-quic", [
            os.path.join(a.bin, "ocw-bot"), "--server", f"127.0.0.1:{quic}", "--insecure-dev",
            "--riders", str(a.riders), "--seconds", str(bot_seconds), "--route", "hills",
            "--label", "quic", "--name-prefix", "quic", *follow])
        procs.append(bots)
        if not a.no_wt_bot:
            wt_bot = Proc("bot-webtransport", [
                os.path.join(a.bin, "ocw-bot"), "--server", info["url"],
                "--server-cert", info["cert_hash"], "--riders", "1",
                "--seconds", str(bot_seconds), "--route", "hills", "--label", "webtransport",
                "--name-prefix", "wt", "--index", str(a.riders), *follow])
            procs.append(wt_bot)

        desktop = None
        if a.desktop:
            # Started now, it rides while the browser loads and rides; its
            # screenshot comes after DELAY s.
            delay = str(int(max(10, a.seconds - 5)))
            desktop = Proc("desktop", [
                os.path.join(HERE, "..", ".claude", "skills", "run-ocw", "drive.sh"), "shot",
                base + "-desktop.jpg", delay, "--", "--server", f"127.0.0.1:{quic}",
                "--insecure-dev", "--route", "hills", "--name", "Desktop"])
            procs.append(desktop)
        stats = {}
        httpd = tw.web_serve().server(a.dir, stats=stats)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        query = f"{info['query']}&name={a.name}&route=hills"
        url = f"http://127.0.0.1:{httpd.server_address[1]}/?{query}"
        summary["url"] = url
        proc, cdp, profile, flags = start_chromium(a, log)
        summary["flags"] = flags
        t0 = time.time()
        cdp.call("Page.navigate", {"url": url})
        joined = connected = None
        errors = []
        last = {}
        shot_at = None
        deadline = t0 + a.timeout + a.seconds
        seen = 0
        net = None
        while time.time() < deadline:
            cdp.pump(0.2)
            for e in cdp.events[seen:]:
                now = time.time() - t0
                if e["method"] == "Runtime.consoleAPICalled":
                    text = tw.console_text(e)
                    lines.append(f"[browser] {now:7.2f} s  {e['params'].get('type', 'log'):7} {text}")
                    if "WebTransport: connected" in text and connected is None:
                        connected = now
                    if "joined the remote server" in text and joined is None:
                        joined = now
                        if a.time:
                            cdp.eval(f"window.ocw && window.ocw.console('time {a.time}')")
                    if "remote server running" in text:
                        last = dict(re.findall(r'(\w+)\s*=\s*"?([\w.]+)', text))
                    if e["params"].get("type") == "error":
                        errors.append(text)
                elif e["method"] == "Runtime.exceptionThrown":
                    d = e["params"]["exceptionDetails"]
                    text = d.get("exception", {}).get("description") or d.get("text", "")
                    if "Using exceptions for control flow" not in text:
                        errors.append(text)
                    lines.append(f"[browser] {now:7.2f} s  EXCEPTION {text}")
                elif e["method"] == "Log.entryAdded":
                    entry = e["params"]["entry"]
                    lines.append(f"[browser] {now:7.2f} s  {entry['level']:7} [{entry['source']}] {entry['text']}")
            seen = len(cdp.events)
            now = time.time() - t0
            if joined is None and now > a.timeout:
                break
            if joined is not None and now >= joined + a.seconds:
                net = cdp.eval("window.ocw && window.ocw.netStats ? window.ocw.netStats() : ''")
                summary["screenshot"] = cdp.screenshot(base + ".jpg")
                shot_at = now
                break
            if any("Disconnected" in x or "WebTransport: closed" in x for x in errors):
                break
        summary.update({"connected_s": connected, "joined_s": joined, "screenshot_s": shot_at,
                        "browser_last": last, "errors": errors})
        browser = json.loads(net) if net else {}
        summary["browser"] = browser
        if shot_at is None:
            summary["screenshot_failed"] = cdp.screenshot(base + "-failed.jpg")
    except Exception as e:  # noqa: BLE001 – reported in the summary
        summary["error"] = repr(e)
    finally:
        if cdp is not None:
            try:
                cdp.send("Browser.close", session=False)
                proc.wait(timeout=10)
            except Exception:  # noqa: BLE001
                proc.kill()
        if httpd is not None:
            httpd.shutdown()
        if profile:
            shutil.rmtree(profile, ignore_errors=True)
        if a.desktop and desktop is not None:
            try:
                desktop.p.wait(timeout=a.seconds + 240)
            except subprocess.TimeoutExpired:
                desktop.stop(signal.SIGKILL)
        # The server (its statistics; it closes every connection), then the
        # bots end on their own with their reports.
        if server is not None:
            server.stop(signal.SIGTERM, timeout=15)
        for b in (bots, wt_bot):
            if b is not None:
                try:
                    b.p.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    pass
                b.stop(signal.SIGKILL)

    # What the server saw.
    srv = {}
    if os.path.exists(stats_file):
        srv = json.load(open(stats_file))
    browser_on_server = server_rider_lines(server, a.name) if server else []
    summary["server"] = {
        "joined": srv.get("joined"), "web_sessions": srv.get("web_sessions"),
        "web_messages_in": srv.get("web_messages_in"), "web_datagrams_in": srv.get("web_datagrams_in"),
        "peak_riders": srv.get("peak_riders"), "max_late_ms": srv.get("max_late_ms"),
        "browser_power_w": sorted({p for p, _ in browser_on_server}),
        "browser_distance_km": [browser_on_server[0][1], browser_on_server[-1][1]] if browser_on_server else None,
    }
    # What the bots saw (and their snapshot timing).
    reports = []
    for b in (bots, wt_bot):
        if b is None:
            continue
        for r in b.text().splitlines():
            if "OCW-BOT-REPORT " in r:
                reports.append(json.loads(r.split("OCW-BOT-REPORT ", 1)[1]))
    bot_riders = [r for rep in reports for r in rep["riders"]]
    summary["bots"] = [{
        "name": r["name"], "label": rep["label"], "snapshots": r["snapshots"],
        "saw_browser": a.name in r.get("others_seen", []), "others_seen": r.get("others_seen"),
        "timing": r.get("timing"), "rtt_ms_max": r.get("rtt_ms_max"),
    } for rep in reports for r in rep["riders"]]
    # Browser vs native: interval, jitter and age of the snapshots.
    def pick(t):
        if not t:
            return None
        return {k: round(t[k], 2) if isinstance(t.get(k), float) else t.get(k) for k in (
            "snapshots", "stale", "interval_mean_ms", "jitter_ms", "interval_p95_ms",
            "interval_p99_ms", "interval_max_ms", "age_mean_ms", "age_p95_ms", "age_max_ms")}
    summary["timing"] = {
        "browser_webtransport": pick(summary.get("browser", {}).get("timing")),
        **{f"bot_{b['label']}_{b['name']}": pick(b["timing"]) for b in summary["bots"]},
    }
    b = summary.get("browser", {})
    others = b.get("others_seen") or []
    checks = {
        "browser joined over WebTransport": summary.get("joined_s") is not None,
        f"browser saw >= {a.riders} other riders": len(others) >= a.riders,
        "browser rode": float(b.get("distance_m") or 0) > 20,
        "server applied the browser's inputs (last_input_seq)": (b.get("acked_seq") or 0) > 20,
        "server rode the browser rider at its power": 200.0 in summary["server"]["browser_power_w"],
        "server got WebTransport datagrams": (srv.get("web_datagrams_in") or 0) > 20,
        "server saw the browser join": a.name in (srv.get("joined") or []),
        "every bot saw the browser rider": bool(bot_riders) and all(
            a.name in r.get("others_seen", []) for r in bot_riders),
        "screenshot": summary.get("screenshot") is not None,
        **({"browser saw the desktop rider": "Desktop" in others,
            "server saw the desktop join": "Desktop" in (srv.get("joined") or []),
            "desktop screenshot": os.path.exists(base + "-desktop.jpg")} if a.desktop else {}),
        "no browser errors": not summary.get("errors") and "error" not in summary,
    }
    summary["checks"] = checks
    summary["ok"] = all(checks.values())
    for pr in procs:
        lines.append(f"\n--- {pr.label}: {' '.join(pr.cmd)} ---")
        lines.extend(l for _, l in pr.lines)
    log.write("\n".join(lines) + "\n")
    log.write("\n--- summary ---\n" + json.dumps(summary, indent=2) + "\n")
    log.close()
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    shutil.rmtree(work, ignore_errors=True)
    print(json.dumps({k: summary.get(k) for k in (
        "ok", "checks", "joined_s", "browser_last", "server", "timing", "errors", "error")}, indent=2))
    print(f"log: {base}.log  screenshot: {summary.get('screenshot')}")
    sys.exit(0 if summary["ok"] else 1)


if __name__ == "__main__":
    main()
