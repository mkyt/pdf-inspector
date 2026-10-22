# pdf-inspector 設計 & 実装プラン（Draft v2）

> 変更履歴:
> - v1: 未決事項への回答を反映（自作レンダラ + PDFium 切替、content stream の命令列編集、大規模ファイル対応、ライセンス方針、React + Mantine）
> - v2: Web UI を **ブラウザ完結（wasm）とローカルサーバー（native）の両対応**に。pdfi-api をトランスポート非依存のメッセージプロトコル化、pdfi-server 追加、マイルストーン再編
> - v2.1: §4.10 macOS インタフェース（pdfi-ffi / PdfInspectorKit）を追加。macOS は最新バージョン前提
> - v2.2: 対象ブラウザを Chromium 系に確定（保存は File System Access API に一本化）

## 1. ゴール

- PDF の **生のオブジェクトモデル**（ファイル上のバイト位置まで）を正確に見せる
- その上に **意味的なビュー**（ページ、メタ情報、画像、フォント、注釈、しおり…）を提供
- Catalog / Page などの **既知スキーマを把握して validation** し、型付きできれいに表示
- content stream の **レンダリング結果・命令列・画像・フォントを可視化**し、相互にリンク
- **編集して保存**: オブジェクト / ストリームの直接編集、**content stream の命令列としての編集**、ページ操作、メタ情報
- **3000 ページ・数百 MB 級の PDF** をローカル実行で快適に扱える
- バックエンドは Rust ライブラリ。フロントは Web（ブラウザ完結, wasm）と macOS（SwiftUI）

### 決定事項（v1）

| 項目 | 決定 |
|---|---|
| レンダラ | **自作レンダラ**と **PDFium** を切り替え可能。Web は自作のみ（wasm） |
| 初期の編集範囲 | オブジェクト / ストリーム直接編集、content stream の命令列編集、ページ操作、メタ情報 |
| 電子署名 | 構造表示のみ。検証は不要。保存時に署名を壊す操作は警告 |
| マクロ | future plan（既存言語を優先して選定）。v1 には含めない |
| Windows | future plan（ネイティブ）。v1 には含めない |
| ライセンス | 未定。**商用利用を妨げない依存のみ**を採用（§8） |
| 大規模ファイル | ローカル実行で数百 MB / 数千ページをサポート。API はすべて遅延・範囲指定（§5） |
| 対象ブラウザ | **Chromium 系**（Chrome / Edge 等）。Safari・Firefox は対象外 |
| Web | **2 モード両対応**: ① ブラウザ完結（Worker 内の wasm、インストール不要）② ローカルサーバー（`pdfi serve`、native の速度・並列性・PDFium・直接保存）。同一の React + Mantine UI |

### 非ゴール（v1）
- WYSIWYG なテキスト / 図形編集（命令列編集で代替）
- 署名検証、PDF/A・PDF/UA 準拠チェック（構造上は後から追加可能にしておく）
- マクロ、Windows フロント（§10 Future）

## 2. 全体アーキテクチャ

```
┌──────────── Frontends ───────────────────────────────────────────────────┐
│ Web UI (React + Mantine)                                │ macOS (SwiftUI) │
│   Backend interface (TS)                                │                 │
│   ├─ WorkerBackend ── postMessage ─┐                    │                 │
│   └─ RemoteBackend ── WebSocket ───┼──┐                 │                 │
└────────────────────────────────────┼──┼─────────────────┴───────▲─────────┘
                    Web Worker       │  │  localhost               │ UniFFI
                 ┌───────────────────▼┐ ┌▼──────────────────────┐  │（型付き直接呼び出し）
                 │ pdfi-wasm          │ │ pdfi-server           │  │
                 │ (wasm-bindgen)     │ │ (`pdfi serve`, native)│  │
                 └─────────┬──────────┘ └──────────┬────────────┘  │
                           └──── dispatch(Request) ┴───────────────┤
┌──────────────────────────────────────────────────────────────────▼───────┐
│ pdfi-api : Session / Handle / Request・Response・Event / Job / EditCommand │
├──────────────────────────────────────────────────────────────────────────┤
│ pdfi-edit    : EditCommand, Transaction, undo/redo, writer         │
│ pdfi-render  : Renderer trait ── raster(自作) │ pdfium(native only) │
│ pdfi-content : CS 解析・命令列モデル・インタプリタ・display list      │
│ pdfi-font / pdfi-image : フォント・画像デコード                      │
│ pdfi-model   : pages, resources, metadata, outline, annots, ...    │
│ pdfi-schema  : Arlington 由来の型定義・型推論・validation             │
│ pdfi-syntax  : ByteSource, lexer, parser, xref, filters, crypt      │
└──────────────────────────────────────────────────────────────────┘
           pdfi-cli（開発・テスト・バッチ。api を叩くだけ）
```

原則:
- **下位層は純粋・I/O 抽象化済み**。ファイル読み込みは `ByteSource` 経由のランダムアクセスのみ（全体をメモリに載せない）
- **壊れた PDF でも落ちない**。エラーは `Diagnostic` として蓄積し読み進める
- **すべてのノードがソース上のバイト範囲 (`Span`) を持つ** → hex ビューと相互ジャンプ
- **何も「全部」返さない**。一覧は件数 + 範囲取得、重い処理はジョブ化（§5）
- フロントは `pdfi-api` にのみ依存。DTO は Rust を単一の真実として各言語の型を生成
- **トランスポート非依存**: pdfi-api はメッセージプロトコル（`dispatch(Request) -> Response` + `Event` ストリーム）として定義し、wasm Worker と WebSocket サーバーが同じ経路を通る
- **下位 crate は常に wasm32 でビルド可能**（スレッド・mmap・ファイルパスはサーバー / FFI 層にのみ置く）。CI で強制

