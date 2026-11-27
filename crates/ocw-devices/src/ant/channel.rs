//! One ANT channel's state machine (spec 3.4.1 *Setup of one receiving
//! channel*): the setup commands are sent one at a time, each after the
//! stick's response to the one before (a failed response ends the setup
//! with [`ChannelEvent::Failed`]); then searching → tracking ↔ lost →
//! closed. Pure: the stick core feeds it the stick's messages and the time
//! and sends what [`Channel::next_message`] gives.

use std::collections::VecDeque;
use std::time::Duration;

use super::codec::{CHANNEL_TYPE_RECEIVE, Incoming, Message, code, id};
use super::{ANT_PLUS_FREQUENCY, AntProfile, ChannelId};

/// Entries an inclusion / exclusion list holds on ANT sticks.
pub const MAX_LIST: usize = 4;
/// A command without a response for this long failed.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How a channel searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConfig {
    /// What to search for (device number / transmission type 0 = any).
    pub search: ChannelId,
    /// 1/32768 s.
    pub period: u16,
    /// MHz above 2400.
    pub rf: u8,
    pub network: u8,
    /// × 2.5 s (searching without blocking the other channels).
    pub low_priority_timeout: u8,
    /// × 2.5 s (0: no high-priority search).
    pub high_priority_timeout: u8,
    /// Sensors not to pair with (an exclusion list, at most [`MAX_LIST`]).
    pub exclude: Vec<ChannelId>,
}

impl ChannelConfig {
    /// An ANT+ receiving channel for `profile` (device number 0 = any):
    /// its period, 2457 MHz, network 0, low priority search 25 s, no high
    /// priority search (spec 3.4.1, step 8).
    pub fn ant_plus(profile: AntProfile, device_number: u16, trans_type: u8) -> Self {
        Self {
            search: ChannelId::new(device_number, profile.device_type(), trans_type),
            period: profile.period(),
            rf: ANT_PLUS_FREQUENCY,
            network: 0,
            low_priority_timeout: 10,
            high_priority_timeout: 0,
            exclude: Vec::new(),
        }
    }
}

/// Where a channel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Not assigned on the stick.
    Unassigned,
    /// Setup commands under way.
    Configuring,
    Searching,
    /// Receiving a sensor.
    Tracking,
    Closing,
    /// Assigned but closed (search timeout, or closed by us).
    Closed,
    /// A command failed; the channel is unusable until unassigned.
    Failed,
}

/// What happened on a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEvent {
    /// Opened: searching.
    Searching,
    /// Paired with this sensor (from the extended data, the channel's own
    /// ID, or a requested channel ID).
    Found(ChannelId),
    /// A page.
    Data { payload: [u8; 8], rssi: Option<i8> },
    /// One message missed (the value is kept).
    Missed,
    /// The sensor is gone; the channel searches for it again.
    Lost,
    /// The channel closed (search timeout, or asked to).
    Closed,
    /// Unassigned: the channel number is free.
    Unassigned,
    /// The command `command` failed with `code`.
    Failed { command: u8, code: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    /// A channel response to this command.
    Response(u8),
    /// This message (a request's answer).
    Message(u8),
}

/// One channel.
#[derive(Debug, Clone)]
pub struct Channel {
    pub number: u8,
    pub config: ChannelConfig,
    state: ChannelState,
    queue: VecDeque<Message>,
    waiting: Option<(Wait, Duration)>,
    found: Option<ChannelId>,
}

impl Channel {
    pub fn new(number: u8, config: ChannelConfig) -> Self {
        Self {
            number,
            config,
            state: ChannelState::Unassigned,
            queue: VecDeque::new(),
            waiting: None,
            found: None,
        }
    }

    pub fn state(&self) -> ChannelState {
        self.state
    }

    /// The sensor it is paired with.
    pub fn found(&self) -> Option<ChannelId> {
        self.found
    }

    /// Commands queued or waiting for their answer.
    pub fn busy(&self) -> bool {
        self.waiting.is_some() || !self.queue.is_empty()
    }

    fn list_messages(&mut self) {
        let n = self.number;
        let list: Vec<ChannelId> = self.config.exclude.iter().take(MAX_LIST).copied().collect();
        for (i, cid) in list.iter().enumerate() {
            self.queue
                .push_back(Message::add_channel_id_to_list(n, *cid, i as u8));
        }
        self.queue
            .push_back(Message::config_id_list(n, list.len() as u8, true));
    }

