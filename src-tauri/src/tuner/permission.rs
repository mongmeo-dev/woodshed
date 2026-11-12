//! Microphone permission. On macOS a denied app still gets an input stream,
//! but it only carries silence, so the tuner asks the system first.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MicPermission {
    Granted,
    Denied,
    /// Never asked yet: the first request shows the system prompt.
    Undetermined,
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    use super::MicPermission;

    pub fn status() -> MicPermission {
        // SAFETY: a framework constant, read-only.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return MicPermission::Granted;
        };
        // SAFETY: AVMediaTypeAudio is a valid media type for this call.
        match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
            AVAuthorizationStatus::Authorized => MicPermission::Granted,
            AVAuthorizationStatus::NotDetermined => MicPermission::Undetermined,
            _ => MicPermission::Denied,
        }
    }

    pub fn request(done: impl Fn(bool) + Send + Sync + 'static) {
        // SAFETY: as in `status`.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            done(true);
            return;
        };
        let handler = RcBlock::new(move |granted: Bool| done(granted.as_bool()));
        // SAFETY: valid media type; the block is copied by the framework and
        // called once on an arbitrary queue, which `done` (Send + Sync) allows.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
    }

    /// Opens Privacy & Security › Microphone in System Settings.
    pub fn open_settings() -> std::io::Result<()> {
        std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .status()
            .map(|_| ())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::MicPermission;

    pub fn status() -> MicPermission {
        MicPermission::Granted
    }

    pub fn request(done: impl Fn(bool) + Send + Sync + 'static) {
        done(true);
    }

    pub fn open_settings() -> std::io::Result<()> {
        Ok(())
    }
}

pub use platform::{open_settings, request, status};
