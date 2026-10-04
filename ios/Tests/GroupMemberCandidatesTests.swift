import XCTest
import SwiftUI
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class GroupMemberCandidatesTests: XCTestCase {
    func testAllCandidatesRemainAvailableAndMembershipRefreshesBetweenAdds() {
        var chats = (0..<20).map { candidate($0) }
        var members: Set<String> = [chats[0].chatId]
        let localOwner = chats[1].chatId
        func available(_ query: String = "") -> [ChatThreadSnapshot] {
            groupMemberCandidates(chats: chats, localOwner: localOwner, memberOwners: members, query: query)
        }
        XCTAssertEqual(available().count, 18)
        XCTAssertEqual(available().last?.chatId, chats[19].chatId)
        members.insert(chats[19].chatId)
        XCTAssertEqual(available().count, 17)
        XCTAssertFalse(available().contains { $0.chatId == chats[19].chatId })
        members.insert(chats[18].chatId)
        XCTAssertEqual(available().count, 16)
        chats.append(candidate(20))
        XCTAssertEqual(available().last?.chatId, chats[20].chatId,
                       "Newly learned direct chats must appear without reopening the picker")
        XCTAssertEqual(available("Person 20").map(\.chatId), [chats[20].chatId])
        XCTAssertTrue(available("Person 19").isEmpty, "Search cannot reoffer existing members")
        chats[20].displayName = "Updated friend"
        XCTAssertEqual(available("updated").map(\.chatId), [chats[20].chatId])
        chats[20].kind = .group
        XCTAssertTrue(available("updated").isEmpty)
    }

    #if os(macOS)
    @MainActor
    func testProductionListScrollsBeyondEightRowsAndSearchResetsViewport() throws {
        for dark in [false, true] {
            let chats = (0..<20).map { candidate($0) }
            func view(_ rows: [ChatThreadSnapshot], query: String = "") -> some View {
                GroupMemberCandidates(chats: rows, query: query, selectedOwners: [chats[19].chatId],
                    isBusy: false, manager: nil, onSelect: { _ in }, onClose: {})
                    .padding(20).frame(width: 480, height: 460, alignment: .top)
                    .background(dark ? Color.black : Color.white)
                    .environment(\.irisPalette, dark ? .dark : .light)
                    .environment(\.colorScheme, dark ? .dark : .light)
            }
            let host = NSHostingView(rootView: view(chats))
            host.frame = NSRect(x: 0, y: 0, width: 480, height: 460)
            let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.orderFront(nil)
            defer { window.close() }
            let scroll = try XCTUnwrap(waitForScroll(host))
            let document = try XCTUnwrap(scroll.documentView)
            XCTAssertGreaterThan(document.frame.height, scroll.contentView.bounds.height * 2)
            scroll.contentView.scroll(to: NSPoint(x: 0, y: document.frame.height - scroll.contentView.bounds.height))
            scroll.reflectScrolledClipView(scroll.contentView)
            settle(host)
            XCTAssertGreaterThan(scroll.contentView.bounds.minY, 8 * 56)
            try capture(host, name: "known-users-bottom-\(dark ? "dark" : "light")")
            let beforeMembershipChange = document.frame.height
            let members = Set([chats[18].chatId, chats[19].chatId])
            let remaining = groupMemberCandidates(chats: chats, localOwner: nil, memberOwners: members, query: "")
            host.rootView = view(remaining)
            let updated = try XCTUnwrap(waitForScroll(host))
            XCTAssertLessThan(try XCTUnwrap(updated.documentView).frame.height, beforeMembershipChange,
                              "The open list must remove newly added members")
            try capture(host, name: "known-users-after-add-\(dark ? "dark" : "light")")
            host.rootView = view([chats[19]], query: "Person 19")
            let searched = try XCTUnwrap(waitForScroll(host))
            settle(host)
            XCTAssertLessThanOrEqual(searched.contentView.bounds.minY, 1)
            try capture(host, name: "known-users-search-\(dark ? "dark" : "light")")
        }
    }

    @MainActor private func waitForScroll(_ host: NSView) -> NSScrollView? {
        func find(_ view: NSView) -> NSScrollView? {
            if let scroll = view as? NSScrollView { return scroll }
            return view.subviews.lazy.compactMap(find).first
        }
        let deadline = Date().addingTimeInterval(2)
        repeat {
            settle(host)
            if let scroll = find(host), (scroll.documentView?.frame.height ?? 0) > 0 { return scroll }
        } while Date() < deadline
        return nil
    }

    @MainActor private func settle(_ host: NSView) {
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        host.layoutSubtreeIfNeeded()
    }

    @MainActor private func capture(_ host: NSView, name: String) throws {
        guard let directory = ProcessInfo.processInfo.environment["IRIS_VISUAL_OUTPUT"] else { return }
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
    }
    #endif

    private func candidate(_ index: Int) -> ChatThreadSnapshot {
        ChatThreadSnapshot(socialConnection: nil, chatId: String(format: "%02x", index + 1) + String(repeating: "0", count: 62), kind: .direct,
            displayName: "Person \(index)", nickname: nil, contactNote: nil, profileName: "Person \(index)",
            subtitle: nil, pictureUrl: nil, about: nil, memberCount: 2, lastMessagePreview: nil,
            lastMessageAtSecs: nil, lastMessageIsOutgoing: nil, lastMessageDelivery: nil,
            unreadCount: 0, isTyping: false, isMuted: false, isPinned: false, draft: "", isRequest: false)
    }
}
