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

- `handle.add_source(Box::new(src), gain) -> SourceId` — `prepare` 후 믹서에 붙음. 소스별 게인(선형, 약 5ms 글라이드)은 `set_gain`, 마스터는 `set_master_gain`.
- `handle.schedule(frame, id, kind)` — 절대 샘플 프레임(`FramePos`, 엔진 생성 후 누적, 장치를 바꿔도 리셋 안 됨)에 이벤트 예약. 그 프레임 이전에 큐에 들어오면 0프레임 오차로 렌더됩니다. 이미 렌더된 프레임이면 다음 프레임에 렌더되고 `stats().late_events`가 늘어납니다.
- `handle.frame_after(lead_secs)` — 지금 렌더 위치 + 여유. 실시간 예약은 최소 한 콜백(64프레임 ≈ 1.3ms) 이상 앞서, 보통 20~50ms 앞서 넣습니다.
- `handle.schedule_notify(...)` — 렌더 후 `RenderedEvent`를 보고합니다(화면 동기용).
- `handle.cancel(id, from_frame)` — 그 프레임 이후 대기 이벤트 제거(정지·템포 변경 시).
- `EventKind`: `Trigger { note, velocity }`, `Start`, `Stop`, `Seek { frame }`, `Param { id, value }`.
- 용량: 소스 64, 대기 이벤트 4096, 명령 큐 4096(`EngineConfig`). 넘치면 버리고 `stats()`의 `dropped_*`가 늘어납니다.

### 앱에서 엔진 쓰기

- Tauri 상태 `AudioService`(`src-tauri/src/app_audio.rs`)가 엔진을 가집니다. `audio.engine().lock()`으로 `AudioEngine`을 얻어 `handle()`을 씁니다. 락은 짧게 잡습니다(오디오 스레드는 이 락을 쓰지 않습니다).
- 도구별 Tauri 명령은 도구 모듈에 두고 `lib.rs`의 `generate_handler!`에 추가합니다.

### 화면 동기 ("이 소리가 실제로 들리는 시각")

- 오디오 콜백이 콜백 시작 프레임과 그 프레임의 DAC 도달 시각(스트림 클럭, 출력 레이턴시 포함)을 seqlock으로 게시합니다.
- `AudioEngine::heard_at_epoch_ms(frame)` — 프레임이 들리는 Unix epoch ms. 명령 `audio_heard_at`.
- `notify` 이벤트는 `audio_subscribe(onEvent: Channel<PlaybackEvent>)`로 등록한 채널에 `{ sourceId, scheduledFrame, renderedFrame, kind, heardAtMs }`로 옵니다. 프런트는 `src/lib/audio.ts`의 `onPlaybackEvent`로 받고, `delayUntilHeard(heardAtMs)`(= `heardAtMs - performance.timeOrigin - performance.now()`) 뒤에 그립니다. 예제: 설정 화면 테스트음(`useHeardBeat`).
- 미리 아는 프레임(메트로놈 박 등)은 예약 시점에 `audio_heard_at`으로 시각을 받아 미리 애니메이션을 걸 수 있습니다.

### 입력 (튜너)

- `list_devices(Direction::Input)` / 명령 `audio_list_devices({ direction: "input" })`로 장치·채널 수 열거.
- `open_input(&InputRequest { device_id, channel: InputChannel::Single(0) | MixDown, target_buffer_frames: 64, ring_frames })` → `InputCapture { samples: rtrb::Consumer<f32>, info, shared }`. 모노 f32 샘플을 일반 스레드에서 읽습니다. `shared.overflows()`, `capture_latency_ns()`, `take_error()` 제공. `InputCapture`를 drop하면 멈춥니다.
- macOS 마이크 권한 문구는 `src-tauri/Info.plist`에 있습니다.

### 장치 변경

- `spawn_device_monitor(interval, hook)`가 장치 목록을 폴링해 바뀌면 `hook(&DeviceSnapshot)`을 부릅니다. 앱은 `audio://devices-changed` 이벤트를 프런트로 내보내고, 현재 출력이 사라졌거나 스트림 오류가 나면 저장된 장치(실패 시 기본 장치)로 다시 엽니다. 입력을 쓰는 노드는 이 이벤트를 받아 입력을 다시 엽니다.

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
