//! Moves samples from the input ring into the analyzer, keeping the analysis
//! about *now*: a backlog older than [`MAX_BACKLOG_SECS`] (the analysis thread
//! was held up) is dropped, and a gap in the input (the ring overflowed, so
//! the callback dropped samples) is never analysed as one continuous signal.
//! Either way the analyzer starts fresh on the newest samples.

use rtrb::Consumer;

use super::analyzer::{Analyzer, Frame};

/// Oldest audio worth analysing when the thread falls behind. Long enough
/// for the lowest note (two periods of B0 ≈ 65 ms), short enough that a
/// note that already ended is not shown as current.
pub const MAX_BACKLOG_SECS: f64 = 0.1;

pub struct InputPump {
    analyzer: Analyzer,
    max_backlog: usize,
    /// Overflow count already accounted for.
    overflows: u64,
    /// Times the input restarted from fresh samples (backlog or gap).
    restarts: u64,
}

impl InputPump {
    /// `overflows` is the ring's overflow count right now.
    pub fn new(analyzer: Analyzer, overflows: u64) -> Self {
        let max_backlog = (analyzer.sample_rate() * MAX_BACKLOG_SECS).round() as usize;
        Self {
            analyzer,
            max_backlog,
            overflows,
            restarts: 0,
        }
    }

    pub fn analyzer_mut(&mut self) -> &mut Analyzer {
        &mut self.analyzer
    }

    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// Reads everything the ring holds and returns the newest frame (`None`
    /// when no analysis step completed). `overflows` is the ring's overflow
    /// count now.
    pub fn pump(&mut self, samples: &mut Consumer<f32>, overflows: u64) -> Option<Frame> {
        if overflows != self.overflows {
            // Samples were dropped after everything now in the ring: what is
            // there is stale and does not join up with what comes next.
            self.overflows = overflows;
            let stale = samples.slots();
            if let Ok(chunk) = samples.read_chunk(stale) {
                chunk.commit_all();
            }
            self.restart();
            return None;
        }
        let mut available = samples.slots();
        if available > self.max_backlog {
            let skip = available - self.max_backlog;
            if let Ok(chunk) = samples.read_chunk(skip) {
                chunk.commit_all();
            }
            available -= skip;
            self.restart();
        }
        let chunk = samples.read_chunk(available).ok()?;
        let (first, second) = chunk.as_slices();
        let mut newest = None;
        for part in [first, second] {
            self.analyzer.push(part, |frame| newest = Some(*frame));
        }
        chunk.commit_all();
        newest
    }

