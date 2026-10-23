# pdf-inspector 設計 & 実装プラン（Draft v2）

> 変更履歴:
> - v1: 未決事項への回答を反映（自作レンダラ + PDFium 切替、content stream の命令列編集、大規模ファイル対応、ライセンス方針、React + Mantine）
> - v2: Web UI を **ブラウザ完結（wasm）とローカルサーバー（native）の両対応**に。pdfi-api をトランスポート非依存のメッセージプロトコル化、pdfi-server 追加、マイルストーン再編
> - v2.1: §4.10 macOS インタフェース（pdfi-ffi / PdfInspectorKit）を追加。macOS は最新バージョン前提
> - v2.2: 対象ブラウザを Chromium 系に確定（保存は File System Access API に一本化）
> - v2.3: M2 実装で確定した事項を §4.8 / §4.9 に追記（TS 型生成は ts-rs、バイナリフレーム形式、ジョブ実行器、トークンの渡し方）
> - v2.4: M3 実装で確定した事項を §5.3 / §7 に追記（JS 側 ByteSource、Worker ホストのスケジューリング、ファイルを開く経路、適合テストの形）
> - v2.5: M4（macOS）を後回しにして M5 を先に実装。確定した事項を §4.2 / §4.8 に追記（codegen の形、述語の扱い、型推論、validation のスコープとジョブ、プロトコル v2）
> - v2.6: M6 実装で確定した事項を §4.3 / §4.8 に追記（ページ索引、継承属性の出所、メタ情報、インベントリと被参照グラフ、プロトコル v3）
> - v2.7: M7 実装で確定した事項を §4.5 / §4.6 / §4.8 に追記（命令列の読み直しとチェックポイント、validation の項目、bbox の求め方、暫定のフォント情報、プロトコル v4）
> - v2.8: M8 実装で確定した事項を §4.6 / §4.8 / §4.9 / §8 に追記（ラスタライザは tiny-skia、Device による描画、ビュー座標、タイルと余白、ハイライト層とヒットテスト、プロトコル v5、サーバーでの並列描画）
> - v2.9: M9 実装で確定した事項を §4.4 / §4.5 / §4.6 / §4.8 / §8 に追記（pdfi-font の形、ttf-parser と hayro-font、グリフ選択、Foxit 代替フォントと AFM 幅、Type 3 の手続き実行、テキストのクリップ、プロトコル v6）

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
- **M5 で確定した形**:
  - `schema/arlington/` に Arlington の `tsv/latest` を vendoring（取り込んだ commit は同ディレクトリの README）。`pdfi-schema` の `build.rs` が TSV を静的テーブル（`TYPES: [TypeDef; N]`）に変換する。実行時に TSV は読まない
  - 述語（`fn:...`）は codegen で 3 種に落とす: ① バージョン系ラッパ（`fn:SinceVersion` / `fn:Deprecated` / `fn:IsPDFVersion` / `fn:Extension`）は version フィールドに、② `Required` の `fn:IsRequired(...)`・`IndirectReference` の条件・`PossibleValues` の `fn:Eval(...)` は式の文字列のまま残して実行時に評価、③ それ以外（他オブジェクトを辿るパス、`fn:RectWidth` 等の未対応関数）は「未知」。評価は**三値論理**で、未知になった条件では何も報告しない（誤検出より見逃しを選ぶ）
  - 式評価（`expr.rs`）は `@Key`、リテラル、比較・論理・`mod` 等の算術、`fn:IsPresent` / `fn:Not` / `fn:Eval` / バージョン系 / `fn:ArrayLength` / `fn:StringLength` に対応。バージョン系は `Required` 等の条件では「かつ」、`PossibleValues` の制約では「ならば」として評価する
  - **型推論**: trailer（newest が xref stream なら `XRefStream`、それ以外は `FileTrailer`）から Link を辿る。候補が複数あるときは、値が固定集合の必須キー（`/Type`, `/Subtype` 等）の一致・不一致を最も重く、次いで必須キーの欠落・未知キーで採点して選ぶ。辿って決まった型はドキュメントごとに記憶（最初の割り当てを優先）。未記憶のオブジェクトは `/Type` からの推測（UI では `?` 付き）。開いた直後に trailer と Catalog の直下だけは型付けしておく
  - name tree / number tree は専用の型（ノード・`/Kids`・`/Names`|`/Nums`）として扱い、葉の値をリンク先の型で検証する
  - **validation のスコープ**: ① 1 オブジェクト（`Schema` リクエスト。直接オブジェクトは再帰的に、各コンテナ先頭 2000 件まで）② 子エントリ単位（`Children` の各 `NodeView.schema`）③ ジョブ `Validate { root }`。root なしは trailer から型付き参照を幅優先で全部、root ありは `/Parent` を辿らず、ページ・ページツリーノードは `/Kids` 経由のときだけ辿る（1 ページの検証で文書全体に広がらないように）
  - 報告する問題: 型不一致、矩形・行列の形、必須キー欠落（継承可能キーは `/Parent` を遡って確認。条件付き必須は warning）、未知キー（info）、値の集合外・述語不成立、indirect / direct 指定違反、導入バージョンより古い文書・廃止済み、配列長、日付形式、ASCII 文字列、欠落オブジェクトへの参照、木構造の不整合。ジョブの問題は先頭 10 万件まで保持して `JobDiagnostics` で範囲取得

### 4.3 pdfi-model — 意味層

- `Document`: version, trailer, catalog, Info, **XMP（ツリー化）**, 暗号情報, リビジョン一覧, 統計
- `Page`: 継承属性解決済み（Resources, MediaBox, CropBox, Rotate）＋「どこから継承したか」、各 Box、content streams、annots
  - **ページツリーは `/Count` を使って index → ページを O(深さ) で引く**。全ページを列挙しない
