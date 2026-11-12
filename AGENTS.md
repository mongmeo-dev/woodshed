# Woodshed AI Agent Rule

베이스·기타 연습 도구 macOS 앱(Tauri 2 + React/TS + Rust)입니다. 루트 저장소(mongmeo-common)의 `AGENTS.md` 규칙(한국어 경어체, 한국어 커밋 메시지, 기능 단위 커밋, Co-Author 없음, `mise exec -- <명령>`, 명시 버전 의존성)을 그대로 따르고, 아래 규칙을 추가로 지킵니다.

## 검사 명령

- 모든 도구는 `mise.toml`에 고정되어 있습니다. 명령은 `mise exec -- <명령>` 또는 `mise run <태스크>`로 실행합니다.
- 커밋·push 전에 `mise run check`를 통과시킵니다. 내용: `biome check` → `tsc --noEmit` → `vitest run --maxWorkers=2` → `cargo fmt --check` → `cargo clippy --all-targets --locked -D warnings` → `cargo test --locked`.
  - 웹만: `mise run check:web`, Rust만: `mise run check:rust`.
- 이 Mac은 다른 프로젝트와 공유합니다. 무거운 명령(빌드·전체 검사)은 한 번에 하나만 돌리고, `CARGO_BUILD_JOBS=4`(mise env에 설정됨), vitest는 `--maxWorkers=2`를 씁니다. 개발 서버·빌드한 앱은 확인 후 종료합니다.
- 앱 빌드: `mise exec -- pnpm tauri build` → `src-tauri/target/release/bundle/macos/Woodshed.app`.
- 실제 출력 장치 레이턴시 확인: `cd src-tauri && mise exec -- cargo run --release --example latency_probe` (스피커로 짧은 클릭음이 납니다).

## 화면 문구 규칙

- 레이블, 현재 상태 값, 사용자가 지금 할 일, 입력 오류, 확인 대화상자의 결과 안내만 씁니다.
- 히어로 문구, 앱 소개 문장, 구현 상태나 정책을 설명하는 문장은 절대 넣지 않습니다. 도구 화면 제목 외 설명 문구도 넣지 않습니다.
- 모든 문구는 `src/locales/ko.json`과 `src/locales/en.json`에 같은 키로 넣습니다(키 불일치는 `src/i18n/i18n.test.ts`가 잡습니다). 컴포넌트에 문자열을 하드코딩하지 않습니다. 언어 이름(`한국어`, `English`)만 예외입니다.
- 한국어 문구는 `/humanize-korean:humanize-korean` 스킬로 번역투가 없는지 다듬고, 영어도 자연스러운 앱 문구(Title Case 레이블)로 씁니다.

## 디자인 규칙

- `DESIGN.md`(VoltAgent/awesome-design-md의 apple 분석본)가 루트 디자인 룰보다 우선합니다.
- 색·타이포·간격·라운드는 `src/styles/tokens.css`의 `--ws-*` 변수만 씁니다. 새 색 값을 컴포넌트에 직접 쓰지 않습니다. 라이트/다크 둘 다 확인합니다.
- 강조색은 하나(`--ws-color-accent`)뿐입니다. 카드·버튼·텍스트에 그림자를 넣지 않습니다. 굵기는 400/600만 씁니다. 버튼 눌림은 `transform: scale(0.95)`입니다.
- 라운드는 컴팩트 컨트롤 `--ws-radius-sm`, 카드 `--ws-radius-lg`, 액션 버튼 `--ws-radius-pill`만 씁니다.
- 모션은 `--ws-duration-*`/`--ws-ease-glide`를 쓰고, `prefers-reduced-motion`에서는 0으로 떨어집니다.

## 앱 구조

- 도구 목록: `src/tools/tools.ts`의 `TOOL_IDS`(`tuner`, `metronome`, `drumMachine`, `stemSplitter`, `fretboard`). 각 도구 노드는 `src/tools/<도구>/`에 화면을 만들고 `src/App.tsx`에서 `ToolScreen` 대신 연결합니다.
- 설정: `src-tauri/src/settings.rs`(`Settings`)와 `src/lib/settings.ts`가 짝입니다. 필드를 추가하면 둘 다 고치고, 누락 필드는 기본값으로 읽히게 합니다(`#[serde(default)]`).
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
  - 백오프는 장치 목록 변경·사용자 선택·선호 장치 성공 때만 초기화됩니다.
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
