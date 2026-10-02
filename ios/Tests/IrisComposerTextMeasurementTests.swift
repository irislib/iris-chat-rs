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
final class IrisComposerTextMeasurementTests: XCTestCase {
    private let largeText = String(repeating: "A pasted paragraph with unicode 🙂 and code: let value = 42;\n", count: 4_000)

    func testMeasurementCapsVisibleLayoutAndPreservesEntireDraftAndSelection() throws {
        #if os(iOS)
        let view = IrisComposerUITextView()
        view.font = .systemFont(ofSize: 16)
        view.text = largeText
        view.selectedRange = NSRange(location: (largeText as NSString).length, length: 0)
        let storage = view.textStorage
        let lineHeight = ceil(try XCTUnwrap(view.font).lineHeight)
        #else
        let view = IrisComposerNSTextView()
        view.font = .systemFont(ofSize: 16)
        view.string = largeText
        view.setSelectedRange(NSRange(location: (largeText as NSString).length, length: 0))
        let storage = try XCTUnwrap(view.textStorage)
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: view)
        #endif
        let measurement = view.composerMeasurement
        let initialStarted = ProcessInfo.processInfo.systemUptime
        let height = measurement.height(for: storage, width: 300, lineHeight: lineHeight)
        let initialFittingMs = (ProcessInfo.processInfo.systemUptime - initialStarted) * 1_000
        XCTAssertEqual(height, lineHeight * 5, accuracy: 1)
        let initialLayout = try assertLookaheadLayout(measurement)
        let sizingStarted = ProcessInfo.processInfo.systemUptime
        let warmHeights = (0..<10).map { _ in
            measurement.height(for: storage, width: 300, lineHeight: lineHeight)
        }
        let warmFittingMs = (ProcessInfo.processInfo.systemUptime - sizingStarted) * 1_000
        XCTAssertEqual(warmHeights, Array(repeating: height, count: 10))
        XCTAssertEqual(storage.string, largeText)
        #if os(iOS)
        XCTAssertEqual(view.selectedRange, NSRange(location: (largeText as NSString).length, length: 0))
        #else
        XCTAssertEqual(view.selectedRange(), NSRange(location: (largeText as NSString).length, length: 0))
        #endif

