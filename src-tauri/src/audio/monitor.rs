//! Device change detection. Polls the device list on a background thread and
//! calls a hook when it changes (device plugged or unplugged, default changed).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::HostTrait;
use serde::Serialize;

use super::device::{device_id_string, host};

/// Which devices exist right now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

impl DeviceSnapshot {
    pub fn capture() -> Self {
        let host = host();
        let ids = |devices: Option<Vec<cpal::Device>>| -> Vec<String> {
            let mut ids: Vec<String> = devices
                .unwrap_or_default()
                .iter()
                .filter_map(device_id_string)
                .collect();
            ids.sort();
            ids
        };
        Self {
            outputs: ids(host.output_devices().ok().map(Iterator::collect)),
            inputs: ids(host.input_devices().ok().map(Iterator::collect)),
            default_output: host
                .default_output_device()
                .as_ref()
                .and_then(device_id_string),
            default_input: host
                .default_input_device()
                .as_ref()
                .and_then(device_id_string),
        }
    }
}

/// Stops the polling thread when dropped.
pub struct DeviceMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DeviceMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Calls `on_change` with the new snapshot whenever the device list differs
/// from the previous poll.
pub fn spawn_device_monitor(
    interval: Duration,
    mut on_change: impl FnMut(&DeviceSnapshot) + Send + 'static,
) -> DeviceMonitor {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("audio-device-monitor".into())
        .spawn(move || {
            let tick = Duration::from_millis(100);
            let mut previous = DeviceSnapshot::capture();
            let mut waited = Duration::ZERO;
            while !thread_stop.load(Ordering::Relaxed) {
                std::thread::sleep(tick);
                waited += tick;
                if waited < interval {
                    continue;
                }
                waited = Duration::ZERO;
                let current = DeviceSnapshot::capture();
                if current != previous {
                    on_change(&current);
                    previous = current;
                }
            }
        })
        .ok();
    DeviceMonitor { stop, thread }
}
