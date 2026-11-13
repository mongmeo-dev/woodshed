//! Startup timeline. Each step is logged once, as `startup: <step> +<ms> ms
//! epoch_ms=<wall clock>`, counted from the start of [`crate::run`], so a slow
//! launch shows where the time went. The wall clock lines the steps up with
//! the system log and with a launcher's own clock.

use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static ORIGIN: OnceLock<Instant> = OnceLock::new();
/// Steps already logged (a few dozen at most).
static LOGGED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Starts the clock. Call first thing in `run`; the logger is not ready yet,
/// so the first [`mark`] after it reports the time spent before it.
pub fn begin() {
    ORIGIN.get_or_init(Instant::now);
}

/// Logs `step` the first time it is reached; later calls do nothing.
pub fn mark(step: &'static str) {
    {
        let mut logged = LOGGED.lock().unwrap_or_else(PoisonError::into_inner);
        if logged.contains(&step) {
            return;
        }
        logged.push(step);
    }
    let since = ORIGIN.get_or_init(Instant::now).elapsed();
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.as_millis());
    log::info!(
        "startup: {step} +{:.1} ms epoch_ms={epoch_ms}",
        since.as_secs_f64() * 1000.0
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_recorded_once() {
        begin();
        mark("test step");
        mark("test step");
        let logged = LOGGED.lock().unwrap();
        assert_eq!(
            logged.iter().filter(|step| **step == "test step").count(),
            1
        );
    }
}
