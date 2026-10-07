import Foundation
import SwiftUI

/// Private physical-test diagnostics, absent from ordinary app launches.
struct FipsBlePhysicalTarget {
    let deviceHex: String

    init?(environment: [String: String]) {
        guard let runID = environment["IRIS_UI_TEST_RUN_ID"],
              runID.range(of: #"\Able-idle-[a-f0-9]{32}\z"#, options: .regularExpression) != nil,
              let device = environment["IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX"],
              device.range(of: #"\A[a-f0-9]{64}\z"#, options: .regularExpression) != nil else {
            return nil
        }
        deviceHex = device
    }

    func isConnected(ownerHex: String, bluetoothPeers: [IrisNearbyPeer]) -> Bool {
        bluetoothPeers.contains { $0.id == deviceHex && $0.ownerPubkeyHex == ownerHex }
    }
}

struct FipsBlePhysicalProbeRow: View {
    @ObservedObject var service: IrisNearbyService
    let target: FipsBlePhysicalTarget
    let ownerHex: String

    var body: some View {
        let connected = target.isConnected(ownerHex: ownerHex, bluetoothPeers: service.bluetoothPeers)
        HStack {
            Text("Bluetooth test link")
            Spacer()
            Text(connected ? "Connected" : "Waiting")
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Bluetooth test link")
        .accessibilityValue(connected ? "Connected" : "Waiting")
        .accessibilityIdentifier("physicalBluetoothTargetLink")
    }
}
