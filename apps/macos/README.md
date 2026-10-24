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

## Web UI との対応

| Web（`apps/web/src/components`） | macOS | 状態 |
|---|---|---|
| App シェル（ドキュメント切替・Open・ドロップ） | NSDocument・タブ・Open Recent | ✅ |
| SummaryCard（概要・Check objects ジョブ） | `SummaryCard` / `CheckObjects` | ✅ |
| Landmarks | `Landmarks` | ✅ |
| 戻る・進む（`route.ts` の履歴） | `Navigation`・Go メニュー | ✅ |
| Breadcrumbs | ウィンドウのサブタイトルのみ | ⬜ |
| PageList（行の取得・移動） | `PagesPanel` | ✅（ページ操作の Add… は ⬜） |
| OutlinePanel | — | ⬜ |
| InventoryPanels（Fonts / Images、InventoryGate） | `FontsPanel` / `ImagesPanel` / `InventoryGate` | ✅（一覧のみ。画像プレビューは ⬜） |
| ObjectList（クラス絞り込み・未参照） | `ObjectsPanel` | 一覧のみ |
| XrefPanel | — | ⬜ |
| ValidatePanel | — | ⬜ |
| DiagnosticsPanel | `DiagnosticsPanel` | ✅ |
| DocumentPanel（Info / カタログ / XMP / Info 編集） | `OverviewView` | Info・カタログ・問題のみ |
| Inspector / Tree / ValueEditor / Referrers / StreamPanel / ImagePreview | `NodeListView`（1 階層の表示） | ⬜ |
| PageView / RenderView / AnnotOverlay / RegionCut | `PageView`（全体を 1 枚で描画・ページ情報） | タイル・ズーム・ヒットテスト・注釈・切り出しは ⬜ |
| ContentView / OpStatePanel / ContentDiagram / OperatorDoc | — | ⬜ |
| OpEditor / ContentText（CodeMirror） | — | ⬜ |
| FontView | — | ⬜ |
| SchemaPanel | インスペクタ枠のみ | ⬜ |
| HexView | 下部ペインの枠のみ | ⬜ |
| edit.tsx（apply / undo / redo・メッセージ） | `InspectorDocument.perform`・NSUndoManager | 経路のみ（編集 UI は各画面と一緒に） |
| SaveDialog（incremental / full のオプション） | NSDocument の保存（incremental、不可なら full） | オプションの UI は ⬜ |
| OpenDialog（パスワード・recover） | — | ⬜ |
