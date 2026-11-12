import { useCallback, useEffect, useRef, useState } from "react";
import { heardAtToPerformanceTime, onOutputChanged, subscribeAsync } from "../../lib/audio";
import {
  closeSong,
  getTransport,
  type LoopRange,
  type OpenedSong,
  openSong,
  pause,
  play,
  positionAt,
  reflects,
  restore,
  type Scheduled,
  seek,
  setLoop as sendLoop,
  setMaster as sendMaster,
  setSpeed as sendSpeed,
  setTrack,
  type Transport,
} from "../../lib/stems";

export interface TrackMix {
  volume: number;
  muted: boolean;
  solo: boolean;
}

const DEFAULT_MIX: TrackMix = { volume: 1, muted: false, solo: false };
const POLL_PLAYING_MS = 200;
const POLL_PAUSED_MS = 1000;
/** Re-reads the transport this long after a command, once it has likely applied. */
const SETTLE_MS = 40;
/** Shortest loop the waveform selection makes. */
export const MIN_LOOP_SECS = 0.1;

/** Last transport sample and the performance time its position is heard. */
export interface TransportSample {
  positionSecs: number;
  playing: boolean;
  heardAt: number;
}

/** What the user asked for. The screen shows this; the engine catches up. */
interface Intent {
  playing: boolean;
  /** Sent but not yet seen applied. */
  seek: number | null;
  speed: number;
  loop: LoopRange | null;
  mix: TrackMix[];
  master: number;
}

function sampleFrom(transport: Transport, now = performance.now()): TransportSample {
  return {
    positionSecs: transport.positionSecs,
    playing: transport.playing,
    heardAt: transport.heardAtMs === null ? now : heardAtToPerformanceTime(transport.heardAtMs),
  };
}

/**
 * The open song, its mix, and transport synced to the heard time.
 *
 * Commands go out one at a time through a queue (so an output-change restore
 * never interleaves with a user command). Each transport command returns its
 * ack frame; transport samples that do not reach it ({@link reflects}) are
 * ignored, so a slow poll never undoes what the user just did.
 */