        storage.replaceCharacters(in: NSRange(location: storage.length, length: 0), with: "tail")
        #if os(iOS)
        let selectionAfterEdit = view.selectedRange
        #else
        let selectionAfterEdit = view.selectedRange()
        #endif
        XCTAssertEqual(measurement.height(for: storage, width: 300, lineHeight: lineHeight), height)
        let editedLayout = try assertLookaheadLayout(measurement)
        XCTAssertEqual(storage.string, largeText + "tail")
        #if os(iOS)
        XCTAssertEqual(view.selectedRange, selectionAfterEdit)
        #else
        XCTAssertEqual(view.selectedRange(), selectionAfterEdit)
        #endif
        attachTiming(["utf16Length": (largeText as NSString).length, "initialFittingMs": initialFittingMs,
                      "warmFitting10CallsMs": warmFittingMs, "initialLayout": initialLayout,
                      "afterEditLayout": editedLayout], name: "composer-capped-visible-sizing")
    }

    private func assertLookaheadLayout(_ measurement: IrisComposerTextMeasurement) throws -> [String: Any] {
        let manager = measurement.layoutManager
        let container = try XCTUnwrap(manager.textContainers.first)
        let used = manager.usedRect(for: container)
        XCTAssertGreaterThan(used.height, 0)
        XCTAssertGreaterThanOrEqual(used.minY, 0)
        let visible = manager.glyphRange(forBoundingRectWithoutAdditionalLayout: used, in: container)
        XCTAssertGreaterThan(visible.length, 0)
        var glyph = visible.location
        var lineBounds: [[Double]] = []
        while glyph < NSMaxRange(visible), lineBounds.count < 7 {
            var range = NSRange()
            let line = manager.lineFragmentRect(forGlyphAt: glyph, effectiveRange: &range, withoutAdditionalLayout: true)
            XCTAssertGreaterThan(line.height, 0)
            XCTAssertGreaterThanOrEqual(line.minY, 0)
            XCTAssertLessThanOrEqual(line.maxY, used.maxY + 1)
            lineBounds.append([Double(line.minY), Double(line.maxY)])
            guard NSMaxRange(range) > glyph else { XCTFail("Visible line range must advance"); break }
            glyph = NSMaxRange(range)
        }
        XCTAssertGreaterThan(lineBounds.count, 0)
        XCTAssertLessThanOrEqual(lineBounds.count, 6, "Fitting needs five lines plus one overflow lookahead")
        // Apple permits layout beyond the requested container. The processed
        // index is diagnostic only; it does not count physically visible lines.
        return ["firstUnlaidCharacterIndex": manager.firstUnlaidCharacterIndex(),
                "fittingGlyphLocation": visible.location, "fittingGlyphLength": visible.length,
                "fittingLineBounds": lineBounds, "usedMinY": Double(used.minY), "usedMaxY": Double(used.maxY)]
    }

    func testShortDraftSizingMatchesNativeMeasurement() throws {
        for text in ["", "hello", "hello\nworld", "first\n", "hello 🙂", "A short paragraph that wraps once across the composer.",
                     "one\ntwo\nthree\nfour\nfive", "one\ntwo\nthree\nfour\nfive\nsix", "🙂\n🙂\n🙂\n🙂\n🙂\n🙂"] {
            #if os(iOS)
            let view = IrisComposerUITextView()
            view.font = .systemFont(ofSize: 16)
            view.textContainerInset = .zero
            view.textContainer.lineFragmentPadding = 0
            view.text = text
            let storage = view.textStorage
            let lineHeight = ceil(try XCTUnwrap(view.font).lineHeight)
            let nativeHeight = view.sizeThatFits(CGSize(width: 240, height: CGFloat.greatestFiniteMagnitude)).height
            #else
            let view = IrisComposerNSTextView()
            view.font = .systemFont(ofSize: 16)
            view.string = text
            let storage = try XCTUnwrap(view.textStorage)
            let lineHeight = IrisAppKitComposerTextView.lineHeight(for: view)
            let nativeHeight = view.attributedString().boundingRect(
                with: CGSize(width: 240, height: CGFloat.greatestFiniteMagnitude),
                options: [.usesLineFragmentOrigin, .usesFontLeading]
            ).height
            #endif
            let expected = min(max(ceil(nativeHeight), lineHeight), lineHeight * 5)
            XCTAssertEqual(view.composerMeasurement.height(for: storage, width: 240, lineHeight: lineHeight),
                           expected, accuracy: 1, "Changed short draft sizing: \(text)")
        }
    }

    func testTextPasteUsesNativeEntryAndPublishesOneWholeEdit() async throws {
        #if os(macOS)
        let pasteboard = NSPasteboard.general
        let saved = (pasteboard.pasteboardItems ?? []).map { item -> NSPasteboardItem in
            let copy = NSPasteboardItem()
            for type in item.types {
                if let data = item.data(forType: type) { copy.setData(data, forType: type) }
            }
            return copy
        }
        defer { pasteboard.clearContents(); pasteboard.writeObjects(saved) }
        #endif
        for (name, text) in [("normal", "A pasted paragraph 🙂\nlet value = 42;"), ("large", largeText), ("rich-large", largeText)] {
            var draft = ""
            var userEdits: [String] = []
            var pasteToEditMs: Double = 0
            var started: TimeInterval = 0
            let didPaste = expectation(description: "native \(name) paste publishes complete draft")
            var richRepresentations: [(UTType, Data)] = []
            if name == "rich-large" {
                let richText = NSAttributedString(string: text)
                let range = NSRange(location: 0, length: richText.length)
                richRepresentations = [
                    (.html, try richText.data(from: range, documentAttributes: [.documentType: NSAttributedString.DocumentType.html])),
                    (.rtf, try richText.data(from: range, documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf]))
                ]
            }
            let binding = irisComposerUserEditingBinding(Binding(get: { draft }, set: { draft = $0 })) { value in
                userEdits.append(value)
                pasteToEditMs = (ProcessInfo.processInfo.systemUptime - started) * 1_000
                didPaste.fulfill()
            }
            #if os(iOS)
            let parent = IrisUIKitComposerTextView(text: binding, isFocused: .constant(false),
                                                  onPasteAttachments: { _ in XCTFail("Text must not stage attachments") })
            let coordinator = parent.makeCoordinator()
            let view = IrisComposerUITextView(frame: CGRect(x: 0, y: 0, width: 300, height: 100))
            view.allowsEditingTextAttributes = false
            view.delegate = coordinator
            view.onPasteAttachments = parent.onPasteAttachments
            let provider = NSItemProvider(object: text as NSString)
            for (type, data) in richRepresentations {
                provider.registerDataRepresentation(forTypeIdentifier: type.identifier, visibility: .all) { completion in
                    completion(data, nil)
                    return nil
                }
            }
            started = ProcessInfo.processInfo.systemUptime
            view.paste(itemProviders: [provider])
            #else
            let parent = IrisAppKitComposerTextView(text: binding, isFocused: .constant(false),
                                                   onSubmit: { _ in .rejected },
                                                   onPasteAttachments: { _ in XCTFail("Text must not stage attachments") })
            let coordinator = parent.makeCoordinator()
            let view = IrisComposerNSTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 100))
            view.isRichText = false
            view.importsGraphics = false
            view.allowsUndo = true
            view.delegate = coordinator
            view.onPasteAttachments = parent.onPasteAttachments
            pasteboard.clearContents()
            pasteboard.setString(text, forType: .string)
            for (type, data) in richRepresentations {
                pasteboard.setData(data, forType: NSPasteboard.PasteboardType(type.identifier))
            }
            started = ProcessInfo.processInfo.systemUptime
            view.paste(nil)
            #endif
            let pasteEntryReturnMs = (ProcessInfo.processInfo.systemUptime - started) * 1_000
            await fulfillment(of: [didPaste], timeout: 10)
            XCTAssertEqual(draft, text)
            XCTAssertEqual(userEdits, [text])
            #if os(iOS)
            XCTAssertEqual(view.text, text)
            XCTAssertEqual(view.selectedRange, NSRange(location: (text as NSString).length, length: 0))
            #else
            XCTAssertEqual(view.string, text)
            XCTAssertEqual(view.selectedRange(), NSRange(location: (text as NSString).length, length: 0))
            #endif
            attachTiming(["utf16Length": (text as NSString).length, "pasteToEditMs": pasteToEditMs,
                          "pasteEntryReturnMs": pasteEntryReturnMs, "editNotifications": userEdits.count],
                         name: "composer-\(name)-text-paste")
            withExtendedLifetime(coordinator) {}
        }
    }

    private func attachTiming(_ values: [String: Any], name: String) {
        guard let json = try? JSONSerialization.data(withJSONObject: values, options: [.sortedKeys]) else { return }
        let attachment = XCTAttachment(data: json, uniformTypeIdentifier: "public.json")
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
