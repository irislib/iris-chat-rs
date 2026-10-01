import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class NativeDirectFileTransferTests: XCTestCase {
    func testNativeFipsTransfersMultipleFilesOnlyAfterAcceptance() throws {
        let json = runDirectFileTransferSmoke(dataDir: FileManager.default.temporaryDirectory.path)
        let attachment = XCTAttachment(string: json)
        attachment.name = "native-direct-file-transfer"
        attachment.lifetime = .keepAlways
        add(attachment)
        let data = try XCTUnwrap(json.data(using: .utf8))
        let evidence = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(evidence["ok"] as? Bool, true, json)
        XCTAssertEqual(evidence["transport"] as? String, "fips-tcp")
        XCTAssertEqual((evidence["files"] as? [Any])?.count, 3)
        XCTAssertEqual(evidence["bytes_before_accept"] as? Int, 0)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
            try data.write(to: URL(fileURLWithPath: output).appendingPathComponent("native-direct-file-transfer.json"))
        }
    }
}
