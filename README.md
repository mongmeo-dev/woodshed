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

## 미결 사항

- 마이크 권한 문구(`src-tauri/Info.plist`)는 영어 한 가지입니다. 현지화는 튜너 작업에서 정합니다.
- 출력이 바뀌어 샘플레이트가 달라지면 이미 예약된 이벤트의 프레임 위치는 그대로라 시간상 위치가 달라집니다. 메트로놈·드럼·스템 플레이어는 `audio://output-changed`를 받아 다시 예약해야 합니다.
- 출력 버퍼 하한은 64프레임입니다. 기기가 더 작은 버퍼를 허용해도 기기별 드롭아웃을 피하려고 쓰지 않습니다.
