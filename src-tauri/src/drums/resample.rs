//! Offline sample-rate conversion for one-shot samples (windowed sinc with a
//! tabulated kernel). Runs off the audio thread, when the kit is prepared for
//! an output rate.

use std::f64::consts::PI;

/// Kernel zero crossings on each side of the centre (at the output rate's
/// band limit). More is sharper and slower.
const ZERO_CROSSINGS: f64 = 16.0;
/// Kernel table resolution: entries per input sample.
const TABLE_STEPS: usize = 512;
/// Pass band as a fraction of the lower Nyquist frequency.
const PASS_BAND: f64 = 0.94;

/// Converts `input` from `from` Hz to `to` Hz. Sample 0 stays at sample 0, so
/// a hit's onset keeps its position.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    // Cut-off relative to the input rate's Nyquist frequency.
    let cutoff = ratio.min(1.0) * PASS_BAND;
    let half_width = ZERO_CROSSINGS / cutoff;
    let reach = half_width.ceil() as isize;
    let table: Vec<f64> = (0..=(reach as usize + 1) * TABLE_STEPS)
        .map(|index| kernel(index as f64 / TABLE_STEPS as f64, cutoff, half_width))
        .collect();

    let last = input.len() as isize - 1;
    // ceil(len * to / from) in integers, so exact ratios give exact lengths.
    let out_len = (input.len() as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
    (0..out_len)
        .map(|index| {
            let position = index as f64 / ratio;
            let centre = position.floor() as isize;
            let mut acc = 0.0;
            for tap in (centre - reach + 1).max(0)..=(centre + reach).min(last) {
                let at = (position - tap as f64).abs() * TABLE_STEPS as f64;
                let whole = at as usize;
                let frac = at - whole as f64;
                let weight = table[whole] + (table[whole + 1] - table[whole]) * frac;
                acc += f64::from(input[tap as usize]) * weight;
            }
            acc as f32
        })
        .collect()
}

/// Blackman-windowed sinc at distance `x` input samples from the centre.
fn kernel(x: f64, cutoff: f64, half_width: f64) -> f64 {
    if x >= half_width {
        return 0.0;
    }
    let sinc = if x == 0.0 {
        1.0
    } else {
        let arg = PI * cutoff * x;
        arg.sin() / arg
    };
    let phase = PI * x / half_width;
    let window = 0.42 + 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
    cutoff * sinc * window
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (2.0 * PI * hz * i as f64 / f64::from(rate)).sin() as f32)
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |max, &v| max.max(v.abs()))
    }

    #[test]
    fn same_rate_is_a_copy() {
        let input = sine(48_000, 440.0, 100);
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }

    #[test]
    fn length_follows_the_rate_ratio() {
        let input = vec![0.0; 44_100];
        assert_eq!(resample(&input, 44_100, 48_000).len(), 48_000);
        assert_eq!(resample(&input, 44_100, 22_050).len(), 22_050);
    }

    #[test]
    fn a_sine_keeps_its_frequency_and_level() {
        for (from, to) in [(44_100, 48_000), (48_000, 44_100), (44_100, 96_000)] {
            let out = resample(&sine(from, 1_000.0, 8_000), from, to);
            let expected = sine(to, 1_000.0, out.len());
            // Skip the edges, where the kernel runs out of input.
            let body = 200..out.len() - 200;
            let error = out[body.clone()]
                .iter()
                .zip(&expected[body])
                .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()));
            assert!(error < 0.01, "{from} -> {to}: error {error}");
        }
    }

    #[test]
    fn an_onset_stays_at_sample_zero() {
        let mut click = vec![0.0; 2_000];
        click[0] = 1.0;
        let out = resample(&click, 44_100, 48_000);
        let loudest = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(index, _)| index);
        assert_eq!(loudest, Some(0));
    }

    #[test]
    fn content_above_the_new_nyquist_is_removed() {
        // 20 kHz cannot exist at 22.05 kHz (Nyquist 11.025 kHz).
        let out = resample(&sine(44_100, 20_000.0, 8_000), 44_100, 22_050);
        assert!(peak(&out[200..out.len() - 200]) < 0.01);
    }
}