- `Resources`: Font / XObject / ColorSpace / Pattern / Shading / ExtGState / Properties
- その他: Outline、Name tree / Number tree、Annotation、AcroForm、EmbeddedFiles、Signature（構造のみ）、StructTree
- **インベントリ**（全フォント・全画像・被参照グラフ）は全オブジェクト走査が要るので**ジョブ**として構築し、結果を範囲取得
- **M6 で確定した形**:
  - `pdfi-model` は `pdfi-syntax` だけに依存し（XMP 用に `quick-xml`）、`DocModel`（ドキュメントごとの索引）を `on(&PdfFile)` で `Model` にして使う。DocSchema と同じ形
  - **ページ索引**: ページツリーのノードごとに「子と、そこまでのページ数の累積」（`ends`）を初回に作って記憶し、index → ページは各段で二分探索。子の数は `/Count`（Page は 1）を信じ、`/Count` が無いノードだけ中を数える。`/Count` と子の合計の食い違い、直接オブジェクトの子、循環は Issue として報告（ドキュメント単位、重複は 1 回）。ページ → index は `/Parent` を遡って累積し、最後に index → ページで一致を確かめる（誤った `/Parent` で誤った番号を出さない）
  - **継承属性**: Resources / MediaBox / CropBox / Rotate を、ページ自身 → index で辿った祖先（近い順）で探し、出所を `Page | Inherited(node) | Default(of) | Missing` で返す。CropBox の既定は MediaBox、Bleed / Trim / ArtBox（継承しない）の既定は CropBox。Rotate は 90 の倍数でなければ報告して 0/90/180/270 に正規化。一覧の「サイズ」は CropBox ∩ MediaBox
  - ページラベル（`/PageLabels` の number tree）はドキュメントごとに一度だけ範囲表にする。name tree / number tree は `/Limits` で降りる検索と全件の巡回（上限付き）
  - **リンク先**: 明示的な配列、名前（`/Names /Dests` の name tree → カタログの `/Dests`）、`/D` を持つ辞書を解決し、ページは index まで引く。アクションは GoTo / URI を解釈、他は `/S` の種類と主な引数だけ
  - **メタ情報**: Info の文字列はテキストとして復号し、PDF 日付は ISO 8601 にする。カタログの `/Metadata` は 16 MB まで復号して XMP をフラットな木（深さ付き、2 万ノードまで）と `rdf:Description` のプロパティ（Alt は x-default、Seq / Bag は連結）にする。Info と XMP の対応キー（Title ↔ dc:title、ModDate ↔ xmp:ModifyDate など 8 組）を比べ、日付は時刻として比べる。不一致は warning
  - **インベントリ**（`inventory::Builder`）: ① 全 live オブジェクトを走査して参照（被参照グラフの辺）を集め、フォント（`/Type /Font` か、フォントの `/Subtype` + `/BaseFont` 等）と画像 XObject を記録 ② 全ページの（継承込みの）リソースから使用フォント・画像を集める。Form XObject の中はフォームごとに一度だけ辿って記憶。被参照グラフは対象番号ごとの CSR（`starts` + `sources`）、trailer からの参照は `TRAILER`。参照元のどこから指しているか（`/Resources/Font/F1`）は要求時に元オブジェクトを読み直して求める。誰からも参照されない live オブジェクト（ObjStm / XRef stream / 線形化辞書を除く）も記録
  - 3000 ページ（`/Kids` 3000 のフラットな木）の合成ファイルで、最初のページ引きが約 13 ms（子 3000 個の読み込み）、以降は即時。インベントリは約 0.25 s（native release）

### 4.4 pdfi-font / pdfi-image

- **フォント**: Type1 / TrueType / Type0 (CID) / Type3。埋め込み（FontFile / FontFile2 / FontFile3）、Encoding / Differences、CMap（定義済み CMap 含む）、ToUnicode、Widths
  - アウトライン: TrueType / CFF / OpenType は `skrifa`、Type1 は自前パーサ（`hayro-font` を評価）
  - 非埋め込みの標準 14 フォント等の代替フォントは**ライセンスに注意**（§8）
  - UI: グリフ一覧、コード → グリフ → Unicode の対応表、メトリクス
