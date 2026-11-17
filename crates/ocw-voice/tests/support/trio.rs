//! Three desktop `VoiceController`s on a real LiveKit server, for the
//! voice block tests (spec §7.1.7 Q9): A and B are kept apart by the game
//! server (each controller gets its `VoiceApart`), C is with both. Each
//! rider speaks alone in turn while the others' outputs are measured, and
//! last B behaves like a modified client – it asks the voice server for
//! A's microphone itself – which A's subscription permissions must refuse.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ocw_core::RiderId;
use ocw_protocol::v1;
use ocw_voice::backend::{BackendCommand, BackendEvent, VoiceBackend};
use ocw_voice::livekit::LiveKitBackend;
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE};
use ocw_voice::spatial::Peer;
use ocw_voice::{MuteState, VoiceController, VoiceSettings};

/// Energy above which a voice counts as heard (a 0.3 tone gives ~45 a
/// second), and below which as silent.
pub const HEARD: f64 = 50.0;
pub const SILENT: f64 = 1.0;

/// A LiveKit backend shared between its controller and the test.
#[derive(Clone)]
pub struct Shared(pub Arc<Mutex<LiveKitBackend>>);

impl VoiceBackend for Shared {
    fn available(&self) -> Result<(), String> {
        self.0.lock().unwrap().available()
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, cmd: BackendCommand) {
        self.0.lock().unwrap().send(cmd);
    }
    fn poll(&mut self) -> Vec<BackendEvent> {
        self.0.lock().unwrap().poll()
    }
}

pub struct Rider {
    pub id: RiderId,
    pub name: String,
    pub grant: v1::VoiceGrant,
    pub c: VoiceController,
    pub b: Shared,
    phase: f32,
}

/// What each rider heard while one spoke.
#[derive(Debug, Default, Clone, Copy)]
pub struct Heard(pub [f64; 3]);

pub struct Trio {
    pub riders: Vec<Rider>,
    start: Instant,
}

fn tone(phase: &mut f32, hz: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|_| {
            *phase += std::f32::consts::TAU * hz / VOICE_RATE as f32;
            0.3 * phase.sin()
        })
        .collect()
}

impl Trio {
    /// The three riders with their grants (from the game server or
    /// minted like it) and the riders the game server keeps apart from
    /// each.
    pub fn new(riders: [(RiderId, &str, v1::VoiceGrant, BTreeSet<RiderId>); 3]) -> Self {
        let riders = riders
            .into_iter()
            .map(|(id, name, grant, apart)| {
                let b = Shared(Arc::new(Mutex::new(LiveKitBackend::headless())));
                let mut c = VoiceController::new(
                    Box::new(b.clone()),
                    VoiceSettings::default(),
                    MuteState::default(),
                );
                c.new_session(true);
                c.set_human_riders(3);
                c.set_apart(apart);
                Rider {
                    id,
                    name: name.to_owned(),
                    grant,
                    c,
                    b,
                    phase: 0.0,
                }
            })
            .collect();
        Self {
            riders,
            start: Instant::now(),
        }
    }

    fn ids(&self) -> Vec<RiderId> {
        self.riders.iter().map(|r| r.id).collect()
    }

