import CoreBluetooth

protocol AppleBleScanner {
    var isScanning: Bool { get }
    func scanForPeripherals(withServices serviceUUIDs: [CBUUID]?, options: [String: Any]?)
    func stopScan()
}

extension CBCentralManager: AppleBleScanner {}

extension AppleBleScanner {
    func updateDiscoveryScan(for service: CBUUID, requested: Bool, bluetoothAudioActive: Bool) {
        if requested && !bluetoothAudioActive {
            if !isScanning {
                scanForPeripherals(withServices: [service], options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
            }
        } else if isScanning {
            stopScan()
        }
    }
}