- **M9 で確定した形**（pdfi-font）:
  - `pdfi-font` は `pdfi-syntax` と `pdfi-model` に依存し、M7 で pdfi-content に置いた辞書レベルの処理（符号の切り分け、幅、`/ToUnicode`、埋め込み CMap）もここへ移した（pdfi-content は `pdfi_font` を `font` として再公開）。`load` は辞書だけを読み、**フォントプログラムは最初にグリフを求められたときに読む**（`OnceLock`。解析だけならプログラムを開かない）。アウトラインはフォントごとに 1.6 万件までキャッシュ、`DocContent` のフォントは 1024 個を超えたら捨てて読み直す
  - プログラムの形式は**キーではなく中身で判定**する（`/FontFile3 /Type1C` に OpenType が入っている等が多い）: sfnt（TrueType / `OTTO`）は **`ttf-parser`**、素の CFF と Type 1（PFB / PFA、Length1–3 の連結）は **`hayro-font`**。`head` などの必須テーブルが無い OpenType の subset は `CFF ` テーブルだけを CFF として読む。アウトラインはフォント行列（sfnt は `1 / unitsPerEm`）を掛けた「文字サイズ 1 のテキスト空間」で返し、二次曲線は三次にする
  - **グリフ選択**: Type 1 は名前（`/Differences` → `/BaseEncoding` → プログラム内蔵のエンコーディング）、CFF は名前から charset で、無ければ内蔵エンコーディング。TrueType は ISO 32000 9.6.5.4 の順（名前 → AGL で Unicode → (3,1)、(3,0) を符号・`0xF000`〜`0xF200`+符号で、(1,0) を MacRoman 符号で、`post` の名前、cmap が無ければ符号 = グリフ番号）。Type 0 は CID → `/CIDToGIDMap`（CIDFontType2）か CID-keyed CFF の charset
  - エンコーディングは**グリフ名の表**で持つ（Standard / WinAnsi / MacRoman と Adobe Glyph List は fontTools から `scripts/gen-font-tables.py` で生成）。テキストは `/ToUnicode` → UCS2 / UTF16 系の定義済み CMap → グリフ名（AGL、`uniXXXX`、`uXXXX`、`.suffix`、`f_i` の合字）
  - **代替フォント**: 埋め込まれていない（または読めない）単純フォントは PDFium の Foxit フォント 14 書体（BSD-3。拡張子は `.pfb` だが中身は CFF、計 265 KB）を名前・`/Flags`・`/FontWeight`・`/ItalicAngle` から選び、バイナリに内蔵する（wasm でも同じ描画）。`/Widths` の無い標準 14 フォントは Adobe の AFM 幅（hayro-interpret の表から `scripts/gen-font-std14.py` で生成）をグリフ名で引くので近似ではなくなった。Symbol / ZapfDingbats は内蔵エンコーディングも使う。埋め込まれていない（または読めない）CID フォントは原ノ味明朝 / ゴシック（Source Han Serif / Sans を Adobe-Japan1-7 の並びにしたもの、OFL-1.1、計 10.5 MB、無加工で内蔵）で描く（M12 の後で追加）。書体は名前で選び（Gothic・ShinGo・FutoGo・Jun・Kaku 等はゴシック、他は明朝）、`/CIDSystemInfo /Ordering` が Japan1 なら CID でそのまま、それ以外はテキスト（`/ToUnicode`、UCS-2 の符号、文字コレクションの CID → Unicode）からフォントの Unicode cmap でグリフを引く。どれも無いもの（Identity で `/ToUnicode` も無いもの）はグリフの枠を描く（注記と警告）。縦書きでは代替フォントの縦書き形（GSUB `vrt2`、無ければ `vert` の単一置換）に置き換える（初めから縦書き形の CID はそのまま。原ノ味では 90ms-RKSJ-V と一致する。埋め込みフォントは置き換えない）。幅は PDF のまま
  - フォントの問題（`/Widths` の数、未知のエンコーディング、プログラムが読めない・宣言と形式が違う、埋め込みなし、使う CMap が定義済みでないなど）は `Font.issues` と `Glyphs.issues` に集める
  - **定義済み CMap**（M12 の後で追加）: Adobe の cmap-resources の CMap（全コレクション。PDF が定義しない UTF-8 / UTF-32 は除く）と mapping-resources-pdf の CID → Unicode（Adobe-*-UCS2）を `scripts/gen-cmaps.py` で 1 ファイル（1.3 MB、CMap ごとの zlib、BSD-3）にまとめて内蔵し、使うときに展開してプロセスで保持する。`/Encoding` が定義済み CMap のフォントは符号空間・CID・書字方向をそこから取り（`usecmap` もたどる）、埋め込み CMap が `usecmap` / `/UseCMap` で定義済み CMap を使うときもそれをたどる。`/ToUnicode` の無い Type 0 のテキストは CID から文字コレクションの Adobe-*-UCS2 で引く
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
- **M7 で確定した形**:
  - `pdfi-content` は `pdfi-syntax` と `pdfi-model` に依存し、`DocContent`（ドキュメントごとのキャッシュ: 解析結果 4 件、フォーム XObject の復号データ 64 MB、フォント情報）を `on(Model)` で使う。対象は `Target::Page(index)`（`/Contents` を連結、継承込みのリソース、境界は CropBox ∩ MediaBox）と `Target::Stream(id)`（Form / タイリングパターン / その他。境界は `/BBox`、リソースはストリーム自身の `/Resources`）
  - **命令列は保持しない**: 1 回の走査で検査・解釈し、残すのは復号済みデータ、1024 命令ごとのチェックポイント（読み取り位置 `ReadPoint` と解釈器の状態）、命令ごとのネスト深さ（`u16`）、描画要素、デコード済みテキスト、問題。`ops(range)` や「命令 N の状態」はチェックポイントから読み直して求める。160 万命令（10 MB）のページで解析 0.2 秒・常駐 50 MB（`Vec<Op>` で持つと 700 MB だった）。上限は 1 ストリーム 64 MB、400 万命令
  - パーサ（`OpReader`）は配列の各ストリームを順に読み、オペランドが前のストリームに残っていれば持ち越す（その命令の `OpOrigin` は演算子のあるストリーム）。未知の演算子は `Operator::Unknown` として残し、`]` や `}` など置き場のないトークンは報告して読み飛ばす。インライン画像は `/L`（`/Length`）、フィルタなしなら `W × H × 成分 × BPC` から長さを求め、それ以外は空白に挟まれた `EI` の後が内容らしく見える位置を探す
  - **validation の項目**: オペランドの数（不足は error でその命令は解釈しない、超過は warning で末尾 N 個を使う）と型、`q/Q`・`BT/ET`・`BMC/BDC/EMC`・`BX/EX` の対応（終端の未閉鎖は内容全体の warning）、テキストオブジェクト内で使えない演算子・外で使えない演算子、パスオブジェクト途中の他の演算子、`W` の後に描画演算子がない、カレントポイントなしの `l` / `c` / `h`、描画するパスがない、フォント未設定の文字表示、`d0`/`d1` の位置、未定義のリソース（`/Resources` を持たないストリームでは「親のものかもしれない」として info）、XObject の `/Subtype`、色空間の成分数・`sc` と `scn` の使い分け・Pattern 色空間でのパターン名、インライン画像の必須キー、線端・結合・`Tr`・レンダリングインテントの値域、ダッシュ配列。`BX … EX` 内の未知演算子は info
  - テキスト表示演算子のデコード（M9 の pdfi-font までの暫定）: `/ToUnicode`（bfchar / bfrange）、単純フォントは WinAnsi / MacRoman / Standard と `/Differences`（主要なグリフ名と `uniXXXX`）、Type 0 は UCS2 / UTF16 系の定義済み CMap なら符号をそのまま。`TJ` の −180 以下の調整は空白として表示（M9 で pdfi-font に移し、グリフ名は Adobe Glyph List 全体で引く）

