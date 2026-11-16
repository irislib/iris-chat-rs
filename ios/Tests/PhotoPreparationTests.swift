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
    private func heicWithPrivateProperties() throws -> Data {
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
        return original as Data
    }

    func testHeicPhotoBecomesJpegWithoutPrivateProperties() throws {
        let prepared = try XCTUnwrap(irisPreparePickedPhoto(data: heicWithPrivateProperties(), filenameExtension: "HEIC"))
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

    func testShareFromPhotosConvertsTheActualFileAndItsDisplayName() async throws {
        let fm = FileManager.default
        let folder = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try fm.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: folder) }
        let file = folder.appendingPathComponent("temporary-image")
        try heicWithPrivateProperties().write(to: file)
        let staging = IrisAttachmentStaging(dataDir: folder, fileManager: fm)
        let results = try await staging.stageAsync(shared: [PendingShareAttachment(path: file.path, filename: "Beach")])
        let photo = try XCTUnwrap(results.first)
        XCTAssertEqual(photo.filename, "Beach.jpg")
        let jpeg = try XCTUnwrap(CGImageSourceCreateWithURL(URL(fileURLWithPath: photo.path) as CFURL, nil))
        XCTAssertEqual(CGImageSourceGetType(jpeg) as String?, UTType.jpeg.identifier)
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(jpeg, 0, nil) as? [CFString: Any])
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary])
        XCTAssertEqual(properties[kCGImagePropertyOrientation] as? Int, 6)
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
