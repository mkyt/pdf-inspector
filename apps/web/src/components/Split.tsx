import type { PointerEvent } from "react";
import type { Pane } from "../layout";
import classes from "./Split.module.css";

/**
 * Border between a pane and the rest that can be dragged to resize the pane
 * (double-click: default size). `grow` is the direction that makes it larger:
 * `right` for a left sidebar, `left` for a right one, `up` for a bottom pane.
 */
export function ResizeHandle({ pane, grow, label }: { pane: Pane; grow: "left" | "right" | "up"; label: string }) {
  const vertical = grow !== "up";
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const start = vertical ? e.clientX : e.clientY;
    const size = pane.size;
    const sign = grow === "right" ? 1 : -1;
    const move = (ev: globalThis.PointerEvent) =>
      pane.setSize(size + sign * ((vertical ? ev.clientX : ev.clientY) - start));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      document.body.style.removeProperty("cursor");
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    document.body.style.cursor = vertical ? "col-resize" : "row-resize";
  };
  return (
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-label={label}
      aria-valuenow={pane.size}
      tabIndex={0}
      title={`Drag to resize the ${label}; double-click to reset`}
      className={classes.handle}
      data-vertical={vertical || undefined}
      onPointerDown={onPointerDown}
      onDoubleClick={pane.reset}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 64 : 16;
        const keys = vertical ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
        const k = keys.indexOf(e.key);
        if (k < 0) return;
        e.preventDefault();
        // ArrowRight / ArrowDown move the border right / down.
        const dir = k === 1 ? 1 : -1;
        pane.setSize(pane.size + dir * step * (grow === "right" ? 1 : -1));
      }}
    />
  );
}