### 4.6 インタプリタと pdfi-render — レンダリング

- **インタプリタ**（pdfi-content）: グラフィックス状態・テキスト状態を追跡し **display list** を生成
  - 要素（path / glyph run / image / shading / clip / group）は **元の op index とページ座標の bbox** を持つ
  - → 「レンダ画像をクリック → 該当命令へ」「命令を選択 → 画像上をハイライト」「op N まで実行した状態で描画（ステップ実行）」
  - Form XObject / Type3 / パターンは再帰（深さ・回数制限あり）
- **M7 で確定した形**（インタプリタと bbox）:
  - 状態は `GState`（CTM、クリップの bbox、線幅・端・結合・マイター・ダッシュ、インテント、平坦度、塗り／線の色空間と成分、α、ブレンド、ソフトマスク、適用した ExtGState、テキスト状態）と、`q` のスタック・テキスト行列・テキストオブジェクト内か・作成中のパス・保留中の `W`・マーク付き内容のタグ。`gs` は LW / LC / LJ / ML / D / RI / FL / CA / ca / BM / SMask / Font を反映
  - 描画要素は 1 命令に最大 1 つ（fill / stroke / fillStroke / clip（`W n`）/ text / image / inlineImage / form / shading）で、bbox は内容の空間（ページなら回転前の既定ユーザー空間）。パスは制御点まで含めた点の bbox、ストロークは線幅 × CTM の拡大率 / 2 だけ広げる、`sh` は現在のクリップ、画像は単位正方形 × CTM。文字は字送り幅とフォントの Ascent / Descent（なければ FontBBox）から求め、`Tc` / `Tw` / `Tz` / `Ts` / `TJ` を反映する。クリップの外にあるもの・不可視（`Tr` 3 / 7）は `hidden`
  - 幅は単純フォントの `/Widths`（なければ標準 14 フォントの ASCII 幅、Courier は 600）、Type 0 の `/W` / `/DW`（埋め込み CMap の cidrange / cidchar で CID を引く。定義済み CMap は M12 の後から Adobe の CMap で引く）と縦書きの `/W2` / `/DW2`、Type 3 は `/FontMatrix` を掛ける。推測を含むフォントは `approximate`
  - フォーム XObject は継承した状態のまま中身を解釈して（深さ 12、1 内容あたり 200 万命令まで、自己参照は error）描画範囲を求め、`/BBox` とクリップで切る。中の問題はフォーム自身の内容として見たときに報告する。パターン・Type 3 のグリフ手続きは辿らない
- **M8 で確定した形**（自作レンダラ (1)）:
  - ラスタライザは **tiny-skia**（BSD-3。AA 付きの塗り・線・ダッシュ・マスク・画像パターンが揃い、wasm でも同じ結果になる）。JPEG は **zune-jpeg**。`vello_cpu` は見送り
  - 描画は display list を作らず、**インタプリタを描画用にもう一度走らせる**（`Content::draw`）。インタプリタは `Rec` に `Device`（`pdfi-content::device`）があるときだけ形を記録し、`fill` / `stroke` / `text` / `image` / `shading` を呼ぶ。トップレベルの命令ごとに `Device::op(i)` を呼ぶので、フォームの中身はそれを呼んだ `Do` の描画として扱える。パスは内容の空間（CTM 適用済み）で受け、線は CTM の逆で戻して CTM ごと tiny-skia に渡す（非一様な拡大でも正しい線幅）。クリップは `GState.clip_path`（`Arc` の連鎖。`q` / `Q` で共有）で、レンダラは連鎖ごとに AA マスクを作り、祖先のマスクがあればそこから 1 パスずつ足す。ビットマップを覆う矩形クリップはマスクを作らない
  - **ビュー**: 表示する領域とその座標系。ページは CropBox ∩ MediaBox を `/Rotate` で回したもの、フォーム / パターンは `/BBox`、その他のストリームは描画要素の範囲。原点は左上・y 下向き・単位はポイント。描画要求は「倍率（ポイントあたりのピクセル）+ ビュー全体のビットマップ中の矩形（タイル）」、ヒットテストはビュー座標で受ける
  - **タイル**: 各タイルの周囲 2 px を余分に描いて捨てる（パスがビットマップの端で切られると AA が端のピクセルに漏れ、継ぎ目になるため）。曲線は端の位置で平坦化が変わるので、曲線の縁のピクセルはタイル間で ±16/255 程度ずれうる。ビットマップに掛からない描画は tiny-skia に渡す前に bbox で捨てる（タイル描画の大半）。1 枚は 1 辺 16384 px・16M ピクセルまで
  - **ハイライト層**: 指定した命令だけを不透明な単色で透明背景に描く（線は最低 3 px）。UI は選択した命令の bbox の範囲だけ要求して重ねる。**ステップ実行**は `until` までの命令だけを走らせる
  - **ヒットテスト**: 点の周り（半径 r）を 3 × 3 px のビットマップに描き、各描画のたびに覆ったかを見る（クリップも効く、線の太さもそのまま）。上にあるものから返す
  - 対応範囲: パス・塗り（非ゼロ / 偶奇）・線（幅・端・結合・マイター・ダッシュ、1 px 未満はヘアライン）・クリップ・定数 α（`CA` / `ca`）。色は Device* / Cal*（デバイス扱い）/ Lab / ICCBased（`/Alternate` か成分数）/ Indexed / Separation・DeviceN（関数型 2・3 の tint transform、それ以外は近似）。画像は 1/2/4/8/16 bit、`/Decode`、JPEG（YCCK は RGB 化、CMYK は `/Decode` どおり）、`/ImageMask`、`/SMask`、`/Mask`（ステンシル / 色キー）、インライン画像。縮小は 2 倍以上なら箱フィルタで縮めた版をキャッシュ、拡大は `/Interpolate` がなければ最近傍。デコード済み画像はドキュメントごとに 192 MB まで保持（同じ画像を複数スレッドが同時に要求しても 1 回だけデコード）
  - まだ描かないものは**注記**（`notes`）として返す: 文字（M9 まではグリフの枠を薄く描く）、shading・パターン色（灰色）、ソフトマスク・ブレンドモード、テキストのクリップモード、JPX / JBIG2 / CCITT（灰色の枠）（M9・M12 でいずれも描くようになった）
  - 1 ページ 80 万命令（25 万の塗り・線）で解釈 0.12 秒、72 dpi 全体 1 秒、300 dpi の 512 px タイル 0.25 秒（native release）
