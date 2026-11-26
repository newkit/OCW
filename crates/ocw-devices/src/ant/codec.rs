//! ANT serial messages (ANT Message Protocol and Usage; the same over USB
//! bulk endpoints and serial ports): `0xA4` (sync), the data's length,
//! the message id, the data, and a checksum (XOR of every byte before it,
//! sync included). [`Decoder`] hunts for the sync byte and drops a message
//! whose checksum fails, so a garbled byte costs one message, not the
//! stream.

use std::fmt;

use super::{ChannelId, NetworkKey};

/// The first byte of every message.
pub const SYNC: u8 = 0xA4;
/// The longest data we accept (a broadcast with every extended field is
/// 18 bytes; the protocol allows a little more).
pub const MAX_DATA: usize = 41;

/// Message ids (ANT Message Protocol, section 9).
pub mod id {
    /// Channel response or event (data: channel, message id, code;
    /// message id 1 = an RF event).
    pub const CHANNEL_EVENT: u8 = 0x40;
    pub const UNASSIGN_CHANNEL: u8 = 0x41;
    pub const ASSIGN_CHANNEL: u8 = 0x42;
    pub const CHANNEL_PERIOD: u8 = 0x43;
    pub const SEARCH_TIMEOUT: u8 = 0x44;
    pub const RF_FREQUENCY: u8 = 0x45;
    pub const SET_NETWORK_KEY: u8 = 0x46;
    pub const RESET_SYSTEM: u8 = 0x4A;
    pub const OPEN_CHANNEL: u8 = 0x4B;
    pub const CLOSE_CHANNEL: u8 = 0x4C;
    pub const REQUEST: u8 = 0x4D;
    pub const BROADCAST_DATA: u8 = 0x4E;
    pub const ACKNOWLEDGED_DATA: u8 = 0x4F;
    pub const BURST_DATA: u8 = 0x50;
    pub const CHANNEL_ID: u8 = 0x51;
    pub const CHANNEL_STATUS: u8 = 0x52;
    pub const CAPABILITIES: u8 = 0x54;
    pub const ADD_CHANNEL_ID_TO_LIST: u8 = 0x59;
    pub const CONFIG_ID_LIST: u8 = 0x5A;
    pub const LOW_PRIORITY_SEARCH_TIMEOUT: u8 = 0x63;
    pub const LIB_CONFIG: u8 = 0x6E;
    pub const STARTUP: u8 = 0x6F;
    pub const SERIAL_ERROR: u8 = 0xAE;
    /// In a channel event: an RF event, not a response.
    pub const RF_EVENT: u8 = 0x01;
}

/// Response and event codes (in [`id::CHANNEL_EVENT`] messages).
pub mod code {
    pub const RESPONSE_NO_ERROR: u8 = 0x00;
    pub const EVENT_RX_SEARCH_TIMEOUT: u8 = 0x01;
    pub const EVENT_RX_FAIL: u8 = 0x02;
    pub const EVENT_TX: u8 = 0x03;
    pub const EVENT_TRANSFER_RX_FAILED: u8 = 0x04;
    pub const EVENT_TRANSFER_TX_COMPLETED: u8 = 0x05;
    pub const EVENT_TRANSFER_TX_FAILED: u8 = 0x06;
    pub const EVENT_CHANNEL_CLOSED: u8 = 0x07;
    pub const EVENT_RX_FAIL_GO_TO_SEARCH: u8 = 0x08;
    pub const EVENT_CHANNEL_COLLISION: u8 = 0x09;
    pub const CHANNEL_IN_WRONG_STATE: u8 = 0x15;
    pub const CHANNEL_NOT_OPENED: u8 = 0x16;
    pub const CHANNEL_ID_NOT_SET: u8 = 0x18;
    pub const INVALID_MESSAGE: u8 = 0x28;
    pub const INVALID_NETWORK_NUMBER: u8 = 0x29;
}

/// Channel types (assign channel).
pub const CHANNEL_TYPE_RECEIVE: u8 = 0x00;

