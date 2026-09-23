import AVFoundation
import ImageIO
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallMediaTests: XCTestCase {
    func testInteroperablePcmUsesSignedLittleEndianSamples() throws {
        var data = Data(repeating: 0, count: 640)
        data.replaceSubrange(0..<8, with: [0x00, 0x80, 0xff, 0x7f, 0x00, 0x40, 0x00, 0xc0])
        let samples = try XCTUnwrap(IrisCallMediaFormat.audioSamples(data))
        XCTAssertEqual(samples.count, 320)
        XCTAssertEqual(samples[0], -1)
        XCTAssertEqual(samples[1], 32767 / 32768, accuracy: 0.000001)
        XCTAssertEqual(samples[2], 0.5)
        XCTAssertEqual(samples[3], -0.5)
        XCTAssertNil(IrisCallMediaFormat.audioSamples(Data(repeating: 0, count: 639)))
        XCTAssertNil(IrisCallMediaFormat.audioSamples(Data(repeating: 0, count: 642)))
    }

    func testVideoDecoderBoundsDimensionsBeforeDecoding() throws {
        let valid = try jpeg(width: 320, height: 240)
        let image = try XCTUnwrap(IrisCallMediaFormat.videoImage(valid))
        XCTAssertEqual(image.width, 320)
        XCTAssertEqual(image.height, 240)
        XCTAssertNil(IrisCallMediaFormat.videoImage(try jpeg(width: 321, height: 240)))
        XCTAssertNil(IrisCallMediaFormat.videoImage(try jpeg(width: 320, height: 241)))
        XCTAssertNil(IrisCallMediaFormat.videoImage(Data(repeating: 0, count: 65_537)))
        XCTAssertNil(IrisCallMediaFormat.videoImage(Data("not an image".utf8)))
    }

    private func jpeg(width: Int, height: Int) throws -> Data {
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try XCTUnwrap(context.makeImage())
        let output = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output, "public.jpeg" as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return output as Data
    }
}