    /// Runs the call for `seconds` (10 ms frames in real time): `speaker`
    /// speaks a tone, the others are silent; what each heard.
    pub fn run(&mut self, seconds: f64, speaker: Option<usize>) -> Heard {
        let ids = self.ids();
        let mut heard = Heard::default();
        let begin = Instant::now();
        let mut frames = 0u64;
        while begin.elapsed() < Duration::from_secs_f64(seconds) {
            let t = self.start.elapsed().as_secs_f64();
            for (i, r) in self.riders.iter_mut().enumerate() {
                // Everybody 5 m from everybody.
                let peers: Vec<Peer> = ids
                    .iter()
                    .filter(|id| **id != r.id)
                    .map(|id| Peer {
                        rider: *id,
                        gap_m: 5.0,
                        lateral_m: 0.0,
                    })
                    .collect();
                if r.c.update(t, &peers).is_some() {
                    r.c.on_grant(&r.grant, t);
                }
                let backend = r.b.0.lock().unwrap();
                let mic = if speaker == Some(i) {
                    tone(&mut r.phase, 440.0 + 220.0 * i as f32)
                } else {
                    vec![0.0; FRAME_SAMPLES]
                };
                backend.push_microphone(&mic);
                heard.0[i] += backend
                    .pull_output(FRAME_SAMPLES)
                    .iter()
                    .map(|s| f64::from(*s).powi(2))
                    .sum::<f64>();
            }
            frames += 1;
            let due = Duration::from_millis(10 * frames);
            if let Some(wait) = due.checked_sub(begin.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        heard
    }

    /// Runs until every rider is in the call and lists the other two (at
    /// most `timeout`); whether they all did.
    pub fn join(&mut self, timeout: Duration) -> bool {
        let begin = Instant::now();
        while begin.elapsed() < timeout {
            self.run(0.1, None);
            let ids = self.ids();
            let all = self.riders.iter().all(|r| {
                ids.iter().filter(|id| **id != r.id).all(|id| {
                    let who = ocw_protocol::voice_identity(*id);
                    r.c.participants().any(|p| p.identity == who)
                })
            });
            if all {
                return true;
            }
        }
        false
    }

    pub fn describe(&mut self) -> String {
        self.riders
            .iter_mut()
            .map(|r| {
                format!(
                    "{} ({}): {}, subscribed to {:?}, messages {:?}",
                    r.name,
                    r.id,
                    r.c.status().text(),
                    r.c.participants()
                        .filter(|p| p.heard.is_some())
                        .map(|p| p.identity.clone())
                        .collect::<Vec<_>>(),
                    r.c.take_messages()
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The block test (A = 0, B = 1 kept apart, C = 2): panics with what
    /// went wrong; the lines it passed.
    pub fn check_block(&mut self, log: &str) -> Vec<String> {
        let mut passed = Vec::new();
        assert!(
            self.join(Duration::from_secs(30)),
            "not all three in the call: {}{log}",
            self.describe()
        );
        passed.push("all three riders in the call".to_owned());
        // Subscriptions and permissions settle; old audio drains.
        self.run(2.0, None);
        let names: Vec<String> = self.riders.iter().map(|r| r.name.clone()).collect();
        for speaker in [2, 0, 1] {
            // Until the listeners that should hear the speaker do (at
            // most 10 s), then 3 s more for the silent ones.
            let mut total = Heard::default();
            let begin = Instant::now();
            loop {
                let h = self.run(0.5, Some(speaker));
                for i in 0..3 {
                    total.0[i] += h.0[i];
                }
                let should: Vec<usize> = match speaker {
                    2 => vec![0, 1],
                    _ => vec![2],
                };
                if should.iter().all(|&i| total.0[i] > HEARD) || begin.elapsed().as_secs() > 10 {
                    break;
                }
            }
            let more = self.run(3.0, Some(speaker));
            for i in 0..3 {
                total.0[i] += more.0[i];
            }
            self.run(0.5, None);
            let s = &names[speaker];
            match speaker {
                2 => {
                    for i in [0, 1] {
                        assert!(
                            total.0[i] > HEARD,
                            "{} does not hear {s} (energy {:.2}): {}{log}",
                            names[i],
                            total.0[i],
                            self.describe()
                        );
                    }
                    passed.push(format!("{} and {} hear {s}", names[0], names[1]));
                }
                _ => {
                    let other = 1 - speaker;
                    assert!(
                        total.0[2] > HEARD,
                        "{} does not hear {s} (energy {:.2}): {}{log}",
                        names[2],
                        total.0[2],
                        self.describe()
                    );
                    assert!(
                        total.0[other] < SILENT,
                        "{} hears {s} although they are kept apart (energy {:.2}): {}{log}",
                        names[other],
                        total.0[other],
                        self.describe()
                    );
                    passed.push(format!("{} hears {s}, {} does not", names[2], names[other]));
                }
            }
        }
        // B as a modified client: asks for A's microphone itself.
        let a = ocw_protocol::voice_identity(self.riders[0].id);
        {
            let b = self.riders[1].b.0.lock();
            let mut b = b.unwrap();
            b.send(BackendCommand::Subscribe {
                identity: a.clone(),
                on: true,
            });
            b.send(BackendCommand::SetPlacement {
                identity: a,
                gain: 1.0,
                pan: 0.0,
            });
        }
        self.run(1.0, None);
        let h = self.run(4.0, Some(0));
        assert!(
            h.0[2] > HEARD,
            "{} does not hear {} (energy {:.2}){log}",
            names[2],
            names[0],
            h.0[2]
        );
        assert!(
            h.0[1] < SILENT,
            "{} hears {} after subscribing by itself: the voice server let it (energy {:.2}){log}",
            names[1],
            names[0],
            h.0[1]
        );
        passed.push(format!(
            "{} asking for {}'s voice itself is refused by the voice server",
            names[1], names[0]
        ));
        for r in &mut self.riders {
            r.c.new_session(false);
        }
        passed
    }
}
