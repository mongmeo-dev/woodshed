# Woodshed AI Agent Rule

베이스·기타 연습 도구 macOS 앱(Tauri 2 + React/TS + Rust)입니다. 루트 저장소(mongmeo-common)의 `AGENTS.md` 규칙(한국어 경어체, 한국어 커밋 메시지, 기능 단위 커밋, Co-Author 없음, `mise exec -- <명령>`, 명시 버전 의존성)을 그대로 따르고, 아래 규칙을 추가로 지킵니다.

## 검사 명령

- 모든 도구는 `mise.toml`에 고정되어 있습니다. 명령은 `mise exec -- <명령>` 또는 `mise run <태스크>`로 실행합니다.
- 커밋·push 전에 `mise run check`를 통과시킵니다. 내용: `biome check` → `tsc --noEmit` → `vitest run --maxWorkers=2` → `cargo fmt --check` → `cargo clippy --all-targets --locked -D warnings` → `cargo test --locked`.
  - 웹만: `mise run check:web`, Rust만: `mise run check:rust`.
- 이 Mac은 다른 프로젝트와 공유합니다. 무거운 명령(빌드·전체 검사)은 한 번에 하나만 돌리고, `CARGO_BUILD_JOBS=4`(mise env에 설정됨), vitest는 `--maxWorkers=2`를 씁니다. 개발 서버·빌드한 앱은 확인 후 종료합니다.
- 앱 빌드: `mise run app`(`scripts/build-app.sh`: 로컬 서명 인증서가 있으면 그것으로, 없으면 ad-hoc 서명) → `src-tauri/target/release/bundle/macos/Woodshed.app`. 인증서는 `mise run signing-cert`로 만들며 운영자 키체인 검색 목록을 바꾸므로 운영자 확인 뒤에만 실행합니다(README 로컬 서명).
- 실제 출력 장치 레이턴시 확인: `cd src-tauri && mise exec -- cargo run --release --example latency_probe` (스피커로 짧은 클릭음이 납니다).

## 화면 문구 규칙

- 레이블, 현재 상태 값, 사용자가 지금 할 일, 입력 오류, 확인 대화상자의 결과 안내만 씁니다.
- 히어로 문구, 앱 소개 문장, 구현 상태나 정책을 설명하는 문장은 절대 넣지 않습니다. 도구 화면 제목 외 설명 문구도 넣지 않습니다.
- 모든 문구는 `src/locales/ko.json`과 `src/locales/en.json`에 같은 키로 넣습니다(키 불일치는 `src/i18n/i18n.test.ts`가 잡습니다). 컴포넌트에 문자열을 하드코딩하지 않습니다. 언어 이름(`한국어`, `English`)만 예외입니다.
- 한국어 문구는 `/humanize-korean:humanize-korean` 스킬로 번역투가 없는지 다듬고, 영어도 자연스러운 앱 문구(Title Case 레이블)로 씁니다.
- 문구를 고친 뒤 `mise exec -- node scripts/collect-copy.mjs --check`로 하드코딩 문구·쓰지 않는 키를 확인합니다. 윤문 입력은 `--ko-text`, 키별 ko/en 표는 인자 없이 실행해 얻습니다.

## 디자인 규칙

- `DESIGN.md`(VoltAgent/awesome-design-md의 apple 분석본)가 루트 디자인 룰보다 우선합니다.
- 색·타이포·간격·라운드는 `src/styles/tokens.css`의 `--ws-*` 변수만 씁니다. 새 색 값을 컴포넌트에 직접 쓰지 않습니다. 라이트/다크 둘 다 확인합니다.
- 강조색은 하나(`--ws-color-accent`)뿐입니다. 카드·버튼·텍스트에 그림자를 넣지 않습니다. 굵기는 400/600만 씁니다. 버튼 눌림은 `transform: scale(0.95)`입니다.
- 라운드는 컴팩트 컨트롤 `--ws-radius-sm`, 카드 `--ws-radius-lg`, 액션 버튼 `--ws-radius-pill`만 씁니다.
- 모션은 `--ws-duration-*`/`--ws-ease-glide`를 쓰고, `prefers-reduced-motion`에서는 0으로 떨어집니다.

