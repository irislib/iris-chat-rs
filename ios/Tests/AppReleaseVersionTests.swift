import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class AppReleaseVersionTests: XCTestCase {
    func testBundleVersionsRecoverTheSharedReleaseTag() {
        let cases: [(String, String?, String)] = [
            ("2026.9.8", "2026090800", "2026.9.8"),
            ("2026.7.28", "2026072801", "2026.7.28.1"),
            ("2026.9.801", "2026090801", "2026.9.8.1"),
            ("2026.9.899", "2026090899", "2026.9.8.99"),
            ("2026.9.900", "2026090900", "2026.9.9"),
            ("2026.10.100", "2026100100", "2026.10.1"),
            ("2027.1.100", "2027010100", "2027.1.1"),
            ("0.1.0", nil, "0.1.0"),
            ("0.1.0", "invalid", "0.1.0"),
            ("2026.9.8", "2026090801", "2026.9.8.1"),
        ]
        for (marketing, build, expected) in cases {
            XCTAssertEqual(
                irisReleaseVersion(marketingVersion: marketing, buildVersion: build),
                expected
            )
        }
    }
}
