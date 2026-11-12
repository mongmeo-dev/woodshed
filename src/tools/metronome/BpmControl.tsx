import { type KeyboardEvent, type PointerEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatBpm, MAX_BPM, MIN_BPM, parseBpm, roundBpm } from "./model";

/** Pixels of vertical drag per step (Kinetics "Value Scrubber", turned upright). */
const DRAG_PX_PER_STEP = 3;
/** Movement below this is a click, which opens direct entry. */
const CLICK_SLOP_PX = 3;

interface BpmControlProps {
  /** Value shown (the trainer's current tempo while it plays). */
  display: number;
  /** Value edited. */
  value: number;
  onChange: (bpm: number) => void;
}

/**
 * The large tempo readout. A spinbutton: arrows ±1 (Shift or Page Up/Down
 * ±10, Option ±0.1), Home/End for the limits, vertical drag, and a click or
 * Enter to type a value.
 */
export function BpmControl({ display, value, onChange }: BpmControlProps) {
  const { t, i18n } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState(false);
  const drag = useRef<{ y: number; start: number; moved: boolean } | null>(null);
  const spin = useRef<HTMLDivElement>(null);
  // Set once Enter or Escape has closed the field, so the blur that follows
  // does not commit (or undo) again.
  const closed = useRef(false);

  const set = (bpm: number) => {
    const next = roundBpm(bpm);
    if (next !== value) onChange(next);
  };

  const beginEdit = () => {
    setDraft(formatBpm(value, "en"));
    setInvalid(false);
    closed.current = false;
    setEditing(true);
  };

  const endEdit = (commit: boolean) => {
    if (commit) {
      const parsed = parseBpm(draft);
      if (parsed === null) {
        setInvalid(true);
        return;
      }
      set(parsed);
    }
    closed.current = true;
    setEditing(false);
    setInvalid(false);
    requestAnimationFrame(() => spin.current?.focus());
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.altKey ? 0.1 : event.shiftKey ? 10 : 1;
    switch (event.key) {
      case "ArrowUp":
      case "ArrowRight":
        set(value + step);
        break;
      case "ArrowDown":
      case "ArrowLeft":
        set(value - step);
        break;
      case "PageUp":
        set(value + 10);
        break;
      case "PageDown":
        set(value - 10);
        break;
      case "Home":
        set(MIN_BPM);
        break;
      case "End":
        set(MAX_BPM);
        break;
      case "Enter":
        beginEdit();
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { y: event.clientY, start: value, moved: false };
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state) return;
    const distance = state.y - event.clientY;
    if (Math.abs(distance) >= CLICK_SLOP_PX) state.moved = true;
    if (!state.moved) return;
    const steps = Math.trunc(distance / DRAG_PX_PER_STEP);
    set(state.start + steps * (event.altKey ? 0.1 : 1));
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (state && !state.moved) beginEdit();
  };

  if (editing) {
    return (
      <div className="bpm">
        <input
          className="bpm-input"
          aria-label={t("metronome.tempo")}
          aria-invalid={invalid}
          aria-describedby={invalid ? "bpm-error" : undefined}
          inputMode="decimal"
          // biome-ignore lint/a11y/noAutofocus: direct entry starts where the user clicked.
          autoFocus
          value={draft}
          onFocus={(event) => event.currentTarget.select()}
          onChange={(event) => {
            setDraft(event.target.value);
            setInvalid(false);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") endEdit(true);
            if (event.key === "Escape") endEdit(false);
          }}
          onBlur={() => {
            if (closed.current) return;
            endEdit(parseBpm(draft) !== null);
          }}
        />
        <span className="bpm-unit">{t("metronome.bpm")}</span>
        {invalid && (
          <span className="row-error" id="bpm-error" role="alert">
            {t("metronome.bpmError", { min: MIN_BPM, max: MAX_BPM })}
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="bpm">
      <div
        ref={spin}
        className="bpm-value"
        role="spinbutton"
        tabIndex={0}
        aria-label={t("metronome.tempo")}
        aria-valuemin={MIN_BPM}
        aria-valuemax={MAX_BPM}
        aria-valuenow={display}
        aria-valuetext={t("metronome.bpmValue", { value: formatBpm(display, i18n.language) })}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => {
          drag.current = null;
        }}
      >
        {formatBpm(display, i18n.language)}
      </div>
      <span className="bpm-unit">{t("metronome.bpm")}</span>
    </div>
  );
}
