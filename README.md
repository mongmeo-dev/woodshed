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
mise exec -- pnpm tauri build
# → src-tauri/target/release/bundle/macos/Woodshed.app
```

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

앱도 시작할 때와 출력 장치를 바꿀 때 실측 출력 레이턴시를 로그에 남깁니다(`~/Library/Logs/dev.mongmeo.woodshed/Woodshed.log`).

## 디자인

- [`DESIGN.md`](DESIGN.md)는 [VoltAgent/awesome-design-md](https://github.com/VoltAgent/awesome-design-md)(MIT)의 [`design-md/apple/DESIGN.md`](https://github.com/VoltAgent/awesome-design-md/blob/fcc702849322/design-md/apple/DESIGN.md)(커밋 `fcc702849322`)를 그대로 가져온 것입니다. 밝은 캔버스와 어두운 타일 팔레트가 함께 있어 라이트/다크를 모두 만들 수 있고, SF Pro 시스템 폰트와 단일 강조색이 미니멀 mac 앱에 맞아 골랐습니다.
- 토큰은 `src/styles/tokens.css`의 `--ws-*` CSS 변수로 옮겼습니다. 다크 모드는 DESIGN.md의 어두운 타일 색(`surface-black`, `surface-tile-1~3`, `body-on-dark`, `primary-on-dark`)으로 구성합니다.
- 컴포넌트 참고: [component.gallery](https://component.gallery)의 Navigation(Primer NavList: 현재 화면을 `aria-current`로 표시하는 사이드바), Segmented control(Primer SegmentedControl, Radix Toggle Group).
- 모션 참고: [Kinetics](https://kinetics.colorion.co)의 Tab Pill Glide(선택 표시가 대상 크기를 재고 미끄러짐 — 0.4s를 240ms로 줄임), Pulse Badge(링이 커지며 사라짐 — 테스트음이 들리는 순간 한 번만 재생).

## 드럼 머신

- 엔진: `src-tauri/src/drums/`(샘플 보이스·스텝 시퀀서·Tauri 명령), 화면: `src/tools/drumMachine/`.
- 편집 중인 패턴과 저장한 패턴은 앱 데이터 디렉터리(`~/Library/Application Support/dev.mongmeo.woodshed/`)의 `drum-machine.json`에 둡니다. `settings.json`과 따로 저장합니다.
- 스케줄러 스레드가 120ms 앞까지 스텝을 샘플 단위로 예약합니다. 재생 중 편집·템포·스윙·길이 변경과 출력 장치 변경(세대 증가)은 20ms 앞부터 취소 후 다시 예약하므로 재생이 멈추지 않습니다.
- 화면의 현재 스텝은 스텝을 예약할 때 계산한 들리는 시각(`heardAtMs`)을 전용 채널(`drums_subscribe`)로 미리 받아 그 시각에 표시합니다.

## 드럼 샘플

드럼 머신의 샘플 10개(`src-tauri/resources/drums/`)는 [Versilian Community Sample Library](https://github.com/sgossner/VCSL)(커밋 `c1ea7bc`)에서 가져왔고, 모두 **CC0 1.0**(퍼블릭 도메인)입니다. 파일별 원본 경로와 가공 내용은 [`LICENSES/drum-samples.md`](LICENSES/drum-samples.md), 라이선스 원문은 [`LICENSES/CC0-1.0.txt`](LICENSES/CC0-1.0.txt)에 있습니다.

## 미결 사항

- 스템 분리 실행(Demucs)은 stem-engine 작업에서 연결합니다. 화면에는 분리 버튼·진행률 자리만 있습니다(계약: [`AGENTS.md`](AGENTS.md#스템-라이브러리-계약-src-taurisrcstems)).
- 번들한 `yt-dlp_macos`는 PyInstaller 묶음이라 결합 저작물로는 GPLv3+입니다(yt-dlp 자체는 Unlicense). 앱은 별도 프로세스로 실행만 합니다. 배포 방식을 정할 때 다시 확인합니다.
- YouTube는 JS 런타임 없이 yt-dlp 기본 클라이언트로 받습니다. 일부 영상 형식이 막히면 JS 런타임(deno 등) 번들 여부를 정해야 합니다.
- 마이크 권한 문구는 `src-tauri/Info.plist`(기본, 영어)와 `src-tauri/macos/{en,ko}.lproj/InfoPlist.strings`(`bundle.macOS.files`로 번들에 복사)에 있습니다.
- 출력이 바뀌어 샘플레이트가 달라지면 이미 예약된 이벤트의 프레임 위치는 그대로라 시간상 위치가 달라집니다. 메트로놈·드럼·스템 플레이어는 `audio://output-changed`를 받아 다시 예약해야 합니다.
- 출력 버퍼 하한은 64프레임입니다. 기기가 더 작은 버퍼를 허용해도 기기별 드롭아웃을 피하려고 쓰지 않습니다.
