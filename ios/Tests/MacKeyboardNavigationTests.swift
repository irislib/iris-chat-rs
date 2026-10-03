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
    func testChatArrowsMoveFocusWithoutOpeningAndKeepIdentityAcrossRefresh() {
        var selection = DesktopChatKeyboardSelection()
        selection.reconcile(["a", "b", "c"], selected: "b")
        XCTAssertEqual(selection.chatID, "b")
        selection.move(1, in: ["a", "b", "c"], selected: "b")
        XCTAssertEqual(selection.chatID, "c")
        selection.reconcile(["c", "a", "b"], selected: "b")
        XCTAssertEqual(selection.chatID, "c")
        selection.move(1, in: ["c", "a", "b"], selected: "b")
        XCTAssertEqual(selection.chatID, "a")
    }

    func testChatFocusHandlesEmptyRemovedAndBoundaryRows() {
        var selection = DesktopChatKeyboardSelection()
        selection.move(1, in: [], selected: nil)
        XCTAssertNil(selection.chatID)
        selection.reconcile(["a", "b"], selected: "a")
        selection.move(-1, in: ["a", "b"], selected: "a")
        XCTAssertEqual(selection.chatID, "a")
        selection.move(1, in: ["a", "b"], selected: "a")
        selection.move(1, in: ["a", "b"], selected: "a")
        XCTAssertEqual(selection.chatID, "b")
        selection.move(-2, in: ["a", "b"], selected: nil)
        XCTAssertEqual(selection.chatID, "a", "Home selects the first row")
        selection.move(2, in: ["a", "b"], selected: nil)
        XCTAssertEqual(selection.chatID, "b", "End selects the last row")
        selection.reconcile(["a"], selected: "b")
        XCTAssertEqual(selection.chatID, "a")
        selection.reconcile([], selected: "a")
        XCTAssertNil(selection.chatID)
    }

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

    func testComposerTabAndBacktabMoveFocusWithoutChangingDraft() {
        prepareApplication()
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 360, height: 160),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let before = KeyboardTestView(frame: NSRect(x: 10, y: 110, width: 80, height: 30))
        let composer = IrisComposerNSTextView(frame: NSRect(x: 10, y: 60, width: 300, height: 40))
        let after = KeyboardTestView(frame: NSRect(x: 10, y: 10, width: 80, height: 30))
        for view in [before, composer, after] { window.contentView?.addSubview(view) }
        window.autorecalculatesKeyViewLoop = false
        before.nextKeyView = composer
        composer.nextKeyView = after
        after.nextKeyView = before
        composer.string = "draft"
        XCTAssertTrue(window.makeFirstResponder(composer))
        composer.doCommand(by: #selector(NSResponder.insertTab(_:)))
        XCTAssertTrue(window.firstResponder === after)
        XCTAssertEqual(composer.string, "draft")
        XCTAssertTrue(window.makeFirstResponder(composer))
        composer.doCommand(by: #selector(NSResponder.insertBacktab(_:)))
        XCTAssertTrue(window.firstResponder === before)
        XCTAssertEqual(composer.string, "draft")
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

private final class KeyboardTestView: NSView {
    override var acceptsFirstResponder: Bool { true }
    override var canBecomeKeyView: Bool { true }
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
#endif
