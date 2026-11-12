//! Which input the tuner listens to, and keeping it alive: the chosen device
//! (or the system default), the system default while the chosen device is
//! gone, back to the chosen device when it returns, and a fresh stream after
//! a stream error.
//!
//! The decisions live in [`InputSupervisor`], which talks to the world through
//! [`InputEnv`], so tests drive it with fake devices and injected failures.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::audio::InputChannel;

/// Wait before trying again when no input could be opened.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// While on the default input, how often to retry a chosen device that is
/// listed but failed to open.
pub const PREFERRED_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// An input device as listed: id and channel count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedInput {
    pub id: String,
    pub channels: u16,
}

/// The input devices present right now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDevices {
    pub devices: Vec<ListedInput>,
    pub default: Option<String>,
}

impl InputDevices {
    fn find(&self, id: &str) -> Option<&ListedInput> {
        self.devices.iter().find(|device| device.id == id)
    }

    fn default_device(&self) -> Option<&ListedInput> {
        self.default.as_deref().and_then(|id| self.find(id))
    }
}

/// The input stream that is running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedInput {
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Channel actually captured (the chosen one, or the first channel when
    /// the device has fewer channels).
    pub channel: InputChannel,
}

/// The world the supervisor acts on.
pub trait InputEnv {
    /// A running stream; dropping it stops the stream.
    type Capture;
    fn devices(&mut self) -> InputDevices;
    /// Opens `device` (`None` = system default) capturing `channel`.
    fn open(
        &mut self,
        device: Option<&str>,
        channel: InputChannel,
    ) -> Result<(Self::Capture, OpenedInput), String>;
}

/// Keeps `channel` if the device has it, else falls back to the first channel.
pub fn fit_channel(channel: InputChannel, channels: u16) -> InputChannel {
    match channel {
        InputChannel::Single(index) if index >= channels.max(1) => InputChannel::Single(0),
        other => other,
    }
}

pub struct InputSupervisor<E: InputEnv> {
    env: E,
    preferred: Option<String>,
    channel: InputChannel,
    devices: InputDevices,
    capture: Option<(E::Capture, OpenedInput)>,
    /// The chosen device is missing or failed, the default plays instead.
    fallback: bool,
    retry_at: Option<Instant>,
    preferred_retry_at: Option<Instant>,
    /// Bumped on every successful open, so the owner can rebuild its analysis.
    generation: u64,
}

impl<E: InputEnv> InputSupervisor<E> {
    pub fn new(env: E, preferred: Option<String>, channel: InputChannel) -> Self {
        Self {
            env,
            preferred,
            channel,
            devices: InputDevices::default(),
            capture: None,
            fallback: false,
            retry_at: None,
            preferred_retry_at: None,
            generation: 0,
        }
    }

    pub fn preferred(&self) -> Option<&str> {
        self.preferred.as_deref()
    }

    pub fn channel(&self) -> InputChannel {
        self.channel
    }

    pub fn opened(&self) -> Option<&OpenedInput> {
        self.capture.as_ref().map(|(_, opened)| opened)
    }

    pub fn capture_mut(&mut self) -> Option<&mut E::Capture> {
        self.capture.as_mut().map(|(capture, _)| capture)
    }

