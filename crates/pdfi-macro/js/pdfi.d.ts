/**
 * The API of pdf-inspector macros (`meta.api: 1`).
 *
 * A macro is one JavaScript module (no `import`). It exports `meta`, which
 * says what it is and which parameters it takes, and a default async
 * function that does the work:
 *
 * ```js
 * // @ts-check
 * /// <reference path="pdfi.d.ts" />
 * export const meta = { api: 1, name: "page-count", title: "Page count", edits: false };
 *
 * /** @type {pdfi.Main} *\/
 * export default async function (doc, params, ctx) {
 *   return { pages: doc.pageCount };
 * }
 * ```
 *
 * Conventions of the whole API:
 *
 * - Everything that reads or changes the document is `async`.
 * - Page numbers, operation numbers and indices are **0-based**. Only what
 *   people write (a `pages` parameter, the command line) counts from 1.
 * - Lengths are in PDF points (1/72 inch) in default user space.
 * - Errors from the document's side are thrown as {@link pdfi.PdfiError}
 *   with a {@link pdfi.ErrorCode}.
 * - Edits apply at once (later reads see them); the whole run is one undo
 *   step, reverted if the macro throws, is cancelled or times out. Macros
 *   do not save: the host does, as `meta.save` asks.
 *
 * This file is the reference: `pdfi macro types -o pdfi.d.ts` writes it
 * next to your macros for an editor's completion and `// @ts-check`.
 *
 * @packageDocumentation
 */

declare namespace pdfi {
  // ------------------------------------------------------------------ meta

  /** What `export const meta` declares. */
  interface Meta {
    /** The API version the macro is written for: `1`. */
    api: 1;
    /**
     * The name to run it by (`pdfi macro run <name>`): letters, digits,
     * `.`, `_` and `-`. Defaults to the file name without `.js`.
     */
    name?: string;
    /** A short title, also the label of the undo step. Defaults to the name. */
    title?: string;
    /** What it does, in a sentence or two. */
    description?: string;
    /**
     * Whether it changes the document. When true the command line wants
     * `-o`, `--out-dir` or `--in-place` (or `--dry-run`), and saves even if
     * nothing changed (as `remove-owner-password` needs). Default false.
     */
    edits?: boolean;
    /**
     * How the host should save the result: `"incremental"` (default:
     * appended to the unchanged file, signatures stay valid) or `"full"`
     * (every object written anew, decrypted, old revisions dropped), or a
     * full rewrite with options. Encrypted files are always written in full.
     */
    save?: "incremental" | "full" | FullSave;
    /** The parameters, by name, in the order a form shows them. */
    params?: Record<string, ParamSpec>;
  }

  interface FullSave {
    /** A cross-reference stream instead of a table. */
    xrefStream?: boolean;
    /** Objects other than streams in object streams. */
    objectStreams?: boolean;
    /** Flate-compress streams that have no filter. */
    compress?: boolean;
    /** Leave out objects the trailer does not reach. */
    removeUnused?: boolean;
  }

  /**
   * The type of a parameter and what the macro receives:
   *
   * | type | given as | received |
   * |---|---|---|
   * | `string` | text | `string` |
   * | `number`, `integer` | `3`, `"2.5"` | `number` |
   * | `boolean` | `true`, `"yes"`, `"0"`, ... | `boolean` |
   * | `enum` | one of `values` | `string` |
   * | `pages` | `"1-3,8"`, `"5-"` (1-based) | `number[]`, 0-based |
   * | `json` | JSON text or a JSON file | the parsed value |
   * | `text` | a file (`k=@notes.txt`) | its text (UTF-8) |
   * | `bytes` | a file | its bytes, `Uint8Array` |
   * | `pdf` | a PDF file | a read-only {@link Doc} |
   */
  type ParamType =
    | "string"
    | "number"
    | "integer"
    | "boolean"
    | "enum"
    | "pages"
    | "json"
    | "text"
    | "bytes"
    | "pdf";

  /**
   * One parameter. The host checks and converts the parameters before the
   * macro runs; there is no way to ask for input while it runs.
   */
  interface ParamSpec {
    type: ParamType;
    /** The choices of an `enum`. */
    values?: string[];
    /** Used when the parameter is not given (not for text, bytes, pdf). */
    default?: unknown;
    /** May be left out: then it is missing from `params`. */
    optional?: boolean;
    description?: string;
  }

