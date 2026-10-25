# pdf-inspector

PDF の内部構造を調べ、編集するためのツール群。設計は [docs/DESIGN.md](docs/DESIGN.md) を参照。

## 構成（現状）

| crate | 内容 |
|---|---|
| `crates/pdfi-syntax` | ByteSource、lexer / parser、xref（table / stream / hybrid / Prev チェーン）、ObjStm、filters、暗号（RC4 / AES-128 / AES-256）、破損時の復旧、書き出し |
| `crates/pdfi-schema` | Arlington PDF Model（`schema/arlington/` に vendoring）を build 時に静的テーブル化。参照を辿る型推論と validation |
| `crates/pdfi-model` | 意味層: ページ（`/Count` による index 引き・継承属性とその出所・ボックス・ラベル）、Info / XMP（不一致の検出）、しおり、注釈、リンク先、フォント・画像・被参照グラフのインベントリ（ジョブ） |
| `crates/pdfi-font` | フォント: 符号の切り分け（定義済み・埋め込み CMap）、幅（`/Widths`・`/W`・標準 14 フォントの AFM）、テキスト（`/ToUnicode`・エンコーディング + Adobe Glyph List）、埋め込みプログラム（TrueType / OpenType は `skrifa`、CFF / Type 1 は `hayro-font`）のグリフ選択とアウトライン、埋め込まれていない単純フォントの代替（PDFium の Foxit フォントを内蔵）、Type 3 のグリフ手続き |
| `crates/pdfi-content` | content stream: 命令列（演算子表・複数ストリームの連結・インライン画像）、validation（オペランド・ネスト・配置・リソース・色）、グラフィックス状態を追うインタプリタと描画要素の bbox（フォーム XObject を辿る）。命令はチェックポイントから読み直して範囲取得。範囲の切り出し（範囲内を描く命令だけを元のバイト列のまま残し、状態・クリップは保持、部分的に掛かるフォームは中身を切る。使うリソースだけを符号化されたまま複製）。命令列の編集（置き換え・挿入・削除・移動を差分で適用し、変わらない命令は元のバイトのまま、変わったストリームだけを書き換え） |
| `crates/pdfi-edit` | 編集と保存: コマンド（ノードの値・辞書エントリ・配列要素、オブジェクトの追加・削除、ストリームのデータとフィルタ、ページの削除・移動・回転・空白ページ・他の文書からの複製、Info と XMP）をトランザクションとして適用、undo / redo、incremental update / full rewrite（xref stream・オブジェクトストリーム・圧縮・未使用の除去）をチャンク単位で書き出し |
| `crates/pdfi-render` | 自作ラスタライザ（tiny-skia）: パス・塗り・線（線幅・端・結合・ダッシュ）・クリップ・画像（1–16 bit、Gray / RGB / CMYK / Indexed / Lab / ICCBased / Separation・DeviceN、JPEG、ステンシル、SMask、色キー）、文字（グリフのアウトライン、Type 3 はグリフ手続き、描画モードの塗り・線・クリップ）。タイル描画、ステップ実行、ハイライト層、ヒットテスト。SVG 書き出し（ページか範囲内を描くもの: パス・線の属性・不透明度、テキストは `<text>`（フォント名・太さ・字形ごとの位置、同じ行は 1 要素に）かグリフのアウトライン、画像は PNG（そのまま見える JPEG はそのまま）、軸・放射シェーディングはグラデーション、タイリングパターン、ソフトマスク。q / Q・テキストオブジェクト・マーク付きコンテンツ・フォーム・透明グループをグループとして残し、オプショナルコンテンツはレイヤー名付きグループ（最上位は Inkscape のレイヤー）、クリップはそれが掛かるグループに） |
| `crates/pdfi-api` | フロントとの境界。`Session` が `Request` → `Response` / `Event` を処理（範囲 API、ジョブ、Batch、バイナリ別フレーム、編集と保存）。TS 型を生成 |
| `crates/pdfi-server` | `pdfi serve` のローカルサーバー（axum + WebSocket、Web UI 内蔵、トークン / Host / Origin / 許可ルート検査） |
| `crates/pdfi-wasm` | ブラウザ完結モード用の wasm バインディング（Worker から `Session` を駆動、Blob から直接読む） |
| `crates/pdfi-ffi` | macOS アプリ用の UniFFI バインディング（プロトコルの型をそのまま Swift に、`Session` をインプロセスで駆動、パスで開く・保存・再読込） |
| `crates/pdfi-cli` | `pdfi` コマンド |
| `apps/macos` | macOS アプリ（AppKit の NSDocument + SwiftUI、`PdfInspectorKit` パッケージ）。骨格: 概要・ページ / オブジェクト / 診断 / フォント / 画像の一覧・ページ描画・ジョブ・undo / 保存の経路。Web UI との対応表は [apps/macos/README.md](apps/macos/README.md) |
| `apps/web` | Web UI（Vite + React + Mantine）。Document / Pages / Outline / Fonts / Images 画面、フォント画面（プログラム・エンコーディング・問題、符号 → グリフ → テキストの表とアウトライン、埋め込みプログラムの全グリフとそれを選ぶ符号、そのフォントで描かれたテキストの一覧（フォーム内も含む）と使われた符号）、命令列ビュー（ネスト・問題・デコード済みテキスト、命令ごとの前後のグラフィックス状態、描画範囲の図とステップ表示）、ページ描画（タイル・ズーム、クリックで命令を選択、選択した命令のハイライト、ステップ実行、矩形で囲んだ範囲を描く命令だけを無劣化で 1 ページの PDF に切り出し、または編集用の SVG に書き出し（テキストは text かアウトライン））、オブジェクト一覧・ツリー・hex・ストリーム表示、被参照一覧、スキーマ型・キー定義・問題の表示と検証ジョブ。編集（ツリーの値を PDF 構文で編集・追加・削除、オブジェクトの追加・削除、ストリームのデータ、ページ操作、Info / XMP）、命令列エディタ（ブロックの折りたたみ・範囲選択・キー操作で挿入 / 編集 / 削除 / 移動 / コピー・貼り付け、CodeMirror のテキストモード: 色分け・折りたたみ・オペランドに合う演算子とリソース名の補完・問題表示・ホバーの説明）と undo / redo、保存（サーバーはパスへ、ブラウザは File System Access API）。サーバー / ブラウザ完結の両モード |

