//! The drum kit: its tracks, the bundled samples (CC0, see
//! `LICENSES/drum-samples.md`) and their copies at each output rate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use super::resample::resample;
use super::wav::{Decoded, WavError, decode_wav};

pub const TRACK_COUNT: usize = 10;

/// One row of the step grid. The order is the grid's, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Track {
    Kick,
    Snare,
    Sidestick,
    Clap,
    ClosedHat,
    OpenHat,
    HighTom,
    LowTom,
    Ride,
    Crash,
}

impl Track {
    pub const ALL: [Track; TRACK_COUNT] = [
        Track::Kick,
        Track::Snare,
        Track::Sidestick,
        Track::Clap,
        Track::ClosedHat,
        Track::OpenHat,
        Track::HighTom,
        Track::LowTom,
        Track::Ride,
        Track::Crash,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: usize) -> Option<Track> {
        Self::ALL.get(index).copied()
    }

    /// Level of a full-velocity hit at full track volume. The samples are
    /// all peak-normalized; this balances them as a kit.
    pub fn mix_gain(self) -> f32 {
        match self {
            Track::Kick => 0.9,
            Track::Snare => 0.75,
            Track::Sidestick => 0.55,
            Track::Clap => 0.6,
            Track::ClosedHat => 0.45,
            Track::OpenHat => 0.4,
            Track::HighTom => 0.65,
            Track::LowTom => 0.7,
            Track::Ride => 0.4,
            Track::Crash => 0.45,
        }
    }

    fn bundled_wav(self) -> &'static [u8] {
        match self {
            Track::Kick => include_bytes!("../../resources/drums/kick.wav"),
            Track::Snare => include_bytes!("../../resources/drums/snare.wav"),
            Track::Sidestick => include_bytes!("../../resources/drums/sidestick.wav"),
            Track::Clap => include_bytes!("../../resources/drums/clap.wav"),
            Track::ClosedHat => include_bytes!("../../resources/drums/hihat-closed.wav"),
            Track::OpenHat => include_bytes!("../../resources/drums/hihat-open.wav"),
            Track::HighTom => include_bytes!("../../resources/drums/tom-high.wav"),
            Track::LowTom => include_bytes!("../../resources/drums/tom-low.wav"),
            Track::Ride => include_bytes!("../../resources/drums/ride.wav"),
            Track::Crash => include_bytes!("../../resources/drums/crash.wav"),
        }
    }
}

/// Every track's sample at one output rate, indexed by [`Track::index`].
pub type KitSamples = [Vec<f32>; TRACK_COUNT];

/// Decoded samples at their recorded rates, plus resampled copies per output
/// rate. Resampling happens off the audio thread (startup and
/// [`crate::audio::Source::prepare`]); each rate is converted once.
pub struct Kit {
    originals: [Decoded; TRACK_COUNT],
    by_rate: Mutex<HashMap<u32, Arc<KitSamples>>>,
}

impl Kit {
    /// Decodes the samples compiled into the app.
    pub fn bundled() -> Result<Self, WavError> {
        let mut originals = Vec::with_capacity(TRACK_COUNT);
        for track in Track::ALL {
            originals.push(decode_wav(track.bundled_wav())?);
        }
        Ok(Self::from_decoded(originals))
    }

    /// A kit of silent samples (used if the bundled kit cannot be decoded).
    pub fn silent() -> Self {
        Self::from_decoded(
            (0..TRACK_COUNT)
                .map(|_| Decoded {
                    sample_rate: 48_000,
                    samples: Vec::new(),
                })
                .collect(),
        )
    }

    /// Builds a kit from decoded samples in [`Track::ALL`] order.
    ///
    /// # Panics
    /// If `originals` does not hold exactly [`TRACK_COUNT`] samples.
    pub fn from_decoded(originals: Vec<Decoded>) -> Self {
        let originals: [Decoded; TRACK_COUNT] = originals
            .try_into()
            .unwrap_or_else(|_| panic!("a kit needs {TRACK_COUNT} samples"));
        Self {
            originals,
            by_rate: Mutex::new(HashMap::new()),
        }
    }

    pub fn original(&self, track: Track) -> &Decoded {
        &self.originals[track.index()]
    }

    /// The kit at `rate` Hz, converting it on first use. Not for the audio thread.
    pub fn at_rate(&self, rate: u32) -> Arc<KitSamples> {
        let mut cache = self.by_rate.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(cache.entry(rate).or_insert_with(|| {
            Arc::new(std::array::from_fn(|index| {
                let original = &self.originals[index];
                resample(&original.samples, original.sample_rate, rate)
            }))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_sample_decodes_with_its_onset_at_the_start() {
        let kit = Kit::bundled().expect("bundled kit decodes");
        for track in Track::ALL {
            let sample = kit.original(track);
            assert!(
                [44_100, 48_000].contains(&sample.sample_rate),
                "{track:?} rate {}",
                sample.sample_rate
            );
            let peak = sample.samples.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.8 && peak <= 1.0, "{track:?} peak {peak}");
            // The hit starts within the first millisecond, so it sounds on its step.
            let onset = sample
                .samples
                .iter()
                .position(|v| v.abs() >= peak * 0.03)
                .unwrap();
            assert!(
                onset < sample.sample_rate as usize / 1_000,
                "{track:?} onset at {onset}"
            );
        }
    }

    #[test]
    fn each_rate_is_converted_once_and_shared() {
        let kit = Kit::from_decoded(
            (0..TRACK_COUNT)
                .map(|i| Decoded {
                    sample_rate: 44_100,
                    samples: vec![i as f32 * 0.1; 441],
                })
                .collect(),
        );
        let first = kit.at_rate(48_000);
        assert!(Arc::ptr_eq(&first, &kit.at_rate(48_000)));
        assert_eq!(first[0].len(), 480);
        assert_eq!(kit.at_rate(44_100)[3], kit.original(Track::Clap).samples);
    }
}
