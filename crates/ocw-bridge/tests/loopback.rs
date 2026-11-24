//! The bridge end to end on loopback (spec 14 Phase 10, 10D.2; 11.4): the
//! desktop's endpoint and a WebTransport client that trusts the server the
//! way a browser does – only by the certificate hashes it derives from the
//! code (`serverCertificateHashes`) – with the browser's `relay::Session`.
//! Pairing, relay order and timestamps, pings, the control round trip,
//! one browser at a time, the same page reconnecting, a wrong code, the
//! clean stop and the latency on the wire.

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use ocw_bridge::PairingCode;
use ocw_bridge::cert::browser_hashes;
use ocw_bridge::host::{BridgeHost, HostEvent, HostOptions};
use ocw_bridge::msg::{Frames, ToBrowser, ToHost, frame};
use ocw_bridge::relay::{KIND, Phase, Session};
use ocw_devices::{DeviceCommand, DeviceEvent, SimulationParameters};

const SECRET: u32 = 0x1A2B_3C4D & 0x3FFF_FFFF;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn host() -> BridgeHost {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    BridgeHost::start(HostOptions {
        secret: SECRET,
        ip: Ipv4Addr::LOCALHOST,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        name: "Test desktop".into(),
        now_unix_s: unix_now(),
    })
    .expect("the bridge starts")
}

/// A browser as the page would be: a session of the code's secret.
struct Browser {
    wt: web_transport_quinn::Session,
    send: web_transport_quinn::SendStream,
    recv: web_transport_quinn::RecvStream,
    frames: Frames,
    session: Session,
    t0: Instant,
}

impl Browser {
    async fn connect(code: &PairingCode, hash_secret: u32, page: &str) -> Result<Self, String> {
        Self::connect_with(code, hash_secret, code.secret, page).await
    }

    async fn connect_with(
        code: &PairingCode,
        hash_secret: u32,
        proof_secret: u32,
        page: &str,
    ) -> Result<Self, String> {
        let hashes = browser_hashes(hash_secret, unix_now())
            .into_iter()
            .map(|h| h.to_vec())
            .collect();
        let client = web_transport_quinn::ClientBuilder::new()
            .with_server_certificate_hashes(hashes)
            .map_err(|e| e.to_string())?;
        let url = url::Url::parse(&code.url()).unwrap();
        let wt = tokio::time::timeout(Duration::from_secs(5), client.connect(url))
            .await
            .map_err(|_| "connect timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        let (send, recv) = wt.open_bi().await.map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let mut b = Self {
            wt,
            send,
            recv,
            frames: Frames::default(),
            session: Session::new(proof_secret, page.into(), "Test browser".into(), 0.0),
            t0,
        };
        for m in b.session.on_open(b.now()) {
            b.send(&m).await;
        }
        Ok(b)
    }

