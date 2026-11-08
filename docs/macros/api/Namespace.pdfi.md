# pdfi

## Classes

### Name

A name (`/Type`).

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-name-1"></a> `name` | `readonly` | `string` | The name's text, without `/` and `#xx` escapes. |

#### Methods

##### toString()

```ts
toString(): string;
```

`/Name`.

###### Returns

`string`

##### toJSON()

```ts
toJSON(): string;
```

###### Returns

`string`

***

### PdfString

A string (`(text)` or `<hex>`).

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-text"></a> `text` | `readonly` | `string` | Decoded as a text string (UTF-16BE, UTF-8 with a BOM, else PDFDocEncoding). |
| <a id="property-bytes"></a> `bytes` | `readonly` | `Uint8Array`\<`ArrayBufferLike`\> \| `null` | The bytes; `null` for one made by `pdf.str(text)`. |
| <a id="property-truncated"></a> `truncated` | `readonly` | `boolean` | Only the first kilobyte was read (it cannot be written back). |

#### Methods

##### toString()

```ts
toString(): string;
```

###### Returns

`string`

##### toJSON()

```ts
toJSON(): string;
```

###### Returns

`string`

***

### Ref

A reference to an indirect object (`12 0 R`).

#### Properties

| Property | Modifier | Type |
| ------ | ------ | ------ |
| <a id="property-num"></a> `num` | `readonly` | `number` |
| <a id="property-generation"></a> `generation` | `readonly` | `number` |

#### Methods

##### toString()

```ts
toString(): string;
```

`12 0 R`.

###### Returns

`string`

##### toJSON()

```ts
toJSON(): string;
```

###### Returns

`string`

##### equals()

```ts
equals(other: unknown): boolean;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `other` | `unknown` |

###### Returns

`boolean`

***

### Raw

PDF syntax written as is (`pdf.raw("<< /S /GoTo >>")`).

#### Properties

| Property | Modifier | Type |
| ------ | ------ | ------ |
| <a id="property-text-1"></a> `text` | `readonly` | `string` |

***

### Node

A dictionary or stream: an indirect object, or one inside another
(`catalog.dict("ViewerPreferences")`). It is read when asked and
always shows the document as it is now.

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-kind"></a> `kind` | `readonly` | `"dict"` \| `"stream"` | - |
| <a id="property-isstream"></a> `isStream` | `readonly` | `boolean` | - |
| <a id="property-ref"></a> `ref` | `readonly` | [`Ref`](#ref) \| `null` | The object's reference; `null` for a dictionary inside another object. |

#### Methods

##### keys()

```ts
keys(): Promise<string[]>;
```

The keys, in file order.

###### Returns

`Promise`\<`string`[]\>

##### has()

```ts
has(key: Key): Promise<boolean>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |

###### Returns

`Promise`\<`boolean`\>

##### get()

```ts
get(key: Key): Promise<Value | undefined>;
```

