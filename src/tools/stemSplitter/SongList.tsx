import { useState } from "react";
import { useTranslation } from "react-i18next";
import { formatTime, type SongSummary } from "../../lib/stems";

interface SongListProps {
  songs: SongSummary[];
  openId: string | null;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
}

function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="icon">
      <path d="M5 7h14" />
      <path d="M10 4h4" />
      <path d="M7 7l1 13h8l1-13" />
    </svg>
  );
}

export function SongList({ songs, openId, onOpen, onDelete }: SongListProps) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState<string | null>(null);

  if (songs.length === 0) {
    return (
      <div className="card">
        <div className="row">
          <span className="row-value">{t("stems.library.empty")}</span>
        </div>
      </div>
    );
  }

  return (
    <ul className="card song-list">
      {songs.map((song) => (
        <li className="row song-row" key={song.id}>
          {confirming === song.id ? (
            <div className="song-confirm">
              <span className="song-confirm-text">{t("stems.library.confirmDelete")}</span>
              <button
                type="button"
                className="button-secondary"
                onClick={() => {
                  setConfirming(null);
                  onDelete(song.id);
                }}
              >
                {t("stems.library.delete")}
              </button>
              <button
                type="button"
                className="button-secondary"
                onClick={() => setConfirming(null)}
              >
                {t("stems.import.cancel")}
              </button>
            </div>
          ) : (
            <>
              <button
                type="button"
                className="song-open"
                aria-current={song.id === openId ? "true" : undefined}
                onClick={() => onOpen(song.id)}
              >
                <span className="song-title">{song.title}</span>
                <span className="row-value">{formatTime(song.durationSecs)}</span>
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={t("stems.library.deleteSong", { title: song.title })}
                title={t("stems.library.delete")}
                onClick={() => setConfirming(song.id)}
              >
                <TrashIcon />
              </button>
            </>
          )}
        </li>
      ))}
    </ul>
  );
}
