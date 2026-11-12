import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Sliding selection indicator (Kinetics "Tab Pill Glide"): measures the child
 * marked `data-glide-active="true"` and moves an indicator onto it.
 * `measureKey` must change whenever the active item or its size can change.
 */
export function useGlide<T extends HTMLElement>(measureKey: string) {
  const containerRef = useRef<T>(null);
  const [box, setBox] = useState<Box | null>(null);
  const [ready, setReady] = useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: measureKey is the re-measure trigger.
  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const measure = () => {
      const active = container.querySelector<HTMLElement>('[data-glide-active="true"]');
      setBox(
        active
          ? {
              left: active.offsetLeft,
              top: active.offsetTop,
              width: active.offsetWidth,
              height: active.offsetHeight,
            }
          : null,
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [measureKey]);

  // Enable the transition only after the first placement has been committed,
  // so the indicator does not glide in from 0. (An effect, not rAF: rAF does
  // not run while the window is hidden.)
  useEffect(() => {
    if (box && !ready) setReady(true);
  }, [box, ready]);

  const style: CSSProperties = box
    ? { transform: `translate(${box.left}px, ${box.top}px)`, width: box.width, height: box.height }
    : { display: "none" };

  return { containerRef, indicatorStyle: style, ready: ready && box !== null };
}
