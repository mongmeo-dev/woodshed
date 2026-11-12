//! The htdemucs_6s core as a compiled Core ML model, called through
//! `objc2-core-ml`. Runs on the GPU (`MLComputeUnits::CPUAndGPU`): the fp32
//! model matches PyTorch there, while the Neural Engine is slow and the
//! default (`All`) drifts (see README "스템 분리 모델"). GPU results are
//! checked by `verify::VerifiedModel`, which falls back to a CPU instance.
//!
//! Core ML objects are neither `Send` nor `Sync`: load and predict on one
//! thread (the separation worker).

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLFeatureValue, MLModel,
    MLModelConfiguration, MLMultiArray, MLMultiArrayDataType,
};
use objc2_foundation::{NSArray, NSDictionary, NSInteger, NSNumber, NSString, NSURL};

use super::dsp::{FRAMES, FREQS, SEGMENT, SOURCES};
use super::model::{SeparationError, StemModel};

const MIX_SHAPE: [usize; 3] = [1, 2, SEGMENT];
const SPEC_SHAPE: [usize; 4] = [1, 4, FREQS, FRAMES];
const SPEC_OUT_SHAPE: [usize; 4] = [1, SOURCES * 4, FREQS, FRAMES];
const WAVE_OUT_SHAPE: [usize; 3] = [1, SOURCES * 2, SEGMENT];

/// Where Core ML runs the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeUnits {
    /// `CPUAndGPU`: fast; checked by `VerifiedModel`.
    Gpu,
    /// `CPUOnly`: deterministic, several times slower.
    Cpu,
}

pub struct CoreMlModel {
    model: Retained<MLModel>,
}

fn numbers(values: &[usize]) -> Retained<NSArray<NSNumber>> {
    let items: Vec<Retained<NSNumber>> = values
        .iter()
        .map(|&value| NSNumber::new_isize(value as isize))
        .collect();
    NSArray::from_retained_slice(&items)
}

fn sizes(array: &NSArray<NSNumber>) -> Vec<usize> {
    array.iter().map(|n| n.as_isize().max(0) as usize).collect()
}

fn model_error(context: &str, error: &objc2_foundation::NSError) -> SeparationError {
    SeparationError::Model(format!("{context}: {}", error.localizedDescription()))
}

fn new_array(shape: &[usize]) -> Result<Retained<MLMultiArray>, SeparationError> {
    // SAFETY: a fresh float32 array of a positive shape.
    unsafe {
        MLMultiArray::initWithShape_dataType_error(
            MLMultiArray::alloc(),
            &numbers(shape),
            MLMultiArrayDataType::Float32,
        )
    }
    .map_err(|error| model_error("cannot allocate an input array", &error))
}

/// Calls `row(contiguous_offset, strided_offset, len)` for every innermost
/// row of a `shape` array laid out with `strides` (in elements), after
/// checking every strided index stays below `capacity` elements.
fn for_each_row(
    shape: &[usize],
    strides: &[usize],
    capacity: usize,
    mut row: impl FnMut(usize, usize, usize, usize),
) -> Result<(), String> {
    if shape.len() != strides.len() || shape.is_empty() {
        return Err(format!("shape {shape:?} and strides {strides:?} differ"));
    }
    let last = shape.len() - 1;
    let end = shape
        .iter()
        .zip(strides)
        .map(|(&n, &s)| n.saturating_sub(1) * s)
        .sum::<usize>()
        + 1;
    if end > capacity {
        return Err(format!(
            "strides {strides:?} reach past the buffer ({end} > {capacity})"
        ));
    }
    let width = shape[last];
    let rows: usize = shape[..last].iter().product();
    let mut index = vec![0_usize; last];
    for r in 0..rows {
        let offset: usize = index.iter().zip(strides).map(|(i, s)| i * s).sum();
        row(r * width, offset, width, strides[last]);
        for d in (0..last).rev() {
            index[d] += 1;
            if index[d] < shape[d] {
                break;
            }
            index[d] = 0;
        }
    }
    Ok(())
}

/// Copies contiguous `data` into `array` (float32 of `shape`), honouring the
/// strides Core ML hands the handler.
fn write_array(array: &MLMultiArray, shape: &[usize], data: &[f32]) -> Result<(), SeparationError> {
    let failure: RefCell<Option<String>> = RefCell::new(None);
    let block = RcBlock::new(
        |ptr: NonNull<c_void>, size: NSInteger, strides: NonNull<NSArray<NSNumber>>| {
            // SAFETY: Core ML passes a valid strides array for the call.
            let strides = sizes(unsafe { strides.as_ref() });
            let dst = ptr.as_ptr().cast::<f32>();
            let capacity = size.max(0) as usize / size_of::<f32>();
            let result = for_each_row(shape, &strides, capacity, |from, to, len, step| {
                for i in 0..len {
                    // SAFETY: `for_each_row` checked every strided index
                    // against the buffer, and `data` holds the whole shape.
                    unsafe { *dst.add(to + i * step) = data[from + i] };
                }
            });
            if let Err(message) = result {
                *failure.borrow_mut() = Some(message);
            }
        },
    );
    // SAFETY: the handler runs synchronously inside this call.
    unsafe { array.getMutableBytesWithHandler(&block) };
    match failure.take() {
        Some(message) => Err(SeparationError::Model(message)),
        None => Ok(()),
    }
}