    /// Assigns, configures and opens it (spec 3.4.1, steps 4 – 10).
    pub fn open(&mut self) {
        let n = self.number;
        let c = self.config.clone();
        self.queue.clear();
        self.queue.extend([
            Message::assign_channel(n, CHANNEL_TYPE_RECEIVE, c.network),
            Message::channel_id(n, c.search),
            Message::channel_period(n, c.period),
            Message::rf_frequency(n, c.rf),
            Message::low_priority_search_timeout(n, c.low_priority_timeout),
            Message::search_timeout(n, c.high_priority_timeout),
        ]);
        if !c.exclude.is_empty() {
            self.list_messages();
        }
        self.queue.push_back(Message::open_channel(n));
        self.found = None;
        self.state = ChannelState::Configuring;
    }

    /// Opens it again after it closed; `exclude` replaces the exclusion
    /// list (the scan's next round).
    pub fn reopen(&mut self, exclude: Option<Vec<ChannelId>>) {
        if let Some(list) = exclude {
            self.config.exclude = list;
            self.list_messages();
        }
        self.queue.push_back(Message::open_channel(self.number));
        self.found = None;
        self.state = ChannelState::Configuring;
    }

    /// Closes it (it answers with [`ChannelEvent::Closed`]).
    pub fn close(&mut self) {
        match self.state {
            ChannelState::Searching | ChannelState::Tracking | ChannelState::Configuring => {
                // Setup not yet sent is dropped; an open already on its way
                // is answered first.
                self.queue.retain(|m| m.id != id::OPEN_CHANNEL);
                self.queue.push_back(Message::close_channel(self.number));
                self.state = ChannelState::Closing;
            }
            _ => {}
        }
    }

    /// Frees the channel number on the stick (after it closed or failed).
    pub fn unassign(&mut self) {
        self.queue.clear();
        self.queue.push_back(Message::unassign_channel(self.number));
    }

    /// The next command to send, when the last one was answered.
    pub fn next_message(&mut self, now: Duration) -> Option<Message> {
        if self.waiting.is_some() {
            return None;
        }
        let m = self.queue.pop_front()?;
        let wait = if m.id == id::REQUEST {
            Wait::Message(m.data.get(1).copied().unwrap_or(0))
        } else {
            Wait::Response(m.id)
        };
        self.waiting = Some((wait, now));
        Some(m)
    }

    /// A message from the stick for this channel.
    pub fn on_incoming(&mut self, inc: &Incoming, _now: Duration) -> Vec<ChannelEvent> {
        let mut out = Vec::new();
        match *inc {
            Incoming::Response { to, code, .. } => {
                if self.waiting.map(|(w, _)| w) == Some(Wait::Response(to)) {
                    self.waiting = None;
                }
                if code == code::RESPONSE_NO_ERROR {
                    match to {
                        id::OPEN_CHANNEL if self.state != ChannelState::Closing => {
                            self.state = ChannelState::Searching;
                            out.push(ChannelEvent::Searching);
                        }
                        id::UNASSIGN_CHANNEL => {
                            self.state = ChannelState::Unassigned;
                            out.push(ChannelEvent::Unassigned);
                        }
                        _ => {}
                    }
                } else if to == id::CLOSE_CHANNEL && code == code::CHANNEL_IN_WRONG_STATE {
                    // It had closed by itself in the meantime.
                    self.state = ChannelState::Closed;
                    out.push(ChannelEvent::Closed);
                } else if to == id::REQUEST {
                    // No channel ID to give: carry on without it.
                } else if to == id::UNASSIGN_CHANNEL {
                    self.state = ChannelState::Unassigned;
                    out.push(ChannelEvent::Unassigned);
                } else {
                    self.queue.clear();
                    self.state = ChannelState::Failed;
                    out.push(ChannelEvent::Failed { command: to, code });
                }
            }
            Incoming::Event { code, .. } => match code {
                code::EVENT_RX_FAIL if self.state == ChannelState::Tracking => {
                    out.push(ChannelEvent::Missed);
                }
                code::EVENT_RX_FAIL_GO_TO_SEARCH => {
                    if self.state == ChannelState::Tracking {
                        out.push(ChannelEvent::Lost);
                    }
                    if self.state != ChannelState::Closing {
                        self.state = ChannelState::Searching;
                    }
                }
                code::EVENT_CHANNEL_CLOSED => {
                    if self.waiting.map(|(w, _)| w) == Some(Wait::Response(id::CLOSE_CHANNEL)) {
                        self.waiting = None;
                    }
                    if self.state != ChannelState::Closed {
                        self.state = ChannelState::Closed;
                        out.push(ChannelEvent::Closed);
                    }
                }
                _ => {}
            },
            Incoming::ChannelId { id, .. } => {
                if self.waiting.map(|(w, _)| w) == Some(Wait::Message(id::CHANNEL_ID)) {
                    self.waiting = None;
                }
                self.queue.retain(|m| m.id != id::REQUEST);
                if id.device_number != 0 && self.found.is_none() {
                    self.found = Some(id);
                    out.push(ChannelEvent::Found(id));
                }
            }
            Incoming::Data { payload, ext, .. } => {
                if matches!(
                    self.state,
                    ChannelState::Closing | ChannelState::Closed | ChannelState::Failed
                ) {
                    return out;
                }
                self.state = ChannelState::Tracking;
                if self.found.is_none() {
                    let id = ext.id.or_else(|| {
                        let s = self.config.search;
                        (!s.is_wildcard()).then_some(s)
                    });
                    match id {
                        Some(id) => {
                            self.found = Some(id);
                            out.push(ChannelEvent::Found(id));
                        }
                        None if !self.queue.iter().any(|m| m.id == id::REQUEST)
                            && self.waiting.map(|(w, _)| w)
                                != Some(Wait::Message(id::CHANNEL_ID)) =>
                        {
                            // A stick without extended data: ask.
                            self.queue
                                .push_front(Message::request(self.number, id::CHANNEL_ID));
                        }
                        None => {}
                    }
                }
                out.push(ChannelEvent::Data {
                    payload,
                    rssi: ext.rssi,
                });
            }
            _ => {}
        }
        out
    }

