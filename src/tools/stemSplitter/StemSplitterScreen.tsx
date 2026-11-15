import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { subscribeAsync } from "../../lib/audio";
import { inTauri } from "../../lib/ipc";
import {
  cancelSeparation,
  formatTime,
  isAudioFile,
  pickAudioFile,
  type SeparationEvent,
  separationFromStems,
  stemsReady,
} from "../../lib/stems";
import { ImportBar } from "./ImportBar";
import { Mixer } from "./Mixer";
import { SeparationSlot } from "./SeparationSlot";
import { SongList } from "./SongList";
import { useLibrary } from "./useLibrary";
import { usePlayer } from "./usePlayer";
import { useSeparations } from "./useSeparations";
import { useAnimationFrame, Waveform } from "./Waveform";
import "./stemSplitter.css";

const SPEED_MIN = 50;
const SPEED_MAX = 100;

function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="icon icon-filled">
      <path d="M8 5.5v13l10.5-6.5z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="icon icon-filled">
      <rect x="6.5" y="5.5" width="4" height="13" rx="1" />
      <rect x="13.5" y="5.5" width="4" height="13" rx="1" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="icon">
      <path d="M7 7l10 10M17 7L7 17" />
    </svg>
  );
}

/** Text fields and selects keep the space key for themselves. */
function typingTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    target.closest('input:not([type="range"]), textarea, select, [contenteditable="true"]') !== null
  );
}

/** Drag-and-drop of files onto the window (Tauri webview). */
function useFileDrop(onDrop: (path: string) => void) {
  const [dragging, setDragging] = useState(false);
  const handler = useRef(onDrop);
  useEffect(() => {
    handler.current = onDrop;
  });
  useEffect(() => {
    if (!inTauri()) return;
    return subscribeAsync(async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      return getCurrentWebview().onDragDropEvent(({ payload }) => {
        if (payload.type === "enter" || payload.type === "over") setDragging(true);
        else if (payload.type === "leave") setDragging(false);
        else {
          setDragging(false);
          const path = payload.paths.find(isAudioFile) ?? payload.paths[0];
          if (path) handler.current(path);
        }
      });
    });
  }, []);
  return dragging;
}

