interface ProgressBarProps {
  label: string;
  /** 0–1, or null while the amount is unknown. */
  value: number | null;
}

/** Thin bar; the fill eases to its new width (Kinetics "Elastic Progress", without overshoot). */
export function ProgressBar({ label, value }: ProgressBarProps) {
  const percent = value === null ? null : Math.round(Math.min(Math.max(value, 0), 1) * 100);
  return (
    <div
      className="progress"
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      data-indeterminate={percent === null}
    >
      <div
        className="progress-fill"
        style={{ width: percent === null ? undefined : `${percent}%` }}
      />
    </div>
  );
}
