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
import {
  coveredTicks,
  dragRange,
  type LoopEdge,
  type LoopGesture,
  labelHalfWidth,
  loopable,
  loopZoneAt,
  MIN_LOOP_SECS,
  moveEdge,
  rulerTicks,
  sameLoop,
  stepEdge,
} from "./timeline";

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
/** Pointer travel that turns a click into a drag. */
const DRAG_THRESHOLD_PX = 4;
/** Arrow keys on the playback position. */
const SEEK_STEP_SECS = 5;
/** Arrow keys on a loop edge, and with Shift. */
const EDGE_STEP_SECS = 0.1;
const EDGE_SHIFT_STEP_SECS = 1;
/** Room a ruler label keeps around its text (padding, border) and from marks beside it. */
const RULER_LABEL_PADDING_PX = 6;
const RULER_LABEL_GAP_PX = 4;
const EDGES: readonly LoopEdge[] = ["start", "end"];

interface WaveformLabels {
  position: string;
  loopStart: string;
  loopEnd: string;
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
  /** Loop edges and the time under the pointer. */
  preciseTimeText: (seconds: number) => string;
  rangeText: (range: LoopRange) => string;
}

interface Drag {
  gesture: LoopGesture;
  pointerId: number;
  x: number;
  moved: boolean;
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
  const drag = useRef<Drag | null>(null);
  /** An edge moved by a held arrow key, sent when the key is released. */
  const keyDraft = useRef<LoopRange | null>(null);
  const handles = useRef<Partial<Record<LoopEdge, HTMLDivElement | null>>>({});
  const [draft, setDraft] = useState<LoopRange | null>(null);
  const [dragKind, setDragKind] = useState<LoopGesture["kind"] | null>(null);
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

  /** What a press at `seconds` grabs on the loop, judged in pixels. */
  const zoneAt = (seconds: number) => {
    if (!loop || duration <= 0 || width <= 0) return null;
    const px = (value: number) => (value / duration) * width;
    return loopZoneAt(px(seconds), px(loop.start), px(loop.end));
  };

  const endDrag = () => {
    drag.current = null;
    setDraft(null);
    setDragKind(null);
  };

  // A drag on an edge moves that edge, inside the loop moves the loop, and
  // anywhere else selects a new loop. A click without a drag seeks.
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const at = secondsAt(event.clientX);
    const zone = zoneAt(at);
    let gesture: LoopGesture = { kind: "select", from: at };
    if (loop && (zone === "start" || zone === "end")) {
      gesture = { kind: "resize", edge: zone, from: at, origin: loop };
      // The grabbed edge takes the keys next (instead of the position).
      event.preventDefault();
      handles.current[zone]?.focus({ preventScroll: true });
    } else if (loop && zone === "inside") {
      gesture = { kind: "move", from: at, origin: loop };
    }
    event.currentTarget.setPointerCapture?.(event.pointerId);
    drag.current = { gesture, pointerId: event.pointerId, x: event.clientX, moved: false };
    setDraft(null);
    setDragKind(gesture.kind);
    setHover(null);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const at = secondsAt(event.clientX);
    const current = drag.current;
    if (!current) {
      setHover(at);
      return;
    }
    if (event.pointerId !== current.pointerId) return;
    // The release was missed (another app took the pointer): drop the drag.
    if ((event.buttons & 1) === 0) {
      endDrag();
      return;
    }
    if (!current.moved) {
      if (Math.abs(event.clientX - current.x) < DRAG_THRESHOLD_PX) return;
      current.moved = true;
    }
    const range = dragRange(current.gesture, at, duration);
    // A new loop shows once it is long enough to be one.
    setDraft(current.gesture.kind !== "select" || loopable(range) ? range : null);
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || event.pointerId !== current.pointerId) return;
    const at = secondsAt(event.clientX);
    endDrag();
    if (!current.moved) {
      if (current.gesture.kind !== "resize") onSeek(at);
      return;
    }
    const range = dragRange(current.gesture, at, duration);
    if (loopable(range) && !sameLoop(range, loop)) onLoop(range);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const now = positionNow();
    const target =
      event.key === "ArrowLeft" || event.key === "ArrowDown"
        ? now - SEEK_STEP_SECS
        : event.key === "ArrowRight" || event.key === "ArrowUp"
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

  const onEdgeKeyDown = (edge: LoopEdge, event: KeyboardEvent<HTMLDivElement>) => {
    if (!loop) return;
    const from = keyDraft.current ?? loop;
    const step = event.shiftKey ? EDGE_SHIFT_STEP_SECS : EDGE_STEP_SECS;
    let next: LoopRange;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowDown":
        next = stepEdge(from, edge, -step, duration);
        break;
      case "ArrowRight":
      case "ArrowUp":
        next = stepEdge(from, edge, step, duration);
        break;
      case "Home":
        next = moveEdge(from, edge, 0, duration);
        break;
      case "End":
        next = moveEdge(from, edge, duration, duration);
        break;
      default:
        return;
    }
    event.preventDefault();
    // Every loop change restarts playback with a short fade, so a held key
    // only shows the edge moving and sends where it stops.
    if (event.repeat) {
      keyDraft.current = next;
      setDraft(next);
      return;
    }
    keyDraft.current = null;
    setDraft(null);
    if (!sameLoop(next, loop)) onLoop(next);
  };

  const onEdgeKeyUp = () => {
    const next = keyDraft.current;
    if (!next) return;
    keyDraft.current = null;
    setDraft(null);
    if (loop && !sameLoop(next, loop)) onLoop(next);
  };

  const region = draft ?? loop;
  const percent = (seconds: number) => (duration > 0 ? (seconds / duration) * 100 : 0);
  const zone = hover !== null ? zoneAt(hover) : null;
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
      data-drag={dragKind ?? undefined}
      data-zone={zone ?? undefined}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={endDrag}
      onLostPointerCapture={() => {
        if (drag.current) endDrag();
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
      {region &&
        EDGES.map((edge) => {
          const [min, max] =
            edge === "start"
              ? [0, region.end - MIN_LOOP_SECS]
              : [region.start + MIN_LOOP_SECS, duration];
          return (
            <div
              key={edge}
              ref={(element) => {
                handles.current[edge] = element;
              }}
              className="loop-handle"
              data-edge={edge}
              role="slider"
              tabIndex={loop ? 0 : -1}
              aria-label={edge === "start" ? labels.loopStart : labels.loopEnd}
              aria-valuemin={Number(min.toFixed(1))}
              aria-valuemax={Number(max.toFixed(1))}
              aria-valuenow={Number(region[edge].toFixed(1))}
              aria-valuetext={preciseTimeText(region[edge])}
              style={{ left: `${percent(region[edge])}%` }}
              onKeyDown={(event) => onEdgeKeyDown(edge, event)}
              onKeyUp={onEdgeKeyUp}
              onBlur={onEdgeKeyUp}
            />
          );
        })}
    </div>
  );
}
