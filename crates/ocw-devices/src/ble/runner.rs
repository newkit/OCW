//! Drives a [`TrainerProtocol`] over a [`GattLink`] until disconnect or
//! shutdown ([`run`]), and reconnects ([`supervise`]). No runtime of its
//! own: the caller's executor runs it (Tokio natively, the browser's event
//! loop in the browser app) and a [`Timer`] gives it time.

use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use futures::future::{Either, select};
use futures::{Stream, StreamExt};
use web_time::Instant;

use super::protocol::{Output, TrainerProtocol};
use crate::gatt::{ftms::Features, uuids};
use crate::{DeviceCommand, DeviceError, DeviceEvent};

/// Stream of `(characteristic, value)` notifications; it ends when the
/// device disconnects (if the link knows; else [`GattLink::is_connected`]
/// tells).
pub type Notifications = Pin<Box<dyn Stream<Item = (u16, Vec<u8>)> + Send>>;

/// A connected GATT device, addressed by SIG characteristic numbers.
/// Implemented by the btleplug link, the Web Bluetooth link and test mocks.
#[allow(async_fn_in_trait)] // used on a single-threaded executor only
pub trait GattLink {
    /// SIG characteristics the device offers (after service discovery).
    fn characteristics(&self) -> Vec<u16>;
    async fn read(&self, characteristic: u16) -> Result<Vec<u8>, DeviceError>;
    /// Write with response.
    async fn write(&self, characteristic: u16, value: &[u8]) -> Result<(), DeviceError>;
    async fn subscribe(&self, characteristic: u16) -> Result<(), DeviceError>;
    async fn notifications(&self) -> Result<Notifications, DeviceError>;
    async fn is_connected(&self) -> bool;
}

/// Time for the runner: Tokio's sleep natively, `setTimeout` in the browser.
pub trait Timer {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()>;
}

/// [`Timer`] on Tokio (the native backend's runtime, the tests).
#[cfg(not(target_arch = "wasm32"))]
pub struct TokioTimer;

#[cfg(not(target_arch = "wasm32"))]
impl Timer for TokioTimer {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> {
        tokio::time::sleep(duration)
    }
}

/// Why [`run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// The app asked to stop (or dropped its command sender).
    Shutdown,
    /// The device went away.
    Disconnected,
}

const TICK: Duration = Duration::from_millis(100);
const CONNECTION_CHECK: Duration = Duration::from_secs(1);
/// How often the reconnect pause looks at the commands.
const POLL: Duration = Duration::from_millis(200);

