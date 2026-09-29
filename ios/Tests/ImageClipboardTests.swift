#if os(macOS)
import AppKit
#else
import UIKit
#endif
import ImageIO
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ImageClipboardTests: XCTestCase {
    @MainActor
    func testCopiesImagePixelsAtOriginalResolutionWithoutLinkOrPath() async throws {
#if os(macOS)
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
#else
        let pasteboard = try XCTUnwrap(UIPasteboard(name: .init(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
#endif
        let context = try XCTUnwrap(CGContext(
            data: nil, width: 1200, height: 800, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(red: 0.4, green: 0.2, blue: 0.8, alpha: 0.5))
        context.fill(CGRect(x: 0, y: 0, width: 1200, height: 800))
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(
            data as CFMutableData, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(destination, try XCTUnwrap(context.makeImage()), nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))

        let copied = await copyIrisImage(data as Data, to: pasteboard)
        XCTAssertTrue(copied)
#if os(macOS)
        let result = try XCTUnwrap(pasteboard.data(forType: .png))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: result))
        XCTAssertEqual(bitmap.pixelsWide, 1200)
        XCTAssertEqual(bitmap.pixelsHigh, 800)
        XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 600, y: 400)).alphaComponent, 0.5, accuracy: 0.01)
        XCTAssertTrue(pasteboard.canReadObject(forClasses: [NSImage.self], options: nil))
        XCTAssertNil(pasteboard.string(forType: .string))
        XCTAssertNil(pasteboard.string(forType: .URL))
        XCTAssertNil(pasteboard.string(forType: .fileURL))
#else
        let image = try XCTUnwrap(pasteboard.image)
        XCTAssertEqual(image.size.width * image.scale, 1200)
        XCTAssertEqual(image.size.height * image.scale, 800)
        XCTAssertNotNil(pasteboard.data(forPasteboardType: "public.png"))
        XCTAssertNil(pasteboard.string)
        XCTAssertNil(pasteboard.url)
#endif
    }

    @MainActor
    func testInvalidImageLeavesExistingClipboardUntouched() async throws {
#if os(macOS)
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        pasteboard.setString("existing clipboard", forType: .string)
#else
        let pasteboard = try XCTUnwrap(UIPasteboard(name: .init(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        pasteboard.string = "existing clipboard"
#endif
        let copied = await copyIrisImage(Data("not an image".utf8), to: pasteboard)
        XCTAssertFalse(copied)
#if os(macOS)
        XCTAssertEqual(pasteboard.string(forType: .string), "existing clipboard")
        XCTAssertNil(pasteboard.data(forType: .png))
#else
        XCTAssertEqual(pasteboard.string, "existing clipboard")
        XCTAssertNil(pasteboard.image)
#endif
    }
}
