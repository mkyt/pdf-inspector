import type { ReactNode } from "react";

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.3} aria-hidden>
      {children}
    </svg>
  );
}

/** A window with one of its panes filled when shown. */
function PaneIcon({ pane, shown }: { pane: "left" | "right" | "bottom"; shown: boolean }) {
  const r =
    pane === "left"
      ? { x: 2, y: 3, width: 4, height: 10 }
      : pane === "right"
        ? { x: 10, y: 3, width: 4, height: 10 }
        : { x: 2, y: 9, width: 12, height: 4 };
  return (
    <Icon>
      <rect x={2} y={3} width={12} height={10} rx={1.5} />
      <rect {...r} fill={shown ? "currentColor" : "none"} />
    </Icon>
  );
}

export const LeftPaneIcon = ({ shown }: { shown: boolean }) => <PaneIcon pane="left" shown={shown} />;
export const RightPaneIcon = ({ shown }: { shown: boolean }) => <PaneIcon pane="right" shown={shown} />;
export const BottomPaneIcon = ({ shown }: { shown: boolean }) => <PaneIcon pane="bottom" shown={shown} />;

export function SunIcon() {
  return (
    <Icon>
      <circle cx={8} cy={8} r={3} />
      <path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" />
    </Icon>
  );
}

export function MoonIcon() {
  return (
    <Icon>
      <path d="M13.5 9.5A5.5 5.5 0 0 1 6.5 2.5a5.5 5.5 0 1 0 7 7Z" />
    </Icon>
  );
}

/** Half light, half dark: follow the system setting. */
export function AutoThemeIcon() {
  return (
    <Icon>
      <circle cx={8} cy={8} r={5.5} />
      <path d="M8 2.5a5.5 5.5 0 0 1 0 11Z" fill="currentColor" />
    </Icon>
  );
}

export function BackIcon() {
  return (
    <Icon>
      <path d="M10 3 5 8l5 5" />
    </Icon>
  );
}

export function ForwardIcon() {
  return (
    <Icon>
      <path d="M6 3l5 5-5 5" />
    </Icon>
  );
}
