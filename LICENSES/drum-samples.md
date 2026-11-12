# 드럼 샘플 라이선스

`src-tauri/resources/drums/`의 샘플 10개는 모두 [Versilian Community Sample Library(VCSL)](https://github.com/sgossner/VCSL)에서 가져왔습니다. VCSL은 Versilian Studios LLC가 저장소 전체를 **CC0 1.0 Universal**(퍼블릭 도메인 헌정)로 공개한 샘플 모음입니다. 폴더별로 다른 라이선스는 없습니다. 원문은 [`CC0-1.0.txt`](CC0-1.0.txt)(VCSL `LICENSE` 그대로)에 있습니다.

- 출처 저장소: https://github.com/sgossner/VCSL
- 가져온 커밋: `c1ea7bcc3c7309650ab0da9d15c9cd1fbc4a4c7e`
- 라이선스: CC0-1.0 (저작자 표시 의무 없음, 재배포·수정·상업적 이용 가능)

| 번들 파일          | 트랙            | VCSL 원본 경로                                                                          | 라이선스 |
| ------------------ | --------------- | --------------------------------------------------------------------------------------- | -------- |
| `kick.wav`         | 킥              | `Membranophones/Struck Membranophones/Bass Drum 1/BDrumNew_hit_v7_rr1_Sum.wav`          | CC0-1.0  |
| `snare.wav`        | 스네어          | `Membranophones/Struck Membranophones/Snare Drum, Modern 1/Snare2_HitSN_v9_rr1_Mid.wav` | CC0-1.0  |
| `sidestick.wav`    | 사이드스틱      | `Membranophones/Struck Membranophones/Snare Drum, Modern 1/Snare2_stick_v1_rr1_Mid.wav` | CC0-1.0  |
| `clap.wav`         | 클랩            | `Idiophones/Struck Idiophones/Claps/Clap_rr1.wav`                                       | CC0-1.0  |
| `hihat-closed.wav` | 클로즈드 하이햇 | `Idiophones/Struck Idiophones/Hi-Hat Cymbal/HiHat_HitC_v4_rr1_Mid.wav`                  | CC0-1.0  |
| `hihat-open.wav`   | 오픈 하이햇     | `Idiophones/Struck Idiophones/Hi-Hat Cymbal/HiHat_HitO_rr1_Mid.wav`                     | CC0-1.0  |
| `tom-high.wav`     | 하이 탐         | `Membranophones/Struck Membranophones/Tom 1/Stick/TomH_HitS_v4_rr1_Mid.wav`             | CC0-1.0  |
| `tom-low.wav`      | 로우 탐         | `Membranophones/Struck Membranophones/Tom 2/Stick/TomL_HitS_v4_rr1_Mid.wav`             | CC0-1.0  |
| `ride.wav`         | 라이드          | `Idiophones/Struck Idiophones/Suspended Cymbal 2/susCymb2_hit_stick_mp1.wav`            | CC0-1.0  |
| `crash.wav`        | 크래시          | `Idiophones/Struck Idiophones/Suspended Cymbal 1/susCymb1_hit_stick_f1.wav`             | CC0-1.0  |

## 대체한 악기

- **크래시·라이드**: VCSL에는 드럼 키트용 크래시·라이드 심벌이 없습니다. 오케스트라용 서스펜디드 심벌을 스틱으로 친 샘플로 대신했습니다(크래시는 `Suspended Cymbal 1`의 forte 타격, 라이드는 `Suspended Cymbal 2`의 mezzo-piano 타격). 키트 심벌만큼 짧고 또렷하지 않고 여운이 더 깁니다.

## 가공

원본(스테레오, 44.1/48kHz, 16/24비트)을 다음 순서로 가공했습니다. 샘플레이트는 원본 그대로이고, 앱이 시작할 때와 출력 장치가 바뀔 때 출력 샘플레이트로 리샘플합니다.

1. 두 채널 평균으로 모노 합성
2. 피크 대비 -30dB를 처음 넘는 지점 0.25ms 앞에서 시작(어택 앞 무음 제거, 0.25ms 페이드인)
3. 트랙별 최대 길이로 자르고 끝에 코사인 페이드아웃
4. 피크 -1dBFS로 정규화, TPDF 디더를 넣어 16비트 PCM WAV로 저장

| 파일               | 길이  | 페이드아웃 |
| ------------------ | ----- | ---------- |
| `kick.wav`         | 0.90s | 0.25s      |
| `snare.wav`        | 0.70s | 0.20s      |
| `sidestick.wav`    | 0.35s | 0.10s      |
| `clap.wav`         | 0.45s | 0.15s      |
| `hihat-closed.wav` | 0.50s | 0.15s      |
| `hihat-open.wav`   | 2.20s | 0.60s      |
| `tom-high.wav`     | 1.10s | 0.40s      |
| `tom-low.wav`      | 1.30s | 0.40s      |
| `ride.wav`         | 3.00s | 1.00s      |
| `crash.wav`        | 3.50s | 1.20s      |
