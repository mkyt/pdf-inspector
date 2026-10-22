# pdf-inspector

PDF の内部構造を調べ、編集するためのツール群。設計は [docs/DESIGN.md](docs/DESIGN.md) を参照。

## 構成（現状）

| crate | 内容 |
|---|---|
| `crates/pdfi-syntax` | ByteSource、lexer / parser、xref（table / stream / hybrid / Prev チェーン）、ObjStm、filters、暗号（RC4 / AES-128 / AES-256）、破損時の復旧、書き出し |
| `crates/pdfi-api` | フロントとの境界。`Session` が `Request` → `Response` / `Event` を処理（範囲 API、ジョブ、Batch、バイナリ別フレーム）。TS 型を生成 |
| `crates/pdfi-server` | `pdfi serve` のローカルサーバー（axum + WebSocket、Web UI 内蔵、トークン / Host / Origin / 許可ルート検査） |
| `crates/pdfi-wasm` | ブラウザ完結モード用の wasm バインディング（Worker から `Session` を駆動、Blob から直接読む） |
| `crates/pdfi-cli` | `pdfi` コマンド |
| `apps/web` | Web UI（Vite + React + Mantine）。オブジェクト一覧・ツリー・hex・ストリーム表示。サーバー / ブラウザ完結の両モード |

## 使い方

```sh
cargo build --release
target/release/pdfi info file.pdf
target/release/pdfi xref file.pdf [--rev N]
target/release/pdfi obj file.pdf 12 [0]
target/release/pdfi stream file.pdf 12 [--raw] [-o out.bin]
target/release/pdfi check file.pdf [--decode] [-v]
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
tests/fixtures/make.sh                    # フィクスチャ再生成（python3, qpdf が必要）
scripts/make-large-pdf.py big.pdf --pages 3000 --image-kib 100   # 大きな合成 PDF（約 300 MB）
scripts/corpus-check.sh <dir> [--decode]  # 手元の PDF 群で qpdf とページ数を突き合わせ
```
