import {
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { LoopRange } from "../../lib/stems";
import { coveredTicks, labelHalfWidth, rulerTicks } from "./timeline";

/** Runs `frame` on every animation frame while `active`. */
export function useAnimationFrame(active: boolean, frame: () => void) {
  const callback = useRef(frame);
  useLayoutEffect(() => {
    callback.current = frame;
  });
  useEffect(() => {
    if (!active) return;
    let id = requestAnimationFrame(function tick() {
      callback.current();
      id = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(id);
  }, [active]);
}

const BAR_WIDTH = 2;
const BAR_GAP = 1;
/** Pointer travel that turns a click into a loop selection. */
const DRAG_THRESHOLD_PX = 4;
/** Arrow keys on the playback position. */
const SEEK_STEP_SECS = 5;
/** Room a ruler label keeps around its text (padding, border) and from marks beside it. */
const RULER_LABEL_PADDING_PX = 6;
const RULER_LABEL_GAP_PX = 4;

interface WaveformLabels {
  position: string;
}

interface WaveformProps {
  peaks: number[];
  duration: number;
  loop: LoopRange | null;
  playing: boolean;
  /** Changes whenever the position moves outside playback (seek, poll). */
  positionVersion: number;
  positionNow: () => number;
  onSeek: (seconds: number) => void;
  onLoop: (range: LoopRange) => void;
  labels: WaveformLabels;
  /** The playback position and ruler marks. */
  timeText: (seconds: number) => string;
  /** The time under the pointer and the loop being selected. */
  preciseTimeText: (seconds: number) => string;
  rangeText: (range: LoopRange) => string;
}

function token(element: Element, name: string) {
  return getComputedStyle(element).getPropertyValue(name).trim();
}

export function Waveform({
  peaks,
  duration,
  loop,
  playing,
  positionVersion,
  positionNow,
  onSeek,
  onLoop,
  labels,
  timeText,
  preciseTimeText,
  rangeText,
}: WaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const drag = useRef<{ x: number; seconds: number } | null>(null);
  const [draft, setDraft] = useState<LoopRange | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [width, setWidth] = useState(0);
  const [shownPosition, setShownPosition] = useState(0);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;
    const ratio = window.devicePixelRatio || 1;
    const { width, height } = canvas.getBoundingClientRect();
    if (width === 0 || height === 0) return;
    const pixelWidth = Math.round(width * ratio);
    const pixelHeight = Math.round(height * ratio);
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);

    const position = positionNow();
    const playX = duration > 0 ? (position / duration) * width : 0;
    const rest = token(canvas, "--ws-color-chip");
    const played = token(canvas, "--ws-color-text-tertiary");
    const step = BAR_WIDTH + BAR_GAP;
    const bars = Math.max(1, Math.floor(width / step));
    for (let bar = 0; bar < bars; bar++) {
      const from = Math.floor((bar / bars) * peaks.length);
      const to = Math.max(from + 1, Math.floor(((bar + 1) / bars) * peaks.length));
      let peak = 0;
      for (let i = from; i < to && i < peaks.length; i++) peak = Math.max(peak, peaks[i] ?? 0);
      const barHeight = Math.max(1, (peak / 255) * height);
      const x = bar * step;
      context.fillStyle = x + BAR_WIDTH <= playX ? played : rest;
      context.fillRect(x, (height - barHeight) / 2, BAR_WIDTH, barHeight);
    }
    context.fillStyle = token(canvas, "--ws-color-accent");
    context.fillRect(Math.min(Math.max(playX - 1, 0), width - 2), 0, 2, height);
  }, [peaks, duration, positionNow]);

  /** The drawn width, for the ruler marks. */
  const measure = useCallback(() => {
    const next = canvasRef.current?.getBoundingClientRect().width ?? 0;
    setWidth((current) => (Math.abs(current - next) < 0.5 ? current : next));
  }, []);

  useAnimationFrame(playing, draw);

  // The first ruler is laid out before the first paint.
  useLayoutEffect(measure, [measure]);

  // A new position (seek, transport poll) is drawn even while paused.
  // biome-ignore lint/correctness/useExhaustiveDependencies: positionVersion is the redraw trigger.
  useEffect(() => {
    draw();
    setShownPosition(positionNow());
  }, [draw, positionNow, positionVersion]);

  // While paused: redraw on size and theme changes.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            draw();
            measure();
          });
    observer?.observe(canvas);
    const themes = new MutationObserver(draw);
    themes.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const scheme = window.matchMedia?.("(prefers-color-scheme: dark)");
    scheme?.addEventListener("change", draw);
    return () => {
      observer?.disconnect();
      themes.disconnect();
      scheme?.removeEventListener("change", draw);
    };
  }, [draw, measure]);

  // Keep the accessible value roughly current without re-rendering per frame.
  useEffect(() => {
    if (!playing) {
      setShownPosition(positionNow());
      return;
    }
    const timer = window.setInterval(() => setShownPosition(positionNow()), 1000);
    return () => window.clearInterval(timer);
  }, [playing, positionNow]);

  const secondsAt = (clientX: number) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 0;
    const ratio = Math.min(Math.max((clientX - rect.left) / rect.width, 0), 1);
    return ratio * duration;
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    drag.current = { x: event.clientX, seconds: secondsAt(event.clientX) };
    setDraft(null);
    setHover(null);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const seconds = secondsAt(event.clientX);
    const start = drag.current;
    if (!start) {
      setHover(seconds);
      return;
    }
    if (Math.abs(event.clientX - start.x) < DRAG_THRESHOLD_PX) return;
    setDraft({ start: Math.min(start.seconds, seconds), end: Math.max(start.seconds, seconds) });
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    drag.current = null;
    if (!start) return;
    if (draft) {
      onLoop(draft);
      setDraft(null);
    } else {
      onSeek(secondsAt(event.clientX));
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const now = positionNow();
    const target =
      event.key === "ArrowLeft"
        ? now - SEEK_STEP_SECS
        : event.key === "ArrowRight"
          ? now + SEEK_STEP_SECS
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? duration
              : null;
    if (target === null) return;
    event.preventDefault();
    onSeek(Math.min(Math.max(target, 0), duration));
  };

  const region = draft ?? loop;
  const percent = (seconds: number) => (duration > 0 ? (seconds / duration) * 100 : 0);
  const ticks = rulerTicks(duration, width);
  const tickLabels = ticks.map((seconds) => timeText(seconds));

  // One label on the ruler: the range while dragging, else the pointer time.
  // It is centred on its time but kept inside the ruler, and hides the marks under it.
  const rulerLabel = draft
    ? { text: rangeText(draft), seconds: (draft.start + draft.end) / 2, range: true }
    : hover !== null
      ? { text: preciseTimeText(hover), seconds: hover, range: false }
      : null;
  const labelHalf = rulerLabel ? labelHalfWidth(rulerLabel.text, RULER_LABEL_PADDING_PX) : 0;
  const labelStyle: CSSProperties | undefined = rulerLabel
    ? {
        left: `clamp(${labelHalf}px, ${percent(rulerLabel.seconds)}%, calc(100% - ${labelHalf}px))`,
      }
    : undefined;
  const covered = rulerLabel
    ? coveredTicks(
        ticks,
        tickLabels,
        Math.min(
          Math.max((percent(rulerLabel.seconds) / 100) * width, labelHalf),
          width - labelHalf,
        ),
        labelHalf + RULER_LABEL_GAP_PX,
        duration,
        width,
      )
    : null;

  return (
    <div
      className="waveform"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => {
        drag.current = null;
        setDraft(null);
      }}
      onPointerLeave={() => {
        if (!drag.current) setHover(null);
      }}
    >
      <div className="waveform-view">
        <div
          className="waveform-track"
          role="slider"
          tabIndex={0}
          aria-label={labels.position}
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(shownPosition)}
          aria-valuetext={timeText(shownPosition)}
          onKeyDown={onKeyDown}
        >
          <canvas ref={canvasRef} className="waveform-canvas" />
        </div>
      </div>
      <div className="waveform-ruler" aria-hidden="true">
        {ticks.map((seconds, index) => (
          <span
            key={seconds}
            className="ruler-tick"
            data-first={seconds === 0 || undefined}
            data-covered={covered?.has(seconds) || undefined}
            style={{ left: `${percent(seconds)}%` }}
          >
            {tickLabels[index]}
          </span>
        ))}
        {rulerLabel && (
          <span
            className="ruler-time"
            data-range={rulerLabel.range || undefined}
            style={labelStyle}
          >
            {rulerLabel.text}
          </span>
        )}
      </div>
      {region && (
        <div
          className="waveform-loop"
          data-draft={draft !== null}
          aria-hidden="true"
          style={{
            left: `${percent(region.start)}%`,
            width: `${percent(region.end - region.start)}%`,
          }}
        />
      )}
      {hover !== null && !draft && (
        <div className="waveform-hover" aria-hidden="true" style={{ left: `${percent(hover)}%` }} />
      )}
    </div>
  );
}
