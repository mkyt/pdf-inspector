# マクロの例

よくある処理の書き方。前半は同梱のマクロ（[`crates/pdfi-macro/macros/`](../../crates/pdfi-macro/macros/)）の解説、後半は小さな例。```` ```js macro ```` のブロックは `cargo test -p pdfi-macro` がテスト用の PDF に対して実行している（例が古くならないように）。

## 同梱のマクロ

### `fonts` — 使われているフォントの一覧

```sh
pdfi macro run fonts in.pdf --format table
pdfi macro run fonts *.pdf --format jsonl | jq 'select(.value[] | .embedded == "no")'
```

`doc.fonts()` はインベントリ（全オブジェクトとページの走査）を実行してからフォントを返す。結果をフラットなオブジェクトの配列にすると、そのまま表になる。

```js
const fonts = await doc.fonts();
return fonts.map((f) => ({ font: f.baseFont ?? "", type: f.subtype ?? "", embedded: f.embedded ?? "no", pages: f.pageCount }));
```

### `metadata` — Info・XMP・カタログの宣言

`doc.info()`（Info 辞書。日付は ISO 8601）、`doc.xmp()`（XMP のプロパティと XML）、`doc.catalog.get(...)` を集める。`Name` は JSON では名前の文字列になる。

### `set-reading` — 見開き・綴じ方向

```sh
pdfi macro run set-reading manga.pdf -o out.pdf                     # 既定: TwoPageRight, R2L
pdfi macro run set-reading report.pdf --in-place -p layout=OneColumn -p direction=L2R
```

```js
await doc.catalog.set("PageLayout", pdf.name(params.layout));
const prefs = await doc.catalog.dict("ViewerPreferences", { create: true });
await prefs.set("Direction", pdf.name(params.direction));
```

名前は `pdf.name()` で作る（JS の文字列はテキスト文字列になる）。`dict(key, { create: true })` は、無ければ空の辞書を置いてそれを返す。2 つの編集は 1 回の undo で戻る。

### `crop-to-content` — 描画範囲に CropBox を詰める

```sh
pdfi macro run crop-to-content scan.pdf -o out.pdf -p margin=12 -p pages=2-
```

`page.content.bounds()` は描かれるものの範囲（クリップと見えないものは除く）。それに余白を足し、MediaBox との共通部分を CropBox にする。`doc.pages(params.pages)` は `pages` パラメータ（0 始まりの配列。無ければ全ページ）のページを順に返す。`ctx.progress` で進み具合を出す。

### `remove-owner-password` — owner password（権限）の除去

```sh
pdfi macro run remove-owner-password locked.pdf -o unlocked.pdf
pdfi macro run remove-owner-password secret.pdf -o plain.pdf --password 'user password'
```

マクロ自身は何も編集しない。`meta.save: "full"` と `edits: true` で、ホストが full rewrite（復号して書く）で保存する。開けたかどうか（`doc.encryption.authenticated`）を確かめるだけ。

## 小さな例

### 文書の概要を返す

```js macro
export const meta = { api: 1, name: "summary", description: "Pages, version and title" };

export default async function (doc) {
  const info = await doc.info();
  const first = await doc.page(0);
  return {
    pages: doc.pageCount,
    version: doc.version,
    title: info.Title ?? null,
    firstPage: first.size,
    encrypted: doc.encryption !== null,
  };
}
```

### Info を書き換える（XMP も追従する）

```js macro
export const meta = {
  api: 1,
  name: "set-title",
  edits: true,
  params: {
    title: { type: "string", default: "Untitled" },
    author: { type: "string", optional: true },
  },
};