    /// Whether the system default plays because the chosen device cannot.
    pub fn is_fallback(&self) -> bool {
        self.fallback && self.capture.is_some()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Starts listening (first open).
    pub fn start(&mut self, now: Instant) {
        self.devices = self.env.devices();
        self.reopen(now);
    }

    /// Stops the stream (permission lost, tuner closed).
    pub fn stop(&mut self) {
        self.capture = None;
        self.retry_at = None;
    }

    /// Records the user's choice without opening anything (not started yet).
    pub fn choose(&mut self, device: Option<String>, channel: InputChannel) {
        self.preferred = device;
        self.channel = channel;
    }

    /// The user picked a device (`None` = system default) and channel.
    pub fn select(&mut self, now: Instant, device: Option<String>, channel: InputChannel) {
        self.preferred = device;
        self.channel = channel;
        self.devices = self.env.devices();
        self.reopen(now);
    }

    /// The device list changed (`audio://devices-changed`).
    pub fn devices_changed(&mut self, now: Instant) {
        let before = std::mem::replace(&mut self.devices, self.env.devices());
        let Some(opened) = self.opened() else {
            self.reopen(now);
            return;
        };
        let opened_id = opened.device_id.clone();
        if self.devices.find(&opened_id).is_none() {
            self.reopen(now);
            return;
        }
        // The chosen device came (back): use it. One that stayed listed but
        // failed to open is retried on its own schedule (`tick`).
        let preferred_returned = self.fallback
            && self
                .preferred
                .as_deref()
                .is_some_and(|id| self.devices.find(id).is_some() && before.find(id).is_none());
        if preferred_returned {
            self.reopen(now);
            return;
        }
        // On the system default (by choice or as a fallback): follow it.
        let on_default = self.preferred.is_none() || self.fallback;
        if on_default && self.devices.default.as_deref() != Some(opened_id.as_str()) {
            self.open_default(now);
        }
    }

    /// The running stream reported an error or stopped delivering samples.
    pub fn stream_failed(&mut self, now: Instant) {
        self.capture = None;
        self.devices = self.env.devices();
        self.reopen(now);
    }

    /// Periodic work: retries after failures.
    pub fn tick(&mut self, now: Instant) {
        if self.capture.is_none() {
            if self.retry_at.is_some_and(|at| now >= at) {
                self.devices = self.env.devices();
                self.reopen(now);
            }
            return;
        }
        if self.fallback && self.preferred_retry_at.is_some_and(|at| now >= at) {
            self.devices = self.env.devices();
            if self
                .preferred
                .as_deref()
                .is_some_and(|id| self.devices.find(id).is_some())
            {
                self.reopen(now);
            } else {
                self.preferred_retry_at = None;
            }
        }
    }

    /// Moves to the current system default, keeping the fallback state and
    /// the schedule for retrying the chosen device.
    fn open_default(&mut self, now: Instant) {
        self.capture = None;
        self.retry_at = None;
        let channels = self.devices.default_device().map_or(1, |d| d.channels);
        let channel = fit_channel(self.channel, channels);
        if !self.try_open(None, channel) {
            self.retry_at = Some(now + RETRY_INTERVAL);
        }
    }

    fn reopen(&mut self, now: Instant) {
        // Close first: some devices refuse a second stream.
        self.capture = None;
        self.retry_at = None;
        self.preferred_retry_at = None;
        if let Some(id) = self.preferred.clone() {
            if let Some(device) = self.devices.find(&id).cloned() {
                let channel = fit_channel(self.channel, device.channels);
                if self.try_open(Some(&id), channel) {
                    self.fallback = false;
                    return;
                }
                self.preferred_retry_at = Some(now + PREFERRED_RETRY_INTERVAL);
            }
            self.fallback = true;
        } else {
            self.fallback = false;
        }
        let channels = self.devices.default_device().map_or(1, |d| d.channels);
        let channel = fit_channel(self.channel, channels);
        if !self.try_open(None, channel) {
            self.retry_at = Some(now + RETRY_INTERVAL);
        }
    }

    fn try_open(&mut self, device: Option<&str>, channel: InputChannel) -> bool {
        match self.env.open(device, channel) {
            Ok(capture) => {
                self.capture = Some(capture);
                self.generation += 1;
                true
            }
            Err(error) => {
                log::warn!("tuner input: cannot open {device:?}: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeEnv {
        devices: InputDevices,
        /// Devices whose open fails.
        broken: Vec<String>,
        opens: Vec<(Option<String>, InputChannel)>,
    }

    impl FakeEnv {
        fn with(devices: &[(&str, u16)], default: &str) -> Self {
            Self {
                devices: InputDevices {
                    devices: devices
                        .iter()
                        .map(|&(id, channels)| ListedInput {
                            id: id.into(),
                            channels,
                        })
                        .collect(),
                    default: Some(default.into()),
                },
                ..Self::default()
            }
        }
    }

    impl InputEnv for FakeEnv {
        type Capture = String;

        fn devices(&mut self) -> InputDevices {
            self.devices.clone()
        }

        fn open(
            &mut self,
            device: Option<&str>,
            channel: InputChannel,
        ) -> Result<(String, OpenedInput), String> {
            self.opens.push((device.map(str::to_owned), channel));
            let id = device
                .map(str::to_owned)
                .or_else(|| self.devices.default.clone())
                .ok_or("no default")?;
            let listed = self.devices.find(&id).cloned().ok_or("not found")?;
            if self.broken.contains(&id) {
                return Err("broken".into());
            }
            if let InputChannel::Single(index) = channel {
                assert!(index < listed.channels, "channel {index} of {id}");
            }
            Ok((
                id.clone(),
                OpenedInput {
                    device_id: id.clone(),
                    device_name: id,
                    sample_rate: 48_000,
                    channels: listed.channels,
                    channel,
                },
            ))
        }
    }

    fn opened_id<E: InputEnv>(supervisor: &InputSupervisor<E>) -> Option<&str> {
        supervisor.opened().map(|opened| opened.device_id.as_str())
    }

    const MIC: &str = "coreaudio:mic";
    const IFACE: &str = "coreaudio:interface";

    #[test]
    fn opens_the_chosen_device_and_channel() {
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 4)], MIC);
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(2));
        supervisor.start(Instant::now());
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert_eq!(
            supervisor.opened().unwrap().channel,
            InputChannel::Single(2)
        );
        assert!(!supervisor.is_fallback());
    }

