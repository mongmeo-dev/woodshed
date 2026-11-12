//! Equal-tempered note math relative to an adjustable A4.

/// Reference pitch range for A4, in Hz.
pub const MIN_REFERENCE_HZ: u16 = 430;
pub const MAX_REFERENCE_HZ: u16 = 450;
pub const DEFAULT_REFERENCE_HZ: u16 = 440;

/// MIDI number of A4.
const A4_MIDI: f64 = 69.0;

/// Clamps a reference pitch into the supported range.
pub fn clamp_reference(hz: u16) -> u16 {
    hz.clamp(MIN_REFERENCE_HZ, MAX_REFERENCE_HZ)
}

/// Fractional MIDI note number of `frequency` (69.0 = A4 at `reference`).
pub fn midi_of(frequency: f64, reference: f64) -> f64 {
    A4_MIDI + 12.0 * (frequency / reference).log2()
}

/// Frequency of a (fractional) MIDI note number.
pub fn frequency_of(midi: f64, reference: f64) -> f64 {
    reference * ((midi - A4_MIDI) / 12.0).exp2()
}

/// Distance from `from` to `to` in cents.
pub fn cents_between(from: f64, to: f64) -> f64 {
    1200.0 * (to / from).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_follow_the_reference() {
        assert!((midi_of(440.0, 440.0) - 69.0).abs() < 1e-12);
        assert!((midi_of(30.8677, 440.0) - 23.0).abs() < 1e-4, "B0");
        assert!((midi_of(82.4069, 440.0) - 40.0).abs() < 1e-4, "E2");
        assert!((frequency_of(69.0, 432.0) - 432.0).abs() < 1e-12);
        assert!((cents_between(440.0, 880.0) - 1200.0).abs() < 1e-9);
        assert_eq!(clamp_reference(420), 430);
        assert_eq!(clamp_reference(455), 450);
    }
}
