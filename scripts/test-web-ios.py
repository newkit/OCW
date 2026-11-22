#!/usr/bin/env python3
"""A long ride on an emulated iPhone in WebKit (spec 14 Phase 10 B2, B7; 11.4).

    scripts/test-web-ios.py [--out test-output/web-ios] [--dir target/web]
        [--world ID --route NAME | --route hills] [--minutes 5]
        [--landscape] [--background-every 60] [--lose-context-at 90]
        [--budget-mb 1100] [--query EXTRA] [--label NAME]

Why: the user's report of 2026-11-22 ("keeps restarting on my iOS"). iOS
Safari reloads a page whose WebContent process was killed – for its memory
(roughly 1 – 1.5 GB on an iPhone, less on older ones, the WebGL memory
counted too) or a crash – and our page itself reloads on a WebGPU failure
(ocw-gpu.js). This test rides the way a phone does and watches both:

  - Playwright's WebKit with the iPhone 15 device (393 × 659, pixel ratio
    3, touch, Safari's user agent; `--landscape` turned), Safari 26's
    `navigator.gpu` given (the page must take WebGL 2), no element full
    screen, as on the iPhone; the ride link the preview's start page makes
    on a phone (`gpu=webgl2&quality=low`) unless `--query` says otherwise;
  - every `--background-every` seconds the page is hidden for 5 s and shown
    again (visibilitychange with document.hidden, as when the app goes to
    the background);
  - after the ride (unless `--no-context-loss`) the WebGL context is lost
    (WEBGL_lose_context, as iOS takes the GPU from a page in the
    background) while the page is hidden: the app must stop at once (before
    the fix the wasm memory grew ~45 MB a second until the tab died) and
    load the ride again once shown; lost again right after, it must stay
    stopped with a message – one reload, never a loop;
  - every 5 s: the wasm memory, the live heap, the GPU meshes and
    textures (`window.ocw.heap()`) and the resident memory (RSS) of
    WebKit's web and GPU processes of this browser;
  - counted: navigations of the page (a reload loop), crashes of the web
    process, page errors.

Checks: rides and draws; no reload and no crash; no page error; the wasm
memory + GPU memory (what an iPhone counts against the tab) stay under
`--budget-mb`; the context loss as above.

  OUT/<label>-<time>-*.png   screenshots (start, every minute, end)
  OUT/<label>-<time>.json    the checks and the memory timeline
  OUT/<label>-<time>.log     the console

Needs `make web` and, for a world, the worlds of `make web-worlds`
(target/web/worlds, the root key in target/web-keys/root.hex). Exits 0
when every check passed, 2 without Playwright (`make deps-webkit`).
"""

import argparse
import datetime
import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
VENV = os.path.join(os.path.expanduser("~"), ".cache", "ocw", "playwright-venv")

FAKE_GPU = """
if (!navigator.gpu) {
  Object.defineProperty(Navigator.prototype, "gpu", {get: () => ({
    requestAdapter: async () => { window.__adapterAsked = (window.__adapterAsked || 0) + 1; return {features: new Set(), limits: {}}; },
    getPreferredCanvasFormat: () => "bgra8unorm",
  })});
}
for (const k of ["fullscreenEnabled", "webkitFullscreenEnabled"]) {
  Object.defineProperty(Document.prototype, k, {get: () => false, configurable: true});
}
// The page's visibility, switchable by the test (the app in the background).
window.__hidden = false;
Object.defineProperty(Document.prototype, "visibilityState", {get: () => window.__hidden ? "hidden" : "visible", configurable: true});
Object.defineProperty(Document.prototype, "hidden", {get: () => window.__hidden, configurable: true});
window.__setHidden = (h) => { window.__hidden = h; document.dispatchEvent(new Event("visibilitychange")); };
// Navigations of this tab (sessionStorage survives a reload).
try {
  const n = Number(sessionStorage.getItem("__loads") || 0) + 1;
  sessionStorage.setItem("__loads", String(n));
  window.__loads = n;
} catch (e) { window.__loads = -1; }
"""

STATE_JS = """() => JSON.stringify({gpu: window.ocwGpu || null, frames: window.ocwFrames || 0,
    failure: window.ocwGpuFailure || null, loads: window.__loads,
    status: document.getElementById('ocw-status') && !document.getElementById('ocw-status').hidden
        ? document.getElementById('ocw-status').textContent : '',
    heap: window.ocw && window.ocw.heap ? JSON.parse(window.ocw.heap()) : null,
    context: window.ocwGpuContext ? JSON.parse(window.ocwGpuContext()) : null,
    canvas: [document.getElementById('ocw-canvas').width, document.getElementById('ocw-canvas').height],
    viewport: [innerWidth, innerHeight, devicePixelRatio], href: location.href})"""