An entry as written (a reference stays a [Ref](#ref)); `undefined` if missing.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |

###### Returns

`Promise`\<[`Value`](#value) \| `undefined`\>

##### resolve()

```ts
resolve(key: Key): Promise<Value | undefined>;
```

Like [get](#get), a reference followed to its object.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |

###### Returns

`Promise`\<[`Value`](#value) \| `undefined`\>

##### entries()

```ts
entries(): Promise<[string, Value][]>;
```

Every entry, references not followed.

###### Returns

`Promise`\<\[`string`, [`Value`](#value)\][]\>

##### dict()

```ts
dict(key: Key, options?: object): Promise<Node | null>;
```

The dictionary at `key` (a reference followed). Missing: `null`, or
a new empty one put there with `{ create: true }`. Throws
`badRequest` when the entry is something else.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |
| `options?` | \{ `create?`: `boolean`; \} |
| `options.create?` | `boolean` |

###### Returns

`Promise`\<[`Node`](#node) \| `null`\>

##### set()

```ts
set(key: Key, value: Writable): Promise<void>;
```

Add or replace an entry.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |
| `value` | [`Writable`](#writable) |

###### Returns

`Promise`\<`void`\>

##### remove()

```ts
remove(key: Key): Promise<boolean>;
```

Remove an entry; whether it was there.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `key` | [`Key`](#key) |

###### Returns

`Promise`\<`boolean`\>

##### data()

```ts
data(options?: object): Promise<Uint8Array<ArrayBufferLike>>;
```

A stream object's data: decoded (filters applied, decrypted) unless
`{ decoded: false }`. Image codecs (DCT, JPX, ...) stay encoded.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `options?` | \{ `decoded?`: `boolean`; \} |
| `options.decoded?` | `boolean` |

###### Returns

`Promise`\<`Uint8Array`\<`ArrayBufferLike`\>\>

##### setData()

```ts
setData(data: string | ArrayBuffer | Uint8Array<ArrayBufferLike>, options?: object): Promise<void>;
```

Replace a stream object's decoded data (a string as UTF-8), Flate-
compressed unless `{ filter: "none" }`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `data` | `string` \| `ArrayBuffer` \| `Uint8Array`\<`ArrayBufferLike`\> |
| `options?` | \{ `filter?`: `"none"` \| `"flate"`; \} |
| `options.filter?` | `"none"` \| `"flate"` |

###### Returns

`Promise`\<`void`\>

***

### Doc

An open document.

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-name-2"></a> `name` | `readonly` | `string` | The file name. |
| <a id="property-pagecount"></a> `pageCount` | `readonly` | `number` | Pages now (it follows page edits). |
| <a id="property-version"></a> `version` | `readonly` | `string` \| `null` | The effective PDF version (`"1.7"`). |
| <a id="property-encryption"></a> `encryption` | `readonly` | [`Encryption`](#encryption) \| `null` | - |
| <a id="property-readonly"></a> `readOnly` | `readonly` | `boolean` | Opened read-only (a `pdf` parameter): edits throw `forbidden`. |
| <a id="property-edited"></a> `edited` | `readonly` | `boolean` | Has edits not saved yet. |
| <a id="property-catalog"></a> `catalog` | `readonly` | [`Node`](#node) | - |
| <a id="property-trailer"></a> `trailer` | `readonly` | [`Node`](#node) | - |
| <a id="property-raw"></a> `raw` | `readonly` | `object` | The protocol, raw: a request and its JSON answer with binary parts. Not covered by `meta.api`: it changes with the protocol. |
| `raw.dispatch` | `public` | (`request`: `object`) => `Promise`\<\{ `body`: `any`; `parts`: `Uint8Array`\<`ArrayBufferLike`\>[]; \}\> | - |

#### Methods

##### object()

```ts
object(ref: 
  | Ref
  | {
  num: number;
  generation?: number;
}): Promise<Value>;
```

An object by reference: a [Node](#node) for dictionaries and streams.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `ref` | \| [`Ref`](#ref) \| \{ `num`: `number`; `generation?`: `number`; \} |

###### Returns

`Promise`\<[`Value`](#value)\>

##### page()

```ts
page(index: number): Promise<Page>;
```

A page, 0-based. Throws `RangeError` outside the document.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `index` | `number` |

###### Returns

`Promise`\<[`Page`](#page-1)\>

##### pages()

```ts
pages(which?: 
  | number[]
  | {
  from?: number;
  to?: number;
}): AsyncIterableIterator<Page>;
```

Pages in order: all of them, those given (0-based, as a `pages`
parameter gives them), or a range.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `which?` | \| `number`[] \| \{ `from?`: `number`; `to?`: `number`; \} |

###### Returns

`AsyncIterableIterator`\<[`Page`](#page-1)\>

##### info()

```ts
info(): Promise<Record<string, string>>;
```

The Info dictionary: text, or ISO 8601 for dates.

###### Returns

`Promise`\<`Record`\<`string`, `string`\>\>

##### setInfo()

```ts
setInfo(fields: Record<string, string | Date | null | undefined>, options?: object): Promise<void>;
```

Set Info entries (`null` removes one). Dates may be ISO 8601, PDF
dates or `Date`. The matching XMP properties follow unless
`{ syncXmp: false }`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `fields` | `Record`\<`string`, `string` \| `Date` \| `null` \| `undefined`\> |
| `options?` | \{ `syncXmp?`: `boolean`; \} |
| `options.syncXmp?` | `boolean` |

###### Returns

`Promise`\<`void`\>

##### xmp()

```ts
xmp(): Promise<Xmp | null>;
```

The catalog's XMP metadata, or `null`.

###### Returns

`Promise`\<[`Xmp`](#xmp-1) \| `null`\>

##### outline()

```ts
outline(): Promise<OutlineItem[]>;
```

The outline (bookmarks) as a tree.

###### Returns

`Promise`\<[`OutlineItem`](#outlineitem)[]\>

##### fonts()

```ts
fonts(): Promise<FontInfo[]>;
```

Fonts found in the document (runs the inventory first).

###### Returns

`Promise`\<[`FontInfo`](#fontinfo)[]\>

##### images()

```ts
images(): Promise<ImageInfo[]>;
```

Image XObjects found in the document (runs the inventory first).

###### Returns

`Promise`\<[`ImageInfo`](#imageinfo)[]\>

##### deletePages()

```ts
deletePages(pages: number | number[]): Promise<void>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `pages` | `number` \| `number`[] |

###### Returns

`Promise`\<`void`\>

##### movePages()

```ts
movePages(pages: number | number[], to: number): Promise<void>;
```

Move pages so that they start at `to` in the result.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `pages` | `number` \| `number`[] |
| `to` | `number` |

###### Returns

`Promise`\<`void`\>

##### rotatePages()

```ts
rotatePages(pages: number | number[], by: number): Promise<void>;
```

Turn pages by a multiple of 90 degrees (clockwise).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `pages` | `number` \| `number`[] |
| `by` | `number` |

###### Returns

`Promise`\<`void`\>

##### insertBlankPage()

```ts
insertBlankPage(at?: number, size?: object): Promise<void>;
```

An empty page before `at` (default: at the end), A4 unless sized.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `at?` | `number` |
| `size?` | \{ `width?`: `number`; `height?`: `number`; \} |
| `size.width?` | `number` |
| `size.height?` | `number` |

###### Returns

`Promise`\<`void`\>

##### insertPages()

```ts
insertPages(from: Doc, options?: object): Promise<void>;
```

Copy pages of another document (all by default) before `at` (default: at the end).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `from` | [`Doc`](#doc) |
| `options?` | \{ `pages?`: `number`[]; `at?`: `number`; \} |
| `options.pages?` | `number`[] |
| `options.at?` | `number` |

###### Returns

`Promise`\<`void`\>

##### apply()

```ts
apply(commands: object[], label?: string): Promise<ApplyResult>;
```

Apply protocol `EditCommand`s as they are (values in PDF syntax).
Low level: other calls are made of this. Not covered by `meta.api`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `commands` | `object`[] |
| `label?` | `string` |

###### Returns

`Promise`\<[`ApplyResult`](#applyresult)\>

***

### Page

A page as it was when read: after page edits (deleting, moving,
inserting), read the pages again.

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-index"></a> `index` | `readonly` | `number` | 0-based. |
| <a id="property-ref-3"></a> `ref` | `readonly` | [`Ref`](#ref) \| `null` | - |
| <a id="property-label"></a> `label` | `readonly` | `string` \| `null` | The page label (`"iv"`, `"A-1"`), if the document has labels. |
| <a id="property-rotate"></a> `rotate` | `readonly` | `number` | `/Rotate`, inherited. |
| <a id="property-size"></a> `size` | `readonly` | [`Rect`](#rect) \| `null` | The visible area: the CropBox clipped to the MediaBox. |
| <a id="property-annotationcount"></a> `annotationCount` | `readonly` | `number` | - |
| <a id="property-node"></a> `node` | `readonly` | [`Node`](#node) | The page dictionary. |
| <a id="property-content"></a> `content` | `readonly` | [`Content`](#content) | The page's content (all of `/Contents`). |

#### Methods

##### boxes()

```ts
boxes(): Promise<Boxes>;
```

The boxes with inheritance and defaults resolved (a missing CropBox
is the MediaBox, ...), and where each comes from.

###### Returns

`Promise`\<[`Boxes`](#boxes-1)\>

##### setBox()

```ts
setBox(name: BoxName, rect: Rect | [number, number, number, number]): Promise<void>;
```

Set a box on the page itself: `"crop"` or `"CropBox"`, ...

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `name` | [`BoxName`](#boxname) |
| `rect` | [`Rect`](#rect) \| \[`number`, `number`, `number`, `number`\] |

###### Returns

`Promise`\<`void`\>

##### removeBox()

```ts
removeBox(name: BoxName): Promise<boolean>;
```

Remove a box from the page itself; whether it was there.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `name` | [`BoxName`](#boxname) |

###### Returns

`Promise`\<`boolean`\>

##### resources()

```ts
resources(): Promise<object[]>;
```

The resources the page can name.

###### Returns

`Promise`\<`object`[]\>

##### annotations()

```ts
annotations(): Promise<any[]>;
```

The annotations, in `/Annots` order (camelCase protocol `AnnotInfo`).

###### Returns

`Promise`\<`any`[]\>

***

### Content

The operations of a page or another content stream.

#### Methods

##### summary()

```ts
summary(): Promise<any>;
```

Streams, size, problems, operator counts (camelCase `ContentSummary`).

###### Returns

`Promise`\<`any`\>

##### count()

```ts
count(): Promise<number>;
```

The number of operations.

###### Returns

`Promise`\<`number`\>

##### ops()

```ts
ops(range?: OpRange): AsyncIterableIterator<Op>;
```

Operations with their operands, decoded text and what they paint.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `range?` | [`OpRange`](#oprange) |

###### Returns

`AsyncIterableIterator`\<[`Op`](#op)\>

##### elements()

```ts
elements(): AsyncIterableIterator<Element>;
```

What the operations paint, in operation order.

###### Returns

`AsyncIterableIterator`\<[`Element`](#element)\>

##### bounds()

```ts
bounds(): Promise<Rect | null>;
```

The union of what is painted and shown (clipping paths and hidden
elements left out), or `null` when nothing is.

###### Returns

`Promise`\<[`Rect`](#rect) \| `null`\>

##### text()

```ts
text(range?: OpRange): Promise<string>;
```

Operations as written, content stream syntax (one char per byte).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `range?` | [`OpRange`](#oprange) |

###### Returns

`Promise`\<`string`\>

##### replace()

```ts
replace(range: OpRange, text: string): Promise<void>;
```

Replace operations with those in `text` (empty: remove them).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `range` | [`OpRange`](#oprange) |
| `text` | `string` |

###### Returns

`Promise`\<`void`\>

##### insert()

```ts
insert(before: number, text: string): Promise<void>;
```

Insert the operations in `text` before operation `before`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `before` | `number` |
| `text` | `string` |

###### Returns

`Promise`\<`void`\>

***

### PdfiError

#### Extends

- `Error`

#### Properties

| Property | Modifier | Type |
| ------ | ------ | ------ |
| <a id="property-code"></a> `code` | `readonly` | [`ErrorCode`](#errorcode) |

## Interfaces

### Meta

What `export const meta` declares.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-api"></a> `api` | `1` | The API version the macro is written for: `1`. |
| <a id="property-name"></a> `name?` | `string` | The name to run it by (`pdfi macro run <name>`): letters, digits, `.`, `_` and `-`. Defaults to the file name without `.js`. |
| <a id="property-title"></a> `title?` | `string` | A short title, also the label of the undo step. Defaults to the name. |
| <a id="property-description"></a> `description?` | `string` | What it does, in a sentence or two. |
| <a id="property-edits"></a> `edits?` | `boolean` | Whether it changes the document. When true the command line wants `-o`, `--out-dir` or `--in-place` (or `--dry-run`), and saves even if nothing changed (as `remove-owner-password` needs). Default false. |
| <a id="property-save"></a> `save?` | `"full"` \| [`FullSave`](#fullsave) \| `"incremental"` | How the host should save the result: `"incremental"` (default: appended to the unchanged file, signatures stay valid) or `"full"` (every object written anew, decrypted, old revisions dropped), or a full rewrite with options. Encrypted files are always written in full. |
| <a id="property-params"></a> `params?` | `Record`\<`string`, [`ParamSpec`](#paramspec)\> | The parameters, by name, in the order a form shows them. |

***

### FullSave

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-xrefstream"></a> `xrefStream?` | `boolean` | A cross-reference stream instead of a table. |
| <a id="property-objectstreams"></a> `objectStreams?` | `boolean` | Objects other than streams in object streams. |
| <a id="property-compress"></a> `compress?` | `boolean` | Flate-compress streams that have no filter. |
| <a id="property-removeunused"></a> `removeUnused?` | `boolean` | Leave out objects the trailer does not reach. |

***

### ParamSpec

One parameter. The host checks and converts the parameters before the
macro runs; there is no way to ask for input while it runs.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-type"></a> `type` | [`ParamType`](#paramtype) | - |
| <a id="property-values"></a> `values?` | `string`[] | The choices of an `enum`. |
| <a id="property-default"></a> `default?` | `unknown` | Used when the parameter is not given (not for text, bytes, pdf). |
| <a id="property-optional"></a> `optional?` | `boolean` | May be left out: then it is missing from `params`. |
| <a id="property-description-1"></a> `description?` | `string` | - |

***

### Context

The second channel of a macro: logs, progress and output files.

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-cancelled"></a> `cancelled` | `readonly` | `boolean` | Whether the run was cancelled. Host calls throw `cancelled` by themselves; check this in long loops that make none. |
| <a id="property-input"></a> `input` | `readonly` | `object` | About the input: never its path. |
| `input.name?` | `readonly` | `string` | - |
| `input.size?` | `readonly` | `number` | - |
| <a id="property-host"></a> `host` | `readonly` | `"cli"` \| `"server"` \| `"macos"` \| `"web"` | What runs the macro. |
| <a id="property-meta"></a> `meta` | `readonly` | `Readonly`\<`Required`\<`Omit`\<[`Meta`](#meta), `"save"`\>\>\> & `object` | The macro's meta, with defaults filled in. |

#### Methods

##### log()

```ts
log(...values: unknown[]): void;
```

A log line (`console.log` writes here too).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`values` | `unknown`[] |

###### Returns

`void`

##### warn()

```ts
warn(...values: unknown[]): void;
```

A warning (`console.warn`).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`values` | `unknown`[] |

###### Returns

`void`

##### progress()

```ts
progress(
   done: number, 
   total: number, 
   label?: string
): void;
```

Report progress: `done` of `total` steps.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `done` | `number` |
| `total` | `number` |
| `label?` | `string` |

###### Returns

`void`

##### emit()

```ts
emit(
   name: string, 
   data: string | object | ArrayBuffer | Uint8Array<ArrayBufferLike>, 
   mime?: string
): void;
```

Write an output file: `name` is a plain file name (no directories).
A string is written as UTF-8, an object as JSON. The command line
writes it to `--out-dir` or `--emit-dir`, with the input's name in
front in a batch.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `name` | `string` |
| `data` | `string` \| `object` \| `ArrayBuffer` \| `Uint8Array`\<`ArrayBufferLike`\> |
| `mime?` | `string` |

###### Returns

`void`

***

### Rect

A rectangle in default user space, `x0 <= x1`, `y0 <= y1`.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-x0"></a> `x0` | `number` |
| <a id="property-y0"></a> `y0` | `number` |
| <a id="property-x1"></a> `x1` | `number` |
| <a id="property-y1"></a> `y1` | `number` |

***

### Encryption

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-filter"></a> `filter` | `string` | - |
| <a id="property-v"></a> `v` | `number` | - |
| <a id="property-r"></a> `r` | `number` | The revision of the standard security handler. |
| <a id="property-keybits"></a> `keyBits` | `number` | - |
| <a id="property-streammethod"></a> `streamMethod` | `string` | - |
| <a id="property-stringmethod"></a> `stringMethod` | `string` | - |
| <a id="property-permissions"></a> `permissions` | `number` | The `/P` bits. |
| <a id="property-authenticated"></a> `authenticated` | `boolean` | A password (or the empty one) opened it. |

***

### Xmp

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-ref-1"></a> `ref` | [`Ref`](#ref) | - |
| <a id="property-properties"></a> `properties` | `object`[] | Simple properties, `dc:title` and the like. |
| <a id="property-xml"></a> `xml` | `string` | The packet as text. |
| <a id="property-error"></a> `error` | `string` \| `null` | - |

***

### OutlineItem

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-ref-2"></a> `ref` | [`Ref`](#ref) | - |
| <a id="property-title-1"></a> `title` | `string` | - |
| <a id="property-page"></a> `page` | `number` \| `null` | The page it goes to, 0-based, when it goes to one of this document. |
| <a id="property-open"></a> `open` | `boolean` \| `null` | Shown open (`null` when it has no children). |
| <a id="property-link"></a> `link` | `any` | Where it goes, in the protocol's terms (camelCase). |
| <a id="property-flags"></a> `flags` | `number` | 1 italic, 2 bold. |
| <a id="property-color"></a> `color` | `number`[] \| `null` | RGB, 0 to 1. |
| <a id="property-children"></a> `children` | [`OutlineItem`](#outlineitem)[] | - |

***

### FontInfo

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-id"></a> `id` | [`Ref`](#ref) | - |
| <a id="property-basefont"></a> `baseFont` | `string` \| `null` | - |
| <a id="property-subtype"></a> `subtype` | `string` \| `null` | - |
| <a id="property-encoding"></a> `encoding` | `string` \| `null` | - |
| <a id="property-embedded"></a> `embedded` | `string` \| `null` | `FontFile2`, `FontFile3/Type1C`, ...; `null` when not embedded. |
| <a id="property-fontfile"></a> `fontFile` | [`Ref`](#ref) \| `null` | - |
| <a id="property-tounicode"></a> `toUnicode` | `boolean` | - |
| <a id="property-descendant"></a> `descendant` | [`Ref`](#ref) \| `null` | - |
| <a id="property-subset"></a> `subset` | `boolean` | - |
| <a id="property-standard"></a> `standard` | `boolean` | One of the standard 14 fonts. |
| <a id="property-pagecount-1"></a> `pageCount` | `number` | Pages using it through their resources. |
| <a id="property-pages"></a> `pages` | `number`[] | The first of those pages, 0-based. |

***

### ImageInfo

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-id-1"></a> `id` | [`Ref`](#ref) | - |
| <a id="property-width"></a> `width` | `number` \| `null` | - |
| <a id="property-height"></a> `height` | `number` \| `null` | - |
| <a id="property-bits"></a> `bits` | `number` \| `null` | - |
| <a id="property-colorspace"></a> `colorSpace` | `string` \| `null` | - |
| <a id="property-filters"></a> `filters` | `string`[] | - |
| <a id="property-rawlen"></a> `rawLen` | `number` | Bytes as stored. |
| <a id="property-imagemask"></a> `imageMask` | `boolean` | - |
| <a id="property-smask"></a> `smask` | [`Ref`](#ref) \| `null` | - |
| <a id="property-mask"></a> `mask` | `boolean` | - |
| <a id="property-interpolate"></a> `interpolate` | `boolean` | - |
| <a id="property-pagecount-2"></a> `pageCount` | `number` | - |
| <a id="property-pages-1"></a> `pages` | `number`[] | - |

***

### ApplyResult

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-created"></a> `created` | [`Ref`](#ref)[] | Objects added, in order. |
| <a id="property-changed"></a> `changed` | `number`[] | Object numbers changed, added or deleted. |
| <a id="property-trailerchanged"></a> `trailerChanged` | `boolean` | - |
| <a id="property-notes"></a> `notes` | `string`[] | - |
| <a id="property-issues"></a> `issues` | `object`[] | Schema problems of the changed objects. |

***

### Boxes

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-media"></a> `media` | [`Rect`](#rect) \| `null` |
| <a id="property-crop"></a> `crop` | [`Rect`](#rect) \| `null` |
| <a id="property-bleed"></a> `bleed` | [`Rect`](#rect) \| `null` |
| <a id="property-trim"></a> `trim` | [`Rect`](#rect) \| `null` |
| <a id="property-art"></a> `art` | [`Rect`](#rect) \| `null` |
| <a id="property-origins"></a> `origins` | `Record`\<`"media"` \| `"crop"` \| `"bleed"` \| `"trim"` \| `"art"`, [`Origin`](#origin)\> |

***

### Op

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-index-1"></a> `index` | `number` | 0-based. |
| <a id="property-operator"></a> `operator` | `string` | - |
| <a id="property-category"></a> `category` | `string` | `pathConstruction`, `textShowing`, ...; `unknown` for unknown operators. |
| <a id="property-operands"></a> `operands` | `string`[] | In PDF syntax, shortened. |
| <a id="property-depth"></a> `depth` | `number` | Nesting of `q`, `BT` and marked content. |
| <a id="property-end"></a> `end` | `number` \| `null` | For an opener (`q`, `BT`, `BDC`, ...): its last operation. |
| <a id="property-text-2"></a> `text` | `string` \| `null` | Text shown, when the font tells it. |
| <a id="property-resource"></a> `resource` | [`Ref`](#ref) \| `null` | The resource it names, when an object. |
| <a id="property-element"></a> `element` | [`Element`](#element) \| `null` | - |
| <a id="property-issues-1"></a> `issues` | `object`[] | - |

***

### Element

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-op"></a> `op` | `number` | - |
| <a id="property-kind-1"></a> `kind` | `string` | `fill`, `stroke`, `fillStroke`, `clip`, `text`, `image`, `inlineImage`, `form`, `shading`. |
| <a id="property-bbox"></a> `bbox` | [`Rect`](#rect) | - |
| <a id="property-hidden"></a> `hidden` | `boolean` | Outside the clipping path, or invisible text. |

## Type Aliases

### ParamType

```ts
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
```

The type of a parameter and what the macro receives:

| type | given as | received |
|---|---|---|
| `string` | text | `string` |
| `number`, `integer` | `3`, `"2.5"` | `number` |
| `boolean` | `true`, `"yes"`, `"0"`, ... | `boolean` |
| `enum` | one of `values` | `string` |
| `pages` | `"1-3,8"`, `"5-"` (1-based) | `number[]`, 0-based |
| `json` | JSON text or a JSON file | the parsed value |
| `text` | a file (`k=@notes.txt`) | its text (UTF-8) |
| `bytes` | a file | its bytes, `Uint8Array` |
| `pdf` | a PDF file | a read-only [Doc](#doc) |

***

### Main

```ts
type Main<P = Record<string, any>> = (doc: Doc, params: P, ctx: Context) => Promise<unknown>;
```

The signature of the default export. `P` is the type of `params`:
`/** @type {pdfi.Main<{ margin: number }>} */`.

The returned value goes back as JSON: a list of flat objects is shown
as a table, anything else as a tree. `undefined` is `null`; names,
strings and references turn into their text.

#### Type Parameters

| Type Parameter | Default type |
| ------ | ------ |
| <a id="type-parameter-p"></a> `P` | `Record`\<`string`, `any`\> |

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `doc` | [`Doc`](#doc) |
| `params` | `P` |
| `ctx` | [`Context`](#context) |

#### Returns

`Promise`\<`unknown`\>

***

### Value

```ts
type Value = 
  | null
  | boolean
  | number
  | Name
  | PdfString
  | Ref
  | Node
  | Value[];
```

A PDF value as read: numbers, booleans, `null` and arrays are plain
JS values; names are [Name](#name), strings [PdfString](#pdfstring),
references [Ref](#ref), dictionaries and streams [Node](#node).

***

### Writable

```ts
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
  | {
[key: string]: Writable;
};
```

What can be written: the values read, JS strings (written as text
strings, not names), plain objects (dictionaries), `Uint8Array`
(hexadecimal strings), `Date` (PDF dates) and [Raw](#raw) PDF syntax.
A [Node](#node) is written as its reference, so it must be an object.

***

### Key

```ts
type Key = string | Name;
```

A key: `"Type"` or `"/Type"`, or a [Name](#name).

***

### OpRange

```ts
type OpRange = 
  | {
  from?: number;
  to?: number;
}
  | [number, number];
```

A range of operations, `from` included, `to` not.

***

### BoxName

```ts
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
```

***

### Origin

```ts
type Origin = 
  | {
  kind: "page";
}
  | {
  kind: "inherited";
  from: Ref;
}
  | {
  kind: "default";
  of: string;
}
  | {
  kind: "missing";
};
```

Where a page attribute comes from.

***

### ErrorCode

```ts
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
  | string & object;
```

Why the document's side refused.
