import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef } from "react";
import { asciiByte, hexByte } from "../format";
import classes from "./Hex.module.css";

const ROW = 16;
const LINE_H = 18;

/** Hex dump of bytes already in memory. `base` is the offset of `data[0]`. */
export function BytesDump({ data, base = 0, height = 400 }: { data: Uint8Array; base?: number; height?: number }) {
  const parent = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: Math.ceil(data.length / ROW),
    getScrollElement: () => parent.current,
    estimateSize: () => LINE_H,
    overscan: 30,
  });
  return (
    <div ref={parent} className={classes.dump} style={{ height, overflow: "auto" }}>
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {v.getVirtualItems().map((it) => {
          const off = it.index * ROW;
          const row = data.subarray(off, off + ROW);
          return (
            <div key={it.key} className={classes.line} style={{ transform: `translateY(${it.start}px)` }}>
              <span className={classes.offset}>{(base + off).toString(16).padStart(8, "0")}</span>
              <span className={classes.hex}>{Array.from(row, hexByte).join(" ")}</span>
              <span className={classes.ascii}>{Array.from(row, asciiByte).join("")}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
