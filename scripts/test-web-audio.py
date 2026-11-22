#!/usr/bin/env python3
"""Headless browser test of the web app's sound and voice (spec §14 Phase 10 B5; §11.4).

    scripts/test-web-audio.py [--out test-output/web] [--part audio|voice|livekit|all]
                              [--bin target/release] [--chromium chromium]
                              [--livekit-server livekit-server]

Serves target/web/ (`make web`), starts headless Chromium on WebGPU (the
sound muted with --mute-audio: the context runs, nothing reaches the
speakers; the autoplay rule enforced with
--autoplay-policy=document-user-activation-required; Chromium's fake
microphone, a beeping tone, for getUserMedia) and drives the page over the
DevTools protocol (scripts/test-web.py's Cdp).

--part audio (`make test-web-audio`): the local server with bots. The game
  sound is locked before a gesture (no chunk scheduled, the hint shown);
  a real mouse click (Input.dispatchMouseEvent: a trusted user activation)
  resumes the context; the mixer's chunks are scheduled back to back and
  carry sound (the own bike, the bots); `audio.play finish`; a volume set
  in the panel's way (`set audio.master 55`) survives a reload
  (localStorage) and the sound waits for a gesture again after it.

--part voice (`make test-web-voice`): ocw-server with voice (a LiveKit
  URL and keys for tokens; a fake LiveKit in the page,
  scripts/web-voice-fake.js) and the Echo rider, an ocw-bot rider over
  QUIC. The browser joins over WebTransport; the shared controller asks for
  a token over the game connection; the shim connects with it (the fake
  reads the token's claims: identity and room); the bot's participant is
  subscribed and placed; M mutes the microphone (the shim's
  setMicrophoneEnabled(false), the HUD), Shift+M the speaker (volume 0);
  active speakers reach the HUD; a network drop shows reconnecting, a lost
  connection fails and rejoins with a new token; the echo test (`voice.echo
  here`) runs the shim's loopback on Chromium's fake microphone (input and
  echo levels); every ride is joined muted, also after a reload (spec
  §7.1.7 Q7); the bot lets all nearby hear it (`ocw-bot --voice-nearby`).

--part livekit (`make test-web-livekit`, when a `livekit-server` binary is
  there): a real `livekit-server --dev` on localhost, the real
  livekit-client in three pages (three browser riders): all connect with
  the game server's tokens, see each other, hear the others' fake
  microphones (active speakers), a mute in one page shows in the other;
  then Ada blocks Bea (spec §7.1.7 Q9): the game server tells both, neither
  plays the other's track (Bea's own request for Ada's is refused by the
  voice server), Cleo plays both; unblocked, all as before. Every page
  joins muted and heard by no one (§7.1.7 Q7, Q8), then lets all nearby
  hear it. Range and audience (§7.1.7 Q6, Q8, Q10, Q20; the server with
  --dev-commands, the pages stop and teleport): a rider 20 m away is not
  played and plays nobody, the game server sends her no voice and hers to
  nobody, her own request for Ada's track is refused; 1.1 km away with
  Voice room = Selected riders (Ada) she plays Ada; back near, all play again;
  Ada's *Who hears me: Selected riders* (Bea) and Cleo's *Voice room:
  Selected riders* hold.

Not testable headlessly: what the sound sounds like on speakers (the
mixer's output is checked as numbers: chunks, level), a real microphone
and its permission prompt (the fake device answers), echo cancellation,
output device choice (setSinkId with real devices), Safari's
"interrupted" state.

Saves OUT/web-audio-<time>[-<step>].jpg, .log and .json. Exits 0 when every
check of the parts run passed.
"""

import argparse
import base64
import datetime
import importlib.util
import json
import os
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web_browser  # noqa: E402  (scripts/web_browser.py: the pinned browser)

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tw = module("test_web", "test-web.py")
mp = module("test_web_multiplayer", "test-web-multiplayer.py")


def write_tone(path, seconds=4.0, rate=48000):
    """A mono WAV: 220 Hz with its harmonics, swelling like speech."""
    import math
    import struct
    import wave
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        frames = bytearray()
        for i in range(int(seconds * rate)):
            t = i / rate
            env = 0.5 + 0.5 * math.sin(2 * math.pi * 3 * t)
            x = sum(math.sin(2 * math.pi * 220 * k * t) / k for k in (1, 2, 3))
            frames += struct.pack("<h", int(9000 * env * x / 1.8))
        w.writeframes(bytes(frames))


