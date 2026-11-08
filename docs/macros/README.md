# マクロ

マクロは JavaScript で書く小さなスクリプトで、情報の抽出（フォント一覧・メタデータ）や定型の編集（見開き・綴じ方向、CropBox の詰め、owner password の除去など）を行う。よく使うものはマクロのディレクトリに置いておき、名前で呼び出す。CLI では複数のファイルにまとめて実行できる。

- [guide.md](guide.md): スクリプトの形・`meta` とパラメータ・PDF の値・編集と undo・保存・出力・止め方・処理系の違い
- [cookbook.md](cookbook.md): よくある処理の例（同梱のマクロの解説を兼ねる）
- [api/](api/README.md): API リファレンス（[`pdfi.d.ts`](../../crates/pdfi-macro/js/pdfi.d.ts) から生成）

設計は [DESIGN.md §4.11](../DESIGN.md#411-pdfi-macro--マクロjavascript)。

> v1 で動くのは CLI（`pdfi macro`）。サーバーモード・ブラウザ完結モード・macOS アプリからの実行は M16 / M17 で足す。どこで動かしても同じ API（prelude）を使うので、ここで書いたマクロはそのまま使える。

## 最初のマクロ

```sh
pdfi macro new page-ops          # マクロのディレクトリに page-ops.js と pdfi.d.ts を書く
```

できたファイルを書き換える。ページごとの命令の数を返すマクロ:

```js macro
// @ts-check
/// <reference path="pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "page-ops",
  title: "Operations per page",
  description: "How many operations each page's content has",
  edits: false,
  params: {
    pages: { type: "pages", optional: true, description: "Pages to look at (all when not given)" },
  },
};

/** @type {pdfi.Main<{ pages?: number[] }>} */
export default async function (doc, params, ctx) {
  const rows = [];
  for await (const page of doc.pages(params.pages)) {
    rows.push({ page: page.index + 1, label: page.label ?? "", operations: await page.content.count() });
  }
  ctx.log(`${rows.length} pages`);
  return rows;
}
```

- `export const meta` で名前・説明・文書を変えるかどうか（`edits`）・パラメータを宣言する
- `export default` の async 関数が本体。`doc`（文書）・`params`（検証済みのパラメータ）・`ctx`（ログ・進捗・出力ファイル）を受け取り、返した値が結果になる
- 1 行目の `// @ts-check` と `pdfi.d.ts` で、VS Code などのエディタが補完と型の検査をする（`pdfi macro types -o pdfi.d.ts` でいつでも書き出せる）

## CLI で実行する

```sh
pdfi macro list                                  # 同梱 + マクロのディレクトリ
pdfi macro show page-ops                         # 説明とパラメータ
pdfi macro run page-ops in.pdf --format table    # 結果を表で
pdfi macro run page-ops in.pdf -p pages=1-3,8    # パラメータ（1 始まり）
pdfi macro run ./draft.js in.pdf                 # ディレクトリに置いていないファイル
```

文書を変えるマクロ（`edits: true`）は保存先が要る:

```sh
pdfi macro run set-reading in.pdf -o out.pdf -p direction=R2L
pdfi macro run set-reading in.pdf --in-place
pdfi macro run set-reading in.pdf --dry-run      # 実行するが保存しない
```

| オプション | 意味 |
|---|---|
| `-p NAME=VALUE`、`-p NAME=@FILE` | パラメータ。`@` はファイルの中身（`json` は JSON として、`text` は文字列、`bytes` はバイト列、`pdf` は文書） |
| `--params p.json` | パラメータを JSON のオブジェクトで（`-p` が優先） |
| `-o FILE` / `--out-dir DIR` / `--in-place` | 保存先。`--out-dir` には同じファイル名で書く |
| `--save incremental\|full` | 保存方法（既定は `meta.save`。暗号化されたファイルは常に full） |
| `--emit-dir DIR` | `ctx.emit` のファイルの置き場所（`--out-dir` があればそこ） |
| `--format json\|jsonl\|table` | 結果の形（既定 json） |
| `--password P` | 入力ファイルのパスワード |
| `--timeout SEC` | 1 ファイルにかける時間の上限 |
| `-q` | ログを出さない（エラーは出す） |

結果は 1 ファイル 1 件の `{ file, ok, value, error, saved, emitted }`。`value` がマクロの返り値。ログと進捗は stderr に出る。Ctrl-C で止めると、そのファイルの編集は戻して保存しない。

## 一括処理

```sh
pdfi macro run fonts *.pdf --format jsonl > fonts.jsonl
find . -name '*.pdf' | pdfi macro run crop-to-content - --out-dir cropped -p margin=6 --jobs 4
```

- 入力ファイルをいくつでも並べられる。`-` は stdin から 1 行 1 パスを読む（ディレクトリや glob の展開はシェルに任せる）
- ファイルごとに別の文書と別の処理系の実行環境で動く。`--jobs N` で N 個を並列に
- `-o` はファイルが 1 つのときだけ。複数なら `--out-dir` か `--in-place`
- 失敗したファイルは保存しない（その実行の編集は戻る）。既定では最初の失敗で止まり、`--keep-going` で残りを続ける。失敗が 1 つでもあれば終了コードは 1
- `ctx.emit` のファイル名の前には入力ファイルの名前が付く（`report.csv` → `in-report.csv`）

## 保存場所

- **同梱のマクロ**（バイナリに内蔵）: `fonts`、`metadata`、`set-reading`、`crop-to-content`、`remove-owner-password`、`remove-marks`、`outline-from-json`、`page-labels`。ソースは [`crates/pdfi-macro/macros/`](../../crates/pdfi-macro/macros/)。[cookbook.md](cookbook.md) に解説がある
- **マクロのディレクトリ**: 1 マクロ 1 ファイル（`*.js`）。`$PDFI_MACRO_PATH`（`:` 区切りで複数）か、既定の場所
  - macOS: `~/Library/Application Support/pdf-inspector/macros`
  - それ以外: `$XDG_CONFIG_HOME/pdf-inspector/macros`（既定 `~/.config/pdf-inspector/macros`）
  - macOS アプリも同じディレクトリを使う（M17）
- 名前の解決順: パス（`/` を含むか `.js` で終わるもの）→ マクロのディレクトリの `<name>.js` → `meta.name` が一致するもの → 同梱のマクロ。同じ名前があればディレクトリのほうが勝つ（`pdfi macro list` に「hidden」と出る）
