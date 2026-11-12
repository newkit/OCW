//! The audio engine (spec §3.3): an output backend behind a trait, driven
//! on its own thread. The thread opens the output device chosen by name
//! (`audio.output`), falls back to the system default when that device is
//! missing, and follows hot-plugging: it re-reads the device list every
//! few seconds, reopens the saved device when it comes back and reopens
//! the default when the stream fails (device unplugged). Without any
//! device (headless runs, CI) it stays silent and says so once.
//!
//! The device decisions ([`choose_device`], [`Controller`]) are
//! synchronous and tested with a mock backend; the thread only calls the
//! controller.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::mix::BusGains;
use super::mixer::{Mixer, MixerMsg};

/// The mixer as the device callback and the engine share it.
pub type SharedMixer = Arc<Mutex<Mixer>>;

/// How often the device list is read again (hot-plugging).
pub const POLL: Duration = Duration::from_secs(3);
/// After a failed open, wait this long before trying again.
pub const RETRY: Duration = Duration::from_secs(15);
/// Messages to the mixer that can wait (a frame's worth of ride updates
/// and cues); more are dropped while no device pulls them.
const QUEUE: usize = 256;

/// The output the user chose (`audio.output`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// The system's default output device.
    Default,
    /// A device by name.
    Named(String),
}

impl Output {
    /// From the setting's text: `default` (any case) or a device name.
    pub fn from_setting(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("default") {
            Output::Default
        } else {
            Output::Named(s.to_string())
        }
    }

    pub fn setting(&self) -> &str {
        match self {
            Output::Default => "default",
            Output::Named(n) => n,
        }
    }
}

/// Which device to open for `wanted` among `devices`: `None` = the
/// default device. The flag says the saved device is missing and the
/// default is used instead.
pub fn choose_device(wanted: &Output, devices: &[String]) -> (Option<String>, bool) {
    match wanted {
        Output::Default => (None, false),
        Output::Named(n) => match devices.iter().find(|d| *d == n) {
            Some(d) => (Some(d.clone()), false),
            // Case or spacing may differ between platforms' listings.
            None => match devices
                .iter()
                .find(|d| d.trim().eq_ignore_ascii_case(n.trim()))
            {
                Some(d) => (Some(d.clone()), false),
                None => (None, true),
            },
        },
    }
}

/// An audio output: lists devices and plays the shared mixer on one.
/// Implementations: cpal (the real one), [`NullBackend`] (no sound) and a
/// mock in the tests.
pub trait OutputBackend {
    /// The output devices' names (duplicates removed, system order).
    fn devices(&mut self) -> Result<Vec<String>, String>;
    /// The default output device's name, if there is one.
    fn default_device(&mut self) -> Option<String>;
    /// Opens `name` (`None` = the default device), sets the mixer's
    /// sample rate and starts pulling samples from it; closes a stream
    /// that was open. Returns the device's name.
    fn open(&mut self, name: Option<&str>, mixer: SharedMixer) -> Result<String, String>;
    /// Stops the stream.
    fn close(&mut self);
    /// The stream failed since it was opened (device gone): the engine
    /// closes it and opens another.
    fn failed(&mut self) -> bool;
}

/// No audio output: never opens anything (`--no-audio`, frame capture).
pub struct NullBackend;

impl OutputBackend for NullBackend {
    fn devices(&mut self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn default_device(&mut self) -> Option<String> {
        None
    }
    fn open(&mut self, _: Option<&str>, _: SharedMixer) -> Result<String, String> {
        Err("audio is off".into())
    }
    fn close(&mut self) {}
    fn failed(&mut self) -> bool {
        false
    }
}

/// What the engine does now, for the menu and `audio.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioStatus {
    /// The output devices (names).
    pub devices: Vec<String>,
    /// The default device's name.
    pub default: Option<String>,
    /// The device playing now, if any.
    pub current: Option<String>,
    /// The saved device is missing; the default plays instead.
    pub fell_back: bool,
    /// One line: "playing on …", "no audio output: …".
    pub message: String,
    /// Bumped on every change (for the Bevy side's change detection).
    pub generation: u64,
}

/// Commands to the engine thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineCmd {
    SetOutput(Output),
    /// Read the device list now (the Audio page opened, `audio.devices`).
    Refresh,
}

/// The device logic of the engine thread, synchronous.
pub struct Controller<B: OutputBackend> {
    backend: B,
    mixer: SharedMixer,
    wanted: Output,
    /// What is open: the choice it was opened for (`None` = default).
    opened: Option<Option<String>>,
    /// A failed open: when, and for which choice.
    failed_open: Option<(Instant, Option<String>)>,
    last_poll: Option<Instant>,
    status: AudioStatus,
}

