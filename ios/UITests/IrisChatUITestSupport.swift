import XCTest
#if os(macOS)
import AppKit
#endif

extension IrisChatUITests {
    func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    func typeText(_ text: String, into target: XCUIElement, app: XCUIApplication) {
#if os(macOS)
        app.activate()
        focusTextTarget(target, app: app)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        app.typeKey("v", modifierFlags: .command)
#else
        target.tap()
        if target.elementType == .textView {
            for character in text {
                target.typeText(String(character))
            }
        } else {
            target.typeText(text)
        }
#endif
    }

    func editableElement(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
#if os(macOS)
        concreteEditableElement(app, identifier: identifier) ?? element(app, identifier)
#else
        element(app, identifier)
#endif
    }

    func concreteEditableElement(_ app: XCUIApplication, identifier: String) -> XCUIElement? {
        for query in [app.textViews, app.textFields, app.secureTextFields] {
            let candidate = query.matching(identifier: identifier).firstMatch
            if candidate.exists {
                return candidate
            }
        }
        return nil
    }

    func focusTextTarget(_ target: XCUIElement, app: XCUIApplication) {
#if os(macOS)
        if target.identifier == "chatMessageInput" {
            let composer = element(app, "chatComposerBar")
            if composer.exists {
                composer.coordinate(withNormalizedOffset: CGVector(dx: 0.55, dy: 0.5)).tap()
                return
            }
        }
#endif
        target.tap()
    }

    func ensureMacWindowVisible(_ app: XCUIApplication) {
#if os(macOS)
        app.activate()
        if !app.windows.firstMatch.waitForExistence(timeout: 2) {
            app.typeKey("n", modifierFlags: .command)
            _ = app.windows.firstMatch.waitForExistence(timeout: 5)
        }
#endif
    }

    func assertKeyboardFocused(
        _ target: XCUIElement,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
#if os(macOS)
        let predicate = NSPredicate(format: "hasKeyboardFocus == true")
        let expectation = expectation(for: predicate, evaluatedWith: target)
        let result = XCTWaiter.wait(for: [expectation], timeout: timeout)
        XCTAssertEqual(result, .completed, "field did not autofocus", file: file, line: line)
#endif
    }
}