export function usePlayer() {
  const [opened, setOpened] = useState<OpenedSong | null>(null);
  const [openError, setOpenError] = useState(false);
  const [mix, setMix] = useState<TrackMix[]>([]);
  const [master, setMasterState] = useState(1);
  const [speed, setSpeedState] = useState(1);
  const [loop, setLoopState] = useState<LoopRange | null>(null);
  const [playing, setPlaying] = useState(false);
  /** Bumped whenever the shown position changes outside playback. */
  const [positionVersion, setPositionVersion] = useState(0);
  const sample = useRef<TransportSample>({ positionSecs: 0, playing: false, heardAt: 0 });
  const intent = useRef<Intent>({
    playing: false,
    seek: null,
    speed: 1,
    loop: null,
    mix: [],
    master: 1,
  });
  const openedRef = useRef<OpenedSong | null>(null);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  /** Transport commands sent whose frame is not known yet. */
  const inFlight = useRef(0);
  /** Samples that do not reach this ack frame predate the latest command. */
  const awaitAck = useRef(0);
  const mounted = useRef(true);

  const publishPosition = useCallback((next: TransportSample) => {
    sample.current = next;
    setPositionVersion((version) => version + 1);
  }, []);

  const refreshTransport = useCallback(async () => {
    if (inFlight.current > 0) return;
    try {
      const transport = await getTransport();
      if (!mounted.current || !transport || transport.songId !== openedRef.current?.song.id) {
        return;
      }
      if (inFlight.current > 0 || !reflects(transport, awaitAck.current)) return;
      intent.current.seek = null;
      intent.current.playing = transport.playing;
      setPlaying(transport.playing);
      const previous = sample.current;
      const next = sampleFrom(transport);
      if (transport.playing || previous.playing || next.positionSecs !== previous.positionSecs) {
        publishPosition(next);
      } else {
        sample.current = next;
      }
    } catch {
      // The next poll tries again.
    }
  }, [publishPosition]);

  /** Runs `task` after every earlier command. */
  const enqueue = useCallback(<T>(task: () => Promise<T>): Promise<T | undefined> => {
    const run = queue.current.then(task, task);
    queue.current = run.catch(() => undefined);
    return run.catch(() => undefined);
  }, []);

  /** Sends a command that changes playback and waits for its frame. */
  const sendTransport = useCallback(
    (command: () => Promise<Scheduled | null>) => {
      inFlight.current += 1;
      return enqueue(async () => {
        try {
          const scheduled = await command();
          if (scheduled) awaitAck.current = Math.max(awaitAck.current, scheduled.ackFrame);
        } finally {
          inFlight.current -= 1;
        }
        window.setTimeout(() => void refreshTransport(), SETTLE_MS);
      });
    },
    [enqueue, refreshTransport],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!opened) return;
    const timer = window.setInterval(
      () => void refreshTransport(),
      playing ? POLL_PLAYING_MS : POLL_PAUSED_MS,
    );
    return () => window.clearInterval(timer);
  }, [opened, playing, refreshTransport]);

  // A new output may run at another rate and the engine dropped what was
  // pending: schedule the whole intent again, in order with user commands.
  useEffect(() => {
    return subscribeAsync(() =>
      onOutputChanged(() => {
        if (!openedRef.current) return;
        void sendTransport(() => {
          const now = intent.current;
          return restore({
            playing: now.playing,
            seek: now.seek,
            speed: now.speed,
            loop: now.loop,
            tracks: now.mix,
            master: now.master,
          });
        });
      }),
    );
  }, [sendTransport]);

  /** Song position heard right now. */
  const positionNow = useCallback((now = performance.now()) => {
    const song = openedRef.current;
    if (!song) return 0;
    return positionAt(
      sample.current,
      sample.current.heardAt,
      now,
      intent.current.speed,
      intent.current.loop,
      song.song.durationSecs,
    );
  }, []);

  const open = useCallback(
    (id: string, startAt = 0) =>
      enqueue(async () => {
        setOpenError(false);
        try {
          const song = await openSong(id);
          if (!mounted.current || !song) return;
          openedRef.current = song;
          const mixes = song.tracks.map(() => ({ ...DEFAULT_MIX }));
          intent.current = {
            playing: false,
            seek: null,
            speed: 1,
            loop: null,
            mix: mixes,
            master: intent.current.master,
          };
          awaitAck.current = 0;
          publishPosition({ positionSecs: startAt, playing: false, heardAt: performance.now() });
          setOpened(song);
          setMix(mixes);
          setSpeedState(1);
          setLoopState(null);
          setPlaying(false);
          await sendMaster(intent.current.master);
          if (startAt > 0) {
            intent.current.seek = startAt;
            const scheduled = await seek(startAt);
            if (scheduled) awaitAck.current = Math.max(awaitAck.current, scheduled.ackFrame);
          }
        } catch {
          if (mounted.current) setOpenError(true);
        }
      }),
    [enqueue, publishPosition],
  );

  const close = useCallback(
    () =>
      enqueue(async () => {
        openedRef.current = null;
        intent.current.playing = false;
        setOpened(null);
        setPlaying(false);
        await closeSong();
      }),
    [enqueue],
  );

  const togglePlay = useCallback(() => {
    if (!openedRef.current) return Promise.resolve(undefined);
    const next = !intent.current.playing;
    intent.current.playing = next;
    setPlaying(next);
    if (!next) {
      // Hold the shown position where the user paused.
      publishPosition({ positionSecs: positionNow(), playing: false, heardAt: performance.now() });
    }
    return sendTransport(next ? play : pause);
  }, [positionNow, publishPosition, sendTransport]);

  const seekTo = useCallback(
    (seconds: number) => {
      const song = openedRef.current;
      if (!song) return Promise.resolve(undefined);
      const target = Math.min(Math.max(seconds, 0), song.song.durationSecs);
      intent.current.seek = target;
      publishPosition({
        positionSecs: target,
        playing: sample.current.playing,
        heardAt: performance.now(),
      });
      return sendTransport(() => seek(target));
    },
    [publishPosition, sendTransport],
  );

  const changeSpeed = useCallback(
    (value: number) => {
      intent.current.speed = value;
      setSpeedState(value);
      return enqueue(() => sendSpeed(value));
    },
    [enqueue],
  );

  const changeLoop = useCallback(
    (range: LoopRange | null) => {
      const valid = range && range.end - range.start >= MIN_LOOP_SECS ? range : null;
      intent.current.loop = valid;
      setLoopState(valid);
      return sendTransport(() => sendLoop(valid));
    },
    [sendTransport],
  );

  const changeTrack = useCallback(
    (index: number, patch: Partial<TrackMix>) => {
      const next = intent.current.mix.map((track, i) =>
        i === index ? { ...track, ...patch } : track,
      );
      const track = next[index];
      if (!track) return;
      intent.current.mix = next;
      setMix(next);
      void enqueue(() => setTrack(index, track));
    },
    [enqueue],
  );

  const changeMaster = useCallback(
    (value: number) => {
      intent.current.master = value;
      setMasterState(value);
      if (!openedRef.current) return Promise.resolve(undefined);
      return enqueue(() => sendMaster(value));
    },
    [enqueue],
  );

  return {
    opened,
    openError,
    mix,
    master,
    speed,
    loop,
    playing,
    positionVersion,
    positionNow,
    open,
    close,
    togglePlay,
    seekTo,
    changeSpeed,
    changeLoop,
    changeTrack,
    changeMaster,
  };
}