def free_tcp_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class Run:
    """One headless Chromium, its pages, the log, the checks."""

    def __init__(self, a, base, flags_extra=()):
        # The fake microphone plays this file: a voice-like tone, all the
        # time (Chromium's own fake device only beeps now and then).
        mic = base + ".mic.wav"
        write_tone(mic)
        self.a, self.base = a, base
        self.lines, self.checks, self.summary, self.errors = [], {}, {}, []
        self.t0 = time.time()
        self.seen = 0
        w, h = a.size.split("x")
        self.profile = tempfile.mkdtemp(prefix="ocw-chromium-")
        flags = [
            "--headless=new", "--remote-debugging-pipe", f"--user-data-dir={self.profile}",
            "--no-first-run", "--no-default-browser-check", "--disable-extensions",
            "--no-proxy-server", "--password-store=basic",
            f"--window-size={w},{h}", "--hide-scrollbars",
            # The context runs, the speakers stay quiet (marge's kiosk).
            "--mute-audio",
            # The autoplay rule as on a real first visit.
            "--autoplay-policy=document-user-activation-required",
            # A fake microphone (a beep) and no permission prompt.
            "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
            f"--use-file-for-fake-audio-capture={mic}",
            "--enable-unsafe-webgpu", "--ignore-gpu-blocklist",
            "--enable-features=Vulkan,WebGPUService",
            "--use-angle=vulkan", "--enable-gpu-rasterization",
            "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            *flags_extra,
        ]
        if a.software:
            flags += ["--use-webgpu-adapter=swiftshader", "--enable-unsafe-swiftshader"]
        self.summary["flags"] = flags
        cmd_r, cmd_w = os.pipe()
        res_r, res_w = os.pipe()
        os.set_inheritable(cmd_r, True)
        os.set_inheritable(res_w, True)

        def child_fds():
            os.dup2(cmd_r, 3)
            os.dup2(res_w, 4)

        self.blog = open(base + ".chromium.log", "w")
        self.proc = subprocess.Popen([a.chromium, *flags, "about:blank"], preexec_fn=child_fds,
                                     close_fds=False, stdout=self.blog, stderr=subprocess.STDOUT)
        os.close(cmd_r)
        os.close(res_w)
        self.cdp = tw.Cdp(self.proc, cmd_w, res_r)
        targets = self.cdp.call("Target.getTargets", session=False)["targetInfos"]
        page = next(t for t in targets if t["type"] == "page")
        self.pages = {}
        self.attach("main", page["targetId"])

    def attach(self, name, target_id):
        sid = self.cdp.call("Target.attachToTarget", {"targetId": target_id, "flatten": True},
                            session=False)["sessionId"]
        self.pages[name] = sid
        self.use(name)
        for m in ("Runtime.enable", "Log.enable", "Page.enable"):
            self.cdp.call(m)
        w, h = self.a.size.split("x")
        self.cdp.call("Emulation.setDeviceMetricsOverride",
                      {"width": int(w), "height": int(h), "deviceScaleFactor": 1, "mobile": False})

    def new_page(self, name):
        # Its own window: a second tab in the same window would be hidden,
        # and a hidden page gets no animation frames – its game loop (and
        # with it the join, the voice controller) stands still.
        tid = self.cdp.call("Target.createTarget", {"url": "about:blank", "newWindow": True},
                            session=False)["targetId"]
        self.attach(name, tid)

    def use(self, name):
        self.cdp.session = self.pages[name]
        self.page = name

    def now(self):
        return time.time() - self.t0

    def pump(self, seconds):
        deadline = time.time() + seconds
        while True:
            self.cdp.pump(min(0.1, max(0.0, deadline - time.time())))
            for e in self.cdp.events[self.seen:]:
                page = next((n for n, s in self.pages.items() if s == e.get("sessionId")), "?")
                t = self.now()
                if e["method"] == "Runtime.consoleAPICalled":
                    text = tw.console_text(e)
                    self.lines.append(f"[{page}] {t:7.2f} s  {e['params'].get('type', 'log'):7} {text}")
                    if e["params"].get("type") == "error" and "favicon" not in text:
                        self.errors.append(f"{page}: {text}")
                elif e["method"] == "Runtime.exceptionThrown":
                    d = e["params"]["exceptionDetails"]
                    text = d.get("exception", {}).get("description") or d.get("text", "")
                    if "Using exceptions for control flow" not in text:
                        self.errors.append(f"{page}: {text}")
                    self.lines.append(f"[{page}] {t:7.2f} s  EXCEPTION {text}")
                elif e["method"] == "Log.entryAdded":
                    en = e["params"]["entry"]
                    self.lines.append(f"[{page}] {t:7.2f} s  {en['level']:7} [{en['source']}] {en['text']}")
            self.seen = len(self.cdp.events)
            if time.time() >= deadline:
                return

    def js(self, expr):
        return self.cdp.eval(expr)

    def json(self, expr):
        v = self.js(f"(() => {{ try {{ return {expr}; }} catch (e) {{ return ''; }} }})()")
        try:
            return json.loads(v) if v else None
        except (TypeError, ValueError):
            return None

    def audio(self):
        return self.json("window.ocw && window.ocw.audio()")

    def voice(self):
        return self.json("window.ocw && window.ocw.voice()")

    def console(self, line):
        self.js(f"window.ocw && window.ocw.console({json.dumps(line)})")

    def wait(self, what, pred, timeout):
        """Pumps until pred() is truthy; its value (or None)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.pump(0.25)
            try:
                v = pred()
            except Exception:  # noqa: BLE001 – not there yet
                v = None
            if v:
                return v
        self.lines.append(f"[test] {self.now():7.2f} s  timed out waiting for {what}")
        return None

    def check(self, name, ok, detail=None):
        self.checks[name] = bool(ok)
        self.lines.append(f"[check] {'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail is not None else ""))
        return bool(ok)

    def click(self, x=None, y=None):
        w, h = (int(v) for v in self.a.size.split("x"))
        x = w // 2 if x is None else x
        y = h // 3 if y is None else y
        for t in ("mousePressed", "mouseReleased"):
            self.cdp.call("Input.dispatchMouseEvent", {"type": t, "x": x, "y": y, "button": "left", "clickCount": 1})

    def key(self, code, key, shift=False):
        mods = 8 if shift else 0
        if shift:
            self.cdp.call("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Shift", "code": "ShiftLeft",
                                                     "windowsVirtualKeyCode": 16, "modifiers": 8})
        vk = ord(key.upper()) if len(key) == 1 else {"ArrowDown": 40, "ArrowUp": 38}.get(key, 0)
        down = {"type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk,
                "modifiers": mods}
        if len(key) == 1:
            down["text"] = key
        self.cdp.call("Input.dispatchKeyEvent", down)
        self.pump(0.1)
        self.cdp.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": key, "code": code,
                                                 "windowsVirtualKeyCode": vk, "modifiers": mods})
        if shift:
            self.cdp.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Shift", "code": "ShiftLeft",
                                                     "windowsVirtualKeyCode": 16})

    def shot(self, step):
        path = f"{self.base}-{step}.jpg"
        self.cdp.screenshot(path)
        self.summary.setdefault("screenshots", []).append(path)
        self.lines.append(f"[test] {self.now():7.2f} s  screenshot {path}")
        return path

    def close(self):
        try:
            self.cdp.send("Browser.close", session=False)
            self.proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            self.proc.kill()
        self.blog.close()
        shutil.rmtree(self.profile, ignore_errors=True)


def serve(a):
    stats = {}
    httpd = tw.web_serve().server(a.dir, stats=stats)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def wait_ride(r, pattern, timeout):
    return r.wait(pattern, lambda: any(re.search(pattern, l) for l in r.lines), timeout)


# ---------------------------------------------------------------- audio


def part_audio(r, port):
    url = f"http://127.0.0.1:{port}/?bots=3&power=250&name=Sound"
    r.cdp.call("Page.navigate", {"url": url})
    if not wait_ride(r, r"local server running", 60):
        r.check("audio: the ride started", False)
        return
    r.pump(1.0)
    a0 = r.wait("audio state", r.audio, 10) or {}
    r.summary["audio_before_gesture"] = a0
    r.check("audio: locked before a gesture (the autoplay rule)", a0.get("state") == "locked", a0.get("state"))
    r.check("audio: nothing scheduled before the gesture", a0.get("chunks") == 0, a0.get("chunks"))
    r.check("audio: the page shows the hint", bool(a0.get("hint")) and
            r.js("!document.getElementById('ocw-sound-hint').hidden") is True, a0.get("hint"))
    r.shot("locked")
    t_click = time.time()
    r.click()
    run = r.wait("running", lambda: (r.audio() or {}).get("state") == "running", 10)
    r.check("audio: running after the click", run, f"{time.time() - t_click:.2f} s")
    r.pump(4.0)
    a1 = r.audio() or {}
    r.pump(2.0)
    a2 = r.audio() or {}
    r.summary["audio_running"] = a2
    r.check("audio: the gesture was seen and resumed", (a2.get("gestures") or 0) >= 1 and (a2.get("resumes") or 0) >= 1,
            (a2.get("gestures"), a2.get("resumes")))
    dt_sched = (a2.get("scheduled_s") or 0) - (a1.get("scheduled_s") or 0)
    r.check("audio: chunks scheduled at the context's pace", 1.6 <= dt_sched <= 2.6,
            f"{dt_sched:.2f} s of audio in ~2 s; {a2.get('chunks')} chunks, {a2.get('sample_rate')} Hz")
    r.check("audio: the chunks carry sound (bike, bots)", (a2.get("audible_chunks") or 0) > 0.8 * (a2.get("chunks") or 1)
            and (a2.get("level") or 0) > 1e-3, (a2.get("audible_chunks"), a2.get("level")))
    r.check("audio: few gaps", (a2.get("underruns") or 0) <= 3, a2.get("underruns"))
    r.console("audio.play finish")
    r.console("audio.status")
    r.pump(1.0)
    status = [l for l in r.lines if "audio: playing on" in l]
    r.check("audio: audio.status says it plays", bool(status), status[-1] if status else None)
    # A setting through the panel's way, kept over a reload.
    r.console("set audio.master 55")
    r.pump(1.0)
    r.check("audio: set audio.master applies", (r.audio() or {}).get("mix", {}).get("master") == 55)
    r.shot("sound")
    r.cdp.call("Page.reload", {})
    r.seen = len(r.cdp.events)
    mark = len(r.lines)
    r.wait("ride after the reload", lambda: any("local server running" in l for l in r.lines[mark:]), 60)
    a3 = r.wait("audio after reload", r.audio, 10) or {}
    r.check("audio: the volume survived the reload (localStorage)", a3.get("mix", {}).get("master") == 55,
            a3.get("mix"))
    # Chromium keeps the sticky activation over a same-origin reload only
    # for the frame that got it; the app follows whatever the context says.
    r.summary["audio_after_reload"] = a3
    r.console("set audio.master 80")
    r.pump(0.5)


# ---------------------------------------------------------------- voice


def start_server(a, work, livekit_url, key, secret, echo=True, dev=False):
    quic, wt = mp.free_udp_port(), mp.free_udp_port()
    info_file = os.path.join(work, "web.json")
    env_cmd = [os.path.join(a.bin, "ocw-server"), "--listen", f"127.0.0.1:{quic}",
               "--webtransport", f"127.0.0.1:{wt}", "--web-info-file", info_file,
               "--test-route", "hills", "--bots", "0", "--time-control", "anyone",
               "--livekit-url", livekit_url, "--livekit-api-key", key]
    if echo:
        env_cmd.append("--echo-bot")
    if dev:
        # Tests only: the pages' teleports place riders near and far.
        env_cmd.append("--dev-commands")
    os.environ["OCW_LIVEKIT_API_SECRET"] = secret
    server = mp.Proc("server", env_cmd)
    if not server.wait_for(r"listening \(WebTransport", 20):
        raise RuntimeError("the server did not start:\n" + server.text())
    info = json.load(open(info_file))
    return server, quic, info


def part_voice(r, port, a, procs):
    work = tempfile.mkdtemp(prefix="ocw-web-voice-")
    server, quic, info = start_server(a, work, "wss://livekit.fake.invalid", "devkey", secrets.token_hex(16))
    procs.append(server)
    bot = mp.Proc("bot", [os.path.join(a.bin, "ocw-bot"), "--server", f"127.0.0.1:{quic}",
                          "--insecure-dev", "--riders", "1", "--seconds", "0", "--route", "hills",
                          "--name-prefix", "quic", "--follow", "Voice", "--voice-nearby"])
    procs.append(bot)
    with open(os.path.join(HERE, "web-voice-fake.js")) as f:
        r.cdp.call("Page.addScriptToEvaluateOnNewDocument", {"source": f.read()})
    url = f"http://127.0.0.1:{port}/?{info['query']}&name=Voice&route=hills"
    r.cdp.call("Page.navigate", {"url": url})
    if not wait_ride(r, r"joined the remote server", 60):
        r.check("voice: joined the server", False)
        return
    r.console("time noon")
    lk = lambda: r.json("window.__lk && window.__lk.state()") or {}  # noqa: E731
    con = r.wait("voice connected", lambda: (r.voice() or {}).get("status") == "connected", 30)
    v = r.voice() or {}
    st = lk()
    r.summary["voice_connected"] = {"voice": v, "fake": {k: st.get(k) for k in ("room", "identity", "webAudioMix")}}
    r.check("voice: the shared controller joined the call (token over WebTransport)", con, v.get("status"))
    r.check("voice: the shim connected with the game server's token", bool(st.get("room")) and
            str(st.get("identity", "")).startswith("rider-"), (st.get("room"), st.get("identity")))
    r.check("voice: voices through Web Audio (webAudioMix)", st.get("webAudioMix") is True)
    # Every shared ride is joined muted (spec §7.1.7 Q7): published, muted.
    r.check("voice: the microphone is published, joined muted",
            st.get("mic") is False and v.get("mic_muted") is True, (st.get("mic"), v.get("mic_muted")))
    # The bot's participant joins the room.
    riders = v.get("riders") or []
    bot_r = next((x for x in riders if not x["local"] and x["name"].startswith("quic")), None)
    r.check("voice: the bot rider is on the road", bot_r is not None, [x["name"] for x in riders])
    if bot_r is None:
        return
    ident = bot_r["identity"]
    r.js(f"window.__lk.addParticipant({json.dumps(ident)}, {json.dumps(bot_r['name'])})")
    sub = r.wait("subscribed", lambda: any(p["identity"] == ident and p["subscribed"] and p["panned"]
                                           for p in lk().get("participants", [])), 10)
    v = r.voice() or {}
    r.check("voice: the participant is listed", any(p["name"] == bot_r["name"] for p in v.get("participants", [])),
            v.get("participants"))
    r.check("voice: its voice is subscribed and placed (gain, pan)", sub, lk().get("participants"))
    r.check("voice: the HUD's voice indicator is shown", v.get("in_call") is True and v.get("hud", "").startswith("connected"),
            v.get("hud"))
    r.js(f"window.__lk.speak([{json.dumps(ident)}])")
    spk = r.wait("speaking", lambda: bot_r["name"] in ((r.voice() or {}).get("speaking") or []), 5)
    r.check("voice: active speakers reach the HUD", spk)
    r.shot("voice")
    # M: the microphone (joined muted: M unmutes, M again mutes).
    r.key("KeyM", "m")
    on = r.wait("mic on", lambda: (r.voice() or {}).get("mic_muted") is False, 5)
    r.pump(0.5)
    r.check("voice: M unmutes the microphone", on and lk().get("mic") is True, lk().get("mic"))
    r.key("KeyM", "m")
    muted = r.wait("mic muted", lambda: (r.voice() or {}).get("mic_muted") is True, 5)
    r.pump(0.5)
    r.check("voice: M mutes the microphone", muted)
    r.check("voice: the shim muted the published track", lk().get("mic") is False)
    r.key("KeyM", "M", shift=True)
    smuted = r.wait("speaker muted", lambda: (r.voice() or {}).get("speaker_muted") is True, 5)
    r.pump(0.5)
    vols = [p["volume"] for p in lk().get("participants", [])]
    r.check("voice: Shift+M mutes the speaker (all voices at 0)", smuted and vols and all(x == 0 for x in vols), vols)
    r.shot("voice-muted")
    # A network drop: LiveKit reconnects by itself.
    r.js("window.__lk.drop(2000)")
    rec = r.wait("reconnecting", lambda: (r.voice() or {}).get("status") == "reconnecting", 5)
    back = r.wait("reconnected", lambda: (r.voice() or {}).get("status") == "connected", 10)
    r.check("voice: a drop shows reconnecting, then connected", rec and back)
    # A lost connection: failed, retried with a new token.
    rooms0 = lk().get("rooms", 0)
    r.js("window.__lk.kill()")
    failed = r.wait("failed", lambda: "failed" in (r.voice() or {}).get("status", ""), 5)
    again = r.wait("rejoined", lambda: lk().get("rooms", 0) > rooms0 and (r.voice() or {}).get("status") == "connected", 20)
    r.check("voice: a lost call fails and rejoins with a new token", failed and again,
            (rooms0, lk().get("rooms")))
    # The echo test over the shim's loopback (Chromium's fake microphone):
    # stop (↓) so the Echo rider stays near.
    r.key("ArrowDown", "ArrowDown")
    r.console("voice.echo here")
    near = r.wait("echo near", lambda: (r.voice() or {}).get("echo", {}).get("near") is True, 20)
    r.check("voice: the Echo rider came near (voice.echo here)", near, (r.voice() or {}).get("echo"))
    # The mic is muted from above: unmute it for the echo.
    r.console("voice.mute-mic off")
    r.console("voice.mute-speaker off")
    lv = r.wait("echo levels", lambda: ((r.voice() or {}).get("echo", {}).get("input") or 0) > 0.001 and
                ((r.voice() or {}).get("echo", {}).get("level") or 0) > 0.0005, 15)
    r.pump(1.5)
    e = (r.voice() or {}).get("echo", {})
    shim = r.json("window.ocwVoice.state()") or {}
    r.check("voice: the echo loop hears the microphone and plays it back", lv,
            {**e, "shim": {k: shim.get(k) for k in ("echo", "echo_running", "context")}})
    r.summary["echo"] = e
    r.shot("echo")
    r.console("voice.echo off")
    r.console("voice.mute-mic on")
    r.pump(1.0)
    # A reload is a new ride: joined muted again, whatever was set before.
    r.console("voice.mute-mic off")
    r.pump(0.5)
    r.cdp.call("Page.reload", {})
    mark = len(r.lines)
    r.wait("rejoined after reload", lambda: any("joined the remote server" in l for l in r.lines[mark:]), 60)
    v = r.wait("voice after reload", r.voice, 10) or {}
    r.check("voice: after a reload the ride is joined muted again", v.get("mic_muted") is True, v.get("mic_muted"))
    r.console("voice.mute-mic off")
    r.pump(0.5)


# ---------------------------------------------------------------- livekit


def part_livekit(r, port, a, procs):
    local_bin = os.path.expanduser("~/.local/bin/livekit-server")
    lks = (shutil.which(a.livekit_server)
           or (a.livekit_server if os.path.exists(a.livekit_server) else None)
           or (local_bin if a.livekit_server == "livekit-server" and os.path.exists(local_bin) else None))
    if not lks:
        r.check("livekit: a livekit-server binary (skipped)", True, "not installed")
        r.summary["livekit"] = "skipped: no livekit-server"
        return
    lk_port = free_tcp_port()
    lk = mp.Proc("livekit", [lks, "--dev", "--bind", "127.0.0.1", "--port", str(lk_port)])
    procs.append(lk)
    if not lk.wait_for(r"starting LiveKit server|starting in development mode|rtc", 20):
        r.check("livekit: the server started", False, lk.text()[-2000:])
        return
    work = tempfile.mkdtemp(prefix="ocw-web-lk-")
    server, quic, info = start_server(a, work, f"ws://127.0.0.1:{lk_port}", "devkey", "secret",
                                      echo=False, dev=True)
    procs.append(server)
    names = ["Ada", "Bea", "Cleo"]
    for i, name in enumerate(names):
        if i:
            r.new_page(name)
        else:
            r.pages[name] = r.pages["main"]
            r.use(name)
        url = f"http://127.0.0.1:{port}/?{info['query']}&name={name}&route=hills"
        r.cdp.call("Page.navigate", {"url": url})
        r.pump(1.0)
    # Both pages must run their frame loops at once (a hidden page's stop:
    # the first run of this test waited 140 s for a join that only came
    # with the screenshot).
    visible = {}
    for name in names:
        r.use(name)
        visible[name] = r.js("document.visibilityState")
    r.check("livekit: all pages are visible (their game loops run)",
            all(v == "visible" for v in visible.values()), visible)
    joined = {}
    for name in names:
        r.use(name)
        joined[name] = r.wait(f"{name} joined the game server",
                              lambda: (r.json("window.ocw && window.ocw.netStats()") or {}).get("rider_id"), 30)
    r.check("livekit: all riders joined the game session", all(joined.values()), joined)
    ok = {}
    for name in names:
        r.use(name)
        r.click()
        ok[name] = r.wait(f"{name} in the call", lambda: (r.voice() or {}).get("status") == "connected", 40)
    r.check("livekit: all pages joined the real LiveKit room", all(ok.values()), ok)
    # Every shared ride starts with My voice off – muted and heard by no
    # one (spec §7.1.7 Q7, Q16): each page checks, then turns it on for
    # everybody nearby.
    muted = {}
    for name in names:
        r.use(name)
        v = r.voice() or {}
        muted[name] = (v.get("mic_muted"), v.get("heard_by"), (v.get("hearing") or {}).get("heard_by"))
    r.check("livekit: every page joins with My voice off, heard by no one",
            all(m[0] is True and m[1] == "no_one" and not m[2] for m in muted.values()), muted)
    for name in names:
        r.use(name)
        r.console("set voice.my_voice everybody")
        r.console("voice.mute-mic off")
    # Voices are heard only within 10 m (spec §7.1.7 Q20): the pages stop
    # pedalling and stand together at 2.0 km (8 % up: at 0 W nobody rolls).
    for name in names:
        r.use(name)
        r.console("sim.power 0")
        r.console("teleport 2.0")
    r.pump(2.0)
    seen = {}
    for name in names:
        others = [o for o in names if o != name]
        r.use(name)
        seen[name] = r.wait(f"{name} sees {others}", lambda: all(any(
            p["name"] == o for p in (r.voice() or {}).get("participants", [])) for o in others), 20)
    r.check("livekit: each sees the other participants", all(seen.values()), seen)
    r.use("Ada")
    talk = r.wait("Ada hears Bea talking", lambda: "Bea" in ((r.voice() or {}).get("speaking") or []), 20)
    r.check("livekit: the other's (fake) microphone is heard (active speaker)", talk)
    st = r.json("window.ocwVoice.state()") or {}
    r.check("livekit: the remote track plays through the panner", "rider-" in json.dumps(st.get("tracks")), st.get("tracks"))
    r.shot("livekit-ada")
    r.use("Bea")
    r.console("voice.mute-mic on")
    r.use("Ada")
    m = r.wait("Ada sees Bea muted", lambda: any(p["name"] == "Bea" and p["mic_muted"]
                                                 for p in (r.voice() or {}).get("participants", [])), 15)
    r.check("livekit: a mute shows in the other page", m)
    r.use("Bea")
    r.shot("livekit-bea")
    r.console("voice.mute-mic off")
    r.pump(0.5)
    livekit_block(r, names, joined)
    livekit_range(r, names, joined)


def livekit_block(r, names, joined):
    """Voice block (spec §7.1.7 Q9) in the browser: Ada blocks Bea; the
    game server tells both, neither plays the other's track (Bea's own
    request for Ada's is refused by the voice server), Cleo plays both."""
    ident = {n: f"rider-{joined[n]}" for n in names}

    def tracks(name):
        r.use(name)
        return set((r.json("window.ocwVoice.state()") or {}).get("tracks") or [])

    # Before: everybody plays everybody's track.
    before = {}
    for name in names:
        want = {ident[o] for o in names if o != name}
        before[name] = bool(r.wait(f"{name} plays the others' tracks", lambda: want <= tracks(name), 30))
    r.check("livekit: before a block every page plays both other riders", all(before.values()), before)
    r.use("Ada")
    r.console("voice.block Bea on")
    r.use("Bea")
    told = r.wait("Bea is told she is apart from Ada",
                  lambda: joined["Ada"] in ((r.voice() or {}).get("apart") or []), 15)
    r.check("livekit: the game server tells the blocked rider (apart, not who blocked)", told,
            (r.voice() or {}).get("apart"))
    bea_list = {p["name"]: p for p in (r.voice() or {}).get("participants", [])}
    r.check("livekit: the blocked rider's list does not say she was blocked",
            not bea_list.get("Ada", {}).get("blocked") and not (r.voice() or {}).get("blocked"),
            bea_list.get("Ada"))
    gone_a = r.wait("Ada stops playing Bea", lambda: ident["Bea"] not in tracks("Ada"), 15)
    gone_b = r.wait("Bea stops playing Ada", lambda: ident["Ada"] not in tracks("Bea"), 15)
    r.check("livekit: after Ada blocks Bea neither plays the other's track", gone_a and gone_b,
            {"Ada": sorted(tracks("Ada")), "Bea": sorted(tracks("Bea"))})
    r.use("Ada")
    st = r.json("window.ocwVoice.state()") or {}
    r.check("livekit: Ada's microphone is allowed to everybody but Bea",
            st.get("subscribers") is not None and ident["Bea"] not in st["subscribers"]
            and ident["Cleo"] in st["subscribers"], st.get("subscribers"))
    r.use("Cleo")
    r.pump(2.0)
    r.check("livekit: Cleo still plays both", {ident["Ada"], ident["Bea"]} <= tracks("Cleo"),
            sorted(tracks("Cleo")))
    # Bea as a modified client: asks for Ada's track itself.
    r.use("Bea")
    r.js("window.ocwVoice.command(%s)" % json.dumps(json.dumps(
        {"op": "subscribe", "identity": ident["Ada"], "on": True})))
    r.pump(5.0)
    r.check("livekit: Bea asking for Ada's track herself is refused by the voice server",
            ident["Ada"] not in tracks("Bea"), sorted(tracks("Bea")))
    r.use("Ada")
    r.shot("livekit-blocked")
    r.console("voice.block Bea off")
    back_a = r.wait("Ada plays Bea again", lambda: ident["Bea"] in tracks("Ada"), 20)
    back_b = r.wait("Bea plays Ada again", lambda: ident["Ada"] in tracks("Bea"), 20)
    r.check("livekit: unblocked, both play each other again", back_a and back_b,
            {"Ada": sorted(tracks("Ada")), "Bea": sorted(tracks("Bea"))})


