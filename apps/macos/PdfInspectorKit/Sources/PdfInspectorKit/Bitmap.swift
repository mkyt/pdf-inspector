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
        rgbaImage(width: width, height: height, data: data)
    }
}

/// RGBA samples, 8 bits per channel, not premultiplied, as an image.
public func rgbaImage(width: Int, height: Int, data: Data) -> CGImage? {
    guard width > 0, height > 0, data.count >= width * height * 4,
        let provider = CGDataProvider(data: data as CFData)
    else { return nil }
    return CGImage(
        width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
        bytesPerRow: width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
}

/// A glyph outline (`GlyphView.outline`: SVG path data with absolute `M`,
/// `L`, `C` and `Z`, y up) as a path; `nil` if it does not parse.
public func glyphPath(_ svg: String) -> CGPath? {
    let path = CGMutablePath()
    var nums: [CGFloat] = []
    var op: Character?
    var token = ""

    func flushNumber() {
        if !token.isEmpty, let v = Double(token) { nums.append(CGFloat(v)) }
        token = ""
    }

    func emit() -> Bool {
        guard let o = op else { return nums.isEmpty }
        let n: Int
        switch o {
        case "M", "L": n = 2
        case "C": n = 6
        case "Z": n = 0
        default: return false
        }
        if n == 0 {
            path.closeSubpath()
            return nums.isEmpty
        }
        guard nums.count % n == 0, !nums.isEmpty else { return false }
        for i in stride(from: 0, to: nums.count, by: n) {
            let a = Array(nums[i..<i + n])
            switch o {
            case "M" where i == 0: path.move(to: CGPoint(x: a[0], y: a[1]))
            case "M", "L": path.addLine(to: CGPoint(x: a[0], y: a[1]))
            default:
                path.addCurve(
                    to: CGPoint(x: a[4], y: a[5]), control1: CGPoint(x: a[0], y: a[1]),
                    control2: CGPoint(x: a[2], y: a[3]))
            }
        }
        return true
    }

    for ch in svg {
        if ch.isLetter && ch != "e" && ch != "E" {
            flushNumber()
            guard emit() else { return nil }
            op = ch
            nums = []
        } else if ch == " " || ch == "," {
            flushNumber()
        } else if ch == "-" && !token.isEmpty && token.last != "e" && token.last != "E" {
            flushNumber()
            token = "-"
        } else {
            token.append(ch)
        }
    }
    flushNumber()
    guard emit() else { return nil }
    return path
}
