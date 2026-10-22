import { createContext, useContext } from "react";
import type { ContentRef, DocId, NodeView, ObjRef, Span } from "./protocol/generated";

export type Target =
  | { kind: "document" }
  | { kind: "trailer" }
  | { kind: "object"; id: ObjRef }
  /** A page by 0-based index. */
  | { kind: "page"; index: number }
  /** The operations of a content, with one of them selected. */
  | { kind: "content"; content: ContentRef; op: number | null }
  /** A font dictionary: program, encoding and glyphs. */
  | { kind: "font"; id: ObjRef };

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
  /** Change the current target in place (no new history entry), e.g. the selected operation. */
  update(t: Target): void;
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
    case "font":
      return b.kind === a.kind && a.id.num === b.id.num && a.id.generation === b.id.generation;
    case "page":
      return b.kind === "page" && a.index === b.index;
    case "content":
      return b.kind === "content" && sameContent(a.content, b.content);
  }
}

export function sameContent(a: ContentRef, b: ContentRef): boolean {
  return a.kind === "page"
    ? b.kind === "page" && a.index === b.index
    : b.kind === "stream" && a.id.num === b.id.num && a.id.generation === b.id.generation;
}

/** Query-key part naming a content. */
export function contentKey(c: ContentRef): (string | number)[] {
  return c.kind === "page" ? ["page", c.index] : ["stream", c.id.num, c.id.generation];
}