  /**
   * The signature of the default export. `P` is the type of `params`:
   * `/** @type {pdfi.Main<{ margin: number }>} *\/`.
   *
   * The returned value goes back as JSON: a list of flat objects is shown
   * as a table, anything else as a tree. `undefined` is `null`; names,
   * strings and references turn into their text.
   */
  type Main<P = Record<string, any>> = (doc: Doc, params: P, ctx: Context) => Promise<unknown>;

  // ------------------------------------------------------------------ context

  /** The second channel of a macro: logs, progress and output files. */
  interface Context {
    /** A log line (`console.log` writes here too). */
    log(...values: unknown[]): void;
    /** A warning (`console.warn`). */
    warn(...values: unknown[]): void;
    /** Report progress: `done` of `total` steps. */
    progress(done: number, total: number, label?: string): void;
    /**
     * Whether the run was cancelled. Host calls throw `cancelled` by
     * themselves; check this in long loops that make none.
     */
    readonly cancelled: boolean;
    /**
     * Write an output file: `name` is a plain file name (no directories).
     * A string is written as UTF-8, an object as JSON. The command line
     * writes it to `--out-dir` or `--emit-dir`, with the input's name in
     * front in a batch.
     */
    emit(name: string, data: Uint8Array | ArrayBuffer | string | object, mime?: string): void;
    /** About the input: never its path. */
    readonly input: { readonly name?: string; readonly size?: number };
    /** What runs the macro. */
    readonly host: "cli" | "server" | "macos" | "web";
    /** The macro's meta, with defaults filled in. */
    readonly meta: Readonly<Required<Omit<Meta, "save">>> & { readonly save: Meta["save"] };
  }

  // ------------------------------------------------------------------ values

  /**
   * A PDF value as read: numbers, booleans, `null` and arrays are plain
   * JS values; names are {@link Name}, strings {@link PdfString},
   * references {@link Ref}, dictionaries and streams {@link Node}.
   */
  type Value = null | boolean | number | Name | PdfString | Ref | Node | Value[];

  /**
   * What can be written: the values read, JS strings (written as text
   * strings, not names), plain objects (dictionaries), `Uint8Array`
   * (hexadecimal strings), `Date` (PDF dates) and {@link Raw} PDF syntax.
   * A {@link Node} is written as its reference, so it must be an object.
   */
  type Writable =
    | null
    | undefined
    | boolean
    | number
    | string
    | Name
    | PdfString
    | Ref
    | Raw
    | Node
    | Uint8Array
    | Date
    | Writable[]
    | { [key: string]: Writable };

  /** A key: `"Type"` or `"/Type"`, or a {@link Name}. */
  type Key = string | Name;

  /** A name (`/Type`). */
  class Name {
    private constructor();
    /** The name's text, without `/` and `#xx` escapes. */
    readonly name: string;
    /** `/Name`. */
    toString(): string;
    toJSON(): string;
  }

  /** A string (`(text)` or `<hex>`). */
  class PdfString {
    private constructor();
    /** Decoded as a text string (UTF-16BE, UTF-8 with a BOM, else PDFDocEncoding). */
    readonly text: string;
    /** The bytes; `null` for one made by `pdf.str(text)`. */
    readonly bytes: Uint8Array | null;
    /** Only the first kilobyte was read (it cannot be written back). */
    readonly truncated: boolean;
    toString(): string;
    toJSON(): string;
  }

  /** A reference to an indirect object (`12 0 R`). */
  class Ref {
    private constructor();
    readonly num: number;
    readonly generation: number;
    /** `12 0 R`. */
    toString(): string;
    toJSON(): string;
    equals(other: unknown): boolean;
  }

  /** PDF syntax written as is (`pdf.raw("<< /S /GoTo >>")`). */
  class Raw {
    private constructor();
    readonly text: string;
  }