export function StemSplitterScreen() {
  const { t } = useTranslation();
  const library = useLibrary();
  const player = usePlayer();
  const timeRef = useRef<HTMLSpanElement>(null);
  const autoOpened = useRef(false);
  const { opened, open, positionNow } = player;
  const openId = opened?.song.id ?? null;
  const duration = opened?.song.durationSecs ?? 0;

  // Open the newest song when the screen first has songs.
  useEffect(() => {
    const newest = library.songs[0];
    if (autoOpened.current || openId || !newest) return;
    autoOpened.current = true;
    void open(newest.id);
  }, [library.songs, openId, open]);

  const addFile = useCallback(
    async (path: string) => {
      const song = await library.addFile(path);
      if (song) await open(song.id);
    },
    [library.addFile, open],
  );

  const addLink = useCallback(
    async (url: string) => {
      const song = await library.addLink(url);
      if (song) await open(song.id);
      return song;
    },
    [library.addLink, open],
  );

  const dragging = useFileDrop((path) => void addFile(path));

  const openFile = async () => {
    const path = await pickAudioFile();
    if (path) await addFile(path);
  };

  const removeSong = async (id: string) => {
    const wasOpen = id === openId;
    if (wasOpen) await player.close();
    await library.remove(id);
    if (wasOpen) autoOpened.current = false;
  };

  // Separation progress from the stem engine, for the screen's lifetime. A
  // finished song that is open reopens at the same position with its stems.
  const refreshLibrary = library.refresh;
  /** The song the latest reopen-with-stems was for (one reopen per finish). */
  const reopened = useRef<string | null>(null);
  const { separations, start: startSeparation } = useSeparations({
    onEvent: (event: SeparationEvent) => {
      if (event.state === "running") return;
      void refreshLibrary();
      if (event.state === "done" && event.songId === openId) {
        reopened.current = event.songId;
        void open(event.songId, positionNow());
      }
    },
    onReconciled: () => void refreshLibrary(),
  });

  // Space toggles playback.
  const { togglePlay } = player;
  useEffect(() => {
    if (!openId) return;
    const isSpace = (event: KeyboardEvent) => event.code === "Space" || event.key === " ";
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isSpace(event) || typingTarget(event.target) || event.metaKey || event.ctrlKey) return;
      event.preventDefault();
      if (!event.repeat) void togglePlay();
    };
    // Keeps a focused button from also activating on key up.
    const onKeyUp = (event: KeyboardEvent) => {
      if (isSpace(event) && !typingTarget(event.target)) event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [openId, togglePlay]);

  const showTime = useCallback(() => {
    if (timeRef.current) {
      timeRef.current.textContent = t("stems.player.time", {
        position: formatTime(positionNow()),
        duration: formatTime(duration),
      });
    }
  }, [t, positionNow, duration]);
  useAnimationFrame(player.playing, showTime);
  useEffect(showTime);

  // The list has the latest manifest (refreshed on every finished separation).
  const openSong = library.songs.find((song) => song.id === openId) ?? opened?.song ?? null;
  const openReady = openSong ? stemsReady(openSong) : false;
  const latest = openId ? separations[openId] : undefined;
  // The screen only separates songs that are not ready, so a ready manifest
  // wins over a running entry left from before.
  const separation =
    latest?.state === "running" && openReady
      ? null
      : (latest ?? (openSong ? separationFromStems(openSong) : null));
  const separating = separation?.state === "running";

  // A finish the screen missed (closed, or still subscribing): the list says
  // ready while the player still has the original track.
  const showingOriginal = opened?.tracks.length === 1 && opened.tracks[0] === "original";
  useEffect(() => {
    if (!openId || !openReady || !showingOriginal || reopened.current === openId) return;
    reopened.current = openId;
    void open(openId, positionNow());
  }, [openId, openReady, showingOriginal, open, positionNow]);

  const speedPercent = Math.round(player.speed * 100);
  const loop = player.loop;

  return (
    <section className="screen" aria-labelledby="screen-title-stemSplitter">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-stemSplitter">
          {t("tools.stemSplitter")}
        </h1>
      </header>
      <div className="screen-body stems">
        <div className="stems-layout">
          {(opened || player.openError) && (
            <div className="stems-stage">
              {player.openError && (
                <p className="row-error" role="alert">
                  {t("stems.errors.open")}
                </p>
              )}
              {opened && (
                <section aria-labelledby="stems-player">
                  <h2 className="stems-section-title song-heading" id="stems-player">
                    {opened.song.title}
                  </h2>
                  <div className="card player">
                    <Waveform
                      peaks={opened.waveform}
                      duration={duration}
                      loop={loop}
                      playing={player.playing}
                      positionVersion={player.positionVersion}
                      positionNow={positionNow}
                      onSeek={(seconds) => void player.seekTo(seconds)}
                      onLoop={(range) => void player.changeLoop(range)}
                      label={t("stems.player.position")}
                      valueText={formatTime}
                    />
                    <div className="player-controls">
                      <button
                        type="button"
                        className="play-button"
                        aria-label={
                          player.playing ? t("stems.player.pause") : t("stems.player.play")
                        }
                        onClick={() => void togglePlay()}
                      >
                        {player.playing ? <PauseIcon /> : <PlayIcon />}
                      </button>
                      <span className="player-time" ref={timeRef} />
                      {loop && (
                        <span className="loop-chip">
                          <span>
                            {t("stems.player.loopRange", {
                              start: formatTime(loop.start),
                              end: formatTime(loop.end),
                            })}
                          </span>
                          <button
                            type="button"
                            className="icon-button icon-button-small"
                            aria-label={t("stems.player.loopClear")}
                            title={t("stems.player.loopClear")}
                            onClick={() => void player.changeLoop(null)}
                          >
                            <CloseIcon />
                          </button>
                        </span>
                      )}
                      <div className="player-speed">
                        <label className="player-speed-label" htmlFor="stems-speed">
                          {t("stems.player.speed")}
                        </label>
                        <input
                          id="stems-speed"
                          type="range"
                          className="range"
                          min={SPEED_MIN}
                          max={SPEED_MAX}
                          step={1}
                          value={speedPercent}
                          aria-valuetext={t("stems.percent", { value: speedPercent })}
                          onChange={(event) =>
                            void player.changeSpeed(Number(event.target.value) / 100)
                          }
                        />
                        <span className="player-speed-value">
                          {t("stems.percent", { value: speedPercent })}
                        </span>
                      </div>
                    </div>
                  </div>
                </section>
              )}
            </div>
          )}

          <div className="stems-column">
            <ImportBar
              importing={library.importing}
              error={library.error}
              dragging={dragging}
              onOpenFile={() => void openFile()}
              onLink={addLink}
              onCancel={library.cancel}
              onEdit={library.clearError}
            />
            <section aria-labelledby="stems-songs">
              <h2 className="stems-section-title" id="stems-songs">
                {t("stems.library.title")}
              </h2>
              <SongList
                songs={library.songs}
                openId={openId}
                onOpen={(id) => void open(id)}
                onDelete={(id) => void removeSong(id)}
              />
            </section>
          </div>

          {opened && (
            <div className="stems-column">
              <section aria-labelledby="stems-mixer">
                <div className="stems-section-header">
                  <h2 className="stems-section-title" id="stems-mixer">
                    {t("stems.mixer.title")}
                  </h2>
                  <SeparationSlot
                    separation={separation}
                    onSeparate={
                      separating || separation?.state === "done" || openReady
                        ? undefined
                        : () => void startSeparation(opened.song.id)
                    }
                    onCancel={separating ? () => void cancelSeparation(opened.song.id) : undefined}
                  />
                </div>
                <Mixer
                  tracks={opened.tracks}
                  mix={player.mix}
                  master={player.master}
                  onTrack={player.changeTrack}
                  onMaster={(value) => void player.changeMaster(value)}
                />
              </section>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