/// Lib config flags: extended data on every received message.
pub mod lib_config {
    /// The sender's channel ID (device number, type, transmission type).
    pub const CHANNEL_ID: u8 = 0x80;
    /// The signal strength.
    pub const RSSI: u8 = 0x40;
    /// The stick's receive time stamp.
    pub const TIMESTAMP: u8 = 0x20;
}

/// One message.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    pub id: u8,
    pub data: Vec<u8>,
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The network key never reaches a log.
        if self.id == id::SET_NETWORK_KEY {
            return write!(f, "Message(0x46 set network key, <hidden>)");
        }
        write!(f, "Message(0x{:02X}, {:02X?})", self.id, self.data)
    }
}

/// XOR of the bytes.
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a ^ b)
}

impl Message {
    pub fn new(id: u8, data: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            data: data.into(),
        }
    }

    /// The bytes on the wire.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() + 4);
        out.push(SYNC);
        out.push(self.data.len() as u8);
        out.push(self.id);
        out.extend_from_slice(&self.data);
        out.push(checksum(&out));
        out
    }

    pub fn reset() -> Self {
        Self::new(id::RESET_SYSTEM, [0])
    }

    /// Asks for a message (capabilities `0x54`, a channel's ID `0x51`).
    pub fn request(channel: u8, what: u8) -> Self {
        Self::new(id::REQUEST, [channel, what])
    }

    pub fn set_network_key(network: u8, key: &NetworkKey) -> Self {
        let mut d = vec![network];
        d.extend_from_slice(key.bytes());
        Self::new(id::SET_NETWORK_KEY, d)
    }

    pub fn assign_channel(channel: u8, channel_type: u8, network: u8) -> Self {
        Self::new(id::ASSIGN_CHANNEL, [channel, channel_type, network])
    }

    pub fn unassign_channel(channel: u8) -> Self {
        Self::new(id::UNASSIGN_CHANNEL, [channel])
    }

    pub fn channel_id(channel: u8, cid: ChannelId) -> Self {
        let mut d = vec![channel];
        d.extend_from_slice(&cid.to_bytes());
        Self::new(id::CHANNEL_ID, d)
    }

    pub fn channel_period(channel: u8, period: u16) -> Self {
        let [lo, hi] = period.to_le_bytes();
        Self::new(id::CHANNEL_PERIOD, [channel, lo, hi])
    }

    /// High priority search timeout (× 2.5 s; 0 = none, 255 = forever).
    pub fn search_timeout(channel: u8, t: u8) -> Self {
        Self::new(id::SEARCH_TIMEOUT, [channel, t])
    }

    /// Low priority search timeout (× 2.5 s): searching without blocking
    /// the other channels.
    pub fn low_priority_search_timeout(channel: u8, t: u8) -> Self {
        Self::new(id::LOW_PRIORITY_SEARCH_TIMEOUT, [channel, t])
    }

    pub fn rf_frequency(channel: u8, f: u8) -> Self {
        Self::new(id::RF_FREQUENCY, [channel, f])
    }

    pub fn lib_config(flags: u8) -> Self {
        Self::new(id::LIB_CONFIG, [0, flags])
    }

    pub fn open_channel(channel: u8) -> Self {
        Self::new(id::OPEN_CHANNEL, [channel])
    }

    pub fn close_channel(channel: u8) -> Self {
        Self::new(id::CLOSE_CHANNEL, [channel])
    }

    /// Puts a channel ID into the channel's inclusion / exclusion list.
    pub fn add_channel_id_to_list(channel: u8, cid: ChannelId, index: u8) -> Self {
        let mut d = vec![channel];
        d.extend_from_slice(&cid.to_bytes());
        d.push(index);
        Self::new(id::ADD_CHANNEL_ID_TO_LIST, d)
    }

    /// Uses the first `size` list entries (0 = no list) as an exclusion
    /// list (`exclude`) or an inclusion list.
    pub fn config_id_list(channel: u8, size: u8, exclude: bool) -> Self {
        Self::new(id::CONFIG_ID_LIST, [channel, size, u8::from(exclude)])
    }

    pub fn broadcast(channel: u8, payload: [u8; 8]) -> Self {
        let mut d = vec![channel];
        d.extend_from_slice(&payload);
        Self::new(id::BROADCAST_DATA, d)
    }

    pub fn acknowledged(channel: u8, payload: [u8; 8]) -> Self {
        let mut d = vec![channel];
        d.extend_from_slice(&payload);
        Self::new(id::ACKNOWLEDGED_DATA, d)
    }

    /// The channel a channel message is about (its first data byte).
    pub fn channel(&self) -> Option<u8> {
        match self.id {
            id::RESET_SYSTEM
            | id::SET_NETWORK_KEY
            | id::LIB_CONFIG
            | id::STARTUP
            | id::CAPABILITIES
            | id::SERIAL_ERROR => None,
            _ => self.data.first().copied(),
        }
    }

    /// What the stick said, read.
    pub fn parse(&self) -> Incoming {
        let d = &self.data;
        match self.id {
            id::CHANNEL_EVENT if d.len() >= 3 => {
                if d[1] == id::RF_EVENT {
                    Incoming::Event {
                        channel: d[0],
                        code: d[2],
                    }
                } else {
                    Incoming::Response {
                        channel: d[0],
                        to: d[1],
                        code: d[2],
                    }
                }
            }
            id::STARTUP => Incoming::StartUp(d.first().copied().unwrap_or(0)),
            id::CAPABILITIES if d.len() >= 2 => Incoming::Capabilities {
                channels: d[0],
                networks: d[1],
                // Advanced options 2, bit 7: extended messages.
                extended: d.get(4).map(|b| b & 0x80 != 0),
            },
            id::CHANNEL_ID if d.len() >= 5 => Incoming::ChannelId {
                channel: d[0],
                id: ChannelId::from_bytes(&d[1..]).expect("4 bytes"),
            },
            id::BROADCAST_DATA | id::ACKNOWLEDGED_DATA | id::BURST_DATA if d.len() >= 9 => {
                let mut payload = [0u8; 8];
                payload.copy_from_slice(&d[1..9]);
                Incoming::Data {
                    channel: d[0] & 0x1F,
                    payload,
                    ext: Extended::parse(&d[9..]),
                    acknowledged: self.id == id::ACKNOWLEDGED_DATA,
                }
            }
            id::SERIAL_ERROR => Incoming::SerialError,
            _ => Incoming::Other(self.clone()),
        }
    }
}