  /**
   * A dictionary or stream: an indirect object, or one inside another
   * (`catalog.dict("ViewerPreferences")`). It is read when asked and
   * always shows the document as it is now.
   */
  class Node {
    private constructor();
    readonly kind: "dict" | "stream";
    readonly isStream: boolean;
    /** The object's reference; `null` for a dictionary inside another object. */
    readonly ref: Ref | null;
    /** The keys, in file order. */
    keys(): Promise<string[]>;
    has(key: Key): Promise<boolean>;
    /** An entry as written (a reference stays a {@link Ref}); `undefined` if missing. */
    get(key: Key): Promise<Value | undefined>;
    /** Like {@link get}, a reference followed to its object. */
    resolve(key: Key): Promise<Value | undefined>;
    /** Every entry, references not followed. */
    entries(): Promise<[string, Value][]>;
    /**
     * The dictionary at `key` (a reference followed). Missing: `null`, or
     * a new empty one put there with `{ create: true }`. Throws
     * `badRequest` when the entry is something else.
     */
    dict(key: Key, options?: { create?: boolean }): Promise<Node | null>;
    /** Add or replace an entry. */
    set(key: Key, value: Writable): Promise<void>;
    /** Remove an entry; whether it was there. */
    remove(key: Key): Promise<boolean>;
    /**
     * A stream object's data: decoded (filters applied, decrypted) unless
     * `{ decoded: false }`. Image codecs (DCT, JPX, ...) stay encoded.
     */
    data(options?: { decoded?: boolean }): Promise<Uint8Array>;
    /**
     * Replace a stream object's decoded data (a string as UTF-8), Flate-
     * compressed unless `{ filter: "none" }`.
     */
    setData(data: Uint8Array | ArrayBuffer | string, options?: { filter?: "flate" | "none" }): Promise<void>;
  }

  /** A rectangle in default user space, `x0 <= x1`, `y0 <= y1`. */
  interface Rect {
    x0: number;
    y0: number;
    x1: number;
    y1: number;
  }

  /** A range of operations, `from` included, `to` not. */
  type OpRange = { from?: number; to?: number } | [number, number];

  // ------------------------------------------------------------------ documents

  /** An open document. */
  class Doc {
    private constructor();
    /** The file name. */
    readonly name: string;
    /** Pages now (it follows page edits). */
    readonly pageCount: number;
    /** The effective PDF version (`"1.7"`). */
    readonly version: string | null;
    readonly encryption: Encryption | null;
    /** Opened read-only (a `pdf` parameter): edits throw `forbidden`. */
    readonly readOnly: boolean;
    /** Has edits not saved yet. */
    readonly edited: boolean;
    readonly catalog: Node;
    readonly trailer: Node;
    /** An object by reference: a {@link Node} for dictionaries and streams. */
    object(ref: Ref | { num: number; generation?: number }): Promise<Value>;
    /** A page, 0-based. Throws `RangeError` outside the document. */
    page(index: number): Promise<Page>;
    /**
     * Pages in order: all of them, those given (0-based, as a `pages`
     * parameter gives them), or a range.
     */
    pages(which?: number[] | { from?: number; to?: number }): AsyncIterableIterator<Page>;
    /** The Info dictionary: text, or ISO 8601 for dates. */
    info(): Promise<Record<string, string>>;
    /**
     * Set Info entries (`null` removes one). Dates may be ISO 8601, PDF
     * dates or `Date`. The matching XMP properties follow unless
     * `{ syncXmp: false }`.
     */
    setInfo(fields: Record<string, string | Date | null | undefined>, options?: { syncXmp?: boolean }): Promise<void>;
    /** The catalog's XMP metadata, or `null`. */
    xmp(): Promise<Xmp | null>;
    /** The outline (bookmarks) as a tree. */
    outline(): Promise<OutlineItem[]>;
    /** Fonts found in the document (runs the inventory first). */
    fonts(): Promise<FontInfo[]>;
    /** Image XObjects found in the document (runs the inventory first). */
    images(): Promise<ImageInfo[]>;
    deletePages(pages: number | number[]): Promise<void>;
    /** Move pages so that they start at `to` in the result. */
    movePages(pages: number | number[], to: number): Promise<void>;
    /** Turn pages by a multiple of 90 degrees (clockwise). */
    rotatePages(pages: number | number[], by: number): Promise<void>;
    /** An empty page before `at` (default: at the end), A4 unless sized. */
    insertBlankPage(at?: number, size?: { width?: number; height?: number }): Promise<void>;
    /** Copy pages of another document (all by default) before `at` (default: at the end). */
    insertPages(from: Doc, options?: { pages?: number[]; at?: number }): Promise<void>;
    /**
     * Apply protocol `EditCommand`s as they are (values in PDF syntax).
     * Low level: other calls are made of this. Not covered by `meta.api`.
     */
    apply(commands: object[], label?: string): Promise<ApplyResult>;
    /**
     * The protocol, raw: a request and its JSON answer with binary parts.
     * Not covered by `meta.api`: it changes with the protocol.
     */
    readonly raw: { dispatch(request: object): Promise<{ body: any; parts: Uint8Array[] }> };
  }

  interface Encryption {
    filter: string;
    v: number;
    /** The revision of the standard security handler. */
    r: number;
    keyBits: number;
    streamMethod: string;
    stringMethod: string;
    /** The `/P` bits. */
    permissions: number;
    /** A password (or the empty one) opened it. */
    authenticated: boolean;
  }

