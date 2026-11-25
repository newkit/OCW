// The network worker (spec 14 Phase 10, 10G *B3 worker*): holds the page's
// WebTransport connections – the game server's (B3) and the sensor
// bridge's (10D.2) – off the render thread. Each message is stamped with
// the wall clock (performance.timeOrigin + performance.now(), ms) when it
// comes off the network, and posted to the page in arrival order with a
// running number; the page reads them in its next frame, but knows when
// they came. The bytes are transferred, not copied.
//
// From the page:
//   {op: "connect", id, url, hashes: [Uint8Array(32)…], datagrams, rtt, refuse}
//   {op: "send", id, s: [Uint8Array…] (the stream), d: [Uint8Array…] (datagrams)}
//   {op: "close", id, code, reason}
// To the page (each with `seq`, 1, 2, … over the worker's life):
//   {k: "hello", wt}                      started; wt: WebTransport here
//   {id, k: "unsupported"}                no WebTransport in this worker:
//                                         the page connects on its thread
//   {id, k: "connected", t, max}          ready, stream open (max: the
//                                         datagram size, null: none)
//   {id, k: "s" | "d", t, b}              stream bytes / one datagram
//   {id, k: "end", t}                     the stream ended (0.5 s later,
//                                         so `closed` can tell why first)
//   {id, k: "failed", stage, error}       stage: "new", "ready", "stream"
//   {id, k: "closed", reason, error}      the connection ended
//   {id, k: "rtt", ms}                    getStats().smoothedRtt, 1 Hz
"use strict";

const conns = new Map();
let seq = 0;
const wall = () => performance.timeOrigin + performance.now();
const post = (m, transfer) => {
  m.seq = ++seq;
  self.postMessage(m, transfer || []);
};
const text = (e) => (e && e.name && e.message ? `${e.name}: ${e.message}` : String(e && e.message || e));
// A chunk of its own buffer, so it can be transferred whole.
const own = (b) => (b.byteOffset === 0 && b.byteLength === b.buffer.byteLength ? b : b.slice());
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function read(id, reader, kind) {
  for (;;) {
    let r;
    try {
      r = await reader.read();
    } catch (e) {
      return; // the connection ended: `closed` says why
    }
    const t = wall();
    if (r.done) {
      if (kind === "s") {
        await sleep(500);
        post({ id, k: "end", t });
      }
      return;
    }
    const b = own(r.value);
    post({ id, k: kind, t, b }, [b.buffer]);
  }
}

async function rttLoop(id, c) {
  while (conns.get(id) === c) {
    await sleep(1000);
    if (conns.get(id) !== c || typeof c.t.getStats !== "function") return;
    try {
      const s = await c.t.getStats();
      post({ id, k: "rtt", ms: typeof s.smoothedRtt === "number" ? s.smoothedRtt : null });
    } catch (e) {
      /* no statistics in this browser */
    }
  }
}

async function connect(m) {
  const id = m.id;
  if (typeof WebTransport !== "function" || m.refuse) {
    post({ id, k: "unsupported" });
    return;
  }
  const opts = {};
  if (m.hashes && m.hashes.length) {
    opts.serverCertificateHashes = m.hashes.map((value) => ({ algorithm: "sha-256", value }));
  }
  let t;
  try {
    t = new WebTransport(m.url, opts);
  } catch (e) {
    post({ id, k: "failed", stage: "new", error: text(e) });
    return;
  }
  const c = { t, stream: null, dg: null };
  conns.set(id, c);
  try {
    await t.ready;
  } catch (e) {
    conns.delete(id);
    post({ id, k: "failed", stage: "ready", error: text(e) });
    return;
  }
  let bidi;
  try {
    bidi = await t.createBidirectionalStream();
  } catch (e) {
    conns.delete(id);
    post({ id, k: "failed", stage: "stream", error: text(e) });
    return;
  }
  c.stream = bidi.writable.getWriter();
  let max = null;
  if (m.datagrams && t.datagrams) {
    const d = t.datagrams;
    max = d.maxDatagramSize > 0 ? d.maxDatagramSize : null;
    if (max) {
      let w;
      try {
        w = d.createWritable(); // newer browsers; `writable` before
      } catch (e) {
        w = d.writable;
      }
      c.dg = w.getWriter();
    }
  }
  // Anything the page sent before the stream was open.
  const early = c.early || [];
  c.early = null;
  post({ id, k: "connected", t: wall(), max });
  for (const e of early) send(e);
  read(id, bidi.readable.getReader(), "s");
  if (m.datagrams && t.datagrams) read(id, t.datagrams.readable.getReader(), "d");
  if (m.rtt) rttLoop(id, c);
  t.closed.then(
    (info) => post({ id, k: "closed", reason: (info && info.reason) || "", error: false }),
    (e) => post({ id, k: "closed", reason: text(e), error: true }),
  ).finally(() => {
    if (conns.get(id) === c) conns.delete(id);
  });
}

function send(m) {
  const c = conns.get(m.id);
  if (!c) return;
  if (!c.stream) {
    (c.early = c.early || []).push(m);
    return;
  }
  for (const b of m.s || []) c.stream.write(b).catch(() => {});
  if (c.dg) for (const b of m.d || []) c.dg.write(b).catch(() => {});
}

function close(m) {
  const c = conns.get(m.id);
  if (!c) return;
  conns.delete(m.id);
  try {
    c.t.close({ closeCode: m.code || 0, reason: m.reason || "" });
  } catch (e) {
    /* already closed */
  }
}

self.onmessage = (e) => {
  const m = e.data;
  if (!m) return;
  if (m.op === "connect") connect(m);
  else if (m.op === "send") send(m);
  else if (m.op === "close") close(m);
};

post({ k: "hello", wt: typeof WebTransport === "function" });
