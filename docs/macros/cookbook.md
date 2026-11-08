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

### `remove-marks` — トンボ・透かしの除去

```sh
pdfi macro run remove-marks print.pdf -o clean.pdf --format table      # TrimBox の外・レジストレーションカラー・透かし
pdfi macro run remove-marks draft.pdf -o out.pdf -p outsideTrim=false -p registration=false -p text=CONFIDENTIAL
pdfi macro run remove-marks plan.pdf -o out.pdf -p watermarks=false -p 'layers=^(notes|review)'
```

条件ごとに `page.content.find()` で探し、当たったものをまとめて `page.content.remove()` で消す。

```js
const boxes = await page.boxes();
if (boxes.trim && boxes.origins.trim.kind !== "default")              // TrimBox が書かれているページだけ
  hits.push(...(await c.find({ bbox: { outside: boxes.trim } })));    // 仕上がりの外: トンボ・カラーバー・スラグ
hits.push(...(await c.find({ colorSpace: { separation: "All" } })));  // レジストレーションカラー
hits.push(...(await c.find({ markedContent: { tag: "Artifact", properties: { Subtype: "Watermark" } }, unit: "block" })));
hits.push(...(await c.find({ ocg: /watermark/i, unit: "block" })));    // 名前が watermark のレイヤー
await c.remove(hits);
```

- 仕上がりの外は「TrimBox と重ならない」もの。裁ち落とし（TrimBox をまたぐ背景）は残る。TrimBox の無いページは CropBox と同じなので、この条件は使わない
- 透かしは `BDC … EMC` ごと（`unit: "block"`）消す。中の `q … Q` や `BT … ET` も一緒に消える
- フォームの中で当たったもの（スラグがフォームになっていることが多い）は、そのフォームを描く `Do` を消す
- 結果はページごとの件数（同じ命令が複数の条件に当たれば、それぞれに数える）と、消した命令の数

### `outline-from-json` — JSON から目次を作る

```sh
pdfi macro run outline-from-json book.pdf -o out.pdf -p outline=@toc.json -p offset=12 -p open=1
```

```json
[
  { "title": "はじめに", "page": 1 },
  { "title": "第 1 章", "page": 5, "children": [
    { "title": "1.1 概要", "page": 6, "bold": true },
    { "title": "付録", "page": "A-1" }
  ] },
  { "title": "Web サイト", "uri": "https://example.com/" }
]
```

`page` は 1 始まりのページ番号か、ページラベル（文字列）。`offset` は番号に足す数（本の印刷されたノンブルと PDF のページがずれているとき）、`open` は開いて見せる階層の数。木を `doc.setOutline()` に渡すと、`/First` / `/Last` / `/Next` / `/Prev` / `/Parent` / `/Count` は文書の側で組み立てる。

### `page-labels` — ページラベル

```sh
pdfi macro run page-labels book.pdf -o out.pdf -p labels='1:r, 5:D, 120:D:A-'   # i, ii, …, 1, 2, …, A-1, A-2, …
pdfi macro run page-labels book.pdf -o out.pdf -p labels=none                   # ラベルを消す
```

`ページ:形式[:接頭辞[:最初の番号]]` をカンマで区切る。ページは 1 始まり、形式は `D`（1, 2）・`R`（I, II）・`r`（i, ii）・`A`（A, B）・`a`（a, b）か `-`（接頭辞だけ）。`doc.setPageLabels([{ start: 0, style: "lowerRoman" }, { start: 4, style: "decimal" }])` で設定し、`doc.pageLabels()` で読み直した結果を返す。

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

### 命令を探して消す（見えない文字）

```js macro
export const meta = { api: 1, name: "remove-hidden-text", edits: true, description: "Remove text shown invisibly (Tr 3)" };

export default async function (doc, _params, ctx) {
  let removed = 0;
  for await (const page of doc.pages()) {
    const hits = await page.content.find({ kinds: "text", hidden: true, where: (h) => h.form === null });
    removed += await page.content.remove(hits);
  }
  ctx.log(`${removed} operations removed`);
  return { removed };
}
```

`hidden` は見えない文字（`Tr` 3 / 7）か、クリップの外にあるもの。`where` は 1 件ずつの最後の判定（ここではフォームの中のものを除く）。何も当たらなければ `remove` は何もしない。

### 目次を書き出す・並べ替える

```js macro
export const meta = { api: 1, name: "sort-outline", edits: true, description: "Sort the outline's top level by page" };

export default async function (doc, _params, ctx) {
  const items = await doc.outline();
  ctx.emit("outline.json", items.map(({ title, page, children }) => ({ title, page: page === null ? null : page + 1, children: children.length })));
  items.sort((a, b) => (a.page ?? Infinity) - (b.page ?? Infinity));
  await doc.setOutline(items);
  return items.map((it) => it.title);
}
```

`doc.outline()` の項目は、そのまま `doc.setOutline()` に渡せる（リンク先・開閉・色・太字を引き継ぐ。別のファイルへ行く項目や他のアクションは行き先が無くなる）。
