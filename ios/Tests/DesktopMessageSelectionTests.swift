#if os(macOS)
import AppKit
import SwiftUI
import XCTest
#if !IRIS_SELECTION_STANDALONE
@testable import IrisChatMac
#endif

final class DesktopMessageSelectionTests: XCTestCase {
    private var selection: IrisDesktopMessageSelection!
    private var views: [IrisMessageTextView] = []

    override func setUp() {
        _ = NSApplication.shared
        selection = IrisDesktopMessageSelection()
        selection.update(chatID: "chat", ids: ["a", "b", "c"])
        views = ["First message", "Middle 👩🏽‍💻 message\nsecond line", "Last message"].enumerated().map { index, text in
            let view = IrisMessageTextView(frame: NSRect(x: 0, y: 0, width: 260, height: 60))
            view.string = text
            view.isEditable = false
            view.messageID = ["a", "b", "c"][index]
            view.messageSelection = selection
            selection.register(view, id: view.messageID!)
            return view
        }
    }

    func testPartialEndpointsAndReverseDragHaveIdenticalCopyOrder() {
        let start = IrisDesktopMessageSelection.Position(id: "a", offset: 6)
        let end = IrisDesktopMessageSelection.Position(id: "c", offset: 4)
        selection.select(from: start, to: end)
        XCTAssertEqual(selection.selectedText, "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast")
        selection.select(from: end, to: start)
        XCTAssertEqual(selection.selectedText, "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast")
    }

    func testEmojiIsNeverSplitAndZeroWidthSelectionIsEmpty() {
        let text = views[1].string as NSString
        let emoji = text.range(of: "👩🏽‍💻")
        selection.select(from: .init(id: "b", offset: emoji.location + 1), to: .init(id: "b", offset: emoji.location + 2))
        XCTAssertEqual(selection.selectedText, "👩🏽‍💻")
        selection.select(from: .init(id: "b", offset: 0), to: .init(id: "b", offset: 0))
        XCTAssertEqual(selection.selectedText, "")
    }

    func testPrependAndNewMessageDoNotMoveTheRange() {
        selection.select(from: .init(id: "a", offset: 6), to: .init(id: "c", offset: 4))
        let before = selection.selectedText
        selection.update(chatID: "chat", ids: ["older", "a", "b", "c", "new"])
        XCTAssertEqual(selection.selectedText, before)
        selection.register(views[0], id: "a")
        XCTAssertEqual(selection.selectedText, before, "An unchanged live snapshot preserves selection")
    }

