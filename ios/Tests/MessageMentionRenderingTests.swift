import XCTest
import SwiftUI
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MessageMentionRenderingTests: XCTestCase {
    func testCashuLinkShowsAmountAndCopiesOriginalToken() throws {
        let json = #"{"token":[{"mint":"https://mint.example","proofs":[{"amount":8,"id":"0011223344556677","secret":"public-test-only","C":"020000000000000000000000000000000000000000000000000000000000000000"}]}],"unit":"sat"}"#
        let token = "cashuA" + Data(json.utf8).base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
        let source = "💜 Gift: (\(token)), https://iris.to!"
        let rendered = linkedMessageAttributedString(source, foreground: .white)
        XCTAssertEqual(String(rendered.characters), "💜 Gift: (Cashu · 8 sats · mint.example · Not checked), https://iris.to!")
        XCTAssertEqual(rendered.runs.compactMap { $0.link?.absoluteString }, ["cashu:" + token, "https://iris.to"])
        XCTAssertEqual(copyableCashuTokens(source), [token])
        XCTAssertEqual(copyableCashuTokens("cashu:" + token), [token])
    }
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
