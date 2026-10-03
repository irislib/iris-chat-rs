#if os(macOS)
import AppKit
import SwiftUI
import XCTest
#if IRIS_KEYBOARD_HARNESS
@testable import IrisKeyboardHarness
#else
@testable import IrisChatMac
#endif

@MainActor
final class MacKeyboardNavigationTests: XCTestCase {
    func testButtonsTabBothWaysSkipDisabledAndGroupedRowsAndActivateOnce() {
        prepareApplication()
        var actions: [String] = []
        let content = VStack {
            keyboardButton("First") { actions.append("first") }
            keyboardButton("Disabled") { actions.append("disabled") }.disabled(true)
            keyboardButton("Grouped row") { actions.append("row") }
                .environment(\.irisKeyboardButtonIsTabStop, false)
            keyboardButton("Last") { actions.append("last") }
        }.padding().irisDesktopFocusSection()
        let previous = NSApp.keyWindow
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 340, height: 220),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: content)
        window.makeKeyAndOrderFront(nil)
        defer { window.close(); previous?.makeKeyAndOrderFront(nil) }
        pumpEvents()
        window.makeFirstResponder(nil)

        press(48, "\t", in: window)
        press(49, " ", in: window)
        press(48, "\t", in: window)
        press(36, "\r", in: window)
        XCTAssertEqual(Set(actions), ["first", "last"])
        XCTAssertEqual(actions.count, 2, "Each activation must run exactly once")
        let first = actions.first
        press(48, "\t", modifiers: .shift, in: window)
        press(49, " ", in: window)
        XCTAssertEqual(actions.last, first, "Shift-Tab returns to the previous enabled control")
        XCTAssertEqual(actions.count, 3)
    }

    func testComposerTabInsertsWhitespaceAndKeepsEditorFocus() {
        prepareApplication()
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 360, height: 160),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let composer = IrisComposerNSTextView(frame: NSRect(x: 10, y: 60, width: 300, height: 40))
        window.contentView?.addSubview(composer)
        composer.string = "draft"
        composer.setSelectedRange(NSRange(location: 5, length: 0))
        XCTAssertTrue(window.makeFirstResponder(composer))
        composer.doCommand(by: #selector(NSResponder.insertTab(_:)))
        XCTAssertTrue(window.firstResponder === composer)
        XCTAssertEqual(composer.string, "draft\t")
    }

    func testComposerFocusRequestIsConsumedAndRecreationDoesNotReuseIt() {
        prepareApplication()
        let state = KeyboardFocusTestState()
        state.request = UUID()
        let host = NSHostingView(rootView: KeyboardFocusTestView(state: state, generation: 0))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 200, height: 100),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderBack(nil)
        defer { window.close() }
        pumpEvents()
        XCTAssertNil(state.request)
        XCTAssertEqual(state.focusApplications, 1)

        state.focused = false
        host.rootView = KeyboardFocusTestView(state: state, generation: 1)
        pumpEvents()
        XCTAssertFalse(state.focused, "A later pointer-open must not reuse an old keyboard request")
        XCTAssertEqual(state.focusApplications, 1)

        state.request = UUID()
        pumpEvents()
        XCTAssertTrue(state.focused)
        XCTAssertNil(state.request)
        XCTAssertEqual(state.focusApplications, 2, "Another keyboard open still requests focus")
    }

    func testLongChatListArrowsScrollOnlyAndKeyboardOpenFocusesComposer() throws {
        prepareApplication()
        let state = KeyboardListTestState()
        let previous = NSApp.keyWindow
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 600, height: 340),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: KeyboardListTestView(state: state))
        window.makeKeyAndOrderFront(nil)
        defer { window.close(); previous?.makeKeyAndOrderFront(nil) }
        pumpEvents()
        XCTAssertTrue(window.makeFirstResponder(state.composer))
        press(17, "t", modifiers: .command, in: window)
        waitFor { state.listRequest == nil && window.firstResponder !== state.composer }
        XCTAssertNil(state.listRequest)
        XCTAssertFalse(window.firstResponder === state.composer)
        let viewport = try XCTUnwrap(descendant(DesktopChatListViewport.self, in: window.contentView!))
        let scroll = try XCTUnwrap(viewport.enclosingScrollView)
        let original = scroll.contentView.bounds.origin.y
        let responder = window.firstResponder
        for _ in 0..<80 { press(125, "\u{F701}", in: window) }
        XCTAssertGreaterThan(scroll.contentView.bounds.origin.y, original + 2_000,
                             "Plain arrows must move the actual long-list viewport")
        XCTAssertEqual(state.opened, "chat-0", "Viewport scrolling must not change the open chat")
        XCTAssertEqual(state.openCount, 0)
        XCTAssertTrue(window.firstResponder === responder, "Arrows must not move row focus")
        let lower = scroll.contentView.bounds.origin.y
        try capture(window, suffix: "-scrolled")
        for _ in 0..<80 { press(126, "\u{F700}", in: window) }
        XCTAssertLessThan(scroll.contentView.bounds.origin.y, lower)
        XCTAssertEqual(scroll.contentView.bounds.origin.y, original, accuracy: 1)
        XCTAssertTrue(window.firstResponder === responder)
        for _ in 0..<80 { press(125, "\u{F701}", in: window) }
        XCTAssertGreaterThan(scroll.contentView.bounds.origin.y, original + 2_000)
        XCTAssertEqual(state.openCount, 0)
        press(36, "\r", in: window)
        waitFor { window.firstResponder === state.composer }
        XCTAssertEqual(state.opened, "chat-0", "Enter opens the focused row, not an arrow-selected row")
        XCTAssertEqual(state.openCount, 1)
        XCTAssertTrue(window.firstResponder === state.composer)
        press(48, "\t", in: window)
        XCTAssertEqual(state.composer.string, "\t")

        press(17, "t", modifiers: .command, in: window)
        waitFor { state.listRequest == nil }
        press(48, "\t", in: window)
        press(36, "\r", in: window)
        waitFor { window.firstResponder === state.composer }
        XCTAssertEqual(state.opened, "chat-1", "Tab advances row focus and Enter opens that row")
        XCTAssertEqual(state.openCount, 2)
        press(97, "\u{F709}", modifiers: [.command, .function], in: window)
        waitFor { state.listRequest == nil && window.firstResponder !== state.composer }
        press(17, "t", modifiers: [.command, .shift], in: window)
        waitFor { window.firstResponder === state.composer }
        XCTAssertTrue(window.firstResponder === state.composer)
        try capture(window)
    }

    private func capture(_ window: NSWindow, suffix: String = "") throws {
        if let path = ProcessInfo.processInfo.environment["IRIS_KEYBOARD_SCREENSHOT"],
           let view = window.contentView, let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let url = URL(fileURLWithPath: path).deletingPathExtension()
                .appendingPathExtension(suffix.isEmpty ? "png" : "scrolled.png")
            try bitmap.representation(using: .png, properties: [:])?.write(to: url)
        }
    }

    private func descendant<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let match = view as? T { return match }
        return view.subviews.lazy.compactMap { self.descendant(type, in: $0) }.first
    }

    private func waitFor(_ condition: () -> Bool) {
        let deadline = Date(timeIntervalSinceNow: 5)
        while !condition() && Date() < deadline { pumpEvents() }
    }

    private func prepareApplication() {
        _ = NSApplication.shared
        #if IRIS_KEYBOARD_HARNESS
        NSApp.setActivationPolicy(.accessory)
        NSApp.finishLaunching()
        NSApp.activate(ignoringOtherApps: true)
        #endif
    }

    private func keyboardButton(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .frame(width: 280, height: 32)
            .buttonStyle(IrisKeyboardButtonStyle(visual: KeyboardTestButtonVisualStyle()))
    }

    private func press(_ code: UInt16, _ characters: String,
                       modifiers: NSEvent.ModifierFlags = [], in window: NSWindow) {
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            let event = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: modifiers,
                                        timestamp: ProcessInfo.processInfo.systemUptime,
                                        windowNumber: window.windowNumber, context: nil,
                                        characters: characters, charactersIgnoringModifiers: characters,
                                        isARepeat: false, keyCode: code)!
            NSApp.postEvent(event, atStart: false)
        }
        pumpEvents()
    }

    private func pumpEvents() {
        let until = Date(timeIntervalSinceNow: 0.1)
        while Date() < until {
            if let event = NSApp.nextEvent(matching: .any, until: until, inMode: .default, dequeue: true) {
                NSApp.sendEvent(event)
            }
        }
    }
}

private struct KeyboardTestButtonVisualStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label }
}

private final class KeyboardFocusTestState: ObservableObject {
    @Published var request: UUID?
    var focused = false
    var focusApplications = 0
}

private struct KeyboardFocusTestView: View {
    @ObservedObject var state: KeyboardFocusTestState
    let generation: Int

    var body: some View {
        Color.clear.frame(width: 100, height: 50)
            .modifier(DesktopComposerKeyboardFocus(isFocused: Binding(
                get: { state.focused },
                set: { state.focused = $0; if $0 { state.focusApplications += 1 } }
            )))
            .environment(\.desktopComposerFocusRequest, $state.request)
            .id(generation)
    }
}
private final class KeyboardListTestState: ObservableObject {
    @Published var listRequest: UUID?
    @Published var composerRequest: UUID?
    @Published var composerFocused = false
    @Published var opened = "chat-0"
    var openCount = 0
    let composer = IrisComposerNSTextView(frame: .zero)
}

private struct KeyboardListTestView: View {
    @ObservedObject var state: KeyboardListTestState
    var body: some View {
        HStack {
            ScrollViewReader { proxy in
                ScrollView {
                    DesktopKeyboardChatList(items: (0..<120).map { "chat-\($0)" }, id: \.self,
                                            selectedChatID: state.opened, proxy: proxy, onOpen: { id in
                        state.opened = id
                        state.openCount += 1
                        state.composerRequest = UUID()
                    }) { id in
                        Text(id).frame(maxWidth: .infinity, minHeight: 48)
                            .background(state.opened == id ? Color.accentColor.opacity(0.2) : Color.clear)
                    }
                }
            }
            .environment(\.desktopChatListFocusRequest, $state.listRequest)
            .frame(width: 280)
            KeyboardComposerTestView(composer: state.composer, isFocused: state.composerFocused)
                .modifier(DesktopComposerKeyboardFocus(isFocused: $state.composerFocused))
                .environment(\.desktopComposerFocusRequest, $state.composerRequest)
        }
        .background {
            DesktopChatSectionShortcuts(onChatList: {
                state.composerFocused = false
                state.listRequest = UUID()
            }, onComposer: { state.composerRequest = UUID() })
                .frame(width: 0, height: 0)
        }
    }
}

private struct KeyboardComposerTestView: NSViewRepresentable {
    let composer: IrisComposerNSTextView
    let isFocused: Bool
    func makeNSView(context: Context) -> IrisComposerNSTextView { composer }
    func updateNSView(_ view: IrisComposerNSTextView, context: Context) {
        if isFocused { view.window?.makeFirstResponder(view) }
    }
}
#endif
