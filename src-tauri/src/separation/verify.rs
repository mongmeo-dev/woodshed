//! Guards against wrong GPU predictions.
//!
//! On the GPU, Core ML occasionally returns a wrong result for a chunk:
//! measured on an M5 with macOS 27, about one prediction in five went wrong
//! once the GPU had idled between chunks, with stem errors up to 3e-2
//! (chunk SNR 26–60 dB against PyTorch instead of 120 dB); it happens far
//! less often back to back. Right GPU results are bit-identical from run to
//! run and wrong ones differ from each other, so a result is accepted once
//! two runs agree. A run with NaN or infinity never agrees. If
//! [`MAX_ATTEMPTS`] runs never agree, the chunk runs on the CPU, which is
//! deterministic (max error 7e-6) but several times slower. This is a
//! mitigation from what was measured on that machine, not an independent
//! check of correctness.

use super::model::{SeparationError, StemModel};

/// GPU runs per chunk before falling back to the CPU.
pub const MAX_ATTEMPTS: usize = 4;
/// Right runs agree exactly; wrong ones were at least 7e-4 apart.
const AGREE_TOLERANCE: f32 = 1e-5;

/// Makes the fallback model when it is first needed.
pub type FallbackLoader = Box<dyn FnMut() -> Result<Box<dyn StemModel>, SeparationError>>;

/// How the guarded predictions went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifyStats {
    pub chunks: usize,
    /// Primary runs beyond the two each chunk needs at least.
    pub extra_runs: usize,
    /// Chunks that ran on the fallback.
    pub fallbacks: usize,
}

/// A model that answers only with results two primary runs agree on, or
/// with the fallback's.
pub struct VerifiedModel {
    primary: Box<dyn StemModel>,
    fallback: Option<Box<dyn StemModel>>,
    load_fallback: FallbackLoader,
    previous_spec: Vec<f32>,
    previous_wave: Vec<f32>,
    fingerprints: Vec<u64>,
    stats: VerifyStats,
}

/// A hash of every bit of both outputs.
fn fingerprint(spec_out: &[f32], wave_out: &[f32]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for &value in spec_out.iter().chain(wave_out) {
        hash =
            (hash.rotate_left(5) ^ u64::from(value.to_bits())).wrapping_mul(0x5170_7CC1_B727_220B);
    }
    hash
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| (x - y).abs() <= AGREE_TOLERANCE)
}

impl VerifiedModel {
    pub fn new(primary: Box<dyn StemModel>, load_fallback: FallbackLoader) -> Self {
        Self {
            primary,
            fallback: None,
            load_fallback,
            previous_spec: Vec::new(),
            previous_wave: Vec::new(),
            fingerprints: Vec::with_capacity(MAX_ATTEMPTS),
            stats: VerifyStats::default(),
        }
    }

    pub fn stats(&self) -> VerifyStats {
        self.stats
    }

    fn run_fallback(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        self.stats.fallbacks += 1;
        let fallback = match &mut self.fallback {
            Some(model) => model,
            slot => slot.insert((self.load_fallback)()?),
        };
        fallback.predict(mix, spec, spec_out, wave_out)?;
        if !finite(spec_out) || !finite(wave_out) {
            return Err(SeparationError::Model(
                "the model returned NaN or infinity".into(),
            ));
        }
        Ok(())
    }
}