## 앱 구조

- 도구 목록: `src/tools/tools.ts`의 `TOOL_IDS`(`tuner`, `metronome`, `drumMachine`, `stemSplitter`, `fretboard`). 각 도구 노드는 `src/tools/<도구>/`에 화면을 만들고 `src/App.tsx`에서 `ToolScreen` 대신 연결합니다.
- 설정: `src-tauri/src/settings.rs`(`Settings`)와 `src/lib/settings.ts`가 짝입니다. 필드를 추가하면 둘 다 고치고, 누락 필드는 기본값으로 읽히게 합니다(`#[serde(default)]`).
  - 설정 파일 쓰기는 `SettingsStore::update(|s| ...)`로 자기 필드만 바꿉니다. 쓰기는 차례대로(현재값 읽기·변경·파일 쓰기·캐시 교체) 일어나 다른 소유자(화면, 출력 관리자, 튜너)의 변경을 되돌리지 않고, 읽기(`get`, `settings_load`)는 파일 쓰기를 기다리지 않습니다. `get()` 후 `save()`로 통째로 쓰지 않습니다.
  - 예외: `Settings.tuner`(입력 장치·채널·기준음)는 튜너 명령(`tuner_select_input`, `tuner_set_reference`)이 소유하고 `settings_save`가 보존하므로 `src/lib/settings.ts`에 두지 않습니다. 튜너 화면은 `tuner_start`가 돌려주는 상태를 씁니다.
- 프런트↔Rust 타입은 `src/lib/audio.ts`가 Rust serde 타입(camelCase)을 그대로 옮긴 것입니다. 한쪽을 바꾸면 다른 쪽도 바꿉니다.
- 플랫폼 전용 코드는 `#[cfg(target_os = "...")]`로 격리합니다.

## 오디오 엔진 계약 (`src-tauri/src/audio/`)

이후 노드는 이 계약 위에 얹습니다. 계약을 바꿔야 하면 멈추고 conductor에게 알립니다.

### 실시간 규칙

- 오디오 스레드(`Source::on_event`, `Source::render`, 엔진 콜백)에서는 할당·해제·락·블로킹·로그·시스템 호출을 하지 않습니다.
- 버퍼는 `Source::prepare`(일반 스레드)에서 미리 잡습니다. 렌더 블록은 최대 `MAX_BLOCK_FRAMES`(1024)프레임, 채널은 최대 `MAX_CHANNELS`(8)입니다.
- UI → 오디오는 `EngineHandle`의 명령(락프리 rtrb SPSC 큐)으로만 보냅니다. 제거된 소스는 UI 스레드로 돌아와 해제됩니다.
- `render_callback_never_allocates_or_frees` 테스트처럼, 새 소스는 카운팅 할당자로 렌더 중 할당 0회를 테스트합니다.

### 소스 만들기

```rust
use woodshed_lib::audio::{EventKind, Source, StreamFormat};

struct Click { /* 미리 할당한 상태 */ }

impl Source for Click {
    fn prepare(&mut self, format: StreamFormat) { /* 일반 스레드: 샘플레이트 반영, 버퍼 할당 */ }
    fn on_event(&mut self, kind: &EventKind) { /* 정확히 이벤트 프레임에 호출됨 */ }
    fn render(&mut self, out: &mut [f32], channels: usize) { /* 0으로 채워진 interleaved 버퍼에 씀 */ }
}
```

### 붙이고 예약하기

