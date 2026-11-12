import {
  type FormEvent,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { SegmentedControl } from "../../shell/SegmentedControl";
import {
  deletePattern,
  emptyPattern,
  LENGTHS,
  loadDrumState,
  MAX_BPM,
  MAX_SWING,
  MIN_BPM,
  MIN_SWING,
  normalizePattern,
  type Pattern,
  type PatternLength,
  type PatternStoreErrorCode,
  paintStep,
  type SavedPattern,
  type StepLevel,
  type SwingUnit,
  savePattern,
  sendPattern,
  setBpm,
  setDrumsPlaying,
  setLength,
  setSwing,
  setSwingUnit,
  setTrackVolume,
  storeErrorCode,
  TRACKS,
  type TrackId,
  toggleMute,
} from "./drums";
import { useStepPlayhead } from "./playhead";
import { PRESET_IDS, type PresetId, presetPattern } from "./presets";
import "./drumMachine.css";

type PaintLevel = Exclude<StepLevel, 0>;

type Dialog =
  | { kind: "save"; name: string; error: PatternStoreErrorCode | null }
  | { kind: "delete"; name: string; failed: boolean }
  | null;

/** Space plays and stops unless the user is typing or picking from a list. */
function takesSpace(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  return target instanceof HTMLInputElement && (target.type === "text" || target.type === "number");
}

function isBpm(text: string): boolean {
  if (!/^\d+$/.test(text)) return false;
  const value = Number(text);
  return value >= MIN_BPM && value <= MAX_BPM;
}

function SpeakerIcon({ muted }: { muted: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" />
      {muted ? <path d="M16 9.5l5 5M21 9.5l-5 5" /> : <path d="M16 9a4.5 4.5 0 0 1 0 6" />}
    </svg>
  );
}

/** Beat numbers of a pattern (four steps each). */
function beatsOf(length: PatternLength): number[] {
  return Array.from({ length: length / 4 }, (_, beat) => beat);
}

interface TrackRowProps {
  track: TrackId;
  steps: readonly StepLevel[];
  length: PatternLength;
  volume: number;
  muted: boolean;
  playhead: number | null;
  onPaint: (track: TrackId, step: number) => void;
  onMute: (track: TrackId) => void;
  onVolume: (track: TrackId, volume: number) => void;
}

const TrackRow = memo(function TrackRow({
  track,
  steps,
  length,
  volume,
  muted,
  playhead,
  onPaint,
  onMute,
  onVolume,
}: TrackRowProps) {
  const { t } = useTranslation();
  const name = t(`drumMachine.tracks.${track}`);
  const beats = beatsOf(length);
  return (
    <fieldset className="drum-row" data-muted={muted || undefined}>
      <legend className="visually-hidden">{name}</legend>
      <span className="drum-track-name" aria-hidden="true">
        {name}
      </span>
      <button
        type="button"
        className="drum-mute"
        aria-pressed={muted}
        aria-label={t("drumMachine.mute", { track: name })}
        onClick={() => onMute(track)}
      >
        <SpeakerIcon muted={muted} />
      </button>
      <input
        type="range"
        className="drum-volume"
        min={0}
        max={1}
        step={0.01}
        value={volume}
        aria-label={t("drumMachine.volume", { track: name })}
        onChange={(event) => onVolume(track, Number(event.target.value))}
      />
      <div className="drum-steps">
        {beats.map((beat) => (
          <div key={beat} className="drum-beat">
            {[0, 1, 2, 3].map((offset) => {
              const step = beat * 4 + offset;
              const level = steps[step] ?? 0;
              return (
                <button
                  key={step}
                  type="button"
                  className="drum-step"
                  data-level={level}
                  data-playhead={step === playhead || undefined}
                  aria-pressed={level > 0}
                  aria-label={t("drumMachine.step", { track: name, step: step + 1 })}
                  onClick={() => onPaint(track, step)}
                >
                  <span className="drum-step-fill" aria-hidden="true" />
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </fieldset>
  );
});

export function DrumMachineScreen() {
  const { t } = useTranslation();
  const [pattern, setPatternState] = useState<Pattern | null>(null);
  const patternRef = useRef<Pattern | null>(null);
  const [playing, setPlaying] = useState(false);
  const playingRef = useRef(false);
  const [saved, setSaved] = useState<SavedPattern[]>([]);
  const [paintLevel, setPaintLevel] = useState<PaintLevel>(2);
  const [presetId, setPresetId] = useState<PresetId | "">("");
  const [savedName, setSavedName] = useState("");
  const [bpmDraft, setBpmDraft] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const playhead = useStepPlayhead(playing);

  useEffect(() => {
    let alive = true;
    loadDrumState().then(
      (state) => {
        if (!alive) return;
        patternRef.current = state.pattern;
        playingRef.current = state.playing;
        setPatternState(state.pattern);
        setPlaying(state.playing);
        setSaved(state.saved);
      },
      () => {
        if (!alive) return;
        patternRef.current = emptyPattern();
        setPatternState(patternRef.current);
      },
    );
    return () => {
      alive = false;
    };
  }, []);

  /** Applies an edit on screen and sends it to the engine (heard while playing). */
  const edit = useCallback((change: (pattern: Pattern) => Pattern) => {
    const current = patternRef.current;
    if (!current) return;
    const next = change(current);
    if (next === current) return;
    patternRef.current = next;
    setPatternState(next);
    sendPattern(next).catch(() => {});
  }, []);

  const togglePlaying = useCallback(() => {
    const next = !playingRef.current;
    playingRef.current = next;
    setPlaying(next);
    setDrumsPlaying(next).catch(() => {
      playingRef.current = !next;
      setPlaying(!next);
    });
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code !== "Space" || event.metaKey || event.ctrlKey || event.altKey) return;
      if (takesSpace(event.target)) return;
      event.preventDefault();
      if (!event.repeat) togglePlaying();
    };
    // Keeps a focused button from also activating on release.
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code === "Space" && !takesSpace(event.target)) event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [togglePlaying]);

  const onPaint = useCallback(
    (track: TrackId, step: number) =>
      edit((current) => paintStep(current, track, step, paintLevel)),
    [edit, paintLevel],
  );
  const onMute = useCallback(
    (track: TrackId) => edit((current) => toggleMute(current, track)),
    [edit],
  );
  const onVolume = useCallback(
    (track: TrackId, volume: number) => edit((current) => setTrackVolume(current, track, volume)),
    [edit],
  );

  if (!pattern) {
    return <ScreenFrame title={t("tools.drumMachine")} />;
  }

  const bpmText = bpmDraft ?? String(pattern.bpm);
  const bpmInvalid = bpmDraft !== null && !isBpm(bpmDraft);
  const existing = dialog?.kind === "save" && saved.some((s) => s.name === dialog.name.trim());

  const loadPreset = (id: PresetId) => {
    setPresetId(id);
    setSavedName("");
    setBpmDraft(null);
    edit(() => presetPattern(id));
  };

  const loadSaved = (name: string) => {
    const entry = saved.find((s) => s.name === name);
    if (!entry) return;
    setSavedName(name);
    setPresetId("");
    setBpmDraft(null);
    edit(() => normalizePattern(entry.pattern));
  };

  const submitSave = async (event: FormEvent) => {
    event.preventDefault();
    if (dialog?.kind !== "save") return;
    const name = dialog.name.trim();
    if (!name) {
      setDialog({ ...dialog, error: "emptyName" });
      return;
    }
    try {
      setSaved(await savePattern(name, pattern));
      setSavedName(name);
      setDialog(null);
    } catch (error) {
      setDialog({ ...dialog, error: storeErrorCode(error) });
    }
  };

  const confirmDelete = async () => {
    if (dialog?.kind !== "delete") return;
    try {
      setSaved(await deletePattern(dialog.name));
      setSavedName("");
      setDialog(null);
    } catch {
      setDialog({ ...dialog, failed: true });
    }
  };

  const lengthOptions = LENGTHS.map((length) => ({
    value: String(length) as `${PatternLength}`,
    label: t("drumMachine.steps", { value: length }),
  }));
  const unitOptions = [
    { value: "eighth", label: t("drumMachine.eighth") },
    { value: "sixteenth", label: t("drumMachine.sixteenth") },
  ] as const;
  const velocityOptions = [
    { value: "1", label: t("drumMachine.soft") },
    { value: "2", label: t("drumMachine.normal") },
    { value: "3", label: t("drumMachine.accent") },
  ] as const;

  return (
    <ScreenFrame title={t("tools.drumMachine")}>
      <div className="drum-machine">
        <div className="drum-toolbar">
          <button
            type="button"
            className="button-primary drum-play"
            aria-pressed={playing}
            onClick={togglePlaying}
          >
            {playing ? t("drumMachine.stop") : t("drumMachine.play")}
          </button>

          <div className="drum-field">
            <label className="drum-field-label" htmlFor="drum-bpm">
              {t("drumMachine.tempo")}
            </label>
            <input
              id="drum-bpm"
              className="drum-number"
              type="number"
              inputMode="numeric"
              min={MIN_BPM}
              max={MAX_BPM}
              value={bpmText}
              aria-invalid={bpmInvalid || undefined}
              aria-describedby={bpmInvalid ? "drum-bpm-error" : undefined}
              onChange={(event) => {
                const text = event.target.value;
                setBpmDraft(text);
                if (isBpm(text)) edit((current) => setBpm(current, Number(text)));
              }}
              onBlur={() => setBpmDraft(null)}
            />
            <input
              type="range"
              className="drum-slider"
              min={MIN_BPM}
              max={MAX_BPM}
              value={pattern.bpm}
              aria-label={t("drumMachine.tempo")}
              onChange={(event) => {
                setBpmDraft(null);
                edit((current) => setBpm(current, Number(event.target.value)));
              }}
            />
            {bpmInvalid && (
              <span className="drum-error" id="drum-bpm-error" role="alert">
                {t("drumMachine.bpmError")}
              </span>
            )}
          </div>

          <div className="drum-field">
            <label className="drum-field-label" htmlFor="drum-swing">
              {t("drumMachine.swing")}
            </label>
            <input
              id="drum-swing"
              type="range"
              className="drum-slider"
              min={MIN_SWING}
              max={MAX_SWING}
              value={pattern.swing}
              onChange={(event) => edit((current) => setSwing(current, Number(event.target.value)))}
            />
            <output className="drum-value" htmlFor="drum-swing">
              {t("drumMachine.swingValue", { value: pattern.swing })}
            </output>
            <SegmentedControl<SwingUnit>
              label={t("drumMachine.swingUnit")}
              options={unitOptions}
              value={pattern.swingUnit}
              onChange={(unit) => edit((current) => setSwingUnit(current, unit))}
            />
          </div>

          <div className="drum-field">
            <span className="drum-field-label" aria-hidden="true">
              {t("drumMachine.length")}
            </span>
            <SegmentedControl
              label={t("drumMachine.length")}
              options={lengthOptions}
              value={String(pattern.length) as `${PatternLength}`}
              onChange={(value) =>
                edit((current) => setLength(current, Number(value) as PatternLength))
              }
            />
          </div>
        </div>

        <div className="drum-toolbar">
          <div className="drum-field">
            <span className="drum-field-label" aria-hidden="true">
              {t("drumMachine.velocity")}
            </span>
            <SegmentedControl
              label={t("drumMachine.velocity")}
              options={velocityOptions}
              value={String(paintLevel) as "1" | "2" | "3"}
              onChange={(value) => setPaintLevel(Number(value) as PaintLevel)}
            />
          </div>

          <div className="drum-field">
            <label className="drum-field-label" htmlFor="drum-preset">
              {t("drumMachine.preset")}
            </label>
            <select
              id="drum-preset"
              className="select"
              value={presetId}
              onChange={(event) => loadPreset(event.target.value as PresetId)}
            >
              <option value="" disabled>
                {t("drumMachine.choosePreset")}
              </option>
              {PRESET_IDS.map((id) => (
                <option key={id} value={id}>
                  {t(`drumMachine.presets.${id}`)}
                </option>
              ))}
            </select>
          </div>

          <div className="drum-field">
            <label className="drum-field-label" htmlFor="drum-saved">
              {t("drumMachine.myPatterns")}
            </label>
            <select
              id="drum-saved"
              className="select"
              value={saved.some((s) => s.name === savedName) ? savedName : ""}
              disabled={saved.length === 0}
              onChange={(event) => loadSaved(event.target.value)}
            >
              <option value="" disabled>
                {saved.length === 0 ? t("drumMachine.noPatterns") : t("drumMachine.choosePattern")}
              </option>
              {saved.map((entry) => (
                <option key={entry.name} value={entry.name}>
                  {entry.name}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="drum-button"
              onClick={() => setDialog({ kind: "save", name: savedName, error: null })}
            >
              {t("drumMachine.save")}
            </button>
            <button
              type="button"
              className="drum-button"
              disabled={!saved.some((s) => s.name === savedName)}
              onClick={() => setDialog({ kind: "delete", name: savedName, failed: false })}
            >
              {t("drumMachine.delete")}
            </button>
          </div>
        </div>

        {dialog?.kind === "save" && (
          <form
            className="drum-dialog"
            onSubmit={(event) => void submitSave(event)}
            onKeyDown={(event) => event.key === "Escape" && setDialog(null)}
          >
            <label className="drum-field-label" htmlFor="drum-pattern-name">
              {t("drumMachine.patternName")}
            </label>
            <input
              id="drum-pattern-name"
              className="drum-text"
              type="text"
              maxLength={40}
              value={dialog.name}
              aria-invalid={dialog.error === "emptyName" || undefined}
              // biome-ignore lint/a11y/noAutofocus: the field opens on request, for typing right away.
              autoFocus
              onChange={(event) => setDialog({ ...dialog, name: event.target.value, error: null })}
            />
            <button type="submit" className="button-primary">
              {existing ? t("drumMachine.replace") : t("drumMachine.save")}
            </button>
            <button type="button" className="drum-button" onClick={() => setDialog(null)}>
              {t("drumMachine.cancel")}
            </button>
            {dialog.error && (
              <span className="drum-error" role="alert">
                {dialog.error === "emptyName"
                  ? t("drumMachine.emptyName")
                  : t("drumMachine.saveFailed")}
              </span>
            )}
          </form>
        )}

        {dialog?.kind === "delete" && (
          <div
            className="drum-dialog"
            role="alertdialog"
            aria-labelledby="drum-delete-title"
            onKeyDown={(event) => event.key === "Escape" && setDialog(null)}
          >
            <span id="drum-delete-title">
              {t("drumMachine.deleteConfirm", { name: dialog.name })}
            </span>
            <button type="button" className="button-primary" onClick={() => void confirmDelete()}>
              {t("drumMachine.delete")}
            </button>
            <button type="button" className="drum-button" onClick={() => setDialog(null)}>
              {t("drumMachine.cancel")}
            </button>
            {dialog.failed && (
              <span className="drum-error" role="alert">
                {t("drumMachine.deleteFailed")}
              </span>
            )}
          </div>
        )}

        <div className="drum-grid">
          <div className="drum-row drum-ruler" aria-hidden="true">
            <span />
            <span />
            <span />
            <div className="drum-steps">
              {beatsOf(pattern.length).map((beat) => (
                <div key={beat} className="drum-beat">
                  {[0, 1, 2, 3].map((offset) => {
                    const step = beat * 4 + offset;
                    return (
                      <span
                        key={step}
                        className="drum-tick"
                        data-playhead={step === playhead || undefined}
                      >
                        {offset === 0 ? (beat % 4) + 1 : ""}
                      </span>
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
          {TRACKS.map((track) => {
            const entry = pattern.tracks[track];
            return (
              <TrackRow
                key={track}
                track={track}
                steps={entry.steps}
                length={pattern.length}
                volume={entry.volume}
                muted={entry.muted}
                playhead={playhead}
                onPaint={onPaint}
                onMute={onMute}
                onVolume={onVolume}
              />
            );
          })}
        </div>
      </div>
    </ScreenFrame>
  );
}

function ScreenFrame({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <section className="screen" aria-labelledby="screen-title-drumMachine">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-drumMachine">
          {title}
        </h1>
      </header>
      <div className="screen-body">{children}</div>
    </section>
  );
}
