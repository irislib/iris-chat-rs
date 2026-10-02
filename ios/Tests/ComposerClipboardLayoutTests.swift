import SwiftUI
import QuartzCore
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
    #if os(iOS)
    func testNativeComposerInitializesItsTextStorageAndFittingMeasurement() async throws {
        let draft = ClipboardComposerDraft(files: [])
        let host = UIHostingController(rootView: IrisTheme {
            ClipboardComposerFixture(draft: draft).frame(width: 390)
        })
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        let deadline = ProcessInfo.processInfo.systemUptime + 3
        while find(IrisComposerUITextView.self, in: host.view) == nil,
              ProcessInfo.processInfo.systemUptime < deadline {
            host.view.layoutIfNeeded()
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let editor = try XCTUnwrap(find(IrisComposerUITextView.self, in: host.view))
        XCTAssertNil(editor.textLayoutManager)
        XCTAssertTrue(editor.isScrollEnabled)
        XCTAssertFalse(editor.scrollsToTop)
        XCTAssertTrue(editor.textContainer.layoutManager?.textStorage === editor.textStorage)
        // Exercise the Swift stored object before any clipboard or keyboard
        // work, guarding against a factory that bypasses subclass initialization.
        let measurement = editor.composerMeasurement
        let height = measurement.height(for: editor.textStorage, width: 240,
                                        lineHeight: ceil(try XCTUnwrap(editor.font).lineHeight))
        XCTAssertGreaterThan(height, 0)
        XCTAssertTrue(measurement.layoutManager.textStorage === editor.textStorage)
        XCTAssertTrue(editor.composerMeasurement === measurement)
    }
    #endif

    func testPastedImagePreviewAndLargeDraftRenderWithoutGrowingPastFiveLines() async throws {
        #if os(macOS)
        let pasteboard = NSPasteboard.general
        let savedClipboard = (pasteboard.pasteboardItems ?? []).map { item -> NSPasteboardItem in
            let copy = NSPasteboardItem()
            for type in item.types {
                if let data = item.data(forType: type) { copy.setData(data, forType: type) }
            }
            return copy
        }
        defer { pasteboard.clearContents(); pasteboard.writeObjects(savedClipboard) }
        #endif
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
        let multiline = String(repeating: "A large pasted paragraph 🙂\nlet value = 42;\n", count: 1_000)
        let paragraph = String(repeating: "A large pasted paragraph 🙂 with code: let value = 42; ", count: 1_000)
        let cases = [("caption", "Here is the picture"), ("multiline", multiline), ("single-paragraph", paragraph)]
        #if os(iOS)
        let pasteBudgetMs = 1_500.0
        let followingEditBudgetMs = 500.0
        #else
        let pasteBudgetMs = 5_000.0
        let followingEditBudgetMs = 5_000.0
        #endif
        for (name, text) in cases {
            let operations = ComposerOperationProbe()
            let draft = ClipboardComposerDraft(files: attachments)
            let view = IrisTheme {
                VStack { Spacer(); ClipboardComposerFixture(draft: draft) }
                    .frame(width: 390, height: 360)
                    .background(Color.white)
                    .preferredColorScheme(.light)
            }
            #if os(macOS)
            let host = NSHostingView(rootView: view)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 390, height: 360),
                                  styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil) }
            try await Task.sleep(nanoseconds: 250_000_000)
            host.layoutSubtreeIfNeeded()
            let editor = try XCTUnwrap(find(IrisComposerNSTextView.self, in: host))
            XCTAssertTrue(window.makeFirstResponder(editor))
            let sample: () throws -> ComposerGeometry = {
                operations.measure("sample-host-layout") { host.layoutSubtreeIfNeeded() }
                operations.measure("sample-transaction-flush") { CATransaction.flush() }
                let viewport = try XCTUnwrap(editor.enclosingScrollView)
                let selection = editor.selectedRange()
                let caretOnScreen = operations.measure("sample-caret-query") {
                    editor.firstRect(forCharacterRange: selection, actualRange: nil)
                }
                let caret = editor.convert(window.convertFromScreen(caretOnScreen), from: nil)
                return ComposerGeometry(frame: editor.convert(editor.bounds, to: host), viewport: editor.visibleRect,
                                        caret: caret, height: viewport.contentView.bounds.height,
                                        maximumHeight: IrisAppKitComposerTextView.maxHeight(for: editor), selection: selection,
                                        scrollingEnabled: viewport.hasVerticalScroller,
                                        firstResponder: window.firstResponder === editor, hasMarkedText: editor.hasMarkedText())
            }
            let screenshot: () throws -> Data = {
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            }
            pasteboard.clearContents()
            pasteboard.setString(text, forType: .string)
            let paste = { editor.paste(nil) }
            let smallEdit = { editor.insertText("!", replacementRange: editor.selectedRange()) }
            #else
            let host = UIHostingController(rootView: view)
            let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
            let window = UIWindow(windowScene: scene)
            window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
            window.rootViewController = host
            window.makeKeyAndVisible()
            defer { window.isHidden = true }
            try await Task.sleep(nanoseconds: 250_000_000)
            host.view.layoutIfNeeded()
            let editor = try XCTUnwrap(find(IrisComposerUITextView.self, in: host.view))
            XCTAssertNil(editor.textLayoutManager)
            XCTAssertTrue(editor.isScrollEnabled)
            XCTAssertFalse(editor.scrollsToTop)
            XCTAssertTrue(editor.becomeFirstResponder())
            defer { editor.resignFirstResponder() }
            #if DEBUG
            editor.onLayoutTiming = { operations.record($0, milliseconds: $1) }
            defer { editor.onLayoutTiming = nil }
            #endif
            let sample: () throws -> ComposerGeometry = {
                operations.measure("sample-host-layout") { host.view.layoutIfNeeded() }
                operations.measure("sample-transaction-flush") { CATransaction.flush() }
                let selection = editor.selectedRange
                let caret = try operations.measure("sample-caret-query") {
                    editor.caretRect(for: try XCTUnwrap(editor.selectedTextRange).end)
                }
                return ComposerGeometry(frame: editor.convert(editor.bounds, to: host.view), viewport: editor.bounds,
                                        caret: caret, height: editor.bounds.height,
                                        maximumHeight: ceil(try XCTUnwrap(editor.font).lineHeight * 5), selection: selection,
                                        scrollingEnabled: editor.isScrollEnabled, firstResponder: editor.isFirstResponder,
                                        hasMarkedText: editor.markedTextRange != nil)
            }
            let screenshot: () throws -> Data = {
                try XCTUnwrap(UIGraphicsImageRenderer(bounds: host.view.bounds).image { _ in
                    host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
                }.pngData())
            }
            let textProvider = NSItemProvider(object: text as NSString)
            let paste = { editor.paste(itemProviders: [textProvider]) }
            let smallEdit = { editor.insertText("!") }
            #endif
            #if DEBUG
            editor.composerMeasurement.onLayoutTiming = { operations.record("fitting-layout", milliseconds: $0) }
            defer { editor.composerMeasurement.onLayoutTiming = nil }
            #endif
            // Reading the legacy layoutManager would itself force compatibility
            // mode. This optional modern accessor only observes the live engine.
            let enginePresence = { editor.textLayoutManager != nil }
            // Window, keyboard and thumbnail setup are outside both measurements.
            try await Task.sleep(nanoseconds: 400_000_000)
            _ = try sample()
            try await measureChange("\(name)-paste", expected: text, draft: draft, operations: operations,
                                    enginePresence: enginePresence, settledBudgetMs: pasteBudgetMs,
                                    action: paste, sample: sample)
            attachScreenshot(try screenshot(), name: "\(name)-paste")
            XCTAssertEqual(draft.files, attachments)
            XCTAssertTrue(draft.directly)
            XCTAssertFalse(draft.didSend)
            try await measureChange("\(name)-following-edit", expected: text + "!", draft: draft, operations: operations,
                                    enginePresence: enginePresence, settledBudgetMs: followingEditBudgetMs,
                                    action: smallEdit, sample: sample)
            attachScreenshot(try screenshot(), name: "\(name)-following-edit")
            XCTAssertEqual(draft.files, attachments)
            XCTAssertTrue(draft.directly)
            XCTAssertFalse(draft.didSend)
            #if os(iOS)
            XCTAssertNil(editor.textLayoutManager)
            XCTAssertTrue(editor.isScrollEnabled)
            XCTAssertFalse(editor.scrollsToTop)
            #endif
        }
    }

    private func measureChange(_ name: String, expected: String, draft: ClipboardComposerDraft,
                               operations: ComposerOperationProbe, enginePresence: () -> Bool, settledBudgetMs: Double,
                               action: () -> Void, sample: () throws -> ComposerGeometry) async throws {
        let expectedSelection = NSRange(location: (expected as NSString).length, length: 0)
        let updated = expectation(description: "\(name) reaches the composer draft")
        var updates = 0
        var draftUpdateMs: Double = -1
        operations.reset()
        let engineBeforeEdit = enginePresence()
        let started = ProcessInfo.processInfo.systemUptime
        let runLoop = ComposerRunLoopProbe()
        defer { runLoop.invalidate() }
        draft.onUserEdit = { value in
            updates += 1
            if value == expected, draftUpdateMs < 0 {
                draftUpdateMs = (ProcessInfo.processInfo.systemUptime - started) * 1_000
                runLoop.mark("draft-update")
                updated.fulfill()
            }
        }
        defer { draft.onUserEdit = { _ in } }
        action()
        let nativeCallMs = (ProcessInfo.processInfo.systemUptime - started) * 1_000
        let engineAfterNativeEdit = enginePresence()
        runLoop.mark("native-edit-return")
        await fulfillment(of: [updated], timeout: max(0.01, 5 - nativeCallMs / 1_000))
        runLoop.mark("draft-fulfillment-return")
        var previous: ComposerGeometry?
        var stableSamples = 0
        var layoutMs = 0.0
        var observationDelayMs = 0.0
        var actualObservationWaitMs = 0.0
        var longestObservationWaitMs = 0.0
        var layoutObservations: [[String: Any]] = []
        var settled = false
        repeat {
            let layoutStarted = ProcessInfo.processInfo.systemUptime
            let geometry = try sample()
            layoutMs += (ProcessInfo.processInfo.systemUptime - layoutStarted) * 1_000
            if previous != geometry, layoutObservations.count < 8 {
                layoutObservations.append(["elapsedMs": (ProcessInfo.processInfo.systemUptime - started) * 1_000,
                                           "geometry": geometry.diagnostics])
                runLoop.mark("hosted-geometry-change")
            }
            stableSamples = previous == geometry ? stableSamples + 1 : 0
            previous = geometry
            let correctHeight = geometry.height > 0 && geometry.height <= geometry.maximumHeight + 1 &&
                (expectedSelection.location < 1_000 || geometry.height >= geometry.maximumHeight - 1)
            if draft.composer.text == expected, geometry.selection == expectedSelection,
               geometry.caretVisible, correctHeight, stableSamples >= 2 {
                settled = true
                break
            }
            if ProcessInfo.processInfo.systemUptime - started >= 5 { break }
            // Sampling gives queued SwiftUI/layout/caret work a run-loop turn.
            // This deliberate delay is reported separately, not as processing.
            let waitStarted = ProcessInfo.processInfo.systemUptime
            try await Task.sleep(nanoseconds: 10_000_000)
            let waitedMs = (ProcessInfo.processInfo.systemUptime - waitStarted) * 1_000
            actualObservationWaitMs += waitedMs
            longestObservationWaitMs = max(longestObservationWaitMs, waitedMs)
            observationDelayMs += 10
        } while ProcessInfo.processInfo.systemUptime - started < 5
        let elapsedMs = (ProcessInfo.processInfo.systemUptime - started) * 1_000
        runLoop.mark(settled ? "settled" : "deadline")
        let runLoopValues = runLoop.finish()
        let engines = ["beforeEdit": engineBeforeEdit, "afterNativeEdit": engineAfterNativeEdit, "settled": enginePresence()]
        let values: [String: Any] = [
            "utf16Length": expectedSelection.location, "nativeEditCallMs": nativeCallMs,
            "draftUpdateMs": draftUpdateMs, "hostedLayoutAndGeometryMs": layoutMs,
            "settledElapsedMs": elapsedMs, "settledBudgetMs": settledBudgetMs,
            "intentionalObservationDelayMs": observationDelayMs,
            "actualObservationWaitMs": actualObservationWaitMs, "longestObservationWaitMs": longestObservationWaitMs,
            "layoutObservations": layoutObservations, "mainRunLoop": runLoopValues,
            "operations": operations.diagnostics, "textKit2Present": engines,
            "editNotifications": updates, "stableGeometrySamples": stableSamples,
            "caretVisible": previous?.caretVisible ?? false, "settled": settled,
            "geometry": previous?.diagnostics ?? [:],
            "attachments": draft.files.count, "directMode": draft.directly, "didSend": draft.didSend
        ]
        let evidence = XCTAttachment(data: try JSONSerialization.data(withJSONObject: values, options: [.sortedKeys]),
                                     uniformTypeIdentifier: "public.json")
        evidence.name = "hosted-composer-\(name)"
        evidence.lifetime = .keepAlways
        add(evidence)
        let logValues: [String: Any] = ["case": name, "utf16Length": expectedSelection.location,
            "nativeEditCallMs": nativeCallMs, "draftUpdateMs": draftUpdateMs, "settledElapsedMs": elapsedMs,
            "settledBudgetMs": settledBudgetMs,
            "longestActiveTurnMs": runLoopValues["longestActiveTurnMs"] ?? 0,
            "operations": operations.diagnostics, "textKit2Present": engines]
        let logData = try JSONSerialization.data(withJSONObject: logValues, options: [.sortedKeys])
        print("COMPOSER_PHASES \(String(decoding: logData, as: UTF8.self))")
        XCTAssertEqual(draft.composer.text, expected)
        XCTAssertEqual(updates, 1, "A native edit should publish one whole draft")
        XCTAssertTrue(settled, "Draft, five-line viewport and visible caret must settle together")
        XCTAssertLessThanOrEqual(elapsedMs, 5_000, "Hosted edit exceeded the generous five-second freeze budget")
        XCTAssertLessThanOrEqual(elapsedMs, settledBudgetMs, "Full native edit, draft and caret settling regressed")
    }

    private func attachScreenshot(_ png: Data, name: String) {
        XCTAssertGreaterThan(png.count, 1_000)
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: UTType.png.identifier)
        attachment.name = "clipboard-composer-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
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
        return view.subviews.lazy.compactMap { self.find(type, in: $0) }.first
    }
    #else
    private func find<T: UIView>(_ type: T.Type, in view: UIView) -> T? {
        if let match = view as? T { return match }
        return view.subviews.lazy.compactMap { self.find(type, in: $0) }.first
    }
    #endif
}