- `handle.add_source(Box::new(src), gain) -> Result<SourceId, EngineError>` — `prepare` 후 믹서에 붙음. 살아 있는 소스(붙었거나 큐에 있는 것)가 `max_sources`면 `EngineError::MixerFull`로 거절되고 소스는 호출한 스레드에서 해제됩니다. 오디오 스레드는 소스를 해제하지 않고 모두 핸들로 돌려보내며, 핸들이 명령마다(`collect_retired`) 해제합니다. 소스별 게인(선형, 약 5ms 글라이드)은 `set_gain`, 마스터는 `set_master_gain`.
- `handle.schedule(frame, id, kind)` — 절대 샘플 프레임(`FramePos`, 엔진 생성 후 누적, 장치를 바꿔도 리셋 안 됨)에 이벤트 예약. 그 프레임 이전에 큐에 들어오면 0프레임 오차로 렌더됩니다. 이미 렌더된 프레임이면 다음 프레임에 렌더되고 `stats().late_events`가 늘어납니다.
- `handle.frame_after(lead_secs)` — 지금 렌더 위치 + 여유. 실시간 예약은 최소 한 콜백(64프레임 ≈ 1.3ms) 이상 앞서, 보통 20~50ms 앞서 넣습니다.
- `handle.schedule_notify(...)` — 렌더 후 `RenderedEvent`를 보고합니다(화면 동기용).
- `handle.cancel(id, from_frame)` — 그 프레임 이후 대기 이벤트 제거(정지·템포 변경 시).
- `EventKind`: `Trigger { note, velocity }`, `Start`, `Stop`, `Seek { frame }`, `Param { id, value }`.
- 용량: 소스 64, 대기 이벤트 4096, 명령 큐 4096(`EngineConfig`). 큐가 차면 `EngineError::QueueFull`, 예약이 차면 이벤트를 버리고 `stats().dropped_events`가 늘어납니다.

### 출력 세대와 포맷 변경

- 출력 스트림을 (다시) 열 때마다 엔진이 새 포맷으로 `reformat`되고 **세대**(`handle.generation()`, `stats().generation`)가 1 늘어납니다. 이때 이미 붙은 소스와 큐에 대기 중인 소스 모두 새 포맷으로 `prepare`됩니다(일반 스레드, 스트림이 멈춘 동안).
- 프레임 시계는 이어지지만 샘플레이트가 바뀔 수 있으므로, 이전 세대에 계산한 예약 프레임은 시간상 위치가 달라집니다. 메트로놈·드럼·스템 플레이어는 `audio://output-changed`를 받으면 `cancel` 후 새 `sampleRate`로 다시 예약합니다.
- 새 세대의 첫 콜백 전에는 `handle.timing()`·`audio_heard_at`이 `None`/`null`입니다.
- 출력 버퍼는 64프레임 이상 중 기기가 허용하는 최솟값입니다(64프레임 하한은 기기별 드롭아웃 방지용, 48kHz에서 약 1.3ms).

### 앱에서 엔진 쓰기

- Tauri 상태 `AudioService`(`src-tauri/src/app_audio.rs`)가 엔진을 가집니다. `audio.engine().lock()`으로 `AudioEngine`을 얻어 `handle()`을 씁니다. 락은 짧게 잡습니다(오디오 스레드는 이 락을 쓰지 않습니다).
- 도구별 Tauri 명령은 도구 모듈에 두고 `lib.rs`의 `generate_handler!`에 추가합니다.

### 화면 동기 ("이 소리가 실제로 들리는 시각")

- 오디오 콜백이 콜백 시작 프레임과 그 프레임의 DAC 도달 시각(스트림 클럭, 출력 레이턴시 포함)을 seqlock으로 게시합니다.
- `AudioEngine::heard_at_epoch_ms(frame)` — 프레임이 들리는 Unix epoch ms. 명령 `audio_heard_at`.
- `notify` 이벤트는 `audio_subscribe(onEvent: Channel<PlaybackEvent>)`로 등록한 채널에 `{ sourceId, scheduledFrame, renderedFrame, kind, generation, heardAtMs }`로 옵니다. `heardAtMs`는 렌더 당시 타이밍으로 계산하며, 이전 세대에 렌더된 알림은 보내지 않습니다. 프런트는 `src/lib/audio.ts`의 `onPlaybackEvent`로 받고, `delayUntilHeard(heardAtMs)`(= `heardAtMs - performance.timeOrigin - performance.now()`) 뒤에 그립니다. 예제: 설정 화면 테스트음(`useHeardBeat`).
- 미리 아는 프레임(메트로놈 박 등)은 예약 시점에 `audio_heard_at`으로 시각을 받아 미리 애니메이션을 걸 수 있습니다.

