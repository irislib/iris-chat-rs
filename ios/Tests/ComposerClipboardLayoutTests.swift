import SwiftUI
import UniformTypeIdentifiers
import XCTest
#if os(iOS)
import UIKit
@testable import IrisChat
#else
import AppKit
@testable import IrisChatMac
#endif

@MainActor
final class ComposerClipboardLayoutTests: XCTestCase {
    func testPastedImagePreviewAndLargeDraftRenderWithoutGrowingPastFiveLines() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let image = try clipboardImage()
        let provider = NSItemProvider(item: image as NSData, typeIdentifier: UTType.png.identifier)
        provider.suggestedName = "Pasted image.png"
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [provider]))
        let staging = IrisAttachmentStaging(dataDir: root, fileManager: .default)
        var attachments: [StagedAttachment] = []
        await clipboard.withURLs { load in
            do { attachments = try await staging.stageAsync(await load()) }
            catch { XCTFail("Clipboard image staging failed: \(error)") }
        }
        XCTAssertEqual(attachments.count, 1)
        let large = String(repeating: "A large pasted paragraph 🙂\nlet value = 42;\n", count: 500)
        for (name, text) in [("caption", "Here is the picture"), ("large-text", large)] {
            let view = IrisTheme {
                VStack { Spacer(); ClipboardComposerFixture(text: text, files: attachments) }
                    .frame(width: 390, height: 360)
                    .background(Color.white)
                    .preferredColorScheme(.light)
            }
            let png: Data
            #if os(macOS)
            let host = NSHostingView(rootView: view)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 390, height: 360),
                                  styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = host
            window.orderFront(nil)
            defer { window.orderOut(nil) }
            try await Task.sleep(nanoseconds: 250_000_000)
            host.layoutSubtreeIfNeeded()
            let editor = try XCTUnwrap(find(IrisComposerNSTextView.self, in: host))
            XCTAssertEqual(editor.string, text)
            let viewport = try XCTUnwrap(editor.enclosingScrollView)
            XCTAssertLessThanOrEqual(viewport.contentView.bounds.height, IrisAppKitComposerTextView.maxHeight(for: editor) + 1)
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            #else
            let host = UIHostingController(rootView: view)
            let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 360))
            window.rootViewController = host
            window.makeKeyAndVisible()
            defer { window.isHidden = true }
            try await Task.sleep(nanoseconds: 250_000_000)
            host.view.layoutIfNeeded()
            let editor = try XCTUnwrap(find(IrisComposerUITextView.self, in: host.view))
            XCTAssertEqual(editor.text, text)
            XCTAssertLessThanOrEqual(editor.bounds.height, ceil(try XCTUnwrap(editor.font).lineHeight * 5) + 1)
            png = try XCTUnwrap(UIGraphicsImageRenderer(bounds: host.view.bounds).image { _ in
                host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
            }.pngData())
            #endif
            XCTAssertGreaterThan(png.count, 1_000)
            let screenshot = XCTAttachment(data: png, uniformTypeIdentifier: UTType.png.identifier)
            screenshot.name = "clipboard-composer-\(name)"
            screenshot.lifetime = .keepAlways
            add(screenshot)
        }
    }

    private func clipboardImage() throws -> Data {
        #if os(macOS)
        let image = NSImage(size: NSSize(width: 96, height: 96))
        image.lockFocus()
        NSColor.systemBlue.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 96, height: 96)).fill()
        image.unlockFocus()
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        #else
        return try XCTUnwrap(UIGraphicsImageRenderer(size: CGSize(width: 96, height: 96)).image { context in
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 96, height: 96))
        }.pngData())
        #endif
    }

    #if os(macOS)
    private func find<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let match = view as? T { return match }
        return view.subviews.lazy.compactMap { find(type, in: $0) }.first
    }
    #else
    private func find<T: UIView>(_ type: T.Type, in view: UIView) -> T? {
        if let match = view as? T { return match }
        return view.subviews.lazy.compactMap { find(type, in: $0) }.first
    }
    #endif
}

@MainActor
private struct ClipboardComposerFixture: View {
    @StateObject private var composer: IrisComposerState
    @State private var files: [StagedAttachment]
    @State private var directly = true
    @FocusState private var focused: Bool

    init(text: String, files: [StagedAttachment]) {
        let composer = IrisComposerState()
        composer.text = text
        _composer = StateObject(wrappedValue: composer)
        _files = State(initialValue: files)
    }

    var body: some View {
        IrisComposerBar(composerState: composer, attachments: $files, sendFilesDirectly: $directly,
                        directFilesAllowed: true, placeholder: "Message", isSending: false, isUploading: false,
                        uploadFraction: nil, isFocused: $focused, onUserEdit: { _ in }, onDraftChange: {},
                        onAttach: { _ in }, voiceRecordingAllowed: false, onStageVoice: { _ in [] },
                        onSendVoice: { _ in false }, onSend: { _ in XCTFail("A paste preview must not send") })
    }
}
