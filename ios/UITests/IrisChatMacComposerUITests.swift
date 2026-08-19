#if os(macOS)
import XCTest

final class IrisChatMacComposerUITests: IrisChatUITestCase {
    func testComposerKeepsSequentialTypingOrder() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        let input = editableElement(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        focusTextTarget(input, app: app)

        var expected = ""
        for character in "hello" {
            expected.append(character)
            app.typeKey(String(character), modifierFlags: [])
            XCTAssertTrue(
                waitUntil(timeout: 2) {
                    (input.value as? String) == expected
                },
                "composer value after typing \(character) was \((input.value as? String) ?? "<nil>"), expected \(expected)"
            )
        }

        app.typeKey(.return, modifierFlags: [])
        XCTAssertTrue(app.staticTexts["hello"].firstMatch.waitForExistence(timeout: 15))
    }
}
#endif
