"""Rust DSP 테스트 기준값을 dsp.py(numpy 사양)로 만든다.

사용(저장소 루트에서, numpy만 필요하고 모델·가중치는 필요 없음):
    python tools/stem-model/make_dsp_fixture.py src-tauri/src/separation/testdata/dsp_reference.json
    pnpm exec biome format --write src-tauri/src/separation/testdata/dsp_reference.json
dsp.py나 아래 입력식을 바꾸면 다시 만든다. 입력식은 src-tauri/src/separation/tests.rs 의
test_signal / test_spectrum / fake_core 와 글자 그대로 같아야 한다.
"""
import json
import sys

import numpy as np

sys.path.insert(0, sys.path[0])
import dsp  # noqa: E402

SR, SEG, F, T = dsp.SR, dsp.SEG, dsp.NFFT // 2, dsp.LE
G = [0.1, 0.2, 0.3, 0.15, 0.15, 0.1]
H = [0.05, -0.02, 0.01, 0.0, 0.03, -0.07]


def noise(n, channel):
    """Integer-hash noise in [-0.05, 0.05): the same bits in Rust (wrapping u32 arithmetic)."""
    u = (np.arange(n, dtype=np.uint64) * 2654435761 + channel * 40503) & 0xFFFFFFFF
    u ^= u >> 16
    u = (u * 0x45D9F3B) & 0xFFFFFFFF
    u ^= u >> 16
    return (u.astype(np.float64) / 4294967296.0 - 0.5) * 0.1


def test_signal(n):
    i = np.arange(n, dtype=np.float64)
    left = (
        0.5 * np.sin(2 * np.pi * 220.0 * i / SR)
        + 0.25 * np.sin(2 * np.pi * 3150.7 * i / SR)
        + 0.1 * np.sin(2 * np.pi * 0.37 * i / SR)
        + 0.2 * np.sin(2 * np.pi * (50.0 * (i / SR) + 750.0 * (i / SR) ** 2))
        + noise(n, 0)
    )
    right = (
        0.4 * np.sin(2 * np.pi * 330.0 * i / SR + 1.0)
        + 0.2 * np.sin(2 * np.pi * 5021.3 * i / SR)
        - 0.05
        + noise(n, 1)
    )
    return np.stack([left, right]).astype(np.float32)


def test_spectrum(channels):
    ch = np.arange(channels, dtype=np.float64)[:, None, None]
    f = np.arange(F, dtype=np.float64)[None, :, None]
    k = np.arange(T, dtype=np.float64)[None, None, :]
    return (np.sin(0.37 * f + 1.13 * k + 2.1 * ch) * np.exp(-f / 800.0)).astype(np.float32)


def fake_core(mix, spec):
    """mix [1,2,SEG], spec [1,4,F,T] -> spec_out [1,24,F,T], wave_out [1,12,SEG]."""
    tilt = (1.0 - np.arange(F, dtype=np.float32) / 4096.0)[None, :, None]
    ramp = np.arange(SEG, dtype=np.float32) / np.float32(SEG)
    spec_out = np.concatenate([spec[0] * np.float32(g) * tilt for g in G])[None].astype(np.float32)
    wave_out = np.concatenate([mix[0] * np.float32(h) * ramp for h in H])[None].astype(np.float32)
    return spec_out, wave_out


def energy(x):
    return float((x.astype(np.float64) ** 2).sum())


out = {}

# STFT (CAC) of one full chunk.
x = test_signal(SEG)
spec = dsp.stft_cac(x)[0]  # [4, F, T]
pts = [(j % 4, (j * 389 + 13) % F, (j * 53 + 5) % T) for j in range(96)]
pts += [(0, 20, 0), (0, 20, T - 1), (1, 21, 168), (2, 30, 100), (3, 31, 100), (0, 0, 0), (1, 0, 10), (2, F - 1, T - 1), (0, 73, 1), (2, 117, T - 2)]
out["stft"] = {
    "points": [[c, f, k, float(spec[c, f, k])] for c, f, k in pts],
    "energy": [energy(spec[c]) for c in range(4)],
}

# iSTFT of an arbitrary (inconsistent) spectrum: 2 stems x 2 channels.
y = dsp.istft_cac(test_spectrum(8))  # [2, 2, SEG]
pts = [(j % 2, (j // 2) % 2, (j * 5381 + 17) % SEG) for j in range(96)]
pts += [(0, 0, 0), (0, 0, 1), (1, 1, SEG - 1), (1, 0, SEG - 2), (0, 1, 3584), (1, 1, 1023), (0, 0, 171990)]
out["istft"] = {
    "points": [[s, c, i, float(y[s, c, i])] for s, c, i in pts],
    "energy": [[energy(y[s, c]) for c in range(2)] for s in range(2)],
}

# Whole separation with a position-dependent fake core: 3 chunks, the last one short and centred.
n = 2 * dsp.STRIDE + 100_000
mix = test_signal(n)
ref = mix.astype(np.float64).mean(0)
stems = dsp.separate(mix, fake_core)  # [6, 2, n]
pts = [(j % 6, (j // 6) % 2, (j * 6007 + 101) % n) for j in range(96)]
for i in (0, 1, dsp.STRIDE - 1, dsp.STRIDE, SEG - 1, SEG, 2 * dsp.STRIDE, 2 * dsp.STRIDE + 50_000, n - 1):
    pts += [(0, 0, i), (5, 1, i)]
out["separate"] = {
    "frames": n,
    "mean": float(ref.mean()),
    "std": float(ref.std(ddof=1) + 1e-8),
    "points": [[s, c, i, float(stems[s, c, i])] for s, c, i in pts],
    "energy": [[energy(stems[s, c]) for c in range(2)] for s in range(6)],
}

with open(sys.argv[1], "w") as fh:
    json.dump(out, fh)
print("wrote", sys.argv[1])