### 입력 (튜너)

- `list_devices(Direction::Input)` / 명령 `audio_list_devices({ direction: "input" })`로 장치·채널 수 열거.
- `open_input(&InputRequest { device_id, channel: InputChannel::Single(0) | MixDown, target_buffer_frames: 64, ring_frames })` → `InputCapture { samples: rtrb::Consumer<f32>, info, shared }`. 모노 f32 샘플을 일반 스레드에서 읽습니다. `shared.overflows()`, `capture_latency_ns()`, `take_error()` 제공. `InputCapture`를 drop하면 멈춥니다.
- macOS 마이크 권한 문구는 `src-tauri/Info.plist`에 있습니다.

### 출력 관리자 (출력의 단일 소유자)

- 출력 스트림을 열고 바꾸고 복구하는 일은 `audio-output` 스레드의 `OutputManager`(`src-tauri/src/audio/output_manager.rs`)만 합니다. 다른 코드는 `AudioEngine::open_output`을 직접 부르지 않습니다.
  - 수동 선택: async 명령 `audio_set_output_device({ deviceId })`가 관리자에게 기한(10초) 있는 요청을 보내고 블로킹 풀에서 응답을 기다립니다(웹뷰 IPC 스레드를 막지 않음). 실행 루프는 `run_output_manager`이고, 대기 중인 요청을 다음 틱보다 먼저 도착 순서대로 처리합니다. 다음 틱은 이전 틱이 끝난 뒤 1초입니다.
  - 착수 시점에 기한이 지난 요청은 실행하지 않고 버립니다. 착수한 요청은 기한이 지나도 끝까지 적용하고, 결과는 `audio://output-changed`와 `audio_status().preferredDeviceId`로 드러납니다.
  - 성공하면 처리 당시 상태 `{ output, preferredDeviceId }`, 실패하면 `{ code: "openFailed" | "saveFailed" | "unavailable" }`(`src/lib/audio.ts`의 `OutputSelectError`). `openFailed`·`saveFailed`여도 선택은 실행 중 선호로 적용됩니다(`openFailed`면 기본 장치로 재생하며 선택 장치를 재시도, `saveFailed`면 새 출력으로 재생하지만 디스크에는 이전 값). `unavailable`이면 나중에 적용될 수 있으므로 화면은 `preferredDeviceId`를 따릅니다.
  - `AudioStatus.preferredDeviceId`가 실행 중 선호의 기준입니다(재생 중인 장치나 설정 파일과 다를 수 있음). 출력 장치 값은 관리자 소유라 `settings_save`는 `outputDeviceId`를 바꾸지 않습니다.
  - 엔진을 잠그거나 장치를 조회하는 명령은 모두 async입니다. 새 명령도 그렇게 만듭니다.
  - 앱 종료 시 `AudioService::shutdown`이 `Shutdown` 요청을 보내고 스레드를 회수합니다.
- 상태 머신(`OutputMode`).
  - `OnTarget`: 선호 장치(없으면 시스템 기본값)로 재생 중. 매 틱 출력 부재·스트림 오류(Xrun 제외)·장치 분리·기본 장치 변경(기본값 모드만)·장치 샘플레이트 변경을 검사해 처음부터 다시 엽니다.
  - `Fallback { preferred_retry }`: 선호 장치를 쓸 수 없어 기본값으로 재생 중. 선호 복귀 시도는 백오프(실패마다 1→30초 두 배)로만 하고, 폴백이 정상이면 건드리지 않습니다.
  - `Failed { retry }`: 아무것도 열리지 않음. 같은 백오프로 재시도.
  - 백오프는 장치 목록 변경·사용자 선택·선호 장치 성공·마이크 응답 때만 초기화됩니다.
