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

    func testMeasurementStopsAtVisibleLinesAndPreservesEntireDraftAndSelection() throws {
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
        let height = measurement.height(for: storage, width: 300, lineHeight: lineHeight)
        XCTAssertEqual(height, lineHeight * 5, accuracy: 1)
        let laidOut = measurement.layoutManager.firstUnlaidCharacterIndex()
        XCTAssertGreaterThan(laidOut, 0)
        XCTAssertLessThan(laidOut, 1_000, "Sizing must not lay out the whole pasted document")
        let sizingStarted = ProcessInfo.processInfo.systemUptime
        for _ in 0..<10 {
            XCTAssertEqual(measurement.height(for: storage, width: 300, lineHeight: lineHeight), height)
        }
        attachTiming(["utf16Length": storage.length, "laidOutCharacters": laidOut,
                      "boundedSizing10CallsMs": (ProcessInfo.processInfo.systemUptime - sizingStarted) * 1_000],
                     name: "composer-bounded-sizing")
        XCTAssertEqual(measurement.layoutManager.firstUnlaidCharacterIndex(), laidOut)
        XCTAssertEqual(storage.string, largeText)
        #if os(iOS)
        XCTAssertEqual(view.selectedRange.location, (largeText as NSString).length)
        #else
        XCTAssertEqual(view.selectedRange().location, (largeText as NSString).length)
        #endif

        storage.replaceCharacters(in: NSRange(location: storage.length, length: 0), with: "tail")
        XCTAssertEqual(measurement.height(for: storage, width: 300, lineHeight: lineHeight), height)
        XCTAssertLessThan(measurement.layoutManager.firstUnlaidCharacterIndex(), 1_000)
        XCTAssertEqual(storage.string, largeText + "tail")
    }

    func testShortDraftSizingMatchesNativeMeasurement() throws {
        for text in ["", "hello", "hello\nworld", "first\n", "hello 🙂", "A short paragraph that wraps once across the composer."] {
            #if os(iOS)
            let view = IrisComposerUITextView()
            view.font = .systemFont(ofSize: 16)
            view.textContainerInset = .zero
            view.textContainer.lineFragmentPadding = 0
            view.text = text
            let storage = view.textStorage
            let lineHeight = ceil(try XCTUnwrap(view.font).lineHeight)
            let nativeHeight = view.sizeThatFits(CGSize(width: 240, height: .greatestFiniteMagnitude)).height
            #else
            let view = IrisComposerNSTextView()
            view.font = .systemFont(ofSize: 16)
            view.string = text
            let storage = try XCTUnwrap(view.textStorage)
            let lineHeight = IrisAppKitComposerTextView.lineHeight(for: view)
            let nativeHeight = view.attributedString().boundingRect(
                with: CGSize(width: 240, height: .greatestFiniteMagnitude),
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
            let focus = FocusState<Bool>()
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
            let parent = IrisUIKitComposerTextView(text: binding, isFocused: focus.projectedValue,
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
            let parent = IrisAppKitComposerTextView(text: binding, isFocused: focus.projectedValue,
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
