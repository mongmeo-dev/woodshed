import { type RefObject, useCallback, useEffect, useRef, useState } from "react";
import { type DeviceInfo, listDevices, onDevicesChanged, subscribeAsync } from "../../lib/audio";
import {
  DEFAULT_STATUS,
  type InputChannel,
  type Reading,
  selectInput,
  setReference,
  startTuner,
  stopTuner,
  type TunerStatus,
} from "./tuner";

/** The newest analysis frame; read by the drawing loop, not rendered by React. */
export interface LatestFrame {
  levelDb: number;
  pitch: Reading | null;
  /** Bumped when earlier readings no longer apply (input lost or changed). */
  epoch: number;
}

/** Whether readings from before `next` still describe the running input. */
export function keepsReadings(previous: TunerStatus, next: TunerStatus): boolean {
  return next.state === "listening" && next.inputGeneration === previous.inputGeneration;
}

// Start and stop run in call order (a quick remount must not end stopped).
let queue: Promise<unknown> = Promise.resolve();
function inOrder<T>(task: () => Promise<T>): Promise<T> {
  const next = queue.then(task, task);
  queue = next.catch(() => undefined);
  return next;
}

export interface TunerControls {
  status: TunerStatus;
  devices: DeviceInfo[];
  latest: RefObject<LatestFrame>;
  saveFailed: boolean;
  chooseInput: (deviceId: string | null, channel: InputChannel) => Promise<void>;
  changeReference: (hz: number) => Promise<void>;
}

/** Listens while mounted: status in React state, frames in a ref. */
export function useTuner(): TunerControls {
  const [status, setStatus] = useState<TunerStatus>(DEFAULT_STATUS);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [saveFailed, setSaveFailed] = useState(false);
  const latest = useRef<LatestFrame>({ levelDb: -100, pitch: null, epoch: 0 });
  const current = useRef<TunerStatus>(DEFAULT_STATUS);

  useEffect(() => {
    let active = true;
    let heard = false;
    const applyStatus = (next: TunerStatus) => {
      if (!keepsReadings(current.current, next)) {
        // No frame may arrive any more (input gone, permission off): drop the
        // last reading now instead of leaving it on screen.
        latest.current = { levelDb: -100, pitch: null, epoch: latest.current.epoch + 1 };
      }
      current.current = next;
      setStatus(next);
    };
    void inOrder(() =>
      startTuner((event) => {
        if (!active) return;
        if (event.type === "status") {
          const { type: _, ...next } = event;
          heard = true;
          applyStatus(next);
        } else {
          latest.current = { ...latest.current, levelDb: event.levelDb, pitch: event.pitch };
        }
      }),
    ).then(
      (initial) => {
        // A status from the channel is newer than this snapshot.
        if (active && !heard) applyStatus(initial);
      },
      () => {},
    );
    return () => {
      active = false;
      void inOrder(stopTuner).catch(() => {});
    };
  }, []);

  useEffect(() => {
    let mounted = true;
    const refresh = () =>
      listDevices("input").then(
        (next) => mounted && setDevices(next),
        () => {},
      );
    void refresh();
    const stop = subscribeAsync(() => onDevicesChanged(() => void refresh()));
    return () => {
      mounted = false;
      stop();
    };
  }, []);

  const chooseInput = useCallback(async (deviceId: string | null, channel: InputChannel) => {
    setStatus((current) => ({ ...current, preferredDeviceId: deviceId, channel }));
    try {
      await selectInput(deviceId, channel);
      setSaveFailed(false);
    } catch {
      setSaveFailed(true);
    }
  }, []);

  const changeReference = useCallback(async (hz: number) => {
    setStatus((current) => ({ ...current, referenceHz: hz }));
    try {
      const applied = await setReference(hz);
      setStatus((current) => ({ ...current, referenceHz: applied }));
      setSaveFailed(false);
    } catch {
      setSaveFailed(true);
    }
  }, []);

  return { status, devices, latest, saveFailed, chooseInput, changeReference };
}