- 마이크 응답 대기: 열려는 장치나 시스템 기본 출력 장치에 입력이 있고(스냅샷 `inputs`에 같은 id) 마이크 권한이 아직 미결정이면, macOS는 출력 스트림 생성을 사용자가 권한 창에 답할 때까지 붙잡습니다(cpal AUHAL은 생성자에서 기본 출력 장치에 먼저 연결하므로 입력 없는 장치를 골라도 같음).
  - 관리자는 이때 백엔드(`OutputEnv::open`)를 부르지 않고 그 열기를 바로 실패로 처리하므로, 관리자 스레드는 CoreAudio 호출에 묶이지 않고 장치 감시·선택·종료를 그대로 처리합니다. 판단은 `OutputEnv::microphone_answered`(AVCaptureDevice 권한 상태)로 합니다.
  - 목표(선호 장치, 없으면 기본값)가 재생 중이 아니고 열기가 이렇게 막혀 있으면 대기 상태입니다. 대기가 시작될 때 `OutputEnv::waiting_for_microphone(true)`가 권한 창을 띄우고(튜너와 같은 창), 응답 콜백이 `ManagerRequest::Recheck`로 관리자를 깨웁니다. 깨어난 검사는 백오프와 상관없이 목표를 처음부터 엽니다(`Starting`).
  - 대기 중에도 입력 없는 기본 출력은 그대로 재생합니다. 거부해도 열기는 진행됩니다(입력만 무음).
  - `audio_status().waitingForMicrophone`이 대기 여부이고, 설정 화면은 이때 출력 칸에 '마이크 접근을 허용하세요'만 보입니다.
- 출력 교체: 새 스트림을 엔진 락 밖에서 빌드(`OutputStarter`)하고, `commit_output`에서 이전 스트림을 일시정지 → reformat → 새 스트림 재생 순으로 바꿉니다. 빌드가 실패하면 기존 출력은 손대지 않습니다. 새 스트림 재생이 실패하면 코어를 이전 포맷·세대로 되돌리고 이전 스트림을 다시 재생합니다. 어느 실패든 세대는 바뀌지 않습니다.
- 장치 목록은 틱과 선택 모두 같은 경로(`observe_devices`)로 관찰하므로, 어느 입력이 먼저 보든 변경 알림은 한 번 나갑니다.
- 이벤트 불변식.
  - `audio://output-changed` (`{ generation, deviceId, sampleRate, channels, bufferFrames }`): 출력 열기 성공마다 정확히 1회, 성공 지점에서 발행합니다(이후 설정 저장 실패와 무관). 프런트는 `onOutputChanged`로 받습니다.
  - `audio://devices-changed` (`DeviceSnapshot { outputs, inputs, defaultOutput, defaultInput }`): 장치 목록·기본 장치가 바뀐 틱에만. 입력을 쓰는 노드는 이를 받아 입력을 다시 엽니다.
- 상태 머신은 `OutputEnv` 트레이트로 환경을 분리해 가짜 환경으로 테스트합니다. 출력 관련 동작을 바꾸면 여러 틱 시나리오 테스트를 추가합니다.

### 오프라인 렌더 (테스트)

```rust
let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
let id = engine.handle.add_source(Box::new(MySource::new()), 1.0).unwrap();
engine.handle.schedule(1_000, id, EventKind::Start).unwrap();
let buffer = engine.render(48_000, 128); // 48 000프레임을 128프레임 콜백으로, interleaved
let reports = engine.take_notifications();
```

- 오프라인 스트림 클럭은 0ns에서 시작하고 레이턴시 0입니다(프레임 f = f / sample_rate 초).
- 검사용 소스: `audio::testing::{ImpulseSource, ConstantSource}`.

## 스템 라이브러리 계약 (`src-tauri/src/stems/`)

스템 분리 화면(stem-player)과 분리 엔진(stem-engine)이 이 계약을 공유합니다. 바꿔야 하면 멈추고 conductor에게 알립니다.

### 디렉터리 (앱 데이터 `songs/<id>/`)