    fn restart(&mut self) {
        self.analyzer.reset();
        self.restarts += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use rtrb::RingBuffer;

    use super::*;

    const RATE: f64 = 48_000.0;
    const BLOCK: usize = 64;

    /// A steady sine from sample `from`, at `freq`.
    struct Tone {
        freq: f64,
        index: u64,
    }

    impl Tone {
        fn take(&mut self, len: usize) -> Vec<f32> {
            (0..len)
                .map(|_| {
                    let t = self.index as f64 / RATE;
                    self.index += 1;
                    (0.3 * (TAU * self.freq * t).sin()) as f32
                })
                .collect()
        }
    }

    fn midi(freq: f64) -> i32 {
        (69.0 + 12.0 * (freq / 440.0).log2()).round() as i32
    }

    /// Feeds `secs` of `tone` in callback-sized blocks, pumping after each.
    fn play(
        pump: &mut InputPump,
        producer: &mut rtrb::Producer<f32>,
        consumer: &mut Consumer<f32>,
        tone: &mut Tone,
        secs: f64,
        mut on_frame: impl FnMut(&Frame),
    ) {
        for _ in 0..(secs * RATE) as usize / BLOCK {
            for sample in tone.take(BLOCK) {
                producer.push(sample).expect("room");
            }
            if let Some(frame) = pump.pump(consumer, 0) {
                on_frame(&frame);
            }
        }
    }

    #[test]
    fn a_stalled_analysis_skips_old_audio_and_catches_up() {
        let (mut producer, mut consumer) = RingBuffer::new(1 << 16);
        let mut pump = InputPump::new(Analyzer::new(48_000, 440.0), 0);
        let mut tone = Tone {
            freq: 110.0,
            index: 0,
        };
        let mut last = None;
        play(
            &mut pump,
            &mut producer,
            &mut consumer,
            &mut tone,
            0.3,
            |f| {
                last = f.pitch;
            },
        );
        assert_eq!(last.map(|p| p.midi), Some(midi(110.0)));

        // The analysis thread stalls for 300 ms while the note changes.
        tone.freq = 146.832;
        for sample in tone.take((0.3 * RATE) as usize) {
            producer.push(sample).expect("room");
        }
        let frame = pump.pump(&mut consumer, 0).expect("a frame");
        assert_eq!(pump.restarts(), 1, "backlog dropped");
        // The newest frame is about the newest audio: never the old A2.
        assert_ne!(frame.pitch.map(|p| p.midi), Some(midi(110.0)));
        // At most the backlog limit was analysed.
        assert!(
            frame.sample <= (MAX_BACKLOG_SECS * RATE) as u64,
            "{}",
            frame.sample
        );
        // The new note shows within the backlog plus 50 ms of live input.
        let mut shown_after = if frame.pitch.map(|p| p.midi) == Some(midi(146.832)) {
            Some(0.0)
        } else {
            None
        };
        let mut elapsed = 0.0;
        play(
            &mut pump,
            &mut producer,
            &mut consumer,
            &mut tone,
            0.1,
            |f| {
                elapsed += BLOCK as f64 / RATE;
                if let Some(p) = f.pitch {
                    assert_ne!(p.midi, midi(110.0), "old note after the stall");
                    if p.midi == midi(146.832) {
                        shown_after.get_or_insert(elapsed);
                    }
                }
            },
        );
        let shown_after = shown_after.expect("new note shown");
        assert!(
            shown_after <= 0.05,
            "D3 {:.1} ms after resuming",
            shown_after * 1000.0
        );
    }

    #[test]
    fn an_overflow_is_a_gap_not_a_join() {
        let capacity = 4096;
        let (mut producer, mut consumer) = RingBuffer::new(capacity);
        let mut pump = InputPump::new(Analyzer::new(48_000, 440.0), 0);
        let mut tone = Tone {
            freq: 220.0,
            index: 0,
        };
        let mut overflows = 0_u64;
        let mut last = None;
        for _ in 0..(0.3 * RATE) as usize / BLOCK {
            for sample in tone.take(BLOCK) {
                producer.push(sample).expect("room");
            }
            last = pump
                .pump(&mut consumer, overflows)
                .and_then(|f| f.pitch)
                .or(last);
        }
        assert_eq!(last.map(|p| p.midi), Some(midi(220.0)));

        // The reader stops; the ring fills and the callback drops samples.
        tone.freq = 329.628;
        for sample in tone.take(capacity * 3) {
            if producer.push(sample).is_err() {
                overflows += 1;
            }
        }
        assert!(overflows > 0);
        assert_eq!(pump.pump(&mut consumer, overflows), None);
        assert_eq!(consumer.slots(), 0, "stale samples dropped");
        assert_eq!(pump.restarts(), 1);

        // Fresh input after the gap: no old note, the new one within 50 ms.
        let mut elapsed = 0.0;
        let mut shown_after = None;
        for _ in 0..(0.1 * RATE) as usize / BLOCK {
            for sample in tone.take(BLOCK) {
                producer.push(sample).expect("room");
            }
            elapsed += BLOCK as f64 / RATE;
            if let Some(p) = pump.pump(&mut consumer, overflows).and_then(|f| f.pitch) {
                assert_ne!(p.midi, midi(220.0), "note from before the gap");
                if p.midi == midi(329.628) {
                    shown_after.get_or_insert(elapsed);
                }
            }
        }
        let shown_after = shown_after.expect("new note shown");
        assert!(
            shown_after <= 0.05,
            "E4 {:.1} ms after the gap",
            shown_after * 1000.0
        );
    }
}
