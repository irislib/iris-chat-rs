import CoreGraphics
import ImageIO
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ImageDecodeResponsivenessTests: XCTestCase {
    @MainActor
    func testBlockedImageDecodeLeavesMainActorResponsive() async {
        let started = expectation(description: "image decode started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let decode = Task {
            await loadIrisDecodedImage {
                XCTAssertFalse(Thread.isMainThread)
                started.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
                return nil
            }
        }

        await fulfillment(of: [started], timeout: 2)
        // This main-actor continuation must run while decompression is blocked.
        gate.signal()
        _ = await decode.value
    }

    @MainActor
    func testCancelledDecodeDoesNotReturnAnImage() async throws {
        let image = try XCTUnwrap(makeChatAttachmentPreviewImage(data: makePNG(), filename: "photo.png"))
        let started = expectation(description: "image decode started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let decode = Task {
            await loadIrisDecodedImage {
                started.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
                return image
            }
        }

        await fulfillment(of: [started], timeout: 2)
        decode.cancel()
        gate.signal()
        let result = await decode.value
        XCTAssertNil(result)
    }

    @MainActor
    func testAlreadyCancelledImageLoadDoesNotStartDecoding() async {
        let decode = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return await loadIrisDecodedImage {
                XCTFail("A cancelled image load should not decompress pixels")
                return nil
            }
        }
        let image = await decode.value
        XCTAssertNil(image)
    }

    @MainActor
    func testBackgroundPreviewAndAvatarLoadersKeepTheirPixelBounds() async throws {
        let data = try makePNG()
        let loadedPreview = await loadChatAttachmentPreviewImage(data: data, filename: "photo.png")
        let preview = try XCTUnwrap(loadedPreview)
        XCTAssertEqual(preview.size.width, 512)
        XCTAssertLessThanOrEqual(preview.size.height, 512)

        let loadedAvatar = await loadIrisAvatarImage(data: data, maxPixelSize: 64)
        let avatar = try XCTUnwrap(loadedAvatar)
        XCTAssertEqual(avatar.size.width, 64)
        XCTAssertLessThanOrEqual(avatar.size.height, 64)

        let loadedFullImage = await loadIrisDecodedImage { makeIrisDecodedImage(data: data) }
        let fullImage = try XCTUnwrap(loadedFullImage)
        XCTAssertEqual(fullImage.size.width, 1200)
        XCTAssertEqual(fullImage.size.height, 800)
    }

    func testBackgroundLoadersRejectInvalidDataAndKeepGIFsAnimated() async {
        let invalidPreview = await loadChatAttachmentPreviewImage(data: Data(), filename: "photo.png")
        let invalidAvatar = await loadIrisAvatarImage(data: Data(), maxPixelSize: 64)
        let animatedPreview = await loadChatAttachmentPreviewImage(data: Data("GIF89a".utf8), filename: "photo.gif")
        XCTAssertNil(invalidPreview)
        XCTAssertNil(invalidAvatar)
        XCTAssertNil(animatedPreview)
    }

    private func makePNG() throws -> Data {
        let context = try XCTUnwrap(CGContext(
            data: nil, width: 1200, height: 800, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ))
        context.setFillColor(CGColor(red: 0.4, green: 0.2, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 1200, height: 800))
        let image = try XCTUnwrap(context.makeImage())
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(data as CFMutableData, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return data as Data
    }
}
