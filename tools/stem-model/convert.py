"""htdemucs_6s 핵심부 -> CoreML mlprogram (fp32) .mlpackage.

사용: python convert.py <5c90dfd2.safetensors> <out.mlpackage>

STFT/iSTFT·청크 처리는 앱(Rust `src-tauri/src/separation/`)이 하고, 모델에는 core.py의
핵심부만 들어간다. 입력 mix [1,2,343980], spec [1,4,2048,336] / 출력 spec_out [1,24,2048,336],
wave_out [1,12,343980]. 스템 순서: drums, bass, other, vocals, guitar, piano.
fp32를 쓰는 이유와 측정치는 README "스템 분리 모델"에 있다.
"""
import sys
import time

import coremltools as ct
import numpy as np
import torch

sys.path.insert(0, sys.path[0])
from core import FRAMES, FREQS, SEGMENT_SAMPLES, HTDemucsCore, load_htdemucs_6s  # noqa: E402

weights, out_path = sys.argv[1], sys.argv[2]
torch.manual_seed(0)
torch.set_num_threads(4)
# eval 모드의 nn.MultiheadAttention 패스트패스(_native_multi_head_attention)는 coremltools가 변환하지 못한다.
torch.backends.mha.set_fastpath_enabled(False)

core = HTDemucsCore(load_htdemucs_6s(weights)).eval()
assert core.m.sources == ["drums", "bass", "other", "vocals", "guitar", "piano"], core.m.sources
mix = torch.randn(1, 2, SEGMENT_SAMPLES) * 0.1
spec = torch.randn(1, 4, FREQS, FRAMES) * 0.1
with torch.no_grad():
    traced = torch.jit.trace(core, (mix, spec), check_trace=False)

t0 = time.time()
ml = ct.convert(
    traced,
    inputs=[
        ct.TensorType(name="mix", shape=mix.shape, dtype=np.float32),
        ct.TensorType(name="spec", shape=spec.shape, dtype=np.float32),
    ],
    outputs=[
        ct.TensorType(name="spec_out", dtype=np.float32),
        ct.TensorType(name="wave_out", dtype=np.float32),
    ],
    compute_precision=ct.precision.FLOAT32,
    minimum_deployment_target=ct.target.macOS13,
    convert_to="mlprogram",
)
ml.short_description = (
    "htdemucs_6s core (STFT/iSTFT outside). sources: drums,bass,other,vocals,guitar,piano"
)
# Code and weights differ; see LICENSES/README.md "Stem separation model".
ml.license = (
    "Code: MIT (Demucs, Meta Platforms). Weights: license unclear, trained only on "
    "Meta internal data, presumed CC BY-NC; non-commercial use only. Source: Meta Demucs"
)
ml.save(out_path)
print(f"fp32 converted in {time.time() - t0:.1f}s -> {out_path}")
