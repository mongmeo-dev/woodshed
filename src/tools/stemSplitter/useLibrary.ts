import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelImport,
  deleteSong,
  type ImportErrorCode,
  type ImportProgress,
  importErrorCode,
  importFile,
  importLink,
  isAudioFile,
  listSongs,
  type SongSummary,
  youtubeVideoId,
} from "../../lib/stems";

export interface ImportState {
  progress: ImportProgress;
}

/** The song list plus file/link import with progress, cancel and errors. */
export function useLibrary() {
  const [songs, setSongs] = useState<SongSummary[]>([]);
  const [importing, setImporting] = useState<ImportState | null>(null);
  const [error, setError] = useState<ImportErrorCode | null>(null);
  const mounted = useRef(true);
  const busy = useRef(false);

  const refresh = useCallback(async () => {
    try {
      const next = await listSongs();
      if (mounted.current) setSongs(next);
    } catch {
      // Keep the last list; the next action refreshes it.
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    return () => {
      mounted.current = false;
    };
  }, [refresh]);

  const run = useCallback(
    async (
      first: ImportProgress,
      start: (onProgress: (p: ImportProgress) => void) => Promise<SongSummary>,
    ): Promise<SongSummary | null> => {
      if (busy.current) {
        setError("busy");
        return null;
      }
      busy.current = true;
      setError(null);
      setImporting({ progress: first });
      try {
        const song = await start((progress) => {
          if (mounted.current) setImporting({ progress });
        });
        await refresh();
        return song;
      } catch (failure) {
        const code = importErrorCode(failure);
        if (mounted.current && code !== "cancelled") setError(code);
        return null;
      } finally {
        busy.current = false;
        if (mounted.current) setImporting(null);
      }
    },
    [refresh],
  );

  const addFile = useCallback(
    (path: string) => {
      if (!isAudioFile(path)) {
        setError("unsupportedFile");
        return Promise.resolve(null);
      }
      return run({ stage: "decode", progress: 0 }, (onProgress) => importFile(path, onProgress));
    },
    [run],
  );

  const addLink = useCallback(
    (url: string) => {
      if (!youtubeVideoId(url)) {
        setError("invalidLink");
        return Promise.resolve(null);
      }
      return run({ stage: "download", progress: null }, (onProgress) =>
        importLink(url, onProgress),
      );
    },
    [run],
  );

  const cancel = useCallback(() => {
    void cancelImport();
  }, []);

  const remove = useCallback(
    async (id: string) => {
      try {
        await deleteSong(id);
      } finally {
        await refresh();
      }
    },
    [refresh],
  );

  const clearError = useCallback(() => setError(null), []);

  return { songs, importing, error, refresh, addFile, addLink, cancel, remove, clearError };
}
