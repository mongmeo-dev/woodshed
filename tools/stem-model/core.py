"""htdemucs_6s 핵심부 래퍼: STFT/iSTFT·청크 처리를 모델 밖으로 뺀 순수 텐서 그래프.

입력
  mix : [1, 2, L]           L = SEGMENT_SAMPLES (343980, 7.8초 @44.1kHz)
  spec: [1, 4, 2048, T]     T = 336. 모델 밖 STFT(CAC: L.re, L.im, R.re, R.im)
출력
  spec_out: [1, 24, 2048, T]  6스템 × 4채널(CAC) 복소 스펙트로그램 (역정규화 완료)
  wave_out: [1, 12, L]        6스템 × 2채널 시간 영역 분기 출력 (역정규화 완료)
최종 스템 = iSTFT(spec_out) + wave_out.
"""
import torch
from torch import nn
from einops import rearrange

SR = 44100
SEGMENT_SAMPLES = int(7.8 * SR)  # 343980
NFFT = 4096
HOP = 1024
FREQS = NFFT // 2  # 2048 (Nyquist bin 제거)
FRAMES = -(-SEGMENT_SAMPLES // HOP)  # ceil -> 336


def load_htdemucs_6s(weights_path):
    """htdemucs_6s 단일 모델(5c90dfd2)을 로컬 safetensors 파일에서 읽는다(네트워크 없음)."""
    from demucs.hf import load_safetensors_model
    m = load_safetensors_model(weights_path)
    m.eval()
    return m


class HTDemucsCore(nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, mix, mag):
        m = self.m
        x = mag
        B, C, Fq, T = x.shape
        mean = x.mean(dim=(1, 2, 3), keepdim=True)
        std = x.std(dim=(1, 2, 3), keepdim=True)
        x = (x - mean) / (1e-5 + std)

        xt = mix
        meant = xt.mean(dim=(1, 2), keepdim=True)
        stdt = xt.std(dim=(1, 2), keepdim=True)
        xt = (xt - meant) / (1e-5 + stdt)

        saved, saved_t, lengths, lengths_t = [], [], [], []
        for idx, encode in enumerate(m.encoder):
            lengths.append(x.shape[-1])
            inject = None
            if idx < len(m.tencoder):
                lengths_t.append(xt.shape[-1])
                tenc = m.tencoder[idx]
                xt = tenc(xt)
                if not tenc.empty:
                    saved_t.append(xt)
                else:
                    inject = xt
            x = encode(x, inject)
            if idx == 0 and m.freq_emb is not None:
                frs = torch.arange(x.shape[-2], device=x.device)
                emb = m.freq_emb(frs).t()[None, :, :, None]  # expand_as 대신 브로드캐스트(상수 33MB 펼침 방지)
                x = x + m.freq_emb_scale * emb
            saved.append(x)

        if m.crosstransformer:
            if m.bottom_channels:
                b, c, f, t = x.shape
                x = rearrange(x, "b c f t-> b c (f t)")
                x = m.channel_upsampler(x)
                x = rearrange(x, "b c (f t)-> b c f t", f=f)
                xt = m.channel_upsampler_t(xt)
            x, xt = m.crosstransformer(x, xt)
            if m.bottom_channels:
                x = rearrange(x, "b c f t-> b c (f t)")
                x = m.channel_downsampler(x)
                x = rearrange(x, "b c (f t)-> b c f t", f=f)
                xt = m.channel_downsampler_t(xt)

        for idx, decode in enumerate(m.decoder):
            skip = saved.pop(-1)
            x, pre = decode(x, skip, lengths.pop(-1))
            offset = m.depth - len(m.tdecoder)
            if idx >= offset:
                tdec = m.tdecoder[idx - offset]
                length_t = lengths_t.pop(-1)
                if tdec.empty:
                    pre = pre[:, :, 0]
                    xt, _ = tdec(pre, None, length_t)
                else:
                    skip = saved_t.pop(-1)
                    xt, _ = tdec(xt, skip, length_t)

        S = len(m.sources)
        x = x.view(B, S, -1, Fq, T)
        x = x * std[:, None] + mean[:, None]
        xt = xt.view(B, S, -1, mix.shape[-1])
        xt = xt * stdt[:, None] + meant[:, None]
        return x.reshape(B, S * 4, Fq, T), xt.reshape(B, S * 2, mix.shape[-1])