- `manifest.json`: `Manifest`(camelCase JSON, 아래).
- `original.<ext>`: 가져온 파일 원본 그대로입니다. YouTube는 yt-dlp가 받은 파일(`webm`·`m4a` 등)입니다. 분리 입력으로 씁니다.
- `audio.wav`: 재생용 사본입니다. **16bit PCM WAV, 원본 샘플레이트 그대로, 1~2채널.**
- `waveform.json`: 파형 요약입니다(구간별 피크, 0~255 정수 배열).
- `stems/<name>.wav`: 분리 결과입니다. `<name>`은 `vocals`, `drums`, `bass`, `guitar`, `piano`, `other`. **형식은 `audio.wav`와 같게 맞춥니다: 16bit PCM WAV, manifest `sampleRate`와 같은 레이트, 1~2채널, 길이는 `frames`와 같게.** 레이트가 다르면 곡을 열지 못하고, 짧은 스템은 뒤가 무음으로 재생됩니다.
- `.separating/<name>.wav`: 분리 중인 스템입니다. 6개가 모두 끝나면 디렉터리째 이름을 바꿔 `stems/`와 교체하고(이전 `stems/`는 교체하는 동안만 `.stems-old/`), 취소·실패·재시작 때 지웁니다.
- 가져오기는 `songs/.incoming/<id>/`에서 조립한 뒤 이름 바꾸기로 `songs/<id>/`에 놓습니다. 목록은 점(.)으로 시작하는 폴더를 건너뛰고, 앱을 시작할 때 `.incoming`을 비웁니다.

### manifest

```json
{
  "version": 1,
  "id": "19a7…",
  "title": "곡 제목",
  "source": { "kind": "file", "fileName": "song.mp3" },
  "original": "original.mp3",
  "sampleRate": 44100,
  "channels": 2,
  "frames": 13230000,
  "addedAtMs": 1794468000000,
  "stems": [{ "name": "vocals", "status": "running" }]
}
```

- `source`는 `{ "kind": "file", "fileName" }` 또는 `{ "kind": "youtube", "url", "videoId" }`입니다.
- `stems`가 없거나 비어 있으면 분리 전입니다. `status`는 `pending` | `running` | `ready` | `failed`입니다.
- stem-engine은 분리 전인 곡을 큐에 넣을 때 6개 항목을 `pending`, 시작할 때 `running`으로 씁니다. 6개를 `.separating/`에 다 쓴 뒤 `stems/`로 교체하고 한 번의 `Library::update`로 모두 `ready`로 바꿉니다. 취소하면 빈 목록(분리 전), 실패하면 `failed`입니다.
- **이미 모든 항목이 `ready`인 곡을 다시 분리할 때는 새 스템 6개가 끝나 교체할 때까지 `stems/`와 manifest를 건드리지 않습니다.** 취소·실패·앱 종료 때도 이전 스템이 그대로 남습니다. 이때 진행 중 표시는 manifest가 아니라 이벤트와 `stems_separations`로만 보입니다.
- 커밋 시점: 이미 분리된 곡은 `.separating/` → `stems/` 이름 바꾸기가 커밋이고(manifest는 이미 `ready`라 다시 쓰지 않음) 그 뒤에는 실패할 단계가 없습니다. 분리 전 곡은 `ready`로 바꾸는 `Library::update`가 커밋이고, 이것이 실패하면 새 `stems/`를 치우고 이전 디렉터리를 되돌립니다. 그래서 `stems/` 옆의 `.stems-old/`는 커밋이 끝난 뒤에만, `stems/` 없이 혼자 있는 `.stems-old/`는 커밋 전에만 생깁니다.
- 앱을 시작하면 stem-engine이 복구합니다: `.separating/`을 지우고, `stems/` 없이 남은 `.stems-old/`는 되돌리고 `stems/` 옆에 남은 것은 지우며, 작업이 없는 `pending`·`running` 항목은 빈 목록으로 되돌립니다(교체까지 끝난 경우는 `ready`). 분리를 시작할 때도 남은 `.stems-old/`를 같은 규칙으로 정리합니다.
- 읽기는 Rust `stems::Library`의 `get`·`list`로 합니다. **가져온 뒤 manifest를 바꾸는 일은 `Library::update(id, |manifest| …)`로만 합니다.** 이 함수는 프로세스 전역 잠금 안에서 파일을 다시 읽고, 클로저로 고친 뒤, 고유한 임시 파일과 이름 바꾸기로 저장합니다. 그래서 분리 진행과 화면이 동시에 고쳐도 서로의 변경을 잃지 않습니다. manifest 파일을 직접 쓰지 않습니다. 타입은 `Manifest`, `StemName`, `StemStatus`, `StemEntry`입니다.
- 트랙 결정: 모든 항목이 `ready`일 때만 `Manifest::tracks()`가 스템 트랙을 `vocals → drums → bass → guitar → piano → other` 순서(manifest에 있는 것만)로 돌려주고, 그 전에는 `original`(`audio.wav`) 하나입니다. 믹서는 이 목록대로 트랙 수가 바뀝니다.

