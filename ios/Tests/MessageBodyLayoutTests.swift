#if os(iOS)
import SwiftUI
import XCTest
@testable import IrisChat

@MainActor
final class MessageBodyLayoutTests: XCTestCase {
    private let paragraph = String(repeating: "We packed warm blankets, fresh bread, and a small map for the woodland walk. ", count: 7)
        + "We packed warm blankets, fresh bread, and small map for the woodland walk. "
        + "The final sentence is fully readable."

    func testWrappedParagraphHasTheFullTextHeightAtNarrowWidthsAndLargeType() {
        XCTAssertEqual(paragraph.count, 651)
        XCTAssertFalse(paragraph.contains("\n"))
        for width: CGFloat in [180, 280] {
            for size: DynamicTypeSize in [.large, .accessibility3] {
                let attributed = linkedMessageAttributedString(paragraph, foreground: .primary)
                let fullText = Text(attributed).font(irisMessageBodyFont(for: paragraph))
                    .fixedSize(horizontal: false, vertical: true)
                    .dynamicTypeSize(size)
                let limitedText = Text(attributed).font(irisMessageBodyFont(for: paragraph))
                    .lineLimit(14).fixedSize(horizontal: false, vertical: true)
                    .dynamicTypeSize(size)
                let body = TruncatableMessageBody(
                    attributed: attributed, isOutgoing: false, bodyFont: irisMessageBodyFont(for: paragraph)
                ).dynamicTypeSize(size)

                let fullHeight = height(fullText, width: width)
                XCTAssertGreaterThan(fullHeight, height(limitedText, width: width), "Fixture must exceed 14 wrapped lines")
                XCTAssertEqual(height(body, width: width), fullHeight, accuracy: 1, "All text must be laid out at width \(width), size \(size)")
            }
        }
    }

    func testShortMessagesAndLinksKeepTheirIntrinsicHeight() {
        for text in ["Hello!", "See you soon.\nBring a hat.", "Read https://example.com then meet us outside."] {
            let attributed = linkedMessageAttributedString(text, foreground: .primary)
            for outgoing in [false, true] {
                let body = TruncatableMessageBody(
                    attributed: attributed, isOutgoing: outgoing, bodyFont: irisMessageBodyFont(for: text)
                )
                let reference = Text(attributed).font(irisMessageBodyFont(for: text))
                    .fixedSize(horizontal: false, vertical: true)
                XCTAssertEqual(height(body, width: 240), height(reference, width: 240), accuracy: 1)
            }
        }
    }

    private func height<V: View>(_ view: V, width: CGFloat) -> CGFloat {
        let host = UIHostingController(rootView: view)
        return host.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude)).height
    }
}
#endif
