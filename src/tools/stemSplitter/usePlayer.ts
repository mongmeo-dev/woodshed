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
  resync,
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
/** Shortest loop the waveform selection makes. */
export const MIN_LOOP_SECS = 0.1;

/** Last transport sample and the performance time its position is heard. */
export interface TransportSample {
  positionSecs: number;
  playing: boolean;
  heardAt: number;
}

function sampleFrom(transport: Transport, now = performance.now()): TransportSample {
  return {
    positionSecs: transport.positionSecs,
    playing: transport.playing,
    heardAt: transport.heardAtMs === null ? now : heardAtToPerformanceTime(transport.heardAtMs),
  };
}

/** The open song, its mix, and transport synced to the heard time. */
export function usePlayer() {
  const [opened, setOpened] = useState<OpenedSong | null>(null);
  const [openError, setOpenError] = useState(false);
  const [mix, setMix] = useState<TrackMix[]>([]);
  const [master, setMasterState] = useState(1);
  const [speed, setSpeedState] = useState(1);
  const [loop, setLoopState] = useState<LoopRange | null>(null);
  const [playing, setPlaying] = useState(false);
  const sample = useRef<TransportSample>({ positionSecs: 0, playing: false, heardAt: 0 });
  const wantPlaying = useRef(false);
  const openedRef = useRef<OpenedSong | null>(null);
  const speedRef = useRef(1);
  const loopRef = useRef<LoopRange | null>(null);
  const masterRef = useRef(1);
  const mixRef = useRef<TrackMix[]>([]);
  const mounted = useRef(true);

  const refreshTransport = useCallback(async () => {
    try {
      const transport = await getTransport();
      if (!mounted.current || !transport || transport.songId !== openedRef.current?.song.id) {
        return;
      }
      sample.current = sampleFrom(transport);
      setPlaying(transport.playing);
      if (!transport.playing) wantPlaying.current = false;
    } catch {
      // The next poll tries again.
    }
  }, []);

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

  // A new output may run at another rate: drop pending events, then put
  // the transport back the way the user left it.
  useEffect(() => {
    return subscribeAsync(() =>
      onOutputChanged(async () => {
        if (!openedRef.current) return;
        await resync();
        if (wantPlaying.current) await play();
        await refreshTransport();
      }),
    );
  }, [refreshTransport]);

  /** Song position heard right now. */
  const positionNow = useCallback((now = performance.now()) => {
    const song = openedRef.current;
    if (!song) return 0;
    return positionAt(
      sample.current,
      sample.current.heardAt,
      now,
      speedRef.current,
      loopRef.current,
      song.song.durationSecs,
    );
  }, []);

  const open = useCallback(async (id: string, startAt = 0) => {
    setOpenError(false);
    try {
      const song = await openSong(id);
      if (!mounted.current || !song) return;
      openedRef.current = song;
      wantPlaying.current = false;
      speedRef.current = 1;
      loopRef.current = null;
      sample.current = { positionSecs: startAt, playing: false, heardAt: performance.now() };
      setOpened(song);
      mixRef.current = song.tracks.map(() => ({ ...DEFAULT_MIX }));
      setMix(mixRef.current);
      setSpeedState(1);
      setLoopState(null);
      setPlaying(false);
      await sendMaster(masterRef.current);
      if (startAt > 0) await seek(startAt);
    } catch {
      if (mounted.current) setOpenError(true);
    }
  }, []);

  const close = useCallback(async () => {
    openedRef.current = null;
    wantPlaying.current = false;
    setOpened(null);
    setPlaying(false);
    await closeSong();
  }, []);

  const togglePlay = useCallback(async () => {
    if (!openedRef.current) return;
    const next = !wantPlaying.current && !sample.current.playing;
    wantPlaying.current = next;
    setPlaying(next);
    if (next) await play();
    else await pause();
    await refreshTransport();
  }, [refreshTransport]);

  const seekTo = useCallback(
    async (seconds: number) => {
      const song = openedRef.current;
      if (!song) return;
      const target = Math.min(Math.max(seconds, 0), song.song.durationSecs);
      sample.current = { ...sample.current, positionSecs: target, heardAt: performance.now() };
      await seek(target);
      await refreshTransport();
    },
    [refreshTransport],
  );

  const changeSpeed = useCallback(async (value: number) => {
    speedRef.current = value;
    setSpeedState(value);
    await sendSpeed(value);
  }, []);

  const changeLoop = useCallback(async (range: LoopRange | null) => {
    const valid = range && range.end - range.start >= MIN_LOOP_SECS ? range : null;
    loopRef.current = valid;
    setLoopState(valid);
    await sendLoop(valid);
  }, []);

  const changeTrack = useCallback((index: number, patch: Partial<TrackMix>) => {
    const next = mixRef.current.map((track, i) => (i === index ? { ...track, ...patch } : track));
    const track = next[index];
    if (!track) return;
    mixRef.current = next;
    setMix(next);
    void setTrack(index, track);
  }, []);

  const changeMaster = useCallback(async (value: number) => {
    masterRef.current = value;
    setMasterState(value);
    if (openedRef.current) await sendMaster(value);
  }, []);

  return {
    opened,
    openError,
    mix,
    master,
    speed,
    loop,
    playing,
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
