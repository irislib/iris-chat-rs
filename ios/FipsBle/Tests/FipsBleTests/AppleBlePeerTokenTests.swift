import Foundation
@testable import FipsBle
import XCTest

final class AppleBlePeerTokenTests: XCTestCase {
    func testAppsOnOnePhoneHaveDistinctCandidatesWithTheSamePeripheral() {
        let peripheral = UUID()
        let first = AppleBlePeerToken.forCharacteristic(peripheral, instance: 1)
        let second = AppleBlePeerToken.forCharacteristic(peripheral, instance: 2)
        XCTAssertNotEqual(first, second)
        XCTAssertEqual(AppleBlePeerToken.peripheralIdentifier(first), peripheral)
        XCTAssertEqual(AppleBlePeerToken.peripheralIdentifier(second), peripheral)
        XCTAssertNotNil(first.range(of: "^[a-zA-Z0-9_.:-]{1,128}$", options: .regularExpression))
    }

    func testIncomingPeripheralTokensStillResolve() {
        let peripheral = UUID()
        XCTAssertEqual(AppleBlePeerToken.peripheralIdentifier(peripheral.uuidString), peripheral)
    }

    func testMalformedEndpointTokensAreRejected() {
        let peripheral = UUID().uuidString
        for token in ["", "invalid.1", "\(peripheral).", "\(peripheral).-1", "\(peripheral).1.2"] {
            XCTAssertNil(AppleBlePeerToken.peripheralIdentifier(token))
        }
    }
}
