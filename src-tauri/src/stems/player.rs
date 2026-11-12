//! Wires a song's tracks into an engine [`StemSource`] and its [`Feeder`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rtrb::RingBuffer;

use super::feeder::{Block, Feeder, MAX_TRACKS, POOL_BLOCKS, REQUEST_CAPACITY};
use super::reader::SongReader;
use super::source::{PlayerShared, StemSource};
use super::track::TrackReader;

/// Loop crossfade at the song rate.
const LOOP_CROSSFADE_SECS: f64 = 0.01;
/// How often the feeder thread looks for free blocks.
const FEEDER_POLL: Duration = Duration::from_millis(2);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlayerError {
    #[error("a song needs 1 to {MAX_TRACKS} tracks")]
    TrackCount,
    #[error("all tracks must share one sample rate")]
    MixedRates,
}

/// A playable song: attach `source` to the engine, run `feeder` on its own
/// thread ([`FeederThread`]) or call [`Feeder::fill`] directly in tests, and
/// read playback state from `shared`.
pub struct StemPlayer {
    pub source: StemSource,
    pub feeder: Feeder,
    pub shared: Arc<PlayerShared>,
    pub song_rate: u32,
    pub frames: u64,
}

impl StemPlayer {
    /// `start` is the song frame playback begins at.
    pub fn new(tracks: Vec<Box<dyn TrackReader>>, start: u64) -> Result<Self, PlayerError> {
        if tracks.is_empty() || tracks.len() > MAX_TRACKS {
            return Err(PlayerError::TrackCount);
        }
        let song_rate = tracks[0].sample_rate();
        if tracks.iter().any(|track| track.sample_rate() != song_rate) {
            return Err(PlayerError::MixedRates);
        }
        let track_count = tracks.len();
        let crossfade = (f64::from(song_rate) * LOOP_CROSSFADE_SECS).round() as u64;
        let reader = SongReader::new(tracks, crossfade);
        let frames = reader.frames();
        let channels = reader.channels();

        let (filled_tx, filled_rx) = RingBuffer::new(POOL_BLOCKS);
        let (mut used_tx, used_rx) = RingBuffer::new(POOL_BLOCKS);
        let (request_tx, request_rx) = RingBuffer::new(REQUEST_CAPACITY);
        for _ in 0..POOL_BLOCKS {
            let _ = used_tx.push(Box::new(Block::new(channels)));
        }

        let shared = Arc::new(PlayerShared::default());
        let source = StemSource::new(
            track_count,
            frames,
            start.min(frames),
            filled_rx,
            used_tx,
            request_tx,
            Arc::clone(&shared),
        );
        let feeder = Feeder::new(reader, song_rate, request_rx, filled_tx, used_rx);
        Ok(Self {
            source,
            feeder,
            shared,
            song_rate,
            frames,
        })
    }
}

/// Runs a [`Feeder`] on its own thread until dropped.
pub struct FeederThread {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FeederThread {
    pub fn spawn(mut feeder: Feeder) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("stem-feeder".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    feeder.fill();
                    std::thread::sleep(FEEDER_POLL);
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for FeederThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