impl<B: OutputBackend> Controller<B> {
    pub fn new(backend: B, mixer: SharedMixer, wanted: Output) -> Self {
        Self {
            backend,
            mixer,
            wanted,
            opened: None,
            failed_open: None,
            last_poll: None,
            status: AudioStatus::default(),
        }
    }

    pub fn status(&self) -> &AudioStatus {
        &self.status
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn handle(&mut self, cmd: EngineCmd, now: Instant) {
        match cmd {
            EngineCmd::SetOutput(o) => {
                if o != self.wanted {
                    self.wanted = o;
                    self.failed_open = None;
                }
            }
            EngineCmd::Refresh => self.last_poll = None,
        }
        self.tick(now);
    }

    /// Reads the devices when due and (re)opens the output when the
    /// choice changed, the stream failed, or a failed open may be retried.
    pub fn tick(&mut self, now: Instant) {
        let mut changed = false;
        if self.opened.is_some() && self.backend.failed() {
            tracing::warn!(
                "audio: output {} failed (unplugged?), reopening",
                self.status.current.as_deref().unwrap_or("?")
            );
            self.backend.close();
            self.opened = None;
            self.status.current = None;
            self.last_poll = None;
            changed = true;
        }
        if self.last_poll.is_none_or(|t| now.duration_since(t) >= POLL) {
            self.last_poll = Some(now);
            let devices = self.backend.devices().unwrap_or_else(|e| {
                tracing::debug!("audio: listing devices: {e}");
                Vec::new()
            });
            let default = self.backend.default_device();
            if devices != self.status.devices || default != self.status.default {
                self.status.devices = devices;
                self.status.default = default;
                changed = true;
            }
        }
        let (choice, fell_back) = choose_device(&self.wanted, &self.status.devices);
        if fell_back != self.status.fell_back {
            self.status.fell_back = fell_back;
            changed = true;
        }
        let retry_due = match &self.failed_open {
            Some((t, c)) => *c != choice || now.duration_since(*t) >= RETRY,
            None => true,
        };
        if self.opened.as_ref() != Some(&choice) && retry_due {
            changed = true;
            match self.backend.open(choice.as_deref(), self.mixer.clone()) {
                Ok(name) => {
                    self.opened = Some(choice);
                    self.failed_open = None;
                    self.status.current = Some(name);
                }
                Err(e) => {
                    self.backend.close();
                    self.opened = None;
                    self.status.current = None;
                    self.failed_open = Some((now, choice));
                    self.status.message = format!("no audio output: {e}");
                }
            }
        }
        if changed {
            let message = match &self.status.current {
                Some(d) if self.status.fell_back => format!(
                    "playing on {d} ({} is missing, using the default)",
                    self.wanted.setting()
                ),
                Some(d) => format!("playing on {d}"),
                None => self.status.message.clone(),
            };
            if message != self.status.message || self.status.generation == 0 {
                tracing::info!("audio: {message}");
            }
            self.status.message = message;
            self.status.generation += 1;
        }
    }
}

/// The game's handle to the engine thread.
pub struct AudioEngine {
    ctrl: Sender<EngineCmd>,
    mixer_tx: SyncSender<MixerMsg>,
    status: Arc<Mutex<AudioStatus>>,
}

impl AudioEngine {
    /// Starts the engine thread; `make` creates the backend on it (cpal
    /// streams must stay on the thread that made them on some platforms).
    pub fn start<B, F>(make: F, wanted: Output, gains: BusGains) -> Self
    where
        B: OutputBackend + 'static,
        F: FnOnce() -> B + Send + 'static,
    {
        let (ctrl, ctrl_rx) = mpsc::channel();
        let (mixer_tx, mixer_rx) = mpsc::sync_channel(QUEUE);
        let status = Arc::new(Mutex::new(AudioStatus::default()));
        let shared = status.clone();
        let spawned = std::thread::Builder::new()
            .name("ocw-audio".into())
            .spawn(move || {
                let mixer = Arc::new(Mutex::new(
                    Mixer::new(48_000, gains).with_receiver(mixer_rx),
                ));
                run(Controller::new(make(), mixer, wanted), ctrl_rx, &shared);
            });
        if let Err(e) = spawned {
            tracing::warn!("audio: no engine thread ({e}), silent");
        }
        Self {
            ctrl,
            mixer_tx,
            status,
        }
    }

    pub fn set_output(&self, output: Output) {
        let _ = self.ctrl.send(EngineCmd::SetOutput(output));
    }

    pub fn refresh(&self) {
        let _ = self.ctrl.send(EngineCmd::Refresh);
    }

    /// Sends to the mixer; dropped when the queue is full (no device
    /// pulls it).
    pub fn send(&self, msg: MixerMsg) {
        if let Err(TrySendError::Disconnected(_)) = self.mixer_tx.try_send(msg) {
            // The engine thread ended (it only does when the app quits).
        }
    }

