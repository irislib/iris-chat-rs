import CoreBluetooth
import Foundation
@testable import FipsBle
import XCTest

final class AppleBleAdvertisementTests: XCTestCase {
    private let service = CBUUID(string: "9c90b792-2cc5-42c0-9f87-c9cc40648f4c")

    func testExplicitServiceAdvertisementAllowsDiscoveryDuringAudio() {
        XCTAssertTrue(AppleBleAdvertisement.explicitlyAdvertises(service, data: [
            CBAdvertisementDataServiceUUIDsKey: [service],
        ]))
    }

    func testServiceDataAlsoIdentifiesAnExplicitAdvertisement() {
        XCTAssertTrue(AppleBleAdvertisement.explicitlyAdvertises(service, data: [
            CBAdvertisementDataServiceDataKey: [service: Data([1])],
        ]))
    }

    func testTentativeOverflowMatchDoesNotIdentifyAnIrisPeer() {
        XCTAssertFalse(AppleBleAdvertisement.explicitlyAdvertises(service, data: [
            CBAdvertisementDataOverflowServiceUUIDsKey: [service],
            CBAdvertisementDataServiceUUIDsKey: [CBUUID(string: "180A")],
            CBAdvertisementDataIsConnectable: true,
        ]))
    }
}
