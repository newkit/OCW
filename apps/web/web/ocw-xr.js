// WebXR for the browser app (spec §14 Phase 14, 14I W1): the immersive VR
// session in the headset's browser (the Meta Quest Browser on the Quest 1),
// only in the WebXR build (`?xr=1`: the module ocw_web_webxr.js, ocw-web's
// features `webgl2,webxr`).
//
//   ocwXr.install()        before the module starts: the app's WebGL 2
//                          context is made XR-compatible, and the page's
//                          requestAnimationFrame is wrapped (see below)
//   ocwXr.offer(ocw)       after it started: asks the browser
//                          (isSessionSupported('immersive-vr')) and shows the
//                          Enter VR button, or a note why there is none
//   ocwXr.enter() / exit() the session (enter must run in a user gesture:
//                          the button's click)
//   ocwXr.state()          {state, space, frames, copy, error, …} (tests)
//   ocwXr.readback(w)      the test copy's pixels (copy=fbo, below)
//   ocwXr.pair(code)       the sensor bridge (14I Q7 (b), 10D.2): the code the
//                          desktop app shows, typed into the page's panel
//                          before Enter VR (or ?bridge=CODE in the link)
//   ocwXr.report(reason)   the beacon (14I W3 / W4): the session's numbers to
//                          the desktop over the bridge (every 10 s in VR and
//                          at the end; the desktop keeps them in
//                          <data dir>/ocw/reports/xr.jsonl)
//
// The reference space (14I Q8 (a)): local-floor, so the real floor is the
// road; `local` when the browser has no floor (the app puts the head at the
// start at the rider's seated eye point). The controller's squeeze, or the
// headset's own recentring (the space's reset event), recentres.
//
// How the app is stepped: Bevy's loop (winit) draws in the window's
// requestAnimationFrame, but a session's frames – the only ones whose
// pictures reach the headset, and on a standalone headset the only frames
// the page gets at all while immersive – come from the session's own
// requestAnimationFrame with the eyes' poses. So while a session runs, the
// window's requestAnimationFrame callbacks (winit's redraw: Bevy's update
// and rendering) are kept and run inside each XR frame, after the frame's
// poses were handed to the app (window.ocw.xrFrame). Bevy then draws both
// eyes side by side into the canvas (WebGL 2 through wgpu), and this file
// copies the canvas into the layer's framebuffer (an opaque framebuffer
// that wgpu cannot draw into) with blitFramebuffer on the same context –
// the canvas's parts are the layer's viewports, so one scaled copy puts each
// eye in place. When the layer's framebuffer is null (the emulator: the
// canvas is the layer) there is nothing to copy; `?xrcopy=fbo` copies into
// a framebuffer of our own instead, to test the copy (readback()).
//
// The Immersive Web Emulation Runtime (iwer, the test's emulator) calls
// window.requestAnimationFrame for its own device frames; the test gives it
// the display's clock (window.ocwDisplayRaf), as a headset's compositor runs
// on its own clock and never on the page's.
(function (root) {
  "use strict";
  const doc = root.document;
  const realRaf = root.requestAnimationFrame.bind(root);
  const realCaf = root.cancelAnimationFrame.bind(root);
  // Frame data layout (apps/web/src/xr.rs: FRAME_LAYOUT, HEADER_LEN, VIEW_LEN).
  const LAYOUT = 1, HEADER = 13, VIEW = 28;
  const query = new URLSearchParams(root.location.search);
  const st = {
    installed: false, app: null, state: "unknown", why: "",
    session: null, gl: null, layer: null, space: null, spaceType: "",
    // Window rAF callbacks waiting for a real frame (id → callback) and,
    // during a session, for the next XR frame.
    pending: new Map(), queued: new Map(), nextId: 1,
    t0: 0, lastWork: 0, frames: 0, copy: "", copies: 0, error: "",
    testFbo: null, workMs: [],
    beacon: {sent: 0, last: "", error: "", timer: null}, pairTimer: null,
  };
  // How often the beacon reports during a session (ms).
  const BEACON_MS = 10000;

  // The page's requestAnimationFrame, wrapped from the start so that the
  // callback winit is waiting for can be taken over when a session starts.
  function wrapRaf() {
    root.requestAnimationFrame = (cb) => {
      const id = st.nextId++;
      if (st.session) {
        st.queued.set(id, cb);
        return id;
      }
      const real = realRaf((t) => { st.pending.delete(id); cb(t); });
      st.pending.set(id, {cb, real});
      return id;
    };
    root.cancelAnimationFrame = (id) => {
      const p = st.pending.get(id);
      if (p) { realCaf(p.real); st.pending.delete(id); }
      st.queued.delete(id);
    };
  }

  // Session start: the window's waiting callbacks move to the XR frames.
  function takeOverFrames() {
    for (const [id, p] of st.pending) { realCaf(p.real); st.queued.set(id, p.cb); }
    st.pending.clear();
  }

  // Session end: whatever waits goes back to the window's frames.
  function giveBackFrames() {
    const q = st.queued;
    st.queued = new Map();
    for (const [id, cb] of q) {
      const real = realRaf((t) => { st.pending.delete(id); cb(t); });
      st.pending.set(id, {cb, real});
    }
  }

  function install() {
    if (st.installed) return;
    st.installed = true;
    // The app's context must be XR-compatible for an XRWebGLLayer; asking
    // when it is created avoids makeXRCompatible() (which may lose the
    // context on a machine with two GPUs).
    const proto = root.HTMLCanvasElement && root.HTMLCanvasElement.prototype;
    if (proto && proto.getContext) {
      const get = proto.getContext;
      proto.getContext = function (type, attrs) {
        if (type === "webgl2" || type === "webgl") attrs = Object.assign({}, attrs || {}, {xrCompatible: true});
        return get.call(this, type, attrs);
      };
    }
    wrapRaf();
  }

  function event(name, detail) {
    st.state = name === "enter" ? "entering" : name === "started" ? "running"
      : name === "ended" ? "ended" : name === "failed" ? "offered"
      : name === "supported" ? (detail === "true" ? "offered" : "unsupported") : st.state;
    try { if (st.app && st.app.xrEvent) st.app.xrEvent(name, detail || ""); } catch (e) { console.error("ocw-xr: " + e); }
  }

  function ui() {
    let b = doc.getElementById("ocw-xr-enter");
    if (!b) {
      b = doc.createElement("button");
      b.id = "ocw-xr-enter";
      b.hidden = true;
      b.textContent = "Enter VR";
      b.onclick = () => (st.session ? exit() : enter());
      doc.body.appendChild(b);
    }
    let n = doc.getElementById("ocw-xr-note");
    if (!n) {
      n = doc.createElement("div");
      n.id = "ocw-xr-note";
      n.hidden = true;
      doc.body.appendChild(n);
    }
    return {button: b, note: n, pair: pairPanel()};
  }

  // The sensor bridge's panel (14I Q7 (b)): pair with the desktop app before
  // entering VR – in the headset the page cannot be reached.
  function pairPanel() {
    let p = doc.getElementById("ocw-xr-pair");
    if (p) return p;
    p = doc.createElement("form");
    p.id = "ocw-xr-pair";
    p.hidden = true;
    p.innerHTML =
      '<label for="ocw-xr-code">Trainer from the desktop app</label>' +
      '<div class="row"><input id="ocw-xr-code" autocomplete="off" autocapitalize="characters" spellcheck="false"' +
      ' placeholder="code, e.g. 7K2M-Q9XP-4DA" maxlength="20"><button type="submit">Pair</button></div>' +
      '<div id="ocw-xr-pair-state">Desktop: Settings → Trainer → Devices → Share with a browser shows the code.</div>';
    p.onsubmit = (e) => { e.preventDefault(); pair(doc.getElementById("ocw-xr-code").value); };
    doc.body.appendChild(p);
    return p;
  }

  function bridgeInfo() {
    try { return JSON.parse((st.app && st.app.devices && st.app.devices()) || "null"); } catch (e) { return null; }
  }

  // The panel's line: the bridge's state and the power that drives the ride.
  function pairState() {
    const j = bridgeInfo();
    const b = j && j.bridge;
    const line = doc.getElementById("ocw-xr-pair-state");
    if (!line || !b) return b;
    if (b.supported === false) {
      line.textContent = "This browser has no WebTransport: the sensor bridge cannot work here (virtual power instead).";
      line.className = "error";
    } else if (b.code) {
      const w = j.rider_power_w != null ? ` · ${Math.round(j.rider_power_w)} W` : "";
      line.textContent = `Bridge ${b.code}: ${b.state || "…"}${b.paired ? w : ""}`;
      line.className = b.paired ? "ok" : "";
    }
    return b;
  }

  function pair(code) {
    code = String(code || "").trim();
    if (!code || !st.app) return false;
    st.app.console("bridge.pair " + code);
    console.log("ocw-xr: pairing with the desktop app");
    pairState();
    return true;
  }

  // The beacon: what the headset measured, to the desktop over the bridge.
  function report(reason) {
    if (!st.app || !st.app.xrReport) return "no app";
    let xr = null;
    try { xr = JSON.parse(st.app.xr() || "null"); } catch (e) { xr = null; }
    const b = bridgeInfo();
    const data = {
      reason, page_s: Math.round(performance.now() / 100) / 10,
      xr, page: state(),
      bridge: b && b.bridge ? {state: b.bridge.state, rtt_ms: b.bridge.rtt_ms, readings: b.bridge.readings,
        dropped: b.bridge.dropped} : null,
      power_w: b ? b.rider_power_w : null, distance_m: b ? b.distance_m : null,
    };
    const why = st.app.xrReport(JSON.stringify(data));
    st.beacon.last = reason;
    if (why) st.beacon.error = why; else st.beacon.sent++;
    return why;
  }

  function note(text, error) {
    const {note: n} = ui();
    n.textContent = text;
    n.className = error ? "error" : "";
    n.hidden = !text;
  }

  // Why this browser cannot enter VR (null: it can be asked).
  function blocker() {
    if (!root.isSecureContext) return "VR needs a secure page (HTTPS or localhost) – this one is not, so the browser hides WebXR.";
    if (!root.navigator.xr) return "This browser has no WebXR: riding in the flat view. VR works in a headset's browser (Meta Quest Browser) or a browser with a VR headset attached.";
    if (!root.XRWebGLLayer) return "This browser's WebXR has no WebGL layer (XRWebGLLayer): riding in the flat view.";
    return null;
  }

  async function offer(app) {
    st.app = app;
    const {button} = ui();
    const why = blocker();
    let ok = false;
    if (!why) {
      try { ok = await root.navigator.xr.isSessionSupported("immersive-vr"); } catch (e) { st.why = String(e); }
    }
    st.why = why || (ok ? "" : st.why || "No VR headset found by this browser (immersive-vr not supported): riding in the flat view.");
    event("supported", ok ? "true" : "false");
    button.hidden = !ok;
    // The pairing panel: with a headset (and in the test's emulator).
    const panel = pairPanel();
    panel.hidden = !ok;
    if (!st.pairTimer) st.pairTimer = setInterval(pairState, 1000);
    pairState();
    note(ok ? "" : st.why, false);
    if (!ok) setTimeout(() => note("", false), 15000);
    console.log(`ocw-xr: immersive-vr ${ok ? "supported" : "not supported"}${ok ? "" : " – " + st.why}`);
    return ok;
  }

  async function enter() {
    if (st.session || st.state === "entering") return false;
    event("enter", "");
    const {button} = ui();
    button.disabled = true;
    try {
      // First, while the click's user activation lasts.
      const session = await root.navigator.xr.requestSession("immersive-vr", {optionalFeatures: ["local-floor"]});
      const canvas = doc.getElementById("ocw-canvas");
      const gl = canvas.getContext("webgl2");
      if (!gl) throw new Error("the app's canvas has no WebGL 2 context");
      if (gl.makeXRCompatible) {
        try { await gl.makeXRCompatible(); } catch (e) { console.warn("ocw-xr: makeXRCompatible: " + e); }
      }
      // No MSAA and no depth in the layer: the app draws with its own and
      // the layer only receives the copied picture (14I W3 sets the scale
      // and the foveation for the Quest 1).
      const scale = Number(query.get("xrscale")) || 1.0;
      const layer = new root.XRWebGLLayer(session, gl, {antialias: false, depth: false, stencil: false, alpha: false, framebufferScaleFactor: scale});
      if ("fixedFoveation" in layer && query.has("xrfoveation")) layer.fixedFoveation = Number(query.get("xrfoveation"));
      session.updateRenderState({baseLayer: layer, depthNear: 0.05, depthFar: 20000});
      let space, type = "local-floor";
      try { space = await session.requestReferenceSpace("local-floor"); } catch (e) {
        type = "local";
        space = await session.requestReferenceSpace("local");
      }
      Object.assign(st, {session, gl, layer, space, spaceType: type, t0: performance.now(), frames: 0, copies: 0, workMs: [], error: ""});
      // The facts W4 logs in the headset (spec 14A, W0's open points).
      const exts = gl.getSupportedExtensions() || [];
      st.device = {
        userAgent: root.navigator.userAgent,
        nativeScale: root.XRWebGLLayer.getNativeFramebufferScaleFactor ? root.XRWebGLLayer.getNativeFramebufferScaleFactor(session) : null,
        framebuffer: [layer.framebufferWidth, layer.framebufferHeight],
        frameRate: session.frameRate || null,
        supportedFrameRates: session.supportedFrameRates ? Array.from(session.supportedFrameRates) : null,
        multiview: exts.filter((e) => /multiview/i.test(e)),
        foveation: "fixedFoveation" in layer ? layer.fixedFoveation : null,
      };
      console.log("ocw-xr: device " + JSON.stringify(st.device));
      session.addEventListener("end", ended);
      session.addEventListener("visibilitychange", () => event("visibility", session.visibilityState));
      // Recentre (14I Q8): the controller's squeeze, or the headset's own
      // recentring (the reference space is reset).
      session.addEventListener("squeeze", () => event("recentre", "squeeze"));
      if (space.addEventListener) space.addEventListener("reset", () => event("recentre", "reset"));
      pairPanel().hidden = true;
      st.beacon.timer = setInterval(() => report("periodic"), BEACON_MS);
      takeOverFrames();
      session.requestAnimationFrame(frame);
      event("started", type);
      button.textContent = "Exit VR";
      console.log(`ocw-xr: session running (${type}, framebuffer ${layer.framebufferWidth}×${layer.framebufferHeight}, ` +
        `${layer.framebuffer ? "opaque framebuffer: copied" : "the canvas is the layer"}` +
        `${session.frameRate ? `, ${session.frameRate} Hz` : ""})`);
      return true;
    } catch (e) {
      st.error = String(e);
      console.warn("ocw-xr: entering VR failed: " + e);
      event("failed", String(e));
      note("Entering VR failed: " + e, true);
      return false;
    } finally {
      button.disabled = false;
    }
  }

  async function exit() {
    if (st.session) await st.session.end().catch((e) => console.warn("ocw-xr: end: " + e));
  }

  function ended() {
    if (st.beacon.timer) { clearInterval(st.beacon.timer); st.beacon.timer = null; }
    report("end");
    pairPanel().hidden = false;
    st.session = null;
    st.layer = null;
    st.space = null;
    giveBackFrames();
    event("ended", "");
    const {button} = ui();
    button.textContent = "Enter VR";
    button.hidden = false;
    console.log(`ocw-xr: session ended after ${st.frames} frames; the flat view again`);
  }

  // One XR frame: the poses to the app, the app's waiting frame callbacks
  // (Bevy's update and rendering), the copy into the layer.
  function frame(t, xrFrame) {
    const session = st.session;
    if (!session) return;
    session.requestAnimationFrame(frame);
    const w0 = performance.now();
    const layer = session.renderState.baseLayer || st.layer;
    const pose = xrFrame.getViewerPose(st.space);
    const views = pose ? pose.views : [];
    const d = new Float32Array(HEADER + VIEW * views.length);
    d[0] = LAYOUT;
    d[1] = views.length;
    d[2] = layer.framebufferWidth;
    d[3] = layer.framebufferHeight;
    d[4] = t - st.t0;
    d[5] = st.lastWork;
    const put = (o, tr) => {
      d[o] = tr.position.x; d[o + 1] = tr.position.y; d[o + 2] = tr.position.z;
      d[o + 3] = tr.orientation.x; d[o + 4] = tr.orientation.y; d[o + 5] = tr.orientation.z; d[o + 6] = tr.orientation.w;
    };
    if (pose) put(6, pose.transform); else d[12] = 1;
    views.forEach((v, i) => {
      const o = HEADER + VIEW * i;
      d[o] = v.eye === "left" ? 1 : v.eye === "right" ? 2 : 0;
      const vp = layer.getViewport(v);
      d[o + 1] = vp.x; d[o + 2] = vp.y; d[o + 3] = vp.width; d[o + 4] = vp.height;
      d.set(v.projectionMatrix, o + 5);
      put(o + 21, v.transform);
    });
    try { st.app.xrFrame(d); } catch (e) { st.error = "xrFrame: " + e; }
    const q = st.queued;
    st.queued = new Map();
    for (const cb of q.values()) {
      try { cb(t); } catch (e) {
        if (!String(e).includes("Using exceptions for control flow")) { st.error = String(e); console.error("ocw-xr: " + e); }
      }
    }
    copy(layer);
    st.frames++;
    st.lastWork = performance.now() - w0;
    st.workMs.push(st.lastWork);
    if (st.workMs.length > 600) st.workMs.shift();
  }

  // The canvas (both eyes) into the layer's framebuffer, scaled.
  function copy(layer) {
    const gl = st.gl;
    let dst = layer.framebuffer;
    const w = layer.framebufferWidth, h = layer.framebufferHeight;
    if (!dst) {
      if (query.get("xrcopy") !== "fbo") { st.copy = "direct"; return; }
      dst = testFbo(w, h);
      st.copy = "fbo";
    } else {
      st.copy = "layer";
    }
    const scissor = gl.isEnabled(gl.SCISSOR_TEST);
    if (scissor) gl.disable(gl.SCISSOR_TEST);
    gl.colorMask(true, true, true, true);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, dst);
    gl.blitFramebuffer(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight, 0, 0, w, h, gl.COLOR_BUFFER_BIT, gl.LINEAR);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    if (scissor) gl.enable(gl.SCISSOR_TEST);
    const err = gl.getError();
    if (err && !st.error) st.error = `copy: GL error 0x${err.toString(16)}`;
    st.copies++;
  }

  // A framebuffer of our own standing in for the headset's (xrcopy=fbo).
  function testFbo(w, h) {
    const gl = st.gl;
    if (st.testFbo && st.testFbo.w === w && st.testFbo.h === h) return st.testFbo.fb;
    const fb = gl.createFramebuffer();
    const rb = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, w, h);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, rb);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);
    st.testFbo = {fb, rb, w, h};
    return fb;
  }

  // The test framebuffer's picture, scaled to `width` px wide: {w, h,
  // rgba: base64}, rows bottom-up (OpenGL's order).
  function readback(width) {
    const gl = st.gl, f = st.testFbo;
    if (!gl || !f) return null;
    const w = Math.max(16, Math.min(width || 640, f.w));
    const h = Math.max(8, Math.round(w * f.h / f.w));
    const small = gl.createFramebuffer();
    const rb = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, w, h);
    gl.bindFramebuffer(gl.FRAMEBUFFER, small);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, rb);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, f.fb);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, small);
    const scissor = gl.isEnabled(gl.SCISSOR_TEST);
    if (scissor) gl.disable(gl.SCISSOR_TEST);
    gl.blitFramebuffer(0, 0, f.w, f.h, 0, 0, w, h, gl.COLOR_BUFFER_BIT, gl.LINEAR);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, small);
    const px = new Uint8Array(w * h * 4);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);
    gl.deleteFramebuffer(small);
    gl.deleteRenderbuffer(rb);
    if (scissor) gl.enable(gl.SCISSOR_TEST);
    let s = "";
    for (let i = 0; i < px.length; i += 0x8000) s += String.fromCharCode.apply(null, px.subarray(i, i + 0x8000));
    return {w, h, rgba: root.btoa(s)};
  }

  function state() {
    const sorted = st.workMs.slice().sort((a, b) => a - b);
    const q = (p) => sorted.length ? Math.round(sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * p))] * 100) / 100 : null;
    return {
      state: st.state, why: st.why, space: st.spaceType, frames: st.frames, copy: st.copy, copies: st.copies,
      error: st.error, waiting: st.queued.size + st.pending.size,
      framebuffer: st.layer ? [st.layer.framebufferWidth, st.layer.framebufferHeight] : null,
      work_ms_p50: q(0.5), work_ms_p95: q(0.95), device: st.device || null,
      beacon: {sent: st.beacon.sent, last: st.beacon.last, error: st.beacon.error},
    };
  }

  function recentre() { event("recentre", "page"); }

  root.ocwXr = {install, offer, enter, exit, state, readback, pair, report, recentre};
})(window);
