import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Convert still photos that the shared metadata cleaner cannot rewrite.
/// Supported containers and videos retain their original encoding. The core
/// removes private records from the resulting JPEG before any upload.
func irisPreparePickedPhoto(data: Data, filenameExtension: String) -> (data: Data, filenameExtension: String)? {
    let needsConversion = ["heic", "heif", "avif", "tif", "tiff", "bmp", "dng"]
        .contains(filenameExtension.lowercased())
    guard needsConversion else { return (data, filenameExtension) }
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          CGImageSourceGetCount(source) == 1,
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0, width <= 32_768, height <= 32_768,
          width * height <= 100_000_000,
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
    let output = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(output, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
    // Build properties from scratch. Do not copy GPS, camera, time, author or
    // thumbnail dictionaries from the source; orientation is needed for display.
    let orientation = (properties[kCGImagePropertyOrientation] as? Int) ?? 1
    guard (1...8).contains(orientation) else { return nil }
    CGImageDestinationAddImage(destination, image, [
        kCGImageDestinationLossyCompressionQuality: 0.95,
        kCGImagePropertyOrientation: orientation,
    ] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else { return nil }
    return (output as Data, "jpg")
}