    /// A command without an answer for [`RESPONSE_TIMEOUT`] failed.
    pub fn on_tick(&mut self, now: Duration) -> Vec<ChannelEvent> {
        match self.waiting {
            Some((w, sent)) if now.saturating_sub(sent) > RESPONSE_TIMEOUT => {
                self.waiting = None;
                let command = match w {
                    Wait::Response(c) => c,
                    Wait::Message(_) => id::REQUEST,
                };
                if command == id::REQUEST {
                    return Vec::new();
                }
                self.queue.clear();
                self.state = ChannelState::Failed;
                vec![ChannelEvent::Failed {
                    command,
                    code: 0xFF,
                }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ant::codec::Extended;

    const T: Duration = Duration::from_millis(10);

    fn ok(to: u8) -> Incoming {
        Incoming::Response {
            channel: 0,
            to,
            code: code::RESPONSE_NO_ERROR,
        }
    }

    fn event(code: u8) -> Incoming {
        Incoming::Event { channel: 0, code }
    }

    fn data(payload: [u8; 8], id: Option<ChannelId>) -> Incoming {
        Incoming::Data {
            channel: 0,
            payload,
            ext: Extended { id, rssi: None },
            acknowledged: false,
        }
    }

    /// Answers every command with "no error" until the channel is open.
    fn set_up(c: &mut Channel) -> Vec<u8> {
        let mut sent = Vec::new();
        while let Some(m) = c.next_message(T) {
            assert!(c.next_message(T).is_none(), "one command at a time");
            sent.push(m.id);
            c.on_incoming(&ok(m.id), T);
        }
        sent
    }

    #[test]
    fn the_setup_waits_for_each_response() {
        let mut c = Channel::new(0, ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0));
        c.open();
        let sent = set_up(&mut c);
        assert_eq!(
            sent,
            [
                id::ASSIGN_CHANNEL,
                id::CHANNEL_ID,
                id::CHANNEL_PERIOD,
                id::RF_FREQUENCY,
                id::LOW_PRIORITY_SEARCH_TIMEOUT,
                id::SEARCH_TIMEOUT,
                id::OPEN_CHANNEL
            ]
        );
        assert_eq!(c.state(), ChannelState::Searching);
        assert!(!c.busy());
    }

    #[test]
    fn a_failed_response_ends_the_setup() {
        let mut c = Channel::new(1, ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0));
        c.open();
        let m = c.next_message(T).unwrap();
        let ev = c.on_incoming(
            &Incoming::Response {
                channel: 1,
                to: m.id,
                code: code::CHANNEL_IN_WRONG_STATE,
            },
            T,
        );
        assert_eq!(
            ev,
            [ChannelEvent::Failed {
                command: id::ASSIGN_CHANNEL,
                code: code::CHANNEL_IN_WRONG_STATE
            }]
        );
        assert_eq!(c.state(), ChannelState::Failed);
        assert!(c.next_message(T).is_none());
        // No answer at all: failed after the timeout.
        let mut c = Channel::new(1, ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0));
        c.open();
        c.next_message(T).unwrap();
        assert!(c.on_tick(T + Duration::from_secs(1)).is_empty());
        assert!(matches!(
            c.on_tick(T + Duration::from_secs(3))[..],
            [ChannelEvent::Failed { .. }]
        ));
    }

    #[test]
    fn search_track_miss_lose_and_find_again() {
        let mut c = Channel::new(0, ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0));
        c.open();
        set_up(&mut c);
        let strap = ChannelId::new(12345, 120, 1);
        let ev = c.on_incoming(&data([0; 8], Some(strap)), T);
        assert_eq!(ev[0], ChannelEvent::Found(strap));
        assert!(matches!(ev[1], ChannelEvent::Data { .. }));
        assert_eq!(c.state(), ChannelState::Tracking);
        assert_eq!(
            c.on_incoming(&event(code::EVENT_RX_FAIL), T),
            [ChannelEvent::Missed]
        );
        assert_eq!(
            c.on_incoming(&event(code::EVENT_RX_FAIL_GO_TO_SEARCH), T),
            [ChannelEvent::Lost]
        );
        assert_eq!(c.state(), ChannelState::Searching);
        // Back: tracking again, the same sensor (no second Found).
        let ev = c.on_incoming(&data([0; 8], Some(strap)), T);
        assert!(matches!(ev[..], [ChannelEvent::Data { .. }]));
        assert_eq!(c.state(), ChannelState::Tracking);
        // Search timeout, then closed.
        assert!(
            c.on_incoming(&event(code::EVENT_RX_SEARCH_TIMEOUT), T)
                .is_empty()
        );
        assert_eq!(
            c.on_incoming(&event(code::EVENT_CHANNEL_CLOSED), T),
            [ChannelEvent::Closed]
        );
        // Reopened: searching again.
        c.reopen(None);
        assert_eq!(set_up(&mut c), [id::OPEN_CHANNEL]);
        assert_eq!(c.state(), ChannelState::Searching);
    }

