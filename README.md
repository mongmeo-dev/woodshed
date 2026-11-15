# Woodshed

베이스·기타 연습 도구 macOS 앱입니다. 튜너, 메트로놈, 드럼 머신, 스템 분리, 스케일 지판 5개 도구를 담으며, 서버 없이 로컬에서만 동작합니다.

- 스택: Tauri 2 + React 19 + TypeScript(Vite) + Rust
- 오디오: Rust 엔진(cpal, 64프레임 이상 중 기기가 허용하는 최솟값의 출력 버퍼·락프리 명령 큐·샘플 단위 스케줄러) — 계약은 [`AGENTS.md`](AGENTS.md#오디오-엔진-계약-src-taurisrcaudio)
- UI 언어: 한국어, 영어 (`src/locales/`)

## 준비

[mise](https://mise.jdx.dev)로 도구 버전을 맞춥니다(`mise.toml`: node 26.13.0 LTS, pnpm 12.18.0, rust 1.99.1).

```sh
mise install
mise exec -- pnpm install
```

## 실행

```sh
mise exec -- pnpm tauri dev
```

## 빌드

```sh
mise run app   # = sh scripts/build-app.sh
# → src-tauri/target/release/bundle/macos/Woodshed.app
```

### 로컬 서명 (마이크 권한 유지)

로컬 빌드는 기본으로 ad-hoc 서명이라, 다시 빌드할 때마다 macOS가 다른 앱으로 보고 마이크 권한 창을 다시 띄웁니다. 자체 서명 인증서를 한 번 만들어 두면 다시 빌드해도 권한이 유지됩니다.

```sh
mise run signing-cert   # = sh scripts/create-local-signing-cert.sh
```

- 인증서 "Woodshed Local Signing"은 전용 키체인 `~/Library/Keychains/woodshed-local-signing.keychain-db`에 들어가고, 이 키체인은 사용자 키체인 검색 목록에 추가됩니다. 키체인 암호는 `~/Library/Application Support/woodshed-local-signing/`에 둡니다. 로그인 키체인은 건드리지 않습니다.
- `mise run app`은 이 인증서가 있으면 그것으로 서명하고(hardened runtime 없이, `src-tauri/tauri.local-signing.conf.json`), 없으면 ad-hoc으로 빌드합니다. `mise run check`와 테스트는 인증서와 상관없습니다.
- 지우기: `sh scripts/create-local-signing-cert.sh --remove`. 배포 빌드는 Developer ID로 따로 서명합니다.

### 배포 빌드 (Developer ID 서명·공증)

다른 Mac에서도 경고 없이 열리는 앱은 Developer ID로 서명하고 Apple 공증을 받습니다. App Store에는 올리지 않습니다.

```sh
# 한 번: Developer ID Application 인증서(.p12)를 전용 키체인에 넣기
sh scripts/create-release-keychain.sh <identity.p12> <p12 암호 파일>
# 한 번: 공증용 App Store Connect API 키
#   ~/Library/Application Support/woodshed-release/notary.env (권한 600)
#   APPLE_API_KEY=<키 ID>
#   APPLE_API_ISSUER=<발급자 ID>
#   APPLE_API_KEY_PATH=<AuthKey_<키 ID>.p8 경로>
mise run app:release    # = sh scripts/build-release.sh
```

- 키체인은 `~/Library/Keychains/woodshed-release.keychain-db`, 암호는 `~/Library/Application Support/woodshed-release/`에 둡니다. 로그인 키체인은 이 세션 같은 비대화형 환경에서 서명할 수 없어(`errSecInternalComponent`) 쓰지 않습니다.
- hardened runtime과 `src-tauri/Entitlements.plist`로 서명합니다. 마이크(`device.audio-input`)와, yt-dlp(PyInstaller)가 실행 중 푼 Python 라이브러리를 불러오도록 `cs.disable-library-validation`을 줍니다. Tauri는 이 파일을 sidecar 서명에도 씁니다.
- 빌드 끝에 공증·staple을 하고 `codesign --verify --deep --strict`, `stapler validate`, `spctl`로 확인합니다. Developer ID 앱은 로컬 서명 앱과 다른 앱으로 보여 마이크 권한을 한 번 다시 묻습니다.

### 업데이트 서명 키

앱은 GitHub 릴리스의 `latest.json`(`https://github.com/mongmeo-dev/woodshed/releases/latest/download/latest.json`)으로 새 버전을 확인하고, 업데이트 파일의 서명을 `src-tauri/tauri.conf.json`의 `plugins.updater.pubkey`로 검증합니다.

```sh
sh scripts/create-updater-key.sh   # 한 번. 이미 있으면 다시 만들지 않고 공개키만 출력
```

- 키는 `~/Library/Application Support/woodshed-release/`에 있습니다(모두 권한 600).
  - `updater.key`: 서명 개인키(암호로 잠김)
  - `updater.key.pub`: 공개키(`plugins.updater.pubkey`와 같은 값)
  - `updater-key-password`: 개인키 암호
- **세 파일을 함께 백업하세요**(암호 관리자나 암호화한 외장 저장소 등, 저장소 밖). 개인키나 암호를 잃으면 새 업데이트에 서명할 수 없고, 이미 설치된 앱은 더 이상 업데이트되지 않습니다. 키를 새로 만들면 사용자가 새 버전을 직접 받아 설치해야 합니다.
- 릴리스 빌드는 `TAURI_SIGNING_PRIVATE_KEY`(키 파일 경로나 내용)와 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`로 이 키를 넘깁니다. 경로 변수는 `scripts/release-signing.sh`의 `WOODSHED_UPDATER_KEY`, `WOODSHED_UPDATER_KEY_PASSWORD_FILE`입니다.
- 업데이트 파일(`bundle.createUpdaterArtifacts`)은 빌드할 때 개인키가 있어야 하므로 기본 설정에 넣지 않습니다. `mise run app`은 키 없이 빌드됩니다.
- 앱은 실행 3초 뒤 한 번 확인하고(실패는 조용히 넘김), 새 버전이 있으면 사이드바 아래에 '업데이트 설치' 버튼을 보입니다. 설치는 누를 때만 하고, 끝나면 앱이 다시 시작됩니다.

#### 로컬 서버로 업데이트 확인

출시 설정을 바꾸지 않고 테스트 전용 `--config`로 확인합니다.

1. 테스트 설정 JSON: `plugins.updater.endpoints`를 `["http://127.0.0.1:<포트>/latest.json"]`, `plugins.updater.dangerousInsecureTransportProtocol`을 `true`로 둡니다. 이전 버전 후보에는 `{"version": "0.0.9"}`처럼 낮은 버전을, 새 버전에는 `{"bundle": {"createUpdaterArtifacts": true}}`를 더 줍니다.
2. `VITE_UPDATE_AUTO_INSTALL=1`로 빌드하면 화면 클릭 없이 찾은 업데이트를 바로 설치합니다(테스트 전용, 기본 빌드에서는 번들러가 이 분기를 지웁니다).

   ```sh
   VITE_UPDATE_AUTO_INSTALL=1 sh scripts/build-app.sh --config test.json --config old.json   # 후보를 임시 폴더로 복사
   TAURI_SIGNING_PRIVATE_KEY="$(cat …/updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat …/updater-key-password)" \
     VITE_UPDATE_AUTO_INSTALL=1 sh scripts/build-app.sh --config test.json --config new.json
   ```
3. `Woodshed.app.tar.gz`와 `.sig` 내용으로 `latest.json`(`platforms.darwin-aarch64.{signature,url}`)을 만들어 `python3 -m http.server`로 내려주고, 후보를 `open -g -n`으로 엽니다. 업데이트되면 `~/Library/Logs/dev.mongmeo.woodshed/Woodshed.log`에 `Woodshed 0.0.9 started` 다음 `Woodshed 0.1.0 started`가 남습니다.
4. 후보는 운영 앱과 같은 앱 데이터 폴더를 쓰므로 실행 전에 백업하고 끝나면 되돌립니다. 빌드가 남긴 `target/release/bundle`의 테스트 설정 앱은 지웁니다.

### 첫 실행과 마이크 권한

마이크 권한이 아직 정해지지 않았고 기본 출력 장치에 입력이 있으면(오디오 인터페이스 등), macOS는 권한 창에 답할 때까지 출력 스트림을 만들지 않습니다. 앱은 이때 권한 창을 띄우고, 답하는 즉시 출력을 엽니다(`AGENTS.md`의 출력 관리자). 첫 실행 조건은 `tccutil reset Microphone dev.mongmeo.woodshed`로 다시 만들 수 있습니다.

### 스템 분리용 sidecar (yt-dlp, ffmpeg)

YouTube 가져오기는 앱에 넣는 yt-dlp와 ffmpeg를 씁니다. 바이너리는 git에 넣지 않고 스크립트로 준비합니다.

```sh
mise run binaries   # = pnpm binaries = sh scripts/fetch-binaries.sh
# → src-tauri/binaries/yt-dlp-<triple>, ffmpeg-<triple>
```

- yt-dlp: 공식 릴리스 `yt-dlp_macos`(universal) 2026.10.26을 받아 SHA-256을 확인합니다.
- ffmpeg: ffmpeg.org 9.0.2 소스(SHA-256 고정)를 받아 오디오 전용 LGPL 구성(`--disable-gpl`, 외부 라이브러리 없음)으로 빌드합니다. 처음 한 번 1~2분 걸립니다.
- 결과는 `~/Library/Caches/dev.mongmeo.woodshed/binaries`에 캐시되어 다음부터는 복사만 합니다. 다른 아키텍처는 `sh scripts/fetch-binaries.sh --target x86_64-apple-darwin`(또는 `universal-apple-darwin`).
- `pnpm tauri build`는 빌드 전에 이 스크립트를 실행합니다(`beforeBuildCommand`). `mise run check`·`cargo test`·`pnpm tauri dev`는 바이너리가 없어도 됩니다. 이때 `build.rs`가 빈 자리표시 파일을 만들고, 앱은 YouTube 가져오기에서 도구를 찾을 수 없다는 오류만 보입니다. 실제로 내려받아 보려면 먼저 `mise run binaries`를 실행합니다.
- 라이선스 고지(FFmpeg LGPL v2.1+ 전문과 소스 위치, yt-dlp)는 [`LICENSES/`](LICENSES/README.md)에 있고 앱 번들에 함께 들어갑니다.

### 스템 분리 모델 (Demucs htdemucs_6s → Core ML)

스템 분리는 Demucs v4 `htdemucs_6s`(6스템: drums, bass, other, vocals, guitar, piano)를 Core ML로 바꿔 앱 안에서 돌립니다. 가중치와 변환한 모델은 git에 넣지 않고 스크립트로 만듭니다.

```sh
mise run stem-model   # = pnpm stem-model = sh tools/stem-model/build.sh
# → src-tauri/models/htdemucs_6s_fp32.mlmodelc (115MB)
```

- 가중치: Hugging Face [`adefossez/HTDemucs-6s`](https://huggingface.co/adefossez/HTDemucs-6s)의 고정 리비전 `3c5ee475be62`에서 `5c90dfd2.safetensors`를 받아 SHA-256(`d2a1745f…d411`)을 확인합니다. Demucs 코드는 MIT(Meta Platforms)이고, 가중치에는 별도 라이선스 표기가 없습니다(모델 카드에 license 필드 없음). 고지는 [`LICENSES/`](LICENSES/README.md)에 있습니다.
- 변환: `tools/stem-model/`의 `core.py`(STFT/iSTFT를 뺀 핵심부)를 `convert.py`가 fp32 mlprogram으로 바꾸고 `xcrun coremlcompiler`로 컴파일합니다. Python은 mise로 고정한 uv 0.10.7이 만든 Python 3.12.8 가상환경에서 `requirements.lock.txt`(torch 2.7.0, coremltools 9.0, demucs 4.1.0 등 전부 고정)로 돌립니다. 처음 한 번은 가중치 52MB 다운로드, Python 패키지 설치, 변환(약 5초)을 합니다. 이 Mac에서 새 캐시로 약 21초 걸렸고, 네트워크와 uv 캐시 상태에 따라 다릅니다.
- 결과와 가중치는 `~/Library/Caches/dev.mongmeo.woodshed/models`에 캐시되어 다음부터는 복사만 합니다. 스크립트·잠금 파일·가중치가 바뀌면 다시 변환합니다. 변환한 모델은 연구 단계 모델과 `model.mil`·가중치가 바이트 단위로 같습니다.
- `pnpm tauri build`는 빌드 전에 이 스크립트를 실행합니다(`beforeBuildCommand`). `mise run check`·`cargo test`·`pnpm tauri dev`는 모델이 없어도 됩니다. 이때 `build.rs`가 빈 자리표시 디렉터리를 만들고, 모델이 필요한 테스트는 건너뛴 사실을 출력합니다. dev 앱은 캐시에 모델이 있으면 그것을 씁니다.
- 앱 밖 DSP(STFT/iSTFT, 7.8초 청크, 삼각 가중치 오버랩-애드, 입력 정규화)는 Rust(`src-tauri/src/separation/`)에 있고, 기준 사양은 `tools/stem-model/dsp.py`(numpy)입니다. Rust 테스트는 `make_dsp_fixture.py`가 만든 고정 입력 기준값(`src-tauri/src/separation/testdata/dsp_reference.json`)과 비교합니다. `dsp.py`나 입력식을 바꾸면 이 파일을 다시 만듭니다.
- 모델이나 변환을 바꾸면 PyTorch 원본과 비교합니다: `make_audio.py`로 시험 음원을 만들고, `ref_torch.py`로 기준을 만든 뒤 `cargo run --release --example separate_wav -- <mix.wav> <out>`의 출력과 `compare.py`로 비교합니다. 앱 경로(가져오기 → 분리 → manifest → 재시작 뒤 다시 열기, 강제 종료 뒤 복구)는 `cargo run --release --example stem_separate -- --library <임시 폴더> <파일>`로 창 없이 확인합니다.

분리 동작(코드: `src-tauri/src/separation/`)

- Core ML은 `CPUAndGPU`로 돌립니다. GPU가 가끔 틀린 결과를 냅니다: Apple M5·macOS 27에서 청크 사이에 GPU가 쉬면 다섯 번에 한 번꼴로 한 청크가 최대 3e-2까지 어긋났습니다(청크 SNR 26~60dB, 맞으면 120dB). 연구 단계 프로토타입도 같았습니다. 맞는 결과는 실행마다 비트 단위로 같고 틀린 결과는 매번 달라서, 같은 청크를 두 번 돌려 일치할 때만 받아들이고 네 번 안에 일치하지 않으면 그 청크만 CPU(결정적, 최대 오차 7e-6)로 돌립니다. 그래서 GPU 시간은 단일 실행의 약 2배입니다. 긴 곡에서는 불일치가 실제로 생깁니다: 5분 곡에서 1번, 30분 곡(308청크)에서 17번 세 번째 실행으로 해결했습니다. 이것은 이 기기에서 관측한 현상에 맞춘 완화책이고, 정확성을 따로 보증하는 기준은 아닙니다.
- 결과: 30초 시험 음원(`make_audio.py`의 `mix30.wav`)을 세 번 분리해 모두 PyTorch `apply_model` 원본과 최대 절대오차 1.16e-6(스템별 SNR 101~129dB, float 출력 기준)이었습니다. Rust 테스트는 12초 합성곡을 같은 `audio.wav`의 PyTorch 결과와 비교합니다: 고정 참조 구간(`testdata/band_pytorch_excerpt.f32`, 청크 경계를 걸치는 0.36초)에서 float 출력의 스템별 SNR 102.8~125.3dB, 최대 오차 4.2e-7이고, 16bit 스템 파일의 스템 레벨과 6스템 합 대 원본 잔차(16.92dB)도 PyTorch와 소수 둘째 자리까지 같습니다. 이 잔차는 Demucs가 스템 합을 원본과 맞추지 않아서 생기는 PyTorch 자체 값이고 곡마다 다릅니다(`mix30.wav`는 25.8dB).
- 분리는 한 번에 한 곡씩 하고, 앱 시작 5초 뒤 모델을 한 번 돌려 GPU 커널을 미리 컴파일합니다. 2분 동안 쓰지 않으면 모델과 버퍼(약 1GB)를 내려놓습니다. 스템은 확정된 구간부터 파일로 흘려 쓰지만, 입력은 44.1kHz 스테레오 f32로 곡 전체를 메모리에 둡니다(1분에 약 21MB, 다른 레이트는 리샘플하는 동안 변환 전후가 함께 있음). 릴리스 빌드 peak footprint 실측(`separate_wav --discard`, 스템 쓰기 제외): 30초 1.22GB, 5분 1.35GB(48kHz 입력 1.43GB), 30분 1.87GB. 분리 시간은 30초 3.9초, 5분 37초, 30분 274초였습니다.

## 검사

```sh
mise run check        # 전체: biome, tsc, vitest, cargo fmt, clippy, cargo test
mise run check:web    # 웹만
mise run check:rust   # Rust만
```

실제 출력 장치의 버퍼·레이턴시 확인(스피커로 짧은 클릭음이 납니다):

```sh
cd src-tauri && mise exec -- cargo run --release --example latency_probe
```

앱도 시작할 때와 출력 장치를 바꿀 때 실측 출력 레이턴시를 로그에 남깁니다(`~/Library/Logs/dev.mongmeo.woodshed/Woodshed.log`). 같은 로그의 `startup:` 줄은 시작 단계별 시각(`run()` 시작 기준 ms와 epoch ms)입니다.

## 디자인

- [`DESIGN.md`](DESIGN.md)는 [VoltAgent/awesome-design-md](https://github.com/VoltAgent/awesome-design-md)(MIT)의 [`design-md/apple/DESIGN.md`](https://github.com/VoltAgent/awesome-design-md/blob/fcc702849322/design-md/apple/DESIGN.md)(커밋 `fcc702849322`)를 그대로 가져온 것입니다. 밝은 캔버스와 어두운 타일 팔레트가 함께 있어 라이트/다크를 모두 만들 수 있고, SF Pro 시스템 폰트와 단일 강조색이 미니멀 mac 앱에 맞아 골랐습니다.
- 토큰은 `src/styles/tokens.css`의 `--ws-*` CSS 변수로 옮겼습니다. 다크 모드는 DESIGN.md의 어두운 타일 색(`surface-black`, `surface-tile-1~3`, `body-on-dark`, `primary-on-dark`)으로 구성합니다.
- 컴포넌트 참고: [component.gallery](https://component.gallery)의 Navigation(Primer NavList: 현재 화면을 `aria-current`로 표시하는 사이드바), Segmented control(Primer SegmentedControl, Radix Toggle Group).
- 모션 참고: [Kinetics](https://kinetics.colorion.co)의 Tab Pill Glide(선택 표시가 대상 크기를 재고 미끄러짐 — 0.4s를 240ms로 줄임), Pulse Badge(링이 커지며 사라짐 — 테스트음이 들리는 순간 한 번만 재생).

## 메트로놈

코드: `src-tauri/src/metronome/`(엔진 쪽), `src/tools/metronome/`(화면).

- **박 단위**: 한 박은 박자 분모의 음표입니다. 6/8은 마디마다 8분음표 6번을 치고, BPM도 8분음표 기준입니다. 박자를 바꾸면 악센트는 그 박자의 기본값(강박 1개, x/8 박자는 3+3·2+2+3처럼 묶음 첫 박은 보통, 나머지는 약)으로 돌아갑니다.
- **악센트**: 강은 높은 음, 보통·약은 낮은 음을 각각 다른 세기로 냅니다. 무음 박은 소리만 빠지고 박 표시와 서브디비전은 그대로입니다.
- **폴리리듬 X:Y**: 주 박 Y개 동안 X번을 고르게 칩니다(3:2는 2박에 3번). 주기는 재생 시작부터 이어지고, 폴리리듬이나 박자를 바꾸면 그 마디 첫 박에서 다시 셉니다.
- **템포 트레이너**: N마디마다 X BPM씩 올리고 최대 템포에서 멈춥니다. 최대 템포로 N마디를 친 뒤 "유지"면 계속 치고, "정지"면 재생을 끝냅니다. 재생 중 BPM을 바꾸면 그 템포에서 다시 셉니다. 템포를 올릴지·멈출지는 다음 마디 첫 박을 예약할 때 정하므로, 아직 들리지 않은 마디 경계는 트레이너 설정을 바꾸면 새 설정으로 다시 정해집니다(예정된 정지도 그 시각이 지나기 전까지는 바뀔 수 있음).
- **변경 반영**: 재생 중 바꾼 값은 지금부터 40ms 넘게 남은 다음 박부터 들립니다. 마디에 들어가지 않게 된 박(4/4 → 3/4에서 4박째)은 새 마디의 첫 박이 됩니다.
- **정확도**: 박 위치는 템포 구간 시작 프레임에서 정수 연산으로 계산하므로 반올림 오차가 쌓이지 않습니다(`src-tauri/tests/metronome_render.rs`: 120.0 BPM 10분 드리프트 0). 출력 장치가 바뀌면 남은 예약을 지우고 새 샘플레이트로 다음 박부터 다시 예약합니다.
- **화면 동기**: 박을 예약하는 순간(약 250ms 앞) 들리는 시각을 계산해 화면에 보내고, 화면은 그 시각이 지난 첫 프레임에 박을 켭니다.
- **저장**: 마지막 설정과 프리셋은 앱 데이터 디렉터리의 `metronome.json`에 둡니다. 같은 이름으로 저장하면 덮어씁니다. 다른 화면으로 가도 재생은 이어집니다.

## 드럼 머신

- 엔진: `src-tauri/src/drums/`(샘플 보이스·스텝 시퀀서·Tauri 명령), 화면: `src/tools/drumMachine/`.
- 편집 중인 패턴과 저장한 패턴은 앱 데이터 디렉터리(`~/Library/Application Support/dev.mongmeo.woodshed/`)의 `drum-machine.json`에 둡니다. `settings.json`과 따로 저장합니다.
- 스케줄러 스레드가 120ms 앞까지 스텝을 샘플 단위로 예약합니다. 재생 중 편집·템포·스윙·길이 변경과 출력 장치 변경(세대 증가)은 20ms 앞부터 취소 후 다시 예약하므로 재생이 멈추지 않습니다.
- 화면의 현재 스텝은 스텝을 예약할 때 계산한 들리는 시각(`heardAtMs`)을 전용 채널(`drums_subscribe`)로 미리 받아 그 시각에 표시합니다.

## 드럼 샘플

드럼 머신의 샘플 10개(`src-tauri/resources/drums/`)는 [Versilian Community Sample Library](https://github.com/sgossner/VCSL)(커밋 `c1ea7bc`)에서 가져왔고, 모두 **CC0 1.0**(퍼블릭 도메인)입니다. 파일별 원본 경로와 가공 내용은 [`LICENSES/drum-samples.md`](LICENSES/drum-samples.md), 라이선스 원문은 [`LICENSES/CC0-1.0.txt`](LICENSES/CC0-1.0.txt)에 있습니다.

## 미결 사항

- htdemucs_6s 가중치에는 별도 라이선스 표기가 없습니다(코드는 MIT). 배포 방식을 정할 때 다시 확인합니다.
- 스템 분리는 Intel Mac에서 시험하지 않았습니다. 서명·번들된 앱의 첫 실행에서 GPU 커널 컴파일(10~35초)이 시스템 캐시에 남는지도 다시 재야 합니다.
- 번들한 `yt-dlp_macos`는 PyInstaller 묶음이라 결합 저작물로는 GPLv3+입니다(yt-dlp 자체는 Unlicense). 앱은 별도 프로세스로 실행만 합니다. 배포 방식을 정할 때 다시 확인합니다.
- YouTube는 JS 런타임 없이 yt-dlp 기본 클라이언트로 받습니다. 일부 영상 형식이 막히면 JS 런타임(deno 등) 번들 여부를 정해야 합니다.
- 마이크 권한 문구는 `src-tauri/Info.plist`(기본, 영어)와 `src-tauri/macos/{en,ko}.lproj/InfoPlist.strings`(`bundle.macOS.files`로 번들에 복사)에 있습니다.
- 출력이 바뀌어 샘플레이트가 달라지면 이미 예약된 이벤트의 프레임 위치는 그대로라 시간상 위치가 달라집니다. 메트로놈·드럼·스템 플레이어는 `audio://output-changed`를 받아 다시 예약해야 합니다.
- 출력 버퍼 하한은 64프레임입니다. 기기가 더 작은 버퍼를 허용해도 기기별 드롭아웃을 피하려고 쓰지 않습니다.