impl StemModel for VerifiedModel {
    fn predict(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        self.stats.chunks += 1;
        self.previous_spec.resize(spec_out.len(), 0.0);
        self.previous_wave.resize(wave_out.len(), 0.0);
        self.fingerprints.clear();
        for attempt in 0..MAX_ATTEMPTS {
            if let Err(error) = self.primary.predict(mix, spec, spec_out, wave_out) {
                log::warn!("stem model run failed, using the fallback: {error}");
                return self.run_fallback(mix, spec, spec_out, wave_out);
            }
            if !finite(spec_out) || !finite(wave_out) {
                // Never agrees, not even with an identical NaN run.
                log::warn!("stem model run returned NaN or infinity");
                continue;
            }
            let print = fingerprint(spec_out, wave_out);
            let agrees = attempt > 0
                && (self.fingerprints.contains(&print)
                    || (close(spec_out, &self.previous_spec)
                        && close(wave_out, &self.previous_wave)));
            if agrees {
                if attempt > 1 {
                    self.stats.extra_runs += attempt - 1;
                    log::info!("stem model runs agreed after {} attempts", attempt + 1);
                }
                return Ok(());
            }
            self.fingerprints.push(print);
            self.previous_spec.copy_from_slice(spec_out);
            self.previous_wave.copy_from_slice(wave_out);
        }
        self.stats.extra_runs += MAX_ATTEMPTS - 2;
        log::warn!("stem model runs disagreed {MAX_ATTEMPTS} times, using the fallback");
        self.run_fallback(mix, spec, spec_out, wave_out)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// The right answer for an input: deterministic.
    fn answer(mix: &[f32], spec_out: &mut [f32], wave_out: &mut [f32]) {
        for (i, o) in spec_out.iter_mut().enumerate() {
            *o = mix[i % mix.len()] * 0.5 + i as f32 * 1e-3;
        }
        for (i, o) in wave_out.iter_mut().enumerate() {
            *o = mix[i % mix.len()] * 0.25;
        }
    }

    /// Wrong on the runs `script` marks `true`, each time differently.
    struct Flaky {
        script: Vec<bool>,
        runs: Rc<Cell<usize>>,
        fail_with_error: bool,
    }

    impl StemModel for Flaky {
        fn predict(
            &mut self,
            mix: &[f32],
            _spec: &[f32],
            spec_out: &mut [f32],
            wave_out: &mut [f32],
        ) -> Result<(), SeparationError> {
            let run = self.runs.get();
            self.runs.set(run + 1);
            if self.fail_with_error {
                return Err(SeparationError::Model("gpu".into()));
            }
            answer(mix, spec_out, wave_out);
            if self.script.get(run).copied().unwrap_or(false) {
                // Like the GPU: a few values off, differently every time.
                spec_out[run % spec_out.len()] += 0.01 * (run + 1) as f32;
                wave_out[(run * 3) % wave_out.len()] -= 0.003;
            }
            Ok(())
        }
    }

    /// The fallback marks its results so the test can tell them apart.
    struct Cpu(Rc<Cell<usize>>);

    impl StemModel for Cpu {
        fn predict(
            &mut self,
            mix: &[f32],
            _spec: &[f32],
            spec_out: &mut [f32],
            wave_out: &mut [f32],
        ) -> Result<(), SeparationError> {
            self.0.set(self.0.get() + 1);
            answer(mix, spec_out, wave_out);
            wave_out[0] = 42.0;
            Ok(())
        }
    }

    fn model(
        script: &[bool],
        fail_with_error: bool,
    ) -> (VerifiedModel, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let runs = Rc::new(Cell::new(0));
        let cpu_runs = Rc::new(Cell::new(0));
        let loads = cpu_runs.clone();
        let primary = Flaky {
            script: script.to_vec(),
            runs: runs.clone(),
            fail_with_error,
        };
        let verified = VerifiedModel::new(
            Box::new(primary),
            Box::new(move || Ok(Box::new(Cpu(loads.clone())) as Box<dyn StemModel>)),
        );
        (verified, runs, cpu_runs)
    }

    fn predict(model: &mut VerifiedModel) -> (Vec<f32>, Vec<f32>) {
        let mix = [0.1, -0.2, 0.3, 0.4];
        let (mut spec_out, mut wave_out) = (vec![0.0; 24], vec![0.0; 12]);
        model
            .predict(&mix, &[], &mut spec_out, &mut wave_out)
            .unwrap();
        (spec_out, wave_out)
    }

    fn right() -> (Vec<f32>, Vec<f32>) {
        let (mut spec_out, mut wave_out) = (vec![0.0; 24], vec![0.0; 12]);
        answer(&[0.1, -0.2, 0.3, 0.4], &mut spec_out, &mut wave_out);
        (spec_out, wave_out)
    }

    #[test]
    fn two_agreeing_runs_are_enough() {
        let (mut model, runs, cpu) = model(&[], false);
        assert_eq!(predict(&mut model), right());
        assert_eq!((runs.get(), cpu.get()), (2, 0));
        assert_eq!(
            model.stats(),
            VerifyStats {
                chunks: 1,
                extra_runs: 0,
                fallbacks: 0
            }
        );
    }

    #[test]
    fn a_wrong_run_is_outvoted() {
        // Wrong first, then right twice; and right, wrong, right (the third
        // agrees with the first, not with its neighbour).
        for script in [vec![true, false, false], vec![false, true, false]] {
            let (mut model, runs, cpu) = model(&script, false);
            assert_eq!(predict(&mut model), right(), "{script:?}");
            assert_eq!((runs.get(), cpu.get()), (3, 0), "{script:?}");
            assert_eq!(model.stats().extra_runs, 1);
        }
    }

    #[test]
    fn runs_that_never_agree_fall_back() {
        let (mut model, runs, cpu) = model(&[true; MAX_ATTEMPTS], false);
        let (spec_out, wave_out) = predict(&mut model);
        assert_eq!(wave_out[0], 42.0, "the fallback answered");
        assert_eq!(spec_out, right().0);
        assert_eq!((runs.get(), cpu.get()), (MAX_ATTEMPTS, 1));
        assert_eq!(model.stats().fallbacks, 1);
        // The next chunk tries the primary again.
        predict(&mut model);
        assert_eq!(runs.get(), MAX_ATTEMPTS + 2);
    }

    /// Returns the same NaN-tainted result every time.
    struct Nan(Rc<Cell<usize>>);

    impl StemModel for Nan {
        fn predict(
            &mut self,
            mix: &[f32],
            _spec: &[f32],
            spec_out: &mut [f32],
            wave_out: &mut [f32],
        ) -> Result<(), SeparationError> {
            self.0.set(self.0.get() + 1);
            answer(mix, spec_out, wave_out);
            spec_out[3] = f32::NAN;
            wave_out[1] = f32::INFINITY;
            Ok(())
        }
    }

    #[test]
    fn identical_non_finite_runs_never_agree() {
        let runs = Rc::new(Cell::new(0));
        let cpu_runs = Rc::new(Cell::new(0));
        let loads = cpu_runs.clone();
        let mut model = VerifiedModel::new(
            Box::new(Nan(runs.clone())),
            Box::new(move || Ok(Box::new(Cpu(loads.clone())) as Box<dyn StemModel>)),
        );
        let (spec_out, wave_out) = predict(&mut model);
        assert!(spec_out.iter().chain(&wave_out).all(|v| v.is_finite()));
        assert_eq!(wave_out[0], 42.0, "the fallback answered");
        assert_eq!((runs.get(), cpu_runs.get()), (MAX_ATTEMPTS, 1));

        // A fallback that is not finite either is an error, not a result.
        let mut model = VerifiedModel::new(
            Box::new(Nan(Rc::new(Cell::new(0)))),
            Box::new(|| Ok(Box::new(Nan(Rc::new(Cell::new(0)))) as Box<dyn StemModel>)),
        );
        let (mut spec_out, mut wave_out) = (vec![0.0; 24], vec![0.0; 12]);
        assert!(
            model
                .predict(&[0.1, 0.2], &[], &mut spec_out, &mut wave_out)
                .is_err()
        );
    }

    #[test]
    fn a_failing_primary_falls_back() {
        let (mut model, runs, cpu) = model(&[], true);
        let (_, wave_out) = predict(&mut model);
        assert_eq!(wave_out[0], 42.0);
        assert_eq!((runs.get(), cpu.get()), (1, 1));
    }
}