# What ocw-gpu.js's WebGPU watchdog would read from this canvas (32 x 18
# through drawImage, outside the frame): on WebKit's WebGL 2 without
# preserveDrawingBuffer it may well be one colour - harmless only as long
# as the watchdog never runs on WebGL 2.
SAMPLE_JS = """() => {
  const c = document.createElement('canvas'); c.width = 32; c.height = 18;
  const g = c.getContext('2d', {willReadFrequently: true});
  g.drawImage(document.getElementById('ocw-canvas'), 0, 0, 32, 18);
  const d = g.getImageData(0, 0, 32, 18).data;
  return JSON.stringify({uniform: window.ocwGpuChoice.uniform(d), rgb: [d[0], d[1], d[2]],
    watchdog: !!window.ocwGpuWatch});
}"""

LOSE_JS = """async () => {
  const c = document.getElementById('ocw-canvas');
  const gl = c.getContext('webgl2');
  const ext = gl && gl.getExtension('WEBGL_lose_context');
  if (!ext) return 'no WEBGL_lose_context';
  window.__lostAt = performance.now();
  ext.loseContext();
  await new Promise((r) => setTimeout(r, 3000));
  ext.restoreContext();
  return 'lost and restored';
}"""


def in_venv():
    try:
        import playwright  # noqa: F401
        return True
    except ImportError:
        return False


