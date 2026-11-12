"""모델 밖 DSP: STFT/iSTFT, 입력 정규화, 청크 분할·오버랩-애드.
Rust(realfft) 이식 기준 사양. demucs apply_model(shifts=0, split=True, overlap=0.25)과 동일.
"""
import numpy as np

SR = 44100
SEG = int(7.8 * SR)          # 343980
NFFT = 4096
HOP = 1024
PAD = HOP // 2 * 3           # 1536
LE = -(-SEG // HOP)          # 336 프레임
OVERLAP = 0.25
STRIDE = int((1 - OVERLAP) * SEG)  # 257985
SOURCES = ["drums", "bass", "other", "vocals", "guitar", "piano"]

# torch.hann_window(4096) (periodic)
WINDOW = (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(NFFT) / NFFT)).astype(np.float64)
NORM = 1.0 / np.sqrt(NFFT)


def stft_cac(x):
    """x: [2, SEG] -> [1, 4, 2048, LE] float32 (L.re, L.im, R.re, R.im)."""
    assert x.shape == (2, SEG)
    right = PAD + LE * HOP - SEG
    xp = np.pad(x.astype(np.float64), ((0, 0), (PAD, right)), mode="reflect")
    # 프레임 k는 xp[k*HOP : k*HOP+NFFT] (torch center 패딩 구간은 쓰이지 않음)
    idx = np.arange(LE)[:, None] * HOP + np.arange(NFFT)[None, :]
    frames = xp[:, idx] * WINDOW  # [2, LE, NFFT]
    z = np.fft.rfft(frames, axis=-1)[..., :NFFT // 2] * NORM  # [2, LE, 2048]
    z = z.transpose(0, 2, 1)  # [2, 2048, LE]
    out = np.stack([z.real, z.imag], axis=1).reshape(4, NFFT // 2, LE)
    return out[None].astype(np.float32)


def istft_cac(spec):
    """spec: [S*4, 2048, LE] -> [S, 2, SEG]."""
    S = spec.shape[0] // 4
    z = spec.astype(np.float64).reshape(S, 2, 2, NFFT // 2, LE)
    z = z[:, :, 0] + 1j * z[:, :, 1]                       # [S, 2, 2048, LE]
    z = np.concatenate([z, np.zeros_like(z[:, :, :1])], 2)  # Nyquist 0 -> 2049
    nfr = LE + 4                                            # 앞뒤 2프레임 0
    frames = np.fft.irfft(z.transpose(0, 1, 3, 2), n=NFFT, axis=-1) / NORM * WINDOW  # [S,2,LE,NFFT]
    total = (nfr - 1) * HOP + NFFT
    y = np.zeros((S, 2, total))
    env = np.zeros(total)
    for f in range(nfr):
        env[f * HOP:f * HOP + NFFT] += WINDOW ** 2
    for k in range(LE):
        f = k + 2
        y[..., f * HOP:f * HOP + NFFT] += frames[:, :, k]
    start = NFFT // 2 + PAD  # center 패딩 2048 + 재패딩 1536 = 3584
    y = y[..., start:start + SEG] / env[start:start + SEG]
    return y.astype(np.float32)


def triangle_weight():
    w = np.concatenate([np.arange(1, SEG // 2 + 1), np.arange(SEG - SEG // 2, 0, -1)]).astype(np.float64)
    return w / w.max()


def separate(mix, run_core):
    """mix: [2, N] float32. run_core(mix_chunk[1,2,SEG], spec[1,4,2048,LE]) -> (spec_out[1,24,2048,LE], wave_out[1,12,SEG])."""
    mix = mix.astype(np.float64)
    ref = mix.mean(0)
    m, s = ref.mean(), ref.std(ddof=1) + 1e-8
    wav = (mix - m) / s
    N = wav.shape[-1]
    out = np.zeros((6, 2, N))
    sw = np.zeros(N)
    w = triangle_weight()
    for off in range(0, N, STRIDE):
        ln = min(SEG, N - off)
        delta = SEG - ln
        start = off - delta // 2
        chunk = np.zeros((2, SEG))
        a, b = max(0, start), min(N, start + SEG)
        chunk[:, a - start:b - start] = wav[:, a:b]
        chunk = chunk.astype(np.float32)
        spec_out, wave_out = run_core(chunk[None], stft_cac(chunk))
        y = istft_cac(spec_out[0]) + wave_out[0].reshape(6, 2, SEG)
        y = y[..., delta // 2: delta // 2 + ln]
        out[..., off:off + ln] += w[:ln] * y
        sw[off:off + ln] += w[:ln]
    out /= sw
    return (out * s + m).astype(np.float32)
