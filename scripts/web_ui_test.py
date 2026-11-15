"""The UI part of the headless browser test (spec §14 Phase 10, B6: UI
parity), used by scripts/test-web.py after the ride:

- drives the in-game menu with the keyboard (CDP key events → the canvas):
  the top page (no Quit, Captures), Settings → Display (a setting changed
  with →), World → Weather (the weather machine), Navi → Riders in this
  ride (the rider list);
- the HUD with weather and wind and the damage widget;
- a fake gamepad (`navigator.getGamepads`, the standard mapping) opens and
  drives the menu: Bevy's gilrs reads the browser's Gamepad API;
- Captures → Screenshot and → Download this ride (FIT): the downloads
  (saved through CDP's download behaviour) are a JPEG with EXIF and a
  FIT file; a short video (MediaRecorder) is a WebM download;
- unicorn mode (spec §14 Phase 13 G5): G flies, + climbs (by the
  `KeyboardEvent.key`), I hides / shows the inspection, P puts a pin, Esc
  opens the menu in flight (World → Unicorn mode), G lands;
- the changed setting survives a reload (localStorage).

Screenshots: BASE-menu.jpg, -settings.jpg, -weather.jpg, -hud.jpg,
-riders.jpg, -gamepad.jpg, -unicorn-flight.jpg, -unicorn-inspect.jpg,
-unicorn-menu.jpg. Each check is a (name, ok, detail) row in the
summary.
"""

import json
import os
import time

KEYS = {
    "Escape": ("Escape", 27, ""),
    "Enter": ("Enter", 13, "\r"),
    "ArrowUp": ("ArrowUp", 38, ""),
    "ArrowDown": ("ArrowDown", 40, ""),
    "ArrowLeft": ("ArrowLeft", 37, ""),
    "ArrowRight": ("ArrowRight", 39, ""),
    # Unicorn mode (spec §14 Phase 13 G5): G, I, P by position, + by what
    # it types (the US `=` key typing "+": `KeyboardEvent.key`).
    "g": ("KeyG", 71, "g"),
    "i": ("KeyI", 73, "i"),
    "p": ("KeyP", 80, "p"),
    "+": ("Equal", 187, "+"),
}

# The standard mapping's buttons.
PAD = {"south": 0, "east": 1, "start": 9, "dpad_up": 12, "dpad_down": 13}

FAKE_PAD = """
(() => {
  const buttons = Array.from({length: 17}, () => ({pressed: false, touched: false, value: 0}));
  const pad = {id: 'OCW test pad (STANDARD GAMEPAD)', index: 0, connected: true,
               mapping: 'standard', timestamp: performance.now(), buttons, axes: [0, 0, 0, 0],
               vibrationActuator: null, hapticActuators: []};
  window.ocwTestPad = {
    set(i, on) { buttons[i].pressed = on; buttons[i].value = on ? 1 : 0; pad.timestamp = performance.now(); },
  };
  navigator.getGamepads = () => [pad, null, null, null];
  return true;
})()
"""