- **M9 で確定した形**（自作レンダラ (2): 文字）:
  - インタプリタは描画中だけ `GlyphRun` に符号（長さ込み）と原点を載せ、`GlyphRun::glyph_path` がアウトラインを内容の空間のパスにする（グリフ行列 = `[size·Tz, 0, 0, size, x, y]` × Tm × CTM。縦書きは位置ベクトル（`/W2`、無ければ `/DW2` の v.y と横幅の半分、既定 (w/2, 0.88)）を引いて横書きの原点に直す。縦の送りも `/W2` の w1y、無ければ `/DW2`（既定 −1 em）。M12 の後で追加）。レンダラは描画モードに従ってそれを既存の `fill` / `stroke` に渡すだけ（ヒットテスト・ハイライトも同じ経路）。アウトラインを持たないフォントだけ M8 のグリフの枠を薄く描く
  - **Type 3**: インタプリタがグリフ手続きをフォーム XObject と同じ仕組み（`run_sub`: 深さ・命令数・自己参照の制限、復号データのキャッシュ）で実行する。行列は `/FontMatrix` × グリフ行列、リソースはフォントの `/Resources`（無ければ内容のもの）。`d1` のグリフ（uncolored）では色の演算子を無視する。描画はそれを呼んだ `Tj` の描画として扱う
  - **テキストのクリップ**（`Tr` 4–7）: `BT` 以降にクリップモードで表示したグリフのパス（アウトラインが無ければ枠）と bbox を `State.text_clip` にためて、`ET` でクリップに重ねる。解析（bbox）でも同じく効く
  - 注記は「どのフォントが代替で描かれたか」「どのフォントが枠で描かれたか」をフォントごとに返す（M8 の「文字はグリフの枠」は廃止）
