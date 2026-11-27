// The page log (spec 14 Phase 10, 10G *The page log*): what a rider on a
// phone without a computer can see and send when the app fails – on an
// iPhone Safari's console needs a Mac and a cable, on Android
// chrome://inspect a computer.
//
// This is the page's FIRST script (index.html loads it before every other
// one), so it captures from the very first line, before the wasm loads:
// every console.log / info / warn / error / debug (the Rust side's lines
// too: Bevy's tracing-wasm writes "%cINFO%c file.rs:12 %cmessage" with
// console.log, its panic hook console.error), uncaught errors (message,
// file:line:column, the stack), failed script / image loads, unhandled
// promise rejections, CSP violations and the page's life (hidden / shown,
// pagehide, online / offline), and every 15 s the memory while the app
// runs. The capture is a cheap ring buffer of the last LIMIT lines; it is
// also kept in sessionStorage (this tab, survives a reload – also the one
// iOS makes after it killed the page), so the next load shows the previous
// page load's log.
//
// The early log (spec 14 Phase 10, 10G *Bisecting a phone's crash*; the
// iPhone that lost the page 2 s into the first frames, 2026-11-27): for
// the first EARLY_MS every line is written to sessionStorage at once
// (synchronously – the last line before a kill survives), the first
// FRAMES_LOGGED frames are logged with their time (and every long one
// after), the memory is sampled every 100 ms (wasm, the JS heap where the
// browser has performance.memory) and logged when it moved by 1 MB or a
// second passed, and each sample goes to its own small key (KEY_PULSE): the
// next load says when the page last showed a sign of life. The app's
// pipeline compiles (name, ms) come as its own lines.
//
// The world load's stages (10G: the iPhone that closed the page while a
// world's base was decoded, 2026-11-27): every "world load: …" line of the
// app (each with the wasm memory and the heap) is also kept at once in its
// own small key (KEY_STAGES, at any time, not only in the early log) and
// pulses; the next load adds the stages its saved log missed and names the
// last one in the title of "the page load before".
//
// The panel is built only when it is shown:
//   ?log=1 in the address, or five quick taps on the version stamp (bottom
//   right corner) or on the start page's build line; the panel's Hide ends
//   it (a reload keeps it open until then: sessionStorage);
//   "Show log", offered by itself when the page shows an error (the
//   status's class "error": too old, no WebGPU / WebGL 2, the module did
//   not load, a renderer failure, the ride stopped after crashes) or the
//   loading screen stays for 45 s.
// Its buttons: Copy (the clipboard, else the text selected for the
// phone's own Copy), Share (the share sheet: a .txt file, else the text),
// Download (.txt), Clear, the size, Hide.
//
//   window.ocwLog.lines()        this page load's lines (strings)
//   window.ocwLog.previous()     {started, ended, stage, lines} of the load before, or null
//   window.ocwLog.stages()       this load's world load stage lines
//   window.ocwLog.report()       Promise → the whole text (header, previous, this load)
//   window.ocwLog.header()       Promise → the environment header
//   window.ocwLog.show() / hide() / offer(why) / clear()
//   window.ocwLog.state()        JSON for the tests
//   window.ocwLog.scrub(text)    the redaction (tests)
//
// Old browsers must reach the log too ("this browser is too old"): plain
// ES2017 only – no optional chaining, no ??, no class fields.
(function () {
  "use strict";
  if (window.ocwLog) return;
  var LIMIT = 2000;
  var LINE_MAX = 4000;
  // Each page load's stored log stays below this many characters (the
  // page's own sessionStorage keys need room too).
  var STORE_MAX = 700000;
  var KEY_RUN = "ocw.log.run";
  var KEY_PREV = "ocw.log.prev";
  var KEY_SHOW = "ocw.log.show";
  var KEY_PULSE = "ocw.log.pulse";
  var KEY_STAGES = "ocw.log.stages";
  var STAGES_MAX = 150;
  var STAGE = /world load: /;
  var stages = [];
  var EARLY_MS = 15000;
  var FRAMES_LOGGED = 30;
  var t0 = (window.performance && performance.timeOrigin) || Date.now();
  var now = function () { return window.performance && performance.now ? performance.now() : Date.now() - t0; };
  var started = new Date(t0).toISOString();
  var lines = [];
  var dropped = 0;
  var listeners = [];
  var storage = (function () { try { return window.sessionStorage; } catch (e) { return null; } })();
  var sget = function (k) { try { return storage ? storage.getItem(k) : null; } catch (e) { return null; } };
  var sset = function (k, v) { try { if (storage) storage.setItem(k, v); return true; } catch (e) { return false; } };
  var sdel = function (k) { try { if (storage) storage.removeItem(k); } catch (e) { /* none */ } };

  // Secrets never go into the log: tokens, the bridge's pairing code, a
  // sign-in code, the rider's name in the address.
  var SECRET_KEYS = /^(access_token|refresh_token|id_token|token|client_secret|secret|password|pass|auth|key|api_key|code|state|bridge|name|session|sig|signature)$/i;
  function scrub(text) {
    return String(text)
      .replace(/\b(access_token|refresh_token|id_token|client_secret|password|secret|api_key|token)(["']?\s*[:=]\s*["']?)(?!\(hidden\))[^"'&\s,;})]+/gi, "$1$2(hidden)")
      .replace(/([?&#](?:code|state|bridge|name)=)(?!\(hidden\))[^&#\s"']+/gi, "$1(hidden)")
      // The app's tracing fields: "name = Ada Rider;" (the rider's name).
      .replace(/\b(name|display_name|email) = (?!\(hidden\))[^;\n]+;/g, "$1 = (hidden);")
      .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+\/=-]{8,}/g, "$1 (hidden)")
      // The build machine's home folder in the app's source paths
      // (".../home/NAME/.cargo/registry/src/INDEX/bevy_…" → "cargo:bevy_…").
      .replace(/(?:\/(?:home|Users)\/[^\/\s]+|[A-Za-z]:\\Users\\[^\\\s]+)\/\.cargo\/registry\/src\/[^\/\s]+\//g, "cargo:")
      .replace(/\/(home|Users)\/[^\/\s]+\//g, "~/")
      .replace(/[A-Za-z]:\\Users\\[^\\\s]+\\/g, "~\\");
  }
  function query() {
    var out = [];
    try {
      new URLSearchParams(location.search).forEach(function (v, k) {
        out.push(k + "=" + (SECRET_KEYS.test(k) ? "(hidden)" : v));
      });
    } catch (e) { return "?"; }
    return out.length ? "?" + out.join("&") : "(none)";
  }

  // console's arguments as one line: "%c" styles dropped (the Rust lines),
  // %s / %d / %i / %f / %o / %O filled in, errors with their stack.
  function str(v) {
    if (typeof v === "string") return v;
    if (v instanceof Error) return (v.stack && v.stack.indexOf(v.message) >= 0 ? v.stack : String(v) + (v.stack ? "\n" + v.stack : ""));
    if (v === null || v === undefined || typeof v !== "object") return String(v);
    try {
      var s = String(v);
      if (s !== "[object Object]") return s;
      return JSON.stringify(v).slice(0, LINE_MAX);
    } catch (e) { return Object.prototype.toString.call(v); }
  }
  function format(args) {
    var a = Array.prototype.slice.call(args);
    var out = "";
    if (typeof a[0] === "string" && a[0].indexOf("%") >= 0) {
      var f = a.shift();
      out = f.replace(/%([csdifoO%])/g, function (m, c) {
        if (c === "%") return "%";
        if (!a.length) return m;
        var v = a.shift();
        if (c === "c") return "\u0001";
        if (c === "d" || c === "i") return String(parseInt(v, 10));
        if (c === "f") return String(Number(v));
        return str(v);
      });
    } else if (a.length) {
      out = str(a.shift());
    }
    for (var i = 0; i < a.length; i++) out += " " + str(a[i]);
    // A style switch between two words keeps them apart (tracing-wasm's
    // "%cINFO%c file.rs:12%cmessage" → "INFO file.rs:12 message").
    return out.replace(/(\S)\u0001+(?=\S)/g, "$1 ").replace(/\u0001/g, "");
  }
  // The Rust side's lines ("INFO crates/x.rs:12 message") keep their level.
  var RUST = /^(ERROR|WARN|INFO|DEBUG|TRACE) /;
  var LEVEL = {error: "E", warn: "W", info: "I", log: "L", debug: "D", uncaught: "!", page: "P"};
  function stamp(ms) {
    var s = (ms / 1000).toFixed(3);
    while (s.length < 8) s = " " + s;
    return "+" + s;
  }
  function add(kind, text) {
    var lv = LEVEL[kind] || "L";
    text = String(text);
    var m = RUST.exec(text);
    if (m) {
      lv = {ERROR: "E", WARN: "W", INFO: "I", DEBUG: "D", TRACE: "D"}[m[1]];
      text = "rs " + text.slice(m[0].length);
      // Bevy's note when a pipeline was tried before a shader library it
      // imports had loaded (spec 10G): it is tried again in a later frame
      // (the app's "pipeline N ok" line follows), not an error.
      if (/has an unresolved import/.test(text)) text += " [not yet loaded – tried again later, see the pipeline lines]";
    }
    if (text.length > LINE_MAX) text = text.slice(0, LINE_MAX) + " … (" + text.length + " characters)";
    var line = stamp(now()) + " " + lv + " " + scrub(text);
    lines.push(line);
    if (lines.length > LIMIT + 200) {
      dropped += lines.length - LIMIT;
      lines.splice(0, lines.length - LIMIT);
    }
    // A world load's stage: at once in its own small key (the page may be
    // killed in the next step of the load).
    if (STAGE.test(text)) {
      stages.push(line);
      if (stages.length > STAGES_MAX) stages.splice(0, stages.length - STAGES_MAX);
      sset(KEY_STAGES, JSON.stringify({started: started, lines: stages}));
      pulse();
    }
    // The early log: at once (the page may be killed before a timer).
    if (now() < EARLY_MS) { save(); pulse(); } else dirty();
    for (var i = 0; i < listeners.length; i++) { try { listeners[i](line); } catch (e) { /* the panel */ } }
  }

  // The console, wrapped: the line first, then the browser's own console
  // (the developer tools show this file as the caller; add ocw-log.js to
  // their ignore list for the real one).
  var original = {};
  ["log", "info", "warn", "error", "debug"].forEach(function (k) {
    var f = console[k];
    if (typeof f !== "function") return;
    original[k] = f;
    console[k] = function () {
      try { add(k, format(arguments)); } catch (e) { /* never break a log call */ }
      return f.apply(console, arguments);
    };
  });

  // Uncaught errors (and failed loads of scripts, images, … : the capture
  // phase sees them), unhandled rejections, CSP violations.
  addEventListener("error", function (e) {
    var t = e.target;
    if (t && t !== window && (t.src || t.href)) {
      add("uncaught", "could not load " + (t.tagName || "").toLowerCase() + " " + (t.src || t.href));
      return;
    }
    var where = e.filename ? " (" + e.filename + ":" + (e.lineno || 0) + ":" + (e.colno || 0) + ")" : "";
    var stack = e.error && e.error.stack ? "\n" + e.error.stack : "";
    // "Script error." without a file: the browser hides an error of a
    // script from another origin. The page's own scripts load with
    // crossorigin="anonymous" (their errors show in full), so it is most
    // likely a script the browser itself put into the page (Firefox and
    // Chrome for iOS inject their own), not the app.
    var hidden = /^Script error\.?$/.test(e.message || "") && !e.filename
      ? " (the details are hidden: a script of another origin – on Firefox / Chrome for iOS most likely the browser's own, not the app's)" : "";
    add("uncaught", "uncaught " + (e.message || str(e.error)) + where + hidden + stack);
  }, true);
  addEventListener("unhandledrejection", function (e) {
    add("uncaught", "unhandled promise rejection: " + str(e.reason));
  });
  document.addEventListener("securitypolicyviolation", function (e) {
    add("uncaught", "blocked by the page's security policy: " + e.violatedDirective + " " + (e.blockedURI || ""));
  });
  // The page's life: iOS takes the GPU (and kills pages) in the background.
  document.addEventListener("visibilitychange", function () {
    add("page", "page " + document.visibilityState);
    save();
  });
  addEventListener("pageshow", function (e) { if (e.persisted) add("page", "page shown again from the back/forward cache"); });
  addEventListener("pagehide", function () {
    add("page", "pagehide (a reload, a link or the tab closed)");
    save("closed normally (pagehide at " + stamp(now()).trim() + " s)");
  });
  addEventListener("online", function () { add("page", "online"); });
  addEventListener("offline", function () { add("page", "offline"); });
  document.addEventListener("freeze", function () { add("page", "frozen by the browser"); save(); });
  document.addEventListener("resume", function () { add("page", "resumed"); });
  // The memory every 15 s while the app runs (what it used before the
  // browser closed it), only when it changed by 8 MB or more.
  var lastMem = 0, lastGpu = 0;
  setInterval(function () {
    var wasm = 0;
    try { if (window.ocw && window.ocw.wasmMemory) wasm = window.ocw.wasmMemory(); } catch (e) { wasm = 0; }
    var g = gpuBytes();
    if (!wasm || (Math.abs(wasm - lastMem) < 8 * 1048576 && Math.abs(g - lastGpu) < 8 * 1048576)) return;
    lastMem = wasm;
    lastGpu = g;
    var js = window.performance && performance.memory
      ? ", JS heap " + mb(performance.memory.usedJSHeapSize) + " of " + mb(performance.memory.jsHeapSizeLimit) : "";
    add("page", "memory: wasm " + mb(wasm) + js + gpuText(g));
  }, 15000);
  function mb(b) { return Math.round(b / 1048576) + " MB"; }
  // The GPU memory the renderer allocated (textures and buffers created
  // through it: render targets, depth and shadow maps, meshes, images;
  // window.ocw.gpuAllocated(), 10G) plus the canvas' own buffers (about
  // two colour buffers and a depth buffer of its size, 4 bytes a pixel):
  // an estimate – the browser's GPU process keeps its own share.
  function gpuBytes() {
    var g = 0;
    try { if (window.ocw && window.ocw.gpuAllocated) g = Number(JSON.parse(window.ocw.gpuAllocated()).total || 0); } catch (e) { g = 0; }
    return g;
  }
  function canvasBytes() {
    var c = document.getElementById("ocw-canvas") || document.querySelector("canvas");
    return c && c.width && c.height ? c.width * c.height * 4 * 3 : 0;
  }
  function gpuText(g) {
    if (!g) return "";
    var c = canvasBytes();
    return ", gpu ~" + mb(g + c) + " (renderer " + mb(g) + (c ? ", canvas " + mb(c) : "") + ")";
  }

  // The early log's samples (every 100 ms for EARLY_MS) and frames.
  function wasmBytes() {
    try { if (window.ocw && window.ocw.wasmMemory) return window.ocw.wasmMemory() || 0; } catch (e) { /* not started */ }
    return 0;
  }
  function jsHeap() {
    var pm = window.performance && performance.memory;
    return pm ? pm.usedJSHeapSize : 0;
  }
  var pulseTimer = 0;
  // The last sign of life: one small key, written with every early line
  // and sample (cheap: a few dozen bytes).
  function pulse() {
    if (!storage) return;
    sset(KEY_PULSE, JSON.stringify({started: started, t: Math.round(now()), wasm: wasmBytes(), js: jsHeap(),
      gpu: gpuBytes(), frames: window.ocwFrames || 0, last: lines.length ? lines[lines.length - 1].slice(0, 200) : ""}));
  }
  var sampled = {t: -1e9, wasm: 0, js: 0, gpu: 0};
  function sample() {
    var t = now();
    if (t > EARLY_MS) { clearInterval(pulseTimer); add("page", "early log ends (" + (EARLY_MS / 1000) + " s): lines are saved every 1.5 s from now on"); return; }
    var wasm = wasmBytes(), js = jsHeap(), gpu = gpuBytes();
    pulse();
    var moved = Math.abs(wasm - sampled.wasm) >= 1048576 || Math.abs(js - sampled.js) >= 1048576
      || Math.abs(gpu - sampled.gpu) >= 1048576;
    if ((wasm || js) && (moved || t - sampled.t >= 1000)) {
      sampled = {t: t, wasm: wasm, js: js, gpu: gpu};
      add("page", "memory: wasm " + (wasm ? mb(wasm) : "-") + (js ? ", JS heap " + mb(js) : "") + gpuText(gpu) + ", frames " + (window.ocwFrames || 0));
    }
  }
  pulseTimer = setInterval(sample, 100);
  (function frames() {
    if (!window.requestAnimationFrame) return;
    var n = 0, last = 0;
    var tick = function (ts) {
      n++;
      if (last) {
        var dt = ts - last;
        if (n <= FRAMES_LOGGED + 1 || dt >= 250) add("page", "frame " + (n - 1) + ": " + dt.toFixed(0) + " ms" + (n > FRAMES_LOGGED + 1 ? " (a long one)" : ""));
      }
      last = ts;
      if (now() < EARLY_MS) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  })();

  // sessionStorage: this load's lines (at most every 1.5 s, at once when
  // the page is hidden or closed); the load before moves to KEY_PREV.
  var previous = null;
  (function () {
    var raw = sget(KEY_RUN);
    if (raw) { sset(KEY_PREV, raw); sdel(KEY_RUN); } else raw = sget(KEY_PREV);
    if (!raw) return;
    try {
      var p = JSON.parse(raw);
      if (p && p.lines && p.lines.length) previous = p;
    } catch (e) { previous = null; }
    // The world load's stages its saved lines missed (written after its
    // last save), and the last one.
    try {
      var st = JSON.parse(sget(KEY_STAGES) || "null");
      if (st && previous && st.started === previous.started && st.lines && st.lines.length) {
        var have = {};
        for (var i = 0; i < previous.lines.length; i++) have[previous.lines[i]] = true;
        var missed = st.lines.filter(function (l) { return !have[l]; });
        if (missed.length) previous.lines = previous.lines.concat(missed);
        previous.stage = st.lines[st.lines.length - 1];
        sset(KEY_PREV, JSON.stringify(previous));
      }
    } catch (e) { /* none */ }
    // Its last sign of life (the early log's pulse), kept with it.
    if (previous && !previous.pulse) {
      try {
        var q = JSON.parse(sget(KEY_PULSE) || "null");
        if (q && q.started === previous.started) { previous.pulse = q; sset(KEY_PREV, JSON.stringify(previous)); }
      } catch (e) { /* none */ }
    }
  })();
  var saveTimer = 0, ended = "";
  function dirty() {
    if (!saveTimer && storage) saveTimer = setTimeout(function () { saveTimer = 0; save(); }, 1500);
  }
  function save(end) {
    if (!storage) return;
    if (end) ended = end;
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = 0; }
    var keep = lines;
    var chars = 0, from = keep.length;
    while (from > 0 && chars + keep[from - 1].length < STORE_MAX) { from--; chars += keep[from].length + 4; }
    keep = keep.slice(from);
    for (var tries = 0; tries < 3; tries++) {
      var rec = JSON.stringify({v: 1, started: started, page: location.pathname, query: query(),
        ended: ended, dropped: dropped + (lines.length - keep.length), lines: keep});
      if (sset(KEY_RUN, rec)) return;
      keep = keep.slice(Math.floor(keep.length / 2));
    }
  }

  // The environment header (built when it is shown or copied; the GPU
  // facts are asked once).
  var gpuFacts = null;
  function webgl2Facts() {
    try {
      var c = document.createElement("canvas");
      var gl = c.getContext("webgl2");
      if (!gl) return "no";
      var dbg = gl.getExtension("WEBGL_debug_renderer_info");
      var r = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
      var v = dbg ? gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR);
      var size = gl.getParameter(gl.MAX_TEXTURE_SIZE);
      var lose = gl.getExtension("WEBGL_lose_context");
      if (lose) lose.loseContext();
      return "yes – " + v + " / " + r + ", max texture " + size;
    } catch (e) { return "no (" + e + ")"; }
  }
  function webgpuFacts() {
    if (!navigator.gpu) return Promise.resolve("no (navigator.gpu missing)");
    var ask = navigator.gpu.requestAdapter().then(function (a) {
      if (!a) return "navigator.gpu yes, but no adapter";
      var info = a.info ? Promise.resolve(a.info) : a.requestAdapterInfo ? a.requestAdapterInfo() : Promise.resolve(null);
      return info.then(function (i) {
        var parts = i ? [i.vendor, i.architecture, i.device, i.description].filter(function (x) { return x; }) : [];
        var lim = a.limits ? ", max texture " + a.limits.maxTextureDimension2D + ", max buffer " + mb(a.limits.maxBufferSize) : "";
        return "yes – adapter " + (parts.join(" / ") || "(no info)") + (a.isFallbackAdapter ? " (fallback adapter)" : "") + lim;
      });
    }).catch(function (e) { return "navigator.gpu yes, requestAdapter failed: " + e; });
    var late = new Promise(function (ok) { setTimeout(function () { ok("navigator.gpu yes, requestAdapter did not answer in 4 s"); }, 4000); });
    return Promise.race([ask, late]);
  }
  function simd() {
    try {
      if (window.ocwSimd !== undefined) return window.ocwSimd ? "yes" : "no";
      if (window.ocwGpuChoice) return window.ocwGpuChoice.simd(window.WebAssembly) ? "yes" : "no";
    } catch (e) { /* below */ }
    return "?";
  }
  function header() {
    if (!gpuFacts) gpuFacts = webgpuFacts().then(function (w) { return {webgpu: w, webgl2: webgl2Facts()}; });
    return gpuFacts.then(function (g) {
      var cfg = {};
      try { cfg = JSON.parse(window.ocwConfig || "{}") || {}; } catch (e) { cfg = {}; }
      var buildLine = "";
      for (var i = 0; i < lines.length && !buildLine; i++) {
        var m = /Open Cycling World (.+?) starting/.exec(lines[i]);
        if (m) buildLine = m[1];
      }
      var gpu = window.ocwGpu;
      var ratio = window.ocwPixelRatio;
      var pm = window.performance && performance.memory;
      var wasm = 0;
      try { if (window.ocw && window.ocw.wasmMemory) wasm = window.ocw.wasmMemory(); } catch (e) { wasm = 0; }
      var uad = navigator.userAgentData;
      var ctx = "";
      try { if (window.ocwGpuContext) ctx = window.ocwGpuContext(); } catch (e) { ctx = ""; }
      var rows = [
        ["page log", "Open Cycling World – started " + started + ", now " + stamp(now()).trim() + " s"],
        ["page build", (cfg.build || "(config.json has no build)") + (cfg.channel ? ", channel " + cfg.channel : "")],
        ["app build", buildLine || "(the app has not started)"],
        ["address", location.origin + location.pathname + " " + query()],
        ["user agent", navigator.userAgent],
        ["platform", (navigator.platform || "?") + (uad ? ", " + (uad.platform || "?") + (uad.mobile ? " (mobile)" : "") : "")
          + ", touch points " + (navigator.maxTouchPoints || 0) + ", language " + (navigator.language || "?")],
        ["screen", screen.width + " × " + screen.height + " CSS px, window " + innerWidth + " × " + innerHeight
          + ", pixel ratio " + (ratio ? ratio.device + " (drawn at " + ratio.used + ")" : window.devicePixelRatio)
          + (screen.orientation ? ", " + screen.orientation.type : "")
          + (navigator.standalone || (window.matchMedia && matchMedia("(display-mode: standalone), (display-mode: fullscreen)").matches)
            ? ", from the home screen" : "")],
        ["memory", "device " + (navigator.deviceMemory ? navigator.deviceMemory + " GB" : "?") + ", cores " + (navigator.hardwareConcurrency || "?")
          + (pm ? ", JS heap " + mb(pm.usedJSHeapSize) + " of " + mb(pm.jsHeapSizeLimit) : "")
          + (wasm ? ", wasm " + mb(wasm) : "")],
        ["WebAssembly", (typeof WebAssembly === "object" ? "yes" : "no") + ", SIMD " + simd()
          + ", threads " + (typeof SharedArrayBuffer === "function" && window.crossOriginIsolated ? "yes" : "no (not cross-origin isolated)")],
        ["WebGPU", g.webgpu],
        ["WebGL 2", g.webgl2],
        ["backend", gpu ? (gpu.api || "none") + " – " + gpu.reason : "(not chosen yet)"],
        ["GPU failure", window.ocwGpuFailure ? String(window.ocwGpuFailure) : "none"],
        ["GPU context", ctx || "-"],
        ["state", "frames " + (window.ocwFrames || 0) + ", " + (document.documentElement.classList.contains("ocw-riding") ? "riding" : "not riding")
          + ", " + (navigator.onLine === false ? "offline" : "online") + ", Web Bluetooth " + (navigator.bluetooth ? "yes" : "no")],
        ["lines", lines.length + " this load" + (dropped ? " (" + dropped + " older ones dropped)" : "")
          + (previous ? ", " + previous.lines.length + " of the load before" : "")
          + "; levels: E error, W warning, I info, L log, D debug, ! uncaught / load failure, P page; rs = the app (Rust)"],
      ];
      return rows.map(function (r) { return r[0] + ": " + scrub(r[1]); }).join("\n");
    });
  }
  function previousTitle() {
    return "--- the page load before: started " + previous.started + ", " + previous.lines.length + " lines"
      + (previous.dropped ? " (" + previous.dropped + " older ones dropped)" : "") + ", "
      + (previous.ended || "ended without pagehide – the browser closed or killed the page (memory?), or it crashed")
      + (previous.pulse ? "; last sign of life at +" + (previous.pulse.t / 1000).toFixed(1) + " s (wasm " + mb(previous.pulse.wasm)
        + (previous.pulse.js ? ", JS heap " + mb(previous.pulse.js) : "")
        + (previous.pulse.gpu ? ", gpu renderer " + mb(previous.pulse.gpu) : "") + ", frames " + previous.pulse.frames + ")" : "")
      + (previous.stage ? "; last world load stage: " + previous.stage.replace(/^.*?world load: /, "") : "") + " ---";
  }
  function report() {
    return header().then(function (h) {
      var out = [h, ""];
      if (previous) out.push(previousTitle(), previous.lines.join("\n"), "");
      out.push("--- this page load (" + started + ") ---", lines.join("\n"));
      return out.join("\n") + "\n";
    });
  }

  // The panel: built when it is first shown.
  var panel = null, body = null, note = null, offerBtn = null, shown = false, offered = "", full = false, pending = [];
  var CSS = "#ocw-log{position:fixed;z-index:60;left:calc(6px + var(--safe-left,0px));right:calc(6px + var(--safe-right,0px));"
    + "top:calc(6px + var(--safe-top,0px));height:min(58vh,calc(100% - 12px - var(--safe-top,0px) - var(--safe-bottom,0px)));"
    + "display:flex;flex-direction:column;box-sizing:border-box;background:rgba(10,13,18,.93);color:#e5e9f0;"
    + "border:1px solid rgba(136,192,208,.55);border-radius:8px;font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;"
    + "-webkit-user-select:text;user-select:text;touch-action:auto;-webkit-text-size-adjust:100%}"
    + "#ocw-log.full{height:calc(100% - 12px - var(--safe-top,0px) - var(--safe-bottom,0px))}"
    + "#ocw-log[hidden],#ocw-log-offer[hidden]{display:none}"
    + "#ocw-log .bar{display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:6px;border-bottom:1px solid rgba(136,192,208,.3)}"
    + "#ocw-log .bar b{font:600 14px system-ui,sans-serif;margin-right:auto;padding-left:4px}"
    + "@media (max-width:430px){#ocw-log .bar b{display:none}#ocw-log .bar{justify-content:space-between;gap:4px}}"
    + "#ocw-log button,#ocw-log-offer{font:600 14px system-ui,sans-serif;min-height:40px;min-width:40px;padding:0 10px;"
    + "border-radius:6px;border:0;background:#88c0d0;color:#10141a;cursor:pointer}"
    + "#ocw-log button.plain{background:rgba(34,42,53,.9);color:#e5e9f0;border:1px solid rgba(98,112,132,.8)}"
    + "#ocw-log .note{padding:0 10px;color:#a3d9a5;font:13px system-ui,sans-serif;min-height:0}"
    + "#ocw-log .note.err{color:#ebcb8b}"
    + "#ocw-log .body{flex:1;overflow:auto;-webkit-overflow-scrolling:touch;overscroll-behavior:contain;margin:0;padding:6px 10px 10px;"
    + "white-space:pre-wrap;word-break:break-word}"
    + "#ocw-log .h{color:#c3cbd8}#ocw-log .sec{color:#88c0d0;margin-top:8px}"
    + "#ocw-log .E,#ocw-log .x{color:#ffb4a8}#ocw-log .W{color:#ebcb8b}#ocw-log .D{color:#8f9bb0}#ocw-log .P{color:#a3be8c}"
    + "#ocw-log .prev{opacity:.8}"
    + "#ocw-log textarea{flex:1;margin:6px;font:inherit;background:#10141a;color:#e5e9f0;border:1px solid #4c566a;border-radius:6px}"
    + "#ocw-log-offer{position:fixed;z-index:59;left:50%;transform:translateX(-50%);bottom:calc(18px + var(--safe-bottom,0px));"
    + "pointer-events:auto;box-shadow:0 2px 8px rgba(0,0,0,.5)}";
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  }
  function style() {
    if (document.getElementById("ocw-log-css")) return;
    var s = el("style");
    s.id = "ocw-log-css";
    s.textContent = CSS;
    (document.head || document.documentElement).appendChild(s);
  }
  function lineEl(text, prev) {
    var m = /^\+\s*[\d.]+ (\S)/.exec(text);
    var lv = m ? m[1] : "L";
    return el("div", (lv === "!" ? "x" : lv) + (prev ? " prev" : ""), text);
  }
  function say(text, err) {
    if (!note) return;
    note.textContent = text;
    note.className = "note" + (err ? " err" : "");
    var said = text;
    setTimeout(function () { if (note.textContent === said) note.textContent = ""; }, 6000);
  }
  function fileName() {
    return "ocw-log-" + new Date().toISOString().replace(/[:T]/g, "-").slice(0, 19) + ".txt";
  }
  function copy() {
    report().then(function (text) {
      var done = function () { say("Copied – paste it into a message or an e-mail."); };
      var fallback = function () {
        // The text selected in a box: the phone's own Copy, or execCommand.
        var ta = panel.querySelector("textarea") || el("textarea");
        ta.value = text;
        ta.setAttribute("readonly", "");
        if (!ta.parentNode) panel.insertBefore(ta, body);
        ta.hidden = false;
        ta.focus();
        ta.select();
        try { ta.setSelectionRange(0, text.length); } catch (e) { /* select() did it */ }
        var ok = false;
        try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
        if (ok) done(); else say("The text is selected below: press and hold it, then Copy.", true);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, fallback);
      } else fallback();
    });
  }
  function share() {
    report().then(function (text) {
      if (!navigator.share) { say("This browser cannot share – use Copy or Download.", true); return; }
      var data = {title: "Open Cycling World log", text: text};
      try {
        var f = new File([text], fileName(), {type: "text/plain"});
        if (navigator.canShare && navigator.canShare({files: [f]})) data = {title: "Open Cycling World log", files: [f]};
      } catch (e) { /* the text then */ }
      navigator.share(data).then(function () { say("Shared."); }, function (e) {
        if (e && e.name === "AbortError") return;
        say("Sharing failed (" + (e && e.name || e) + ") – use Copy or Download.", true);
      });
    });
  }
  function download() {
    report().then(function (text) {
      var url = URL.createObjectURL(new Blob([text], {type: "text/plain"}));
      var a = el("a");
      a.href = url;
      a.download = fileName();
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(function () { URL.revokeObjectURL(url); }, 60000);
      say("Downloaded " + a.download + ".");
    });
  }
  function render() {
    body.textContent = "";
    var h = el("div", "h", "(reading the browser's facts …)");
    body.appendChild(h);
    headerEl = h;
    header().then(function (t) { h.textContent = t; });
    var frag = document.createDocumentFragment();
    if (previous) {
      frag.appendChild(el("div", "sec", previousTitle()));
      for (var i = 0; i < previous.lines.length; i++) frag.appendChild(lineEl(previous.lines[i], true));
    }
    frag.appendChild(el("div", "sec", "--- this page load (" + started + ") ---"));
    for (var j = 0; j < lines.length; j++) frag.appendChild(lineEl(lines[j], false));
    body.appendChild(frag);
    body.scrollTop = body.scrollHeight;
  }
  var flushing = false;
  function flush() {
    flushing = false;
    if (!shown || !pending.length) { pending = []; return; }
    var atEnd = body.scrollTop + body.clientHeight >= body.scrollHeight - 30;
    var frag = document.createDocumentFragment();
    for (var i = 0; i < pending.length; i++) frag.appendChild(lineEl(pending[i], false));
    pending = [];
    body.appendChild(frag);
    var extra = body.childNodes.length - (LIMIT + (previous ? previous.lines.length : 0) + 60);
    // The oldest lines of this load leave the panel as they leave the buffer.
    while (extra-- > 0) {
      var n = body.querySelector("div:not(.h):not(.sec):not(.prev)");
      if (!n) break;
      n.remove();
    }
    if (atEnd) body.scrollTop = body.scrollHeight;
  }
  // The header follows the page while the panel is open (config.json,
  // the app's build and backend come after the first lines).
  var headerEl = null;
  setInterval(function () {
    if (!shown || !headerEl) return;
    var h = headerEl;
    header().then(function (t) { if (h.textContent !== t) h.textContent = t; });
  }, 3000);
  function build() {
    style();
    panel = el("section");
    panel.id = "ocw-log";
    panel.setAttribute("aria-label", "Log");
    var bar = el("div", "bar");
    bar.appendChild(el("b", "", "Log"));
    var buttons = [["Copy", copy, ""], ["Share", share, ""], ["Download", download, "plain"],
      ["Clear", function () { clear(); say("Cleared."); }, "plain"],
      ["↕", function () { full = !full; panel.className = full ? "full" : ""; }, "plain"],
      ["Hide", hide, "plain"]];
    for (var i = 0; i < buttons.length; i++) {
      var b = el("button", buttons[i][2], buttons[i][0]);
      b.type = "button";
      b.setAttribute("data-log", buttons[i][0] === "↕" ? "size" : buttons[i][0].toLowerCase());
      if (buttons[i][0] === "Share" && !navigator.share) b.hidden = true;
      b.onclick = buttons[i][1];
      bar.appendChild(b);
    }
    panel.appendChild(bar);
    note = el("div", "note");
    panel.appendChild(note);
    body = el("div", "body");
    panel.appendChild(body);
    // Touches in the panel stay in the panel (the ride's steering, the
    // page's blocked gestures).
    ["pointerdown", "touchstart", "touchmove", "wheel", "keydown"].forEach(function (t) {
      panel.addEventListener(t, function (e) { e.stopPropagation(); }, {passive: true});
    });
    document.body.appendChild(panel);
    listeners.push(function (line) {
      if (!shown) return;
      pending.push(line);
      if (!flushing) { flushing = true; requestAnimationFrame(flush); }
    });
  }
  function show() {
    if (!document.body) { document.addEventListener("DOMContentLoaded", show); return; }
    if (!panel) build();
    shown = true;
    sset(KEY_SHOW, "1");
    panel.hidden = false;
    if (offerBtn) offerBtn.hidden = true;
    render();
  }
  function hide() {
    shown = false;
    sdel(KEY_SHOW);
    if (panel) panel.hidden = true;
    if (offerBtn && offered) offerBtn.hidden = false;
    var c = document.getElementById("ocw-canvas");
    if (c) c.focus();
  }
  function clear() {
    lines = [];
    dropped = 0;
    previous = null;
    sdel(KEY_PREV);
    sdel(KEY_STAGES);
    stages = [];
    save();
    if (shown) render();
  }
  // "Show log" on an error or a stuck loading screen.
  function offer(why) {
    if (!document.body) { document.addEventListener("DOMContentLoaded", function () { offer(why); }); return; }
    if (offered) return;
    offered = String(why || "offered");
    add("page", "\"Show log\" offered: " + offered);
    style();
    offerBtn = el("button", "", "Show log");
    offerBtn.id = "ocw-log-offer";
    offerBtn.type = "button";
    offerBtn.onclick = show;
    offerBtn.hidden = shown;
    document.body.appendChild(offerBtn);
  }

  // Five quick taps on the version stamp (the app draws it 11 px high in
  // the bottom right corner) or on the start page's build line.
  var taps = [];
  addEventListener("pointerdown", function (e) {
    var onStamp = e.clientX >= innerWidth - 170 && e.clientY >= innerHeight - 34;
    var onBuild = e.target && e.target.closest && e.target.closest("#ocw-start-build");
    if (!onStamp && !onBuild) { taps = []; return; }
    var t = now();
    taps.push(t);
    while (taps.length && t - taps[0] > 3000) taps.shift();
    if (taps.length >= 5) {
      taps = [];
      add("page", "five taps on the version stamp");
      if (shown) hide(); else show();
    }
  }, true);

  function watchStatus() {
    var status = document.getElementById("ocw-status");
    if (!status) return;
    var check = function () {
      if (!status.hidden && /\berror\b/.test(status.className)) {
        offer("the page shows an error: " + status.textContent.replace(/\s+/g, " ").trim().slice(0, 200));
      }
    };
    if (window.MutationObserver) {
      new MutationObserver(check).observe(status, {attributes: true, attributeFilter: ["class", "hidden"], childList: true});
    }
    check();
    // A loading screen that stays (the module never starts, a world that
    // does not come): after 45 s.
    var since = 0;
    setInterval(function () {
      if (status.hidden || /\berror\b/.test(status.className)) { since = 0; return; }
      if (!since) since = now();
      if (now() - since > 45000) offer("still loading after 45 s: " + status.textContent.replace(/\s+/g, " ").trim().slice(0, 200));
    }, 5000);
  }
  var wanted = false;
  try { wanted = /^(1|true|on|yes)$/.test(new URLSearchParams(location.search).get("log") || ""); } catch (e) { wanted = false; }
  var ready = function () {
    watchStatus();
    if (wanted || sget(KEY_SHOW)) show();
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", ready); else ready();

  add("page", "page log started (" + location.pathname + " " + query() + ")" + (previous ? "; the load before is kept" : ""));
  window.ocwLog = {
    lines: function () { return lines.slice(); },
    previous: function () { return previous ? {started: previous.started, ended: previous.ended, stage: previous.stage || null, lines: previous.lines.slice()} : null; },
    stages: function () { return stages.slice(); },
    report: report,
    header: header,
    show: show,
    hide: hide,
    offer: offer,
    clear: clear,
    scrub: scrub,
    add: function (text) { add("log", text); },
    state: function () {
      return JSON.stringify({lines: lines.length, dropped: dropped, shown: shown, built: !!panel, offered: offered,
        previous: previous ? previous.lines.length : 0, previous_ended: previous ? previous.ended : null,
        previous_pulse: previous && previous.pulse ? previous.pulse : null, early_ms: EARLY_MS,
        rendered: body ? body.childNodes.length : 0, share: !!navigator.share, clipboard: !!(navigator.clipboard && navigator.clipboard.writeText)});
    },
  };
})();
