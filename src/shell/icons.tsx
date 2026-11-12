import type { ReactElement } from "react";
import type { ViewId } from "../tools/tools";

const paths: Record<ViewId, ReactElement> = {
  tuner: (
    <>
      <path d="M4 16a8 8 0 0 1 16 0" />
      <path d="M12 16l3.5-6" />
      <circle cx="12" cy="16" r="1.2" fill="currentColor" stroke="none" />
    </>
  ),
  metronome: (
    <>
      <path d="M9.5 3.5h5L18 20.5H6z" />
      <path d="M12 16.5l4.5-8" />
      <path d="M8.5 16.5h7" />
    </>
  ),
  drumMachine: (
    <>
      <rect x="4" y="4" width="7" height="7" rx="1.5" />
      <rect x="13" y="4" width="7" height="7" rx="1.5" />
      <rect x="4" y="13" width="7" height="7" rx="1.5" />
      <rect x="13" y="13" width="7" height="7" rx="1.5" />
    </>
  ),
  stemSplitter: (
    <>
      <path d="M4 7h16" />
      <path d="M4 12h11" />
      <path d="M4 17h14" />
    </>
  ),
  fretboard: (
    <>
      <rect x="3.5" y="6" width="17" height="12" rx="1.5" />
      <path d="M9 6v12M14.5 6v12M3.5 10h17M3.5 14h17" />
      <circle cx="11.75" cy="12" r="1.3" fill="currentColor" stroke="none" />
    </>
  ),
  settings: (
    <>
      <path d="M4 7.5h9M18 7.5h2M4 16.5h3M11 16.5h9" />
      <circle cx="15.5" cy="7.5" r="2.2" />
      <circle cx="8.5" cy="16.5" r="2.2" />
    </>
  ),
};

export function ViewIcon({ view }: { view: ViewId }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[view]}
    </svg>
  );
}
