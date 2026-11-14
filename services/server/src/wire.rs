//! The game connection's wire rules, without I/O (spec §6.2; wasm-safe,
//! shared by the QUIC transport [`crate::net`] and the browser app's
//! WebTransport client, spec §14 Phase 10 B3):
//!
//! - **frames** on the reliable stream: a 4-byte little-endian length,
//!   then that many bytes of one encoded protocol message
//!   ([`encode_frame`], the incremental [`FrameDecoder`]);
//! - **which messages go unreliably** (as datagrams): snapshots (server →
//!   client) and sensor samples (client → server), both superseded by the
//!   next one ([`server_unreliable`], [`client_unreliable`]);
//! - the client's **connection state machine** [`ClientLink`]: queues
//!   what is sent before the connection is up, sends samples on the stream
//!   until the server has answered the Hello, uses datagrams only when the
//!   peer accepts them and the message fits, decodes the stream's frames.
//!   The transport (browser WebTransport, a test fake) only moves bytes.

use std::collections::VecDeque;

/// Largest accepted frame. Protocol messages are small (a path update of a
/// long ride is a few kB); anything near this is a broken or hostile peer.
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds the limit of {MAX_FRAME}")]
    TooLarge(usize),
    #[error("stream ended inside a frame")]
    Truncated,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The frame header for a payload.
pub fn header(len: usize) -> Result<[u8; 4], FrameError> {
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    Ok((len as u32).to_le_bytes())
}

/// One whole frame: header and payload.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(&header(payload.len())?);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Splits a byte stream that arrives in arbitrary chunks into frames.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Adds received bytes; returns the frames completed by them. An
    /// oversized length is an error (the stream cannot be trusted any more).
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut at = 0;
        while self.buf.len() - at >= 4 {
            let head: [u8; 4] = self.buf[at..at + 4].try_into().expect("4 bytes");
            let len = u32::from_le_bytes(head) as usize;
            if len > MAX_FRAME {
                return Err(FrameError::TooLarge(len));
            }
            if self.buf.len() - at - 4 < len {
                break;
            }
            frames.push(self.buf[at + 4..at + 4 + len].to_vec());
            at += 4 + len;
        }
        self.buf.drain(..at);
        Ok(frames)
    }

    /// The stream ended: an error if it ended inside a frame.
    pub fn finish(&self) -> Result<(), FrameError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Truncated)
        }
    }
}

/// The field number of the `oneof` member an encoded protocol message
/// carries (its first protobuf key), without decoding the message.
pub fn oneof_field(bytes: &[u8]) -> Option<u32> {
    let mut key: u64 = 0;
    for (i, b) in bytes.iter().take(10).enumerate() {
        key |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            // Wire type 2 (length-delimited): a message field.
            return (key & 7 == 2).then_some((key >> 3) as u32);
        }
    }
    None
}

/// `ServerMessage.snapshot`: superseded by the next one, sent unreliably.
pub const SERVER_SNAPSHOT_FIELD: u32 = 2;
/// `ClientMessage.sample`: superseded by the next one, sent unreliably.
pub const CLIENT_SAMPLE_FIELD: u32 = 2;

/// Whether an encoded `ServerMessage` may go as an unreliable datagram.
pub fn server_unreliable(bytes: &[u8]) -> bool {
    oneof_field(bytes) == Some(SERVER_SNAPSHOT_FIELD)
}

/// Whether an encoded `ClientMessage` may go as an unreliable datagram.
pub fn client_unreliable(bytes: &[u8]) -> bool {
    oneof_field(bytes) == Some(CLIENT_SAMPLE_FIELD)
}

/// State of the link, for the HUD and `net.stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Connecting,
    /// `datagrams`: datagrams work both ways (else what they would carry
    /// goes on the stream).
    Connected {
        datagrams: bool,
    },
    /// The connection ended (or never came up); why, for the user.
    Closed(String),
}

/// What the transport must send for the [`ClientLink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// One unreliable datagram (one whole encoded message).
    Datagram(Vec<u8>),
    /// Bytes for the reliable stream (whole frames, in order).
    Stream(Vec<u8>),
}

/// Counters of a [`ClientLink`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkCounters {
    pub frames_in: u64,
    pub datagrams_in: u64,
    pub frames_out: u64,
    pub datagrams_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Messages sent after the link closed (dropped).
    pub dropped: u64,
}

