// swift-tools-version: 6.4

import PackageDescription

// PdfiCore (the UniFFI bindings) and its library PdfiCoreFFI are generated
// by ../scripts/build-core.sh; run it before building.
let package = Package(
    name: "PdfInspectorKit",
    platforms: [.macOS(.v27)],
    products: [
        .library(name: "PdfInspectorKit", targets: ["PdfInspectorKit"]),
    ],
    targets: [
        .binaryTarget(name: "PdfiCoreFFI", path: "Frameworks/PdfiCoreFFI.xcframework"),
        .target(name: "PdfiCore", dependencies: ["PdfiCoreFFI"]),
        .target(name: "PdfInspectorKit", dependencies: ["PdfiCore"]),
        .testTarget(name: "PdfInspectorKitTests", dependencies: ["PdfInspectorKit"]),
    ]
)
