# Woodshed

베이스·기타 연습 도구 macOS 앱입니다. 튜너, 메트로놈, 드럼 머신, 스템 분리, 스케일 지판 5개 도구를 담으며, 서버 없이 로컬에서만 동작합니다.

- 스택: Tauri 2 + React 19 + TypeScript(Vite) + Rust
- 오디오: Rust 엔진(cpal, 64프레임 이상 중 기기가 허용하는 최솟값의 출력 버퍼·락프리 명령 큐·샘플 단위 스케줄러) — 계약은 [`AGENTS.md`](AGENTS.md#오디오-엔진-계약-src-taurisrcaudio)
- UI 언어: 한국어, 영어 (`src/locales/`)
- 라이선스: [MIT](LICENSE). 함께 배포하는 서드파티 구성 요소는 [라이선스](#라이선스) 절을 봅니다.

## 설치

Apple Silicon Mac, macOS 13 이상에서 동작합니다.

1. [Releases](https://github.com/mongmeo-dev/woodshed/releases/latest)에서 `Woodshed_<버전>_aarch64.dmg`를 받습니다.
2. DMG를 열고 `Woodshed.app`을 `Applications` 폴더로 끌어 놓습니다.
3. 앱을 엽니다. 튜너를 쓰거나 기본 출력 장치에 입력이 있으면 마이크 권한을 묻습니다.

새 버전이 나오면 사이드바 아래에 '업데이트 설치' 버튼이 보입니다. 누르면 받아서 설치하고 다시 시작합니다. Intel Mac용 릴리스는 없으니 [소스에서 빌드](#소스에서-빌드)를 따릅니다.

## 소스에서 빌드

### 준비

- macOS 13 이상, Xcode(스템 분리 모델을 컴파일하는 `xcrun coremlcompiler`)
- [mise](https://mise.jdx.dev): 도구 버전을 `mise.toml`로 맞춥니다(node 26.13.0 LTS, pnpm 12.18.0, rust 1.99.1).

```sh
git clone https://github.com/mongmeo-dev/woodshed.git
cd woodshed
mise install
mise exec -- pnpm install --frozen-lockfile
```

### 실행

```sh
mise exec -- pnpm tauri dev
```

sidecar(yt-dlp, ffmpeg)와 스템 분리 모델이 없어도 앱은 뜹니다. YouTube 가져오기와 스템 분리까지 쓰려면 먼저 `mise run binaries`와 `mise run stem-model`을 실행합니다(아래 절).

### 앱 빌드

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

- 키체인은 `~/Library/Keychains/woodshed-release.keychain-db`, 암호는 `~/Library/Application Support/woodshed-release/`에 둡니다. 로그인 키체인은 SSH 세션 같은 비대화형 환경에서 서명할 수 없어(`errSecInternalComponent`) 쓰지 않습니다.
- hardened runtime과 `src-tauri/Entitlements.plist`로 서명합니다. 마이크(`device.audio-input`)와, yt-dlp(PyInstaller)가 실행 중 푼 Python 라이브러리를 불러오도록 `cs.disable-library-validation`을 줍니다. Tauri는 이 파일을 sidecar 서명에도 씁니다.
- 빌드 끝에 공증·staple을 하고 `codesign --verify --deep --strict`, `stapler validate`, `spctl`로 확인합니다. Developer ID 앱은 로컬 서명 앱과 다른 앱으로 보여 마이크 권한을 한 번 다시 묻습니다.
- 출시 설정(`src-tauri/tauri.release.conf.json`)은 업데이트 파일(`bundle.createUpdaterArtifacts`)도 켭니다. 그래서 이 빌드에는 [업데이트 서명 키](#업데이트-서명-키)가 있어야 하고, 앱 옆에 `Woodshed.app.tar.gz`와 `.sig`가 생깁니다. GitHub 릴리스는 [릴리스](#릴리스)의 `mise run release`로 만듭니다.

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
- 업데이트 파일(`bundle.createUpdaterArtifacts`)은 빌드할 때 개인키가 있어야 하므로 기본 설정에 넣지 않고 출시 설정에서만 켭니다. `mise run app`은 키 없이 빌드되고, `mise run app:release`(`scripts/build-release.sh`)가 키를 넘깁니다. 키 값은 로그에 찍지 않습니다.
- 앱은 실행 3초 뒤 한 번 확인하고(실패는 조용히 넘김), 새 버전이 있으면 사이드바 아래에 '업데이트 설치' 버튼을 보입니다. 설치는 누를 때만 하고, 끝나면 앱이 다시 시작됩니다.

#### 로컬 서버로 업데이트 확인

테스트 빌드는 `scripts/build-update-test.sh`로만 만듭니다. 이 스크립트는 임시 `--config`로 엔드포인트를 `http://127.0.0.1:<포트>/latest.json`(http 허용)으로 바꾸고, 프런트엔드를 Vite mode `update-test`로 빌드합니다. 이 모드에서만 찾은 업데이트를 클릭 없이 바로 설치합니다. 환경 변수나 `.env`로는 켤 수 없습니다.

```sh
sh scripts/build-update-test.sh 0.0.9 18765 <임시 폴더>/old                       # 이전 버전 후보
sh scripts/build-update-test.sh 0.1.0 18765 <임시 폴더>/new --updater-artifacts   # 새 버전과 서명된 업데이트 파일
```

1. `new/Woodshed.app.tar.gz`와 `.sig` 내용으로 `latest.json`(`version`, `platforms.darwin-aarch64.{signature,url}`)을 만들어 `python3 -m http.server 18765 --bind 127.0.0.1`로 내려줍니다.
2. `open -g -n old/Woodshed.app`으로 후보를 엽니다. 업데이트되면 `~/Library/Logs/dev.mongmeo.woodshed/Woodshed.log`에 `Woodshed 0.0.9 started` 다음 `Woodshed 0.1.0 started`가 남습니다.
3. 후보는 설치된 앱과 같은 앱 데이터 폴더를 쓰므로 실행 전에 백업하고 끝나면 되돌립니다.

- 스크립트는 앱과 업데이트 파일을 지정한 폴더로 옮기고 `dist/`를 지워, `target/`에 테스트 빌드를 남기지 않습니다.
- `pnpm build`, `mise run app`, `mise run app:release`는 빌드한 `dist/`를 `scripts/check-release-dist.mjs`로 검사합니다. 자동 설치 코드가 들어 있으면 실패하고, 앱 빌드는 만든 앱을 지웁니다.
- `mise run app`·`mise run app:release`는 앱에 들어가는 프런트엔드를 바꿀 수 있는 설정을 받지 않습니다(`scripts/release-guard.sh`). `--config`·`-c` 인자, `TAURI_CONFIG`·`TAURI_FRONTEND_PATH`·`TAURI_APP_PATH`·`TAURI_PATH` 환경 변수, CLI가 스스로 합치는 `src-tauri/tauri.macos.conf.json`(`.json5`, `Tauri.macos.toml`, `Tauri.toml`)이 있으면 빌드 전에 멈춥니다.

### 릴리스

GitHub 릴리스는 손으로 만듭니다(CI 없음). Apple Silicon Mac에서, 깨끗한 `main`이 `origin/main`과 같을 때 실행합니다.

```sh
mise run release -- 0.2.0 --dry-run   # 같은 파일을 만들고 검증만. push·릴리스 없음
mise run release -- 0.2.0             # 실제 릴리스
mise run release -- 0.2.0 --notes-file notes.md   # 릴리스 본문과 latest.json notes (기본: "Woodshed 0.2.0")
```

필요한 것: 출시 키체인과 `notary.env`([배포 빌드](#배포-빌드-developer-id-서명공증)), [업데이트 서명 키](#업데이트-서명-키), `gh auth login`(저장소에 push·릴리스 권한), 네트워크(공증, 소스 아카이브, 첫 빌드의 모델 변환).

`scripts/release.sh`가 하는 일:

1. 확인: `main`이고 작업 트리가 깨끗하며 `origin/main`과 같은지, 태그 `v<버전>`과 그 릴리스가 로컬·원격에 없는지, 새 버전이 현재 버전과 기존 태그보다 큰 `X.Y.Z`인지, gh 로그인·출시 키체인·`notary.env`·업데이트 키가 있는지, 업데이트 공개키가 `tauri.conf.json`의 `plugins.updater.pubkey`와 같은지 봅니다.
2. yt-dlp·FFmpeg 소스 아카이브를 받아 `scripts/fetch-binaries.sh`의 고정 체크섬(`YTDLP_SOURCE_SHA256`, `FFMPEG_SHA256`)과 대조합니다.
3. `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`에 버전을 쓰고 커밋 "버전 X.Y.Z"와 annotated 태그 `vX.Y.Z`를 만듭니다.
4. `scripts/build-release.sh`로 Developer ID 서명·공증·staple한 앱과 서명된 업데이트 파일을 만들고, 앱을 담은 DMG를 서명·공증·staple합니다. 빌드가 추적 파일을 바꾸면 멈춥니다.
5. 검증: `latest.json`의 서명을 `tauri.conf.json`의 공개키로 확인하고(바꾼 파일은 거부되는지도), DMG를 `spctl --type open`으로, DMG 안과 업데이트 파일 안의 앱을 `codesign`·`stapler validate`·`spctl --type execute`·버전으로 확인합니다.
6. 커밋과 태그를 한 번에(`git push --atomic`) push하고, 초안 릴리스를 만들어 자산을 모두 올린 뒤 latest로 공개합니다. 앱이 읽는 `latest.json`은 공개된 뒤에야 보입니다.

산출물은 `releases/v<버전>/`(git 무시)에 모입니다.

| 파일                                           | 내용                                                                       |
| ---------------------------------------------- | -------------------------------------------------------------------------- |
| `Woodshed_<버전>_aarch64.dmg`                  | 공증한 앱을 담은 DMG(DMG도 서명·공증·staple)                               |
| `Woodshed_<버전>_aarch64.app.tar.gz`           | 업데이트 파일(공증한 앱)                                                   |
| `Woodshed_<버전>_aarch64.app.tar.gz.sig`       | 업데이트 서명                                                              |
| `latest.json`                                  | `version`, `notes`, `pub_date`, `platforms.darwin-aarch64.{signature,url}` |
| `ffmpeg-<버전>.tar.xz`, `yt-dlp-<버전>.tar.gz` | 앱에 들어간 sidecar의 소스([라이선스](#라이선스))                          |
| `SHA256SUMS`                                   | 위 파일의 SHA-256                                                          |
| `release-notes.md`                             | 릴리스 본문(올리지 않음)                                                   |

- `--dry-run`은 같은 파일을 만들고 같은 검증까지 합니다. 공증은 실제로 Apple에 제출합니다. push와 릴리스 생성은 하지 않고, 끝나면 버전 커밋과 태그를 지워 브랜치와 작업 트리를 원래대로 둡니다. `--allow-branch`를 더하면 `main`이 아닌 브랜치에서도 시험할 수 있습니다(dry-run 전용).
- 업데이트 엔드포인트는 `releases/latest/download/latest.json`이라, 초안과 프리릴리스는 설치된 앱에 보이지 않습니다.

#### 실패했을 때

- **push 전 실패**(확인, 빌드, 공증, 검증): 스크립트가 버전 커밋과 태그를 지우고 멈춥니다. 원인을 고치고 같은 명령을 다시 실행합니다.
- **공증 거부**: `notarytool log`가 거부 사유를 출력합니다. 서명·entitlements를 고친 뒤 다시 실행합니다.
- **push 실패**(`origin/main`이 앞서 나감, 권한이나 `main` 보호 규칙 등): `--atomic`이라 커밋도 태그도 올라가지 않고, 로컬도 되돌려집니다. `git pull` 뒤 다시 실행합니다.
- **업로드 실패**(push 뒤): 커밋과 태그는 이미 원격에 있습니다. 같은 파일로 업로드만 다시 합니다. 초안이 남아 있으면 이어서 올리고, 같은 이름의 자산은 덮어씁니다.

  ```sh
  mise run release -- 0.2.0 --upload-only
  ```

- **잘못 올린 릴리스**: 이미 받은 앱이 있을 수 있으니 지우지 말고, 고친 새 버전을 올립니다. 공개 직후 바로 거둬야 하면 `gh release edit v0.2.0 --draft=true`로 숨깁니다. 그러면 `latest.json`은 그 이전 릴리스를 가리킵니다.

#### 키 백업

아래 파일을 저장소 밖(암호 관리자, 암호화한 외장 저장소 등)에 함께 백업합니다. `notary.env`와 업데이트 키는 `~/Library/Application Support/woodshed-release/`에, 인증서는 출시 키체인에 있습니다. `.p12` 원본과 `.p8`은 `create-release-keychain.sh`와 `APPLE_API_KEY_PATH`에 준 위치에 있습니다. 어느 것도 저장소에 넣지 않습니다.

| 키                              | 파일                                                     | 잃으면                                                                                                          |
| ------------------------------- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Developer ID Application 인증서 | 키체인에 넣은 `.p12`와 그 암호                           | Apple Developer에서 새 인증서를 발급해 `create-release-keychain.sh`로 다시 넣습니다. 발급 수에 제한이 있습니다. |
| 공증용 App Store Connect API 키 | `AuthKey_<키 ID>.p8`, `notary.env`(키 ID, 발급자 ID)     | `.p8`은 한 번만 내려받을 수 있습니다. App Store Connect에서 키를 폐기하고 새로 만듭니다.                        |
| 업데이트 서명 키                | `updater.key`, `updater.key.pub`, `updater-key-password` | **대신할 수 없습니다.** 새 키로 서명한 업데이트는 설치된 앱이 거부하므로, 사용자가 새 DMG를 직접 받아야 합니다. |

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
- 두 프로그램의 그 버전 소스 아카이브는 각 릴리스에 함께 올립니다. 라이선스 고지(FFmpeg LGPL v2.1+ 전문과 빌드 구성·소스 위치, yt-dlp 번들의 GPLv3+와 소스 위치)는 [`LICENSES/`](LICENSES/README.md#bundled-programs)에 있고 앱 번들에 함께 들어갑니다.

### 스템 분리 모델 (Demucs htdemucs_6s → Core ML)

스템 분리는 Demucs v4 `htdemucs_6s`(6스템: drums, bass, other, vocals, guitar, piano)를 Core ML로 바꿔 앱 안에서 돌립니다. 가중치와 변환한 모델은 git에 넣지 않고 스크립트로 만듭니다.

```sh
mise run stem-model   # = pnpm stem-model = sh tools/stem-model/build.sh
# → src-tauri/models/htdemucs_6s_fp32.mlmodelc (115MB)
```

- 가중치: Hugging Face [`adefossez/HTDemucs-6s`](https://huggingface.co/adefossez/HTDemucs-6s)의 고정 리비전 `3c5ee475be62`에서 `5c90dfd2.safetensors`를 받아 SHA-256(`d2a1745f…d411`)을 확인합니다. Demucs 코드는 MIT(Meta Platforms)입니다. 가중치는 라이선스가 불명확해 CC BY-NC(비상업)로 보고, 이 모델을 넣은 릴리스 앱은 무료·비상업으로 배포합니다([라이선스](#라이선스) 절).
- 변환: `tools/stem-model/`의 `core.py`(STFT/iSTFT를 뺀 핵심부)를 `convert.py`가 fp32 mlprogram으로 바꾸고 `xcrun coremlcompiler`로 컴파일합니다. Python은 mise로 고정한 uv 0.10.7이 만든 Python 3.12.8 가상환경에서 `requirements.lock.txt`(torch 2.7.0, coremltools 9.0, demucs 4.1.0 등 전부 고정)로 돌립니다. 처음 한 번은 가중치 52MB 다운로드, Python 패키지 설치, 변환(약 5초)을 합니다. 새 캐시에서 약 21초 걸린 기록이 있고, 네트워크와 uv 캐시 상태에 따라 다릅니다.
- 결과와 가중치는 `~/Library/Caches/dev.mongmeo.woodshed/models`에 캐시되어 다음부터는 복사만 합니다. 스크립트·잠금 파일·가중치가 바뀌면 다시 변환합니다. 변환한 모델은 같은 입력이면 `model.mil`·가중치가 바이트 단위로 같습니다.
- `pnpm tauri build`는 빌드 전에 이 스크립트를 실행합니다(`beforeBuildCommand`). `mise run check`·`cargo test`·`pnpm tauri dev`는 모델이 없어도 됩니다. 이때 `build.rs`가 빈 자리표시 디렉터리를 만들고, 모델이 필요한 테스트는 건너뛴 사실을 출력합니다. dev 앱은 캐시에 모델이 있으면 그것을 씁니다.
- 앱 밖 DSP(STFT/iSTFT, 7.8초 청크, 삼각 가중치 오버랩-애드, 입력 정규화)는 Rust(`src-tauri/src/separation/`)에 있고, 기준 사양은 `tools/stem-model/dsp.py`(numpy)입니다. Rust 테스트는 `make_dsp_fixture.py`가 만든 고정 입력 기준값(`src-tauri/src/separation/testdata/dsp_reference.json`)과 비교합니다. `dsp.py`나 입력식을 바꾸면 이 파일을 다시 만듭니다.
- 모델이나 변환을 바꾸면 PyTorch 원본과 비교합니다: `make_audio.py`로 시험 음원을 만들고, `ref_torch.py`로 기준을 만든 뒤 `cargo run --release --example separate_wav -- <mix.wav> <out>`의 출력과 `compare.py`로 비교합니다. 앱 경로(가져오기 → 분리 → manifest → 재시작 뒤 다시 열기, 강제 종료 뒤 복구)는 `cargo run --release --example stem_separate -- --library <임시 폴더> <파일>`로 창 없이 확인합니다.

분리 동작(코드: `src-tauri/src/separation/`)

- Core ML은 `CPUAndGPU`로 돌립니다. GPU가 가끔 틀린 결과를 냅니다: Apple M5·macOS 27에서 청크 사이에 GPU가 쉬면 다섯 번에 한 번꼴로 한 청크가 최대 3e-2까지 어긋났습니다(청크 SNR 26~60dB, 맞으면 120dB). 맞는 결과는 실행마다 비트 단위로 같고 틀린 결과는 매번 달라서, 같은 청크를 두 번 돌려 일치할 때만 받아들이고 네 번 안에 일치하지 않으면 그 청크만 CPU(결정적, 최대 오차 7e-6)로 돌립니다. 그래서 GPU 시간은 단일 실행의 약 2배입니다. 긴 곡에서는 불일치가 실제로 생깁니다: 5분 곡에서 1번, 30분 곡(308청크)에서 17번 세 번째 실행으로 해결했습니다. 이것은 이 기기에서 관측한 현상에 맞춘 완화책이고, 정확성을 따로 보증하는 기준은 아닙니다.
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

## 알려진 제한

- 스템 분리는 Intel Mac에서 시험하지 않았습니다. 서명·번들된 앱의 첫 실행에서는 GPU 커널 컴파일에 10~35초가 걸릴 수 있습니다.
- YouTube는 JS 런타임 없이 yt-dlp 기본 클라이언트로 받습니다. 일부 영상 형식은 받지 못할 수 있습니다.
- 마이크 권한 문구는 `src-tauri/Info.plist`(기본, 영어)와 `src-tauri/macos/{en,ko}.lproj/InfoPlist.strings`(`bundle.macOS.files`로 번들에 복사)에 있습니다.
- 출력이 바뀌어 샘플레이트가 달라지면 이미 예약된 이벤트의 프레임 위치는 그대로라 시간상 위치가 달라집니다. 메트로놈·드럼·스템 플레이어는 `audio://output-changed`를 받아 다시 예약해야 합니다.
- 출력 버퍼 하한은 64프레임입니다. 기기가 더 작은 버퍼를 허용해도 기기별 드롭아웃을 피하려고 쓰지 않습니다.

## 라이선스

Woodshed의 소스 코드는 [MIT License](LICENSE)로 공개합니다. 앱은 아래 서드파티 구성 요소와 함께 빌드·배포되며, 고지 전문은 [`LICENSES/`](LICENSES/README.md)에 있고 앱 번들의 `Contents/Resources/LICENSES/`에도 들어갑니다.

| 구성 요소                                | 앱에 들어가는 방식     | 라이선스                                                                                  | 고지                                                              |
| ---------------------------------------- | ---------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Rust 크레이트(Tauri, 오디오, Core ML 등) | 앱 바이너리에 링크     | MIT, Apache-2.0, MPL-2.0, BSD, ISC, Zlib, Unicode 등(GPL 없음)                            | [`LICENSES/rust-crates.md`](LICENSES/rust-crates.md)              |
| npm 패키지(React, i18next 등)            | 웹뷰 번들              | MIT, Apache-2.0 OR MIT(GPL 없음)                                                          | [`LICENSES/npm-packages.md`](LICENSES/npm-packages.md)            |
| FFmpeg 9.0.2                             | 별도 프로그램(sidecar) | LGPL-2.1-or-later, 오디오 전용 구성(`--disable-gpl --disable-nonfree --disable-version3`) | [빌드 구성·소스 위치](LICENSES/README.md#ffmpeg-902)              |
| yt-dlp 2026.10.26 `yt-dlp_macos`         | 별도 프로그램(sidecar) | yt-dlp는 Unlicense, PyInstaller 번들 전체는 GPLv3+                                        | [소스 위치](LICENSES/README.md#yt-dlp-20261026-yt-dlp_macos)      |
| Demucs `htdemucs_6s` 모델(Core ML 변환)  | 번들 리소스            | 코드 MIT, **가중치는 CC BY-NC 추정(비상업)**                                              | [근거](LICENSES/README.md#stem-separation-model-bundled-resource) |
| 드럼 샘플(VCSL)                          | 번들 리소스            | CC0-1.0                                                                                   | [`LICENSES/drum-samples.md`](LICENSES/drum-samples.md)            |

- Demucs 가중치: 라이선스가 불명확합니다. Hugging Face 모델 카드에 license 필드가 없고, Demucs README([License 절, `829d2189`](https://github.com/adefossez/demucs/blob/829d21890023800713e5dd231c67e43ecc06069b/README.md#license))는 6스템 변형이 Meta 내부 데이터로만 학습되어 CC BY-NC가 적용될 가능성이 높다고 적습니다. 그래서 CC BY-NC로 보고, 이 모델을 넣은 릴리스 앱은 무료로, 비상업 용도로만 배포합니다. 출처: Meta Demucs(`htdemucs_6s`). Woodshed 소스 코드 자체는 MIT입니다. 모델은 저장소에 넣지 않고 `mise run stem-model`이 원본에서 받아 만듭니다.
- yt-dlp와 FFmpeg는 앱이 자식 프로세스로 실행만 하고 링크하지 않습니다. 각 릴리스에 그 버전의 소스 아카이브를 함께 올립니다. 둘 다 원본을 고치지 않았고, 정확한 버전·체크섬·빌드 구성은 `scripts/fetch-binaries.sh`에 고정되어 있습니다.
- [`DESIGN.md`](DESIGN.md)는 VoltAgent/awesome-design-md(MIT, Copyright (c) 2026 VoltAgent)에서 가져왔습니다([`LICENSES/awesome-design-md-MIT.txt`](LICENSES/awesome-design-md-MIT.txt)).
- 의존성을 바꾸면 `mise exec -- python3 scripts/collect-licenses.py`로 Rust·npm 고지를 다시 만듭니다.