  interface Xmp {
    ref: Ref;
    /** Simple properties, `dc:title` and the like. */
    properties: { name: string; namespace: string; value: string }[];
    /** The packet as text. */
    xml: string;
    error: string | null;
  }

  interface OutlineItem {
    ref: Ref;
    title: string;
    /** The page it goes to, 0-based, when it goes to one of this document. */
    page: number | null;
    /** Shown open (`null` when it has no children). */
    open: boolean | null;
    /** Where it goes, in the protocol's terms (camelCase). */
    link: any;
    /** 1 italic, 2 bold. */
    flags: number;
    /** RGB, 0 to 1. */
    color: number[] | null;
    children: OutlineItem[];
  }

  interface FontInfo {
    id: Ref;
    baseFont: string | null;
    subtype: string | null;
    encoding: string | null;
    /** `FontFile2`, `FontFile3/Type1C`, ...; `null` when not embedded. */
    embedded: string | null;
    fontFile: Ref | null;
    toUnicode: boolean;
    descendant: Ref | null;
    subset: boolean;
    /** One of the standard 14 fonts. */
    standard: boolean;
    /** Pages using it through their resources. */
    pageCount: number;
    /** The first of those pages, 0-based. */
    pages: number[];
  }

  interface ImageInfo {
    id: Ref;
    width: number | null;
    height: number | null;
    bits: number | null;
    colorSpace: string | null;
    filters: string[];
    /** Bytes as stored. */
    rawLen: number;
    imageMask: boolean;
    smask: Ref | null;
    mask: boolean;
    interpolate: boolean;
    pageCount: number;
    pages: number[];
  }

  interface ApplyResult {
    /** Objects added, in order. */
    created: Ref[];
    /** Object numbers changed, added or deleted. */
    changed: number[];
    trailerChanged: boolean;
    notes: string[];
    /** Schema problems of the changed objects. */
    issues: { severity: string; code: string; message: string }[];
  }

  // ------------------------------------------------------------------ pages

  /**
   * A page as it was when read: after page edits (deleting, moving,
   * inserting), read the pages again.
   */
  class Page {
    private constructor();
    /** 0-based. */
    readonly index: number;
    readonly ref: Ref | null;
    /** The page label (`"iv"`, `"A-1"`), if the document has labels. */
    readonly label: string | null;
    /** `/Rotate`, inherited. */
    readonly rotate: number;
    /** The visible area: the CropBox clipped to the MediaBox. */
    readonly size: Rect | null;
    readonly annotationCount: number;
    /** The page dictionary. */
    readonly node: Node;
    /** The page's content (all of `/Contents`). */
    readonly content: Content;
    /**
     * The boxes with inheritance and defaults resolved (a missing CropBox
     * is the MediaBox, ...), and where each comes from.
     */
    boxes(): Promise<Boxes>;
    /** Set a box on the page itself: `"crop"` or `"CropBox"`, ... */
    setBox(name: BoxName, rect: Rect | [number, number, number, number]): Promise<void>;
    /** Remove a box from the page itself; whether it was there. */
    removeBox(name: BoxName): Promise<boolean>;
    /** The resources the page can name. */
    resources(): Promise<{ category: string; name: string; id: Ref | null; detail: string }[]>;
    /** The annotations, in `/Annots` order (camelCase protocol `AnnotInfo`). */
    annotations(): Promise<any[]>;
  }

  type BoxName =
    | "media"
    | "crop"
    | "bleed"
    | "trim"
    | "art"
    | "MediaBox"
    | "CropBox"
    | "BleedBox"
    | "TrimBox"
    | "ArtBox";

  /** Where a page attribute comes from. */
  type Origin =
    | { kind: "page" }
    | { kind: "inherited"; from: Ref }
    | { kind: "default"; of: string }
    | { kind: "missing" };

  interface Boxes {
    media: Rect | null;
    crop: Rect | null;
    bleed: Rect | null;
    trim: Rect | null;
    art: Rect | null;
    origins: Record<"media" | "crop" | "bleed" | "trim" | "art", Origin>;
  }

  // ------------------------------------------------------------------ content

