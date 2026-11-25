//! The game connection over WebTransport in the browser (spec §14 Phase 10
//! B3; wasm32 only, kept thin): `ocw_server::wire::ClientLink` (unit-tested
//! natively, with the session's fake transport) decides what goes how –
//! the Hello and early samples on the stream, samples as datagrams after
//! the Welcome, frames on the stream – and this only moves the bytes.
//!
//! The connection itself is `crate::netconn`'s: read in the network worker
//! where the browser lets it (10G *B3 worker*), so each message carries
//! the time it came off the network, else on the page's thread as before.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ocw_server::wire::{ClientLink, Outgoing};

use crate::netconn::{self, ConnOptions, NetEvent};
use crate::session::{RemoteOptions, Transport};

/// The link's clock: the wall clock in seconds (the snapshot timing's
/// interval and age; the handshake RTT). The messages carry their stamps
/// on it.
fn page_s() -> f64 {
    wall_ms() / 1000.0
}

/// The wall clock in Unix ms with sub-ms precision (snapshot ages).
pub fn wall_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.time_origin() + p.now())
}

type SharedLink = Arc<Mutex<ClientLink>>;

fn lock(link: &SharedLink) -> std::sync::MutexGuard<'_, ClientLink> {
    link.lock().expect("not poisoned")
}

/// The session's transport in the browser: connects on its first pump
/// (then it has the link to report to) and writes the link's bytes.
pub struct WebTransportLink {
    options: RemoteOptions,
    id: Option<u32>,
}

impl WebTransportLink {
    pub fn new(options: RemoteOptions) -> Self {
        Self { options, id: None }
    }
}

impl Transport for WebTransportLink {
    fn pump(&mut self, link: &SharedLink, _now_s: f64) {
        let id = match self.id {
            Some(id) => id,
            None => *self.id.insert(connect(&self.options, link.clone())),
        };
        let out = lock(link).take_outgoing();
        if out.is_empty() {
            return;
        }
        let (mut stream, mut datagrams) = (Vec::new(), Vec::new());
        for o in out {
            match o {
                Outgoing::Stream(b) => stream.push(b),
                Outgoing::Datagram(b) => datagrams.push(b),
            }
        }
        netconn::send(id, stream, datagrams);
    }

    fn close(&mut self) {
        if let Some(id) = self.id.take() {
            netconn::close(id, 0, "client quit");
        }
    }

    fn clock_s(&self, _now_s: f64) -> f64 {
        page_s()
    }

    fn reader(&self) -> &'static str {
        self.id
            .and_then(netconn::reader)
            .map_or("", netconn::Reader::as_str)
    }
}

/// Opens the connection; its events go into the link.
fn connect(options: &RemoteOptions, link: SharedLink) -> u32 {
    info!(url = %options.url, hashes = options.cert_hashes.len(), "WebTransport: connecting");
    let opts = ConnOptions {
        url: options.url.clone(),
        hashes: options.cert_hashes.clone(),
        datagrams: true,
        rtt: true,
    };
    netconn::open(opts, move |ev| {
        if let Some(reason) = netconn::server_end(&ev) {
            if matches!(ev, NetEvent::Closed { .. }) {
                warn!(%reason, "WebTransport: closed");
            }
            lock(&link).on_closed(reason);
            return;
        }
        let mut l = lock(&link);
        match ev {
            NetEvent::Connected {
                at_ms,
                max_datagram,
            } => {
                l.on_connected(at_ms / 1000.0, max_datagram);
                info!(max_datagram = ?max_datagram, "WebTransport: connected");
            }
            NetEvent::Stream { at_ms, bytes } if !l.is_closed() => {
                l.on_stream_data(at_ms / 1000.0, &bytes)
            }
            NetEvent::Datagram { at_ms, bytes } if !l.is_closed() => {
                l.on_datagram(at_ms / 1000.0, bytes)
            }
            NetEvent::StreamEnd => l.on_stream_end(),
            NetEvent::Rtt(ms) => l.set_transport_rtt(ms.map(|ms| ms / 1000.0)),
            _ => {}
        }
    })
}