def web_serve():
    import importlib.util
    spec = importlib.util.spec_from_file_location("web_serve", os.path.join(HERE, "web-serve.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def children(pid):
    """All descendants of `pid` (Linux /proc)."""
    kids = {}
    for d in os.listdir("/proc"):
        if not d.isdigit():
            continue
        try:
            with open(f"/proc/{d}/stat") as f:
                parts = f.read().rsplit(")", 1)[1].split()
            kids.setdefault(int(parts[1]), []).append(int(d))
        except OSError:
            continue
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        for k in kids.get(p, []):
            out.append(k)
            todo.append(k)
    return out


def webkit_rss():
    """The RSS (MB) of this test's WebKit web and GPU processes."""
    rss = {"web": 0.0, "gpu": 0.0, "network": 0.0, "other": 0.0}
    for p in children(os.getpid()):
        try:
            name = open(f"/proc/{p}/comm").read().strip()
            with open(f"/proc/{p}/status") as f:
                kb = next((int(line.split()[1]) for line in f if line.startswith("VmRSS:")), 0)
        except (OSError, StopIteration, ValueError):
            continue
        key = ("web" if "WebProcess" in name or "WebKitWebProces" in name else
               "gpu" if "GPUProcess" in name or "GPUProces" in name else
               "network" if "Network" in name else "other")
        rss[key] += kb / 1024.0
    return rss


def image_drawn(path):
    try:
        from PIL import Image, ImageStat
    except ImportError:
        return os.path.getsize(path) > 60000, "no PIL"
    st = ImageStat.Stat(Image.open(path).convert("RGB"))
    return max(st.stddev) > 8.0, f"stddev {max(st.stddev):.1f}"


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--dir", default="target/web")
    p.add_argument("--out", default="test-output/web-ios")
    p.add_argument("--world", default=None)
    p.add_argument("--route", default="hills")
    p.add_argument("--free", action="store_true", help="a free ride (no route)")
    p.add_argument("--minutes", type=float, default=5.0)
    p.add_argument("--landscape", action="store_true")
    p.add_argument("--power", type=int, default=250)
    p.add_argument("--background-every", type=float, default=60.0, help="seconds; 0: never")
    p.add_argument("--no-context-loss", action="store_true", help="skip the lost WebGL context at the end")
    p.add_argument("--deaths", action="store_true",
                   help="at the end: the page dies in the ride twice (as iOS kills a tab for memory)")
    p.add_argument("--budget-mb", type=float, default=1100.0,
                   help="wasm + GPU memory the phone may use (MB; 0: no check)")
    p.add_argument("--query", default="gpu=webgl2&quality=low", help="the start page's phone options")
    p.add_argument("--label", default=None)
    p.add_argument("--site", default=None,
                   help="ride a deployed site instead of --dir (e.g. https://newkit.github.io/OCW/)")
    p.add_argument("--trust-root", default=None)
    p.add_argument("--python", default=None)
    a = p.parse_args()
    if not in_venv():
        py = a.python or os.path.join(VENV, "bin", "python")
        if not os.path.exists(py):
            print("test-web-ios: no Playwright here – run `make deps-webkit` first", file=sys.stderr)
            sys.exit(2)
        sys.exit(subprocess.call([py, os.path.abspath(__file__), *sys.argv[1:]]))
    from playwright.sync_api import sync_playwright

    ws = web_serve()
    if not a.site:
        missing = ws.missing_build(a.dir, ["ocw-gpu.js", "ocw_web_webgl2_bg.wasm"])
        if missing:
            sys.exit(missing)
    os.makedirs(a.out, exist_ok=True)
    label = a.label or ((os.path.basename(a.world) if a.world else a.route) + ("-landscape" if a.landscape else ""))
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = os.path.join(a.out, f"{label}-{stamp}")
    httpd = None
    if a.site:
        root = a.site.rstrip("/") + "/"
    else:
        httpd = ws.server(a.dir, stats={})
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        root = f"http://127.0.0.1:{httpd.server_address[1]}/"
    opts = [f"power={a.power}", "touch=on", "bots=2", "safe-insets=" + ("0,59,21,59" if a.landscape else "59,0,34,0")]
    if a.world:
        opts.insert(0, f"world={a.world}")
        if a.free:
            opts.append("free=1")
        elif a.route:
            opts.append(f"route={a.route}")
        trust = a.trust_root
        if trust is None and os.path.isfile("target/web-keys/root.hex"):
            trust = open("target/web-keys/root.hex").read().strip()
        if not a.site:
            opts.append(f"trust-root={trust}" if trust else "allow-unsigned=1")
    else:
        opts.insert(0, f"route={a.route}")
    if a.query:
        opts.append(a.query)
    url = root + "?" + "&".join(opts)

    checks, lines, timeline, errors = [], [], [], []
    navigations, crashes = [], []
    t0 = time.time()

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": str(detail)[:800]})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {str(detail)[:240]}", flush=True)

    summary = {"ok": False, "url": url, "label": label}
    with sync_playwright() as pw:
        browser = pw.webkit.launch()
        summary["webkit"] = browser.version
        dev = dict(pw.devices["iPhone 15 landscape" if a.landscape else "iPhone 15"])
        ctx = browser.new_context(**dev)
        ctx.add_init_script(FAKE_GPU)
        pg = ctx.new_page()
        pg.on("console", lambda m: lines.append(f"{time.time() - t0:8.2f} s  {m.type}: {m.text[:700]}"))
        pg.on("pageerror", lambda e: (lines.append(f"{time.time() - t0:8.2f} s  PAGEERROR {e}"),
                                      errors.append(str(e)[:400])))
        pg.on("crash", lambda _p: (crashes.append(time.time() - t0), lines.append(f"{time.time() - t0:8.2f} s  CRASH")))
        pg.on("framenavigated", lambda f: navigations.append((time.time() - t0, f.url))
              if f == pg.main_frame else None)

        def state():
            try:
                return json.loads(pg.evaluate(STATE_JS))
            except Exception as e:  # noqa: BLE001
                return {"error": str(e)[:300]}

        def sample(tag=""):
            st = state()
            h = st.get("heap") or {}
            mbs = lambda k: round((h.get(k) or 0) / 1048576, 1)  # noqa: E731
            rss = webkit_rss()
            s = {"t": round(time.time() - t0, 1), "tag": tag, "frames": st.get("frames"),
                 "wasm_mb": mbs("wasm"), "heap_mb": mbs("live"), "heap_peak_mb": mbs("peak"),
                 "gpu_meshes_mb": mbs("gpu_meshes"), "gpu_buffers_mb": mbs("gpu_buffers"), "gpu_textures_mb": mbs("gpu_textures"),
                 "rss_web_mb": round(rss["web"], 1), "rss_gpu_mb": round(rss["gpu"], 1),
                 "loads": st.get("loads"), "status": (st.get("status") or "")[:160],
                 "context": st.get("context")}
            s["phone_mb"] = round(s["wasm_mb"] + max(s["gpu_meshes_mb"], s["gpu_buffers_mb"]) + s["gpu_textures_mb"], 1)
            timeline.append(s)
            print(f"  {s['t']:7.1f} s {tag:10s} wasm {s['wasm_mb']:7.1f}  heap {s['heap_mb']:7.1f}  "
                  f"gpu {s['gpu_buffers_mb']:6.1f}+{s['gpu_textures_mb']:6.1f}  phone {s['phone_mb']:7.1f}  rss web {s['rss_web_mb']:7.1f} "
                  f"gpu {s['rss_gpu_mb']:6.1f}  frames {s['frames']}  loads {s['loads']}  {s['status'][:60]}",
                  flush=True)
            return st

        def wasm_mb():
            return round(pg.evaluate("window.ocw && window.ocw.wasmMemory ? window.ocw.wasmMemory() : 0") / 1048576, 1)

        def context_loss():
            """iOS takes the GPU from a page in the background: lost while
            hidden, the app stops (the memory stays) and the page loads the
            ride again once it is shown; lost again soon after, it stays
            stopped and says so (no reload loop, no memory growth)."""
            out = {}
            pg.evaluate("window.__setHidden(true)")
            w0 = wasm_mb()
            out["lose_1"] = pg.evaluate(LOSE_JS)
            pg.wait_for_timeout(10000)
            w1, st = wasm_mb(), state()
            sample("lost-hidden")
            out["hidden"] = {"wasm": [w0, w1], "loads": st.get("loads"), "context": st.get("context")}
            check("lost while hidden: the app stops (wasm memory flat over 13 s) and nothing reloads",
                  w1 - w0 < 30 and st.get("loads") == 1, out["hidden"])
            pg.evaluate("window.__setHidden(false)")
            try:
                pg.wait_for_function("() => window.__loads === 2 && window.ocwGpu && window.ocwFrames > 20"
                                     " && document.getElementById('ocw-status').hidden", timeout=240000)
                ok = True
            except Exception as e:  # noqa: BLE001
                ok, out["wait"] = False, str(e)[:300]
            pg.wait_for_timeout(5000)
            st = sample("reloaded")
            pg.screenshot(path=f"{base}-context-reloaded.png")
            drawn, how = image_drawn(f"{base}-context-reloaded.png")
            check("shown again: the page loads the ride once and it rides and draws",
                  ok and st.get("loads") == 2 and drawn and not st.get("status"), f"{how}; {st.get('loads')} loads")
            pg.evaluate(LOSE_JS)
            pg.wait_for_timeout(1000)
            w0 = wasm_mb()
            pg.wait_for_timeout(15000)
            w1, st = wasm_mb(), state()
            sample("lost-again")
            pg.screenshot(path=f"{base}-context-lost-again.png")
            out["again"] = {"wasm": [w0, w1], "loads": st.get("loads"), "context": st.get("context"),
                            "status": st.get("status")}
            check("lost again within ten minutes: no second reload, a message, the memory flat",
                  st.get("loads") == 2 and (st.get("context") or {}).get("action") == "message"
                  and "again" in (st.get("status") or "") and w1 - w0 < 20, out["again"])
            out["navigations"] = list(navigations)
            return out

        def deaths():
            """iOS kills a tab that uses too much memory and Safari loads it
            again – without a pagehide. Emulated: the page's pagehide mark
            switched off, then a reload. The first time the ride goes on
            with memory=saver, the second time the page stops with the
            reason instead of loading the ride again (no restart loop)."""
            out = {}
            n0 = state().get("loads")
            pg.evaluate("window.ocwGpuChoice.rideClosed = () => {}")
            pg.reload()
            try:
                pg.wait_for_function("() => window.ocwGpu && window.ocwFrames > 20"
                                     " && document.getElementById('ocw-status').hidden", timeout=240000)
            except Exception as e:  # noqa: BLE001
                out["wait"] = str(e)[:300]
            st = state()
            note = pg.evaluate("document.getElementById('ocw-gpu-note').textContent")
            out["first"] = {"href": st.get("href"), "loads": st.get("loads"), "note": note}
            pg.screenshot(path=f"{base}-died-once.png")
            check("died in the ride once: it rides on with memory=saver and says so",
                  "memory=saver" in (st.get("href") or "") and "less detail" in note
                  and st.get("loads") == n0 + 1 and not st.get("status"), out["first"])
            pg.evaluate("window.ocwGpuChoice.rideClosed = () => {}")
            pg.reload()
            pg.wait_for_timeout(8000)
            st = state()
            started = pg.evaluate("!!window.ocw")
            out["second"] = {"loads": st.get("loads"), "status": st.get("status"), "module": started}
            pg.screenshot(path=f"{base}-died-twice.png")
            check("died twice: the page stops with the reason and loads no ride (no restart loop)",
                  "stopped twice" in (st.get("status") or "") and not started and st.get("loads") == n0 + 2,
                  out["second"])
            return out

        try:
            pg.goto(url)
            try:
                pg.wait_for_function("() => window.ocwGpu && document.getElementById('ocw-status').hidden"
                                     " && window.ocwFrames > 20", timeout=300000)
            except Exception as e:  # noqa: BLE001
                lines.append(f"wait: {e}")
            ready_t = time.time()
            st = sample("ready")
            summary["ready_s"] = round(ready_t - t0, 1)
            check("the ride shows on WebGL 2 (Safari with WebGPU)", (st.get("gpu") or {}).get("api") == "webgl2"
                  and not st.get("status"), {"gpu": st.get("gpu"), "status": st.get("status")})
            pg.screenshot(path=f"{base}-start.png")
            try:
                smp = json.loads(pg.evaluate(SAMPLE_JS))
            except Exception as e:  # noqa: BLE001
                smp = {"error": str(e)[:200]}
            summary["canvas_sample"] = smp
            check("the WebGPU watchdog is not installed on WebGL 2 (its one-colour check never runs here)",
                  not smp.get("watchdog"), smp)
            end = ready_t + a.minutes * 60
            next_bg = ready_t + a.background_every if a.background_every > 0 else float("inf")
            next_shot = ready_t + 60
            while time.time() < end and not crashes:
                pg.wait_for_timeout(5000)
                now = time.time()
                tag = ""
                if now >= next_bg:
                    pg.evaluate("window.__setHidden(true)")
                    pg.wait_for_timeout(5000)
                    pg.evaluate("window.__setHidden(false)")
                    next_bg = now + a.background_every
                    tag = "background"
                sample(tag)
                if now >= next_shot:
                    pg.screenshot(path=f"{base}-{int((now - ready_t) / 60)}min.png")
                    next_shot += 60
            st = sample("end")
            shot = f"{base}-end.png"
            if not crashes:
                pg.screenshot(path=shot)
                drawn, how = image_drawn(shot)
            else:
                drawn, how = False, "crashed"
            check(f"rides {a.minutes:g} min and draws", drawn and st.get("frames", 0) > 100 and not st.get("status"),
                  f"{how}; frames {st.get('frames')}; status {st.get('status')!r}")
            check("no reload of the page while riding (no loop)", len(navigations) == 1 and st.get("loads") == 1,
                  {"navigations": navigations[:6], "loads": st.get("loads")})
            check("no crash of the web process", not crashes, crashes)
            check("no page errors", not errors, errors[:5])
            if not a.no_context_loss and not crashes:
                summary["context_loss"] = context_loss()
            if a.deaths and not crashes:
                summary["deaths"] = deaths()
            phone = max((s["phone_mb"] for s in timeline), default=0)
            wasm = max((s["wasm_mb"] for s in timeline), default=0)
            rss = max((s["rss_web_mb"] + s["rss_gpu_mb"] for s in timeline), default=0)
            summary.update({"peak_phone_mb": phone, "peak_wasm_mb": wasm, "peak_rss_mb": round(rss, 1)})
            if a.budget_mb > 0:
                check(f"wasm + GPU memory stays under the phone budget of {a.budget_mb:g} MB", phone <= a.budget_mb,
                      f"peak {phone} MB (wasm {wasm} MB; WebKit RSS peak {rss:.0f} MB)")
            summary["ok"] = all(c["ok"] for c in checks)
        except Exception as e:  # noqa: BLE001
            summary["error"] = repr(e)
            print("ERROR", repr(e))
        finally:
            try:
                browser.close()
            except Exception:  # noqa: BLE001
                pass
            if httpd:
                httpd.shutdown()
    summary.update({"checks": checks, "timeline": timeline, "navigations": navigations, "crashes": crashes})
    with open(base + ".log", "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(base + ".json", "w") as f:
        json.dump(summary, f, indent=2)
    failed = [c for c in checks if not c["ok"]]
    print(f"{len(checks) - len(failed)} of {len(checks)} checks passed; {base}.json")
    sys.exit(0 if summary["ok"] and checks else 1)


if __name__ == "__main__":
    main()
