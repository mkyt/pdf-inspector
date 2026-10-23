import { useCallback, useState } from "react";

const PREFIX = "pdfi.";

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(PREFIX + key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}

/** State kept in this browser across reloads (sizes and toggles of the panes). */
export function useStored<T>(key: string, fallback: T): [T, (v: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState(() => load(key, fallback));
  const set = useCallback(
    (v: T | ((prev: T) => T)) =>
      setValue((prev) => {
        const next = typeof v === "function" ? (v as (prev: T) => T)(prev) : v;
        try {
          localStorage.setItem(PREFIX + key, JSON.stringify(next));
        } catch {
          // Private mode or storage full: keep it for this tab only.
        }
        return next;
      }),
    [key],
  );
  return [value, set];
}

/** A pane's size in px (width of a sidebar or a column, height of the hex view) and whether it is shown. */
export interface Pane {
  size: number;
  shown: boolean;
  setSize(px: number): void;
  toggle(): void;
  /** Back to the default size. */
  reset(): void;
}

export function usePane(key: string, def: number, min: number, max: number): Pane {
  const [size, setSize] = useStored(`${key}.size`, def);
  const [shown, setShown] = useStored(`${key}.shown`, true);
  return {
    size: Math.min(max, Math.max(min, size)),
    shown,
    setSize: (px) => setSize(Math.round(Math.min(max, Math.max(min, px)))),
    toggle: () => setShown((s) => !s),
    reset: () => setSize(def),
  };
}

export interface Layout {
  left: Pane;
  right: Pane;
  hex: Pane;
}

export function useLayout(): Layout {
  return {
    left: usePane("left", 380, 220, 900),
    right: usePane("right", 300, 200, 800),
    hex: usePane("hex", 280, 100, 1200),
  };
}