## 使い方

よく使う手順は `Makefile` にまとめてある（`make` で一覧）。

```sh
make setup        # ツールチェーン（wasm32 ターゲット・wasm-bindgen-cli）と npm パッケージ
make cli          # Web UI を埋め込んだ target/release/pdfi
make web          # apps/web/dist（ブラウザ完結モード）
make macos        # macOS アプリ（CONFIGURATION=Debug も可）→ apps/macos/build/DerivedData
make check        # CI と同じ検査（fmt・clippy・テスト・wasm・Swift）
```

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
target/release/pdfi extract file.pdf N --rect x0,y0,x1,y1 -o out.pdf [--overlaps] [--fit] # 範囲内を描く命令だけの 1 ページ PDF
target/release/pdfi svg file.pdf N -o out.svg [--rect x0,y0,x1,y1 [--overlaps] [--fit]] [--outlines] # 編集用の SVG（--outlines: テキストをアウトラインに）
target/release/pdfi edit in.pdf -o out.pdf --info 'Title=New title' --set '1:/PageLayout=/OneColumn' \
    --rotate 1-2:90 --move 5:1 --delete-pages 3 --append other.pdf   # 編集して incremental update で保存
target/release/pdfi edit in.pdf -o out.pdf --full --object-streams --compress --gc   # 全体を書き直して圧縮
target/release/pdfi rewrite in.pdf out.pdf [--reserialize]
```

暗号化ファイルは `--password`、xref を無視して走査で復元するには `--recover`。

### macOS アプリ

```sh
apps/macos/scripts/build-core.sh          # Rust のコア（XCFramework）と Swift バインディングを生成
open apps/macos/PDFInspector.xcodeproj
```

詳しくは [apps/macos/README.md](apps/macos/README.md)。

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
保存は File System Access API でファイルに直接書く（「Open…」の「Browse…」かドラッグ & ドロップで開いたファイルには上書きもできる）。

#### ローカルサーバー

```sh
(cd apps/web && npm ci && npm run build)   # UI をビルド（pdfi に埋め込まれる）
cargo build --release
target/release/pdfi serve file.pdf          # ブラウザで http://127.0.0.1:<port>/#token=... が開く
target/release/pdfi serve --root ~/pdfs     # ディレクトリ配下を UI から開けるようにする
```

UI から開けるのは引数のファイルと `--root` 配下のみ（どちらも無ければカレントディレクトリ）。
保存できるのも同じ範囲（既存のファイルへの上書き、または `--root` 配下の既存のディレクトリへの新しいファイル）。一時ファイルに書いてから置き換える。
UI 開発時は `pdfi serve --port 7878 --dev-origin http://localhost:5173 file.pdf` を起動し、
`npm run dev` の画面を `http://localhost:5173/#token=<token>&server=127.0.0.1:7878` で開く。

## テスト

```sh
cargo test --workspace
UPDATE_TS=1 cargo test -p pdfi-api        # プロトコルを変えたら TS 型（apps/web/src/protocol/generated.ts）を再生成
UPDATE_CONFORMANCE=1 cargo test -p pdfi-api --test conformance   # 適合テストの期待結果を再生成（意図した変更のとき）
(cd apps/web && npm run build:wasm && npm run test:conformance)  # 同じシナリオを wasm で実行
cargo test -p pdfi-ffi                    # 同じシナリオを FFI（macOS の経路）で実行
swift test --package-path apps/macos/PdfInspectorKit             # Swift から（先に apps/macos/scripts/build-core.sh）
tests/fixtures/make.sh                    # フィクスチャ再生成（python3 + fontTools, qpdf が必要）
scripts/make-large-pdf.py big.pdf --pages 3000 --image-kib 100   # 大きな合成 PDF（約 300 MB）
scripts/corpus-check.sh <dir> [--decode]  # 手元の PDF 群で qpdf とページ数を突き合わせ
```
