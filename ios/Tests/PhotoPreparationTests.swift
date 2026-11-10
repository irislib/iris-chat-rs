import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class PhotoPreparationTests: XCTestCase {
    func testHeicPhotoBecomesJpegWithoutPrivateProperties() throws {
        let context = try XCTUnwrap(CGContext(
            data: nil, width: 32, height: 24, bitsPerComponent: 8, bytesPerRow: 128,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
        ))
        context.setFillColor(CGColor(red: 0.9, green: 0.2, blue: 0.1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 32, height: 24))
        let image = try XCTUnwrap(context.makeImage())
        let original = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(original, UTType.heic.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, [
            kCGImagePropertyOrientation: 6,
            kCGImagePropertyGPSDictionary: [kCGImagePropertyGPSLatitude: 60.1699, kCGImagePropertyGPSLongitude: 24.9384],
            kCGImagePropertyExifDictionary: [kCGImagePropertyExifDateTimeOriginal: "2026:10:31 12:00:00", kCGImagePropertyExifBodySerialNumber: "PrivateSerial"],
            kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFArtist: "PrivateAuthor"],
        ] as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        let prepared = try XCTUnwrap(irisPreparePickedPhoto(data: original as Data, filenameExtension: "HEIC"))
        XCTAssertEqual(prepared.filenameExtension, "jpg")
        let source = try XCTUnwrap(CGImageSourceCreateWithData(prepared.data as CFData, nil))
        XCTAssertEqual(CGImageSourceGetType(source) as String?, UTType.jpeg.identifier)
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertEqual(properties[kCGImagePropertyOrientation] as? Int, 6)
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 32)
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 24)
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary])
        let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
        XCTAssertNil(exif?[kCGImagePropertyExifDateTimeOriginal])
        XCTAssertNil(exif?[kCGImagePropertyExifBodySerialNumber])
        let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
        XCTAssertNil(tiff?[kCGImagePropertyTIFFArtist])
        XCTAssertNotNil(CGImageSourceCreateImageAtIndex(source, 0, nil))
    }

    func testSupportedContainersAndVideosKeepTheirOriginalEncoding() throws {
        let data = Data("bytes passed to the shared cleaner".utf8)
        for ext in ["jpg", "png", "gif", "webp", "mp4", "mov"] {
            let prepared = try XCTUnwrap(irisPreparePickedPhoto(data: data, filenameExtension: ext))
            XCTAssertEqual(prepared.data, data)
            XCTAssertEqual(prepared.filenameExtension, ext)
        }
    }

    func testFailedConversionDoesNotReturnTheOriginal() {
        for ext in ["heic", "heif", "avif", "tiff", "bmp", "dng"] {
            XCTAssertNil(irisPreparePickedPhoto(data: Data("private invalid image".utf8), filenameExtension: ext))
        }
    }
}