- **M12 で確定した形**（自作レンダラ (3): 描画の網羅性。編集（M10 / M11）より先に実施）:
  - **関数**（`function.rs`）: 型 0（サンプル。多重線形補間、BitsPerSample 1–32）、2、3、4（PostScript 計算機。`{ … }` を `if` / `ifelse` の飛び先付きの平たい命令列にコンパイルし、スタック 100・命令 2^20 ステップまで）と関数の配列。Separation / DeviceN の tint transform はすべての型で正確に評価する（読めない関数だけ従来の近似と注記）
  - **色**: ICCBased は **moxcms** でプロファイルから sRGB への変換を作り（ドキュメントごとに ICC ストリームのオブジェクト単位でキャッシュ）、画像は行ごとにまとめて変換する。使えないプロファイル（壊れている・成分数が合わない・Lab データ）は `/Alternate`、無ければ成分数のデバイス空間。CalGray / CalRGB は Gamma・Matrix を適用して XYZ 経由、Lab と同じく白色点を D65 へスケールする。DeviceCMYK（塗り・画像・Separation / DeviceN の代替空間）は既定のプロファイル FOGRA39L_coated.icc（TeX Live の colorprofiles の、パブリックドメインに置かれた版。元データは zlib ライセンス、118 KB を内蔵）で変換し、画像は ICCBased と同じく行ごと。moxcms に黒点補償が無いので、CMYK のプロファイルには自前で掛ける（全インキの色が黒になるよう、チャンネルごとに線形光で伸ばす 1024 段の表）。プロセスカラーは MuPDF と数階調以内（K 100 %: 30 30 32 対 34 31 31）（M12 の後で追加）
  - **shading**（`shading.rs`）: 7 種すべて。型 1 は画素ごとに関数を評価、型 2 / 3 は媒介変数の 1024 段の色表を引く（`/Extend`、半径が負になる側は捨てる）。メッシュ（型 4–7）は頂点の色を RGB にして（関数付きなら媒介変数を補間して色表）三角形をグーローで塗り、パッチ（Coons は内側の制御点を補って tensor として）は 4 px 程度の三角形に分ける。メッシュのデータは型 4 / 5 が頂点ごと、6 / 7 がパッチごとにバイト境界へ揃える。`sh` は現在のクリップ（の bbox）を、shading パターンは塗る範囲を、デバイス画素の画像として描き（`/BBox` で切り、パターンでは `/Background` も）、それを塗り・線・文字・ステンシルマスクの「源」にする。解析した shading はドキュメントごとにキャッシュ。ヒットテストとハイライトは shading が実際に覆う画素だけを使う
  - **パターン**: インタプリタが色に名前の指すパターンを解決して載せ（`Color.pattern_def`）、パターン空間（その内容が始まったときの CTM: ページの既定空間、フォームの空間、グリフの空間）を `GState.pattern_ctm` に持つ。タイリングパターンは 1 セルをほぼデバイス解像度（最大 400 万画素）で、レンダラが入れ子のラスタで内容を走らせて描き（`Content::draw_stream`）、シェーダの繰り返しで敷き詰める。セルはパターン・行列・色ごとにキャッシュ。`/BBox` が刻みより大きければ内容をずらして重ね描きして回り込ませ、uncolored（PaintType 2）は `/Pattern` 空間の基底の色で塗り直す。パターンと soft mask の入れ子は 6 段まで
  - **透明度**: 16 のブレンドモードすべて（tiny-skia の合成をそのまま使う）。`/Group /S /Transparency` を持つフォームは自前のビットマップに描いてから、それを描いたときの α・ブレンド・soft mask・クリップで合成する（入れ子 8 段まで。それより深いものは直接描く）。**非 isolated** のグループは下地の複製から描き始め、グループ自身の被覆（α）を別のビットマップにためて、終わりに下地の寄与を取り除く（ISO 32000 11.4.8 の `Cn + (Cn − C0)(α0/αg − α0)`）。**knockout**（`/K true`）は、各オブジェクトをそれまでのオブジェクトではなくグループの最初の下地に合成し、その形の割合 `fs`（被覆を定数 α で割ったもの）で置き換える（`(1 − fs) × 結果 + fs × (下地にオブジェクト)`、ISO 32000 11.4.6.4）。最初の下地・作業用の複製・オブジェクト単体の 3 枚を持ち、オブジェクトが覆った画素だけを触る。非 isolated なら被覆も同じく置き換えて、終わりの下地の除去はそのまま（M12 の後で追加）。**soft mask** は `gs` の時点の CTM とグループ（`/G`）を `GState.smask` に持ち、レンダラが最初に使うときにビットマップ全体へ描いて（Luminosity は `/BC` を下地に、Alpha は透明に。`/TR` を適用）マスクにし、パスごと（グループ・CTM 単位）にキャッシュ、クリップのマスクと掛け合わせる。ページは透明な下地に描いて最後に白い紙に載せる（何も描かれていない所ではブレンドが効かない）
  - **画像**: JPEG 2000・JBIG2・CCITT は純 Rust の **hayro-jpeg2000 / hayro-jbig2 / hayro-ccitt**（Apache-2.0 / MIT、wasm 可）。JPX は辞書の色空間が成分数に合えばそれを、合わなければ画像自身の空間（ICC は moxcms）を使い、`/SMaskInData` を α に、`/Indexed` ではパレットを解決せずに番号のまま読む。JBIG2（`/JBIG2Globals`）と CCITT（K / Columns / Rows / BlackIs1 / EncodedByteAlign / EndOfLine / EndOfBlock。壊れたデータは読めた行まで）は 1 bit のサンプルにして、通常の `/Decode`・色空間・ステンシルの経路に流す。**巨大な二値画像**（スキャン本の 2 万 × 4.5 万画素の Group 4 マスクなど）は、ラン単位で k × k ブロックの 1 の割合に数えながら復号する縮小デコードで、全体のビットマップを持たない（要求された縮小率を、上限 3200 万画素に収まるまで引き上げる）。Adobe の YCCK JPEG は libjpeg と同じく CMYK にしてから他の CMYK JPEG と同様にそのまま使う（反転は `/Decode` が補う）。JPEG は辞書の色空間の成分数が合えばそれを使う（1 成分の `/Separation /Black` や `/DeviceN [/Black]` を DeviceGray にすると反転する）
  - 注記に残るもの: 深すぎる入れ子、読めない関数・shading・プロファイル、壊れた画像、Identity で `/ToUnicode` も無い埋め込みのない CID フォント。描画しないもの（後回し）: オーバープリント、オプショナルコンテンツ（非表示のレイヤー）、`/Matte`
  - **注釈の外観**（M12 の後で追加）: ページの内容とは別の層（`Layer::Annots`）として、表示される注釈（Hidden・NoView でなく、外観の無い Invisible でもないもの）の `/AP /N`（状態の辞書なら `/AS` で選ぶ）を `/Annots` の順に透明な下地へ描く。フォーム空間 → ページは ISO 32000-2 12.5.5 のとおり `/Matrix` を掛けた `/BBox` の外接矩形を `/Rect` に合わせる行列 A を続けたもの（`Matrix × A`、pdfi-model が `Appearance.to_page` として求める）で、変換した `/BBox` でクリップし、`/CA` が 1 未満か外観に透明グループがあれば isolated なグループとして合成する。内容の層には含めない（命令列とヒットテストは内容だけのまま）ので、UI は 2 枚を重ねて「内容の上に注釈がどう載るか」を見せる。`/AP` のどの外観（`/N`・`/R`・`/D` とその状態）でも、注釈ごとに選んで代わりに描ける（`pick`。ビューアが操作中にしか見せない外観や、`/AS` で選ばれていない状態を確かめるため）。NoZoom・NoRotate、外観の無い注釈の外観の生成は扱わない
  - 検証: `tests/fixtures/paint.pdf`（色・shading・パターン・透明度・画像形式の 5 ページ。make.sh が生成し、JPX / JBIG2 / CCITT / YCCK のデータは opj_compress・jbig2enc・Pillow・ImageMagick で作ったものを埋め込む）の画素を MuPDF（`mutool draw`）と突き合わせ、ずれはアンチエイリアスの縁だけ。ローカルの 549 冊の蔵書から 61 冊 × 3 ページを描いて MuPDF と比べ、差の大きいものを調べて上の非 isolated グループ・YCCK・巨大な二値画像を直した。その後、549 冊すべて × 4 ページ（1 ページ目と無作為な 3 ページ、計 2165 ページ、72 dpi）を PyMuPDF（注釈なし）と 4 × 4 画素のブロックで比べ、1 成分 JPEG の色空間・knockout・既定の CMYK プロファイル・CJK の代替フォントを直して、ブロックの差が 5 % を超えるページは 134 → 6、20 % を超えるページは 45 → 0 になった。残る差の多くは 1 px 未満の線をヘアライン（濃い 1 px）で描くこと（MuPDF は被覆の割合で薄く描く）と、FOGRA39 と MuPDF の CMYK プロファイルの差
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
- **M2 で確定した形**:
  - 型生成は **ts-rs**。全 DTO を 1 ファイル（`apps/web/src/protocol/generated.ts`）に出力し、`cargo test -p pdfi-api` が差分を検出（`UPDATE_TS=1` で再生成）
  - バイナリフレーム = `request_id (u64 LE) + part (u32 LE) + bytes`。応答 JSON の `BytesInfo.part` が参照し、**バイナリは参照元の応答より先に送る**
  - ジョブは `step()` を繰り返す有界な作業単位。実行は `Executor` trait（server: ジョブごとのスレッド、wasm: `QueueExecutor` を協調的に回す）
  - ファイルアクセスは `SourceProvider` trait で注入（pdfi-api 自体は fs に触れない）
  - `Cancel(RequestId)` はトランスポート層で未着手のリクエストを破棄する。`Batch` はネスト不可
