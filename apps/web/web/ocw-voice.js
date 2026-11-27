// The browser app's voice shim (spec §7, §14 Phase 10 B5): a thin wrapper
// of LiveKit's JS SDK (`livekit-client`, Apache-2.0, loaded before this file
// as window.LivekitClient) for the app's voice backend
// (`ocw_voice::web::WebVoiceBackend`). All decisions – when to join, which
// voices to hear and how loud, the mute switches, retries – are the app's
// (the same VoiceController as the desktop); this file only does what it
// is told and reports what happened.
//
//   ocwVoice.available()   "" or why voice cannot work on this page
//   ocwVoice.command(json) {"op": "connect" | "disconnect" | "mic_muted" |
//                          "speaker_muted" | "input" | "output" |
//                          "subscribe" | "place" | "echo" | "subscribers", ...}
//   ocwVoice.poll()        the events since the last call (a JSON array)
//   ocwVoice.devices()     {"inputs": [labels], "outputs": [labels]}
//   ocwVoice.levels()      {"input": rms, "echo": rms}
//   ocwVoice.state()       a summary for tests and the page's panel
//
// Voices play through their own AudioContext (like the desktop's separate
// voice output): each remote microphone track goes through a stereo panner
// and its gain (LiveKit's webAudioMix), so the app's spatial placement
// works. The echo test (spec §7 *Echo bot*) is the microphone through a
// delay into the same context. The context is resumed by the first user
// gesture (the autoplay rule).
(function () {
  "use strict";
  const LK = () => window.LivekitClient;
  // LiveKit's SDK is loaded when a room is joined, not with the page
  // (spec 14 Phase 10, 10G: a solo ride never fetches it):
  // window.ocwVoiceSdk is its file (index.html; "" or unset: none).
  let sdkLoading = null;
  function loadSdk() {
    if (LK()) return Promise.resolve(true);
    const src = window.ocwVoiceSdk;
    if (!src) return Promise.resolve(false);
    if (!sdkLoading) {
      sdkLoading = new Promise((ok) => {
        const el = document.createElement("script");
        el.crossOrigin = "anonymous";
        el.src = src;
        el.onload = () => { console.log("ocw-voice: LiveKit's SDK loaded (" + src + ")"); ok(!!LK()); };
        el.onerror = () => { console.warn("ocw-voice: " + src + " not loaded"); sdkLoading = null; ok(false); };
        document.head.appendChild(el);
      });
    }
    return sdkLoading;
  }
  const events = [];
  const push = (e) => { events.push(e); if (events.length > 1000) events.shift(); };
  const msg = (e) => String((e && (e.message || e.name)) || e);
  const st = {
    room: null, gen: 0, micMuted: false, speakerMuted: false,
    input: null, output: null,
    want: new Map(), place: new Map(), tracks: new Map(), elements: new Map(),
    inputs: [], outputs: [], ids: { input: new Map(), output: new Map() },
    echoParams: null, echo: null, echoStarting: null, micMeter: null,
    // Who may subscribe to the own microphone (null: everybody; spec
    // §7.1.7 Q9, a voice block): LiveKit's track subscription permissions.
    subscribers: null,
    ctx: null,
  };

  function ctx() {
    if (!st.ctx) {
      const C = window.AudioContext || window.webkitAudioContext;
      if (!C) return null;
      st.ctx = new C();
    }
    return st.ctx;
  }
  // The autoplay rule: resume in the handler of a gesture.
  for (const ev of ["pointerdown", "keydown", "touchend", "mousedown"]) {
    window.addEventListener(ev, () => {
      if (st.ctx && st.ctx.state !== "running") st.ctx.resume().catch(() => {});
      if (st.room && !st.room.canPlaybackAudio) st.room.startAudio().catch(() => {});
    }, true);
  }

  function available() {
    if (!LK() && !window.ocwVoiceSdk) return "livekit-client is not loaded (the build could not fetch it, or voice is off for this visit)";
    if (!window.RTCPeerConnection) return "this browser has no WebRTC";
    if (!window.isSecureContext) return "voice needs a secure page (https or localhost)";
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) return "no microphone access on this page";
    return "";
  }

  async function refreshDevices() {
    if (!navigator.mediaDevices || !navigator.mediaDevices.enumerateDevices) return;
    try {
      const list = await navigator.mediaDevices.enumerateDevices();
      const of = (kind) => list.filter((d) => d.kind === kind && d.label.trim());
      st.inputs = of("audioinput").map((d) => d.label.trim());
      st.outputs = of("audiooutput").map((d) => d.label.trim());
      st.ids.input = new Map(of("audioinput").map((d) => [d.label.trim(), d.deviceId]));
      st.ids.output = new Map(of("audiooutput").map((d) => [d.label.trim(), d.deviceId]));
    } catch (e) { /* no devices: the defaults */ }
  }
  if (navigator.mediaDevices && navigator.mediaDevices.addEventListener) {
    navigator.mediaDevices.addEventListener("devicechange", refreshDevices);
  }
  refreshDevices();
  const deviceId = (kind, label) => (label && st.ids[kind].get(label)) || undefined;

  // ------------------------------------------------------------ the room

  function joined(p) {
    push({ type: "joined", identity: p.identity, name: p.name || "" });
    for (const pub of p.audioTrackPublications.values()) {
      if (pub.isMuted) push({ type: "muted", identity: p.identity, muted: true });
    }
    applySubscription(p);
  }

  function applySubscription(p) {
    const on = !!st.want.get(p.identity);
    for (const pub of p.audioTrackPublications.values()) {
      if (pub.isSubscribed !== on && pub.setSubscribed) pub.setSubscribed(on);
    }
  }

  function attach(track, identity) {
    st.tracks.set(identity, track);
    // The element keeps the remote stream flowing (Chromium needs it);
    // with webAudioMix it is silent and the context plays.
    const el = track.attach();
    el.style.display = "none";
    document.body.appendChild(el);
    st.elements.set(identity, el);
    const c = ctx();
    if (c && track.setWebAudioPlugins) {
      const pan = c.createStereoPanner();
      track.__ocwPan = pan;
      track.setWebAudioPlugins([pan]);
    }
    applyPlacement(identity);
  }

  function detach(identity) {
    const t = st.tracks.get(identity);
    if (t) t.detach();
    const el = st.elements.get(identity);
    if (el) el.remove();
    st.tracks.delete(identity);
    st.elements.delete(identity);
  }

  function applyPlacement(identity) {
    const p = st.place.get(identity) || { gain: 1, pan: 0 };
    if (identity === "echo") return applyEcho();
    const t = st.tracks.get(identity);
    if (!t) return;
    const gain = st.speakerMuted ? 0 : Math.max(0, p.gain);
    if (t.setVolume) t.setVolume(gain);
    if (t.__ocwPan) t.__ocwPan.pan.value = Math.max(-1, Math.min(1, p.pan));
  }

  function reasonText(r) {
    const D = LK().DisconnectReason || {};
    if (r === undefined || r === D.CLIENT_INITIATED) return "";
    const name = D[r];
    return typeof name === "string" ? name.toLowerCase().replace(/_/g, " ") : String(r);
  }

  function wire(room) {
    const E = LK().RoomEvent;
    const local = () => room.localParticipant;
    room.on(E.ParticipantConnected, joined);
    room.on(E.ParticipantDisconnected, (p) => { detach(p.identity); push({ type: "left", identity: p.identity }); });
    room.on(E.TrackPublished, (pub, p) => applySubscription(p));
    room.on(E.TrackSubscribed, (track, pub, p) => { if (track.kind === "audio") attach(track, p.identity); });
    room.on(E.TrackUnsubscribed, (track, pub, p) => { if (track.kind === "audio") detach(p.identity); });
    room.on(E.TrackMuted, (pub, p) => { if (p !== local() && pub.kind === "audio") push({ type: "muted", identity: p.identity, muted: true }); });
    room.on(E.TrackUnmuted, (pub, p) => { if (p !== local() && pub.kind === "audio") push({ type: "muted", identity: p.identity, muted: false }); });
    room.on(E.ActiveSpeakersChanged, (speakers) => push({ type: "speaking", identities: speakers.map((s) => s.identity) }));
    room.on(E.Reconnecting, () => push({ type: "reconnecting" }));
    room.on(E.Reconnected, () => push({ type: "reconnected" }));
    room.on(E.Disconnected, (reason) => {
      if (st.room !== room) return;
      st.room = null;
      for (const id of [...st.tracks.keys()]) detach(id);
      push({ type: "disconnected", reason: reasonText(reason) });
    });
    room.on(E.MediaDevicesError, (e) => push({ type: "device_error", error: msg(e) }));
    if (E.MediaDevicesChanged) room.on(E.MediaDevicesChanged, refreshDevices);
  }

  async function connect(url, token) {
    const gen = ++st.gen;
    if (st.room) { const old = st.room; st.room = null; await old.disconnect(); }
    if (!(await loadSdk())) {
      if (gen === st.gen) push({ type: "connect_failed", error: "livekit-client could not be loaded" });
      return;
    }
    if (gen !== st.gen) return;
    const c = ctx();
    const opts = {
      adaptiveStream: false, dynacast: false,
      audioCaptureDefaults: {
        deviceId: deviceId("input", st.input),
        echoCancellation: true, noiseSuppression: true, autoGainControl: true,
      },
    };
    if (c) opts.webAudioMix = { audioContext: c };
    const room = new (LK().Room)(opts);
    st.room = room;
    wire(room);
    try {
      await room.connect(url, token, { autoSubscribe: false });
    } catch (e) {
      if (st.room === room) st.room = null;
      if (gen === st.gen) push({ type: "connect_failed", error: msg(e) });
      return;
    }
    if (gen !== st.gen || st.room !== room) { room.disconnect(); return; }
    if (st.subscribers !== null) applySubscribers(room);
    push({ type: "connected", room: room.name });
    for (const p of room.remoteParticipants.values()) joined(p);
    // The microphone, published at once (always on, like a call) and
    // muted by the switch: unmuting is instant.
    try {
      await room.localParticipant.setMicrophoneEnabled(!st.micMuted);
      await refreshDevices();
    } catch (e) {
      push({ type: "device_error", error: msg(e) });
    }
    if (st.output) setOutput(st.output);
    if (st.echoParams) applyEcho();
  }

  // The allow list of the own microphone, enforced by the voice server.
  function applySubscribers(room) {
    room = room || st.room;
    if (!room || !room.localParticipant.setTrackSubscriptionPermissions) return;
    const allowed = st.subscribers;
    try {
      room.localParticipant.setTrackSubscriptionPermissions(
        allowed === null,
        allowed === null ? [] : allowed.map((id) => ({ participantIdentity: id, allowAll: true })));
    } catch (e) {
      push({ type: "device_error", error: "subscription permissions: " + msg(e) });
    }
  }

  async function disconnect() {
    st.gen++;
    const room = st.room;
    st.room = null;
    for (const id of [...st.tracks.keys()]) detach(id);
    if (room) {
      await room.disconnect();
      push({ type: "disconnected", reason: "" });
    }
  }

  function setOutput(label) {
    st.output = label;
    const id = deviceId("output", label) || "";
    if (st.room && st.room.switchActiveDevice) {
      st.room.switchActiveDevice("audiooutput", id || "default").catch((e) => push({ type: "device_error", error: msg(e) }));
    } else if (st.ctx && st.ctx.setSinkId) {
      st.ctx.setSinkId(id).catch((e) => push({ type: "device_error", error: msg(e) }));
    }
  }

  function setInput(label) {
    st.input = label;
    if (st.room && st.room.switchActiveDevice) {
      st.room.switchActiveDevice("audioinput", deviceId("input", label) || "default").catch((e) => push({ type: "device_error", error: msg(e) }));
    }
    if (st.echo) { stopEcho(); applyEcho(); }
  }

  // ------------------------------------------------------------ echo test

  function stopEcho() {
    const e = st.echo;
    st.echo = null;
    if (!e) return;
    for (const t of e.stream.getTracks()) t.stop();
    try { e.src.disconnect(); e.out.disconnect(); e.pan.disconnect(); } catch (x) { /* gone */ }
  }

  async function startEcho() {
    const c = ctx();
    if (!c) throw new Error("no Web Audio");
    const stream = await navigator.mediaDevices.getUserMedia({ audio: {
      deviceId: deviceId("input", st.input),
      echoCancellation: true, noiseSuppression: true, autoGainControl: true,
    } });
    await refreshDevices();
    const src = c.createMediaStreamSource(stream);
    const rec = c.createGain(), delay = c.createDelay(3.5), out = c.createGain();
    const pan = c.createStereoPanner(), inA = c.createAnalyser(), outA = c.createAnalyser();
    inA.fftSize = outA.fftSize = 1024;
    src.connect(inA); src.connect(rec); rec.connect(delay); delay.connect(out);
    out.connect(pan); pan.connect(c.destination); out.connect(outA);
    return { stream, src, rec, delay, out, pan, inA, outA };
  }

  function applyEcho() {
    const p = st.echoParams;
    if (!p || !p.active) { stopEcho(); return; }
    if (!st.echo) {
      if (!st.echoStarting) {
        st.echoStarting = startEcho().then((e) => {
          st.echoStarting = null;
          if (st.echoParams && st.echoParams.active) { st.echo = e; applyEcho(); }
          else { st.echo = e; stopEcho(); }
        }).catch((e) => {
          st.echoStarting = null;
          push({ type: "device_error", error: msg(e) });
        });
      }
      return;
    }
    const e = st.echo, t = e.out.context.currentTime;
    // Nothing said while muted is ever echoed; what was said before plays
    // out within the delay.
    e.rec.gain.setTargetAtTime(st.micMuted ? 0 : p.record_gain, t, 0.02);
    e.delay.delayTime.setValueAtTime(Math.max(0.02, Math.min(3, p.delay_ms / 1000)), t);
    const pl = st.place.get("echo") || { gain: 1, pan: 0 };
    e.out.gain.setTargetAtTime(st.speakerMuted ? 0 : Math.max(0, pl.gain), t, 0.02);
    e.pan.pan.setValueAtTime(Math.max(-1, Math.min(1, pl.pan)), t);
  }

  // The level meters (like the desktop's: the RMS of each 50 ms, rising at
  // once, falling by 0.7 per 50 ms).
  st.meter = { input: 0, echo: 0 };
  setInterval(() => {
    const e = st.echo;
    st.meter.input = e ? Math.max(rms(e.inA), st.meter.input * 0.7) : 0;
    st.meter.echo = e ? Math.max(rms(e.outA), st.meter.echo * 0.7) : 0;
  }, 50);

  function rms(analyser) {
    if (!analyser) return 0;
    const a = new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(a);
    let s = 0;
    for (const v of a) s += v * v;
    return Math.sqrt(s / a.length);
  }

  // ------------------------------------------------------------ the API

  function command(json) {
    let c;
    try { c = JSON.parse(json); } catch (e) { return; }
    switch (c.op) {
      case "connect": connect(c.url, c.token); break;
      case "disconnect": disconnect(); break;
      case "mic_muted":
        st.micMuted = !!c.on;
        if (st.room) st.room.localParticipant.setMicrophoneEnabled(!st.micMuted).catch((e) => push({ type: "device_error", error: msg(e) }));
        applyEcho();
        break;
      case "speaker_muted":
        st.speakerMuted = !!c.on;
        for (const id of st.tracks.keys()) applyPlacement(id);
        applyEcho();
        break;
      case "input": setInput(c.device || null); break;
      case "output": setOutput(c.device || null); break;
      case "subscribe": {
        st.want.set(c.identity, !!c.on);
        const p = st.room && st.room.remoteParticipants.get(c.identity);
        if (p) applySubscription(p);
        break;
      }
      case "place":
        st.place.set(c.identity, { gain: c.gain, pan: c.pan });
        applyPlacement(c.identity);
        break;
      case "subscribers":
        st.subscribers = Array.isArray(c.allowed) ? c.allowed.map(String) : null;
        applySubscribers();
        break;
      case "echo":
        st.echoParams = { active: !!c.active, delay_ms: c.delay_ms, record_gain: c.record_gain };
        applyEcho();
        break;
    }
  }

  window.ocwVoice = {
    available,
    loadSdk,
    command,
    poll: () => JSON.stringify(events.splice(0)),
    devices: () => JSON.stringify({ inputs: st.inputs, outputs: st.outputs }),
    levels: () => JSON.stringify(st.meter),
    state: () => JSON.stringify({
      connected: !!st.room, room: st.room ? st.room.name : null,
      mic_muted: st.micMuted, speaker_muted: st.speakerMuted,
      mic_enabled: !!(st.room && st.room.localParticipant.isMicrophoneEnabled),
      subscribed: [...st.want].filter(([, on]) => on).map(([id]) => id),
      placed: Object.fromEntries(st.place),
      tracks: [...st.tracks.keys()],
      subscribers: st.subscribers,
      echo: st.echoParams, echo_running: !!st.echo,
      context: st.ctx ? st.ctx.state : null,
    }),
  };
})();
