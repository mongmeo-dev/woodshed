import { useCallback, useEffect, useRef, useState } from "react";
import {
  deletePreset as deletePresetCommand,
  loadMetronome,
  type MetronomeMessage,
  type Preset,
  type PresetErrorCode,
  presetErrorCode,
  savePreset as savePresetCommand,
  startMetronome,
  stopMetronome,
  subscribeMetronome,
  updateMetronome,
} from "./api";
import { type BeatMessage, BeatQueue, type PulseMessage } from "./beatQueue";
import { DEFAULT_SETTINGS, type MetronomeSettings } from "./model";

/** The latest beat and polyrhythm pulse that have become audible. */
export interface Heard {
  beat: BeatMessage | null;
  pulse: PulseMessage | null;
}

const NOTHING_HEARD: Heard = { beat: null, pulse: null };

/**
 * Drops what is shown from `fromFrame` on: those beats will not play (an
 * output switch resets from 0). Earlier ones stay lit.
 */
export function retractHeard(heard: Heard, fromFrame: number): Heard {
  const beat = heard.beat && heard.beat.frame >= fromFrame ? null : heard.beat;
  const pulse = heard.pulse && heard.pulse.frame >= fromFrame ? null : heard.pulse;
  return beat === heard.beat && pulse === heard.pulse ? heard : { beat, pulse };
}

/** Settings, presets and playback, with beats lit when they are heard. */
export function useMetronome() {
  const [settings, setSettings] = useState<MetronomeSettings>(DEFAULT_SETTINGS);
  const [presets, setPresets] = useState<Preset[]>([]);
  const [playing, setPlaying] = useState(false);
  const [heard, setHeard] = useState<Heard>(NOTHING_HEARD);
  const queue = useRef(new BeatQueue());
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  useEffect(() => {
    let active = true;
    const onMessage = (message: MetronomeMessage) => {
      if (!active) return;
      switch (message.type) {
        case "beat":
        case "pulse":
          queue.current.push(message);
          break;
        case "reset":
          queue.current.reset(message.fromFrame);
          setHeard((previous) => retractHeard(previous, message.fromFrame));
          break;
        case "stopped":
          queue.current.clear();
          setPlaying(false);
          setHeard(NOTHING_HEARD);
          break;
      }
    };
    subscribeMetronome(onMessage).catch(() => {});
    loadMetronome().then(
      (state) => {
        if (!active) return;
        setSettings(state.settings);
        setPresets(state.presets);
        setPlaying(state.playing);
      },
      () => {},
    );
    return () => {
      active = false;
    };
  }, []);

  // Light beats on the frame they become audible.
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const step = () => {
      const due = queue.current.take(performance.now());
      if (due.beat || due.pulse) {
        setHeard((previous) => ({
          beat: due.beat ?? previous.beat,
          pulse: due.pulse ?? previous.pulse,
        }));
      }
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [playing]);

  const change = useCallback((next: MetronomeSettings) => {
    settingsRef.current = next;
    setSettings(next);
    updateMetronome(next).catch(() => {});
  }, []);

  const start = useCallback(() => {
    queue.current.clear();
    setHeard(NOTHING_HEARD);
    setPlaying(true);
    startMetronome(settingsRef.current).catch(() => setPlaying(false));
  }, []);

  const stop = useCallback(() => {
    queue.current.clear();
    setHeard(NOTHING_HEARD);
    setPlaying(false);
    stopMetronome().catch(() => {});
  }, []);

  const savePreset = useCallback(async (name: string): Promise<PresetErrorCode | null> => {
    try {
      setPresets(await savePresetCommand(name, settingsRef.current));
      return null;
    } catch (error) {
      return presetErrorCode(error);
    }
  }, []);

  const deletePreset = useCallback(async (name: string): Promise<PresetErrorCode | null> => {
    try {
      setPresets(await deletePresetCommand(name));
      return null;
    } catch (error) {
      return presetErrorCode(error);
    }
  }, []);

  return { settings, presets, playing, heard, change, start, stop, savePreset, deletePreset };
}
