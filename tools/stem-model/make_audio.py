"""30초 시험 음원 합성(저작권 없음). 드럼·베이스·기타·피아노·보컬 흉내 + 스템별 정답 저장."""
import sys, numpy as np, soundfile as sf
SR, DUR = 44100, 30.0
rng = np.random.default_rng(0)
n = int(SR * DUR); t = np.arange(n) / SR
bpm = 100; beat = 60 / bpm
def env(start, dec, ln):
    e = np.zeros(n); i = int(start * SR); m = min(n - i, int(ln * SR))
    e[i:i + m] = np.exp(-np.arange(m) / SR / dec); return e
drums = np.zeros(n)
for k in range(int(DUR / beat)):
    s = k * beat
    i = int(s * SR); m = min(n - i, int(0.3 * SR)); tt = np.arange(m) / SR
    drums[i:i + m] += np.sin(2 * np.pi * (50 + 100 * np.exp(-tt * 30)) * tt) * np.exp(-tt * 12)  # kick
    if k % 2 == 1: drums += 0.5 * rng.standard_normal(n) * env(s, 0.08, 0.3)  # snare
    for h in (0, 0.5): drums += 0.15 * rng.standard_normal(n) * env(s + h * beat, 0.02, 0.1)  # hat
roots = [41.2, 55.0, 49.0, 36.7]  # E1 A1 G1 D1
bass = np.zeros(n); piano = np.zeros(n); guitar = np.zeros(n)
for k in range(int(DUR / beat)):
    f = roots[(k // 8) % 4]; s = k * beat
    e = env(s, 0.5, beat)
    bass += e * (np.sin(2 * np.pi * f * t) + 0.4 * np.sin(4 * np.pi * f * t) + 0.2 * np.sin(6 * np.pi * f * t))
    if k % 4 == 0:
        for ratio in (4, 5, 6):  # 화음
            pf = f * 4 * ratio / 4
            piano += env(s, 1.2, 4 * beat) * sum(np.sin(2 * np.pi * pf * h * t) / h ** 1.5 for h in range(1, 6))
    if k % 2 == 0:  # Karplus-Strong 기타
        gf = f * 8 * (1.5 if k % 4 else 1); L = int(SR / gf); i = int(s * SR); m = min(n - i, int(beat * 2 * SR))
        buf = rng.uniform(-1, 1, L); out = np.zeros(m)
        for j in range(m):
            out[j] = buf[j % L]; buf[j % L] = 0.996 * 0.5 * (buf[j % L] + buf[(j + 1) % L])
        guitar[i:i + m] += out
vib = 5 * np.sin(2 * np.pi * 5 * t); melody = 220 * 2 ** (np.floor((t / (2 * beat)) % 5)[..., None] == np.arange(5)).argmax(-1).astype(float).__mul__(2 / 12)
ph = 2 * np.pi * np.cumsum(melody + vib) / SR
saw = sum(np.sin(h * ph) / h for h in range(1, 30))
from scipy.signal import lfilter, butter
voc = np.zeros(n)
for fc, bw in ((700, 110), (1220, 120), (2600, 160)):
    b, a = butter(2, [(fc - bw) / (SR / 2), (fc + bw) / (SR / 2)], "band"); voc += lfilter(b, a, saw)
voc *= (np.sin(2 * np.pi * t / (4 * beat)) > -0.3)
stems = dict(drums=drums, bass=bass, other=np.zeros(n), vocals=voc, guitar=guitar, piano=piano)
pan = dict(drums=0.5, bass=0.5, other=0.5, vocals=0.5, guitar=0.3, piano=0.7)
gain = dict(drums=0.25, bass=0.2, other=0, vocals=0.15, guitar=0.12, piano=0.06)
mix = np.zeros((n, 2)); outdir = sys.argv[1]
for k, x in stems.items():
    x = x / (np.abs(x).max() + 1e-9) * gain[k]
    st = np.stack([x * np.sqrt(1 - pan[k]) * 1.41, x * np.sqrt(pan[k]) * 1.41], 1)
    sf.write(f"{outdir}/truth_{k}.wav", st.astype(np.float32), SR); mix += st
print("peak", np.abs(mix).max())
sf.write(f"{outdir}/mix30.wav", mix.astype(np.float32), SR)
