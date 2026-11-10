import XCTest
import SwiftUI
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MessageURLRenderingTests: XCTestCase {
    func testDatabaseFilenamesRemainPlainText() {
        for filename in ["file.db", "file.sqlite", "FILE.DB", "File.SQLite", "file.db3", "file.sqlite3", "file.db-wal", "file.sqlite-shm", "chat.backup.sqlite"] {
            let source = "💬 Saved (\(filename)), [\(filename)]."
            XCTAssertTrue(messageURLMatches(in: source).isEmpty, filename)
            let rendered = linkedMessageAttributedString(source, foreground: .white)
            XCTAssertEqual(String(rendered.characters), source)
            XCTAssertTrue(rendered.runs.allSatisfy { $0.link == nil && $0.underlineStyle == nil }, filename)
        }
    }

    func testBareWebAddressesStillLink() {
        let addresses = ["iris.to", "example.com", "example.dev", "example.chat", "example.xyz", "example.zip", "db.com", "sqlite.org"]
        let source = addresses.joined(separator: " ")
        let matches = messageURLMatches(in: source)
        XCTAssertEqual(matches.map { String(source[$0.range]) }, addresses)
        XCTAssertEqual(matches.map { $0.url.absoluteString }, addresses.map { "https://" + $0 })
    }

    func testExplicitLinksAndDatabaseDownloadsStillLink() {
        let addresses = ["https://file.db", "http://file.sqlite", "HTTPS://FILE.DB", "www.file.sqlite", "https://example.com/file.db", "example.com/file.sqlite"]
        let source = addresses.joined(separator: " ")
        let matches = messageURLMatches(in: source)
        XCTAssertEqual(matches.map { String(source[$0.range]) }, addresses)
        XCTAssertEqual(matches.map { $0.url.absoluteString }, [
            "https://file.db", "http://file.sqlite", "HTTPS://FILE.DB", "https://www.file.sqlite",
            "https://example.com/file.db", "https://example.com/file.sqlite",
        ])
    }

    func testMixedMessageKeepsTextAndOnlyUnderlinesWebLinks() {
        let source = "💬 file.db and file.sqlite; visit (iris.to), or https://example.com/file.sqlite!"
        let rendered = linkedMessageAttributedString(source, foreground: .white)
        XCTAssertEqual(String(rendered.characters), source)
        XCTAssertEqual(rendered.runs.compactMap { $0.link?.absoluteString }, ["https://iris.to", "https://example.com/file.sqlite"])
        for run in rendered.runs {
            XCTAssertEqual(run.underlineStyle != nil, run.link != nil)
        }
    }

    #if os(macOS)
    func testNativeMessageLeavesFilenamesUnlinked() throws {
        _ = NSApplication.shared
        let source = "Saved file.db and file.sqlite.\nVisit iris.to or https://example.com/file.sqlite."
        let attributed = linkedMessageAttributedString(source, foreground: .primary)
        let native = IrisDesktopLinkedText.nativeText(attributed, foreground: .textColor)
        XCTAssertEqual(native.string, source)
        for filename in ["file.db", "file.sqlite"] {
            let range = (source as NSString).range(of: filename)
            XCTAssertNil(native.attribute(.link, at: range.location, effectiveRange: nil))
            XCTAssertNil(native.attribute(.underlineStyle, at: range.location, effectiveRange: nil))
        }
        let linkRange = (source as NSString).range(of: "iris.to")
        XCTAssertEqual(native.attribute(.link, at: linkRange.location, effectiveRange: nil) as? URL, URL(string: "https://iris.to"))

        if let directory = ProcessInfo.processInfo.environment["IRIS_URL_EVIDENCE_DIR"] {
            let host = NSHostingView(rootView: IrisDesktopLinkedText(attributed: attributed, foreground: .primary, lineLimit: nil)
                .padding(20).frame(width: 520).background(Color(nsColor: .windowBackgroundColor)))
            host.frame = NSRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            host.displayIfNeeded()
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("macos-message-links.png"))
        }
    }
    #endif
}
