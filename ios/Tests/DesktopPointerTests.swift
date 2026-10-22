#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopPointerTests: XCTestCase {
    override func setUp() {
        super.setUp()
        // Keep these event-loop tests synchronous and unwind Swift cleanup
        // normally on failure. Aborting an async test can leave its hover
        // task running after XCTest has started the next test.
        continueAfterFailure = true
        _ = NSApplication.shared
        if NSApp.activationPolicy() == .prohibited { NSApp.setActivationPolicy(.regular) }
    }

    func testControlsShowHandWhileDisabledAndSelectableTextDoNot() throws {
        let host = NSHostingView(rootView: PointerControls())
        let window = makeWindow(host, size: CGSize(width: 400, height: 500))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        settle()

        for (row, expected) in [(0, NSCursor.pointingHand), (1, .pointingHand), (2, .pointingHand),
                                (3, .pointingHand), (4, .pointingHand), (5, .arrow), (6, .iBeam), (7, .arrow)] {
            try hover(NSPoint(x: 200, y: 425 - row * 50), in: window)
            assertCursor(expected, in: host, context: "control row \(row)")
        }
        capture(host, name: "desktop-control-cursors")
    }

    func testLegacyCursorReleasesWhenHoveredControlDisablesOrDisappears() throws {
        let model = PointerModel()
        let host = NSHostingView(rootView: LegacyPointerControls(model: model))
        let window = makeWindow(host, size: CGSize(width: 400, height: 200))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        settle()
        try hover(NSPoint(x: 200, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        try hover(NSPoint(x: 280, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand, "leaving the nested button must retain its clickable parent's cursor")
        try hover(NSPoint(x: 390, y: 100), in: window)
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
        try hover(NSPoint(x: 200, y: 100), in: window)
        model.enabled = false
        settle()
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
        model.enabled = true
        try hover(NSPoint(x: 200, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        model.shown = false
        settle()
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
    }

    func testInlineLinksKeepHandTextSelectionAndAppURLRouting() throws {
        let text = try AttributedString(markdown: "Open [a profile](irischat-mention:example) and select this text.")
        var opened: [URL] = []
        let host = NSHostingView(rootView: IrisDesktopLinkedText(attributed: text, foreground: .primary, lineLimit: nil)
            .environment(\.openURL, OpenURLAction { opened.append($0); return .handled })
            .fixedSize(horizontal: false, vertical: true)
            .frame(width: 400, height: 100))
        let window = makeWindow(host, size: CGSize(width: 400, height: 100))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        settle()
        let view = try XCTUnwrap(descendant(NSTextView.self, in: host))
        let storage = try XCTUnwrap(view.textStorage)
        let layout = try XCTUnwrap(view.layoutManager)
        let container = try XCTUnwrap(view.textContainer)
        let linkRange = (storage.string as NSString).range(of: "a profile")
        let plainRange = (storage.string as NSString).range(of: "select")
        func point(_ range: NSRange) -> NSPoint {
            let glyphs = layout.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
            return view.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil)
        }
        try hover(point(linkRange), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        try hover(point(plainRange), in: window)
        XCTAssertEqual(NSCursor.current, .iBeam)
        XCTAssertTrue(view.isSelectable)
        XCTAssertFalse(view.isEditable)
        view.setSelectedRange(plainRange)
        XCTAssertEqual((view.string as NSString).substring(with: view.selectedRange()), "select")
        let url = try XCTUnwrap(storage.attribute(.link, at: linkRange.location, effectiveRange: nil) as? URL)
        XCTAssertEqual(view.delegate?.textView?(view, clickedOnLink: url, at: linkRange.location), true)
        XCTAssertEqual(opened, [url])
        capture(host, name: "desktop-inline-link-selection")
    }

    func testLinkedMessagesWrapAndExpandWithoutGrowingShortBubbles() throws {
        let short = try AttributedString(markdown: "Read [this](https://example.com).")
        let shortSize = measure(short, width: 400, limit: nil)
        XCTAssertLessThan(shortSize.width, 120)
        XCTAssertLessThan(shortSize.height, 30)
        let long = try AttributedString(markdown: String(repeating: "Read [this link](https://example.com) and the rest of the message. ", count: 70))
        let expanded = measure(long, width: 220, limit: nil)
        let collapsed = measure(long, width: 220, limit: 14)
        XCTAssertGreaterThan(expanded.height, collapsed.height * 2)
        XCTAssertGreaterThan(collapsed.height, 150)
        XCTAssertLessThan(collapsed.height, 260)
        XCTAssertLessThanOrEqual(expanded.width, 220)
    }

    private func measure(_ text: AttributedString, width: CGFloat, limit: Int?) -> CGSize {
        let host = NSHostingView(rootView: IrisDesktopLinkedText(attributed: text, foreground: .primary, lineLimit: limit)
            .fixedSize(horizontal: false, vertical: true)
            .frame(width: width, alignment: .leading))
        host.frame = NSRect(origin: .zero, size: host.fittingSize)
        host.layoutSubtreeIfNeeded()
        return descendant(NSTextView.self, in: host)?.frame.size ?? .zero
    }

    private func makeWindow<V: View>(_ host: NSHostingView<V>, size: CGSize) -> PointerTestWindow {
        let window = PointerTestWindow(contentRect: NSRect(origin: CGPoint(x: 200, y: 200), size: size),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.level = .floating
        window.acceptsMouseMovedEvents = true
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        return window
    }

    private func hover(_ point: NSPoint, in window: PointerTestWindow) throws {
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        settle()
        if !window.establishedPointerEntry {
            // First establish the pointer outside all controls. Opening a
            // window under a stationary pointer need not generate hover entry.
            try move(NSPoint(x: 10, y: 10), in: window)
            window.establishedPointerEntry = true
        }
        try move(point, in: window)
    }

    private func move(_ point: NSPoint, in window: PointerTestWindow) throws {
        let screen = window.convertPoint(toScreen: point)
        let location = CGPoint(x: screen.x, y: NSScreen.screens[0].frame.maxY - screen.y)
        CGWarpMouseCursorPosition(location)
        let previousMoves = window.mouseMoveCount
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .mouseMoved, location: point,
            modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0,
            clickCount: 0, pressure: 0))
        // Deliver through AppKit's normal event queue. A global HID event
        // needs Accessibility permission, unlike moving the pointer itself.
        NSApp.postEvent(event, atStart: false)
        settle()
        let actual = window.mouseLocationOutsideOfEventStream
        guard NSApp.isActive, window.isKeyWindow, window.mouseMoveCount > previousMoves,
              abs(actual.x - point.x) <= 1, abs(actual.y - point.y) <= 1 else {
            if let content = window.contentView { capture(content, name: "cursor-input-failure") }
            throw PointerInputFailure(detail: "active=\(NSApp.isActive), key=\(window.isKeyWindow), " +
                "delivered=\(window.mouseMoveCount - previousMoves), expected=\(point), actual=\(actual)")
        }
    }

    private func assertCursor(_ expected: NSCursor, in host: NSView, context: String,
                              file: StaticString = #filePath, line: UInt = #line) {
        let actual = NSCursor.current
        if actual != expected { capture(host, name: "cursor-mismatch-\(context)") }
        func name(_ cursor: NSCursor) -> String {
            cursor == .arrow ? "arrow" : cursor == .pointingHand ? "hand" : cursor == .iBeam ? "text" : cursor.description
        }
        XCTAssertEqual(actual, expected, "\(context): actual \(name(actual)), expected \(name(expected))", file: file, line: line)
    }

    private func settle() {
        let deadline = Date().addingTimeInterval(0.18)
        while Date() < deadline {
            while let event = NSApp.nextEvent(matching: .any, until: .now, inMode: .default, dequeue: true) {
                NSApp.sendEvent(event)
            }
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
    }

    private func finish(_ window: NSWindow, restoring point: NSPoint) {
        window.orderOut(nil)
        window.contentView = nil
        CGWarpMouseCursorPosition(CGPoint(x: point.x, y: NSScreen.screens[0].frame.maxY - point.y))
    }

    private func descendant<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let found = view as? T { return found }
        return view.subviews.lazy.compactMap { self.descendant(type, in: $0) }.first
    }

    private func capture(_ host: NSView, name: String) {
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let image = NSImage(size: host.bounds.size)
        image.addRepresentation(bitmap)
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

private struct PointerInputFailure: Error, CustomStringConvertible {
    let detail: String
    var description: String { "Cursor input was not delivered to the active test window: \(detail)" }
}

@MainActor private final class PointerTestWindow: NSWindow {
    private(set) var mouseMoveCount = 0
    var establishedPointerEntry = false

    override func sendEvent(_ event: NSEvent) {
        if event.type == .mouseMoved { mouseMoveCount += 1 }
        super.sendEvent(event)
    }
}

private struct PointerControls: View {
    var body: some View {
        VStack(spacing: 0) {
            Button("Button") {}.buttonStyle(.plain).irisHoverPointer().frame(height: 50)
            Link("Link", destination: URL(string: "https://example.com")!).irisHoverPointer().frame(height: 50)
            Toggle("Toggle", isOn: .constant(true)).irisHoverPointer().frame(width: 120, height: 50)
            Picker("Picker", selection: .constant(1)) { Text("One").tag(1) }.irisHoverPointer().frame(width: 150, height: 50)
            Slider(value: .constant(0.5)).irisHoverPointer().frame(width: 150, height: 50)
            Button("Disabled") {}.buttonStyle(.plain).irisHoverPointer().disabled(true).frame(height: 50)
            Text("Selectable text").textSelection(.enabled).frame(height: 50)
            Text("Ordinary text").frame(height: 50)
        }.frame(width: 400, height: 500)
    }
}

@MainActor private final class PointerModel: ObservableObject {
    @Published var enabled = true
    @Published var shown = true
}

private struct LegacyPointerControls: View {
    @ObservedObject var model: PointerModel
    var body: some View {
        VStack {
            if model.shown {
                Button("Legacy button") {}.buttonStyle(.plain)
                    .modifier(IrisLegacyHoverPointerModifier(enabled: model.enabled))
            }
        }
        .frame(width: 220, height: 80)
        .contentShape(Rectangle())
        .onTapGesture {}
        .modifier(IrisLegacyHoverPointerModifier(enabled: model.enabled && model.shown))
        .frame(width: 400, height: 200)
    }
}
#endif