    #[test]
    fn unplugging_falls_back_to_the_default_and_plugging_returns() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 4)], MIC);
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(3));
        supervisor.start(now);
        let first = supervisor.generation();

        // Unplug: the stream errors, then the device list changes.
        supervisor.env.devices.devices.retain(|d| d.id != IFACE);
        supervisor.stream_failed(now);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        assert!(supervisor.is_fallback());
        // Input 4 does not exist on a mono mic: its only channel is used.
        assert_eq!(
            supervisor.opened().unwrap().channel,
            InputChannel::Single(0)
        );
        assert!(supervisor.generation() > first);
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(MIC), "stays on the default");

        // Plug back in.
        supervisor.env.devices.devices.push(ListedInput {
            id: IFACE.into(),
            channels: 4,
        });
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert_eq!(
            supervisor.opened().unwrap().channel,
            InputChannel::Single(3)
        );
        assert!(!supervisor.is_fallback());
    }

    #[test]
    fn a_device_list_change_alone_moves_off_a_removed_device() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::MixDown);
        supervisor.start(now);
        supervisor.env.devices.devices.retain(|d| d.id != IFACE);
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        assert_eq!(supervisor.opened().unwrap().channel, InputChannel::MixDown);
    }

    #[test]
    fn following_the_default_moves_with_it() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        let mut supervisor = InputSupervisor::new(env, None, InputChannel::Single(0));
        supervisor.start(now);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        supervisor.env.devices.default = Some(IFACE.into());
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert!(!supervisor.is_fallback());
    }

    #[test]
    fn an_unrelated_change_keeps_the_stream() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(1));
        supervisor.start(now);
        let generation = supervisor.generation();
        supervisor.env.devices.devices.push(ListedInput {
            id: "coreaudio:other".into(),
            channels: 2,
        });
        supervisor.env.devices.default = Some("coreaudio:other".into());
        supervisor.devices_changed(now);
        assert_eq!(supervisor.generation(), generation);
    }

    #[test]
    fn a_stream_error_reopens_the_same_device() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(1));
        supervisor.start(now);
        let generation = supervisor.generation();
        supervisor.stream_failed(now);
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert_eq!(supervisor.generation(), generation + 1);
    }

    #[test]
    fn a_chosen_device_that_fails_to_open_is_retried_from_the_default() {
        let now = Instant::now();
        let mut env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        env.broken.push(IFACE.into());
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(1));
        supervisor.start(now);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        assert!(supervisor.is_fallback());

        supervisor.tick(now + Duration::from_secs(1));
        assert_eq!(opened_id(&supervisor), Some(MIC), "not before the interval");
        supervisor.env.broken.clear();
        supervisor.tick(now + PREFERRED_RETRY_INTERVAL);
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert!(!supervisor.is_fallback());
    }

    #[test]
    fn with_nothing_to_open_it_retries_later() {
        let now = Instant::now();
        let mut env = FakeEnv::with(&[(MIC, 1)], MIC);
        env.broken.push(MIC.into());
        let mut supervisor = InputSupervisor::new(env, None, InputChannel::Single(0));
        supervisor.start(now);
        assert_eq!(supervisor.opened(), None);
        let attempts = supervisor.env.opens.len();
        supervisor.tick(now + Duration::from_millis(500));
        assert_eq!(supervisor.env.opens.len(), attempts, "waits");
        supervisor.env.broken.clear();
        supervisor.tick(now + RETRY_INTERVAL);
        assert_eq!(opened_id(&supervisor), Some(MIC));
    }

    #[test]
    fn choosing_another_device_switches_right_away() {
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        let mut supervisor = InputSupervisor::new(env, None, InputChannel::Single(0));
        supervisor.start(now);
        supervisor.select(now, Some(IFACE.into()), InputChannel::Single(1));
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert_eq!(
            supervisor.opened().unwrap().channel,
            InputChannel::Single(1)
        );
        supervisor.select(now, None, InputChannel::MixDown);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        assert_eq!(supervisor.preferred(), None);
    }

    #[test]
    fn a_fallback_follows_the_system_default() {
        const OTHER: &str = "coreaudio:other";
        let now = Instant::now();
        let env = FakeEnv::with(&[(MIC, 1), (OTHER, 2)], MIC);
        // The chosen interface is not plugged in: the default mic plays.
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(1));
        supervisor.start(now);
        assert_eq!(opened_id(&supervisor), Some(MIC));
        assert!(supervisor.is_fallback());

        // The system default moves to another device; the mic stays.
        supervisor.env.devices.default = Some(OTHER.into());
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(OTHER));
        assert!(supervisor.is_fallback());
        assert_eq!(supervisor.preferred(), Some(IFACE), "choice kept");
        assert_eq!(supervisor.channel(), InputChannel::Single(1), "choice kept");
        assert_eq!(
            supervisor.opened().unwrap().channel,
            InputChannel::Single(1),
            "the new default has a second channel"
        );

        // The interface arrives: back to it.
        supervisor.env.devices.devices.push(ListedInput {
            id: IFACE.into(),
            channels: 4,
        });
        supervisor.devices_changed(now);
        assert_eq!(opened_id(&supervisor), Some(IFACE));
        assert!(!supervisor.is_fallback());
    }

    #[test]
    fn a_healthy_fallback_is_not_reopened_by_unrelated_changes() {
        let now = Instant::now();
        let mut env = FakeEnv::with(&[(MIC, 1), (IFACE, 2)], MIC);
        // Listed but failing: the retry schedule handles it, not list changes.
        env.broken.push(IFACE.into());
        let mut supervisor = InputSupervisor::new(env, Some(IFACE.into()), InputChannel::Single(1));
        supervisor.start(now);
        assert!(supervisor.is_fallback());
        let generation = supervisor.generation();
        let attempts = supervisor.env.opens.len();

        supervisor.env.devices.devices.push(ListedInput {
            id: "coreaudio:other".into(),
            channels: 2,
        });
        supervisor.devices_changed(now);
        assert_eq!(supervisor.generation(), generation, "no reopen");
        assert_eq!(supervisor.env.opens.len(), attempts, "no open attempt");
    }
}