## 3. Crate 構成（cargo workspace）

```
crates/
  pdfi-syntax/    ByteSource, lexer/parser, オブジェクトモデル, xref, filters, 暗号, シリアライザ
  pdfi-schema/    Arlington PDF Model からのコード生成、型推論、validation
  pdfi-model/     意味層（Document / Page / Resources / Metadata / Outline / Annot / Form）
  pdfi-font/      フォント解析（TrueType / CFF / Type1 / Type3 / CMap / ToUnicode）とグリフアウトライン
  pdfi-image/     画像デコード（JPEG / JPX / JBIG2 / CCITT / raw）、色空間変換
  pdfi-content/   content stream の命令列モデル、validation、インタプリタ、display list
  pdfi-render/    Renderer trait, raster バックエンド（自作）, pdfium バックエンド（feature）
  pdfi-edit/      EditCommand, Transaction, undo/redo, 保存（incremental / full）
  pdfi-api/       ファサード + プロトコル。Session, Request/Response/Event, DTO, Job, EditCommand
  pdfi-ffi/       UniFFI バインディング（Swift。将来 C#）
  pdfi-wasm/      wasm-bindgen バインディング（dispatch の公開）+ Blob ベースの ByteSource
  pdfi-server/    ローカルサーバー（axum + WebSocket）、UI 静的ファイル内蔵、認証
  pdfi-cli/       `pdfi dump|obj|xref|ops|validate|render|save|serve ...`
  pdfi-protocol-ts/ （生成物）TS 型定義と Backend クライアント共通部
schema/arlington/ Arlington TSV（vendored, Apache-2.0）
apps/web/         Vite + React + Mantine
apps/macos/       SwiftUI + SwiftPM（XCFramework を参照）
tests/corpus/     取得スクリプトのみ（PDF 本体はリポジトリに入れない）
fuzz/             cargo-fuzz ターゲット
```

## 4. 各層の設計

### 4.1 pdfi-syntax — 生オブジェクトモデル

```rust
pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}
// native: mmap (memmap2) / pread、Web: Blob.slice + FileReaderSync（§5.2）、テスト: Vec<u8>

pub struct ObjId { pub num: u32, pub gen: u16 }
pub struct Span  { pub start: u64, pub end: u64 }

pub enum Object {
    Null, Bool(bool), Integer(i64),
    Real(f64),                                       // 元表記は Span から取得可能
    String { bytes: Vec<u8>, kind: StrKind },        // Literal | Hex
    Name(Name),                                      // #xx デコード済み（元表記は Span から）
    Array(Vec<Node>),
    Dict(Dict),                                      // IndexMap: キー順保持
    Stream(Stream),                                  // dict + data（Span 参照 or owned）
    Ref(ObjId),
}
pub struct Node { pub obj: Object, pub span: Option<Span> }  // 編集で作ったノードは None
```

- **ObjectStore（レイヤ構造）**
  - `Source`: 元ファイル（`ByteSource`、不変）
  - `Revision[]`: incremental update ごとの xref セクション。**リビジョン単位で履歴を表示**
  - `Overlay`: 編集差分（copy-on-write）。未変更オブジェクトは元バイトを保持
  - 遅延パース + **メモリ上限付き LRU キャッシュ**（パース済みオブジェクト・デコード済みストリーム別）
  - ストリームのデータは `Span` で保持し、必要時に読み出す（Dict だけ先にパース）
- **xref**: 従来型テーブル / xref stream / hybrid / `Prev` チェーン / ObjStm。xref テーブル自体はコンパクトな配列（100 万オブジェクトでも数十 MB 未満）
- **リカバリ**: xref 破損時は全体スキャンで `N G obj` を拾い再構築（大きいファイルではジョブとして実行、進捗表示）
- **filters**: Flate, LZW, ASCIIHex, ASCII85, RunLength (+ PNG/TIFF predictor)、Crypt。**ストリーミングデコーダ**（`Read` 合成）として実装し、巨大ストリームでも先頭から必要量だけ取り出せる
- **暗号**: Standard security handler (RC4 40/128, AES-128/256, R2–R6)
- **シリアライザ**: `io::Write` へのストリーミング出力。未変更オブジェクトは元バイトをコピー
- **Diagnostic**: `{ severity, code, span, obj: Option<ObjId>, path, message }`

### 4.2 pdfi-schema — 既知スキーマと validation

