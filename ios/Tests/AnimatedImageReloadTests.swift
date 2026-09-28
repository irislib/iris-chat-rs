import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class AnimatedImageReloadTests: XCTestCase {
    @MainActor
    func testUnchangedImageUpdatesKeepTheExistingDocument() async {
        let loader = IrisAnimatedImageLoader()
        let data = Data("GIF89a-original".utf8)
        var documents: [String] = []
        loader.update(data) { documents.append($0) }

        for _ in 0..<20 {
            loader.update(Data(Array(data))) { documents.append($0) }
        }

        XCTAssertEqual(documents.count, 1)
        XCTAssertTrue(documents.first?.contains(data.base64EncodedString()) == true)
    }

    @MainActor
    func testChangedImageUpdatesLoadNewContentEvenAtTheSameSize() async {
        let loader = IrisAnimatedImageLoader()
        let original = Data("GIF89a-original".utf8)
        let changed = Data("GIF89a-replaced".utf8)
        XCTAssertEqual(original.count, changed.count)
        var documents: [String] = []

        loader.update(original) { documents.append($0) }
        loader.update(changed) { documents.append($0) }
        loader.update(changed) { documents.append($0) }
        loader.update(original) { documents.append($0) }

        XCTAssertEqual(documents.count, 3)
        XCTAssertTrue(documents.first?.contains(original.base64EncodedString()) == true)
        XCTAssertTrue(documents.dropFirst().first?.contains(changed.base64EncodedString()) == true)
        XCTAssertEqual(documents.first, documents.last)
    }
}
