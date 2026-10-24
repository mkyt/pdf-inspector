# macOS アプリ（PDF Inspector）

Web UI と同じ機能を SwiftUI で持つ macOS アプリ。設計は [docs/DESIGN.md §4.10](../../docs/DESIGN.md) を参照。

```
PDFInspector.app       AppKit の土台（NSDocument / NSWindowController）に SwiftUI の画面を NSHostingController で載せる
  └ PdfInspectorKit    手書きの Swift: Backend（非同期・キャンセル・イベント）、型付き API、Bitmap → CGImage、PagedList、Navigation、DocumentModel
     └ PdfiCore        UniFFI が生成する Swift（pdfi-api のプロトコル型そのまま）+ PdfiCoreFFI.xcframework（libpdfi_ffi.a）
        └ pdfi-ffi     Rust: pdfi-api の Session を UniFFI で公開（dispatch / キャンセル / イベント / パスで開く・保存・再読込）
```

- 型はプロトコルと共通: `Request` / `ResponseBody` / DTO に pdfi-api の `ffi` feature で `uniffi::Record` / `Enum` を derive。JSON を経由しない。`Page<T>` は具体型（`ObjectSummaryPage` など）、Swift 標準と衝突する `Range` / `Span` は `ListRange` / `ByteSpan`（`uniffi.toml`）
- pdfi-ffi の適合テスト（`cargo test -p pdfi-ffi`）が `tests/conformance/expected.json` を同じく満たす
- Rust の呼び出しは同期なので、Kit は 1 リクエストごとに 8 MiB スタックのスレッドで呼ぶ（GCD の 512 KiB では描画がスタックを溢れる）
- 編集は Rust の履歴を NSUndoManager に写す（NSDocument の編集状態もそこから）。保存は NSDocument が渡す一時 URL に Rust が書き、置き換えは NSDocument、終わったらそのファイルから開き直す

## ビルド

macOS 27 / Xcode 27 が要る。Rust のコアと Swift バインディングを先に生成する（どちらも生成物で、チェックサムで対になっているので commit しない）。

```sh
apps/macos/scripts/build-core.sh            # release（--debug も可。デバッグの描画は遅い）
swift test --package-path apps/macos/PdfInspectorKit
open apps/macos/PDFInspector.xcodeproj       # または xcodebuild -project … -scheme PDFInspector build
```

Rust のプロトコルを変えたら `build-core.sh` をやり直す。

SwiftUI のプレビュー（`PDFInspector/Views/Previews.swift` など）はテスト用 PDF を開く。アプリはサンドボックスなので、先にコンテナへコピーしておく:

```sh
mkdir -p ~/Library/Containers/com.juzbox.pdf-inspector/Data/Library/Caches/fixtures
cp tests/fixtures/*.pdf ~/Library/Containers/com.juzbox.pdf-inspector/Data/Library/Caches/fixtures
```

## Web UI との対応

| Web（`apps/web/src/components`） | macOS | 状態 |
|---|---|---|
| App シェル（ドキュメント切替・Open・ドロップ） | NSDocument・タブ・Open Recent | ✅ |
| SummaryCard（概要・Check objects ジョブ） | `SummaryCard` / `CheckObjects` | ✅ |
| Landmarks | `Landmarks` | ✅ |
| 戻る・進む（`route.ts` の履歴） | `Navigation`・Go メニュー | ✅ |
| Breadcrumbs | `Breadcrumbs`（詳細の上）・ウィンドウのサブタイトル | ✅ |
| PageList（行の取得・移動・ページ操作・Add…） | `PagesPanel`（行のコンテキストメニュー・`AddPages`） | ✅ |
| OutlinePanel | `OutlinePanel` | ✅ |
| InventoryPanels（Fonts / Images、InventoryGate） | `FontsPanel` / `ImagesPanel` / `InventoryGate` | ✅ |
| ObjectList（クラス絞り込み・未参照） | `ObjectsPanel` | ✅ |
| XrefPanel | `XrefPanel` | ✅ |
| ValidatePanel | `ValidatePanel` | ✅ |
| DiagnosticsPanel | `DiagnosticsPanel` | ✅ |
| DocumentPanel（Info / カタログ / XMP / Info 編集） | `OverviewView` | ✅ |
| Inspector / Tree / ValueEditor / Referrers / StreamPanel / ImagePreview | `InspectorView` / `NodeChildren` / `ValueEditor` / `Referrers` / `StreamPanel` / `ImagePreview`（JPEG・JPEG 2000 は ImageIO） | ✅ |
| PageView / RenderView / AnnotOverlay / RegionCut | `PageView` / `RenderView`（タイル・ズーム・ヒットテスト・注釈・切り出し） | ✅ |
| ContentView / OpStatePanel / ContentDiagram / OperatorDoc | `ContentView` / `OpStatePanel` / `ContentDiagram` / `OperatorDoc`（説明は Web の表から `scripts/operator-docs.mjs` で `Resources/OperatorDocs.json` に） | ✅ |
| OpEditor / ContentText（CodeMirror） | `OpEditor`（List・折りたたみ・キー操作・コピー/ペースト）/ `ContentTextEditor`（NSTextView） | 補完・入力中の問題表示・カーソルと選択の連動は ⬜ |
| FontView | `FontView` | ✅ |
| SchemaPanel | `SchemaPane`（インスペクタ） | ✅ |
| HexView | `HexView`（表示行だけ取得、対象オブジェクトに追従） | ✅ |
| edit.tsx（apply / undo / redo・メッセージ） | `InspectorDocument.perform`・NSUndoManager・`MessagesOverlay` | ✅ |
| SaveDialog（incremental / full のオプション） | 保存パネルのアクセサリ（`SaveOptionsView`）、⌘S は incremental（不可なら full） | ✅ |
| OpenDialog（パスワード・recover） | File > Reopen with Options…（`ReopenSheet`）、壊れた xref は開くときに自動で recover | ✅ |