### 화면 연결

- 분리 명령(stem-engine, `src-tauri/src/separation/`): `stems_separate({ songId })`는 곡을 큐에 넣고, `stems_cancel_separation({ songId })`는 대기 중이면 바로 빼고 실행 중이면 다음 청크(7.8초)에서 멈춥니다. 둘 다 결과는 이벤트로 옵니다. `stems_separations()`는 끝나지 않은 분리(실행 중 먼저, 그다음 대기)를 마지막 이벤트 모양으로 돌려줍니다. 분리는 한 번에 한 곡씩 합니다.
- 진행 이벤트 `stems://separation`(Rust `stems::SEPARATION_EVENT`, `SeparationEvent`): `{ songId, state: "running" | "done" | "failed" | "cancelled", progress: 0.0~1.0 | null }`. `running`의 `progress`는 대기·모델 준비 중이면 `null`, 청크를 시작하면 0.0, 이후 끝난 청크 비율입니다. `done`은 `progress` 1.0, 나머지는 `null`입니다.
- 화면(`SeparationSlot`): 분리 전이거나 실패한 곡에 분리 버튼을 보이고, `running`이면 진행률(`null`이면 준비 중)과 취소 버튼, `failed`면 짧은 오류를 보입니다. `done`·`failed`·`cancelled`를 받으면 곡 목록을 다시 읽고, `done`이면 열린 곡을 같은 위치에서 다시 엽니다(스템 트랙 믹서). 화면은 이벤트 구독을 화면이 열려 있는 동안 한 번만 하고, 구독이 연결된 뒤에 `stems_separations()`와 곡 목록을 다시 읽어 맞춥니다(그 뒤 온 이벤트가 우선, 확인되지 않은 `running`은 버림). 이벤트가 없으면 manifest를 따릅니다: `pending`·`running`이면 준비 중, `failed`면 오류, 모두 `ready`면 남은 `running` 표시보다 우선하고 원본 트랙으로 열려 있으면 스템으로 다시 엽니다.
- 플레이어는 곡을 열 때 트랙 파일을 열어 둡니다. 열린 곡의 `audio.wav`는 바꾸지 말고, 스템은 새 이름으로 쓴 뒤 이름을 바꿉니다.

### 플레이어 (`StemPlayer`)

