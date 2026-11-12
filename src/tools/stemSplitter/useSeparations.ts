import { useCallback, useEffect, useRef, useState } from "react";
import { listSeparations, onSeparation, type SeparationEvent, separate } from "../../lib/stems";

interface SeparationHandlers {
  /** Every event, after it is stored. */
  onEvent: (event: SeparationEvent) => void;
  /** After the snapshot taken once the subscription is in place is merged. */
  onReconciled: () => void;
}

/**
 * Separation state per song for the screen's lifetime: one subscription to
 * `stems://separation`, and once it is in place, a snapshot of the unfinished
 * separations that fills in whatever the screen missed while it was closed or
 * still subscribing. Events (and separations started here) after the snapshot
 * was asked for win over it, and a `running` entry it does not confirm is
 * dropped, so no finished separation stays shown as running.
 */
export function useSeparations(handlers: SeparationHandlers) {
  const [separations, setSeparations] = useState<Record<string, SeparationEvent>>({});
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });
  /** Songs with news newer than the snapshot being fetched. */
  const fresh = useRef(new Set<string>());

  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | null = null;
    const reconcile = async () => {
      fresh.current.clear();
      let running: SeparationEvent[];
      try {
        running = await listSeparations();
      } catch {
        return;
      }
      if (!live) return;
      setSeparations((current) => {
        const next: Record<string, SeparationEvent> = {};
        for (const [songId, event] of Object.entries(current)) {
          if (fresh.current.has(songId) || event.state !== "running") next[songId] = event;
        }
        for (const event of running) {
          if (!fresh.current.has(event.songId)) next[event.songId] = event;
        }
        return next;
      });
      latest.current.onReconciled();
    };
    onSeparation((event) => {
      if (!live) return;
      fresh.current.add(event.songId);
      setSeparations((current) => ({ ...current, [event.songId]: event }));
      latest.current.onEvent(event);
    }).then(
      (stop) => {
        if (!live) {
          stop();
          return;
        }
        unlisten = stop;
        void reconcile();
      },
      () => {},
    );
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  /** Starts separating a song; shown as preparing until its events arrive. */
  const start = useCallback(async (songId: string) => {
    fresh.current.add(songId);
    setSeparations((current) => ({
      ...current,
      [songId]: { songId, state: "running", progress: null },
    }));
    try {
      await separate(songId);
    } catch {
      setSeparations((current) => ({
        ...current,
        [songId]: { songId, state: "failed", progress: null },
      }));
    }
  }, []);

  return { separations, start };
}
