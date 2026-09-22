import CoreBluetooth
import Foundation

enum AppleBleAdvertisement {
    static func explicitlyAdvertises(_ service: CBUUID, data: [String: Any]) -> Bool {
        if let services = data[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID], services.contains(service) {
            return true
        }
        let serviceData = data[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data]
        return serviceData?[service] != nil
    }
}
