"""PyTorch 기준 스템의 짧은 구간을 Rust Core ML 테스트의 고정 참조로 저장한다.

사용(저장소 루트에서):
    # 1) 테스트 합성곡의 audio.wav를 내보낸다(모델 필요)
    WOODSHED_STEM_TEST_OUT=/tmp/band cargo test --lib coreml_model_separates   # src-tauri에서
    # 2) PyTorch 원본으로 분리한다
    python tools/stem-model/ref_torch.py <5c90dfd2.safetensors> /tmp/band/mix.wav /tmp/band_torch.npy
    # 3) 구간을 잘라 저장한다
    python tools/stem-model/make_reference_excerpt.py /tmp/band_torch.npy \\
        src-tauri/src/separation/testdata/band_pytorch_excerpt.f32 250000 16000

출력: little-endian float32 [6 stems][2 channels][len], 스템 순서 drums, bass, other, vocals, guitar, piano.
시작 프레임과 길이는 src-tauri/src/separation/tests.rs 의 REFERENCE_START / REFERENCE_FRAMES 와 같아야 한다.
테스트 합성곡(tests.rs `band`)이나 가져오기 디더를 바꾸면 다시 만든다.
"""
import sys

import numpy as np

ref = np.load(sys.argv[1])
start, length = int(sys.argv[3]), int(sys.argv[4])
assert ref.ndim == 3 and ref.shape[:2] == (6, 2), ref.shape
excerpt = np.ascontiguousarray(ref[:, :, start : start + length], dtype="<f4")
assert excerpt.shape[2] == length, excerpt.shape
excerpt.tofile(sys.argv[2])
print(f"wrote {sys.argv[2]}: {excerpt.nbytes} bytes, frames {start}..{start + length}")