/// Extended data after a received payload (lib config): the flag byte,
/// then the fields its bits name, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extended {
    pub id: Option<ChannelId>,
    pub rssi: Option<i8>,
}

impl Extended {
    fn parse(b: &[u8]) -> Self {
        let Some((&flags, mut rest)) = b.split_first() else {
            return Self::default();
        };
        let mut out = Self::default();
        if flags & lib_config::CHANNEL_ID != 0 && rest.len() >= 4 {
            out.id = ChannelId::from_bytes(rest);
            rest = &rest[4..];
        }
        if flags & lib_config::RSSI != 0 && rest.len() >= 3 {
            // Measurement type, RSSI (dBm, signed), threshold.
            out.rssi = Some(rest[1] as i8);
        }
        out
    }

    /// Appends the flag byte and fields (the simulated stick).
    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut flags = 0;
        if self.id.is_some() {
            flags |= lib_config::CHANNEL_ID;
        }
        if self.rssi.is_some() {
            flags |= lib_config::RSSI;
        }
        if flags == 0 {
            return;
        }
        out.push(flags);
        if let Some(id) = self.id {
            out.extend_from_slice(&id.to_bytes());
        }
        if let Some(r) = self.rssi {
            out.extend_from_slice(&[0x20, r as u8, 0x00]);
        }
    }
}