- **[Arlington PDF Model](https://github.com/pdf-association/arlington-pdf-model)**（PDF Association による ISO 32000-2 の機械可読モデル、Apache-2.0）を codegen で Rust の静的テーブルに変換
- **型推論**: Trailer → `/Root` は Catalog → `/Pages` は PageTreeNode → `/Kids` は `/Type` で Page/PageTreeNode… とリンク情報を辿って各オブジェクトに「スキーマ型」を割り当てる。複数候補は条件（`/Type`, `/Subtype`, キーの有無）で絞る
- **validation**: 型不一致、必須キー欠落、未知キー、値域外、バージョン不整合、indirect 必須違反、参照切れ、循環
- **スコープ別実行**: 「このオブジェクト」「このページとその配下」「ドキュメント全体（ジョブ）」
- UI 向けにキーの説明・仕様の節番号・導入/廃止バージョンを DTO に含める

### 4.3 pdfi-model — 意味層

- `Document`: version, trailer, catalog, Info, **XMP（ツリー化）**, 暗号情報, リビジョン一覧, 統計
- `Page`: 継承属性解決済み（Resources, MediaBox, CropBox, Rotate）＋「どこから継承したか」、各 Box、content streams、annots
  - **ページツリーは `/Count` を使って index → ページを O(深さ) で引く**。全ページを列挙しない
- `Resources`: Font / XObject / ColorSpace / Pattern / Shading / ExtGState / Properties
- その他: Outline、Name tree / Number tree、Annotation、AcroForm、EmbeddedFiles、Signature（構造のみ）、StructTree
- **インベントリ**（全フォント・全画像・被参照グラフ）は全オブジェクト走査が要るので**ジョブ**として構築し、結果を範囲取得

### 4.4 pdfi-font / pdfi-image

- **フォント**: Type1 / TrueType / Type0 (CID) / Type3。埋め込み（FontFile / FontFile2 / FontFile3）、Encoding / Differences、CMap（定義済み CMap 含む）、ToUnicode、Widths
  - アウトライン: TrueType / CFF / OpenType は `skrifa`、Type1 は自前パーサ（`hayro-font` を評価）
  - 非埋め込みの標準 14 フォント等の代替フォントは**ライセンスに注意**（§8）
  - UI: グリフ一覧、コード → グリフ → Unicode の対応表、メトリクス
- **画像**: 寸法、BPC、ColorSpace（Device*, Cal*, Lab, ICCBased, Indexed, Separation, DeviceN）、Filter、SMask / Mask、Decode、Interpolate
  - RGBA へのデコード。大きい画像は**縮小デコード / 領域デコード**の API を用意（数万 px 四方の画像対策）

### 4.5 pdfi-content — 命令列モデルと編集

```rust
pub struct ContentProgram {                    // 1 つの「論理 content」
    pub sources: Vec<ContentSource>,           // ページの /Contents 配列の各ストリーム
    pub ops: Vec<Op>,
}
pub struct Op {
    pub operator: Operator,                    // 既知 enum + Unknown(raw)
    pub operands: Vec<Node>,
    pub origin: Option<OpOrigin>,              // { source_index, span }。編集で作った op は None
}
pub enum OpPayload { InlineImage { dict, data } /* BI … ID … EI */ }
```

- 対象: ページ content、Form XObject、Type3 CharProcs、Tiling Pattern、注釈の Appearance Stream — **すべて同じモデルで閲覧・編集**
- ページの `/Contents` が配列の場合は連結して 1 つの命令列として扱い、各 op の出所（どのストリームの何バイト目か）を保持
- **validation**: オペランド数・型、`q/Q`・`BT/ET`・`BMC/EMC` のネスト、テキストオブジェクト内でのみ有効な演算子、未定義リソース参照、互換セクション（`BX/EX`）
- **編集操作**（`EditCommand` の一部）: `InsertOps`, `RemoveOps`, `ReplaceOp`, `MoveOps`, `SetOperand`
  - 命令列の構造的編集のほか、**テキストモードで範囲を書き換え → 再パース → 命令列差分**として適用することも可能に
  - テキスト表示演算子（`Tj`, `TJ` など）は、フォントのエンコーディングで**デコードした文字列を併記**（編集時の再エンコード補助は将来）
- **シリアライズ**: 変更のない op は元バイトを再利用、変更箇所だけ再生成（差分が最小になる）。保存時に「元のストリーム分割を維持」「1 本に統合」を選択可能、フィルタ（Flate 等）も選択可能
- **大きな content stream** に備え、`ops(range)` で範囲取得、命令列はページ単位で遅延構築・キャッシュ

### 4.6 インタプリタと pdfi-render — レンダリング

- **インタプリタ**（pdfi-content）: グラフィックス状態・テキスト状態を追跡し **display list** を生成
  - 要素（path / glyph run / image / shading / clip / group）は **元の op index とページ座標の bbox** を持つ
  - → 「レンダ画像をクリック → 該当命令へ」「命令を選択 → 画像上をハイライト」「op N まで実行した状態で描画（ステップ実行）」
  - Form XObject / Type3 / パターンは再帰（深さ・回数制限あり）
- **Renderer trait**

```rust
pub trait Renderer {
    fn render(&self, page: &PageCtx, req: &RenderRequest, cancel: &CancelToken) -> Result<Bitmap>;
}
pub struct RenderRequest { scale: f32, clip: Option<Rect>, until_op: Option<usize>, highlight: Vec<OpIndex>, ... }
```

- **raster バックエンド（自作）**: display list → 2D ラスタライザ。ラスタライザは `tiny-skia`（BSD-3）と `vello_cpu`（Apache-2.0/MIT）を評価して採用。wasm / native 共通
  - 段階的に対応: path・塗り・線・クリップ・画像 → テキスト（埋め込み / 代替フォント）→ shading・pattern → 透明度グループ・soft mask・blend mode → 特殊色空間（Separation / DeviceN / overprint は近似）
- **pdfium バックエンド（native only, cargo feature）**: `pdfium-render` 経由。動的ロードで PDFium バイナリを同梱。リファレンス表示 / 自作レンダラとの**差分比較ビュー**にも使う
  - PDFium には編集後のドキュメントを渡す必要があるため、Overlay を含めたバイト列をストリーミング出力して渡す（大きいファイルでは incremental 形式の一時ファイルで高速化）
- **タイルレンダリング**: `clip` 指定で表示中の領域だけ描画。サムネイルは低倍率で別キャッシュ。古いリクエストは `CancelToken` でキャンセル

### 4.7 pdfi-edit — 編集と保存

- 編集はすべて **シリアライズ可能な `EditCommand`** で表現（undo/redo、将来のマクロ記録、フロント共通）
  - オブジェクト: `SetDictEntry`, `RemoveDictEntry`, `ReplaceObject`, `AddObject`, `DeleteObject`, `ArrayInsert/Remove`, `SetStreamData { encode }`, `SetStreamFilter`
  - content stream: §4.5 の命令列操作
  - ページ: 削除、並べ替え、回転、挿入（空ページ / 他 PDF から）、ページツリー再構成
  - メタ情報: Info 辞書、XMP（Info と XMP の同期オプション）
- `Transaction` でまとめて適用・まとめて undo。適用後に影響範囲の validation を自動実行
- **保存**（いずれも `io::Write` へのストリーミング出力）
  - **Incremental update**: 元バイトをそのままコピー + 変更分を末尾追記。署名を壊さない。大きいファイルで最速
  - **Full rewrite**: xref stream / ObjStm 使用有無、再圧縮、未使用オブジェクト除去を選択
  - 署名付き PDF で full rewrite / 署名範囲に影響する変更をする場合は警告

### 4.8 pdfi-api — フロントとの境界（プロトコル）

pdfi-api は **Rust API とメッセージプロトコルの二面**を持つ。中身は同一で、プロトコル側は Rust API への薄い dispatch。

```rust
pub struct Envelope<T> { pub id: RequestId, pub body: T }
pub enum Request  { Open(OpenSource), Summary(DocId), Object(DocId, ObjId), Children { .. }, Render { .. },
                    StartJob { .. }, CancelJob(JobId), Cancel(RequestId), Apply { .. }, Save { .. }, Batch(Vec<Request>), ... }
pub enum Response { Ok(ResponseBody), Err(ApiError) }
pub enum Event    { JobProgress { .. }, JobPartial { .. }, JobDone { .. }, Invalidated { doc, changes }, ... }

impl Session {
    pub fn dispatch(&mut self, req: Envelope<Request>) -> Envelope<Response>;
    pub fn events(&self) -> EventReceiver;
}
```

- **フレーミング**: 制御メッセージは JSON（デバッグ容易性優先。必要になれば MessagePack に切替可能な設計）。**バイナリは別フレーム**で `{ request_id, part }` ヘッダ付きで送る（WebSocket binary frame / Worker の Transferable `ArrayBuffer`）
- **コールバック・クロージャを API に含めない**。非同期性はすべて RequestId / JobId とイベントで表現
- **キャンセル**: 任意のリクエスト（特に Render）を `Cancel(RequestId)` で取り消せる。古いタイル要求の破棄に使用
- **Batch**: 1 往復で複数リクエスト（ツリー展開 + インスペクタ情報など）。先読みもこれで行う
- **型生成**: Rust の型から TS 型を生成（`ts-rs` / `specta` / `tsify` を比較して選定）。Swift は UniFFI で Rust API を直接呼ぶ（プロトコルを経由しない）
- **バージョニング**: ハンドシェイクでプロトコルバージョンを交換（server と UI のバージョン不一致検出）

Rust API（論理的な操作一覧）:

```rust
Session::open(source: OpenSource, password: Option<String>) -> OpenResult
// OpenSource = ByteSource（wasm: Blob / FFI: パス）| ServerPath（サーバーが許可したパス）
// ---- 概要・生構造（すべて範囲指定）----
doc.summary() -> DocSummary                          // version, page_count, object_count, encrypt, revisions, diag count
doc.object(ObjId) -> ObjectView                      // 1 階層 + schema 型 + 説明 + diagnostics
doc.children(NodePath, Range) -> Page<NodeView>      // 大きい配列・辞書も分割取得
doc.objects(Range, filter) -> Page<ObjectSummary>    // オブジェクト一覧（xref 順）
doc.xref_sections() / revisions()
doc.raw_bytes(Span) -> Bytes                         // hex ビュー（上限付き）
doc.stream_data(ObjId, decoded: bool, Range) -> Bytes
// ---- 意味層 ----
doc.pages(Range) -> Page<PageSummary>;  doc.page(index) -> PageDetail
doc.content_ops(ContentRef, Range) -> Page<OpView>
doc.render(ContentRef, RenderRequest) -> RenderHandle   // 非同期、キャンセル可
doc.hit_test(ContentRef, x, y) -> Vec<OpIndex>
doc.font(ObjId) / glyphs(ObjId, Range) / glyph_outline(...)
doc.image(ObjId) / decode_image(ObjId, ImageRequest)    // 縮小・領域指定
// ---- 重い処理はジョブ ----
doc.start_job(JobKind) -> JobId      // FullValidation, Inventory(Fonts|Images), ReverseRefs, Recovery, Save
job.poll() -> JobStatus { progress, partial_results_available }
job.results(Range) / job.cancel()
// ---- 編集 ----
doc.apply(Vec<EditCommand>) -> ApplyResult;  doc.undo() / redo()
doc.save(SaveOptions, sink: Box<dyn ByteSink>) -> JobId
```

設計上の要点:
- **ハンドルベース**: フロントはドキュメント全体のモデルを持たない。必要な部分だけ問い合わせる（仮想スクロールと相性が良い）
- **範囲 API の統一型** `Page<T> { total: u64, offset: u64, items: Vec<T> }`
- **ジョブモデル**は native（スレッド）と wasm（協調的に `step(budget)` を回す）の両方で動く同一インタフェース。進捗・部分結果は `Event` として push
- **バイナリは JSON にしない**: `Bytes` → Swift `Data` / JS `Uint8Array`。ビットマップは JS 側で `ImageBitmap` 化
- **無効化通知**: `apply` の結果として `Event::Invalidated`（変更されたオブジェクト / ページ / content）を発行し、フロントはそのキャッシュだけ捨てる。サーバーモードで複数タブが同じドキュメントを開いていても整合する
- DTO は `serde` + `uniffi::Record` を feature で derive

### 4.9 pdfi-server — ローカルサーバーモード

- `pdfi serve [file.pdf ...]` で起動 → `http://127.0.0.1:<port>/#token=...` をブラウザで開く
- axum + tokio。Web UI のビルド成果物を**バイナリに内蔵**して配信（単一バイナリで完結）
- WebSocket 1 本でリクエスト / レスポンス / イベント / バイナリを多重化
- **ファイルの開き方**: ブラウザからはパスを渡せないため、CLI 引数 / サーバー側ファイルブラウザ（ユーザーが起動時に許可したルート配下のみ）/ 最近使ったファイル。巨大ファイルのアップロードはしない
- **保存**: サーバー側で直接パスに書き出し（上書き・別名保存）。上書き時は一時ファイル → atomic rename
- **セッション**: ドキュメント状態（Overlay, undo 履歴）はサーバーが保持。ブラウザのリロード・再接続後も継続。明示的に閉じるかアイドルタイムアウトで解放
- **並列性**: ジョブ・タイル描画を `rayon` で並列実行。PDFium バックエンドも選択可能
- **セキュリティ**（必須要件）
  - `127.0.0.1` / `::1` のみで listen（外部公開オプションは設けない）
  - 起動ごとにランダムトークンを生成。URL フラグメントで UI に渡し、WebSocket ハンドシェイクで検証（Cookie は使わない）
  - `Host` ヘッダが `127.0.0.1:<port>` / `localhost:<port>` 以外は拒否（DNS rebinding 対策）、`Origin` を検証（CSRF / 他サイトからの WebSocket 接続対策）
  - ファイルアクセスは許可ルート配下のみ。パスの正規化・シンボリックリンク解決後に再検査
  - 解析対象の PDF は信頼しない入力として扱う（fuzz 済みのパーサ、再帰・メモリ上限）

### 4.10 pdfi-ffi / macOS インタフェース

macOS アプリは Web 用プロトコルを経由せず、**UniFFI で Rust の型付き API をインプロセスで直接呼ぶ**。

```
SwiftUI / AppKit App (PDFInspector.app)
  └─ PdfInspectorKit  … 手書き Swift パッケージ（actor, @Observable, CGImage 変換, AsyncStream）
       └─ PdfiCore    … UniFFI 生成の Swift バインディング + XCFramework（静的ライブラリ）
            └─ pdfi-ffi (Rust) … pdfi-api の Rust API を UniFFI 向けに公開する薄い層
```

#### Rust 側（pdfi-ffi）

```rust
#[derive(uniffi::Object)]
pub struct PdfSession { inner: Mutex<pdfi_api::Session> }   // Arc 共有のため Send + Sync

#[uniffi::export]
impl PdfSession {
    #[uniffi::constructor]
    fn new(config: SessionConfig) -> Arc<Self>;
    fn open_path(&self, path: String, password: Option<String>) -> Result<DocId, ApiError>;
    fn summary(&self, doc: DocId) -> Result<DocSummary, ApiError>;
    fn children(&self, doc: DocId, path: NodePath, range: Range) -> Result<NodeViewPage, ApiError>;
    async fn render(&self, doc: DocId, target: ContentRef, req: RenderRequest,
                    cancel: Arc<CancelToken>) -> Result<Bitmap, ApiError>;   // Swift: async throws
    fn start_job(&self, doc: DocId, kind: JobKind) -> Result<JobId, ApiError>;
    fn apply(&self, doc: DocId, cmds: Vec<EditCommand>) -> Result<ApplyResult, ApiError>;
    fn save_to_path(&self, doc: DocId, opts: SaveOptions, path: String) -> Result<JobId, ApiError>;
    fn set_event_listener(&self, listener: Box<dyn EventListener>);
}

#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {          // Swift 側で実装
    fn on_event(&self, event: Event);
}
```

- **DTO はプロトコルと共通**: 同じ型に `serde` と `uniffi::Record` / `uniffi::Enum` を derive。`ApiError` は Swift の `throws` になる
- **DTO 設計上の制約（UniFFI / 将来の C# 生成に共通）**
  - ジェネリクス不可 → `Page<T>` は `NodeViewPage` 等の具体型をマクロで生成
  - 再帰型を避ける → オブジェクトツリーは 1 階層分のフラットな `NodeView` を返し、子は遅延取得（大規模対応とも一致）
  - ライフタイム・参照を含まない所有型のみ
- **キャンセル**: Swift Task のキャンセルが Rust 側に確実に伝わる保証に依存せず、明示的な `CancelToken` を渡す。Kit 側で `withTaskCancellationHandler` と接続
- **バイナリ**: `Vec<u8>` → `Data`（コピー 1 回）
- **ずれ防止**: Rust API を単一の真実とし、プロトコルの `dispatch` と pdfi-ffi はどちらもその関数を機械的に呼ぶだけにする

#### Swift 側（PdfInspectorKit）

```swift
public actor DocumentSession {
    public func summary() async throws -> DocSummary
    public func children(of path: NodePath, range: Range<Int>) async throws -> NodeViewPage
    public func renderTile(_ target: ContentRef, _ req: RenderRequest) async throws -> CGImage
    public var events: AsyncStream<Event> { get }      // EventListener → AsyncStream
    public func apply(_ cmds: [EditCommand]) async throws -> ApplyResult
}

@Observable final class PageListModel { … }         // 範囲取得・キャッシュ・Invalidated で部分破棄
```

- Rust の同期 API はバックグラウンドで呼び、UI スレッドを塞がない
- `EventListener` は Rust のスレッドから呼ばれるため、`AsyncStream` に流して `@MainActor` のモデルで受ける
- `Bitmap` → `CGImage` 変換とタイルキャッシュは Kit 側

#### macOS 固有の設計判断
- **ドキュメント管理は `NSDocument`**: SwiftUI の `FileDocument` / `ReferenceFileDocument` は `FileWrapper` 経由で内容を読むため、数百 MB の PDF には不向き。`NSDocument.read(from: URL, ofType:)` で URL を受け取り、Rust に渡して mmap させる。保存は `write(to:ofType:)` で受けた一時 URL へ Rust がストリーミング出力し、置き換えは NSDocument に任せる（undo は Rust 側の履歴を `NSUndoManager` と連携）
- **UI は AppKit の土台（NSDocument / NSWindowController）に SwiftUI の画面を `NSHostingView` で載せる**構成
- **App Sandbox**: パネル・ブックマークで得た URL は Swift 側で security-scoped access を開始してから Rust にパスを渡し、mmap している間は維持する
- **ビルド・配布**: `aarch64-apple-darwin` / `x86_64-apple-darwin` をビルド → `lipo` → `xcodebuild -create-xcframework` → SwiftPM の `binaryTarget` + 生成 Swift ソースでパッケージ化。PDFium は dylib としてアプリに同梱（M13）
- **並列処理**: `rayon` のスレッドプールは Rust 側で保持。Swift 側で並列化しない
- **最低バージョン**: 最新の macOS を前提とする（デプロイメントターゲットは開発開始時点の最新メジャー）。Observation (`@Observable`)、最新の SwiftUI API を制約なく使う

## 5. 大規模ファイル対応（3000 ページ / 数百 MB）

### 5.1 共通方針
| 課題 | 対策 |
|---|---|
| ファイル全体の読み込み | `ByteSource` のランダムアクセスのみ。native は mmap |
| 起動時間 | 起動時は trailer + xref のみ。ページツリー・オブジェクトは要求時に解析 |
| メモリ | パース済みオブジェクト / デコード済みストリーム / グリフ / 画像それぞれに上限付き LRU |
| 一覧表示 | すべて範囲 API + 仮想スクロール。件数だけ先に返す |
| 全体走査（validation, インベントリ, 被参照） | ジョブ化、進捗・キャンセル・部分結果。結果はインデックスとして保持 |
| 描画 | タイル + 表示範囲のみ、キャンセル、サムネイルは低解像度キャッシュ |
| 保存 | ストリーミング出力。incremental は元バイトのチャンクコピー + 追記 |
| 編集データ | Overlay に保持。巨大ストリームの差し替えは一時ストレージ（native: temp file、Web: OPFS）に逃がす |

### 5.2 モードの使い分け

| | ブラウザ完結（wasm） | ローカルサーバー（native） |
|---|---|---|
| 想定用途 | インストールなしでさっと見る、中小規模の PDF | 数百 MB / 数千ページ、全体 validation、重い編集 |
| 性能 | native の 1.5〜2 倍程度遅い、単一スレッド | native、マルチスレッド |
| レンダラ | 自作のみ | 自作 / PDFium |
| 保存 | File System Access API | パスへ直接 |
| 状態の永続性 | タブ単位 | サーバープロセス単位（リロード耐性あり） |

UI は同じものを使い、起動経路（静的ホスティング or `pdfi serve`）でバックエンドが決まる。UI 上には現在のモードと、重い処理をブラウザモードで実行する際の注意（「サーバーモード推奨」）を表示する。

### 5.3 Web（ブラウザ完結モード）での実現
- 解析は **Dedicated Worker 内の wasm** で実行。UI スレッドとは Comlink 等で型付き RPC
- **ファイルは wasm メモリにコピーしない**: ユーザーが選んだ `File`（`Blob`）を Worker に渡し、`ByteSource::read_at` を **`Blob.slice()` + `FileReaderSync`（Worker 内で同期読み出し可能）** で実装。wasm32 の 4 GB メモリ制約に依存しない
- ファイルを開く: `showOpenFilePicker` / ドラッグ&ドロップ。`FileSystemFileHandle` を IndexedDB に保存して「最近使ったファイル」を再オープン可能に（再許可プロンプトあり）
- 書き出し: **File System Access API**（`showSaveFilePicker` → `FileSystemWritableFileStream`）へ直接ストリーミング。上書き保存も同じハンドルに対して行う（Chromium は一時ファイルに書いてから置き換えるため安全）
- 一時データ（巨大ストリーム差し替え等）は OPFS（`FileSystemSyncAccessHandle`）に置く
- Chromium 前提で使える機能: wasm SIMD、Memory64（必要になった場合の選択肢。初期は wasm32 + 上限付きキャッシュで設計）。マルチスレッド（SharedArrayBuffer）は COOP/COEP が必要なため初期は使わない
- 初版は単一 Worker。将来、レンダリング専用 Worker を複数立てる（読み取り専用 Source を共有し、Overlay のスナップショットを渡す）

### 5.4 ローカルサーバーモード
- `ByteSource` は mmap。キャッシュ上限はブラウザモードより大きく設定
- localhost 往復のレイテンシは小さいが、ツリー展開などの細粒度アクセスは Batch と先読みで往復回数を抑える
- ビットマップは WebSocket バイナリフレームで転送（必要なら PNG/QOI 圧縮を選択可能に）

### 5.5 macOS
- `ByteSource` は mmap（NSDocument から受けた URL）。重い API は Kit の actor 経由で `async` として呼ぶ（§4.10）
- ジョブはバックグラウンドスレッド、レンダリングはスレッドプール（`rayon`）でタイル並列

## 6. フロントエンド

### 共通の画面構成
- 左: ナビゲーション（Document / Pages / Objects / XRef & Revisions / Fonts / Images / Diagnostics）
- 中央: 型付き辞書ビュー / ページレンダ / **命令列エディタ** / 画像 / グリフ表
- 右: インスペクタ（スキーマ説明、参照元一覧、diagnostics、編集フォーム）
- 下: hex ビュー（Span と同期）

### 命令列エディタ
- 表形式: `#`, operator, operands（型付きで編集可）, 状態（ネスト深さ、現在の CTM / フォント等）, diagnostics
- 挿入・削除・ドラッグ並べ替え・複数選択、`q…Q` / `BT…ET` を折りたたみ可能なブロック表示
- テキストモード（ソース表示・編集 → 再パースして差分適用）
- レンダプレビューと双方向ハイライト、ステップ実行スライダー

### Web
- Vite + TypeScript + **React + Mantine**
- **`Backend` インタフェース**（`request`, `batch`, `cancel`, `onEvent`）を UI の唯一の窓口とし、`WorkerBackend`（wasm in Worker）と `RemoteBackend`（WebSocket）を実装。UI コンポーネントはモードを意識しない
- 同じビルド成果物を「静的ホスティング（ブラウザ完結）」と「`pdfi serve` 内蔵」の両方で使う。起動時にトークンの有無などでモードを判定
- 仮想スクロール: `@tanstack/react-virtual`。API 呼び出しのキャッシュ・無効化: TanStack Query（範囲 API と無効化通知に合う）
- 描画: `<canvas>` にタイル単位で `ImageBitmap` を描画

### macOS
- NSDocument ベースのアプリに SwiftUI の画面を載せる。Rust は XCFramework + SwiftPM パッケージ（PdfiCore）、その上に PdfInspectorKit（§4.10）
- `CGImage` で描画。大きいリストは `List` / `Table` の遅延ロード

## 7. 品質・テスト

- 単体テスト（各層）、**round-trip テスト**（parse → write → parse で等価、未変更なら byte 一致）
- **命令列 round-trip**（ops → serialize → parse で一致、未変更 op は byte 一致）
- **cargo-fuzz**: lexer / parser / xref / filters / content stream / フォント / 画像
- レンダリングの回帰テスト: 自作 vs PDFium の画像差分（しきい値付き）をコーパスで実行
- 大規模ファイルのベンチマーク: 起動時間、ページ i の表示時間、ピークメモリ（CI では合成した巨大 PDF を使用）
- **プロトコル適合テスト**: 同一のテストシナリオを Rust 直呼び / wasm（Node or headless browser）/ サーバー（WebSocket）/ UniFFI（Swift の XCTest）の 4 経路で実行し、結果が一致することを確認
- サーバーのセキュリティテスト: 不正 Host / Origin / トークン、許可ルート外パス、シンボリックリンク
- コーパス: pdf.js / qpdf / PDF Association 等の公開テストファイル（取得スクリプトで管理）
- 参照実装との比較: `qpdf --check`, `qpdf --json` の出力と突き合わせ
- CI: `cargo test` / `clippy` / **`cargo-deny`（ライセンス・脆弱性）** / **下位 crate の `wasm32-unknown-unknown` ビルド** / wasm ビルド / XCFramework ビルド / web 型チェック

## 8. ライセンス方針と依存ライブラリ

**方針**: 製品に組み込む依存は MIT / Apache-2.0 / BSD / Zlib / ISC 等の permissive のみ。MPL-2.0 はファイル単位 copyleft なので「改変しない前提で可」。**GPL / LGPL / AGPL は不可**。`cargo-deny` で CI 強制。

| 用途 | 候補 | ライセンス | 備考 |
|---|---|---|---|
| Flate | `flate2` + `miniz_oxide` or `zlib-rs` | MIT/Apache, Zlib | |
| LZW | `weezl` | MIT/Apache | |
| JPEG | `zune-jpeg` | MIT/Apache/Zlib | |
| JPEG 2000 | `hayro-jpeg2000`（純 Rust）/ OpenJPEG | Apache/MIT / BSD-2 | 要評価。wasm では純 Rust 優先 |
| JBIG2 | 純 Rust 実装を評価、なければ自前 | — | **jbig2dec は AGPL のため不可** |
| CCITT | `fax` | MIT | 要評価 |
| ICC | `moxcms` / `qcms` | 要確認 / MIT | |
| フォント | `skrifa` / `read-fonts`, `ttf-parser` | MIT/Apache | Type1 は `hayro-font` を評価 |
| 代替フォント | PDFium 同梱の Foxit 標準フォント / Liberation 等 | BSD / OFL | **URW base35 は AGPL のため不可** |
| ラスタライザ | `tiny-skia` / `vello_cpu` | BSD-3 / Apache/MIT | |
| PDFium | `pdfium-render` + PDFium バイナリ | MIT/Apache + BSD-3/Apache | PDFium 内の FreeType は FTL（表示義務あり） |
| 暗号 | RustCrypto（`aes`, `cbc`, `md-5`, `sha2`, `rc4`） | MIT/Apache | |
| XMP | `quick-xml` | MIT | |
| スキーマ | Arlington PDF Model | Apache-2.0 | |
| バインディング | `wasm-bindgen`, `ts-rs` / `specta` / `tsify` / `uniffi` | MIT/Apache / MPL-2.0 | |
| サーバー | `axum`, `tokio`, `rust-embed`, `rayon` | MIT（/Apache） | |
| Web UI | React, Mantine, TanStack | MIT | |

参照のみ（コードは取り込まない）: pdf.js（Apache-2.0、挙動比較）、MuPDF / Poppler / Ghostscript（AGPL/GPL、使用しない）。

## 9. 実装プラン（マイルストーン）

各マイルストーンで「CLI か UI で触れる成果物」を出す。M2〜M4 で**全バックエンド経路（server / wasm / FFI）を早期に貫通**させ、プロトコル・DTO・大規模ファイル対応の手戻りを最小化する。

| # | 内容 | 成果物 |
|---|---|---|
| **M0** | workspace、CI（test / clippy / cargo-deny / 下位 crate の wasm32 ビルド / XCFramework）、コーパス取得、Diagnostic 基盤、巨大 PDF 合成ツール | CI が回る |
| **M1** | pdfi-syntax: ByteSource, lexer/parser, Span, xref 全形式 + Prev + ObjStm, ストリーミング filters, リカバリ, 暗号, シリアライザ | `pdfi dump / obj / xref / stream` |
| **M2** | pdfi-api 最小版（プロトコル・範囲 API・ジョブ・イベント・TS 型生成）+ **pdfi-server + Web 最小ビューア**（RemoteBackend。オブジェクトツリー、生表示、hex、ストリーム表示）+ セキュリティ要件 | `pdfi serve` で数百 MB の PDF を開いて歩ける |
| **M3** | **pdfi-wasm + WorkerBackend**（Blob ByteSource、協調ジョブ）、プロトコル適合テスト | 同じ UI がブラウザ完結でも動く |
| **M4** | **pdfi-ffi + PdfInspectorKit + macOS 最小アプリ**（NSDocument + SwiftUI、M2 と同等機能、XCTest による適合テスト） | Swift から同じ API |
| **M5** | pdfi-schema: Arlington codegen、型推論、validation（スコープ別・ジョブ）、型付き表示 | Catalog/Page 等がラベル・警告付きで見える |
| **M6** | pdfi-model: ページ（継承解決・高速 index）、Info/XMP、Outline、Annot、インベントリジョブ、被参照グラフ | Document / Pages / Fonts / Images 画面 |
| **M7** | pdfi-content: 命令列モデル、validation、命令列ビュー（閲覧のみ）、インタプリタ + bbox | 命令列と状態の可視化 |
| **M8** | pdfi-render 自作 (1): path / 塗り / 線 / クリップ / 画像（基本色空間）、タイル描画（server では並列）、ヒットテスト・ハイライト・ステップ実行 | ページ描画 ⇔ 命令列同期 |
| **M9** | pdfi-font + 自作レンダラ (2): 埋め込みフォント（TrueType / CFF / Type1 / Type3 / CID）、代替フォント、グリフ表示・ToUnicode | テキストが描ける、フォント画面 |
| **M10** | pdfi-edit: EditCommand、undo/redo、incremental / full 保存（ストリーミング。server はパスへ直接、wasm は File System Access API）、オブジェクト・ストリーム編集、ページ操作、メタ情報 | 編集して保存 |
| **M11** | 命令列エディタ（構造編集 + テキストモード）、命令列シリアライズ | content stream を命令列として編集 |
| **M12** | 自作レンダラ (3): shading、pattern、透明度、soft mask、blend、ICC / Separation / DeviceN、JPX / JBIG2 / CCITT | 描画の網羅性 |
| **M13** | pdfium バックエンド（server / macOS）、自作との差分比較ビュー、レンダリング回帰テスト | レンダラ切替 |

※ M10 は M7 と並行着手可能（依存は M1/M6 のみ）。M11 は M7 + M10 に依存。M3 は M2 直後に置き、wasm 制約を早期に検証する。

## 10. Future plan

### マクロ
- 方針: **既存言語**を採用し、スクリプトは `pdfi-api` の query と `EditCommand` だけにアクセス（ファイル I/O なし）。UI 操作の記録 → スクリプト生成も `EditCommand` から機械的に可能
- 埋め込み方式の候補

| 方式 | 言語 | 長所 | 短所 |
|---|---|---|---|
| backend に純 Rust 処理系を埋め込み | JavaScript（Boa, MIT） / Rhai（MIT/Apache） | 全プラットフォームで同一挙動、wasm 可 | Boa は速度・互換性、Rhai は独自言語 |
| backend に C 製処理系を埋め込み | JavaScript（QuickJS / `rquickjs`, MIT）/ Lua（MIT） | 成熟・高速 | wasm ビルドが手間 |
| フロント側のホスト JS 処理系で実行 | JavaScript（ブラウザ / macOS は JavaScriptCore） | 追加の処理系不要、Web は最速 | プラットフォームごとにバインディングが必要、Windows で別途必要 |

- 現時点の第一候補: **JavaScript**（言語として既存・型定義を TS で提供できる）。処理系は M13 以降に Boa と QuickJS を比較評価して決定
- プロトコル化により、**Web UI 上のマクロはブラウザの JS 処理系で実行し `Backend` 経由で操作**する方式も、両モード共通で取れる（サーバーモードでもスクリプトはブラウザ側で動く）

### Windows
- ネイティブ（WinUI 3 + C#）。バインディングは UniFFI の C# 生成（`uniffi-bindgen-cs`）を第一候補。pdfi-api の DTO 設計は C# からも使える形（ジェネリクスに依存しない）を維持する

### その他
- 署名検証、PDF/A・PDF/UA チェック（validation 基盤の拡張として）
- テキスト表示演算子の再エンコード補助（Unicode → フォントのコード）
- Linearize 出力、複数 Worker による並列レンダリング

## 11. 未決事項（残り）

1. ライセンス（OSS / 商用）の最終決定は後回しで可。決まり次第 §8 を確定
