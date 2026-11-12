//! Output recovery decisions, kept free of cpal and Tauri so they can be
//! tested directly. The app polls these on every device-monitor tick, whether
//! or not the device list changed.

use std::time::{Duration, Instant};

use super::device::StreamErrorKind;
use super::monitor::DeviceSnapshot;

/// Why the output should be reopened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReopenReason {
    /// No output stream is open (startup failure or a failed reopen).
    NoOutput,
    /// The backend reported an error that stops the stream.
    StreamError(StreamErrorKind),
    /// The device the stream plays on is no longer listed.
    DeviceGone,
    /// Following the system default and the default changed.
    DefaultChanged,
    /// A preferred device is set, the stream runs elsewhere (fallback), and
    /// the preferred device is back.
    PreferredAvailable,
    /// The device now runs at a different sample rate than the stream.
    SampleRateChanged,
}

/// What the app knows about its output at one poll.
#[derive(Clone, Copy, Debug, Default)]
pub struct OutputState<'a> {
    /// Device the open stream plays on; `None` when no stream is open.
    pub current_device: Option<&'a str>,
    /// Sample rate the open stream runs at.
    pub current_rate: Option<u32>,
    /// The device's sample rate right now, if it could be read.
    pub device_rate_now: Option<u32>,
    /// Pending backend error on the open stream.
    pub stream_error: Option<StreamErrorKind>,
    /// Device chosen in settings; `None` follows the system default.
    pub preferred: Option<&'a str>,
}

/// Decides whether the output must be reopened, given the current device list.
pub fn reopen_reason(state: &OutputState<'_>, snapshot: &DeviceSnapshot) -> Option<ReopenReason> {
    let Some(current) = state.current_device else {
        return Some(ReopenReason::NoOutput);
    };
    if let Some(error) = state.stream_error.filter(|error| error.needs_reopen()) {
        return Some(ReopenReason::StreamError(error));
    }
    let listed = |id: &str| snapshot.outputs.iter().any(|output| output == id);
    if !listed(current) {
        return Some(ReopenReason::DeviceGone);
    }
    match state.preferred {
        Some(preferred) if preferred != current && listed(preferred) => {
            return Some(ReopenReason::PreferredAvailable);
        }
        None if snapshot
            .default_output
            .as_deref()
            .is_some_and(|default| default != current) =>
        {
            return Some(ReopenReason::DefaultChanged);
        }
        _ => {}
    }
    match (state.current_rate, state.device_rate_now) {
        (Some(stream), Some(device)) if stream != device => Some(ReopenReason::SampleRateChanged),
        _ => None,
    }
}

/// Spaces out reopen attempts: none while cooling down, doubling the wait
/// after each failure up to `max`.
#[derive(Clone, Debug)]
pub struct RetryBackoff {
    min: Duration,
    max: Duration,
    delay: Duration,
    next_attempt: Option<Instant>,
}

