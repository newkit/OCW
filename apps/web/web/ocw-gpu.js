// The browser app's GPU choice and its watchdog (spec §14 Phase 10 B7,
// B8a; §3.5). Bevy is compiled for WebGPU or for WebGL 2, so the page loads
// one of two modules; this file decides which, and makes sure a WebGPU that
// fails never leaves a silent black screen.
//
//   ocwGpuChoice.facts(nav, win)        the browser facts the choice uses
//   ocwGpuChoice.choose({query, facts, gpu, webgl2, storage})
//                                       → {api, reason, forced, note}
//   ocwGpuChoice.recall / remember / forget (storage, …)
//                                       the remembered WebGPU failure
//   ocwGpuChoice.fallbackUrl(href, why) the same ride on WebGL 2
//   ocwGpuChoice.startPage(facts, remembered, hasGpu)
//                                       the start page's checkbox
//   ocwGpuChoice.uniform(rgba)          a sampled canvas of one colour
//   ocwGpuChoice.watch({...})           the WebGPU watchdog (below)
//   ocwGpuChoice.simd(WebAssembly)      WebAssembly SIMD there? (Q10 (a):
//                                       both builds use it; the page says
//                                       "too old" without, TOO_OLD)
//
// The rules (in this order):
//   1. `?gpu=webgl2` / `?gpu=webgpu` choose (a test can force WebGPU);
//   2. a WebGPU failure remembered in this browser (localStorage, the same
//      user agent: an update of the browser tries WebGPU again) → WebGL 2;
//   3. Safari's WebGPU (WebKit: Safari on macOS, every browser on iOS and
//      iPadOS) → WebGL 2. Safari 26 ships WebGPU on by default, and the
//      WebGPU build gave an iPhone 15 on iOS 26 sound but no image
//      (2026-11-15); WebKit's WebGL 2 build draws (checked in WebKit);
//   4. a WebGPU adapter → WebGPU; else WebGL 2; else neither.
//
// The watchdog (WebGPU only) reports a failure when the device is lost or
// reports an uncaptured error, `requestDevice` fails, the module does not
// start, Bevy's renderer reports an error (`window.ocwRenderFailed`, called
// by the app), no frame was drawn after 20 s of a visible page, or the
// canvas stays one colour once the ride shows. The page then remembers it
// and reloads the ride on WebGL 2 with a note – unless `gpu=webgpu` forced
// WebGPU, where it shows the error instead.
(function (root) {
  "use strict";
  const KEY = "ocw.gpu.webgpu-failed";
  const FAILED_PARAM = "webgpu-failed";

  // WebAssembly SIMD (spec 14 Phase 10, Q10 (a)): both modules are built
  // with `+simd128`, and a browser without SIMD cannot compile them – so
  // the page validates this tiny module first (a function that returns a
  // v128: i32.const 0, i8x16.splat, i8x16.popcnt; the test of
  // wasm-feature-detect) and tells such a browser it is too old instead of
  // a failed load. Safari 16.4, Chrome 91 and Firefox 89 have it.
  const SIMD_TEST = [0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0,
    10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11];
  const TOO_OLD = "This browser is too old for Open Cycling World – please update "
    + "(Safari 16.4 / Chrome 91 / Firefox 89 or newer).";

  function simd(wa) {
    try {
      return !!(wa && typeof wa.validate === "function" && wa.validate(new Uint8Array(SIMD_TEST)));
    } catch (e) {
      return false;
    }
  }

  function facts(nav, win) {
    nav = nav || {};
    win = win || {};
    const ua = String(nav.userAgent || "");
    const touch = Number(nav.maxTouchPoints || 0);
    // iPadOS asks for desktop pages: a "Mac" with a touch screen.
    const ios = /iPhone|iPad|iPod/.test(ua) || (nav.platform === "MacIntel" && touch > 1);
    // navigator.vendor is "Apple Computer, Inc." in WebKit only (Safari,
    // and every browser on iOS); Chromium says "Google Inc.", Firefox "".
    const webkit = /^Apple/.test(String(nav.vendor || "")) || ios;
    const coarse = !!(win.matchMedia && win.matchMedia("(pointer: coarse)").matches);
    const mobile = ios || !!(nav.userAgentData ? nav.userAgentData.mobile
      : /Android|iPhone|iPad|iPod|Mobile/i.test(ua));
    return {ua, ios, webkit, mobile, coarse, phone: mobile || (coarse && touch > 0)};
  }

  function storageGet(storage, key) {
    try { return storage ? storage.getItem(key) : null; } catch (e) { return null; }
  }
  function storageSet(storage, key, value) {
    try { if (storage) { storage.setItem(key, value); return true; } } catch (e) { /* private mode, blocked */ }
    return false;
  }
  function storageDel(storage, key) {
    try { if (storage) storage.removeItem(key); } catch (e) { /* ignore */ }
  }

  // The remembered WebGPU failure of this browser, or null (another user
  // agent – the browser was updated – forgets it).
  function recall(storage, ua) {
    const text = storageGet(storage, KEY);
    if (!text) return null;
    let e;
    try { e = JSON.parse(text); } catch (err) { return null; }
    if (!e || typeof e !== "object" || e.ua !== ua) return null;
    return {reason: String(e.reason || "unknown"), at: String(e.at || ""), build: String(e.build || "")};
  }
  function remember(storage, ua, reason, at, build) {
    return storageSet(storage, KEY, JSON.stringify({ua, reason: String(reason).slice(0, 300), at, build: build || ""}));
  }
  function forget(storage) { storageDel(storage, KEY); }

  async function probeAdapter(gpu) {
    if (!gpu) return {ok: false, why: "no WebGPU in this browser (navigator.gpu)"};
    try {
      if (await gpu.requestAdapter()) return {ok: true, why: "WebGPU adapter found"};
      return {ok: false, why: "WebGPU without an adapter (blocklisted or disabled GPU)"};
    } catch (e) {
      return {ok: false, why: "WebGPU failed: " + e};
    }
  }

  // The GPU API for this visit. `query`: URLSearchParams; `gpu`:
  // navigator.gpu; `webgl2`: () => bool; `storage`: localStorage or null.
  async function choose(o) {
    const q = o.query;
    const f = o.facts;
    const want = q.get("gpu");
    const failedBefore = q.has(FAILED_PARAM);
    const remembered = recall(o.storage, f.ua);
    const webgl2 = () => { try { return !!o.webgl2(); } catch (e) { return false; } };
    const note = failedBefore || remembered
      ? "WebGPU failed in this browser" + (remembered ? " (" + remembered.reason + ")" : "")
        + " – using WebGL 2. ?gpu=webgpu tries WebGPU again."
      : null;
    if (want === "webgl2") return {api: "webgl2", reason: "page option gpu=webgl2", forced: true, note};
    if (want === "webgpu") {
      const a = await probeAdapter(o.gpu);
      if (a.ok) return {api: "webgpu", reason: "page option gpu=webgpu (" + a.why + ")", forced: true, note: null};
      return {api: null, reason: a.why + "; gpu=webgpu asked for it", forced: true, note: null};
    }
    if (!webgl2()) {
      // Nothing to fall back to: WebGPU is the only chance.
      const a = await probeAdapter(o.gpu);
      if (a.ok) return {api: "webgpu", reason: a.why + " (no WebGL 2)", forced: false, note: null};
      return {api: null, reason: a.why + "; no WebGL 2 either", forced: false, note: null};
    }
    if (remembered) {
      return {api: "webgl2", forced: false, note,
        reason: "WebGPU failed in this browser before: " + remembered.reason
          + (remembered.at ? " (" + remembered.at + ")" : "")};
    }
    if (f.webkit && o.gpu) {
      return {api: "webgl2", forced: false, note: null,
        reason: "Safari's WebGPU (WebKit" + (f.ios ? ", iOS / iPadOS" : "")
          + ") is not used yet – no image on iOS 26 (spec §14 Phase 10 B7); ?gpu=webgpu tries it"};
    }
    const a = await probeAdapter(o.gpu);
    if (a.ok) return {api: "webgpu", reason: a.why, forced: false, note: null};
    return {api: "webgl2", reason: a.why, forced: false, note};
  }

  // The same ride on WebGL 2 with low graphics, marked as a fallback.
  function fallbackUrl(href) {
    const u = new URL(href);
    u.searchParams.set("gpu", "webgl2");
    u.searchParams.set("quality", "low");
    u.searchParams.set(FAILED_PARAM, "1");
    return u.toString();
  }

  // Whether a failure of `gpu` may reload the page on WebGL 2: only a
  // WebGPU the page chose itself, never from WebGL 2, never a second time
  // (the URL already is a fallback) – else the error shows (no loop).
  function mayFallBack(gpu, href, hasWebgl2) {
    if (!gpu || gpu.api !== "webgpu" || gpu.forced || !hasWebgl2) return false;
    let q;
    try { q = new URL(href).searchParams; } catch (e) { return false; }
    return !q.has(FAILED_PARAM) && q.get("gpu") !== "webgl2";
  }

  // The page's own automatic reloads after a lost WebGL context (the
  // WebGPU fallback cannot loop: `mayFallBack`): at most one in RELOAD_WINDOW_MS per tab (sessionStorage, it
  // survives the reload), so a failure that comes back shows a message
  // instead of reloading forever. `now`: ms (Date.now()). True: reload
  // (and the reload is recorded); false: show the message.
  const RELOAD_KEY = "ocw.page.auto-reload";
  const RELOAD_WINDOW_MS = 10 * 60 * 1000;
  function mayReload(storage, now, why) {
    let last = null;
    try { last = JSON.parse(storageGet(storage, RELOAD_KEY) || "null"); } catch (e) { last = null; }
    if (last && typeof last.at === "number" && now - last.at >= 0 && now - last.at < RELOAD_WINDOW_MS) return false;
    // Without a storage the page cannot know it reloaded before: no reload.
    return storageSet(storage, RELOAD_KEY, JSON.stringify({at: now, why: String(why || "").slice(0, 200)}));
  }

  // A ride that ended without its page being closed (iOS Safari kills a
  // tab that uses too much memory and loads it again, spec 14 Phase 10
  // B2): `rideStarted` marks the ride in sessionStorage, `rideClosed`
  // (pagehide) clears it; `lastRideDied` at the next start says the page
  // before died in the ride, and how often in a row. The page then rides
  // with less memory, and after two such ends shows the start page with
  // the reason instead of loading the world again (no restart loop).
  const RIDE_KEY = "ocw.page.ride";
  function lastRideDied(storage, href) {
    let r = null;
    try { r = JSON.parse(storageGet(storage, RIDE_KEY) || "null"); } catch (e) { r = null; }
    if (!r || !r.open) return {died: 0, href: null};
    return {died: Number(r.died || 0) + 1, href: String(r.href || ""), same: r.href === href};
  }
  function rideStarted(storage, href, died) {
    return storageSet(storage, RIDE_KEY, JSON.stringify({open: true, href, died: died || 0}));
  }
  function rideClosed(storage) { storageDel(storage, RIDE_KEY); }

  // The same ride with less memory after the page died in it: Low
  // graphics, the phone's memory saver, marked so the page says why.
  function saverUrl(href) {
    const u = new URL(href);
    u.searchParams.set("quality", "low");
    u.searchParams.set("memory", "saver");
    return u.toString();
  }

  // Each death goes one step further (v5.42: the iPhone died in the saver
  // mode too): the page's memory mode → saver → minimal → stop. Returns
  // {href, mode} for the next try, or null when even the minimal mode
  // died (the page stops and says so; never a loop).
  const MEMORY_STEPS = ["saver", "minimal"];
  function nextMemoryStep(href) {
    const u = new URL(href);
    const now = u.searchParams.get("memory");
    const i = MEMORY_STEPS.indexOf(now);
    if (i === MEMORY_STEPS.length - 1) return null;
    const mode = MEMORY_STEPS[i + 1];
    u.searchParams.set("quality", "low");
    u.searchParams.set("memory", mode);
    return {href: u.toString(), mode};
  }

  // The canvas' pixel ratio for the app on a phone (spec 14 Phase 10 B7,
  // the iPhone's memory). WebKit (Safari, every browser on iOS) has no
  // `devicePixelContentBoxSize`, so winit sizes the canvas at the full
  // device pixel ratio (3 on an iPhone: 1179 × 2556 px) and the app's own
  // cap (the quality preset's) cannot apply there – every render target
  // (main target, depth, the canvas' buffers) is 9 × that of a 1 × canvas,
  // and iOS counts it against the tab. So the page tells the app a smaller
  // `devicePixelRatio` before it starts: the drawing buffer shrinks, the
  // CSS size stays (the browser scales the picture up). `?pixel-ratio=N`
  // (0.5 – 4) chooses; else phones and tablets in WebKit get 1.5 (1 in the
  // memory saver modes); elsewhere `null` (the device's own: desktops and
  // Chromium phones, where the preset's cap works, are unchanged).
  function pixelRatioCap(f, query, dpr, hasDeviceBox) {
    const q = Number(query && query.get ? query.get("pixel-ratio") : NaN);
    let cap = null;
    if (q >= 0.5 && q <= 4) cap = q;
    else if (!hasDeviceBox && f && f.phone) {
      const mem = query && query.get ? query.get("memory") : null;
      cap = mem === "saver" || mem === "minimal" ? 1 : 1.5;
    }
    return cap !== null && Number(dpr || 1) > cap ? cap : null;
  }

  // The start page's checkbox "WebGL 2 with low graphics": ticked where
  // the page would take WebGL 2 anyway (Safari, a remembered failure, no
  // WebGPU) and on phones and tablets. `query(checked)`: what a ride card
  // adds to its options – the box unticked where the page would take
  // WebGL 2 means "try WebGPU".
  function startPage(f, remembered, hasGpu) {
    const webgl2Anyway = !hasGpu || !!remembered || f.webkit;
    const ticked = webgl2Anyway || f.phone;
    return {
      ticked,
      query: (checked) => checked ? "&gpu=webgl2&quality=low" : (webgl2Anyway && hasGpu ? "&gpu=webgpu" : ""),
      webgpuLabel: f.webkit && hasGpu ? "WebGPU (not used in Safari yet)" : "WebGPU",
    };
  }

  // A sampled canvas (RGBA bytes) of one colour: nothing was drawn.
  function uniform(rgba, tolerance) {
    const tol = tolerance == null ? 2 : tolerance;
    if (!rgba || rgba.length < 8) return false;
    for (let i = 4; i < rgba.length; i += 4) {
      for (let c = 0; c < 3; c++) if (Math.abs(rgba[i + c] - rgba[c]) > tol) return false;
    }
    return true;
  }

  // The WebGPU watchdog. o: {win, canvas, started: () => bool (the module
  // runs), ready: () => bool (the ride shows), onFail(reason), onOk(),
  // frameSeconds (20), sampleEvery (2), samples (3)}. Installed before the
  // module requests its device; `state.fail(why)` reports a failure found
  // elsewhere (the module's start, Bevy's renderer).
  function watch(o) {
    const win = o.win;
    const state = {frames: 0, failed: null, ok: false, visibleS: 0, uniformRuns: 0, samples: [], device: false};
    // A failure ends the watch; a drawn frame ends only the checks of the
    // start (the device's errors are watched for the whole ride).
    let failed = false;
    const fail = (why) => {
      if (failed) return;
      failed = true;
      state.failed = String(why);
      try { o.onFail(state.failed); } catch (e) { /* the page's handler */ }
    };
    const pass = () => {
      if (failed || state.ok) return;
      state.ok = true;
      try { if (o.onOk) o.onOk(); } catch (e) { /* ignore */ }
    };
    state.fail = fail;
    if (win.GPUAdapter && win.GPUAdapter.prototype.requestDevice) {
      const rd = win.GPUAdapter.prototype.requestDevice;
      win.GPUAdapter.prototype.requestDevice = function (desc) {
        const p = rd.call(this, desc);
        return Promise.resolve(p).then((dev) => {
          state.device = true;
          try {
            dev.addEventListener("uncapturederror", (e) => {
              const err = e && e.error;
              fail("WebGPU error: " + ((err && err.message) || String(err)).slice(0, 300));
            });
            dev.lost.then((info) => {
              if (info && info.reason === "destroyed") return;
              fail("WebGPU device lost: " + ((info && info.message) || (info && info.reason) || "unknown"));
            });
          } catch (e) { /* a device without the events */ }
          return dev;
        }, (e) => {
          fail("WebGPU requestDevice failed: " + e);
          throw e;
        });
      };
    }
    let sampleWanted = false;
    const sample = () => {
      sampleWanted = false;
      let rgba;
      try {
        const c = win.document.createElement("canvas");
        c.width = 32;
        c.height = 18;
        const g = c.getContext("2d", {willReadFrequently: true});
        g.drawImage(o.canvas, 0, 0, c.width, c.height);
        rgba = g.getImageData(0, 0, c.width, c.height).data;
      } catch (e) {
        state.samples.push("unreadable: " + e);
        return;
      }
      const one = uniform(rgba);
      state.samples.push(one ? "uniform" : "drawn");
      if (!one) pass();
      else if (++state.uniformRuns >= (o.samples || 3)) {
        fail("nothing drawn: the canvas stayed one colour (rgb " + rgba[0] + ", " + rgba[1] + ", " + rgba[2] + ")");
      }
    };
    if (win.GPUCanvasContext && win.GPUCanvasContext.prototype.getCurrentTexture) {
      const gct = win.GPUCanvasContext.prototype.getCurrentTexture;
      win.GPUCanvasContext.prototype.getCurrentTexture = function () {
        const t = gct.call(this);
        state.frames++;
        // After this frame's commands (the same task, before the browser
        // presents): the canvas still holds the frame.
        // The sample runs in a microtask after the app's frame (Bevy draws
        // in one call into the module): drawImage then reads this frame's
        // texture, before the browser presents it.
        if (sampleWanted && this.canvas === o.canvas) {
          sampleWanted = false;
          Promise.resolve().then(sample);
        }
        return t;
      };
    }
    // Seconds counted while the page is visible and the module has started
    // (a slow download is no failure; a hidden tab draws nothing).
    const every = o.sampleEvery || 2;
    let lastSample = -every;
    const timer = win.setInterval(() => {
      if (failed || state.ok) { win.clearInterval(timer); return; }
      if (win.document.visibilityState !== "visible" || !o.started()) return;
      state.visibleS += 1;
      if (state.frames === 0 && state.visibleS >= (o.frameSeconds || 20)) {
        fail("no frame drawn in " + state.visibleS + " s");
        return;
      }
      if (state.frames > 0 && o.ready() && state.visibleS - lastSample >= every) {
        lastSample = state.visibleS;
        sampleWanted = true;
      }
    }, 1000);
    return state;
  }

  root.ocwGpuChoice = {facts, choose, recall, remember, forget, fallbackUrl, startPage, uniform, watch, KEY, FAILED_PARAM,
    mayFallBack, mayReload, RELOAD_KEY, RELOAD_WINDOW_MS, lastRideDied, rideStarted, rideClosed, saverUrl, RIDE_KEY,
    pixelRatioCap, nextMemoryStep, MEMORY_STEPS, simd, SIMD_TEST, TOO_OLD};
})(typeof window !== "undefined" ? window : globalThis);