/// The client side of a game connection without I/O: what to send how,
/// and what was received (see the module docs). Times are the caller's
/// clock in seconds.
#[derive(Debug)]
pub struct ClientLink {
    status: LinkStatus,
    /// Sent before the connection is up.
    pending: Vec<Vec<u8>>,
    outbox: VecDeque<Outgoing>,
    /// Received messages with their arrival time (the caller's clock).
    inbox: Vec<(f64, Vec<u8>)>,
    decoder: FrameDecoder,
    /// The server answered on the stream (Welcome or Reject): from now on
    /// samples may go as datagrams (none overtakes the Hello).
    answered: bool,
    max_datagram: Option<usize>,
    connected_at_s: Option<f64>,
    /// Hello → first answer on the stream (a round trip plus the server's
    /// tick wait), the RTT estimate until the transport reports one.
    handshake_rtt_s: Option<f64>,
    transport_rtt_s: Option<f64>,
    counters: LinkCounters,
}

impl Default for ClientLink {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientLink {
    pub fn new() -> Self {
        Self {
            status: LinkStatus::Connecting,
            pending: Vec::new(),
            outbox: VecDeque::new(),
            inbox: Vec::new(),
            decoder: FrameDecoder::default(),
            answered: false,
            max_datagram: None,
            connected_at_s: None,
            handshake_rtt_s: None,
            transport_rtt_s: None,
            counters: LinkCounters::default(),
        }
    }

    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.status, LinkStatus::Closed(_))
    }

    pub fn counters(&self) -> &LinkCounters {
        &self.counters
    }

    /// Queues an encoded `ClientMessage`: held until the connection is up,
    /// dropped once it is closed.
    pub fn send(&mut self, bytes: Vec<u8>) {
        match self.status {
            LinkStatus::Connecting => self.pending.push(bytes),
            LinkStatus::Connected { .. } => self.route(bytes),
            LinkStatus::Closed(_) => self.counters.dropped += 1,
        }
    }

    fn route(&mut self, bytes: Vec<u8>) {
        self.counters.bytes_out += bytes.len() as u64;
        if self.answered
            && client_unreliable(&bytes)
            && self.max_datagram.is_some_and(|max| bytes.len() <= max)
        {
            self.counters.datagrams_out += 1;
            self.outbox.push_back(Outgoing::Datagram(bytes));
            return;
        }
        match encode_frame(&bytes) {
            Ok(frame) => {
                self.counters.frames_out += 1;
                self.outbox.push_back(Outgoing::Stream(frame));
            }
            // Never happens for real messages; dropped rather than closing.
            Err(_) => self.counters.dropped += 1,
        }
    }

    /// The transport is up (the stream is open). `max_datagram`: the
    /// largest datagram payload the peer accepts, `None` without datagrams.
    pub fn on_connected(&mut self, now_s: f64, max_datagram: Option<usize>) {
        if !matches!(self.status, LinkStatus::Connecting) {
            return;
        }
        let max_datagram = max_datagram.filter(|&m| m > 0);
        self.status = LinkStatus::Connected {
            datagrams: max_datagram.is_some(),
        };
        self.max_datagram = max_datagram;
        self.connected_at_s = Some(now_s);
        for m in std::mem::take(&mut self.pending) {
            self.route(m);
        }
    }

    /// Bytes from the reliable stream (any chunking). A broken frame
    /// closes the link (the caller closes the transport).
    pub fn on_stream_data(&mut self, now_s: f64, chunk: &[u8]) {
        if self.is_closed() {
            return;
        }
        self.counters.bytes_in += chunk.len() as u64;
        match self.decoder.push(chunk) {
            Ok(frames) => {
                for f in frames {
                    if !self.answered {
                        self.answered = true;
                        self.handshake_rtt_s = self.connected_at_s.map(|t| (now_s - t).max(0.0));
                    }
                    self.counters.frames_in += 1;
                    self.inbox.push((now_s, f));
                }
            }
            Err(e) => self.on_closed(format!("bad frame from the server: {e}")),
        }
    }

    /// The stream ended (the server closed it).
    pub fn on_stream_end(&mut self) {
        let reason = match self.decoder.finish() {
            Ok(()) => "the server closed the stream".to_owned(),
            Err(e) => format!("bad frame from the server: {e}"),
        };
        self.on_closed(reason);
    }

    /// One datagram (one whole encoded `ServerMessage`) arrived at `now_s`.
    pub fn on_datagram(&mut self, now_s: f64, bytes: Vec<u8>) {
        if self.is_closed() {
            return;
        }
        self.counters.datagrams_in += 1;
        self.counters.bytes_in += bytes.len() as u64;
        self.inbox.push((now_s, bytes));
    }

    /// The connection ended (or could not be made). The first reason stays.
    pub fn on_closed(&mut self, reason: impl Into<String>) {
        if !self.is_closed() {
            self.status = LinkStatus::Closed(reason.into());
            self.pending.clear();
            self.outbox.clear();
        }
    }

    /// The transport's smoothed round-trip time (e.g. WebTransport stats).
    pub fn set_transport_rtt(&mut self, rtt_s: Option<f64>) {
        self.transport_rtt_s = rtt_s.filter(|r| r.is_finite() && *r >= 0.0);
    }

    /// The round trip: the transport's, else the Hello → Welcome time.
    pub fn rtt_s(&self) -> Option<f64> {
        self.transport_rtt_s.or(self.handshake_rtt_s)
    }

    /// What the transport must send now, in order.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        self.outbox.drain(..).collect()
    }

    /// The encoded `ServerMessage`s received since the last call.
    pub fn receive(&mut self) -> Vec<Vec<u8>> {
        self.receive_timed().into_iter().map(|(_, m)| m).collect()
    }

    /// The same with each message's arrival time (when the transport got
    /// it, not when the frame loop looks: snapshot timing).
    pub fn receive_timed(&mut self) -> Vec<(f64, Vec<u8>)> {
        std::mem::take(&mut self.inbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{client_message, encode, server_message};

    fn sample(seq: u32) -> Vec<u8> {
        encode(&client_message(C::Sample(v1::SensorSample {
            seq,
            power_w: Some(200.0),
            ..Default::default()
        })))
    }

    fn hello() -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            display_name: "Web".into(),
            ..Default::default()
        })))
    }

    #[test]
    fn only_snapshots_and_samples_are_unreliable() {
        let snapshot = encode(&server_message(S::Snapshot(v1::Snapshot::default())));
        // An empty snapshot still carries its (empty) field.
        assert!(server_unreliable(&snapshot));
        let reliable = [
            S::Welcome(v1::Welcome::default()),
            S::Reject(v1::Reject::default()),
            S::DevResult(v1::DevCommandResult::default()),
            S::Path(v1::PathUpdate::default()),
        ];
        for m in reliable {
            assert!(!server_unreliable(&encode(&server_message(m))));
        }
        assert!(client_unreliable(&sample(7)));
        let reliable = [
            C::Hello(v1::Hello::default()),
            C::Steer(v1::SteerIntent::default()),
            C::Turn(v1::TurnIntent::default()),
            C::UTurn(v1::UTurn::default()),
            C::RouteRejoin(v1::RouteRejoin::default()),
            C::Dev(v1::DevCommand::default()),
        ];
        for m in reliable {
            assert!(!client_unreliable(&encode(&client_message(m))));
        }
    }

    #[test]
    fn oneof_field_reads_long_keys_and_refuses_garbage() {
        // Field 300, wire type 2: key 2402 = 0xe2 0x12.
        assert_eq!(oneof_field(&[0xe2, 0x12, 0x00]), Some(300));
        assert_eq!(oneof_field(&[]), None);
        // Wire type 0 (varint), not a message.
        assert_eq!(oneof_field(&[0x10, 0x01]), None);
        // Unterminated varint.
        assert_eq!(oneof_field(&[0xff; 12]), None);
    }

    #[test]
    fn the_decoder_reassembles_frames_from_any_chunking() {
        let payloads: Vec<Vec<u8>> = (0..40u8).map(|i| vec![i; usize::from(i) * 37]).collect();
        let mut bytes = Vec::new();
        for p in &payloads {
            bytes.extend(encode_frame(p).unwrap());
        }
        for chunk in [1, 3, 7, 64, 1000, bytes.len()] {
            let mut d = FrameDecoder::default();
            let mut got = Vec::new();
            for c in bytes.chunks(chunk) {
                got.extend(d.push(c).unwrap());
            }
            assert_eq!(got, payloads, "chunks of {chunk}");
            assert!(d.finish().is_ok());
        }
        // Cut off inside a frame.
        let mut d = FrameDecoder::default();
        d.push(&bytes[..10]).unwrap();
        assert!(matches!(d.finish(), Err(FrameError::Truncated)));
        // An oversized length.
        let mut d = FrameDecoder::default();
        let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(matches!(d.push(&huge), Err(FrameError::TooLarge(_))));
    }

    /// The connection state machine: queued until connected, the Hello and
    /// early samples on the stream, samples as datagrams once answered.
    #[test]
    fn the_client_link_sends_samples_unreliably_only_after_the_answer() {
        let mut l = ClientLink::new();
        assert_eq!(l.status(), &LinkStatus::Connecting);
        l.send(hello());
        l.send(sample(1));
        assert!(
            l.take_outgoing().is_empty(),
            "nothing before the transport is up"
        );
        l.on_connected(1.0, Some(1200));
        assert_eq!(l.status(), &LinkStatus::Connected { datagrams: true });
        let out = l.take_outgoing();
        let mut d = FrameDecoder::default();
        let frames: Vec<Vec<u8>> = out
            .iter()
            .flat_map(|o| match o {
                Outgoing::Stream(b) => d.push(b).unwrap(),
                Outgoing::Datagram(_) => panic!("no datagram before the answer"),
            })
            .collect();
        assert_eq!(frames, vec![hello(), sample(1)]);
        // Before the answer a sample still goes on the stream.
        l.send(sample(2));
        assert!(matches!(&l.take_outgoing()[..], [Outgoing::Stream(_)]));
        // The Welcome arrives in two pieces.
        let welcome = encode(&server_message(S::Welcome(v1::Welcome {
            rider_id: 3,
            ..Default::default()
        })));
        let frame = encode_frame(&welcome).unwrap();
        l.on_stream_data(1.02, &frame[..3]);
        assert!(l.receive().is_empty());
        l.on_stream_data(1.04, &frame[3..]);
        assert_eq!(l.receive(), vec![welcome]);
        assert!((l.rtt_s().unwrap() - 0.04).abs() < 1e-9);
        // Now samples go as datagrams, reliable messages on the stream.
        l.send(sample(3));
        let steer = encode(&client_message(C::Steer(v1::SteerIntent::default())));
        l.send(steer.clone());
        assert_eq!(
            l.take_outgoing(),
            vec![
                Outgoing::Datagram(sample(3)),
                Outgoing::Stream(encode_frame(&steer).unwrap())
            ]
        );
        // Snapshots come as datagrams.
        let snap = encode(&server_message(S::Snapshot(v1::Snapshot::default())));
        l.on_datagram(1.1, snap.clone());
        assert_eq!(l.receive_timed(), vec![(1.1, snap)]);
        // The transport's RTT wins over the handshake estimate.
        l.set_transport_rtt(Some(0.003));
        assert_eq!(l.rtt_s(), Some(0.003));
        let c = l.counters();
        assert_eq!((c.frames_out, c.datagrams_out), (4, 1));
        assert_eq!((c.frames_in, c.datagrams_in), (1, 1));
        // Closed: nothing more goes out or comes in; the first reason stays.
        l.on_closed("the server closed the connection: bye");
        l.on_closed("later");
        l.send(sample(4));
        l.on_datagram(2.0, vec![1]);
        assert!(l.take_outgoing().is_empty() && l.receive().is_empty());
        assert_eq!(
            l.status(),
            &LinkStatus::Closed("the server closed the connection: bye".into())
        );
        assert_eq!(l.counters().dropped, 1);
    }

    #[test]
    fn without_datagrams_everything_goes_on_the_stream() {
        let mut l = ClientLink::new();
        l.on_connected(0.0, None);
        assert_eq!(l.status(), &LinkStatus::Connected { datagrams: false });
        l.on_stream_data(0.1, &encode_frame(b"\x0a\x00").unwrap());
        l.send(sample(1));
        assert!(matches!(&l.take_outgoing()[..], [Outgoing::Stream(_)]));
        // Too big for the datagram limit: the stream as well.
        let mut l = ClientLink::new();
        l.on_connected(0.0, Some(4));
        l.on_stream_data(0.1, &encode_frame(b"\x0a\x00").unwrap());
        l.send(sample(1));
        assert!(matches!(&l.take_outgoing()[..], [Outgoing::Stream(_)]));
    }

    #[test]
    fn a_broken_stream_closes_the_link() {
        let mut l = ClientLink::new();
        l.on_connected(0.0, Some(1200));
        l.on_stream_data(0.1, &((MAX_FRAME + 1) as u32).to_le_bytes());
        assert!(matches!(l.status(), LinkStatus::Closed(r) if r.contains("bad frame")));
        let mut l = ClientLink::new();
        l.on_connected(0.0, Some(1200));
        l.on_stream_data(0.1, &[9, 0, 0, 0, 1]);
        l.on_stream_end();
        assert!(matches!(l.status(), LinkStatus::Closed(r) if r.contains("inside a frame")));
        let mut l = ClientLink::new();
        l.on_stream_end();
        assert_eq!(
            l.status(),
            &LinkStatus::Closed("the server closed the stream".into())
        );
        // A connection that never came up.
        let mut l = ClientLink::new();
        l.send(hello());
        l.on_closed("cannot connect: refused");
        l.on_connected(1.0, Some(1200));
        assert!(l.is_closed() && l.take_outgoing().is_empty());
    }
}