/// Runs the protocol on `link`. `last_target` is the latest SIM/ERG command;
/// it is re-applied when a run starts (after a reconnect) and updated with
/// every new one.
pub async fn run<L: GattLink, T: Timer>(
    link: &L,
    commands: &Receiver<DeviceCommand>,
    events: &Sender<DeviceEvent>,
    started: Instant,
    last_target: &mut Option<DeviceCommand>,
    timer: &T,
) -> Result<RunEnd, DeviceError> {
    let available = link.characteristics();
    let features = if available.contains(&uuids::FITNESS_MACHINE_FEATURE) {
        match link.read(uuids::FITNESS_MACHINE_FEATURE).await {
            Ok(v) => Features::parse(&v).ok(),
            Err(e) => {
                tracing::warn!("reading FTMS features failed: {e}");
                None
            }
        }
    } else {
        None
    };
    let mut protocol = TrainerProtocol::new(&available, features);
    let control = protocol.control_characteristic();
    tracing::info!(kind = protocol.describe(), "BLE device ready");
    let mut stream = link.notifications().await?;
    for c in protocol.subscriptions() {
        link.subscribe(c).await?;
    }
    let _ = events.send(DeviceEvent::Connected {
        kind: protocol.describe(),
    });
    if let Some(cmd) = *last_target {
        let out = protocol.on_command(cmd, started.elapsed());
        apply(link, events, out, control).await;
    }

    let mut tick = pin!(timer.sleep(TICK));
    let mut last_check = Instant::now();
    loop {
        match select(stream.next(), tick.as_mut()).await {
            Either::Left((n, _)) => match n {
                Some((c, value)) => {
                    if c == control {
                        tracing::debug!(?value, "control point response");
                    }
                    let out = protocol.on_notification(c, &value, started.elapsed());
                    apply(link, events, out, control).await;
                }
                None => return Ok(RunEnd::Disconnected),
            },
            Either::Right(((), _)) => {
                tick.set(timer.sleep(TICK));
                loop {
                    match commands.try_recv() {
                        Ok(DeviceCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                            return Ok(RunEnd::Shutdown);
                        }
                        Ok(cmd) => {
                            if is_target(cmd) {
                                *last_target = Some(cmd);
                            }
                            let out = protocol.on_command(cmd, started.elapsed());
                            apply(link, events, out, control).await;
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }
                let out = protocol.on_tick(started.elapsed());
                apply(link, events, out, control).await;
                if last_check.elapsed() >= CONNECTION_CHECK {
                    last_check = Instant::now();
                    if !link.is_connected().await {
                        return Ok(RunEnd::Disconnected);
                    }
                }
            }
        }
    }
}

fn is_target(cmd: DeviceCommand) -> bool {
    matches!(
        cmd,
        DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_)
    )
}

/// Connects (`connect`), runs, and after a disconnect or a failed attempt
/// reports [`DeviceEvent::Disconnected`], waits `reconnect_delay` (still
/// taking commands: shutdown, and the latest SIM / ERG target, which the
/// next run applies) and connects again – until shutdown or until the app
/// is gone. `name` is for the log.
pub async fn supervise<L, C, F, T>(
    mut connect: C,
    commands: &Receiver<DeviceCommand>,
    events: &Sender<DeviceEvent>,
    timer: &T,
    reconnect_delay: Duration,
    name: &str,
) -> RunEnd
where
    L: GattLink,
    C: FnMut() -> F,
    F: Future<Output = Result<L, DeviceError>>,
    T: Timer,
{
    let started = Instant::now();
    let mut last_target = None;
    loop {
        let attempt = match connect().await {
            Ok(link) => run(&link, commands, events, started, &mut last_target, timer).await,
            Err(e) => Err(e),
        };
        match attempt {
            Ok(RunEnd::Shutdown) => return RunEnd::Shutdown,
            Ok(RunEnd::Disconnected) => tracing::warn!(device = %name, "disconnected"),
            Err(e) => tracing::warn!(device = %name, "{e}"),
        }
        if events.send(DeviceEvent::Disconnected).is_err() {
            return RunEnd::Shutdown; // app gone
        }
        let mut waited = Duration::ZERO;
        while waited < reconnect_delay {
            let step = POLL.min(reconnect_delay - waited);
            timer.sleep(step).await;
            waited += step;
            loop {
                match commands.try_recv() {
                    Ok(DeviceCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                        return RunEnd::Shutdown;
                    }
                    Ok(cmd) if is_target(cmd) => last_target = Some(cmd),
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                }
            }
        }
    }
}

async fn apply<L: GattLink>(link: &L, events: &Sender<DeviceEvent>, out: Output, control: u16) {
    for w in out.warnings {
        tracing::warn!("{w}");
    }
    for e in out.events {
        // The app may already be gone during shutdown; nothing to do then.
        let _ = events.send(e);
    }
    for value in out.control_writes {
        tracing::debug!(?value, "control point write");
        if let Err(e) = link.write(control, &value).await {
            tracing::warn!("control point write failed: {e}");
        }
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A scripted FTMS trainer for tests.

    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

    use super::*;

    type Notification = (u16, Vec<u8>);

    pub struct MockTrainer {
        pub available: Vec<u16>,
        pub features: Vec<u8>,
        /// Answer control point writes with success indications.
        pub auto_respond: bool,
        pub writes: RefCell<Vec<(u16, Vec<u8>)>>,
        pub subscribed: RefCell<Vec<u16>>,
        tx: UnboundedSender<Notification>,
        rx: RefCell<Option<UnboundedReceiver<Notification>>>,
        pub connected: Arc<AtomicBool>,
    }

    impl MockTrainer {
        pub fn ftms() -> Self {
            let (tx, rx) = unbounded();
            let mut features = ((1u32 << 1) | (1 << 14)).to_le_bytes().to_vec();
            features.extend(((1u32 << 3) | (1 << 13)).to_le_bytes());
            Self {
                available: vec![
                    uuids::FITNESS_MACHINE_FEATURE,
                    uuids::INDOOR_BIKE_DATA,
                    uuids::FITNESS_MACHINE_CONTROL_POINT,
                ],
                features,
                auto_respond: true,
                writes: RefCell::new(Vec::new()),
                subscribed: RefCell::new(Vec::new()),
                tx,
                rx: RefCell::new(Some(rx)),
                connected: Arc::new(AtomicBool::new(true)),
            }
        }

        /// An older Wahoo KICKR: Cycling Power plus Wahoo's control point, no FTMS.
        pub fn wahoo_kickr() -> Self {
            Self {
                available: vec![
                    uuids::CYCLING_POWER_MEASUREMENT,
                    uuids::WAHOO_TRAINER_CONTROL,
                ],
                features: Vec::new(),
                ..Self::ftms()
            }
        }

        /// A sender to inject notifications from the test.
        pub fn notifier(&self) -> UnboundedSender<Notification> {
            self.tx.clone()
        }
    }

    impl GattLink for MockTrainer {
        fn characteristics(&self) -> Vec<u16> {
            self.available.clone()
        }
        async fn read(&self, c: u16) -> Result<Vec<u8>, DeviceError> {
            if c == uuids::FITNESS_MACHINE_FEATURE {
                Ok(self.features.clone())
            } else {
                Err(DeviceError::Bluetooth(format!("cannot read 0x{c:04x}")))
            }
        }
        async fn write(&self, c: u16, value: &[u8]) -> Result<(), DeviceError> {
            self.writes.borrow_mut().push((c, value.to_vec()));
            if self.auto_respond && c == uuids::FITNESS_MACHINE_CONTROL_POINT {
                let _ = self.tx.unbounded_send((c, vec![0x80, value[0], 0x01]));
            }
            if self.auto_respond && c == uuids::WAHOO_TRAINER_CONTROL {
                let _ = self.tx.unbounded_send((c, vec![0x01, value[0], 0x01]));
            }
            Ok(())
        }
        async fn subscribe(&self, c: u16) -> Result<(), DeviceError> {
            self.subscribed.borrow_mut().push(c);
            Ok(())
        }
        async fn notifications(&self) -> Result<Notifications, DeviceError> {
            let rx = self
                .rx
                .borrow_mut()
                .take()
                .expect("notifications() called once");
            Ok(Box::pin(rx))
        }
        async fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;

    use std::cell::RefCell;

    use super::mock::MockTrainer;
    use super::*;
    use crate::SimulationParameters;

    fn sim(grade: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(SimulationParameters {
            grade,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.2,
            mass_kg: 83.0,
        })
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    /// Device backend with a mock BLE peripheral ↔ app (spec §14, Phase 2).
    #[test]
    fn mock_trainer_end_to_end() {
        let trainer = MockTrainer::ftms();
        let notify = trainer.notifier();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let mut last = None;
        cmd_tx.send(sim(0.04)).unwrap();
        let end = rt().block_on(async {
            let run = run(
                &trainer,
                &cmd_rx,
                &ev_tx,
                Instant::now(),
                &mut last,
                &TokioTimer,
            );
            let script = async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                // 250 W at 90 rpm.
                notify
                    .unbounded_send((
                        uuids::INDOOR_BIKE_DATA,
                        vec![0x44, 0x00, 0, 0, 0xB4, 0, 0xFA, 0],
                    ))
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                cmd_tx.send(DeviceCommand::SetTargetPower(180.0)).unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
            };
            let (end, ()) = tokio::join!(run, script);
            end
        });
        assert_eq!(end.unwrap(), RunEnd::Shutdown);
        assert_eq!(
            *trainer.subscribed.borrow(),
            vec![
                uuids::INDOOR_BIKE_DATA,
                uuids::FITNESS_MACHINE_CONTROL_POINT
            ]
        );
        let writes: Vec<Vec<u8>> = trainer
            .writes
            .borrow()
            .iter()
            .map(|w| w.1.clone())
            .collect();
        assert_eq!(writes[0], vec![0x00]); // request control
        assert_eq!(writes[1], vec![0x07]); // start
        assert_eq!(writes[2][0], 0x11); // SIM 4 %
        assert_eq!(writes[3], vec![0x05, 0xB4, 0x00]); // ERG 180 W
        assert!(
            trainer
                .writes
                .borrow()
                .iter()
                .all(|w| w.0 == uuids::FITNESS_MACHINE_CONTROL_POINT)
        );
        let events: Vec<_> = ev_rx.try_iter().collect();
        assert!(matches!(
            events[..],
            [
                DeviceEvent::Connected { kind: "smart trainer (FTMS)" },
                DeviceEvent::Power { watts, cadence_rpm: Some(c), .. }
            ] if watts == 250.0 && c == 90.0
        ));
        assert_eq!(last, Some(DeviceCommand::SetTargetPower(180.0)));
    }

    #[test]
    fn disconnect_ends_the_run_and_last_target_is_reapplied() {
        let trainer = MockTrainer::ftms();
        let connected = trainer.connected.clone();
        let (_cmd_tx, cmd_rx) = mpsc::channel::<DeviceCommand>();
        let (ev_tx, _ev_rx) = mpsc::channel();
        let mut last = Some(sim(0.02));
        let end = rt().block_on(async {
            let run = run(
                &trainer,
                &cmd_rx,
                &ev_tx,
                Instant::now(),
                &mut last,
                &TokioTimer,
            );
            let script = async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                connected.store(false, Ordering::SeqCst);
            };
            let (end, ()) = tokio::join!(run, script);
            end
        });
        assert_eq!(end.unwrap(), RunEnd::Disconnected);
        // The remembered SIM target was sent again after (re)connecting.
        let writes: Vec<u8> = trainer.writes.borrow().iter().map(|w| w.1[0]).collect();
        assert_eq!(writes, vec![0x00, 0x07, 0x11]);
    }

    #[test]
    fn dropped_command_sender_shuts_down() {
        let trainer = MockTrainer::ftms();
        let (cmd_tx, cmd_rx) = mpsc::channel::<DeviceCommand>();
        drop(cmd_tx);
        let (ev_tx, _ev_rx) = mpsc::channel();
        let end = rt().block_on(run(
            &trainer,
            &cmd_rx,
            &ev_tx,
            Instant::now(),
            &mut None,
            &TokioTimer,
        ));
        assert_eq!(end.unwrap(), RunEnd::Shutdown);
    }

    /// The Wahoo control path end to end against a mock KICKR (no FTMS).
    #[test]
    fn mock_wahoo_kickr_end_to_end() {
        let trainer = MockTrainer::wahoo_kickr();
        let notify = trainer.notifier();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        cmd_tx.send(sim(0.03)).unwrap();
        let mut last = None;
        let end = rt().block_on(async {
            let run = run(
                &trainer,
                &cmd_rx,
                &ev_tx,
                Instant::now(),
                &mut last,
                &TokioTimer,
            );
            let script = async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                // 180 W from the Cycling Power service.
                notify
                    .unbounded_send((uuids::CYCLING_POWER_MEASUREMENT, vec![0, 0, 0xB4, 0]))
                    .unwrap();
                cmd_tx.send(DeviceCommand::SetTargetPower(150.0)).unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
            };
            let (end, ()) = tokio::join!(run, script);
            end
        });
        assert_eq!(end.unwrap(), RunEnd::Shutdown);
        let writes = trainer.writes.borrow();
        assert!(writes.iter().all(|w| w.0 == uuids::WAHOO_TRAINER_CONTROL));
        let ops: Vec<u8> = writes.iter().map(|w| w.1[0]).collect();
        assert_eq!(ops, vec![0x20, 0x43, 0x46, 0x42]);
        let events: Vec<_> = ev_rx.try_iter().collect();
        assert!(matches!(
            events[..],
            [
                DeviceEvent::Connected { kind: "smart trainer (Wahoo control)" },
                DeviceEvent::Power { watts, .. }
            ] if watts == 180.0
        ));
    }

    /// The reconnect loop: a failed attempt and a drop each report
    /// `Disconnected`; the target set while away is applied on the next
    /// connection; shutdown during the pause ends it.
    #[test]
    fn supervise_reconnects_and_reapplies_the_latest_target() {
        use std::cell::Cell;
        let attempts = Cell::new(0);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        cmd_tx.send(sim(0.02)).unwrap();
        let end = rt().block_on(async {
            let connect = || {
                attempts.set(attempts.get() + 1);
                let n = attempts.get();
                async move {
                    match n {
                        // The first attempt fails (out of range).
                        1 => Err(DeviceError::Bluetooth("not found".into())),
                        _ => Ok(Shared(MockTrainer::ftms())),
                    }
                }
            };
            let sup = supervise(
                connect,
                &cmd_rx,
                &ev_tx,
                &TokioTimer,
                Duration::from_millis(500),
                "mock",
            );
            let script = async {
                // Attempt 2 connects at 0.5 s; it drops at 0.8 s, which its
                // connection check sees at ≈ 1.5 s; the pause ends at ≈ 2 s.
                tokio::time::sleep(Duration::from_millis(800)).await;
                DROP.with(|d| d.set(true));
                // While away: ERG 160 W.
                tokio::time::sleep(Duration::from_millis(950)).await;
                cmd_tx.send(DeviceCommand::SetTargetPower(160.0)).unwrap();
                tokio::time::sleep(Duration::from_millis(750)).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
            };
            let (end, ()) = tokio::join!(sup, script);
            end
        });
        assert_eq!(end, RunEnd::Shutdown);
        assert!(attempts.get() >= 3, "{}", attempts.get());
        let events: Vec<_> = ev_rx.try_iter().collect();
        let kinds: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                DeviceEvent::Connected { .. } => Some("connected"),
                DeviceEvent::Disconnected => Some("disconnected"),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds[..4],
            ["disconnected", "connected", "disconnected", "connected"],
            "{events:?}"
        );
        // Every connection's writes: control, start, the latest target.
        let writes = WRITES.with(|w| w.borrow().clone());
        assert_eq!(writes[0], vec![0x00]);
        assert_eq!(writes[1], vec![0x07]);
        assert_eq!(writes[2][0], 0x11); // SIM 2 %
        // After the reconnect: control again, start, ERG 160 W.
        assert_eq!(writes[3], vec![0x00]);
        assert_eq!(writes[4], vec![0x07]);
        assert_eq!(writes[5], vec![0x05, 0xA0, 0x00]);
    }

    thread_local! {
        static DROP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static WRITES: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    }

    /// A mock trainer that records its writes in [`WRITES`] and drops
    /// when [`DROP`] is set (once).
    struct Shared(MockTrainer);

    impl GattLink for Shared {
        fn characteristics(&self) -> Vec<u16> {
            self.0.characteristics()
        }
        async fn read(&self, c: u16) -> Result<Vec<u8>, DeviceError> {
            self.0.read(c).await
        }
        async fn write(&self, c: u16, value: &[u8]) -> Result<(), DeviceError> {
            WRITES.with(|w| w.borrow_mut().push(value.to_vec()));
            self.0.write(c, value).await
        }
        async fn subscribe(&self, c: u16) -> Result<(), DeviceError> {
            self.0.subscribe(c).await
        }
        async fn notifications(&self) -> Result<Notifications, DeviceError> {
            self.0.notifications().await
        }
        async fn is_connected(&self) -> bool {
            !DROP.with(|d| d.replace(false))
        }
    }
}