/// Copies output `name` (float32 of `shape`, any strides) into `out`.
fn read_output(
    output: &ProtocolObject<dyn MLFeatureProvider>,
    name: &str,
    shape: &[usize],
    out: &mut [f32],
) -> Result<(), SeparationError> {
    // SAFETY: plain Core ML getters on a live prediction result.
    let array = unsafe { output.featureValueForName(&NSString::from_str(name)) }
        .and_then(|value| unsafe { value.multiArrayValue() })
        .ok_or_else(|| SeparationError::Model(format!("output {name} is missing")))?;
    let shape_array = unsafe { array.shape() };
    let actual = sizes(&shape_array);
    if actual != shape {
        return Err(SeparationError::Model(format!(
            "output {name} has shape {actual:?}, expected {shape:?}"
        )));
    }
    if unsafe { array.dataType() } != MLMultiArrayDataType::Float32 {
        return Err(SeparationError::Model(format!(
            "output {name} is not float32"
        )));
    }
    let stride_array = unsafe { array.strides() };
    let strides = sizes(&stride_array);
    let failure: RefCell<Option<String>> = RefCell::new(None);
    let out = RefCell::new(out);
    let block = RcBlock::new(|ptr: NonNull<c_void>, size: NSInteger| {
        let src = ptr.as_ptr().cast::<f32>().cast_const();
        let capacity = size.max(0) as usize / size_of::<f32>();
        let mut out = out.borrow_mut();
        let result = for_each_row(shape, &strides, capacity, |to, from, len, step| {
            for i in 0..len {
                // SAFETY: checked against the buffer by `for_each_row`.
                out[to + i] = unsafe { *src.add(from + i * step) };
            }
        });
        if let Err(message) = result {
            *failure.borrow_mut() = Some(message);
        }
    });
    // SAFETY: the handler runs synchronously inside this call.
    unsafe { array.getBytesWithHandler(&block) };
    match failure.take() {
        Some(message) => Err(SeparationError::Model(message)),
        None => Ok(()),
    }
}

impl CoreMlModel {
    /// Loads a compiled model (`.mlmodelc`). On the GPU, the first prediction
    /// after installing a model compiles kernels (10–35 s); later runs, even
    /// in new processes, take about a second.
    pub fn load(path: &Path, units: ComputeUnits) -> Result<Self, SeparationError> {
        let path = path
            .to_str()
            .ok_or_else(|| SeparationError::Model("the model path is not UTF-8".into()))?;
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            // SAFETY: plain Core ML configuration and loading calls.
            let config = unsafe { MLModelConfiguration::new() };
            let units = match units {
                ComputeUnits::Gpu => MLComputeUnits::CPUAndGPU,
                ComputeUnits::Cpu => MLComputeUnits::CPUOnly,
            };
            unsafe { config.setComputeUnits(units) };
            let model =
                unsafe { MLModel::modelWithContentsOfURL_configuration_error(&url, &config) }
                    .map_err(|error| model_error("cannot load the model", &error))?;
            Ok(Self { model })
        })
    }
}

impl StemModel for CoreMlModel {
    fn predict(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        autoreleasepool(|_| {
            // Fresh input arrays for every prediction, as in the measured
            // prototype.
            let mix_array = new_array(&MIX_SHAPE)?;
            let spec_array = new_array(&SPEC_SHAPE)?;
            write_array(&mix_array, &MIX_SHAPE, mix)?;
            write_array(&spec_array, &SPEC_SHAPE, spec)?;
            // SAFETY: feature values wrap the input arrays for this call.
            let values: [Retained<AnyObject>; 2] = unsafe {
                [
                    MLFeatureValue::featureValueWithMultiArray(&mix_array).into(),
                    MLFeatureValue::featureValueWithMultiArray(&spec_array).into(),
                ]
            };
            let keys = [NSString::from_str("mix"), NSString::from_str("spec")];
            let dictionary = NSDictionary::<NSString, AnyObject>::from_retained_objects(
                &[&*keys[0], &*keys[1]],
                &values,
            );
            let provider = unsafe {
                MLDictionaryFeatureProvider::initWithDictionary_error(
                    MLDictionaryFeatureProvider::alloc(),
                    &dictionary,
                )
            }
            .map_err(|error| model_error("cannot pass the inputs", &error))?;
            let input = ProtocolObject::<dyn MLFeatureProvider>::from_ref(&*provider);
            let output = unsafe { self.model.predictionFromFeatures_error(input) }
                .map_err(|error| model_error("prediction failed", &error))?;
            read_output(&output, "spec_out", &SPEC_OUT_SHAPE, spec_out)?;
            read_output(&output, "wave_out", &WAVE_OUT_SHAPE, wave_out)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::for_each_row;

    #[test]
    fn rows_follow_strides_and_stay_inside_the_buffer() {
        // A [2][3] array stored with a padded row stride of 4.
        let mut rows = Vec::new();
        for_each_row(&[2, 3], &[4, 1], 7, |from, to, len, step| {
            rows.push((from, to, len, step))
        })
        .unwrap();
        assert_eq!(rows, vec![(0, 0, 3, 1), (3, 4, 3, 1)]);
        assert!(for_each_row(&[2, 3], &[4, 1], 6, |_, _, _, _| {}).is_err());
        assert!(for_each_row(&[2, 3], &[1], 8, |_, _, _, _| {}).is_err());
        // Transposed storage: the innermost axis has stride 2.
        let mut rows = Vec::new();
        for_each_row(&[2, 3], &[1, 2], 6, |from, to, len, step| {
            rows.push((from, to, len, step))
        })
        .unwrap();
        assert_eq!(rows, vec![(0, 0, 3, 2), (3, 1, 3, 2)]);
    }
}
