import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Convert still photos that the shared metadata cleaner cannot rewrite.
/// Supported containers and videos retain their original encoding. The core
/// removes private records from the resulting JPEG before any upload.
func irisPreparePickedPhoto(data: Data, filenameExtension: String) -> (data: Data, filenameExtension: String)? {
    guard irisPhotoNeedsConversion(filenameExtension) else { return (data, filenameExtension) }
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
    let output = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(output, UTType.jpeg.identifier as CFString, 1, nil),
          irisWritePhotoAsJPEG(source, to: destination) else { return nil }
    return (output as Data, "jpg")
}

private func irisPhotoNeedsConversion(_ ext: String) -> Bool {
    ["heic", "heif", "avif", "tif", "tiff", "bmp", "dng"].contains(ext.lowercased())
}

// Photos can vend a temporary file with a generic or misleading name.
func irisSharedPhotoForConversion(at url: URL, filename: String) throws -> CGImageSource? {
    let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
    if let source, let type = CGImageSourceGetType(source) as String?,
       let ext = UTType(type)?.preferredFilenameExtension {
        if irisPhotoNeedsConversion(ext) { return source }
    }
    if irisPhotoNeedsConversion((filename as NSString).pathExtension) {
        guard let source else { throw CocoaError(.fileReadCorruptFile) }
        return source
    }
    return nil
}

func irisWriteSharedPhoto(_ source: CGImageSource, to url: URL) throws {
    guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil),
          irisWritePhotoAsJPEG(source, to: destination) else { throw CocoaError(.fileWriteUnknown) }
}

private func irisWritePhotoAsJPEG(_ source: CGImageSource, to destination: CGImageDestination) -> Bool {
    guard CGImageSourceGetCount(source) == 1,
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0, width <= 32_768, height <= 32_768,
          width * height <= 100_000_000,
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return false }
    // Build properties from scratch. Do not copy GPS, camera, time, author or
    // thumbnail dictionaries from the source; orientation is needed for display.
    let orientation = (properties[kCGImagePropertyOrientation] as? Int) ?? 1
    guard (1...8).contains(orientation) else { return false }
    CGImageDestinationAddImage(destination, image, [
        kCGImageDestinationLossyCompressionQuality: 0.95,
        kCGImagePropertyOrientation: orientation,
    ] as CFDictionary)
    return CGImageDestinationFinalize(destination)
}