def livekit_range(r, names, joined):
    """Voice by range and audience (spec §7.1.7 Q6, Q8, Q10, §7.1.3) in the
    browser: the game server decides who receives whose voice from the
    riders' places. A far rider is neither played nor plays anyone (her own
    request is refused by the voice server), a near one is; *Who hears me:
    Selected riders* and *Voice room: Selected riders* hold."""
    ident = {n: f"rider-{joined[n]}" for n in names}

    def tracks(name):
        r.use(name)
        return set((r.json("window.ocwVoice.state()") or {}).get("tracks") or [])

    def allowed(name):
        r.use(name)
        s = (r.json("window.ocwVoice.state()") or {}).get("subscribers")
        return None if s is None else set(s)

    def hearing(name):
        r.use(name)
        return (r.voice() or {}).get("hearing") or {}

    # All three together at 2 km, then Cleo 20 m further on (out of the
    # 10 m range, spec §7.1.7 Q20).
    for name in names:
        r.use(name)
        r.console("sim.power 0")
        r.console("teleport 2.0")
    r.pump(2.0)
    together = {}
    for name in names:
        want = {ident[o] for o in names if o != name}
        together[name] = bool(r.wait(f"{name} plays both others", lambda: want <= tracks(name), 30))
    r.check("livekit range: together every page plays both others", all(together.values()), together)
    r.use("Cleo")
    r.console("teleport 2.02")
    gone = r.wait("Ada stops playing the far Cleo", lambda: ident["Cleo"] not in tracks("Ada"), 20)
    alone = r.wait("Cleo plays nobody", lambda: not tracks("Cleo"), 20)
    r.check("livekit range: a rider 20 m away is not played, and plays nobody", gone and alone,
            {n: sorted(tracks(n)) for n in names})
    h = hearing("Cleo")
    r.check("livekit range: the game server sends the far rider nobody's voice and hers to nobody",
            not h.get("may_hear") and not h.get("heard_by"), h)
    r.check("livekit range: Ada's microphone is not allowed to the far rider",
            ident["Cleo"] not in (allowed("Ada") or set()) and ident["Bea"] in (allowed("Ada") or set()),
            sorted(allowed("Ada") or []))
    r.check("livekit range: Bea still plays Ada", ident["Ada"] in tracks("Bea"), sorted(tracks("Bea")))
    # Cleo as a modified client asks for Ada's track herself.
    r.use("Cleo")
    r.js("window.ocwVoice.command(%s)" % json.dumps(json.dumps(
        {"op": "subscribe", "identity": ident["Ada"], "on": True})))
    r.pump(5.0)
    r.check("livekit range: the far rider asking for Ada's track herself is refused by the voice server",
            ident["Ada"] not in tracks("Cleo"), sorted(tracks("Cleo")))
    r.shot("livekit-range-far")
    # Selected riders are always audible (Q20): Cleo 1.1 km away listens
    # to Voice room = Selected riders with Ada.
    r.use("Cleo")
    r.console("teleport 0.9")
    r.console("set voice.i_hear selected")
    r.console("voice.select Ada on")
    far_ada = r.wait("Cleo plays the selected Ada 1.1 km away", lambda: tracks("Cleo") == {ident["Ada"]}, 30)
    h = hearing("Cleo")
    r.check("livekit range: a selected rider 1.1 km away is sent and played (Voice room = Selected riders)",
            far_ada and h.get("may_hear") == [joined["Ada"]], {"tracks": sorted(tracks("Cleo")), "hearing": h})
    r.check("livekit range: the others do not play Cleo 1.1 km away",
            ident["Cleo"] not in tracks("Ada") and ident["Cleo"] not in tracks("Bea"),
            {n: sorted(tracks(n)) for n in names})
    r.use("Cleo")
    r.shot("livekit-range-selected-far")
    r.console("voice.select Ada off")
    r.console("set voice.i_hear everybody")
    # Back together.
    r.use("Cleo")
    r.console("teleport 2.0")
    back = r.wait("Ada plays Cleo again", lambda: ident["Cleo"] in tracks("Ada"), 30)
    back_c = r.wait("Cleo plays Ada again", lambda: ident["Ada"] in tracks("Cleo"), 30)
    r.check("livekit range: back near, both play each other again", back and back_c,
            {n: sorted(tracks(n)) for n in names})
    # Ada: My voice = on · Selected, with Bea.
    r.use("Ada")
    r.console("voice.select Bea on")
    r.console("set voice.my_voice selected")
    only_bea = r.wait("Ada allows Bea only", lambda: allowed("Ada") == {ident["Bea"]}, 15)
    r.check("livekit audience: My voice = Selected allows only Bea", only_bea,
            sorted(allowed("Ada") or []))
    stop = r.wait("Cleo stops playing Ada", lambda: ident["Ada"] not in tracks("Cleo"), 20)
    r.check("livekit audience: Cleo (not selected) no longer plays Ada, Bea does",
            stop and ident["Ada"] in tracks("Bea"), {n: sorted(tracks(n)) for n in names})
    r.use("Ada")
    v = r.voice()
    r.check("livekit audience: the selection is shown and remembered",
            any(s.get("name") == "Bea" for s in (v or {}).get("selected") or []), (v or {}).get("selected"))
    # Cleo: I hear = selected, nobody selected yet, then Bea.
    r.use("Cleo")
    r.console("set voice.i_hear selected")
    quiet = r.wait("Cleo plays nobody", lambda: not tracks("Cleo"), 20)
    r.use("Cleo")
    r.console("voice.select Bea on")
    bea = r.wait("Cleo plays Bea only", lambda: tracks("Cleo") == {ident["Bea"]}, 20)
    r.check("livekit audience: I hear = selected plays only the selection", quiet and bea,
            sorted(tracks("Cleo")))
    # Cleo: I hear = nobody (spec §7.1.7 Q16): she plays nobody, the
    # server sends her nobody's voice, and Bea still plays her.
    r.use("Cleo")
    r.console("set voice.i_hear nobody")
    deaf = r.wait("Cleo plays nobody (I hear nobody)", lambda: not tracks("Cleo"), 20)
    h = hearing("Cleo")
    r.check("livekit audience: I hear = nobody plays nobody and is sent nobody's voice",
            deaf and not h.get("may_hear"), {"tracks": sorted(tracks("Cleo")), "hearing": h})
    r.use("Cleo")
    r.shot("livekit-i-hear-nobody")
    r.console("set voice.i_hear everybody")
    r.use("Cleo")
    r.shot("livekit-selected")


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web")
    p.add_argument("--bin", default="target/release", help="where ocw-server and ocw-bot are")
    p.add_argument("--part", default="all", choices=["audio", "voice", "livekit", "all"])
    p.add_argument("--livekit-server", default="livekit-server")
    p.add_argument("--chromium", default=None,
                   help="the browser (default: OCW_BROWSER, else the pinned Chromium of `make deps-browser`,"
                        " else the system's chromium; scripts/web_browser.py)")
    p.add_argument("--size", default="1280x720")
    p.add_argument("--software", action="store_true")
    a = p.parse_args()
    a.chromium = a.chromium or web_browser.find()
    missing = tw.web_serve().missing_build(a.dir, ["ocw_web_bg.wasm"])
    if missing:
        sys.exit(missing)
    parts = ["audio", "voice", "livekit"] if a.part == "all" else [a.part]
    if any(x in parts for x in ("voice", "livekit")):
        for b in ("ocw-server", "ocw-bot"):
            if not os.path.exists(os.path.join(a.bin, b)):
                sys.exit(f"{a.bin}/{b} missing: cargo build --release -p ocw-server -p ocw-bot")
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    procs, summary, lines, checks, errors = [], {}, [], {}, []
    httpd = serve(a)
    port = httpd.server_address[1]
    for part in parts:
        base = os.path.join(a.out, f"web-audio-{stamp}-{part}")
        r = Run(a, base)
        try:
            {"audio": lambda: part_audio(r, port),
             "voice": lambda: part_voice(r, port, a, procs),
             "livekit": lambda: part_livekit(r, port, a, procs)}[part]()
        except Exception as e:  # noqa: BLE001 – reported
            r.check(f"{part}: no test error", False, repr(e))
            try:
                r.shot("failed")
            except Exception:  # noqa: BLE001
                pass
        finally:
            r.close()
            for pr in procs:
                pr.stop(signal.SIGTERM, timeout=10)
            for pr in procs:
                r.lines.append(f"\n--- {pr.label}: {' '.join(pr.cmd)} ---")
                r.lines.extend(l for _, l in pr.lines[-400:])
            procs.clear()
        bad = [x for x in r.errors if "livekit.fake.invalid" not in x]
        r.check(f"{part}: no browser errors", not bad, bad[:5])
        summary[part] = r.summary
        checks.update(r.checks)
        errors.extend(bad)
        lines.extend(r.lines)
    httpd.shutdown()
    ok = all(checks.values())
    result = {"ok": ok, "checks": checks, "parts": summary, "errors": errors,
              "browser": web_browser.describe(a.chromium)}
    base = os.path.join(a.out, f"web-audio-{stamp}")
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n\n--- summary ---\n" + json.dumps(result, indent=2) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(result, f, indent=2)
    print(json.dumps({"ok": ok, "checks": checks}, indent=2))
    shots = [s for part in summary.values() for s in part.get("screenshots", [])]
    print(f"log: {base}.log\nscreenshots: {' '.join(shots)}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
