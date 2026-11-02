import Darwin
import XCTest
@testable import IrisChat

final class IrisNetworkResourcesTests: XCTestCase {
    func testRaisesOnlySoftLimitWithinExistingHardLimit() {
        for (soft, hard, expected): (rlim_t, rlim_t, rlim_t) in [
            (256, 1024, 1024), (256, 2048, 2048), (256, rlim_t.max, 2048)
        ] {
            var writes = 0
            IrisNetworkResources.prepare(
                readLimit: { $0 = rlimit(rlim_cur: soft, rlim_max: hard); return 0 },
                writeLimit: {
                    writes += 1
                    XCTAssertEqual($0.rlim_cur, expected)
                    XCTAssertEqual($0.rlim_max, hard)
                    return 0
                }
            )
            XCTAssertEqual(writes, 1)
        }
    }

    func testPreservesHigherExistingOrRestrictiveLimits() {
        for (soft, hard): (rlim_t, rlim_t) in [
            (4096, rlim_t.max), (2048, rlim_t.max), (256, 256), (0, 0)
        ] {
            IrisNetworkResources.prepare(
                readLimit: { $0 = rlimit(rlim_cur: soft, rlim_max: hard); return 0 },
                writeLimit: { _ in XCTFail("Must preserve existing limits"); return -1 }
            )
        }
    }

    func testQueryFailureDoesNotAttemptToChangeLimits() {
        IrisNetworkResources.prepare(
            readLimit: { _ in -1 },
            writeLimit: { _ in XCTFail("No valid limit was read"); return -1 }
        )
    }

    func testSetFailureDoesNotRetryOrChangeHardLimit() {
        var writes = 0
        IrisNetworkResources.prepare(
            readLimit: { $0 = rlimit(rlim_cur: 256, rlim_max: 4096); return 0 },
            writeLimit: {
                writes += 1
                XCTAssertEqual($0.rlim_cur, 2048)
                XCTAssertEqual($0.rlim_max, 4096)
                return -1
            }
        )
        XCTAssertEqual(writes, 1)
    }
}