    #[test]
    fn a_stick_without_extended_data_is_asked_for_the_id() {
        let mut c = Channel::new(2, ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0));
        c.open();
        set_up(&mut c);
        let ev = c.on_incoming(&data([0; 8], None), T);
        assert!(matches!(ev[..], [ChannelEvent::Data { .. }]));
        let m = c.next_message(T).unwrap();
        assert_eq!(m, Message::request(2, id::CHANNEL_ID));
        // Only once.
        c.on_incoming(&data([0; 8], None), T);
        assert!(c.next_message(T).is_none());
        let strap = ChannelId::new(4242, 120, 5);
        assert_eq!(
            c.on_incoming(
                &Incoming::ChannelId {
                    channel: 2,
                    id: strap
                },
                T
            ),
            [ChannelEvent::Found(strap)]
        );
        assert!(!c.busy());
        // A channel for one strap knows it without asking.
        let mut c = Channel::new(3, ChannelConfig::ant_plus(AntProfile::HeartRate, 4242, 5));
        c.open();
        set_up(&mut c);
        assert_eq!(
            c.on_incoming(&data([0; 8], None), T)[0],
            ChannelEvent::Found(strap)
        );
    }

    #[test]
    fn exclusion_lists_and_closing() {
        let mut cfg = ChannelConfig::ant_plus(AntProfile::HeartRate, 0, 0);
        cfg.exclude = vec![ChannelId::new(1, 120, 1)];
        let mut c = Channel::new(0, cfg);
        c.open();
        let sent = set_up(&mut c);
        assert_eq!(
            &sent[6..],
            [
                id::ADD_CHANNEL_ID_TO_LIST,
                id::CONFIG_ID_LIST,
                id::OPEN_CHANNEL
            ]
        );
        c.close();
        assert_eq!(c.next_message(T).unwrap().id, id::CLOSE_CHANNEL);
        c.on_incoming(&ok(id::CLOSE_CHANNEL), T);
        assert_eq!(
            c.on_incoming(&event(code::EVENT_CHANNEL_CLOSED), T),
            [ChannelEvent::Closed]
        );
        // The next round excludes two.
        c.reopen(Some(vec![
            ChannelId::new(1, 120, 1),
            ChannelId::new(2, 120, 1),
        ]));
        assert_eq!(
            set_up(&mut c),
            [
                id::ADD_CHANNEL_ID_TO_LIST,
                id::ADD_CHANNEL_ID_TO_LIST,
                id::CONFIG_ID_LIST,
                id::OPEN_CHANNEL
            ]
        );
        // Closing a channel that closed by itself meanwhile.
        c.close();
        c.next_message(T).unwrap();
        let ev = c.on_incoming(
            &Incoming::Response {
                channel: 0,
                to: id::CLOSE_CHANNEL,
                code: code::CHANNEL_IN_WRONG_STATE,
            },
            T,
        );
        assert_eq!(ev, [ChannelEvent::Closed]);
        c.unassign();
        assert_eq!(c.next_message(T).unwrap().id, id::UNASSIGN_CHANNEL);
        assert_eq!(
            c.on_incoming(&ok(id::UNASSIGN_CHANNEL), T),
            [ChannelEvent::Unassigned]
        );
    }
}