  /** The operations of a page or another content stream. */
  class Content {
    private constructor();
    /** Streams, size, problems, operator counts (camelCase `ContentSummary`). */
    summary(): Promise<any>;
    /** The number of operations. */
    count(): Promise<number>;
    /** Operations with their operands, decoded text and what they paint. */
    ops(range?: OpRange): AsyncIterableIterator<Op>;
    /** What the operations paint, in operation order. */
    elements(): AsyncIterableIterator<Element>;
    /**
     * The union of what is painted and shown (clipping paths and hidden
     * elements left out), or `null` when nothing is.
     */
    bounds(): Promise<Rect | null>;
    /** Operations as written, content stream syntax (one char per byte). */
    text(range?: OpRange): Promise<string>;
    /** Replace operations with those in `text` (empty: remove them). */
    replace(range: OpRange, text: string): Promise<void>;
    /** Insert the operations in `text` before operation `before`. */
    insert(before: number, text: string): Promise<void>;
  }

  interface Op {
    /** 0-based. */
    index: number;
    operator: string;
    /** `pathConstruction`, `textShowing`, ...; `unknown` for unknown operators. */
    category: string;
    /** In PDF syntax, shortened. */
    operands: string[];
    /** Nesting of `q`, `BT` and marked content. */
    depth: number;
    /** For an opener (`q`, `BT`, `BDC`, ...): its last operation. */
    end: number | null;
    /** Text shown, when the font tells it. */
    text: string | null;
    /** The resource it names, when an object. */
    resource: Ref | null;
    element: Element | null;
    issues: { severity: string; code: string; message: string }[];
  }

  interface Element {
    op: number;
    /** `fill`, `stroke`, `fillStroke`, `clip`, `text`, `image`, `inlineImage`, `form`, `shading`. */
    kind: string;
    bbox: Rect;
    /** Outside the clipping path, or invisible text. */
    hidden: boolean;
  }

  // ------------------------------------------------------------------ errors

  /** Why the document's side refused. */
  type ErrorCode =
    | "badRequest"
    | "notFound"
    | "forbidden"
    | "unsupported"
    | "tooLarge"
    | "notReady"
    | "busy"
    | "io"
    | "parse"
    | "cancelled"
    | "internal"
    | (string & {});

  class PdfiError extends Error {
    private constructor();
    readonly code: ErrorCode;
  }
}

/** Making PDF values and the classes of those read. */
declare const pdf: {
  /** The API version: 1. */
  readonly API: 1;
  /** A name: `pdf.name("TwoPageRight")` is `/TwoPageRight`. */
  name(name: string): pdfi.Name;
  /** A string: text (written as a text string) or bytes. */
  str(textOrBytes: string | Uint8Array): pdfi.PdfString;
  ref(num: number, generation?: number): pdfi.Ref;
  /** PDF syntax written as is. */
  raw(syntax: string): pdfi.Raw;
  /**
   * `"1-3,8"` (1-based, `"5-"` to the end) → 0-based page numbers; `count`
   * is the number of pages (needed for open ranges, and checked).
   */
  pages(spec: string, count?: number): number[];
  readonly Name: typeof pdfi.Name;
  readonly PdfString: typeof pdfi.PdfString;
  readonly Ref: typeof pdfi.Ref;
  readonly Node: typeof pdfi.Node;
  readonly Doc: typeof pdfi.Doc;
  readonly Page: typeof pdfi.Page;
  readonly Content: typeof pdfi.Content;
  readonly PdfiError: typeof pdfi.PdfiError;
};

// What the prelude provides beyond ECMAScript (declared as lib.dom does, so
// both can be used together).

interface Console {
  log(...data: any[]): void;
  info(...data: any[]): void;
  debug(...data: any[]): void;
  warn(...data: any[]): void;
  error(...data: any[]): void;
}
/** Writes to the macro's log (`warn` and `error` as such). */
declare var console: Console;

interface TextEncoder {
  readonly encoding: string;
  encode(input?: string): Uint8Array;
}
/** UTF-8. */
declare var TextEncoder: {
  prototype: TextEncoder;
  new (): TextEncoder;
};

interface TextDecodeOptions {
  stream?: boolean;
}
interface TextDecoderOptions {
  fatal?: boolean;
  ignoreBOM?: boolean;
}
interface TextDecoder {
  readonly encoding: string;
  readonly fatal: boolean;
  readonly ignoreBOM: boolean;
  decode(input?: ArrayBufferLike | ArrayBufferView, options?: TextDecodeOptions): string;
}
/** `utf-8`, `utf-16le`, `utf-16be` and `latin1` (`windows-1252`) in QuickJS. */
declare var TextDecoder: {
  prototype: TextDecoder;
  new (label?: string, options?: TextDecoderOptions): TextDecoder;
};
