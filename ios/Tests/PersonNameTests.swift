import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class PersonNameTests: XCTestCase {
    private let owner = String(repeating: "ab", count: 32)

    func testEmptyAndCachedIdentityNamesUseTheSameAnimalName() {
        let expected = fallbackProfileNameForIdentity(owner)
        let npub = peerInputToNpub(input: owner)
        for label in ["", "   ", owner, shortMessageIdentifier(owner), npub, shortMessageIdentifier(npub)] {
            let value = PersonNamePresentation(label, identity: owner)
            XCTAssertEqual(value.name, expected)
            XCTAssertTrue(value.isFallback)
        }
        XCTAssertEqual(fallbackProfileNameForIdentity(npub), expected)
        XCTAssertEqual(expected, "Golden Hare")
    }

    func testRealProfileNamesAndNicknamesStayUpright() {
        for name in ["Alice", "Mum", "Amber Fox", owner, fallbackProfileNameForIdentity(owner)] {
            let value = PersonNamePresentation(name, identity: owner, explicitName: name)
            XCTAssertEqual(value.name, name)
            XCTAssertFalse(value.isFallback)
        }
        XCTAssertFalse(PersonNamePresentation("Alice", identity: owner).isFallback)
        XCTAssertFalse(PersonNamePresentation("Note to Self", identity: owner).isFallback)
        let name = fallbackProfileNameForIdentity(owner)
        let explicit = explicitPersonName(nickname: "   ", profileName: name)
        XCTAssertEqual(explicit, name)
        XCTAssertFalse(PersonNamePresentation(name, identity: owner, explicitName: explicit).isFallback)
    }

    func testLegacyGeneratedLabelUsesAnimalNameWithoutRewritingStorage() {
        let legacy = "Golden Listener"
        let value = PersonNamePresentation(legacy, identity: owner)
        XCTAssertEqual(value.name, fallbackProfileNameForIdentity(owner))
        XCTAssertTrue(value.isFallback)
        XCTAssertEqual(legacy, "Golden Listener")
    }

#if os(macOS)
    @MainActor
    func testMacConversationNamesRenderFallbackAndRealNames() throws {
        let owner = self.owner
        let view = IrisTheme {
            VStack(spacing: 0) {
                DesktopPaneTopBar(title: shortMessageIdentifier(owner), personIdentity: owner)
                IrisChatRow(ownerPubkeyHex: owner, title: owner, preview: "An unnamed contact", subtitle: nil, timeLabel: "Now", unreadCount: 0) {}
                IrisChatRow(ownerPubkeyHex: owner, title: "Alice", explicitName: "Alice", preview: "A profile name", subtitle: nil, timeLabel: "Now", unreadCount: 0) {}
                IrisChatRow(ownerPubkeyHex: owner, title: "Mum", explicitName: "Mum", preview: "A saved nickname", subtitle: nil, timeLabel: "Now", unreadCount: 0) {}
                HStack {
                    personNameText(owner, identity: owner).font(.footnote.weight(.semibold))
                    Spacer()
                }.padding()
            }.frame(width: 520).padding(.bottom, 12).background(Color.white)
        }
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        renderer.isOpaque = true
        let image = try XCTUnwrap(renderer.nsImage)
        let png = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation))?.representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "mac-person-name-fallbacks"
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_VISUAL_OUTPUT"] {
            try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("mac-person-name-fallbacks.png"))
        }
    }
#endif
}