    func testUnmountedSelectedRowsStillCopyAndRemountHighlighted() {
        selection.select(from: .init(id: "a", offset: 6), to: .init(id: "c", offset: 4))
        let before = selection.selectedText
        views.remove(at: 1)
        XCTAssertEqual(selection.selectedText, before)
        let middle = IrisMessageTextView()
        middle.string = "Middle 👩🏽‍💻 message\nsecond line"
        selection.register(middle, id: "b")
        XCTAssertEqual(selection.selectedText, before)
        XCTAssertNotNil(middle.layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: 0, effectiveRange: nil))
    }

    func testEditDeletionAndChatSwitchClearStaleText() {
        selection.select(from: .init(id: "a", offset: 0), to: .init(id: "c", offset: 4))
        views[1].string = "Edited"
        selection.register(views[1], id: "b")
        XCTAssertEqual(selection.selectedText, "")
        selection.select(from: .init(id: "a", offset: 0), to: .init(id: "c", offset: 4))
        selection.update(chatID: "chat", ids: ["a", "b"])
        XCTAssertEqual(selection.selectedText, "")
        selection.select(from: .init(id: "a", offset: 0), to: .init(id: "b", offset: 4))
        selection.update(chatID: "other", ids: ["a", "b", "c"])
        XCTAssertEqual(selection.selectedText, "")
    }

    func testHitTestingUsesTextRowsAndIgnoresHorizontalBubbleGaps() throws {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 300),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        for (i, view) in views.enumerated() {
            view.frame = NSRect(x: i == 1 ? 150 : 20, y: 220 - i * 90, width: 240, height: 60)
            window.contentView?.addSubview(view)
        }
        XCTAssertEqual(selection.position(at: NSPoint(x: 410, y: 170), in: window)?.id, "b")
        XCTAssertEqual(selection.position(at: NSPoint(x: 10, y: 299), in: window), .init(id: "a", offset: 0))
        XCTAssertEqual(selection.position(at: NSPoint(x: 10, y: 0), in: window), .init(id: "c", offset: 12))
        selection.select(from: .init(id: "a", offset: 6), to: .init(id: "c", offset: 4))
        XCTAssertNotNil(views[0].layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: 7, effectiveRange: nil))
        selection.clear()
        XCTAssertNil(views[0].layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: 7, effectiveRange: nil))
    }

    func testNativeBodyKeepsLinksWrappingAndEmojiSizing() throws {
        let text = try AttributedString(markdown: "Read [this link](https://example.com) then select across messages.")
        let rendered = IrisDesktopLinkedText.nativeText(text, foreground: .textColor)
        let linkRange = (rendered.string as NSString).range(of: "this link")
        XCTAssertEqual(rendered.attribute(.link, at: linkRange.location, effectiveRange: nil) as? URL,
                       URL(string: "https://example.com"))
        let emoji = IrisDesktopLinkedText.nativeText(AttributedString("🙂"), foreground: .textColor, pointSize: 56)
        XCTAssertEqual((emoji.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 56)
        let host = NSHostingView(rootView: IrisDesktopLinkedText(attributed: text, foreground: .primary, lineLimit: nil)
            .frame(width: 180))
        host.frame = NSRect(origin: .zero, size: host.fittingSize)
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(host.fittingSize.height, 25, "Native message text wraps at the bubble width")
    }

    func testMouseGestureCrossesBubblesWithoutActivatingLinks() throws {
        let window = NSWindow(contentRect: NSRect(x: -2000, y: -2000, width: 420, height: 300),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        for (i, view) in views.enumerated() {
            view.frame = NSRect(x: i == 1 ? 150 : 20, y: 220 - i * 90, width: 240, height: 60)
            window.contentView?.addSubview(view)
        }
        let down = NSPoint(x: 25, y: 270)
        let up = NSPoint(x: 50, y: 0)
        func event(_ type: NSEvent.EventType, _ point: NSPoint) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: 1,
                              windowNumber: window.windowNumber, context: nil, eventNumber: 1,
                              clickCount: 1, pressure: type == .leftMouseUp ? 0 : 1)!
        }
        let expectedStart = views[0].characterIndexForInsertion(at: views[0].convert(down, from: nil))
        // Post only events for this hidden fixture window; no system pointer or
        // foreground app is moved. AppKit's actual tracking loop consumes them.
        NSApp.postEvent(event(.leftMouseUp, up), atStart: true)
        NSApp.postEvent(event(.leftMouseDragged, up), atStart: true)
        views[0].mouseDown(with: event(.leftMouseDown, down))
        XCTAssertEqual(selection.anchor, .init(id: "a", offset: expectedStart))
        XCTAssertEqual(selection.extent, .init(id: "c", offset: 12))
        XCTAssertTrue(selection.selectedText.contains("Middle 👩🏽‍💻 message\nsecond line\nLast message"))
        XCTAssertEqual(views[0].menu(for: event(.rightMouseDown, down))?.items.first?.title, "Copy")
        views[0].cancelOperation(nil)
        XCTAssertEqual(selection.selectedText, "")
    }

    func testSingleClickStillOpensNativeLink() throws {
        final class Links: NSObject, NSTextViewDelegate {
            var opened = 0
            func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
                opened += 1
                return true
            }
        }
        let window = NSWindow(contentRect: NSRect(x: -2000, y: -2000, width: 300, height: 150),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let view = views[0]
        window.contentView?.addSubview(view)
        let links = Links()
        view.delegate = links
        view.textStorage?.addAttribute(.link, value: URL(string: "https://example.com")!, range: NSRange(location: 0, length: 5))
        let layout = try XCTUnwrap(view.layoutManager)
        let container = try XCTUnwrap(view.textContainer)
        layout.ensureLayout(for: container)
        let bounds = layout.boundingRect(forGlyphRange: NSRange(location: 0, length: 5), in: container)
        let point = view.convert(NSPoint(x: bounds.midX + view.textContainerOrigin.x,
                                        y: bounds.midY + view.textContainerOrigin.y), to: nil)
        func event(_ type: NSEvent.EventType) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: 1,
                              windowNumber: window.windowNumber, context: nil, eventNumber: 1,
                              clickCount: 1, pressure: 0)!
        }
        NSApp.postEvent(event(.leftMouseUp), atStart: true)
        view.mouseDown(with: event(.leftMouseDown))
        XCTAssertEqual(links.opened, 1)
        XCTAssertEqual(selection.selectedText, "")
    }

    func testHostedBubblesShareSelectionAndRenderHighlights() throws {
        let fixture = VStack(alignment: .leading, spacing: 16) {
            ForEach(Array(zip(["a", "b", "c"], ["A little selection starts here", "Across another message 🙂", "And ends here."])), id: \.0) { id, text in
                HStack {
                    if id == "b" { Spacer(minLength: 80) }
                    IrisDesktopLinkedText(attributed: AttributedString(text), foreground: .primary,
                                          lineLimit: nil, messageID: id)
                        .padding(12)
                        .background(id == "b" ? Color.blue.opacity(0.2) : Color.gray.opacity(0.15))
                        .clipShape(RoundedRectangle(cornerRadius: 14))
                    if id != "b" { Spacer(minLength: 80) }
                }
            }
        }
        .padding(20).frame(width: 460)
        .environment(\.irisMessageSelection, selection)
        let host = NSHostingView(rootView: fixture)
        host.frame = NSRect(origin: .zero, size: host.fittingSize)
        host.layoutSubtreeIfNeeded()
        selection.select(from: .init(id: "a", offset: 9), to: .init(id: "c", offset: 8))
        XCTAssertEqual(selection.selectedText, "selection starts here\nAcross another message 🙂\nAnd ends")
        host.displayIfNeeded()
        if let directory = ProcessInfo.processInfo.environment["IRIS_SELECTION_EVIDENCE_DIR"],
           let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try bitmap.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: directory).appendingPathComponent("macos-selection.png"))
        }
    }
}
#endif