/// A message from the stick, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// The answer to a command (`to`: its message id).
    Response {
        channel: u8,
        to: u8,
        code: u8,
    },
    /// An RF event of a channel ([`code`]).
    Event {
        channel: u8,
        code: u8,
    },
    /// After a reset (the reason bits).
    StartUp(u8),
    Capabilities {
        channels: u8,
        networks: u8,
        extended: Option<bool>,
    },
    /// The answer to a request for a channel's ID.
    ChannelId {
        channel: u8,
        id: ChannelId,
    },
    /// 8 bytes of a sensor's page.
    Data {
        channel: u8,
        payload: [u8; 8],
        ext: Extended,
        acknowledged: bool,
    },
    /// The stick could not read what we sent.
    SerialError,
    Other(Message),
}

/// Reads messages out of a byte stream: split reads, several messages in
/// one read, and garbage in between.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Messages dropped (checksum, length) – for the log.
    pub dropped: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes; returns the complete messages among everything so far.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Message> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // Hunt for the sync byte.
            match self.buf.iter().position(|b| *b == SYNC) {
                Some(0) => {}
                Some(i) => {
                    self.buf.drain(..i);
                }
                None => {
                    self.buf.clear();
                    break;
                }
            }
            if self.buf.len() < 2 {
                break;
            }
            let len = usize::from(self.buf[1]);
            if len > MAX_DATA {
                self.dropped += 1;
                self.buf.drain(..1);
                continue;
            }
            let total = len + 4;
            if self.buf.len() < total {
                break;
            }
            if checksum(&self.buf[..total - 1]) == self.buf[total - 1] {
                out.push(Message::new(self.buf[2], self.buf[3..total - 1].to_vec()));
                self.buf.drain(..total);
            } else {
                // Not a message here: look for the next sync byte.
                self.dropped += 1;
                self.buf.drain(..1);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_messages_are_byte_exact() {
        // Values of the ANT protocol document and the ANT+ HR profile.
        let hr = ChannelId::new(0, 120, 0);
        assert_eq!(
            Message::assign_channel(0, CHANNEL_TYPE_RECEIVE, 0).encode(),
            [0xA4, 0x03, 0x42, 0x00, 0x00, 0x00, 0xE5]
        );
        assert_eq!(
            Message::channel_id(0, hr).encode(),
            [0xA4, 0x05, 0x51, 0x00, 0x00, 0x00, 0x78, 0x00, 0x88]
        );
        // 8070 = 0x1F86.
        assert_eq!(
            Message::channel_period(0, 8070).encode(),
            [0xA4, 0x03, 0x43, 0x00, 0x86, 0x1F, 0x7D]
        );
        assert_eq!(
            Message::rf_frequency(0, 57).encode(),
            [0xA4, 0x02, 0x45, 0x00, 0x39, 0xDA]
        );
        assert_eq!(
            Message::low_priority_search_timeout(0, 10).encode(),
            [0xA4, 0x02, 0x63, 0x00, 0x0A, 0xCF]
        );
        assert_eq!(
            Message::search_timeout(0, 0).encode(),
            [0xA4, 0x02, 0x44, 0x00, 0x00, 0xE2]
        );
        assert_eq!(
            Message::lib_config(0x80).encode(),
            [0xA4, 0x02, 0x6E, 0x00, 0x80, 0x48]
        );
        assert_eq!(Message::reset().encode(), [0xA4, 0x01, 0x4A, 0x00, 0xEF]);
        assert_eq!(
            Message::open_channel(0).encode(),
            [0xA4, 0x01, 0x4B, 0x00, 0xEE]
        );
        let key = Message::set_network_key(0, &NetworkKey::TEST);
        assert_eq!(key.data.len(), 9);
        assert!(!format!("{key:?}").contains("1B"), "{key:?}");
    }

    #[test]
    fn every_message_we_send_round_trips() {
        let cid = ChannelId::new(12345, 120, 1);
        let all = [
            Message::reset(),
            Message::request(0, id::CAPABILITIES),
            Message::set_network_key(0, &NetworkKey::TEST),
            Message::assign_channel(3, 0, 0),
            Message::unassign_channel(3),
            Message::channel_id(3, cid),
            Message::channel_period(3, 8182),
            Message::search_timeout(3, 0),
            Message::low_priority_search_timeout(3, 10),
            Message::rf_frequency(3, 57),
            Message::lib_config(0xC0),
            Message::open_channel(3),
            Message::close_channel(3),
            Message::add_channel_id_to_list(3, cid, 2),
            Message::config_id_list(3, 3, true),
            Message::broadcast(3, [1, 2, 3, 4, 5, 6, 7, 8]),
            Message::acknowledged(3, [9; 8]),
        ];
        let mut d = Decoder::new();
        for m in &all {
            assert_eq!(d.push(&m.encode()), std::slice::from_ref(m));
        }
        // All in one read.
        let stream: Vec<u8> = all.iter().flat_map(Message::encode).collect();
        assert_eq!(d.push(&stream), all);
        assert_eq!(d.dropped, 0);
    }

    #[test]
    fn the_decoder_resyncs_after_garbage_and_split_reads() {
        let a = Message::broadcast(0, [0, 0, 0, 0, 0x10, 0x20, 7, 72]).encode();
        let b = Message::new(id::CHANNEL_EVENT, [0, 1, code::EVENT_RX_FAIL]).encode();
        let mut d = Decoder::new();
        // Garbage, then a split message.
        assert!(d.push(&[0x00, 0x13, 0x37]).is_empty());
        assert!(d.push(&a[..5]).is_empty());
        assert_eq!(d.push(&a[5..]).len(), 1);
        // A garbled checksum costs that message only.
        let mut bad = a.clone();
        *bad.last_mut().unwrap() ^= 0xFF;
        let mut stream = bad;
        stream.extend_from_slice(&b);
        let got = d.push(&stream);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].parse(),
            Incoming::Event {
                channel: 0,
                code: code::EVENT_RX_FAIL
            }
        );
        assert!(d.dropped >= 1);
        // An impossible length is skipped, not waited for.
        let mut stream = vec![SYNC, 0xF0, 0x00];
        stream.extend_from_slice(&a);
        assert_eq!(d.push(&stream).len(), 1);
        // USB reads padded with zeros.
        let mut padded = a.clone();
        padded.extend_from_slice(&[0; 20]);
        assert_eq!(d.push(&padded).len(), 1);
    }

    #[test]
    fn incoming_messages_are_read() {
        let r = Message::new(id::CHANNEL_EVENT, [2, id::OPEN_CHANNEL, 0]).parse();
        assert_eq!(
            r,
            Incoming::Response {
                channel: 2,
                to: id::OPEN_CHANNEL,
                code: code::RESPONSE_NO_ERROR
            }
        );
        let caps = Message::new(id::CAPABILITIES, [8, 8, 0, 0xBA, 0xDF, 0]).parse();
        assert_eq!(
            caps,
            Incoming::Capabilities {
                channels: 8,
                networks: 8,
                extended: Some(true)
            }
        );
        // A broadcast with the sender's ID and the signal.
        let mut d = vec![1, 4, 0xFF, 0x12, 0x34, 0x00, 0x04, 0x42, 72];
        Extended {
            id: Some(ChannelId::new(12345, 120, 1)),
            rssi: Some(-61),
        }
        .encode(&mut d);
        let Incoming::Data {
            channel,
            payload,
            ext,
            ..
        } = Message::new(id::BROADCAST_DATA, d).parse()
        else {
            panic!()
        };
        assert_eq!(channel, 1);
        assert_eq!(payload[7], 72);
        assert_eq!(ext.id, Some(ChannelId::new(12345, 120, 1)));
        assert_eq!(ext.rssi, Some(-61));
        assert_eq!(
            Message::new(id::STARTUP, [0x20]).parse(),
            Incoming::StartUp(0x20)
        );
    }
}
