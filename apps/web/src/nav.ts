import { createContext, useContext } from "react";
import type { DocId, NodeView, ObjRef, Span } from "./protocol/generated";

export type Target =
  | { kind: "document" }
  | { kind: "trailer" }
  | { kind: "object"; id: ObjRef }
  /** A page by 0-based index. */
  | { kind: "page"; index: number };

/** An entry picked in the structure tree, shown in the schema panel. */
export interface Selection {
  node: NodeView;
  /** Location in the object, like `/Pages/Kids[3]`. */
  path: string;
}

export interface Nav {
  doc: DocId;
  target: Target;
  open(t: Target): void;
  back(): void;
  canBack: boolean;
  /** Highlight a span of the file in the hex view and scroll to it. */
  showBytes(s: Span): void;
  hexSpan: Span | null;
  selection: Selection | null;
  select(s: Selection | null): void;
}

export const NavContext = createContext<Nav | null>(null);

export function useNav(): Nav {
  const n = useContext(NavContext);
  if (!n) throw new Error("no document");
  return n;
}

export function sameTarget(a: Target, b: Target): boolean {
  switch (a.kind) {
    case "document":
    case "trailer":
      return a.kind === b.kind;
    case "object":
      return b.kind === "object" && a.id.num === b.id.num && a.id.generation === b.id.generation;
    case "page":
      return b.kind === "page" && a.index === b.index;
  }
}
