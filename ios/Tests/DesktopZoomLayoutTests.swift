#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class DesktopZoomLayoutTests: XCTestCase {
    func testZoomRetainsTheNativeComposerAndDraft() throws {
        let model = ZoomFixtureModel()
        let host = NSHostingView(rootView: ZoomFixture(model: model))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 980, height: 640), styleMask: [.titled], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        settle(host)
        let input = try XCTUnwrap(editor(host))
        window.makeFirstResponder(input)
        IrisAppKitComposerTextView.insertTextAtSelection("See you soon", into: input)
        settle(host)
        XCTAssertEqual(model.composer.text, "See you soon")
        let normalFrame = input.convert(input.bounds, to: nil)
        model.level = 1
        settle(host)
        XCTAssertTrue(editor(host) === input)
        XCTAssertEqual(input.string, "See you soon")
        XCTAssertTrue(window.firstResponder === input)
        XCTAssertGreaterThan(input.convert(input.bounds, to: nil).height, normalFrame.height * 1.1)
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let attachment = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])), uniformTypeIdentifier: "public.png")
        attachment.name = "desktop-zoom-composer"
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
            try bitmap.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: output).appendingPathComponent("desktop-zoom-composer.png"))
        }
        model.level = 0
        settle(host)
        XCTAssertEqual(input.convert(input.bounds, to: nil).height, normalFrame.height, accuracy: 1)
        XCTAssertEqual(input.string, "See you soon")
    }

    private func settle(_ host: NSView) {
        RunLoop.main.run(until: Date().addingTimeInterval(0.15))
        host.layoutSubtreeIfNeeded()
    }

    private func editor(_ view: NSView) -> NSTextView? {
        if let text = view as? NSTextView { return text }
        return view.subviews.lazy.compactMap(editor).first
    }
}

@MainActor
private final class ZoomFixtureModel: ObservableObject {
    @Published var level = 0
    let composer = IrisComposerState()
}

private struct ZoomFixture: View {
    @ObservedObject var model: ZoomFixtureModel
    @FocusState private var focused: Bool
    var body: some View {
        VStack {
            Text("Weekend plans").font(.title2)
            Spacer()
            Text("Shall we meet at the park?").padding().background(IrisPalette.dark.bubbleTheirs, in: RoundedRectangle(cornerRadius: 16))
            IrisComposerBar(composerState: model.composer, attachments: .constant([]), placeholder: "Message", isSending: false,
                isUploading: false, uploadFraction: nil, isFocused: $focused, onUserEdit: { _ in }, onDraftChange: {},
                onAttach: { _ in }, voiceRecordingAllowed: true, onStageVoice: { _ in [] }, onSendVoice: { _ in false }, onSend: { _ in })
        }
        .padding()
        .background(IrisPalette.dark.background)
        .environment(\.irisPalette, .dark)
        .preferredColorScheme(.dark)
        .modifier(IrisDesktopZoom(level: model.level))
    }
}
#endif
