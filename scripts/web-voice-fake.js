// A fake `window.LivekitClient` for the browser test of voice (spec §14
// Phase 10 B5; scripts/test-web-audio.py --part voice). Injected before the
// page's scripts (Page.addScriptToEvaluateOnNewDocument). The page's
// livekit-client.umd.js assigns window.LivekitClient as it loads: this file
// defines it as a getter whose setter ignores that, so the fake stays.
//
// It acts like a LiveKit server and SDK for the app's shim (web/ocw-voice.js):
// connect() with the token the game server issued (its claims are read:
// identity `sub`, room `video.room`), remote participants the test adds,
// their audio tracks subscribed on request, mute, active speakers, a
// network drop (reconnecting → reconnected) and a lost connection.
// window.__lk is the test's handle: rooms, the log of calls, and
// addParticipant / removeParticipant / speak / muteRemote / drop / kill.
(function () {
  "use strict";
  const RoomEvent = {
    Connected: "connected", ParticipantConnected: "participantConnected",
    ParticipantDisconnected: "participantDisconnected", TrackPublished: "trackPublished",
    TrackSubscribed: "trackSubscribed", TrackUnsubscribed: "trackUnsubscribed",
    TrackMuted: "trackMuted", TrackUnmuted: "trackUnmuted",
    ActiveSpeakersChanged: "activeSpeakersChanged", Reconnecting: "reconnecting",
    Reconnected: "reconnected", Disconnected: "disconnected",
    MediaDevicesError: "mediaDevicesError", MediaDevicesChanged: "mediaDevicesChanged",
  };
  const DisconnectReason = { UNKNOWN_REASON: 0, CLIENT_INITIATED: 1, SIGNAL_CLOSE: 9 };
  for (const [k, v] of Object.entries({ ...DisconnectReason })) DisconnectReason[v] = k;
  const log = [];
  const rooms = [];
  const note = (what, extra) => log.push({ t: performance.now(), what, ...(extra || {}) });

  function claims(token) {
    try {
      const b = token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
      return JSON.parse(atob(b + "=".repeat((4 - b.length % 4) % 4)));
    } catch (e) { return null; }
  }

  class Emitter {
    constructor() { this.handlers = {}; }
    on(ev, fn) { (this.handlers[ev] = this.handlers[ev] || []).push(fn); return this; }
    emit(ev, ...args) { for (const fn of this.handlers[ev] || []) { try { fn(...args); } catch (e) { console.error("fake livekit handler", e); } } }
  }

  class Track {
    constructor(identity) { this.kind = "audio"; this.identity = identity; this.volume = 1; this.plugins = []; }
    attach() { const el = document.createElement("audio"); el.dataset.identity = this.identity; return el; }
    detach() { return []; }
    setVolume(v) { this.volume = v; note("volume", { identity: this.identity, v }); }
    setWebAudioPlugins(nodes) { this.plugins = nodes; }
  }

  class Publication {
    constructor(room, participant) {
      this.room = room; this.participant = participant; this.kind = "audio";
      this.isMuted = false; this.isSubscribed = false; this.track = null;
    }
    setSubscribed(on) {
      if (on === this.isSubscribed) return;
      this.isSubscribed = on;
      note("subscribe", { identity: this.participant.identity, on });
      setTimeout(() => {
        if (on) { this.track = new Track(this.participant.identity); this.room.emit(RoomEvent.TrackSubscribed, this.track, this, this.participant); }
        else if (this.track) { const t = this.track; this.track = null; this.room.emit(RoomEvent.TrackUnsubscribed, t, this, this.participant); }
      }, 20);
    }
  }

  class Room extends Emitter {
    constructor(opts) {
      super();
      this.opts = opts; this.name = ""; this.state = "disconnected";
      this.remoteParticipants = new Map();
      this.canPlaybackAudio = true;
      const room = this;
      this.localParticipant = {
        identity: "", isMicrophoneEnabled: false,
        async setMicrophoneEnabled(on) {
          note("mic", { on });
          this.isMicrophoneEnabled = on;
          // A real SDK opens the microphone here.
          if (on && navigator.mediaDevices) {
            try { room.micStream = room.micStream || await navigator.mediaDevices.getUserMedia({ audio: true }); } catch (e) { /* fake device */ }
          }
        },
      };
      rooms.push(this);
    }
    async connect(url, token, opts) {
      const c = claims(token);
      note("connect", { url, ok: !!c, room: c && c.video && c.video.room, identity: c && c.sub, autoSubscribe: opts && opts.autoSubscribe });
      await new Promise((r) => setTimeout(r, 50));
      if (!c || !c.video || !c.video.roomJoin) throw new Error("could not establish signal connection: invalid token");
      this.name = c.video.room;
      this.localParticipant.identity = c.sub;
      this.state = "connected";
      this.emit(RoomEvent.Connected);
    }
    async disconnect() {
      note("disconnect");
      if (this.state === "disconnected") return;
      this.state = "disconnected";
      this.emit(RoomEvent.Disconnected, DisconnectReason.CLIENT_INITIATED);
    }
    async startAudio() { note("startAudio"); }
    async switchActiveDevice(kind, id) { note("switchDevice", { kind, id }); return true; }
  }

  function live() { return rooms.filter((r) => r.state === "connected"); }

  const api = {
    Room, RoomEvent, DisconnectReason,
    Track: { Source: { Microphone: "microphone" } },
  };
  Object.defineProperty(window, "LivekitClient", { get: () => api, set: () => {}, configurable: false });

  window.__lk = {
    log, rooms,
    state() {
      const r = live()[0];
      return JSON.stringify({
        rooms: rooms.length, connected: live().length,
        room: r ? r.name : null, identity: r ? r.localParticipant.identity : null,
        mic: r ? r.localParticipant.isMicrophoneEnabled : null,
        participants: r ? [...r.remoteParticipants.values()].map((p) => {
          const pub = [...p.audioTrackPublications.values()][0];
          return { identity: p.identity, subscribed: pub.isSubscribed, volume: pub.track ? pub.track.volume : null,
                   panned: !!(pub.track && pub.track.plugins.length) };
        }) : [],
        webAudioMix: !!(r && r.opts && r.opts.webAudioMix),
        log: log.slice(-60),
      });
    },
    addParticipant(identity, name) {
      for (const r of live()) {
        const p = { identity, name, audioTrackPublications: new Map() };
        p.audioTrackPublications.set("TR_" + identity, new Publication(r, p));
        r.remoteParticipants.set(identity, p);
        r.emit(RoomEvent.ParticipantConnected, p);
      }
    },
    removeParticipant(identity) {
      for (const r of live()) {
        const p = r.remoteParticipants.get(identity);
        if (!p) continue;
        r.remoteParticipants.delete(identity);
        r.emit(RoomEvent.ParticipantDisconnected, p);
      }
    },
    speak(identities) {
      for (const r of live()) {
        const all = [...r.remoteParticipants.values(), r.localParticipant];
        r.emit(RoomEvent.ActiveSpeakersChanged, all.filter((p) => identities.includes(p.identity)));
      }
    },
    muteRemote(identity, muted) {
      for (const r of live()) {
        const p = r.remoteParticipants.get(identity);
        if (!p) continue;
        const pub = [...p.audioTrackPublications.values()][0];
        pub.isMuted = muted;
        r.emit(muted ? RoomEvent.TrackMuted : RoomEvent.TrackUnmuted, pub, p);
      }
    },
    drop(ms) {
      for (const r of live()) {
        r.emit(RoomEvent.Reconnecting);
        setTimeout(() => r.emit(RoomEvent.Reconnected), ms || 1500);
      }
    },
    kill() {
      for (const r of live()) {
        r.state = "disconnected";
        r.emit(RoomEvent.Disconnected, DisconnectReason.SIGNAL_CLOSE);
      }
    },
  };
})();