- **M12 の後で追加（プロトコル v9）**: `AnnotInfo.variants`（`/AP` の外観すべて: `/N`・`/R`・`/D` の各状態。`AppearanceInfo.kind` がどのエントリか）と `as_state`（`/AS`）。`RenderLayer::Annots.pick`（`AppearancePick { index, kind, state }`: その注釈は通常の外観の代わりにこれを描く）。UI は注釈の詳細で外観を切り替える（`/AS` が選ぶものに ·AS の印）
- **M12 の後で追加（プロトコル v8）**: `AnnotInfo` を広げた（`/F` の名前、表示されるか、`/T`・`/Subj`・`/M`、`/C`・`/IC`・`/CA`、線幅、`/QuadPoints`、インク・頂点・線の座標、`/AP` のキー、`/Popup`、`appearance: AppearanceInfo`（外観ストリーム、`/AS` の状態とその一覧、`/BBox`、`/Matrix`、フォーム空間 → ページの行列、描けない理由））。`RenderLayer::Annots { only }`（ページの注釈の外観だけを透明な下地に。`only` で `/Annots` の番号を絞る）。UI はページの描画に「Appearances」（この層を内容のタイルに重ねる）と「Outlines」（SVG で `/Rect`・`/QuadPoints`・パス、選んだ注釈は変換後の `/BBox` とフォームの原点・x 軸）を出し、ラベルで選んだ注釈の詳細（描かれる順番、外観ストリームの命令列へのリンク）を示す
- **M9 で追加（プロトコル v6）**: `Font { doc, id }`（`FontDetail`: 種類、`/Subtype`・`/BaseFont`、子孫 CID フォント、記述子と `/Flags` の名前、エンコーディングの説明、縦書き、`/ToUnicode` と件数、`/CIDToGIDMap`、プログラム（キー・`/Subtype`・読めた形式・バイト数・グリフ数・unitsPerEm・CID-keyed）、何で描くか（`embedded` | `substitute` | `procedures` | `none`）と代替フォント名、ascent / descent、Type 3 の `/FontMatrix`、`Glyphs` の件数、問題）、`Glyphs { doc, id, range }`（`GlyphView`: 符号とそのバイト列、CID、グリフ番号、名前、テキスト、幅、アウトラインの SVG パス（1000 単位 / em、y 上向き）と bbox、Type 3 のグリフ手続き）。一覧に出す符号は単純フォントが 0–255、Type 0 は CMap / `/ToUnicode` に出てくる符号、identity なら `/CIDToGIDMap` の範囲かグリフ数。子孫の CID フォントは `BadRequest`（Type 0 から見る）
- **M8 で追加（プロトコル v5）**: `Render { content, scale, tile, until, layer }`（`layer`: `content` | `highlight { ops }`。応答は `BitmapInfo`: タイルの位置と大きさ、ビュー全体のピクセル数、実行した命令数、注記。RGBA（非乗算）はバイナリ部）、`HitTest { content, x, y, radius, until }`（`Hits { ops }`、上から順）。`ContentSummary.view`（`ViewInfo`: ビューの大きさ（pt）、内容の空間 → ビューの行列、`/Rotate`）。`Session::dispatch_cancellable` で実行中の描画を止められる
- **M7 で追加（プロトコル v4）**: `ContentRef`（`page { index }` | `stream { id }`）を取る `Content`（`ContentSummary`: ストリーム一覧、命令数、描画要素数、問題の件数、境界、リソース、演算子グループ別の件数）、`ContentOps { range }`（`OpView`: 演算子・グループ・オペランドの PDF 構文表記・ネスト深さ・出所（ストリーム番号と復号データ中の範囲）・デコード済みテキスト・名前で指すリソース・描画要素・その命令の問題）、`ContentIssues { range }`（内容全体の問題が先、あとは命令順）、`ContentElements { range }`、`OpState { index }`（`GraphicsState` を命令の前後で）
- **M6 で追加（プロトコル v3）**: `Document { doc }`（Info・カタログの宣言・XMP の有無・ページ数・モデルの問題）、`Xmp { doc, range }`（プロパティと木の範囲取得）、`Pages { doc, range }`、`Page { doc, index }`（`PageDetail`: 祖先、ボックスと出所、Rotate、リソース一覧、contents、注釈数、問題）、`PageIndex { doc, id }`、`Annotations { doc, page, range }`、`Outline { doc, parent, range }`、`JobKind::Inventory` とその結果を読む `Fonts` / `Images` / `Referrers { id }` / `Unreferenced`（ジョブ前は `ErrorCode::NotReady`）。`DocSummary.pages` と `DocSummary.inventory`（`InventoryStats`。リロード後も UI が件数を知れる）
- **M5 で追加（プロトコル v2）**: `Schema { doc, root }`（型・推論の出所・仕様の参照・問題一覧）、`JobKind::Validate { root }`、`JobDiagnostics { job, range }`。`NodeView.schema`（キー定義 `KeyDoc`・値の型・そのエントリの問題）、`ObjectSummary.schema_type`、`DiagnosticInfo.path`（`/Pages/Kids[3]`）、`DocSummary.pdf_version`（Catalog の `/Version` を反映した実効バージョン）

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
  - M8: 接続ごとのリクエストは 1 本のスレッドで順に処理し、`Render` / `HitTest` だけ rayon のプールに回す（応答は ID で対応づくので順不同でよい）。`Cancel` は未着手なら破棄、実行中の描画は数百命令ごとの確認で止まる（`ErrorCode::Cancelled`）
