#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopPointerTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        _ = NSApplication.shared
        if NSApp.activationPolicy() == .prohibited { NSApp.setActivationPolicy(.regular) }
    }

    func testControlsShowHandWhileDisabledAndSelectableTextDoNot() async throws {
        let host = NSHostingView(rootView: PointerControls())
        let window = makeWindow(host, size: CGSize(width: 400, height: 500))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        await settle()

        for (row, expected) in [(0, NSCursor.pointingHand), (1, .pointingHand), (2, .pointingHand),
                                (3, .pointingHand), (4, .pointingHand), (5, .arrow), (6, .iBeam), (7, .arrow)] {
            try await hover(NSPoint(x: 200, y: 425 - row * 50), in: window)
            XCTAssertEqual(NSCursor.current, expected, "wrong cursor for control row \(row)")
        }
        capture(host, name: "desktop-control-cursors")
    }

    func testLegacyCursorReleasesWhenHoveredControlDisablesOrDisappears() async throws {
        let model = PointerModel()
        let host = NSHostingView(rootView: LegacyPointerControls(model: model))
        let window = makeWindow(host, size: CGSize(width: 400, height: 200))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        await settle()
        try await hover(NSPoint(x: 200, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        try await hover(NSPoint(x: 280, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand, "leaving the nested button must retain its clickable parent's cursor")
        try await hover(NSPoint(x: 390, y: 100), in: window)
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
        try await hover(NSPoint(x: 200, y: 100), in: window)
        model.enabled = false
        await settle()
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
        model.enabled = true
        try await hover(NSPoint(x: 200, y: 100), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        model.shown = false
        await settle()
        XCTAssertNotEqual(NSCursor.current, .pointingHand)
    }

    func testInlineLinksKeepHandTextSelectionAndAppURLRouting() async throws {
        let text = try AttributedString(markdown: "Open [a profile](irischat-mention:example) and select this text.")
        var opened: [URL] = []
        let host = NSHostingView(rootView: IrisDesktopLinkedText(attributed: text, foreground: .primary, lineLimit: nil)
            .environment(\.openURL, OpenURLAction { opened.append($0); return .handled })
            .fixedSize(horizontal: false, vertical: true)
            .frame(width: 400, height: 100))
        let window = makeWindow(host, size: CGSize(width: 400, height: 100))
        let originalPointer = NSEvent.mouseLocation
        defer { finish(window, restoring: originalPointer) }
        await settle()
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
        try await hover(point(linkRange), in: window)
        XCTAssertEqual(NSCursor.current, .pointingHand)
        try await hover(point(plainRange), in: window)
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

    func testLinkedMessagesWrapAndExpandWithoutGrowingShortBubbles() async throws {
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

    private func makeWindow<V: View>(_ host: NSHostingView<V>, size: CGSize) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(origin: CGPoint(x: 200, y: 200), size: size),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.level = .floating
        window.acceptsMouseMovedEvents = true
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        return window
    }

    private func hover(_ point: NSPoint, in window: NSWindow) async throws {
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        await settle()
        let screen = window.convertPoint(toScreen: point)
        let location = CGPoint(x: screen.x, y: NSScreen.screens[0].frame.maxY - screen.y)
        CGWarpMouseCursorPosition(location)
        let event = try XCTUnwrap(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: location, mouseButton: .left))
        event.post(tap: .cghidEventTap)
        await settle()
        XCTAssertTrue(window.isKeyWindow, "cursor test lost its focused window")
        XCTAssertEqual(window.mouseLocationOutsideOfEventStream.x, point.x, accuracy: 1)
        XCTAssertEqual(window.mouseLocationOutsideOfEventStream.y, point.y, accuracy: 1)
    }

    private func settle() async {
        let deadline = Date().addingTimeInterval(0.18)
        while Date() < deadline {
            while let event = NSApp.nextEvent(matching: .any, until: .now, inMode: .default, dequeue: true) {
                NSApp.sendEvent(event)
            }
            try? await Task.sleep(nanoseconds: 10_000_000)
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
