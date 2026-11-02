import AppKit
import XCTest

let suite = DesktopMessageSelectionTests.defaultTestSuite
suite.run()
let result = suite.testRun!
print("Desktop message selection: \(result.executionCount) tests, \(result.totalFailureCount) failures")
exit(result.hasSucceeded ? 0 : 1)