- **セキュリティ**（必須要件）
  - `127.0.0.1` / `::1` のみで listen（外部公開オプションは設けない）
  - 起動ごとにランダムトークンを生成。URL フラグメントで UI に渡し、WebSocket ハンドシェイクで検証（Cookie は使わない）。ブラウザの WebSocket はヘッダを付けられないため、サブプロトコル `pdfi.token.<hex>` として送る（UI は受け取ったトークンを sessionStorage に移して URL から消す）
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
- **M3 で確定した形**:
  - `pdfi-wasm` は `WasmSession`（`dispatch(json)` / `open(id, source, …)` / `runJobStep()` / `takeEvents()`）を公開する。プロトコルは WebSocket と同じ JSON（`ClientMessage` / `ServerMessage`）、バイナリ部は Transferable な `Uint8Array` で送る
  - ByteSource は JS オブジェクト `{ size, read(offset, length) }`（`JsByteSource`）。`JsValue` は `Send` でないため、Rust 側はスレッドローカルの表に置いてキーだけを持つ。Worker では `Blob.slice()` + `FileReaderSync`。パーサは窓単位（〜1 MB）で読むのでブロックキャッシュは置いていない
  - ファイルは JSON に載らないため、Worker への専用メッセージ `{ kind: "open", id, file }` で渡し、`Request::Open` と同じ形の応答を返す（`Backend.openFile`）。UI はファイル選択とドラッグ & ドロップ。`showOpenFilePicker` と「最近使ったファイル」（IndexedDB の `FileSystemFileHandle`）は未実装
  - Worker 内の `Host`（`apps/web/src/worker/host.ts`）がリクエストをキューし、1 ターンに「リクエスト 1 件 → ジョブを最大 16 ms」実行して `MessageChannel` で次ターンを予約する。`Cancel` は未着手のリクエストをキューから外す。イベントはリクエストの応答より先に送る（WebSocket と同じ順序）
  - モード判定: URL / sessionStorage にトークンがあれば RemoteBackend、なければ WorkerBackend。`pdfi serve` の CSP に `'wasm-unsafe-eval'` を追加
  - ビルド: `scripts/build-wasm.sh`（`wasm-bindgen --target web`、出力は `apps/web/src/wasm/`、git 管理外）。`wasm-bindgen` crate は `=` で固定し CLI と版を揃える。`npm run build` が先に実行する

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
  - シナリオは `tests/conformance/scenario.json`、期待結果は `tests/conformance/expected.json`（Rust 直呼びの結果。`UPDATE_CONFORMANCE=1 cargo test -p pdfi-api --test conformance` で再生成）。各経路はこれと比較する: `pdfi-api/tests/conformance.rs`、`pdfi-server/tests/server.rs`（WebSocket）、`apps/web/scripts/conformance.ts`（Node で wasm + Worker の `Host`）。Swift は M4
  - 経路で当然異なるもの（hello の `backend` / `filesystem`、ファイルパス）は比較前にマスクする。ジョブは `jobDone` までのそのジョブのイベントも記録する
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
| JPEG | `zune-jpeg` | MIT/Apache/Zlib | M8 で採用 |
| JPEG 2000 | `hayro-jpeg2000`（純 Rust）/ OpenJPEG | Apache/MIT / BSD-2 | M12 で `hayro-jpeg2000` を採用（既定機能の `image` は外す） |
| JBIG2 | `hayro-jbig2`（純 Rust） | Apache/MIT | M12 で採用。**jbig2dec は AGPL のため不可** |
| CCITT | `hayro-ccitt` / `fax` | Apache/MIT / MIT | M12 で `hayro-ccitt` を採用 |
| ICC | `moxcms` / `qcms` | BSD-3 or Apache / MIT | M12 で `moxcms` を採用（rust-version 1.85 に合う 0.8 系） |
| 既定の CMYK プロファイル | FOGRA39L_coated（TeX Live colorprofiles） | パブリックドメイン（元データ zlib） | M12 の後で採用（`crates/pdfi-render/assets/README.md`）。Ghostscript / MuPDF 同梱のものは AGPL の配布物なので使わない |
| フォント | `skrifa` / `read-fonts`, `ttf-parser` | MIT/Apache | M9 で TrueType / OpenType に `ttf-parser`、CFF / Type 1 に `hayro-font`（MIT/Apache）を採用 |
| 代替フォント | PDFium 同梱の Foxit 標準フォント / Liberation 等 | BSD / OFL | M9 で Foxit フォント（BSD-3、`crates/pdfi-font/assets/LICENSE_FOXIT`）を採用。**URW base35 は AGPL のため不可**。`pdf-standard-fonts` crate も AGPL のため不可 |
| CJK の代替フォント | 原ノ味フォント（Source Han ベース） | OFL-1.1 | M12 の後で明朝 Regular・ゴシック Medium を採用（`crates/pdfi-font/assets/LICENSE_HARANOAJI`）。Adobe-Japan1 の並びなので CID でそのまま引ける |
| 定義済み CMap | Adobe cmap-resources / mapping-resources-pdf（TeX Live adobemapping） | BSD-3 | M12 の後で採用（`crates/pdfi-font/assets/LICENSE_CMAPS`） |
| 標準 14 の幅・グリフ名 | Adobe Core14 AFM、Adobe Glyph List | 再配布可 / BSD-3 | M9 で表に生成して内蔵 |
| ラスタライザ | `tiny-skia` / `vello_cpu` | BSD-3 / Apache/MIT | M8 で tiny-skia を採用 |
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
| **M4** | （後回し。M5 を先に実装）**pdfi-ffi + PdfInspectorKit + macOS 最小アプリ**（NSDocument + SwiftUI、M2 と同等機能、XCTest による適合テスト） | Swift から同じ API |
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