    fn now(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    async fn send(&mut self, m: &ToHost) {
        self.send.write_all(&frame(m)).await.expect("write");
    }

    /// The next message (None: the stream ended).
    async fn next(&mut self) -> Option<ToBrowser> {
        let mut buf = [0u8; 2048];
        loop {
            if let Some(m) = self.frames.next_message::<ToBrowser>() {
                return Some(m.expect("a message"));
            }
            let n = tokio::time::timeout(Duration::from_secs(5), self.recv.read(&mut buf))
                .await
                .expect("a message in time");
            match n {
                Ok(Some(n)) => self.frames.push(&buf[..n]),
                _ => return None,
            }
        }
    }

    /// The next message through the browser's session: its events.
    async fn step(&mut self) -> Option<(ToBrowser, Vec<DeviceEvent>)> {
        let m = self.next().await?;
        let now = self.now();
        let ev = self.session.on_message(m.clone(), now);
        Some((m, ev))
    }
}

/// Waits for host events until one matches.
async fn host_event(h: &BridgeHost, ok: impl Fn(&HostEvent) -> bool) -> HostEvent {
    for _ in 0..200 {
        for e in h.events() {
            if ok(&e) {
                return e;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no such host event");
}

fn power(w: f64) -> DeviceEvent {
    DeviceEvent::Power {
        watts: w,
        cadence_rpm: Some(90.0),
        timestamp: Duration::ZERO,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_pairs_gets_the_trainer_in_order_and_controls_it() {
    let h = host();
    assert_eq!(h.code.ip, Ipv4Addr::LOCALHOST);
    h.relay(
        "Wahoo KICKR 0001",
        &DeviceEvent::Connected { kind: "trainer" },
        false,
    );
    let mut b = Browser::connect(&h.code, SECRET, "page-a")
        .await
        .expect("connects");
    // Welcome: paired, the desktop's devices listed.
    let (m, ev) = b.step().await.unwrap();
    let ToBrowser::Welcome { host, devices, .. } = m else {
        panic!("{m:?}")
    };
    assert_eq!(host, "Test desktop");
    assert_eq!(devices[0].name, "Wahoo KICKR 0001");
    assert_eq!(ev, [DeviceEvent::Connected { kind: KIND }]);
    assert!(matches!(b.session.phase, Phase::Paired { .. }));
    assert!(matches!(
        host_event(&h, |e| matches!(e, HostEvent::Paired { .. })).await,
        HostEvent::Paired { client } if client == "Test browser"
    ));
    assert_eq!(h.paired().as_deref(), Some("Test browser"));
    // Clock: three pings.
    for _ in 0..3 {
        let p = b.session.ping(b.now());
        b.send(&p).await;
        let (m, _) = b.step().await.unwrap();
        assert!(matches!(m, ToBrowser::Pong { .. }));
    }
    let (_, rtt) = b.session.clock().unwrap();
    assert!(rtt < 50.0, "loopback round trip {rtt} ms");
    // Readings in order, with timestamps that go forward, and the wire's
    // latency (relay → arrival) far below the 150 ms target.
    let mut got = Vec::new();
    let mut worst = Duration::ZERO;
    for i in 0..20 {
        let sent = Instant::now();
        h.relay("Wahoo KICKR 0001", &power(200.0 + f64::from(i)), false);
        let (m, ev) = b.step().await.unwrap();
        worst = worst.max(sent.elapsed());
        assert!(matches!(m, ToBrowser::Power { seq, .. } if seq == u64::from(i as u32) + 1));
        for e in ev {
            if let DeviceEvent::Power {
                watts, timestamp, ..
            } = e
            {
                got.push((watts, timestamp));
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(got.len(), 20);
    assert!(
        got.windows(2)
            .all(|w| w[1].1 >= w[0].1 && w[1].0 == w[0].0 + 1.0)
    );
    assert!(
        worst < Duration::from_millis(150),
        "relay latency {worst:?}"
    );
    assert!(b.session.sample_age_ms.unwrap() < 150.0);
    // Heart rate.
    h.relay(
        "TICKR",
        &DeviceEvent::HeartRate {
            bpm: 133.0,
            timestamp: Duration::ZERO,
        },
        false,
    );
    let (_, ev) = b.step().await.unwrap();
    assert!(matches!(ev[..], [DeviceEvent::HeartRate { bpm, .. }] if bpm == 133.0));
    // Control round trip: the browser's SIM gradient reaches the desktop as
    // the same command; `applied` comes back.
    let sim = DeviceCommand::SetSimulation(SimulationParameters {
        grade: 0.062,
        wind_speed_mps: 1.5,
        crr: 0.004,
        cw_kg_per_m: 0.4,
        mass_kg: 81.0,
        wind_grade: 0.065,
    });
    let now = b.now();
    let m = b.session.control(sim, now).expect("a control");
    b.send(&m).await;
    let HostEvent::Control { id, command } =
        host_event(&h, |e| matches!(e, HostEvent::Control { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(command, sim);
    h.applied(id);
    let (m, _) = b.step().await.unwrap();
    assert_eq!(m, ToBrowser::Applied { id });
    let rtt = b.session.control_rtt_ms.unwrap();
    assert!(rtt < 150.0, "control round trip {rtt} ms");
    assert_eq!(h.stats().controls, 1);
    assert_eq!(h.stats().relayed, 21);

    // One browser at a time: another page is refused while this one lives.
    let mut other = Browser::connect(&h.code, SECRET, "page-b")
        .await
        .expect("connects");
    let (m, _) = other.step().await.unwrap();
    assert!(
        matches!(&m, ToBrowser::Refused { why } if why.contains("another browser")),
        "{m:?}"
    );
    assert!(other.session.is_refused());
    assert!(other.next().await.is_none(), "closed after the refusal");

    // The same page reconnecting (its old connection not noticed dead yet)
    // takes over; the old one hears why.
    let mut again = Browser::connect(&h.code, SECRET, "page-a")
        .await
        .expect("connects");
    let (m, _) = again.step().await.unwrap();
    assert!(matches!(m, ToBrowser::Welcome { .. }));
    loop {
        match b.next().await {
            Some(ToBrowser::Closed { why }) => {
                assert!(why.contains("took over"));
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    h.relay("Wahoo KICKR 0001", &power(300.0), false);
    let (m, _) = again.step().await.unwrap();
    assert!(matches!(m, ToBrowser::Power { seq: 22, w, .. } if w == 300.0));
    drop(b);

    // A clean stop: the browser hears it, the stream ends.
    h.stop();
    let mut closed = false;
    while let Some(m) = again.next().await {
        if let ToBrowser::Closed { why } = m {
            assert!(why.contains("stopped sharing"));
            closed = true;
        }
    }
    assert!(closed);
    let _ = again.wt.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_code_does_not_pair() {
    let h = host();
    // Another secret: another certificate – the TLS handshake fails, as in
    // a browser.
    let wrong = Browser::connect(&h.code, SECRET ^ 1, "p").await;
    assert!(wrong.is_err(), "a wrong certificate hash must not connect");
    // The right certificate but a wrong proof (cannot happen from a typed
    // code; a forged hello): refused.
    let mut forged = Browser::connect_with(&h.code, SECRET, SECRET ^ 1, "p")
        .await
        .expect("TLS works");
    let (m, _) = forged.step().await.unwrap();
    assert!(matches!(&m, ToBrowser::Refused { why } if why.contains("wrong pairing code")));
    assert!(matches!(
        host_event(&h, |e| matches!(e, HostEvent::Refused { .. })).await,
        HostEvent::Refused { .. }
    ));
    assert_eq!(h.paired(), None);
    // The browser leaves cleanly with Bye: the desktop unpairs.
    let mut b = Browser::connect(&h.code, SECRET, "q").await.unwrap();
    assert!(matches!(
        b.step().await.unwrap().0,
        ToBrowser::Welcome { .. }
    ));
    let _ = host_event(&h, |e| matches!(e, HostEvent::Paired { .. })).await;
    // 14I W4: the headset page's report reaches the desktop as it was sent.
    let data = serde_json::json!({"fps": 71.9, "space": "local-floor"});
    b.send(&ToHost::Report {
        kind: "xr".into(),
        data: data.clone(),
    })
    .await;
    let HostEvent::Report { kind, data: got } =
        host_event(&h, |e| matches!(e, HostEvent::Report { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((kind.as_str(), got), ("xr", data));
    b.send(&ToHost::Bye).await;
    let HostEvent::Unpaired { reason } =
        host_event(&h, |e| matches!(e, HostEvent::Unpaired { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("stopped"), "{reason}");
    assert_eq!(h.paired(), None);
    // After a reconnect the control applies again with the hello.
    let mut c = Browser::connect(&h.code, SECRET, "q").await.unwrap();
    c.session.control(DeviceCommand::SetTargetPower(210.0), 0.0);
    // (Not paired in this session object yet: kept for the next open.)
    let msgs = c.session.on_open(0.0);
    assert!(matches!(msgs[1], ToHost::Control { .. }));
    for m in msgs {
        c.send(&m).await;
    }
    let HostEvent::Control { command, .. } =
        host_event(&h, |e| matches!(e, HostEvent::Control { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(command, DeviceCommand::SetTargetPower(210.0));
}
