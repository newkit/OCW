//! Drives a [`TrainerProtocol`] over a [`GattLink`] until disconnect or shutdown.

use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};

use super::protocol::{Output, TrainerProtocol};
use crate::gatt::{ftms::Features, uuids};
use crate::{DeviceCommand, DeviceError, DeviceEvent};

/// Stream of `(characteristic, value)` notifications.
pub type Notifications = Pin<Box<dyn Stream<Item = (u16, Vec<u8>)> + Send>>;

/// A connected GATT device, addressed by SIG characteristic numbers.
/// Implemented by the btleplug link and by test mocks.
#[allow(async_fn_in_trait)] // used on a single-threaded runtime only
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

/// Runs the protocol on `link`. `last_target` is the latest SIM/ERG command;
/// it is re-applied when a run starts (after a reconnect) and updated with
/// every new one.
pub async fn run<L: GattLink>(
    link: &L,
    commands: &Receiver<DeviceCommand>,
    events: &Sender<DeviceEvent>,
    started: Instant,
    last_target: &mut Option<DeviceCommand>,
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
    tracing::info!(kind = protocol.describe(), "BLE device ready");
    let mut stream = link.notifications().await?;
    for c in protocol.subscriptions() {
        link.subscribe(c).await?;
    }
    if let Some(cmd) = *last_target {
        let out = protocol.on_command(cmd, started.elapsed());
        apply(link, events, out).await;
    }

    let mut tick = tokio::time::interval(TICK);
    let mut last_check = Instant::now();
    loop {
        tokio::select! {
            n = stream.next() => match n {
                Some((c, value)) => {
                    let out = protocol.on_notification(c, &value, started.elapsed());
                    apply(link, events, out).await;
                }
                None => return Ok(RunEnd::Disconnected),
            },
            _ = tick.tick() => {
                loop {
                    match commands.try_recv() {
                        Ok(DeviceCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                            return Ok(RunEnd::Shutdown);
                        }
                        Ok(cmd) => {
                            if matches!(cmd, DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_)) {
                                *last_target = Some(cmd);
                            }
                            let out = protocol.on_command(cmd, started.elapsed());
                            apply(link, events, out).await;
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }
                let out = protocol.on_tick(started.elapsed());
                apply(link, events, out).await;
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

async fn apply<L: GattLink>(link: &L, events: &Sender<DeviceEvent>, out: Output) {
    for w in out.warnings {
        tracing::warn!("{w}");
    }
    for e in out.events {
        // The app may already be gone during shutdown; nothing to do then.
        let _ = events.send(e);
    }
    for value in out.control_writes {
        if let Err(e) = link
            .write(uuids::FITNESS_MACHINE_CONTROL_POINT, &value)
            .await
        {
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

    use super::mock::MockTrainer;
    use super::*;
    use crate::SimulationParameters;

    fn sim(grade: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(SimulationParameters {
            grade,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.2,
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
            let run = run(&trainer, &cmd_rx, &ev_tx, Instant::now(), &mut last);
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
            [DeviceEvent::Power { watts, cadence_rpm: Some(c), .. }] if watts == 250.0 && c == 90.0
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
            let run = run(&trainer, &cmd_rx, &ev_tx, Instant::now(), &mut last);
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
        let end = rt().block_on(run(&trainer, &cmd_rx, &ev_tx, Instant::now(), &mut None));
        assert_eq!(end.unwrap(), RunEnd::Shutdown);
    }
}