@MainActor
private final class ClipboardComposerDraft: ObservableObject {
    let composer = IrisComposerState()
    @Published var files: [StagedAttachment]
    @Published var directly = true
    var didSend = false
    var onUserEdit: (String) -> Void = { _ in }

    init(files: [StagedAttachment]) { self.files = files }
}

private struct ComposerGeometry: Equatable {
    let frame: CGRect
    let viewport: CGRect
    let caret: CGRect
    let height: CGFloat
    let maximumHeight: CGFloat
    let selection: NSRange
    let scrollingEnabled: Bool
    let firstResponder: Bool
    let hasMarkedText: Bool

    var diagnostics: [String: Any] {
        func rect(_ value: CGRect) -> [Any] {
            [value.minX, value.minY, value.width, value.height].map { $0.isFinite ? Double($0) as Any : NSNull() }
        }
        return ["frame": rect(frame), "viewport": rect(viewport), "caret": rect(caret),
                "height": Double(height), "maximumHeight": Double(maximumHeight),
                "selectionLocation": selection.location, "selectionLength": selection.length,
                "scrollingEnabled": scrollingEnabled, "firstResponder": firstResponder, "hasMarkedText": hasMarkedText]
    }

    var caretVisible: Bool {
        !caret.isNull && !caret.isInfinite && caret.height > 0 &&
            caret.minY >= viewport.minY - 4 && caret.maxY <= viewport.maxY + 4
    }
}

@MainActor
private struct ClipboardComposerFixture: View {
    @ObservedObject var draft: ClipboardComposerDraft
    @FocusState private var focused: Bool

    var body: some View {
        IrisComposerBar(composerState: draft.composer, attachments: $draft.files, sendFilesDirectly: $draft.directly,
                        directFilesAllowed: true, placeholder: "Message", isSending: false, isUploading: false,
                        uploadFraction: nil, isFocused: $focused, onUserEdit: { draft.onUserEdit($0) }, onDraftChange: {},
                        onAttach: { _ in }, voiceRecordingAllowed: false, onStageVoice: { _ in [] },
                        onSendVoice: { _ in false }, onSend: { _ in draft.didSend = true })
    }
}