class UiTest:
    def __init__(self, cdp, base, lines, out_dir):
        self.cdp, self.base, self.lines = cdp, base, lines
        self.checks = []
        self.shots = {}
        self.seen = len(cdp.events)
        self.t0 = time.time()
        self.downloads = os.path.abspath(base + "-downloads")
        os.makedirs(self.downloads, exist_ok=True)

    # ------------------------------------------------------------ helpers

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            self.cdp.pump(min(0.1, max(0.0, end - time.time())))
        self.collect()

    def collect(self):
        from_events = self.cdp.events[self.seen:]
        self.seen = len(self.cdp.events)
        for e in from_events:
            if e["method"] == "Runtime.consoleAPICalled":
                text = " ".join(str(a.get("value", a.get("description", "")))
                                for a in e["params"].get("args", []))
                self.lines.append(f"[ui] {time.time() - self.t0:7.2f} s  {text}")
            elif e["method"] == "Runtime.exceptionThrown":
                d = e["params"]["exceptionDetails"]
                text = d.get("exception", {}).get("description") or d.get("text", "")
                self.lines.append(f"[ui] {time.time() - self.t0:7.2f} s  EXCEPTION {text}")

    def check(self, name, ok, detail=None):
        self.checks.append({"check": name, "ok": bool(ok), "detail": detail})
        mark = "ok " if ok else "FAIL"
        self.lines.append(f"[ui] check {mark} {name}: {detail}")

    def key(self, name, times=1):
        code, vk, text = KEYS[name]
        for _ in range(times):
            down = {"type": "keyDown", "key": name, "code": code,
                    "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}
            if text:
                down["text"] = text
            self.cdp.call("Input.dispatchKeyEvent", down)
            self.pump(0.12)
            self.cdp.call("Input.dispatchKeyEvent", {
                "type": "keyUp", "key": name, "code": code,
                "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
            self.pump(0.12)

    def hold(self, name, seconds):
        """Holds a key down for `seconds` (the flight's + climbs while held)."""
        code, vk, text = KEYS[name]
        down = {"type": "keyDown", "key": name, "code": code,
                "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}
        if text:
            down["text"] = text
        self.cdp.call("Input.dispatchKeyEvent", down)
        self.pump(seconds)
        self.cdp.call("Input.dispatchKeyEvent", {
            "type": "keyUp", "key": name, "code": code,
            "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
        self.pump(0.2)

    def god(self):
        raw = self.cdp.eval("window.ocw && window.ocw.god ? window.ocw.god() : ''")
        try:
            return json.loads(raw) if raw else {}
        except ValueError:
            return {}

    def wait_god(self, pred, timeout=10.0):
        end = time.time() + timeout
        g = self.god()
        while time.time() < end and not pred(g):
            self.pump(0.1)
            g = self.god()
        return g

    def menu(self):
        raw = self.cdp.eval("window.ocw && window.ocw.menu ? window.ocw.menu() : ''")
        try:
            return json.loads(raw) if raw else {}
        except ValueError:
            return {}

    def wait_menu(self, pred, timeout=3.0):
        end = time.time() + timeout
        m = self.menu()
        while time.time() < end and not pred(m):
            self.pump(0.1)
            m = self.menu()
        return m

    def focus_item(self, prefix):
        """Moves the focus down to the first item starting with `prefix`."""
        m = self.menu()
        items = m.get("items", [])
        at = next((i for i, it in enumerate(items) if it.startswith(prefix)), None)
        if at is None:
            return None
        self.key("ArrowDown", (at - m.get("focused", 0)) % len(items))
        return at

    def open_item(self, prefix, title):
        if self.focus_item(prefix) is None:
            self.check(f"menu: an item {prefix!r}", False, self.menu().get("items"))
            return False
        self.key("Enter")
        m = self.wait_menu(lambda m: m.get("title") == title)
        return m.get("title") == title

    def shot(self, label):
        self.pump(0.4)
        path = f"{self.base}-{label}.jpg"
        self.cdp.screenshot(path)
        self.shots[label] = path
        return path

    def console(self, line):
        self.cdp.eval(f"window.ocw.console({json.dumps(line)})")
        self.pump(0.3)

    def close_menu(self):
        for _ in range(6):
            if not self.menu().get("open"):
                return
            self.key("Escape")

    def devices_panel_state(self):
        """The Devices overlay (B4) on the page: shown or not, its box, its
        chooser buttons."""
        raw = self.cdp.eval("""JSON.stringify((() => {
            const p = document.getElementById('ocw-devices');
            const s = document.getElementById('ocw-devices-show');
            const shown = (el) => el && !el.hidden && getComputedStyle(el).visibility !== 'hidden'
                && getComputedStyle(el).display !== 'none';
            const r = p.getBoundingClientRect();
            return {visible: shown(p), show_button: shown(s),
                    box: [r.left, r.top, r.right, r.bottom],
                    width: innerWidth, height: innerHeight,
                    buttons: p.querySelectorAll('button[data-kind]').length};
        })())""")
        return json.loads(raw) if raw else {}

    def devices_panel_clear(self):
        """Hidden, or not overlapping the menu's panel (the centred box of
        the menu: 19 %–81 % of the width at 16:9)."""
        st = self.devices_panel_state()
        if not st:
            return False
        if not st["visible"] and not st["show_button"]:
            return True
        left = st["box"][0]
        return left >= 0.81 * st["width"]

    def setting(self, key):
        return self.cdp.eval(f"window.ocw.setting({json.dumps(key)})")

    def page_downloads(self):
        return self.cdp.eval("JSON.stringify(window.ocwDownloads || [])")

    def wait_download(self, suffix, timeout=10.0):
        end = time.time() + timeout
        while time.time() < end:
            listed = json.loads(self.page_downloads() or "[]")
            hit = [d for d in listed if d["name"].endswith(suffix)]
            if hit:
                name = hit[-1]["name"]
                path = os.path.join(self.downloads, name)
                # The browser writes the file shortly after.
                for _ in range(50):
                    if os.path.exists(path) and os.path.getsize(path) == hit[-1]["size"]:
                        return hit[-1], path
                    self.pump(0.1)
                return hit[-1], None
            self.pump(0.2)
        return None, None

    # ------------------------------------------------------------ unicorn mode

    def god_part(self, height=None):
        """Unicorn mode (spec §14 Phase 13 G5): G flies, + climbs, I hides and
        shows the inspection, P puts a pin, Esc (after the pointer) opens the
        menu with World → Unicorn mode, G lands. `height`: first put the
        unicorn that high above the ground (a dev command; the world test's
        view over the town). Screenshots -unicorn-flight, -unicorn-inspect,
        -unicorn-menu."""
        self.cdp.eval("document.getElementById('ocw-canvas').focus()")
        g = self.god()
        self.check("unicorn: riding before G", g and not g.get("active"), g)
        self.key("g")
        g = self.wait_god(lambda g: g.get("flying"))
        self.check("unicorn: G flies (World → Unicorn mode's key)", g.get("flying"), g)
        if not g.get("flying"):
            return
        if height:
            self.console(f"god.pose here here {height}")
            self.pump(1.0)
        y0 = (self.god().get("pos") or [0, 0, 0])[1]
        self.hold("+", 2.5)
        g = self.god()
        y1 = (g.get("pos") or [0, 0, 0])[1]
        self.check("unicorn: + climbs (by KeyboardEvent.key)", y1 > y0 + 5.0,
                   f"{y0:.1f} → {y1:.1f} m, {g.get('height_m')} m above ground")
        self.check("unicorn: the pointer (Pointer Lock or freed when the browser refused it)",
                   "captured" in g, g.get("captured"))
        # The inspection: shown by default; I hides it, I shows it again.
        self.wait_god(lambda g: g.get("panel"), timeout=3.0)
        self.key("i")
        hidden = self.wait_god(lambda g: not g.get("panel"), timeout=3.0)
        self.check("unicorn: I hides the inspection", not hidden.get("panel"), hidden.get("panel"))
        self.shot("unicorn-flight")
        self.key("i")
        g = self.wait_god(lambda g: g.get("panel") and g.get("crosshair"), timeout=3.0)
        panel = g.get("panel") or []
        self.check("unicorn: I opens the inspection (panel and crosshair)",
                   any(str(l).startswith("Position x") for l in panel) and g.get("crosshair"),
                   (panel[:3], g.get("crosshair")))
        self.key("p")
        g = self.wait_god(lambda g: g.get("pins", 0) >= 1, timeout=3.0)
        self.check("unicorn: P puts a pin", g.get("pins", 0) >= 1, g.get("pins"))
        self.shot("unicorn-inspect")
        # Esc: the first may only free the pointer (and the browser ends a
        # Pointer Lock itself); then the menu, World → Unicorn mode.
        for _ in range(3):
            self.key("Escape")
            if self.wait_menu(lambda m: m.get("open"), timeout=1.5).get("open"):
                break
        m = self.menu()
        self.check("unicorn: Esc opens the menu in flight", m.get("open"), m.get("title"))
        ok = self.open_item("World", "World")
        item = self.focus_item("Unicorn mode")
        m = self.menu()
        self.check("menu: World → Unicorn mode", ok and item is not None,
                   m.get("items"))
        self.shot("unicorn-menu")
        self.close_menu()
        self.key("g")
        g = self.wait_god(lambda g: not g.get("active"), timeout=15.0)
        self.check("unicorn: G lands on the road", not g.get("active") and g.get("pos") is None, g)

    # ------------------------------------------------------------ the part

    def run(self, reload_and_wait):
        cdp = self.cdp
        cdp.call("Browser.setDownloadBehavior",
                 {"behavior": "allow", "downloadPath": self.downloads, "eventsEnabled": True},
                 session=False)
        cdp.eval(FAKE_PAD)
        cdp.eval("document.getElementById('ocw-canvas').focus()")
        self.pump(0.6)
        self.close_menu()

        # The top page by keyboard.
        self.key("Escape")
        m = self.wait_menu(lambda m: m.get("open"))
        self.check("menu: Esc opens it", m.get("open") and m.get("title") == "Open Cycling World", m.get("title"))
        items = m.get("items", [])
        self.check("menu: the browser's top page (Captures, no Quit)",
                   "Captures" in items and "Quit" not in items and "Settings" in items, items)
        self.pump(0.7)  # the page's panels follow the menu twice a second
        self.check("page: the Devices panel steps aside while the menu is open",
                   self.devices_panel_clear(), self.devices_panel_state())
        self.shot("menu")

        # Settings → Display: a setting changed with →.
        ok = self.open_item("Settings", "Settings")
        items = self.menu().get("items", [])
        self.check("menu: Settings without the desktop's rider profiles", ok and "Riders" not in items, items)
        ok = self.open_item("Display", "Display")
        self.check("menu: Settings → Display", ok, self.menu().get("title"))
        before = self.setting("display.units")
        self.focus_item("Units")
        self.key("ArrowRight")
        self.pump(0.5)
        after = self.setting("display.units")
        self.check("settings: → changes display.units", before != after and after, f"{before} → {after}")
        fullscreen = [i for i in self.menu().get("items", []) if i.startswith("Fullscreen")]
        self.check("settings: no fullscreen row (the browser has its own)", not fullscreen, fullscreen)
        self.shot("settings")
        stored = cdp.eval("localStorage.getItem('ocw-settings') || ''") or ""
        self.check("settings: saved in localStorage", "units" in stored, stored[:200])
        self.key("Escape", 2)

        # World → Weather: the weather machine.
        m = self.wait_menu(lambda m: m.get("title") == "Open Cycling World")
        ok = self.open_item("World", "World")
        self.check("menu: World", ok, self.menu().get("items"))
        ok = self.open_item("Weather", "Weather machine")
        m = self.menu()
        self.check("menu: World → Weather (the weather machine)",
                   ok and str(m.get("status", "")).endswith("set by you"), m.get("status"))
        ok = self.open_item("Presets", "Weather presets")
        self.check("menu: the preset grid", ok and m is not None and self.menu().get("grid") == 5,
                   self.menu().get("grid"))
        # → ↓ ↓ Enter: hail in the transition time.
        self.key("ArrowRight")
        self.key("ArrowDown", 2)
        self.key("Enter")
        self.pump(0.5)
        self.key("Escape")
        self.shot("weather")
        self.close_menu()

        # The HUD: storm with wind, damage on and some damage.
        self.console("weather.transition instant")
        self.console("weather storm")
        self.console("ride.damage on")
        self.pump(0.5)
        self.console("dev.damage rider 55")
        self.console("dev.damage front_wheel 40")
        self.pump(2.5)
        self.shot("hud")
        hud_log = "\n".join(self.lines[-80:])
        self.check("hud: the damage rule is on", "error: ride.damage" not in hud_log, None)

        # Navi → Riders in this ride.
        self.key("Escape")
        self.wait_menu(lambda m: m.get("open"))
        ok = self.open_item("Navi", "Navi")
        ok = ok and self.open_item("Riders in this ride", "Riders in this ride")
        m = self.menu()
        self.check("menu: Navi → Riders in this ride", ok and "riders" in str(m.get("status", "")),
                   (m.get("status"), m.get("items")))
        self.shot("riders")
        self.close_menu()

        # The gamepad: Start opens, D-pad down moves, B goes back.
        def pad(button, on):
            cdp.eval(f"window.ocwTestPad.set({PAD[button]}, {'true' if on else 'false'})")

        def press(button):
            pad(button, True)
            self.pump(0.2)
            pad(button, False)
            self.pump(0.2)

        press("start")
        m = self.wait_menu(lambda m: m.get("open"))
        self.check("gamepad: Start opens the menu (the browser's Gamepad API)", m.get("open"), m.get("title"))
        f0 = m.get("focused")
        press("dpad_down")
        f1 = self.menu().get("focused")
        self.check("gamepad: the D-pad moves the focus", f0 is not None and f1 == f0 + 1, (f0, f1))
        press("south")
        m = self.wait_menu(lambda m: m.get("title") == "Ride")
        self.check("gamepad: A opens Ride", m.get("title") == "Ride", m.get("title"))
        self.shot("gamepad")
        press("east")
        m = self.wait_menu(lambda m: m.get("title") == "Open Cycling World")
        self.check("gamepad: B goes back", m.get("title") == "Open Cycling World", m.get("title"))
        self.close_menu()
        self.pump(0.7)
        st = self.devices_panel_state()
        self.check("page: the Devices panel is back (its buttons open the chooser) once the menu closes",
                   st.get("visible") and st.get("buttons", 0) == 3, st)

        # Captures → Screenshot: a JPEG download with EXIF.
        self.key("Escape")
        self.wait_menu(lambda m: m.get("open"))
        ok = self.open_item("Captures", "Captures")
        self.check("menu: Captures", ok, self.menu().get("items"))
        self.focus_item("Screenshot")
        self.key("Enter")
        d, path = self.wait_download(".jpg")
        data = open(path, "rb").read() if path else b""
        self.check("captures: the screenshot is a download named with the time",
                   d is not None and d["name"].startswith("ocw-20"), d)
        self.check("captures: a JPEG with its time in EXIF",
                   data[:2] == b"\xff\xd8" and b"Exif" in data[:64] and b"Open Cycling World" in data[:4096],
                   f"{len(data)} bytes at {path}")

        # Captures → Download this ride (FIT).
        self.key("Escape")
        self.wait_menu(lambda m: m.get("open"))
        self.open_item("Captures", "Captures")
        self.focus_item("Download this ride")
        self.key("Enter")
        d, path = self.wait_download(".fit")
        data = open(path, "rb").read() if path else b""
        self.check("fit: the ride is a download", d is not None and d["name"].startswith("ocw-ride-"), d)
        self.check("fit: a FIT file", data[8:12] == b".FIT" and len(data) > 100, f"{len(data)} bytes at {path}")
        self.close_menu()

        # A short video (MediaRecorder on the canvas, with the game sound).
        self.console("video start")
        self.pump(3.0)
        rec_s = cdp.eval("window.ocwVideo.state()")
        self.console("video stop")
        d, path = self.wait_download(".webm", timeout=15)
        size = os.path.getsize(path) if path else 0
        self.check("video: a WebM download named with the time",
                   d is not None and d["name"].startswith("ocw-20") and size > 1000,
                   (rec_s, d, size))

        # Unicorn mode (Phase 13 G5).
        self.god_part()

        # The setting survives a reload (localStorage).
        units = self.setting("display.units")
        reload_and_wait()
        self.seen = len(cdp.events)
        self.pump(1.0)
        again = self.setting("display.units")
        self.check("settings: display.units survived the reload", again == units and again, f"{units} → {again}")
        return all(c["ok"] for c in self.checks)


def run(cdp, base, lines, out_dir, reload_and_wait):
    t = UiTest(cdp, base, lines, out_dir)
    try:
        ok = t.run(reload_and_wait)
    except Exception as e:  # noqa: BLE001 – reported
        t.check("ui part ran", False, repr(e))
        ok = False
    return {"ok": ok, "checks": t.checks, "screenshots": t.shots, "downloads": t.downloads}


def run_god(cdp, base, lines, out_dir, height=None):
    """Only the unicorn-mode part (the world test: `scripts/test-web.py
    --world … --god`, flying over the world's town)."""
    t = UiTest(cdp, base, lines, out_dir)
    try:
        t.god_part(height=height)
    except Exception as e:  # noqa: BLE001 – reported
        t.check("unicorn part ran", False, repr(e))
    ok = all(c["ok"] for c in t.checks) and bool(t.checks)
    return {"ok": ok, "checks": t.checks, "screenshots": t.shots, "downloads": t.downloads}

