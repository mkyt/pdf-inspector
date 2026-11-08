# マクロの書き方

API の一つ一つは [api/](api/README.md)（`pdfi.d.ts` から生成）にある。ここではその前提になる約束ごとをまとめる。

## スクリプトの形

1 ファイルの ES module。`import` は使えない（v1）。

```js
export const meta = { api: 1, name: "…", title: "…", description: "…", edits: false, params: {} };

/** @type {pdfi.Main} */
export default async function (doc, params, ctx) {
  // …
  return value;
}
```

- `meta` はモジュールを評価して読む。一覧（`pdfi macro list`）を作るときは文書無しで評価するので、**トップレベルで文書の API は使えない**。トップレベルには定数と関数だけを置く
- 本体は `doc`（[`Doc`](api/Namespace.pdfi.md#doc)）・`params`（検証と変換が済んだパラメータ）・`ctx`（[`Context`](api/Namespace.pdfi.md#context)）を受け取る
- **API はすべて async**。読み取りも編集も `await` する。ページや命令の列は `for await` で回す
- 返り値は JSON にできる値。フラットなオブジェクトの配列なら表として、それ以外は JSON の木として見せる。`Name` / `PdfString` / `Ref` はその文字列になり、`undefined` は `null`

## `meta`

| キー | 既定 | 意味 |
|---|---|---|
| `api` | （必須） | この API の版: `1`。ホストの prelude が違う版ならエラー |
| `name` | ファイル名 | 呼び出しに使う名前（英数字と `.` `_` `-`） |
| `title` | `name` | 短い題名。undo の項目の名前にもなる |
| `description` | `""` | 説明 |
| `edits` | `false` | 文書を変えるか。true なら CLI は保存先を求め、何も変えなくても保存する |
| `save` | `"incremental"` | 保存方法の希望: `"incremental"` / `"full"` / `{ xrefStream, objectStreams, compress, removeUnused }`（full の設定） |
| `params` | `{}` | パラメータ（下記） |

## パラメータ

**宣言したものだけ**を受け取る。実行中に対話的に入力を求める手段は無い（CLI の一括処理と GUI で挙動を揃えるため）。ホストは実行前に、宣言に照らして検証と変換を行い、`default` を補う。知らない名前・必須のものの欠け・型の違いはエラーになり、マクロは呼ばれない。

```js
params: {
  margin: { type: "number", default: 0, description: "Points added on every side" },
  pages:  { type: "pages", optional: true },
  mode:   { type: "enum", values: ["fast", "exact"], default: "fast" },
}
```

| type | 与え方（CLI） | 受け取る値 |
|---|---|---|
| `string` | `-p k=text` | 文字列 |
| `number` / `integer` | `-p k=2.5` | 数（`integer` は整数でなければエラー） |
| `boolean` | `-p k=true`（`yes` `no` `1` `0` `on` `off` も） | 真偽値 |
| `enum` | `values` のどれか | 文字列 |
| `pages` | `-p k=1-3,8`、`-p k=5-`（**1 始まり**） | **0 始まり**のページ番号の配列。文書のページ数を超えるとエラー |
| `json` | `-p k='{"a":1}'`、`-p k=@data.json` | 解析した値 |
| `text` | `-p k=@notes.txt` | ファイルのテキスト（UTF-8） |
| `bytes` | `-p k=@image.png` | ファイルの中身（`Uint8Array`） |
| `pdf` | `-p k=other.pdf` | **読み取り専用**で開いた別の `Doc`（`doc.insertPages(other)` などに使う） |

各パラメータに `default`・`optional`・`description` を付けられる（`text` / `bytes` / `pdf` は `default` を持てない）。`optional` で `default` が無いものは、与えられなければ `params` に入らない。`--params p.json` の値は JSON のまま（文字列を数に直すのは `-p` の文字列だけ）。

## 番号と単位

- API のページ番号・命令の番号・添字は **0 始まり**（プロトコルと同じ）。人が書くもの（`pages` パラメータ、CLI）だけが 1 始まり
- 長さはポイント（1/72 インチ）。ページの矩形 `{ x0, y0, x1, y1 }` はデフォルトのユーザー空間で、`x0 <= x1`、`y0 <= y1`
- 命令の範囲は `{ from, to }`（`to` は含まない）か `[from, to]`

## PDF の値

読み取ると:

| PDF | JS |
|---|---|
| 数・真偽値・`null` | `number` / `boolean` / `null` |
| 配列 | JS の配列（中身も変換する） |
| 名前 `/Type` | `Name`（`.name` は `"Type"`。`#xx` は戻してある） |
| 文字列 | `PdfString`（`.text` は復号したテキスト、`.bytes` はバイト列、長いもの（1 KB 超）は `.truncated`） |
| 参照 `12 0 R` | `Ref`（`.num`、`.generation`）。辿るには `doc.object(ref)` か `node.resolve(key)` |
| 辞書・ストリーム | `Node`（中身は聞かれたときに読む） |

書き込みでは、読んだ値のほか JS の値も使える:

- JS の文字列は **PDF のテキスト文字列**として書く（ASCII ならそのまま、それ以外は UTF-16BE）。名前にするには `pdf.name("TwoPageRight")` で明示する
- プレーンなオブジェクトは辞書、配列は配列、`Uint8Array` は 16 進の文字列、`Date` は PDF の日付
- `pdf.str(text | bytes)`、`pdf.ref(num, gen)`、そのまま書く PDF 構文は `pdf.raw("<< /S /GoTo /D [3 0 R /Fit] >>")`
- キーは `"PageLayout"` と `"/PageLayout"` のどちらでもよい

`Node` はオブジェクトの中の位置をキーで覚えていて、編集の後も今の文書を見る。直接オブジェクト（`catalog.dict("ViewerPreferences")` が参照でなく辞書そのものだった場合など）でも同じように読み書きできる。別の場所へ書き込めるのは間接オブジェクトの `Node`（参照として書かれる）だけ。

```js
const prefs = await doc.catalog.dict("ViewerPreferences", { create: true }); // 無ければ作る
await prefs.set("Direction", pdf.name("R2L"));
const lang = await doc.catalog.get("Lang");          // PdfString か undefined
const pages = await doc.catalog.resolve("Pages");    // 参照を辿って Node
```

## 命令列の検索と除去

`page.content.find(query)` は、ページが描くもののうち条件をすべて満たすものを命令の範囲で返す。全ページの命令を JS に渡すと数が多すぎるので、絞り込みは文書の側（native / Worker）で行い、JS には当たったものだけが来る。

```js
const { trim } = await page.boxes();
const hits = await page.content.find({
  bbox: { outside: trim },                                   // inside / outside / intersects
  colorSpace: { separation: "All" },                         // レジストレーションカラー
  // markedContent: { tag: "Artifact", properties: { Subtype: "Watermark" } },
  // ocg: /watermark/i,                                     // レイヤーの名前（文字列・RegExp・関数・Ref）
  // operators: ["re", "f"], kinds: ["text"], text: "CONFIDENTIAL", hidden: false,
  unit: "element",                                           // "op" | "element" | "block"
});
await page.content.remove(hits);                             // 1 回の編集。消した命令の数を返す
```

- 候補は**描画要素**: パス（最初の構築演算子から描画演算子まで。`W n` のクリップも含む）、文字を表示する命令、`Do`、`sh`、インライン画像。クリップのパスは `kinds` に `"clip"` を挙げたときだけ当たる（消すと隠れていたものが見えるため）
- `unit` は当たったものごとに返す範囲: `"element"`（既定。描画要素だけを消しても、残りは同じように描かれる）、`"block"`（それを囲む `q`…`Q` か `BMC` / `BDC`…`EMC`。`markedContent` / `ocg` を指定したときは当たったマークの範囲）、`"op"`（命令 1 つ。`operators` に挙げた命令だけ。描画しない命令（`gs` など）も、描画要素にしか無い条件を使わなければ当たる）
- フォーム XObject の中も検索するが、当たるのはそれを描く `Do`（`hit.form` が中のフォーム）。フォームは複数のページから共有されることがあるので、中の命令は消さない
- `text` の `RegExp`、2 つ目以降の `bbox` の関係、`where` は、返ってきた結果を JS で絞る
- 範囲は `{ from, to }`（`to` は含まない）。`remove` は範囲が重なっていても、まとめて 1 回で消す
- 戻り値の `bbox` はデフォルトのユーザー空間（ページの回転の前）

## 目次とページラベル

- `doc.outline()` は今の目次を木で返し、`doc.setOutline(items)` は木を丸ごと置き換える（`[]` で目次を消す）。`outline()` の結果をそのまま（並べ替え・書き換えて）渡せる。項目は `{ title, page, view, dest, uri, open, bold, italic, color, children }`。`page` は 0 始まり、`view` は `"Fit"`（既定）か `{ fit: "XYZ", params: [left, top, zoom] }`（`null` はビューアに任せる）。古い目次の項目は削除する
- `doc.pageLabels()` / `doc.setPageLabels(ranges)`: `{ start, style, prefix, first }` の列（`start` は 0 始まりで、最初の範囲は 0 から）。`style` は `decimal`・`upperRoman`・`lowerRoman`・`upperLetters`・`lowerLetters`（`D`・`R`・`r`・`A`・`a` でもよい）か `null`（接頭辞だけ）

## 編集と undo の単位

- 編集はその場で反映し、後の読み取りには反映後の内容が見える（`doc.pageCount` もページの編集に従う）
- **マクロ 1 回の実行が 1 つの undo の単位**（名前は `meta.title`）。いくつ編集しても、GUI の undo 1 回で戻る
- 例外・キャンセル・時間切れのときは、実行前の状態に戻す。中途半端な状態で保存されることは無い
- 実行中は、その文書への他の編集（UI からの編集・undo・redo・保存・閉じる）は `busy` で断られる。読み取りはできる
- `Page` はそのとき読んだページを表す。ページの削除・移動・挿入の後は `doc.page(i)` / `doc.pages()` で読み直す
- `pdf` パラメータの文書は読み取り専用で、編集すると `forbidden` が投げられる

## 保存

マクロ自身は保存しない。`meta.save` は希望で、実際の保存はホストが行う。

- CLI: `-o` / `--out-dir` / `--in-place` に保存する。`--save` で方法を上書きできる。`meta.edits` が false でも文書が変わっていれば、保存先があれば保存する
- GUI（Web、macOS は M17）: 未保存の編集として残り、`meta.save` を初期値にして保存を勧める
- 暗号化されたファイルや xref の壊れたファイルは incremental にできないので、full rewrite になる（暗号化は外れる）。owner password の除去は「空の user password で開いて full rewrite で保存」で実現できる（`remove-owner-password` が `save: "full"` でこれを行う）

## 出力

| 種類 | CLI | GUI（Web、macOS は M17） |
|---|---|---|
| 返り値 | stdout に JSON（`--format json` / `jsonl` / `table`） | 表か JSON の木。コピー・書き出し |
| `ctx.log` / `ctx.warn` / `console.*` | stderr（`-q` で消す） | ログ欄 |
| `ctx.progress(done, total, label)` | 端末なら stderr に 1 行 | 進捗バー |
| `ctx.emit(name, data, mime)` | `--out-dir`（無ければ `--emit-dir`）に書く | 一覧からダウンロード・保存 |

`ctx.emit` の `data` は `Uint8Array` / `ArrayBuffer` / 文字列（UTF-8）/ オブジェクト（JSON にする）。`name` はディレクトリを含まないファイル名。

## エラー

- 文書の側で断られたものは `pdf.PdfiError`（`.code` は `badRequest`、`notFound`、`forbidden`、`notReady`、`busy`、`cancelled` など）
- マクロが投げた例外はそのまま結果のエラーになる（メッセージと、マクロのファイルの行番号つきのスタック）
- `try` / `catch` で受けて続けてもよい。そのときも、それまでの編集は残る（undo の単位はマクロ全体）

## 止め方

安全のための制限（メモリの上限など）は設けない。止める手段だけを持つ。

- キャンセル（CLI の Ctrl-C、GUI のボタン）と `--timeout`。JavaScript を実行している最中でも止まる（ホスト関数を呼ばない無限ループでも）
- 長い繰り返しでは `ctx.cancelled` を見て自分で抜けてもよい
- ブラウザ完結モードでは、止めるとマクロの Worker ごと終わらせる（`ctx.cancelled` は true にならない。`finally` も走らない）。編集は文書を持つ側で戻す

## 使えるもの・処理系の違い

- ECMAScript（ES2023 の範囲）の組み込みと `console`、`TextEncoder` / `TextDecoder`
- ファイル・ネットワーク・タイマー・`import` の API は用意しない。入力はパラメータで受け取る。ただし**これはサンドボックスではない**（ローカルで動かす前提なので、安全のための隔離はしない）
- 処理系: CLI・サーバー・macOS は QuickJS、ブラウザ完結モードはブラウザの JS（マクロ専用の Worker）。違いは ECMAScript 本体の細部だけで、API は同じ prelude が提供する。`TextDecoder` は QuickJS では `utf-8` / `utf-16le` / `utf-16be` / `latin1` だけ
- `ctx.host` で実行している場所（`"cli"` / `"server"` / `"macos"` / `"web"`）が分かる
- PDF の中の JavaScript（`/JS`）は実行しない。それとは別物で、UI では「マクロ」と呼ぶ

## プロトコルとの関係

マクロとの互換性を約束するのは prelude（`meta.api` の版）だけ。prelude は文書を持つ `Session` のプロトコル（`crates/pdfi-api`）の上に JS で書かれていて（[`prelude.js`](../../crates/pdfi-macro/js/prelude.js)）、プロトコルの変化はその中で吸収する。

- `doc.apply(commands, label)` はプロトコルの `EditCommand` をそのまま適用する低水準の手段（値は PDF 構文の文字列）
- `doc.raw.dispatch(request)` は生のプロトコル。何でもできるが、互換性は約束しない
- ホストの側（`__host`）は `dispatch`・`waitJob`・`log`・`progress`・`emit`・`cancelled` の小さな関数群で、マクロからは直接使わない（prelude の先頭に説明がある）。native は `crates/pdfi-macro/src/engine.rs`、ブラウザは `apps/web/src/macro/engine.ts`
- サーバー（と macOS）では、マクロの実行はプロトコルの `JobKind::Macro` のジョブ。ログは `MacroLog` イベント、進捗は `JobProgress`（`label` 付き）、結果は `JobResult::Macro`、`ctx.emit` のファイルは `JobOutput` で読む。ブラウザでは macro Worker が `EditGroup` で undo の単位を開き、要求に `ClientMessage.group` を付けて送る
- 同梱のマクロは両方の処理系で実行して結果を突き合わせる（`tests/conformance/macros.json`、期待値は `macros-expected.json`。`UPDATE_CONFORMANCE=1 cargo test -p pdfi-macro --test conformance` で作り直す）

## API を変えるとき

`pdfi.d.ts` が型と説明の唯一の出どころ。prelude を変える PR では、`pdfi.d.ts` とこのディレクトリの文書も一緒に直す。

```sh
cd crates/pdfi-macro/js && npm ci
npm run check    # prelude・同梱のマクロ・ひな形を pdfi.d.ts に照らして検査（tsc --checkJs）
npm run docs     # docs/macros/api/ を作り直す（CI は差分を検出する）
cargo test -p pdfi-macro   # 同梱のマクロと、文書の ```js macro の例を実行する
```
