"""앱이 쓴 스템 WAV를 PyTorch 기준(ref_torch.py 출력)과 비교한다.

사용: python compare.py <torch_ref.npy> <mix.wav> <stems_dir>
stems_dir에는 <name>.wav 6개(drums, bass, other, vocals, guitar, piano; 44.1kHz 스테레오)가 있어야 한다.
칸: 원본 대비 SNR dB / 믹스 대비 오차비 dB(=10log10(|mix|^2/|ref-est|^2), 거의 무음인 스템도 공정하게 봄).
16bit로 저장된 스템은 양자화 잡음(약 -96dBFS) 때문에 SNR 상한이 낮아진다.
"""
import sys

import numpy as np
import soundfile as sf

sys.path.insert(0, sys.path[0])
import dsp  # noqa: E402

ref = np.load(sys.argv[1])
mix = sf.read(sys.argv[2], dtype="float32", always_2d=True)[0].T
est = np.stack(
    [sf.read(f"{sys.argv[3]}/{name}.wav", dtype="float32", always_2d=True)[0].T for name in dsp.SOURCES]
)
n = min(ref.shape[-1], est.shape[-1], mix.shape[-1])
ref, est, mix = ref[..., :n], est[..., :n], mix[..., :n]


def snr(r, e):
    return 10 * np.log10((r**2).sum() / max(((r - e) ** 2).sum(), 1e-30))


def mix_ratio(r, e):
    return 10 * np.log10((mix**2).sum() / max(((r - e) ** 2).sum(), 1e-30))


print("| " + " | ".join(dsp.SOURCES) + " | 합 vs 믹스 (앱) | 합 vs 믹스 (PyTorch) | 최대 절대오차 |")
print("|---" * (len(dsp.SOURCES) + 3) + "|")
cols = [f"{snr(ref[i], est[i]):.1f} / {mix_ratio(ref[i], est[i]):.1f}" for i in range(6)]
print(
    "| "
    + " | ".join(cols)
    + f" | {snr(mix, est.sum(0)):.1f} | {snr(mix, ref.sum(0)):.1f} | {np.abs(ref - est).max():.2e} |"
)