impl RetryBackoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self {
            min,
            max,
            delay: min,
            next_attempt: None,
        }
    }

    pub fn ready(&self, now: Instant) -> bool {
        self.next_attempt.is_none_or(|at| now >= at)
    }

    pub fn failed(&mut self, now: Instant) {
        self.next_attempt = Some(now + self.delay);
        self.delay = (self.delay * 2).min(self.max);
    }

    /// Resets the backoff; a short cooldown still applies so a condition that
    /// persists right after a reopen cannot cause a reopen on every poll.
    pub fn succeeded(&mut self, now: Instant) {
        self.delay = self.min;
        self.next_attempt = Some(now + self.min);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "coreaudio:A";
    const B: &str = "coreaudio:B";

    fn snapshot(outputs: &[&str], default: Option<&str>) -> DeviceSnapshot {
        DeviceSnapshot {
            outputs: outputs.iter().map(|s| (*s).to_owned()).collect(),
            default_output: default.map(str::to_owned),
            ..DeviceSnapshot::default()
        }
    }

    fn healthy(current: &'static str, preferred: Option<&'static str>) -> OutputState<'static> {
        OutputState {
            current_device: Some(current),
            current_rate: Some(48_000),
            device_rate_now: Some(48_000),
            stream_error: None,
            preferred,
        }
    }

    #[test]
    fn healthy_output_needs_nothing() {
        assert_eq!(
            reopen_reason(&healthy(A, None), &snapshot(&[A, B], Some(A))),
            None
        );
        assert_eq!(
            reopen_reason(&healthy(A, Some(A)), &snapshot(&[A, B], Some(B))),
            None
        );
    }

    #[test]
    fn stream_error_on_an_unchanged_device_list_reopens() {
        let list = snapshot(&[A], Some(A));
        for error in [
            StreamErrorKind::StreamInvalidated,
            StreamErrorKind::DeviceChanged,
            StreamErrorKind::DeviceNotAvailable,
            StreamErrorKind::Other,
        ] {
            let state = OutputState {
                stream_error: Some(error),
                ..healthy(A, None)
            };
            assert_eq!(
                reopen_reason(&state, &list),
                Some(ReopenReason::StreamError(error))
            );
        }
        let xrun = OutputState {
            stream_error: Some(StreamErrorKind::Xrun),
            ..healthy(A, None)
        };
        assert_eq!(reopen_reason(&xrun, &list), None, "an xrun keeps playing");
    }

    #[test]
    fn missing_output_is_retried() {
        let state = OutputState {
            preferred: None,
            ..OutputState::default()
        };
        assert_eq!(
            reopen_reason(&state, &snapshot(&[A], Some(A))),
            Some(ReopenReason::NoOutput)
        );
    }

    #[test]
    fn unplugged_device_reopens() {
        assert_eq!(
            reopen_reason(&healthy(A, Some(A)), &snapshot(&[B], Some(B))),
            Some(ReopenReason::DeviceGone)
        );
    }

    #[test]
    fn system_default_mode_follows_a_default_change_while_both_stay_connected() {
        assert_eq!(
            reopen_reason(&healthy(A, None), &snapshot(&[A, B], Some(B))),
            Some(ReopenReason::DefaultChanged)
        );
    }

    #[test]
    fn chosen_device_ignores_default_changes_and_returns_when_back() {
        assert_eq!(
            reopen_reason(&healthy(A, Some(A)), &snapshot(&[A, B], Some(B))),
            None
        );
        // Running on the fallback B while the preferred A was away; A is back.
        assert_eq!(
            reopen_reason(&healthy(B, Some(A)), &snapshot(&[A, B], Some(B))),
            Some(ReopenReason::PreferredAvailable)
        );
        assert_eq!(
            reopen_reason(&healthy(B, Some(A)), &snapshot(&[B], Some(B))),
            None
        );
    }

    #[test]
    fn device_rate_change_with_the_same_id_reopens() {
        let state = OutputState {
            device_rate_now: Some(44_100),
            ..healthy(A, None)
        };
        assert_eq!(
            reopen_reason(&state, &snapshot(&[A], Some(A))),
            Some(ReopenReason::SampleRateChanged)
        );
    }

    #[test]
    fn backoff_doubles_after_failures_and_cools_down_after_success() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut backoff = RetryBackoff::new(second, 4 * second);
        assert!(backoff.ready(start));

        backoff.failed(start);
        assert!(!backoff.ready(start + second / 2));
        assert!(backoff.ready(start + second));

        backoff.failed(start + second);
        assert!(!backoff.ready(start + 2 * second));
        assert!(backoff.ready(start + 3 * second));

        backoff.failed(start + 3 * second); // 4 s
        backoff.failed(start + 7 * second); // capped at 4 s
        assert!(!backoff.ready(start + 10 * second));
        assert!(backoff.ready(start + 11 * second));

        backoff.succeeded(start + 11 * second);
        assert!(!backoff.ready(start + 11 * second));
        assert!(backoff.ready(start + 12 * second));
    }
}
