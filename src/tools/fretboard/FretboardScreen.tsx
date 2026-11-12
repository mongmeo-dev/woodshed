import { type CSSProperties, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  chromaticName,
  DOUBLE_MARKER_FRETS,
  displayName,
  INSTRUMENT_IDS,
  SCALE_GROUPS,
  SINGLE_MARKER_FRETS,
} from "../../lib/theory";
import { SegmentedControl } from "../../shell/SegmentedControl";
import { buildBoard } from "./board";
import "./fretboard.css";
import {
  type DisplayMode,
  FRET_COUNTS,
  type FretboardPrefs,
  readPrefs,
  writePrefs,
} from "./preferences";

const KEYS = Array.from({ length: 12 }, (_, pitchClass) => pitchClass);

function keyLabel(pitchClass: number): string {
  const sharp = chromaticName(pitchClass, "sharp");
  const flat = chromaticName(pitchClass, "flat");
  return displayName(sharp === flat ? sharp : `${sharp} / ${flat}`);
}

/** Thicker strings toward the bass; `index` counts from the top row. */
function stringWidth(index: number, count: number): string {
  const rank = count === 1 ? 0 : (count - 1 - index) / (count - 1);
  return `${1 + Math.round(rank * 2)}px`;
}

export function FretboardScreen() {
  const { t } = useTranslation();
  const [prefs, setPrefs] = useState<FretboardPrefs>(readPrefs);
  const set = (patch: Partial<FretboardPrefs>) => setPrefs((current) => ({ ...current, ...patch }));

  useEffect(() => {
    writePrefs(prefs);
  }, [prefs]);

  const board = useMemo(
    () => buildBoard(prefs.instrument, prefs.key, prefs.scale, prefs.frets, prefs.display),
    [prefs.instrument, prefs.key, prefs.scale, prefs.frets, prefs.display],
  );
  const frets = Array.from({ length: prefs.frets + 1 }, (_, fret) => fret);
  const markerLabel = (note: string, root: boolean) =>
    root ? `${displayName(note)} ${t("fretboard.root")}` : displayName(note);
  const rootName = displayName(board.scale.notes[0]?.name ?? "");
  const boardStyle = {
    "--frets": prefs.frets,
    "--strings": board.strings.length,
  } as CSSProperties;

  const instrumentOptions = INSTRUMENT_IDS.map((id) => ({
    value: id,
    label: t(`fretboard.instrument.${id}`),
  }));
  const displayOptions: { value: DisplayMode; label: string }[] = [
    { value: "notes", label: t("fretboard.display.notes") },
    { value: "intervals", label: t("fretboard.display.intervals") },
  ];
  const fretOptions = FRET_COUNTS.map((count) => ({ value: String(count), label: String(count) }));

  return (
    <section className="screen" aria-labelledby="screen-title-fretboard">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-fretboard">
          {t("tools.fretboard")}
        </h1>
      </header>
      <div className="screen-body">
        <div className="fretboard-page">
          <div className="fretboard-controls">
            <div className="field">
              <span className="field-label" aria-hidden="true">
                {t("fretboard.instrument.label")}
              </span>
              <SegmentedControl
                label={t("fretboard.instrument.label")}
                options={instrumentOptions}
                value={prefs.instrument}
                onChange={(instrument) => set({ instrument })}
              />
            </div>
            <label className="field">
              <span className="field-label">{t("fretboard.key")}</span>
              <select
                className="select"
                value={prefs.key}
                onChange={(event) => set({ key: Number(event.target.value) })}
              >
                {KEYS.map((pitchClass) => (
                  <option key={pitchClass} value={pitchClass}>
                    {keyLabel(pitchClass)}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">{t("fretboard.scale")}</span>
              <select
                className="select"
                value={prefs.scale}
                onChange={(event) => set({ scale: event.target.value as FretboardPrefs["scale"] })}
              >
                {SCALE_GROUPS.map((group) => (
                  <optgroup key={group.id} label={t(`fretboard.scaleGroups.${group.id}`)}>
                    {group.scales.map((id) => (
                      <option key={id} value={id}>
                        {t(`fretboard.scales.${id}`)}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </label>
            <div className="field">
              <span className="field-label" aria-hidden="true">
                {t("fretboard.display.label")}
              </span>
              <SegmentedControl
                label={t("fretboard.display.label")}
                options={displayOptions}
                value={prefs.display}
                onChange={(display) => set({ display })}
              />
            </div>
            <div className="field">
              <span className="field-label" aria-hidden="true">
                {t("fretboard.frets")}
              </span>
              <SegmentedControl
                label={t("fretboard.frets")}
                options={fretOptions}
                value={String(prefs.frets)}
                onChange={(value) => set({ frets: Number(value) as FretboardPrefs["frets"] })}
              />
            </div>
            <label className="field field--check">
              <input
                type="checkbox"
                checked={prefs.leftHanded}
                onChange={(event) => set({ leftHanded: event.target.checked })}
              />
              <span>{t("fretboard.leftHanded")}</span>
            </label>
          </div>

          <div className="card fretboard-card">
            <div className="fretboard-scroll">
              <figure
                className="fretboard"
                dir={prefs.leftHanded ? "rtl" : "ltr"}
                style={boardStyle}
                aria-label={t("fretboard.board")}
              >
                <div className="fretboard-inlays" aria-hidden="true">
                  <span />
                  {frets.map((fret) => (
                    <span className="inlay-cell" key={fret}>
                      {inlayCount(fret) >= 1 && <i className="inlay" />}
                      {inlayCount(fret) === 2 && <i className="inlay" />}
                    </span>
                  ))}
                </div>
                <div className="fretboard-strings">
                  {board.strings.map((string, row) => (
                    <div
                      className="string-row"
                      key={string.index}
                      style={
                        {
                          "--string-width": stringWidth(row, board.strings.length),
                        } as CSSProperties
                      }
                    >
                      <span className="string-name">{string.name}</span>
                      {string.markers.map((marker, fret) => (
                        // biome-ignore lint/suspicious/noArrayIndexKey: the fret number is the stable identity of a cell.
                        <span className="fret-cell" data-open={fret === 0} key={fret}>
                          {marker && (
                            <span
                              className="marker"
                              data-root={marker.root}
                              data-note={marker.note}
                              data-interval={marker.interval}
                              role="img"
                              aria-label={markerLabel(marker.note, marker.root)}
                            >
                              {displayName(marker.label)}
                            </span>
                          )}
                        </span>
                      ))}
                    </div>
                  ))}
                </div>
                <div className="fret-numbers" aria-hidden="true">
                  <span />
                  {frets.map((fret) => (
                    <span className="fret-number" key={fret}>
                      {fret}
                    </span>
                  ))}
                </div>
              </figure>
            </div>
          </div>

          <section className="scale-readout" aria-labelledby="fretboard-scale-title">
            <h2 className="scale-title" id="fretboard-scale-title">
              {rootName} {t(`fretboard.scales.${prefs.scale}`)}
            </h2>
            <ol className="scale-notes">
              {board.scale.notes.map((note) => (
                <li
                  className="scale-note"
                  key={note.interval}
                  data-root={note.pitchClass === board.scale.root}
                >
                  <span className="scale-note-name">{displayName(note.name)}</span>
                  <span className="scale-note-interval">{displayName(note.interval)}</span>
                </li>
              ))}
            </ol>
          </section>
        </div>
      </div>
    </section>
  );
}

function inlayCount(fret: number): 0 | 1 | 2 {
  if (DOUBLE_MARKER_FRETS.includes(fret)) return 2;
  return SINGLE_MARKER_FRETS.includes(fret) ? 1 : 0;
}
