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
  const latest = useRef<LatestFrame>({ levelDb: -100, pitch: null });

  useEffect(() => {
    let active = true;
    void inOrder(() =>
      startTuner((event) => {
        if (!active) return;
        if (event.type === "status") {
          const { type: _, ...next } = event;
          setStatus(next);
        } else {
          latest.current = { levelDb: event.levelDb, pitch: event.pitch };
        }
      }),
    ).then(
      (initial) => {
        if (active) setStatus(initial);
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
