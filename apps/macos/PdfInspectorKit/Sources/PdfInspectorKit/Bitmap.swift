import CoreGraphics
import Foundation

/// A rendered region: RGBA, 8 bits per channel, not premultiplied.
public struct Bitmap: Sendable {
    public let info: BitmapInfo
    public let data: Data

    public init(info: BitmapInfo, data: Data) {
        self.info = info
        self.data = data
    }

    public var width: Int { Int(info.rect.width) }
    public var height: Int { Int(info.rect.height) }

    public func cgImage() -> CGImage? {
        guard width > 0, height > 0, data.count >= width * height * 4,
            let provider = CGDataProvider(data: data as CFData)
        else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
            bytesPerRow: width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}
