// Flatten an Icon Composer iOS render (squircle with transparent corners) into
// the opaque square iOS wants for apple-touch-icon: each row's transparent
// edge pixels take the tile color found just inside the squircle on that row
// (iOS masks the corners away again), then scale and save without alpha.
//
//   xcrun swift scripts/flatten-icon.swift <render.png> <out.png> <size> [<out.png> <size> …]
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let args = Array(CommandLine.arguments.dropFirst())
guard args.count >= 3, (args.count - 1) % 2 == 0 else {
  FileHandle.standardError.write("usage: flatten-icon.swift <render.png> <out.png> <size> [...]\n".data(using: .utf8)!)
  exit(2)
}
guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[0]) as CFURL, nil),
  let image = CGImageSourceCreateImageAtIndex(src, 0, nil)
else { fatalError("cannot read \(args[0])") }

let w = image.width, h = image.height
let srgb = CGColorSpace(name: CGColorSpace.sRGB)!
var px = [UInt8](repeating: 0, count: w * h * 4)
let ctx = CGContext(
  data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: srgb,
  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))

let inset = max(4, w / 80)  // step past the glass rim highlight
for y in 0..<h {
  let row = y * w * 4
  guard let first = (0..<w).first(where: { px[row + $0 * 4 + 3] == 255 }),
    let last = (0..<w).last(where: { px[row + $0 * 4 + 3] == 255 })
  else { continue }
  let l = row + min(first + inset, last) * 4
  let r = row + max(last - inset, first) * 4
  for x in 0..<w {
    let i = row + x * 4
    let a = Int(px[i + 3])
    if a == 255 { continue }
    let f = x < (first + last) / 2 ? l : r
    for c in 0..<3 {
      // premultiplied "over": out = src + fill * (1 - a)
      px[i + c] = UInt8(min(255, Int(px[i + c]) + Int(px[f + c]) * (255 - a) / 255))
    }
    px[i + 3] = 255
  }
}
let flat = ctx.makeImage()!

var k = 1
while k + 1 < args.count {
  let out = args[k], size = Int(args[k + 1])!
  k += 2
  let c = CGContext(
    data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: srgb,
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
  c.interpolationQuality = .high
  c.draw(flat, in: CGRect(x: 0, y: 0, width: size, height: size))
  let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: out) as CFURL, UTType.png.identifier as CFString, 1, nil)!
  CGImageDestinationAddImage(dest, c.makeImage()!, nil)
  guard CGImageDestinationFinalize(dest) else { fatalError("cannot write \(out)") }
  print("wrote \(out) (\(size)×\(size), opaque)")
}
