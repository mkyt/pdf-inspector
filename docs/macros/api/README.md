# pdfi macro API

The API of pdf-inspector macros (`meta.api: 1`).

A macro is one JavaScript module (no `import`). It exports `meta`, which
says what it is and which parameters it takes, and a default async
function that does the work:

```js
// @ts-check
/// <reference path="pdfi.d.ts" />
export const meta = { api: 1, name: "page-count", title: "Page count", edits: false };

/** @type {pdfi.Main} */
export default async function (doc, params, ctx) {
  return { pages: doc.pageCount };
}
```

Conventions of the whole API:

- Everything that reads or changes the document is `async`.
- Page numbers, operation numbers and indices are **0-based**. Only what
  people write (a `pages` parameter, the command line) counts from 1.
- Lengths are in PDF points (1/72 inch) in default user space.
- Errors from the document's side are thrown as [pdfi.PdfiError](Namespace.pdfi.md#pdfierror)
  with a [pdfi.ErrorCode](Namespace.pdfi.md#errorcode).
- Edits apply at once (later reads see them); the whole run is one undo
  step, reverted if the macro throws, is cancelled or times out. Macros
  do not save: the host does, as `meta.save` asks.

This file is the reference: `pdfi macro types -o pdfi.d.ts` writes it
next to your macros for an editor's completion and `// @ts-check`.

## Namespaces

- [pdfi](Namespace.pdfi.md)

## Interfaces

### Console

#### Methods

##### log()

```ts
log(...data: any[]): void;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`data` | `any`[] |

###### Returns

`void`

##### info()

```ts
info(...data: any[]): void;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`data` | `any`[] |

###### Returns

`void`

##### debug()

```ts
debug(...data: any[]): void;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`data` | `any`[] |

###### Returns

`void`

##### warn()

```ts
warn(...data: any[]): void;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`data` | `any`[] |

###### Returns

`void`

##### error()

```ts
error(...data: any[]): void;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| ...`data` | `any`[] |

###### Returns

`void`

***

### TextEncoder

#### Properties

| Property | Modifier | Type |
| ------ | ------ | ------ |
| <a id="property-encoding"></a> `encoding` | `readonly` | `string` |

#### Methods

##### encode()

```ts
encode(input?: string): Uint8Array;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `input?` | `string` |

###### Returns

`Uint8Array`

***

### TextDecodeOptions

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-stream"></a> `stream?` | `boolean` |

***

### TextDecoderOptions

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-fatal"></a> `fatal?` | `boolean` |
| <a id="property-ignorebom"></a> `ignoreBOM?` | `boolean` |

***

### TextDecoder

#### Properties

| Property | Modifier | Type |
| ------ | ------ | ------ |
| <a id="property-encoding-1"></a> `encoding` | `readonly` | `string` |
| <a id="property-fatal-1"></a> `fatal` | `readonly` | `boolean` |
| <a id="property-ignorebom-1"></a> `ignoreBOM` | `readonly` | `boolean` |

#### Methods

##### decode()

```ts
decode(input?: ArrayBufferLike | ArrayBufferView<ArrayBufferLike>, options?: TextDecodeOptions): string;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `input?` | `ArrayBufferLike` \| `ArrayBufferView`\<`ArrayBufferLike`\> |
| `options?` | [`TextDecodeOptions`](#textdecodeoptions) |

###### Returns

`string`

## Variables

### pdf

```ts
const pdf: object;
```

Making PDF values and the classes of those read.

#### Type Declaration

| Name | Type | Description |
| ------ | ------ | ------ |
| <a id="property-api"></a> `API` | `1` | The API version: 1. |
| `name()` | (`name`: `string`) => [`Name`](Namespace.pdfi.md#name) | A name: `pdf.name("TwoPageRight")` is `/TwoPageRight`. |
| `str()` | (`textOrBytes`: `string` \| `Uint8Array`\<`ArrayBufferLike`\>) => [`PdfString`](Namespace.pdfi.md#pdfstring) | A string: text (written as a text string) or bytes. |
| `ref()` | (`num`: `number`, `generation?`: `number`) => [`Ref`](Namespace.pdfi.md#ref) | - |
| `raw()` | (`syntax`: `string`) => [`Raw`](Namespace.pdfi.md#raw) | PDF syntax written as is. |
| `pages()` | (`spec`: `string`, `count?`: `number`) => `number`[] | `"1-3,8"` (1-based, `"5-"` to the end) → 0-based page numbers; `count` is the number of pages (needed for open ranges, and checked). |
| <a id="property-name"></a> `Name` | *typeof* [`Name`](Namespace.pdfi.md#name) | - |
| <a id="property-pdfstring"></a> `PdfString` | *typeof* [`PdfString`](Namespace.pdfi.md#pdfstring) | - |
| <a id="property-ref"></a> `Ref` | *typeof* [`Ref`](Namespace.pdfi.md#ref) | - |
| <a id="property-node"></a> `Node` | *typeof* [`Node`](Namespace.pdfi.md#node) | - |
| <a id="property-doc"></a> `Doc` | *typeof* [`Doc`](Namespace.pdfi.md#doc) | - |
| <a id="property-page"></a> `Page` | *typeof* [`Page`](Namespace.pdfi.md#page-1) | - |
| <a id="property-content"></a> `Content` | *typeof* [`Content`](Namespace.pdfi.md#content) | - |
| <a id="property-pdfierror"></a> `PdfiError` | *typeof* [`PdfiError`](Namespace.pdfi.md#pdfierror) | - |

***

### console

```ts
console: Console;
```

Writes to the macro's log (`warn` and `error` as such).

***

### TextEncoder

```ts
TextEncoder: {
new (): TextEncoder;
  prototype: TextEncoder;
};
```

UTF-8.

#### Type Declaration

#### Returns

[`TextEncoder`](#textencoder-2)

| Name | Type |
| ------ | ------ |
| <a id="property-prototype"></a> `prototype` | [`TextEncoder`](#textencoder-2) |

***

### TextDecoder

```ts
TextDecoder: {
new (label?: string, options?: TextDecoderOptions): TextDecoder;
  prototype: TextDecoder;
};
```

`utf-8`, `utf-16le`, `utf-16be` and `latin1` (`windows-1252`) in QuickJS.

#### Type Declaration

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `label?` | `string` |
| `options?` | [`TextDecoderOptions`](#textdecoderoptions) |

#### Returns

[`TextDecoder`](#textdecoder-2)

| Name | Type |
| ------ | ------ |
| <a id="property-prototype-1"></a> `prototype` | [`TextDecoder`](#textdecoder-2) |
