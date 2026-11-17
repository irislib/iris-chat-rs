import CoreBluetooth
@testable import FipsBle
import XCTest

final class AppleBleScannerTests: XCTestCase {
    private final class Radio: AppleBleScanner {
        var isScanning = false
        var operations: [String] = []
        func scanForPeripherals(withServices serviceUUIDs: [CBUUID]?, options: [String: Any]?) {
            XCTAssertEqual(serviceUUIDs, [CBUUID(string: "ABCD")])
            XCTAssertEqual(options?[CBCentralManagerScanOptionAllowDuplicatesKey] as? Bool, false)
            isScanning = true
            operations.append("start")
        }
        func stopScan() {
            isScanning = false
            operations.append("stop")
        }
    }

    func testPlaybackAtStartupAndDuringDiscovery() {
        let radio = Radio()
        let service = CBUUID(string: "ABCD")
        func update(requested: Bool = true, playing: Bool) {
            radio.updateDiscoveryScan(for: service, requested: requested, bluetoothAudioActive: playing)
        }
        update(playing: true)
        XCTAssertTrue(radio.operations.isEmpty)
        update(playing: false)
        update(playing: false)
        update(playing: true)
        update(playing: true)
        XCTAssertEqual(radio.operations, ["start", "stop"])
        update(playing: false)
        update(playing: true)
        update(requested: false, playing: false)
        XCTAssertEqual(radio.operations, ["start", "stop", "start", "stop"])
    }
}
