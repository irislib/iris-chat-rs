import XCTest

final class InstalledSelfDraftEscrowTests: XCTestCase {
    private let old = UUID().uuidString.lowercased()
    private let fresh = UUID().uuidString.lowercased()
    private let account = "test account payload"
    private var bytes: Data { Data(("s quality check " + old + "riI").utf8) }
    private var configuration: [String: String] {
        ["IRIS_QUALITY_PENDING_RUN_ID": old, "IRIS_QUALITY_RUN_ID": fresh,
         "IRIS_QUALITY_SELF_ESCROW_BASE64": bytes.base64EncodedString(), "IRIS_QUALITY_SELF_ESCROW_SHA256": InstalledSelfDraftEscrow.hash(bytes),
         "IRIS_QUALITY_ACCOUNT_QR_SHA256": InstalledSelfDraftEscrow.hash(Data(account.utf8)),
         "IRIS_QUALITY_CANDIDATE_BINDING_SHA256": InstalledSelfDraftEscrow.hash(Data("test candidate".utf8))]
    }

    func testOnlyExactRecordedSyntheticBytesAndOneMatchingAccountQualify() throws {
        let escrow = try InstalledSelfDraftEscrow(environment: configuration, originalRow: "original")
        try escrow.checkAccount([account])
        try escrow.checkSaved(String(decoding: bytes, as: UTF8.self))
        for values in [[], ["different"], [account, account]] { XCTAssertThrowsError(try escrow.checkAccount(values)) }
        for other in [nil, "", "personal draft", escrow.body, String(decoding: bytes.dropLast(), as: UTF8.self)] {
            XCTAssertThrowsError(try escrow.checkSaved(other))
        }
        for (key, value) in [("IRIS_QUALITY_RUN_ID", old), ("IRIS_QUALITY_PENDING_RUN_ID", fresh),
                             ("IRIS_QUALITY_SELF_ESCROW_SHA256", String(repeating: "0", count: 64)),
                             ("IRIS_QUALITY_SELF_ESCROW_BASE64", Data("personal draft".utf8).base64EncodedString())] {
            var different = configuration
            different[key] = value
            XCTAssertThrowsError(try InstalledSelfDraftEscrow(environment: different, originalRow: "original"))
        }
    }

    func testRecoveryRequiresThisRunsWitnessAndOnlyItsKnownState() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let escrow = try InstalledSelfDraftEscrow(environment: configuration, originalRow: "original")
        let oldText = String(decoding: bytes, as: UTF8.self)
        XCTAssertThrowsError(try escrow.recovery("", in: directory))
        try escrow.record("escrow", in: directory)
        XCTAssertThrowsError(try escrow.record("escrow", in: directory))
        XCTAssertEqual(try escrow.recovery(oldText, in: directory), .alreadyRestored)
        XCTAssertThrowsError(try escrow.recovery("", in: directory))
        try escrow.record("cleared", in: directory)
        XCTAssertEqual(try escrow.recovery("", in: directory), .empty)
        XCTAssertThrowsError(try escrow.recovery(escrow.body, in: directory))
        try escrow.record("typing", in: directory)
        XCTAssertEqual(try escrow.recovery(escrow.body, in: directory), .freshBody)
        for unknown in ["new personal draft", String(escrow.body.dropLast()), escrow.body + " "] {
            XCTAssertThrowsError(try escrow.recovery(unknown, in: directory))
        }
        let other = try InstalledSelfDraftEscrow(environment: configuration, originalRow: "changed row")
        XCTAssertThrowsError(try other.recovery(oldText, in: directory))
        try escrow.record("send", in: directory)
        XCTAssertThrowsError(try escrow.record("send", in: directory))
        try escrow.record("restored", in: directory)
        XCTAssertThrowsError(try escrow.recovery("", in: directory))
        XCTAssertThrowsError(try escrow.recovery(escrow.body, in: directory))
        XCTAssertEqual(try escrow.recovery(oldText, in: directory), .alreadyRestored)
    }
}
