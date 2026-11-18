import Foundation
import UniformTypeIdentifiers
import XCTest

final class ShareURLTextTests: XCTestCase {
    func testURLObjectPreservesEscapingQueryAndFragment() async throws {
        let url = try XCTUnwrap(URL(string: "https://example.org/a%20b?item=3#part"))
        let text = await NSItemProvider(object: url as NSURL).sharedURLText()
        XCTAssertEqual(text, url.absoluteString)
    }

    func testURLVendedAsAStringStaysText() async {
        let url = "https://example.org/plain?item=2"
        let provider = NSItemProvider(item: url as NSString, typeIdentifier: UTType.url.identifier)
        let text = await provider.sharedURLText()
        XCTAssertEqual(text, url)
    }

    func testFileURLIsLeftForAttachmentHandling() async {
        let url = URL(fileURLWithPath: "/example/photo.jpg")
        let object = await NSItemProvider(object: url as NSURL).sharedURLText()
        let provider = NSItemProvider(item: url.absoluteString as NSString, typeIdentifier: UTType.url.identifier)
        let string = await provider.sharedURLText()
        XCTAssertNil(object)
        XCTAssertNil(string)
    }

    func testPlainTextIsLeftForTextHandling() async {
        let value = await NSItemProvider(object: "some text" as NSString).sharedURLText()
        XCTAssertNil(value)
    }
}
