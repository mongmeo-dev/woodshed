"""PyTorch 기준 결과를 만든다. 모델이나 변환 스크립트를 바꾼 뒤 앱 출력과 비교할 때 쓴다.

사용: python ref_torch.py <5c90dfd2.safetensors> <mix.wav (44.1kHz 스테레오)> <out.npy>

(1) demucs apply_model(shifts=0, split=True, overlap=0.25) 원본과 (2) 외부 numpy DSP(dsp.py) + torch
핵심부(core.py)를 함께 돌려 둘이 같은지(스템별 100dB 이상) 출력하고, (1)을 [6, 2, N] float32로 저장한다.
"""
import sys
import time

import numpy as np
import soundfile as sf
import torch

sys.path.insert(0, sys.path[0])
import dsp  # noqa: E402
from core import HTDemucsCore, load_htdemucs_6s  # noqa: E402
from demucs.apply import apply_model  # noqa: E402

weights, wav_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
torch.set_num_threads(4)
mix, sr = sf.read(wav_path, dtype="float32", always_2d=True)
assert sr == dsp.SR and mix.shape[1] == 2, (sr, mix.shape)
mix = mix.T
m = load_htdemucs_6s(weights)
assert m.sources == dsp.SOURCES

wav = torch.from_numpy(mix)
ref = wav.mean(0)
mu, sd = ref.mean(), ref.std() + 1e-8
t0 = time.time()
with torch.no_grad():
    out = apply_model(m, ((wav - mu) / sd)[None], shifts=0, split=True, overlap=0.25)[0] * sd + mu
t_orig = time.time() - t0
out = out.numpy()

core = HTDemucsCore(m).eval()


def run(mc, sp):
    with torch.no_grad():
        a, b = core(torch.from_numpy(mc), torch.from_numpy(sp))
    return a.numpy(), b.numpy()


t0 = time.time()
out2 = dsp.separate(mix, run)
t_split = time.time() - t0


def snr(r, e):
    return 10 * np.log10((r**2).sum() / ((r - e) ** 2).sum() + 1e-20)


print(f"apply_model {t_orig:.2f}s, numpy-dsp+core {t_split:.2f}s")
for i, s in enumerate(dsp.SOURCES):
    print(f"  {s:7s} SNR(apply_model vs dsp.py+core) = {snr(out[i], out2[i]):.1f} dB")
print("sum-vs-mix SNR apply_model:", round(snr(mix, out.sum(0)), 2), "dsp.py:", round(snr(mix, out2.sum(0)), 2))
np.save(out_path, out)