export default async function (doc, params) {
  await doc.setInfo({ Title: params.title, Author: params.author, ModDate: new Date() });
  return doc.info();
}
```

`null` や `undefined` を渡したキーは消える（ここでは `author` を与えなければ Author が消える）。`{ syncXmp: false }` で XMP を変えない。

### ページを選んで回転・削除する

```js macro
export const meta = {
  api: 1,
  name: "tidy-pages",
  edits: true,
  params: {
    rotate: { type: "pages", default: "1", description: "Pages turned by 90 degrees" },
    remove: { type: "pages", optional: true, description: "Pages removed" },
  },
};

export default async function (doc, params) {
  await doc.rotatePages(params.rotate, 90);
  if (params.remove) await doc.deletePages(params.remove);
  return { pages: doc.pageCount };
}
```

番号はそのときの文書で数える。回転は番号を変えないのでこの順なら同じ番号で通じるが、削除や移動の後は番号がずれるので、`doc.page(i)` / `doc.pages()` で読み直す。

### 命令列を見る

```js macro
export const meta = { api: 1, name: "operators", description: "How often each operator is used" };

export default async function (doc, _params, ctx) {
  /** @type {Map<string, number>} */
  const counts = new Map();
  let done = 0;
  for await (const page of doc.pages()) {
    ctx.progress(done++, doc.pageCount);
    for await (const op of page.content.ops()) counts.set(op.operator, (counts.get(op.operator) ?? 0) + 1);
  }
  return [...counts].sort((a, b) => b[1] - a[1]).map(([operator, count]) => ({ operator, count }));
}
```

`op.text` は文字を表示する命令のデコード済みテキスト、`op.element` はその命令が描くもの（`kind` と `bbox`）。

### 命令を足す（ページの上に枠を描く）

```js macro
export const meta = { api: 1, name: "frame", edits: true, params: { pages: { type: "pages", default: "1" } } };

export default async function (doc, params) {
  for await (const page of doc.pages(params.pages)) {
    const { crop } = await page.boxes();
    if (!crop) continue;
    const n = await page.content.count();
    // 全体を q … Q で囲んでから、最後に枠を描く。
    await page.content.insert(0, "q");
    await page.content.insert(n + 1, `Q q 1 0 0 RG 2 w ${crop.x0 + 4} ${crop.y0 + 4} ${crop.x1 - crop.x0 - 8} ${crop.y1 - crop.y0 - 8} re S Q`);
  }
  return { pages: params.pages.map((i) => i + 1) };
}
```

命令のテキストは content stream の構文（1 文字 1 バイト）。変わらない命令は元のバイトのまま残る。

### オブジェクトを直接読み書きする

```js macro
export const meta = { api: 1, name: "page-tree", description: "The page tree's root" };

export default async function (doc) {
  const pages = await doc.catalog.resolve("Pages");   // 参照を辿る
  if (!(pages instanceof pdf.Node)) throw new Error("no page tree");
  const kids = await pages.get("Kids");                // Ref の配列
  return {
    ref: pages.ref,
    count: await pages.get("Count"),
    kids: Array.isArray(kids) ? kids.length : 0,
    keys: await pages.keys(),
  };
}
```

### ファイルを出力する

```js macro
export const meta = { api: 1, name: "labels-csv", description: "Page labels as CSV" };

export default async function (doc, _params, ctx) {
  const lines = ["page,label"];
  for await (const page of doc.pages()) lines.push(`${page.index + 1},${page.label ?? ""}`);
  ctx.emit("labels.csv", lines.join("\n") + "\n", "text/csv");
  return { pages: lines.length - 1 };
}
```

```sh
pdfi macro run ./labels-csv.js a.pdf b.pdf --emit-dir out   # out/a-labels.csv、out/b-labels.csv
```

### 別の PDF のページを足す

```js
export const meta = {
  api: 1,
  name: "append",
  edits: true,
  params: { other: { type: "pdf", description: "The PDF whose pages go at the end" } },
};

export default async function (doc, { other }) {
  await doc.insertPages(other);
  return { pages: doc.pageCount };
}
```

```sh
pdfi macro run ./append.js main.pdf -p other=appendix.pdf -o out.pdf
```
