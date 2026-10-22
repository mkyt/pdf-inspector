import { useCallback, useEffect, useState } from "react";
import type { Target } from "./nav";
import type { DocId } from "./protocol/generated";

/**
 * What the window shows, kept in the query string (`?doc=1&tab=images&view=obj:12.0`)
 * so that reloads and links restore it and the browser's back and forward
 * buttons walk through it.
 */
export interface Route {
  doc: DocId | null;
  target: Target;
  /** Tab of the side panel. */
  tab: string;
}

export const DEFAULT_TAB = "pages";

function parseTarget(v: string | null): Target {
  let m: RegExpMatchArray | null;
  if (v === "trailer") return { kind: "trailer" };
  if ((m = v?.match(/^obj:(\d+)\.(\d+)$/) ?? null)) {
    return { kind: "object", id: { num: Number(m[1]), generation: Number(m[2]) } };
  }
  // Pages are 1-based in the URL, like everywhere else people see them.
  if ((m = v?.match(/^page:(\d+)$/) ?? null) && Number(m[1]) > 0) return { kind: "page", index: Number(m[1]) - 1 };
  return { kind: "document" };
}

function targetParam(t: Target): string | null {
  switch (t.kind) {
    case "document":
      return null;
    case "trailer":
      return "trailer";
    case "object":
      return `obj:${t.id.num}.${t.id.generation}`;
    case "page":
      return `page:${t.index + 1}`;
  }
}

export function parseRoute(search: string): Route {
  const p = new URLSearchParams(search);
  const doc = p.get("doc");
  return {
    doc: doc !== null && /^\d+$/.test(doc) ? Number(doc) : null,
    target: parseTarget(p.get("view")),
    tab: p.get("tab") ?? DEFAULT_TAB,
  };
}

function routeUrl(r: Route): string {
  const p = new URLSearchParams();
  if (r.doc !== null) {
    p.set("doc", String(r.doc));
    if (r.tab !== DEFAULT_TAB) p.set("tab", r.tab);
    const view = targetParam(r.target);
    if (view) p.set("view", view);
  }
  const q = p.toString().replace(/%3A/g, ":");
  return location.pathname + (q ? `?${q}` : "") + location.hash;
}

/** Position of an entry in this tab's history, to know whether "back" stays in the app. */
interface EntryState {
  pdfiIndex: number;
}

function entryIndex(): number {
  const s = history.state as Partial<EntryState> | null;
  return typeof s?.pdfiIndex === "number" ? s.pdfiIndex : 0;
}

export interface Router {
  route: Route;
  /** Go somewhere new: a new history entry. */
  push(r: Route): void;
  /** Change the current entry in place (tab switches, corrections). */
  replace(r: Route): void;
  back(): void;
  canBack: boolean;
}

export function useRouter(): Router {
  const [route, setRoute] = useState(() => parseRoute(location.search));
  const [index, setIndex] = useState(entryIndex);

  useEffect(() => {
    // Mark the entry we were loaded in, keeping its index across reloads.
    history.replaceState({ ...history.state, pdfiIndex: entryIndex() } satisfies EntryState, "");
    const onPop = () => {
      setRoute(parseRoute(location.search));
      setIndex(entryIndex());
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const push = useCallback((r: Route) => {
    const url = routeUrl(r);
    if (url === location.pathname + location.search + location.hash) return;
    const next = entryIndex() + 1;
    history.pushState({ pdfiIndex: next } satisfies EntryState, "", url);
    setRoute(r);
    setIndex(next);
  }, []);

  const replace = useCallback((r: Route) => {
    history.replaceState(history.state, "", routeUrl(r));
    setRoute(r);
  }, []);

  const back = useCallback(() => history.back(), []);
  return { route, push, replace, back, canBack: index > 0 };
}
