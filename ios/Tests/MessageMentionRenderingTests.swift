import XCTest
import SwiftUI
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MessageMentionRenderingTests: XCTestCase {
    func testLocalRenameChangesVisibleNameWithoutChangingIdentityOrLinks() {
        let owner = String(repeating: "1", count: 64)
        let token = formatMessageMention(ownerPubkeyHex: owner, name: "Old name")
        let source = "Hello \(token), https://iris.to!"
        let rendered = linkedMessageAttributedString(source, foreground: .white, names: [owner: "New name"])
        XCTAssertEqual(String(rendered.characters), "Hello @New name, https://iris.to!")
        let links = rendered.runs.compactMap { $0.link?.absoluteString }
        XCTAssertTrue(links.contains("irischat-mention:" + owner))
        XCTAssertTrue(links.contains("https://iris.to"))
        XCTAssertEqual(messageMentions(text: source).first?.ownerPubkeyHex, owner)
    }
    func testPlainNameAndMalformedReferenceRemainPlainText() {
        let source = "@Alex [@Alex](nostr:not-a-valid-key)"
        let rendered = linkedMessageAttributedString(source, foreground: .white)
        XCTAssertEqual(String(rendered.characters), source)
        XCTAssertTrue(rendered.runs.allSatisfy { $0.link == nil })
    }
}