- `StemPlayer::new(tracks, start)` → `StemSource`(엔진 `Source`) + `Feeder`(일반 스레드, `FeederThread`) + `PlayerShared`. 트랙은 최대 `MAX_TRACKS`(8)개, 모두 같은 샘플레이트입니다.
- 제어는 엔진 예약 이벤트로만 보냅니다: `Start`, `Stop`, `Seek { frame }`(곡 프레임), `Param`(`stems::param`: `SPEED` 0.5~1.0, 트랙별 `VOLUME`/`MUTE`/`SOLO`), A-B 반복은 `loop_events(range)`. 마스터 볼륨은 소스 게인(`set_gain`)입니다.
- A-B 반복은 `MIN_LOOP_FRAMES`(256프레임) 이상이어야 합니다. 더 짧으면 플레이어가 무시합니다(반복 없음). 화면은 0.1초 미만 선택을 반복으로 만들지 않습니다.
- 재생 명령(`stems_play`·`pause`·`seek`·`set_speed`·`set_loop`·`set_track`·`restore`)은 `{ ackFrame }`을 돌려주고, `stems_transport`는 `renderedFrame`을 함께 돌려줍니다. **적용 판정은 한 가지입니다: `renderedFrame >= ackFrame`이면 그 명령이 반영된 상태입니다.** 엔진은 `[start, end)` 블록에서 `frame < end`인 이벤트만 실행한 뒤 `end`를 게시하므로, frame F에 예약한 명령의 `ackFrame`은 F+1입니다(Rust `stems::ack_frame`, TS `reflects`). `renderedFrame == F`는 아직 실행 전입니다. 화면은 이 판정을 통과하지 못한 transport를 무시하고, 명령은 하나의 큐로 차례대로 보냅니다.
- 출력 레이트가 바뀌면 `Source::prepare`에서 같은 위치로 다시 시작합니다. 화면은 `audio://output-changed`를 받으면 `stems_restore(intent)`를 보냅니다. 이 명령은 한 번의 엔진 잠금 안에서 대기 이벤트를 취소하고, 화면의 의도 전체(아직 적용되지 않은 탐색, 속도, 반복, 트랙 믹스, 마스터, 재생 또는 정지)를 다시 예약합니다.

### Sidecar (yt-dlp, ffmpeg)

- `src-tauri/binaries/{yt-dlp,ffmpeg}-<target triple>`은 git에 넣지 않습니다. `mise run binaries`(= `pnpm binaries`, `scripts/fetch-binaries.sh`)가 yt-dlp를 받고 ffmpeg(오디오 전용 LGPL 구성)를 빌드합니다. 버전·체크섬은 스크립트에 고정되어 있고, 결과는 `~/Library/Caches/dev.mongmeo.woodshed/binaries`에 캐시됩니다. `pnpm tauri build`는 `beforeBuildCommand`에서 이를 실행합니다.
- 바이너리가 없으면 `src-tauri/build.rs`가 빈 자리표시 파일을 만들어 check·테스트·dev 빌드가 네트워크 없이 통과합니다. 이때 앱은 YouTube 가져오기에서 `toolsMissing` 오류만 보입니다.
- 실행 파일 옆의 sidecar는 `stems::Tools::locate()`로 찾습니다. stem-engine이 ffmpeg를 쓰면 같은 방법으로 찾습니다. 라이선스 고지는 `LICENSES/`에 있고 앱 번들 `Resources/LICENSES/`에 들어갑니다.

### 분리 모델 (stem-engine, `src-tauri/src/separation/`)

- `src-tauri/models/htdemucs_6s_fp32.mlmodelc`는 git에 넣지 않습니다. `mise run stem-model`(= `pnpm stem-model`, `tools/stem-model/build.sh`)이 HF 고정 리비전 가중치를 sha256으로 확인해 받고, 고정한 uv·Python·잠금 파일로 변환·컴파일해 `~/Library/Caches/dev.mongmeo.woodshed/models`에 캐시합니다. `pnpm tauri build`는 `beforeBuildCommand`에서 실행합니다.
- 모델이 없으면 `build.rs`가 빈 디렉터리를 만들어 check·테스트가 네트워크 없이 통과하고, Core ML 실측 테스트는 건너뛴 사실을 출력합니다. 앱은 번들 리소스, 그다음 위 캐시에서 모델을 찾고, 없으면 분리가 `failed`로 끝납니다.
- Core ML은 `CPUAndGPU`로만 돌립니다. GPU가 가끔 틀린 결과를 내므로 `VerifiedModel`이 같은 청크를 두 번 돌려 일치할 때만 받아들이고, 4번 안에 일치하지 않으면 그 청크를 CPU로 돌립니다. 모델·Core ML 객체는 작업 스레드 하나에서만 만들고 씁니다.
- 모델 밖 DSP를 바꾸면 `tools/stem-model/make_dsp_fixture.py`로 기준값을 다시 만들고, 모델·변환을 바꾸면 `ref_torch.py`·`compare.py`(예제 `separate_wav`)로 PyTorch와 다시 비교합니다.
