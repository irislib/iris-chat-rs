#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class AttachmentStagingTests: XCTestCase {
    @MainActor
    func testBundledIrisLogoIsStagedAndDispatchedAsAnAttachment() throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let sourceDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: sourceDir)
        }
        try FileManager.default.createDirectory(at: sourceDir, withIntermediateDirectories: true)
        let sourceURL = sourceDir.appendingPathComponent("iris-logo.png")
        let logo = try XCTUnwrap(UIImage(named: "IrisLogo")?.pngData())
        try logo.write(to: sourceURL)

        let rust = MockRustApp(state: makeAppState(rev: 1))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let staged = try XCTUnwrap(manager.stageOutgoingAttachments([sourceURL]).first)

        XCTAssertTrue(staged.path.contains("/attachments/outgoing/"))
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: staged.path)), logo)

        manager.sendAttachments(
            chatId: "chat-logo-test",
            attachments: [staged],
            caption: "Iris logo"
        )
        XCTAssertTrue(rust.dispatchedActions.contains { action in
            if case let .sendAttachments(chatId, attachments, caption) = action {
                return chatId == "chat-logo-test"
                    && caption == "Iris logo"
                    && attachments.count == 1
                    && attachments[0].filename == "iris-logo.png"
                    && attachments[0].filePath == staged.path
            }
            return false
        })
    }
}
#endif
