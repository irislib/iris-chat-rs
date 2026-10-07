import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class FipsBlePhysicalProbeTests: XCTestCase {
    private let device = String(repeating: "a", count: 64)
    private let owner = String(repeating: "b", count: 64)

    func testProbeRequiresExactIsolatedEnvironment() throws {
        let environment = ["IRIS_UI_TEST_RUN_ID": "ble-idle-" + String(repeating: "1", count: 32),
                           "IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX": device]
        XCTAssertNotNil(FipsBlePhysicalTarget(environment: environment))
        for runID in ["", "normal", "ble-idle-../ordinary", environment["IRIS_UI_TEST_RUN_ID"]! + "\n"] {
            var invalid = environment
            invalid["IRIS_UI_TEST_RUN_ID"] = runID
            XCTAssertNil(FipsBlePhysicalTarget(environment: invalid))
        }
        for key in environment.keys {
            var missing = environment
            missing.removeValue(forKey: key)
            XCTAssertNil(FipsBlePhysicalTarget(environment: missing))
        }
        for deviceID in ["", "not-a-key", device + "\n", String(repeating: "g", count: 64)] {
            var invalid = environment
            invalid["IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX"] = deviceID
            XCTAssertNil(FipsBlePhysicalTarget(environment: invalid))
        }
    }

    func testOnlyExactBluetoothDeviceAndOwnerQualify() throws {
        let target = try XCTUnwrap(FipsBlePhysicalTarget(environment: [
            "IRIS_UI_TEST_RUN_ID": "ble-idle-" + String(repeating: "1", count: 32),
            "IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX": device,
        ]))
        let service = IrisNearbyService()
        var peer = IrisNearbyPeer(id: device, name: "Peer", ownerPubkeyHex: owner,
                                  pictureURL: nil, profileEventID: nil, bluetoothRSSI: nil)
        func connected() -> Bool { target.isConnected(ownerHex: owner, bluetoothPeers: service.bluetoothPeers) }
        func apply(bluetooth: [String], lan: [String] = []) {
            service.applyScreenshotFixturePeers(peers: [peer], bluetoothPeerIDs: bluetooth, lanPeerIDs: lan)
        }
        XCTAssertFalse(connected())
        apply(bluetooth: [], lan: [device])
        XCTAssertFalse(connected(), "A LAN link does not prove Bluetooth connectivity")
        apply(bluetooth: ["another-device"])
        XCTAssertFalse(connected())
        apply(bluetooth: [device])
        XCTAssertTrue(connected())
        peer.ownerPubkeyHex = "another-owner"
        apply(bluetooth: [device])
        XCTAssertFalse(connected())
        peer.ownerPubkeyHex = nil
        apply(bluetooth: [device])
        XCTAssertFalse(connected())
        service.applyScreenshotFixturePeers(peers: [], bluetoothPeerIDs: [], lanPeerIDs: [])
        XCTAssertFalse(connected(), "A disconnected target must stop qualifying")
    }
}
