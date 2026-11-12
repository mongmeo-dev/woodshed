//! Chromatic tuner: input device handling, pitch analysis and Tauri commands.

pub mod analyzer;
pub mod input;
pub mod mpm;
pub mod notes;
pub mod permission;
pub mod pump;
pub mod service;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::audio::InputChannel;

pub use service::{TunerError, TunerEvent, TunerService, TunerState, TunerStatus};

/// Tuner choices kept in the settings file. Owned by the tuner commands:
/// `settings_save` from the screen never changes them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TunerSettings {
    /// `None` follows the system default input.
    pub input_device_id: Option<String>,
    pub input_channel: InputChannel,
    /// A4 in Hz (430–450).
    pub reference_hz: u16,
}

impl Default for TunerSettings {
    fn default() -> Self {
        Self {
            input_device_id: None,
            input_channel: InputChannel::Single(0),
            reference_hz: notes::DEFAULT_REFERENCE_HZ,
        }
    }
}
