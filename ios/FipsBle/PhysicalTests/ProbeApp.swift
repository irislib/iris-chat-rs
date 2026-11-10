import SwiftUI
import FipsBle

// A separate host keeps radio tests out of the installed Iris app and account.
@main
struct ProbeApp: App {
    @StateObject private var probe = BluetoothProbe()

    var body: some Scene {
        WindowGroup {
            Text(probe.status).accessibilityIdentifier("bluetoothProbeStatus")
                .onAppear { probe.start() }
        }
    }
}

final class BluetoothProbe: ObservableObject {
    @Published var status = "Bluetooth test"
    private var platform: AppleFipsBlePlatform?
    private var otherApp: AppleFipsBlePlatform?
    private var buffers: [UInt64: Data] = [:]
    private var connections: Set<UInt64> = []
    private var connecting = false
    private var received = false
    private var readySent = false
    private var request: UInt64 = 10

    func start() {
        guard otherApp == nil,
              let token = ProcessInfo.processInfo.environment["IRIS_BLE_PROBE_TOKEN"],
              token.count == 32, token.allSatisfy({ $0.isHexDigit }) else { return }
        // Deliberately publish another app's matching service before ours.
        let otherApp = AppleFipsBlePlatform()
        self.otherApp = otherApp
        otherApp.eventSink = { [weak self] event in
            switch event {
            case .listening:
                otherApp.submit(.startAdvertising(requestId: 2, bootstrap: Data("ignored!".utf8)))
            case .advertisingStarted: self?.startMain(token: token)
            case let .failed(_, message): self?.status = "Bluetooth error: \(message)"
            default: break
            }
        }
        otherApp.submit(.listen(requestId: 1, preferredPsm: 0))
    }

    private func startMain(token: String) {
        let prefix = Data("iris-ble-probe:\(token):".utf8)
        let hello = prefix + Data("apple".utf8)
        let expected = prefix + Data("android".utf8)
        let platform = AppleFipsBlePlatform()
        self.platform = platform
        status = "Finding Android"
        platform.eventSink = { [weak self] event in
            guard let self else { return }
            switch event {
            case let .listening(_, psm):
                platform.submit(.startAdvertising(requestId: 2, bootstrap: prefix + Data([
                    73, UInt8(psm & 255), UInt8(psm >> 8),
                ])))
            case let .peerDiscovered(peerToken, bootstrap):
                guard bootstrap.count == prefix.count + 3,
                      bootstrap.prefix(prefix.count) == prefix, bootstrap[prefix.count] == 65,
                      !self.connecting else { return }
                self.connecting = true
                self.status = "Found Android"
                self.updateCompletion(prefix: prefix)
                let psm = UInt16(bootstrap[prefix.count + 1]) | UInt16(bootstrap[prefix.count + 2]) << 8
                platform.submit(.connect(requestId: 4, peerToken: peerToken, psm: psm))
            case let .connected(_, id, _, _, _), let .incomingConnection(id, _, _, _):
                self.connections.insert(id)
                platform.submit(.write(requestId: self.request, connectionId: id, bytes: hello))
                self.request += 1
            case let .bytesReceived(id, bytes):
                var data = self.buffers[id] ?? Data()
                data.append(bytes)
                if data.count <= expected.count { self.buffers[id] = data }
                if data == expected {
                    self.received = true
                    self.updateCompletion(prefix: prefix)
                }
            case let .failed(_, message):
                if !self.readySent { self.status = "Bluetooth error: \(message)" }
            default: break
            }
        }
        platform.submit(.listen(requestId: 1, preferredPsm: 0))
        platform.submit(.startScanning(requestId: 3))
    }

    private func updateCompletion(prefix: Data) {
        guard connecting, received, !readySent, let platform else { return }
        readySent = true
        status = "Bluetooth exchange complete"
        for id in connections {
            platform.submit(.write(requestId: request, connectionId: id, bytes: prefix + Data("apple-ready".utf8)))
            request += 1
        }
    }
}
