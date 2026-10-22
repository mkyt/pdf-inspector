import { createContext, useContext } from "react";
import type { DocSummary, InventoryStats, JobInfo } from "./protocol/generated";

/** Document-wide state shared by the panels: the summary and the inventory job. */
export interface DocState {
  summary: DocSummary;
  inventory: {
    /** What the backend's inventory holds, once built (fonts, images, references). */
    stats: InventoryStats | null;
    job: JobInfo | null;
    start(): void;
    error: string | null;
  };
}

export const DocContext = createContext<DocState | null>(null);

export function useDoc(): DocState {
  const d = useContext(DocContext);
  if (!d) throw new Error("no document");
  return d;
}
