# pdf-inspector

PDF の内部構造を調べ、編集するためのツール群。設計は [docs/DESIGN.md](docs/DESIGN.md) を参照。

## 構成（現状）

| crate | 内容 |
|---|---|
| `crates/pdfi-syntax` | ByteSource、lexer / parser、xref（table / stream / hybrid / Prev チェーン）、ObjStm、filters、暗号（RC4 / AES-128 / AES-256）、破損時の復旧、書き出し |
| `crates/pdfi-schema` | Arlington PDF Model（`schema/arlington/` に vendoring）を build 時に静的テーブル化。参照を辿る型推論と validation |
| `crates/pdfi-model` | 意味層: ページ（`/Count` による index 引き・継承属性とその出所・ボックス・ラベル）、Info / XMP（不一致の検出）、しおり、注釈、リンク先、フォント・画像・被参照グラフのインベントリ（ジョブ） |
| `crates/pdfi-font` | フォント: 符号の切り分け（定義済み・埋め込み CMap）、幅（`/Widths`・`/W`・標準 14 フォントの AFM）、テキスト（`/ToUnicode`・エンコーディング + Adobe Glyph List）、埋め込みプログラム（TrueType / OpenType は `ttf-parser`、CFF / Type 1 は `hayro-font`）のグリフ選択とアウトライン、埋め込まれていない単純フォントの代替（PDFium の Foxit フォントを内蔵）、Type 3 のグリフ手続き |
| `crates/pdfi-content` | content stream: 命令列（演算子表・複数ストリームの連結・インライン画像）、validation（オペランド・ネスト・配置・リソース・色）、グラフィックス状態を追うインタプリタと描画要素の bbox（フォーム XObject を辿る）。命令はチェックポイントから読み直して範囲取得 |
| `crates/pdfi-render` | 自作ラスタライザ（tiny-skia）: パス・塗り・線（線幅・端・結合・ダッシュ）・クリップ・画像（1–16 bit、Gray / RGB / CMYK / Indexed / Lab / ICCBased / Separation・DeviceN、JPEG、ステンシル、SMask、色キー）、文字（グリフのアウトライン、Type 3 はグリフ手続き、描画モードの塗り・線・クリップ）。タイル描画、ステップ実行、ハイライト層、ヒットテスト |
| `crates/pdfi-api` | フロントとの境界。`Session` が `Request` → `Response` / `Event` を処理（範囲 API、ジョブ、Batch、バイナリ別フレーム）。TS 型を生成 |
| `crates/pdfi-server` | `pdfi serve` のローカルサーバー（axum + WebSocket、Web UI 内蔵、トークン / Host / Origin / 許可ルート検査） |
| `crates/pdfi-wasm` | ブラウザ完結モード用の wasm バインディング（Worker から `Session` を駆動、Blob から直接読む） |
| `crates/pdfi-cli` | `pdfi` コマンド |
| `apps/web` | Web UI（Vite + React + Mantine）。Document / Pages / Outline / Fonts / Images 画面、フォント画面（プログラム・エンコーディング・問題、符号 → グリフ → テキストの表とアウトライン、埋め込みプログラムの全グリフとそれを選ぶ符号、そのフォントで描かれたテキストの一覧（フォーム内も含む）と使われた符号）、命令列ビュー（ネスト・問題・デコード済みテキスト、命令ごとの前後のグラフィックス状態、描画範囲の図とステップ表示）、ページ描画（タイル・ズーム、クリックで命令を選択、選択した命令のハイライト、ステップ実行）、オブジェクト一覧・ツリー・hex・ストリーム表示、被参照一覧、スキーマ型・キー定義・問題の表示と検証ジョブ。サーバー / ブラウザ完結の両モード |

## 使い方

```sh
cargo build --release
target/release/pdfi info file.pdf
target/release/pdfi xref file.pdf [--rev N]
target/release/pdfi obj file.pdf 12 [0]
target/release/pdfi stream file.pdf 12 [--raw] [-o out.bin]
target/release/pdfi check file.pdf [--decode] [-v]
target/release/pdfi validate file.pdf [--obj N] [-v]   # Arlington PDF Model で検証
target/release/pdfi pages file.pdf [--from N] [--count M]
target/release/pdfi page file.pdf N                    # 継承属性・ボックス・リソース・注釈
target/release/pdfi outline file.pdf
target/release/pdfi meta file.pdf                      # Info / XMP / カタログ
target/release/pdfi inventory file.pdf [--refs N]      # フォント・画像（--refs: N を参照するオブジェクト）
target/release/pdfi ops file.pdf N [--from I --count C] # ページ N の命令列（--obj M: ストリーム、--state I: 状態、--issues: 問題のみ）
target/release/pdfi font file.pdf N [--glyphs]        # フォント N: プログラム・エンコーディング・問題（--glyphs: 符号 → グリフ → テキスト）
target/release/pdfi render file.pdf N -o out.png [--dpi D] # ページ N を描画（--until I: 命令 I まで、--tile x,y,w,h、--highlight I,J、--hit x,y: その点を描く命令）
target/release/pdfi rewrite in.pdf out.pdf [--reserialize]
```

暗号化ファイルは `--password`、xref を無視して走査で復元するには `--recover`。

### Web UI

UI のビルドには wasm32 ターゲットと、`crates/pdfi-wasm/Cargo.toml` で固定した版の wasm-bindgen-cli が要る。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
```

#### ブラウザ完結

```sh
(cd apps/web && npm ci && npm run build)   # dist/ を静的ホスティングすればよい
(cd apps/web && npm run preview)           # 手元で確認: http://localhost:4173/
```

トークン無しで開くと Worker 内の wasm で動く。PDF は「Open…」かドラッグ & ドロップで開き、アップロードはされない（Chromium 系のみ対応）。

#### ローカルサーバー

```sh
(cd apps/web && npm ci && npm run build)   # UI をビルド（pdfi に埋め込まれる）
cargo build --release
target/release/pdfi serve file.pdf          # ブラウザで http://127.0.0.1:<port>/#token=... が開く
target/release/pdfi serve --root ~/pdfs     # ディレクトリ配下を UI から開けるようにする
```

UI から開けるのは引数のファイルと `--root` 配下のみ（どちらも無ければカレントディレクトリ）。
UI 開発時は `pdfi serve --port 7878 --dev-origin http://localhost:5173 file.pdf` を起動し、
`npm run dev` の画面を `http://localhost:5173/#token=<token>&server=127.0.0.1:7878` で開く。

## テスト

```sh
cargo test --workspace
UPDATE_TS=1 cargo test -p pdfi-api        # プロトコルを変えたら TS 型（apps/web/src/protocol/generated.ts）を再生成
UPDATE_CONFORMANCE=1 cargo test -p pdfi-api --test conformance   # 適合テストの期待結果を再生成（意図した変更のとき）
(cd apps/web && npm run build:wasm && npm run test:conformance)  # 同じシナリオを wasm で実行
tests/fixtures/make.sh                    # フィクスチャ再生成（python3 + fontTools, qpdf が必要）
scripts/make-large-pdf.py big.pdf --pages 3000 --image-kib 100   # 大きな合成 PDF（約 300 MB）
scripts/corpus-check.sh <dir> [--decode]  # 手元の PDF 群で qpdf とページ数を突き合わせ
```