    pub fn status(&self) -> AudioStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn run<B: OutputBackend>(
    mut c: Controller<B>,
    rx: Receiver<EngineCmd>,
    shared: &Mutex<AudioStatus>,
) {
    c.tick(Instant::now());
    loop {
        if let Ok(mut s) = shared.lock()
            && s.generation != c.status().generation
        {
            *s = c.status().clone();
        }
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(cmd) => c.handle(cmd, Instant::now()),
            Err(RecvTimeoutError::Timeout) => c.tick(Instant::now()),
            // The app is gone.
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    c.backend_mut().close();
}

#[cfg(test)]
pub mod mock {
    //! A scripted backend: devices come and go, opens fail on request, the
    //! stream can fail; the opened mixer can be pulled like a device.
    use super::*;

    #[derive(Default)]
    pub struct MockState {
        pub devices: Vec<String>,
        pub default: Option<String>,
        /// Opening these names fails ("" = the default device).
        pub broken: Vec<String>,
        pub opens: Vec<Option<String>>,
        pub open: Option<String>,
        pub failed: bool,
        pub mixer: Option<SharedMixer>,
    }

    #[derive(Clone, Default)]
    pub struct MockBackend(pub Arc<Mutex<MockState>>);

    impl MockBackend {
        pub fn with_devices(devices: &[&str], default: Option<&str>) -> Self {
            let b = Self::default();
            {
                let mut s = b.0.lock().unwrap();
                s.devices = devices.iter().map(|d| d.to_string()).collect();
                s.default = default.map(str::to_string);
            }
            b
        }

        pub fn state(&self) -> std::sync::MutexGuard<'_, MockState> {
            self.0.lock().unwrap()
        }
    }

    impl OutputBackend for MockBackend {
        fn devices(&mut self) -> Result<Vec<String>, String> {
            Ok(self.state().devices.clone())
        }
        fn default_device(&mut self) -> Option<String> {
            self.state().default.clone()
        }
        fn open(&mut self, name: Option<&str>, mixer: SharedMixer) -> Result<String, String> {
            let mut s = self.state();
            s.opens.push(name.map(str::to_string));
            let resolved = match name {
                Some(n) => s.devices.iter().find(|d| *d == n).cloned(),
                None => s.default.clone(),
            }
            .ok_or("no such device")?;
            if s.broken.contains(&name.unwrap_or("").to_string()) {
                return Err("device busy".into());
            }
            s.open = Some(resolved.clone());
            s.failed = false;
            s.mixer = Some(mixer);
            Ok(resolved)
        }
        fn close(&mut self) {
            let mut s = self.state();
            s.open = None;
            s.mixer = None;
        }
        fn failed(&mut self) -> bool {
            self.state().failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn mixer() -> SharedMixer {
        Arc::new(Mutex::new(Mixer::new(48_000, BusGains::default())))
    }

    #[test]
    fn device_choice_falls_back_to_the_default() {
        let devs = names(&["Speakers", "USB Headset"]);
        assert_eq!(choose_device(&Output::Default, &devs), (None, false));
        let usb = Output::Named("USB Headset".into());
        assert_eq!(
            choose_device(&usb, &devs),
            (Some("USB Headset".into()), false)
        );
        // Case and spacing do not matter.
        assert_eq!(
            choose_device(&Output::Named(" usb headset".into()), &devs),
            (Some("USB Headset".into()), false)
        );
        // Missing: the default, flagged.
        assert_eq!(choose_device(&usb, &names(&["Speakers"])), (None, true));
        assert_eq!(choose_device(&usb, &[]), (None, true));
    }

    #[test]
    fn output_setting_text() {
        assert_eq!(Output::from_setting("default"), Output::Default);
        assert_eq!(Output::from_setting(" Default "), Output::Default);
        assert_eq!(Output::from_setting(""), Output::Default);
        assert_eq!(
            Output::from_setting("HDMI 1"),
            Output::Named("HDMI 1".into())
        );
        assert_eq!(Output::Named("x".into()).setting(), "x");
        assert_eq!(Output::Default.setting(), "default");
    }

    #[test]
    fn opens_the_saved_device_and_falls_back_when_it_is_missing() {
        let b = MockBackend::with_devices(&["Speakers", "Headset"], Some("Speakers"));
        let t0 = Instant::now();
        let mut c = Controller::new(b.clone(), mixer(), Output::Named("Headset".into()));
        c.tick(t0);
        assert_eq!(b.state().open.as_deref(), Some("Headset"));
        assert_eq!(c.status().message, "playing on Headset");
        assert!(!c.status().fell_back);
        // A saved device that is not there: the default plays.
        let b = MockBackend::with_devices(&["Speakers"], Some("Speakers"));
        let mut c = Controller::new(b.clone(), mixer(), Output::Named("Headset".into()));
        c.tick(t0);
        assert_eq!(b.state().opens, [None]);
        assert_eq!(b.state().open.as_deref(), Some("Speakers"));
        assert!(c.status().fell_back);
        assert!(
            c.status().message.contains("Headset is missing"),
            "{}",
            c.status().message
        );
    }

    #[test]
    fn follows_hot_plugging() {
        let b = MockBackend::with_devices(&["Speakers"], Some("Speakers"));
        let t0 = Instant::now();
        let mut c = Controller::new(b.clone(), mixer(), Output::Named("Headset".into()));
        c.tick(t0);
        assert!(c.status().fell_back);
        // The headset is plugged in: found at the next poll, opened.
        b.state().devices.push("Headset".into());
        c.tick(t0 + Duration::from_secs(1));
        assert_eq!(
            b.state().open.as_deref(),
            Some("Speakers"),
            "not yet polled"
        );
        c.tick(t0 + POLL);
        assert_eq!(b.state().open.as_deref(), Some("Headset"));
        assert!(!c.status().fell_back);
        // Unplugged: the stream fails, the default takes over at once.
        {
            let mut s = b.state();
            s.devices.retain(|d| d != "Headset");
            s.failed = true;
        }
        c.tick(t0 + POLL + Duration::from_millis(500));
        assert_eq!(b.state().open.as_deref(), Some("Speakers"));
        assert!(c.status().fell_back);
        assert_eq!(b.state().opens.len(), 3);
    }

    #[test]
    fn changing_the_output_reopens() {
        let b = MockBackend::with_devices(&["Speakers", "Headset"], Some("Speakers"));
        let t0 = Instant::now();
        let mut c = Controller::new(b.clone(), mixer(), Output::Default);
        c.tick(t0);
        assert_eq!(b.state().opens, [None]);
        c.handle(EngineCmd::SetOutput(Output::Named("Headset".into())), t0);
        assert_eq!(b.state().open.as_deref(), Some("Headset"));
        // The same choice again: nothing reopens.
        c.handle(EngineCmd::SetOutput(Output::Named("Headset".into())), t0);
        c.tick(t0 + POLL * 3);
        assert_eq!(b.state().opens.len(), 2);
    }

    /// No device at all (headless, CI): silent, one message, retried
    /// only now and then.
    #[test]
    fn without_a_device_it_stays_silent_and_retries_later() {
        let b = MockBackend::default();
        let t0 = Instant::now();
        let mut c = Controller::new(b.clone(), mixer(), Output::Default);
        c.tick(t0);
        assert_eq!(c.status().current, None);
        assert!(c.status().message.starts_with("no audio output"));
        c.tick(t0 + POLL);
        assert_eq!(b.state().opens.len(), 1, "no retry before RETRY");
        b.state().default = Some("Speakers".into());
        b.state().devices.push("Speakers".into());
        c.tick(t0 + RETRY);
        assert_eq!(b.state().open.as_deref(), Some("Speakers"));
        assert_eq!(c.status().message, "playing on Speakers");
        // A device that refuses to open (busy): the error, then the
        // retry; a refresh does not force an open before it.
        let b = MockBackend::with_devices(&["Speakers"], Some("Speakers"));
        b.state().broken.push(String::new());
        let mut c = Controller::new(b.clone(), mixer(), Output::Default);
        c.tick(t0);
        assert_eq!(c.status().message, "no audio output: device busy");
        c.handle(EngineCmd::Refresh, t0 + Duration::from_secs(1));
        assert_eq!(b.state().opens.len(), 1);
    }

    #[test]
    fn the_null_backend_never_plays() {
        let mut c = Controller::new(NullBackend, mixer(), Output::Default);
        c.tick(Instant::now());
        assert_eq!(c.status().current, None);
        assert_eq!(c.status().message, "no audio output: audio is off");
    }

    /// The engine thread with the mock: status arrives, the opened mixer
    /// gets the game's messages.
    #[test]
    fn engine_thread_runs_the_controller() {
        let b = MockBackend::with_devices(&["Speakers"], Some("Speakers"));
        let backend = b.clone();
        let full = super::super::mix::Mix {
            master: 100,
            effects: 100,
            ..Default::default()
        }
        .gains();
        let engine = AudioEngine::start(move || backend, Output::Default, full);
        engine.refresh();
        let deadline = Instant::now() + Duration::from_secs(5);
        while engine.status().current.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(engine.status().current.as_deref(), Some("Speakers"));
        assert_eq!(engine.status().devices, ["Speakers"]);
        engine.send(MixerMsg::Cue(
            super::super::cues::Cue::CountdownGo,
            Instant::now(),
        ));
        let m = b.state().mixer.clone().expect("opened");
        let mut out = vec![0.0f32; 4800];
        m.lock().unwrap().render(&mut out, 1);
        assert!(out.iter().any(|&s| s.abs() > 0.05));
    }
}
