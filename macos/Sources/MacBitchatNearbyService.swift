import Combine
import CoreBluetooth
import CryptoKit
import Foundation

struct MacNearbyBitchatPeer: Identifiable, Equatable {
    let id: String
    var nickname: String
    var serviceName: String
    var lastSeen: Date
}

struct MacNearbyBitchatMessage: Identifiable, Equatable {
    let id: UUID
    var sender: String
    var text: String
    var isLocal: Bool
    var timestamp: Date
}

final class MacBitchatNearbyService: NSObject, ObservableObject {
    @Published private(set) var isVisible = false
    @Published private(set) var status = "Off"
    @Published private(set) var peers: [MacNearbyBitchatPeer] = []
    @Published private(set) var messages: [MacNearbyBitchatMessage] = []

    private static let mainnetServiceUUID = CBUUID(string: "F47B5E2D-4A9E-4C5A-9B3F-8E1D2C3A4B5C")
    private static let testnetServiceUUID = CBUUID(string: "F47B5E2D-4A9E-4C5A-9B3F-8E1D2C3A4B5A")
    private static let characteristicUUID = CBUUID(string: "A1B2C3D4-E5F6-4A5B-8C9D-0E1F2A3B4C5D")

    private let noiseKey = Curve25519.KeyAgreement.PrivateKey()
    private let signingKey = Curve25519.Signing.PrivateKey()
    private lazy var peerID = Data(SHA256.hash(data: noiseKey.publicKey.rawRepresentation).prefix(8))

    private var centralManager: CBCentralManager?
    private var peripheralManager: CBPeripheralManager?
    private var localCharacteristics: [CBUUID: CBMutableCharacteristic] = [:]
    private var pendingServiceAdds = 0
    private var peripherals: [UUID: CBPeripheral] = [:]
    private var writableCharacteristics: [UUID: CBCharacteristic] = [:]
    private var notificationAssemblers: [UUID: MacBitchatNotificationAssembler] = [:]
    private var subscribedCentralChannels: [UUID: MacBitchatCentralChannel] = [:]
    private var centralAssemblers: [UUID: MacBitchatNotificationAssembler] = [:]
    private var pendingCentralNotifications: [(data: Data, channel: MacBitchatCentralChannel?)] = []
    private var serviceNames: [UUID: String] = [:]
    private var peerIDByPeripheral: [UUID: String] = [:]
    private var peerIDByCentral: [UUID: String] = [:]
    private var signingKeysByPeerID: [String: Data] = [:]
    private var announcedPeripherals: Set<UUID> = []
    private var announcedCentrals: Set<UUID> = []
    private var announceTimer: Timer?
    private var debugMessageToSendOnFirstPeer: String?
    private var didSendDebugMessage = false
    private var lastCentralStateLog: String?
    private var lastPeripheralStateLog: String?

    var sidebarSubtitle: String {
        if !isVisible {
            return "Off"
        }
        if peers.isEmpty {
            return status
        }
        return peers.count == 1 ? "1 nearby" : "\(peers.count) nearby"
    }

    func toggleVisibility() {
        setVisible(!isVisible)
    }

    func configureDebugMessageToSendOnFirstPeer(_ message: String?) {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines)
        debugMessageToSendOnFirstPeer = trimmed?.isEmpty == false ? trimmed : nil
    }

    func setVisible(_ visible: Bool) {
        guard visible != isVisible else {
            if visible {
                announceToConnectedPeripherals()
            }
            return
        }
        isVisible = visible
        if visible {
            NSLog("Iris nearby BitChat: visible on")
            start()
        } else {
            NSLog("Iris nearby BitChat: visible off")
            stop()
        }
    }

    func announceToConnectedPeripherals() {
        for (id, characteristic) in writableCharacteristics {
            guard let peripheral = peripherals[id] else { continue }
            sendAnnounce(to: peripheral, characteristic: characteristic)
        }
        for channel in subscribedCentralChannels.values {
            sendAnnounce(to: channel)
        }
    }

    func sendPublicMessage(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if !isVisible {
            setVisible(true)
        }
        guard let packet = signedPacket(
            type: MacBitchatMessageType.message.rawValue,
            ttl: 7,
            recipientID: Data(repeating: 0xff, count: MacBitchatPacket.senderIDSize),
            payload: Data(trimmed.utf8)
        ) else {
            status = "Send failed"
            return
        }
        sendPacket(packet)
        appendMessage(sender: "You", text: trimmed, isLocal: true, timestampMs: packet.timestampMs)
    }

    private func start() {
        status = "Starting"
        if centralManager == nil {
            centralManager = CBCentralManager(
                delegate: self,
                queue: .main,
                options: [CBCentralManagerOptionShowPowerAlertKey: true]
            )
        } else {
            startScanningIfReady()
        }
        if peripheralManager == nil {
            peripheralManager = CBPeripheralManager(
                delegate: self,
                queue: .main,
                options: [CBPeripheralManagerOptionShowPowerAlertKey: true]
            )
        } else {
            startAdvertisingIfReady()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            guard let self else { return }
            self.logBluetoothStates()
            self.startScanningIfReady()
            self.startAdvertisingIfReady()
        }
        announceTimer?.invalidate()
        announceTimer = Timer.scheduledTimer(withTimeInterval: 20, repeats: true) { [weak self] _ in
            self?.announceToConnectedPeripherals()
        }
    }

    private func stop() {
        status = "Off"
        announceTimer?.invalidate()
        announceTimer = nil
        centralManager?.stopScan()
        peripheralManager?.stopAdvertising()
        peripheralManager?.removeAllServices()
        for peripheral in peripherals.values {
            centralManager?.cancelPeripheralConnection(peripheral)
        }
        localCharacteristics.removeAll()
        pendingServiceAdds = 0
        peripherals.removeAll()
        writableCharacteristics.removeAll()
        notificationAssemblers.removeAll()
        subscribedCentralChannels.removeAll()
        centralAssemblers.removeAll()
        pendingCentralNotifications.removeAll()
        serviceNames.removeAll()
        peerIDByPeripheral.removeAll()
        peerIDByCentral.removeAll()
        signingKeysByPeerID.removeAll()
        announcedPeripherals.removeAll()
        announcedCentrals.removeAll()
        peers.removeAll()
    }

    private func startScanningIfReady() {
        guard isVisible, let centralManager else {
            return
        }
        guard centralManager.state == .poweredOn else {
            updateStatusForCentralState(centralManager.state)
            logCentralStateIfChanged()
            return
        }
        status = "Scanning"
        NSLog("Iris nearby BitChat: scanning")
        centralManager.scanForPeripherals(
            withServices: [Self.mainnetServiceUUID, Self.testnetServiceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
    }

    private func startAdvertisingIfReady() {
        guard isVisible, let peripheralManager else {
            return
        }
        guard peripheralManager.state == .poweredOn else {
            updateStatusForPeripheralState(peripheralManager.state)
            logPeripheralStateIfChanged()
            return
        }
        if localCharacteristics.isEmpty {
            addLocalServices()
            return
        }
        guard !peripheralManager.isAdvertising else { return }
        peripheralManager.startAdvertising([
            CBAdvertisementDataServiceUUIDsKey: [Self.mainnetServiceUUID, Self.testnetServiceUUID]
        ])
        NSLog("Iris nearby BitChat: advertising")
    }

    private func addLocalServices() {
        guard let peripheralManager else { return }
        peripheralManager.removeAllServices()
        localCharacteristics.removeAll()
        let serviceUUIDs = [Self.mainnetServiceUUID, Self.testnetServiceUUID]
        pendingServiceAdds = serviceUUIDs.count
        for uuid in serviceUUIDs {
            let characteristic = CBMutableCharacteristic(
                type: Self.characteristicUUID,
                properties: [.read, .write, .writeWithoutResponse, .notify],
                value: nil,
                permissions: [.readable, .writeable]
            )
            let service = CBMutableService(type: uuid, primary: true)
            service.characteristics = [characteristic]
            localCharacteristics[uuid] = characteristic
            peripheralManager.add(service)
        }
    }

    private func signedPacket(
        type: UInt8,
        ttl: UInt8,
        recipientID: Data?,
        payload: Data
    ) -> MacBitchatPacket? {
        var packet = MacBitchatPacket(
            version: 1,
            type: type,
            ttl: ttl,
            timestampMs: UInt64(Date().timeIntervalSince1970 * 1000),
            senderID: peerID,
            recipientID: recipientID,
            route: [],
            isRSR: false,
            payload: payload,
            signature: nil
        )
        guard let signingData = packet.signingData(),
              let signature = try? signingKey.signature(for: signingData) else {
            return nil
        }
        packet.signature = signature
        return packet
    }

    private func makeAnnouncePacket() -> MacBitchatPacket? {
        let announcement = MacBitchatAnnouncement(
            nickname: Host.current().localizedName ?? "Iris Mac",
            noisePublicKey: noiseKey.publicKey.rawRepresentation,
            signingPublicKey: signingKey.publicKey.rawRepresentation,
            directNeighbors: []
        )
        guard let payload = announcement.encode() else {
            status = "Announce failed"
            return nil
        }
        guard let packet = signedPacket(
            type: MacBitchatMessageType.announce.rawValue,
            ttl: 7,
            recipientID: nil,
            payload: payload
        ) else {
            status = "Sign failed"
            return nil
        }
        return packet
    }

    private func sendAnnounce(to peripheral: CBPeripheral, characteristic: CBCharacteristic) {
        guard isVisible else { return }
        guard let packet = makeAnnouncePacket(),
              let data = packet.encode(padding: true) else { return }
        write(data, to: peripheral, characteristic: characteristic)
        announcedPeripherals.insert(peripheral.identifier)
        status = "Announced"
        NSLog(
            "Iris nearby BitChat: sent announce to %@ (%ld bytes)",
            peripheral.identifier.uuidString,
            data.count
        )
    }

    private func sendAnnounce(to channel: MacBitchatCentralChannel) {
        guard isVisible else { return }
        guard let packet = makeAnnouncePacket(),
              let data = packet.encode(padding: true) else { return }
        notify(data, to: channel)
        announcedCentrals.insert(channel.central.identifier)
        status = "Announced"
        NSLog(
            "Iris nearby BitChat: sent announce to central %@ (%ld bytes)",
            channel.central.identifier.uuidString,
            data.count
        )
    }

    private func sendPacket(_ packet: MacBitchatPacket) {
        guard let data = packet.encode(padding: true) else {
            status = "Encode failed"
            return
        }
        for (id, characteristic) in writableCharacteristics {
            guard let peripheral = peripherals[id] else { continue }
            write(data, to: peripheral, characteristic: characteristic)
        }
        for channel in subscribedCentralChannels.values {
            notify(data, to: channel)
        }
        status = peers.isEmpty ? "Sent" : sidebarSubtitle
        NSLog("Iris nearby BitChat: sent packet type %d (%ld bytes)", packet.type, data.count)
    }

    private func write(_ data: Data, to peripheral: CBPeripheral, characteristic: CBCharacteristic) {
        let writeType: CBCharacteristicWriteType =
            characteristic.properties.contains(.write) ? .withResponse : .withoutResponse
        peripheral.writeValue(data, for: characteristic, type: writeType)
    }

    private func notify(_ data: Data, to channel: MacBitchatCentralChannel?) {
        guard let peripheralManager else { return }
        guard let characteristic = channel?.characteristic ?? localCharacteristics.values.first else { return }
        let maxLength = max(20, channel?.central.maximumUpdateValueLength ?? 180)
        var offset = 0
        while offset < data.count {
            let end = min(offset + maxLength, data.count)
            let chunk = Data(data[offset..<end])
            let sent = peripheralManager.updateValue(
                chunk,
                for: characteristic,
                onSubscribedCentrals: channel.map { [$0.central] }
            )
            if !sent {
                pendingCentralNotifications.append((Data(data[offset..<data.count]), channel))
                return
            }
            offset = end
        }
    }

    private func ingest(_ packet: MacBitchatPacket, from peripheral: CBPeripheral) {
        ingest(packet, source: .peripheral(peripheral))
    }

    private func ingest(_ packet: MacBitchatPacket, from central: CBCentral) {
        ingest(packet, source: .central(central))
    }

    private func ingest(_ packet: MacBitchatPacket, source: MacBitchatPeerSource) {
        if packet.senderID == peerID {
            return
        }
        let peerID = packet.senderID.map { String(format: "%02x", $0) }.joined()

        switch packet.type {
        case MacBitchatMessageType.announce.rawValue:
            ingestAnnounce(packet, peerID: peerID, source: source)
        case MacBitchatMessageType.message.rawValue:
            ingestMessage(packet, peerID: peerID)
        default:
            NSLog("Iris nearby BitChat: packet type %d from %@", packet.type, peerID)
        }
    }

    private func ingestAnnounce(_ packet: MacBitchatPacket, peerID: String, source: MacBitchatPeerSource) {
        guard let announcement = MacBitchatAnnouncement.decode(packet.payload) else {
            NSLog("Iris nearby BitChat: ignored malformed announce from %@", peerID)
            return
        }
        let derivedPeerID = Data(SHA256.hash(data: announcement.noisePublicKey).prefix(8))
            .map { String(format: "%02x", $0) }
            .joined()
        guard derivedPeerID == peerID else {
            NSLog("Iris nearby BitChat: ignored announce with mismatched key from %@", peerID)
            return
        }
        guard verifyPacketSignature(packet, publicKey: announcement.signingPublicKey) else {
            NSLog("Iris nearby BitChat: ignored unsigned announce from %@", peerID)
            return
        }

        let serviceName: String
        switch source {
        case .peripheral(let peripheral):
            serviceName = serviceNames[peripheral.identifier] ?? "BitChat"
            peerIDByPeripheral[peripheral.identifier] = peerID
        case .central(let central):
            serviceName = "BitChat"
            peerIDByCentral[central.identifier] = peerID
        }
        signingKeysByPeerID[peerID] = announcement.signingPublicKey
        let peer = MacNearbyBitchatPeer(
            id: peerID,
            nickname: announcement.nickname.isEmpty ? peerID : announcement.nickname,
            serviceName: serviceName,
            lastSeen: Date()
        )
        if let index = peers.firstIndex(where: { $0.id == peer.id }) {
            peers[index] = peer
        } else {
            peers.append(peer)
            peers.sort { $0.nickname.localizedCaseInsensitiveCompare($1.nickname) == .orderedAscending }
        }
        status = peers.count == 1 ? "1 nearby" : "\(peers.count) nearby"
        NSLog("Iris nearby BitChat: saw %@ (%@)", peer.nickname, peer.id)
        sendDebugMessageIfNeeded()
    }

    private func ingestMessage(_ packet: MacBitchatPacket, peerID: String) {
        if let recipientID = packet.recipientID,
           recipientID != Data(repeating: 0xff, count: MacBitchatPacket.senderIDSize),
           recipientID != self.peerID {
            return
        }
        if let signingKey = signingKeysByPeerID[peerID],
           !verifyPacketSignature(packet, publicKey: signingKey) {
            NSLog("Iris nearby BitChat: ignored message with bad signature from %@", peerID)
            return
        }
        guard let text = String(data: packet.payload, encoding: .utf8),
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let sender = peers.first(where: { $0.id == peerID })?.nickname ?? peerID
        appendMessage(sender: sender, text: text, isLocal: false, timestampMs: packet.timestampMs)
        status = sidebarSubtitle
        NSLog("Iris nearby BitChat: message from %@: %@", sender, text)
    }

    private func appendMessage(sender: String, text: String, isLocal: Bool, timestampMs: UInt64) {
        messages.append(
            MacNearbyBitchatMessage(
                id: UUID(),
                sender: sender,
                text: text,
                isLocal: isLocal,
                timestamp: Date(timeIntervalSince1970: Double(timestampMs) / 1000)
            )
        )
        if messages.count > 200 {
            messages.removeFirst(messages.count - 200)
        }
    }

    private func verifyPacketSignature(_ packet: MacBitchatPacket, publicKey: Data) -> Bool {
        guard let signature = packet.signature,
              let signingData = packet.signingData(),
              let key = try? Curve25519.Signing.PublicKey(rawRepresentation: publicKey) else {
            return false
        }
        return key.isValidSignature(signature, for: signingData)
    }

    private func logBluetoothStates() {
        logCentralStateIfChanged()
        logPeripheralStateIfChanged()
    }

    private func updateStatusForCentralState(_ state: CBManagerState) {
        switch state {
        case .poweredOff:
            status = "Bluetooth off"
        case .unauthorized:
            status = "No Bluetooth access"
        case .unsupported:
            status = "Bluetooth unavailable"
        case .resetting:
            status = "Bluetooth reset"
        case .unknown:
            status = "Bluetooth"
        case .poweredOn:
            break
        @unknown default:
            status = "Bluetooth"
        }
    }

    private func updateStatusForPeripheralState(_ state: CBManagerState) {
        switch state {
        case .poweredOff:
            status = "Bluetooth off"
        case .unauthorized:
            status = "No Bluetooth access"
        case .unsupported:
            status = "Bluetooth unavailable"
        case .resetting:
            status = "Bluetooth reset"
        case .unknown:
            status = "Bluetooth"
        case .poweredOn:
            break
        @unknown default:
            status = "Bluetooth"
        }
    }

    private func logCentralStateIfChanged() {
        guard let centralManager else { return }
        let state = bluetoothDescription(centralManager.state)
        guard state != lastCentralStateLog else { return }
        lastCentralStateLog = state
        NSLog("Iris nearby BitChat: central state %@", state)
    }

    private func logPeripheralStateIfChanged() {
        guard let peripheralManager else { return }
        let state = bluetoothDescription(peripheralManager.state)
        guard state != lastPeripheralStateLog else { return }
        lastPeripheralStateLog = state
        NSLog("Iris nearby BitChat: peripheral state %@", state)
    }

    private func bluetoothDescription(_ state: CBManagerState) -> String {
        switch state {
        case .unknown: return "unknown"
        case .resetting: return "resetting"
        case .unsupported: return "unsupported"
        case .unauthorized: return "unauthorized"
        case .poweredOff: return "powered off"
        case .poweredOn: return "powered on"
        @unknown default: return "unknown"
        }
    }

    private func sendDebugMessageIfNeeded() {
        guard !didSendDebugMessage, let debugMessageToSendOnFirstPeer, !peers.isEmpty else {
            return
        }
        didSendDebugMessage = true
        sendPublicMessage(debugMessageToSendOnFirstPeer)
    }
}

extension MacBitchatNearbyService: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        logCentralStateIfChanged()
        switch central.state {
        case .poweredOn:
            NSLog("Iris nearby BitChat: bluetooth powered on")
            startScanningIfReady()
        case .poweredOff:
            status = "Bluetooth off"
            NSLog("Iris nearby BitChat: bluetooth off")
        case .unauthorized:
            status = "No Bluetooth access"
            NSLog("Iris nearby BitChat: no bluetooth access")
        case .unsupported:
            status = "Bluetooth unavailable"
            NSLog("Iris nearby BitChat: bluetooth unavailable")
        case .resetting:
            status = "Bluetooth reset"
            NSLog("Iris nearby BitChat: bluetooth reset")
        case .unknown:
            status = "Bluetooth"
        @unknown default:
            status = "Bluetooth"
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi RSSI: NSNumber
    ) {
        guard isVisible else { return }
        if peripherals[peripheral.identifier] != nil {
            return
        }
        peripherals[peripheral.identifier] = peripheral
        peripheral.delegate = self
        if let uuids = advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID],
           uuids.contains(Self.testnetServiceUUID) {
            serviceNames[peripheral.identifier] = "BitChat test"
        } else {
            serviceNames[peripheral.identifier] = "BitChat"
        }
        status = "Connecting"
        NSLog("Iris nearby BitChat: discovered %@", peripheral.identifier.uuidString)
        central.connect(peripheral)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        status = "Connected"
        NSLog("Iris nearby BitChat: connected %@", peripheral.identifier.uuidString)
        peripheral.delegate = self
        peripheral.discoverServices(nil)
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        status = "Connect failed"
        peripherals.removeValue(forKey: peripheral.identifier)
        let message = error.map { String(describing: $0) } ?? "unknown"
        NSLog("Iris nearby BitChat: connect failed %@ %@", peripheral.identifier.uuidString, message)
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        peripherals.removeValue(forKey: peripheral.identifier)
        writableCharacteristics.removeValue(forKey: peripheral.identifier)
        notificationAssemblers.removeValue(forKey: peripheral.identifier)
        if let peerID = peerIDByPeripheral.removeValue(forKey: peripheral.identifier) {
            peers.removeAll { $0.id == peerID }
        }
        announcedPeripherals.remove(peripheral.identifier)
        if isVisible {
            status = peers.isEmpty ? "Scanning" : sidebarSubtitle
            startScanningIfReady()
        }
    }
}

extension MacBitchatNearbyService: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard error == nil, let services = peripheral.services else {
            status = "Service failed"
            let message = error.map { String(describing: $0) } ?? "no services"
            NSLog("Iris nearby BitChat: service discovery failed %@ %@", peripheral.identifier.uuidString, message)
            return
        }
        NSLog(
            "Iris nearby BitChat: services %@ %@",
            peripheral.identifier.uuidString,
            services.map { $0.uuid.uuidString }.joined(separator: ",")
        )
        let bitchatServices = services.filter {
            $0.uuid == Self.mainnetServiceUUID || $0.uuid == Self.testnetServiceUUID
        }
        guard !bitchatServices.isEmpty else {
            NSLog("Iris nearby BitChat: no BitChat service on %@", peripheral.identifier.uuidString)
            centralManager?.cancelPeripheralConnection(peripheral)
            return
        }
        for service in bitchatServices {
            peripheral.discoverCharacteristics([Self.characteristicUUID], for: service)
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard error == nil,
              let characteristic = service.characteristics?.first(where: { $0.uuid == Self.characteristicUUID }) else {
            status = "No channel"
            let message = error.map { String(describing: $0) } ?? "characteristic missing"
            NSLog("Iris nearby BitChat: characteristic discovery failed %@ %@", peripheral.identifier.uuidString, message)
            return
        }
        writableCharacteristics[peripheral.identifier] = characteristic
        notificationAssemblers[peripheral.identifier] = MacBitchatNotificationAssembler()
        NSLog("Iris nearby BitChat: found characteristic %@", peripheral.identifier.uuidString)
        if characteristic.properties.contains(.notify) {
            peripheral.setNotifyValue(true, for: characteristic)
        }
        sendAnnounce(to: peripheral, characteristic: characteristic)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard error == nil, let value = characteristic.value else { return }
        var assembler = notificationAssemblers[peripheral.identifier] ?? MacBitchatNotificationAssembler()
        let frames = assembler.append(value)
        notificationAssemblers[peripheral.identifier] = assembler
        for frame in frames {
            guard let packet = MacBitchatPacket.decode(frame) else { continue }
            ingest(packet, from: peripheral)
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
        if let error {
            status = "Write failed"
            NSLog("Iris nearby BitChat write failed: %@", "\(error)")
        }
    }
}

extension MacBitchatNearbyService: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        logPeripheralStateIfChanged()
        switch peripheral.state {
        case .poweredOn:
            NSLog("Iris nearby BitChat: peripheral bluetooth powered on")
            startAdvertisingIfReady()
        case .poweredOff:
            status = "Bluetooth off"
            NSLog("Iris nearby BitChat: peripheral bluetooth off")
        case .unauthorized:
            status = "No Bluetooth access"
            NSLog("Iris nearby BitChat: no peripheral bluetooth access")
        case .unsupported:
            status = "Bluetooth unavailable"
            NSLog("Iris nearby BitChat: peripheral bluetooth unavailable")
        case .resetting:
            status = "Bluetooth reset"
        case .unknown:
            status = "Bluetooth"
        @unknown default:
            status = "Bluetooth"
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService, error: Error?) {
        if let error {
            status = "Bluetooth failed"
            NSLog("Iris nearby BitChat: add service failed %@", String(describing: error))
            return
        }
        pendingServiceAdds = max(0, pendingServiceAdds - 1)
        if pendingServiceAdds == 0 {
            startAdvertisingIfReady()
        }
    }

    func peripheralManagerDidStartAdvertising(_ peripheral: CBPeripheralManager, error: Error?) {
        if let error {
            status = "Advertise failed"
            NSLog("Iris nearby BitChat: advertising failed %@", String(describing: error))
        } else {
            status = peers.isEmpty ? "Visible" : sidebarSubtitle
            NSLog("Iris nearby BitChat: advertising started")
        }
    }

    func peripheralManager(
        _ peripheral: CBPeripheralManager,
        central: CBCentral,
        didSubscribeTo characteristic: CBCharacteristic
    ) {
        guard let mutableCharacteristic = characteristic as? CBMutableCharacteristic else { return }
        let channel = MacBitchatCentralChannel(central: central, characteristic: mutableCharacteristic)
        subscribedCentralChannels[central.identifier] = channel
        centralAssemblers[central.identifier] = MacBitchatNotificationAssembler()
        status = "Connected"
        NSLog("Iris nearby BitChat: central subscribed %@", central.identifier.uuidString)
        sendAnnounce(to: channel)
    }

    func peripheralManager(
        _ peripheral: CBPeripheralManager,
        central: CBCentral,
        didUnsubscribeFrom characteristic: CBCharacteristic
    ) {
        subscribedCentralChannels.removeValue(forKey: central.identifier)
        centralAssemblers.removeValue(forKey: central.identifier)
        if let peerID = peerIDByCentral.removeValue(forKey: central.identifier) {
            peers.removeAll { $0.id == peerID }
        }
        announcedCentrals.remove(central.identifier)
        status = peers.isEmpty ? "Visible" : sidebarSubtitle
        NSLog("Iris nearby BitChat: central unsubscribed %@", central.identifier.uuidString)
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
        guard let packet = makeAnnouncePacket(),
              let data = packet.encode(padding: true),
              request.offset <= data.count else {
            peripheral.respond(to: request, withResult: .unlikelyError)
            return
        }
        request.value = Data(data.dropFirst(request.offset))
        peripheral.respond(to: request, withResult: .success)
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for request in requests {
            guard request.characteristic.uuid == Self.characteristicUUID,
                  let value = request.value else {
                if request.characteristic.properties.contains(.write) {
                    peripheral.respond(to: request, withResult: .requestNotSupported)
                }
                continue
            }
            let central = request.central
            var assembler = centralAssemblers[central.identifier] ?? MacBitchatNotificationAssembler()
            let frames = assembler.append(value)
            centralAssemblers[central.identifier] = assembler
            for frame in frames {
                guard let packet = MacBitchatPacket.decode(frame) else { continue }
                ingest(packet, from: central)
            }
            if request.characteristic.properties.contains(.write) {
                peripheral.respond(to: request, withResult: .success)
            }
        }
    }

    func peripheralManagerIsReady(toUpdateSubscribers peripheral: CBPeripheralManager) {
        let pending = pendingCentralNotifications
        pendingCentralNotifications.removeAll()
        for item in pending {
            notify(item.data, to: item.channel)
        }
    }
}

private struct MacBitchatCentralChannel {
    let central: CBCentral
    let characteristic: CBMutableCharacteristic
}

private enum MacBitchatPeerSource {
    case peripheral(CBPeripheral)
    case central(CBCentral)
}

private enum MacBitchatMessageType: UInt8 {
    case announce = 0x01
    case message = 0x02
    case leave = 0x03
    case noiseHandshake = 0x10
    case noiseEncrypted = 0x11
    case fragment = 0x20
    case requestSync = 0x21
    case fileTransfer = 0x22
}

private struct MacBitchatAnnouncement {
    var nickname: String
    var noisePublicKey: Data
    var signingPublicKey: Data
    var directNeighbors: [Data]

    func encode() -> Data? {
        var data = Data()
        guard appendTLV(type: 0x01, value: Data(nickname.utf8), to: &data),
              appendTLV(type: 0x02, value: noisePublicKey, to: &data),
              appendTLV(type: 0x03, value: signingPublicKey, to: &data) else {
            return nil
        }
        let neighbors = directNeighbors.prefix(10).reduce(into: Data()) { partial, neighbor in
            partial.append(neighbor.prefix(8))
        }
        if !neighbors.isEmpty {
            guard appendTLV(type: 0x04, value: neighbors, to: &data) else { return nil }
        }
        return data
    }

    static func decode(_ data: Data) -> MacBitchatAnnouncement? {
        let bytes = [UInt8](data)
        var offset = 0
        var nickname: String?
        var noisePublicKey: Data?
        var signingPublicKey: Data?
        var directNeighbors: [Data] = []

        while offset + 2 <= bytes.count {
            let type = bytes[offset]
            offset += 1
            let length = Int(bytes[offset])
            offset += 1
            guard offset + length <= bytes.count else { return nil }
            let value = Data(bytes[offset..<offset + length])
            offset += length

            switch type {
            case 0x01:
                nickname = String(data: value, encoding: .utf8)
            case 0x02:
                noisePublicKey = value
            case 0x03:
                signingPublicKey = value
            case 0x04 where value.count % 8 == 0:
                directNeighbors = stride(from: 0, to: value.count, by: 8).map {
                    Data(value[$0..<min($0 + 8, value.count)])
                }
            default:
                continue
            }
        }
        guard let nickname, let noisePublicKey, let signingPublicKey else { return nil }
        return MacBitchatAnnouncement(
            nickname: nickname,
            noisePublicKey: noisePublicKey,
            signingPublicKey: signingPublicKey,
            directNeighbors: directNeighbors
        )
    }

    private func appendTLV(type: UInt8, value: Data, to data: inout Data) -> Bool {
        guard value.count <= UInt8.max else { return false }
        data.append(type)
        data.append(UInt8(value.count))
        data.append(value)
        return true
    }
}

private struct MacBitchatPacket {
    static let senderIDSize = 8
    static let signatureSize = 64
    static let v1HeaderSize = 14
    static let v2HeaderSize = 16

    var version: UInt8
    var type: UInt8
    var ttl: UInt8
    var timestampMs: UInt64
    var senderID: Data
    var recipientID: Data?
    var route: [Data]
    var isRSR: Bool
    var payload: Data
    var signature: Data?

    func encode(padding: Bool) -> Data? {
        guard version == 1 || version == 2 else { return nil }
        guard version == 2 || payload.count <= UInt16.max else { return nil }
        guard route.count <= UInt8.max else { return nil }
        var flags: UInt8 = 0
        if recipientID != nil { flags |= 0x01 }
        if signature != nil { flags |= 0x02 }
        if version == 2 && !route.isEmpty { flags |= 0x08 }
        if isRSR { flags |= 0x10 }

        var data = Data()
        data.append(version)
        data.append(type)
        data.append(ttl)
        data.append(contentsOf: timestampMs.bigEndianBytes)
        data.append(flags)
        if version == 2 {
            data.append(contentsOf: UInt32(payload.count).bigEndianBytes)
        } else {
            data.append(contentsOf: UInt16(payload.count).bigEndianBytes)
        }
        data.append(senderID.fixedPeerID)
        if let recipientID {
            data.append(recipientID.fixedPeerID)
        }
        if version == 2 && !route.isEmpty {
            data.append(UInt8(route.count))
            for hop in route {
                data.append(hop.fixedPeerID)
            }
        }
        data.append(payload)
        if let signature {
            data.append(signature.prefix(Self.signatureSize))
        }
        return padding ? data.paddedForBitchat : data
    }

    func signingData() -> Data? {
        var packet = self
        packet.ttl = 0
        packet.isRSR = false
        packet.signature = nil
        return packet.encode(padding: true)
    }

    static func decode(_ data: Data) -> MacBitchatPacket? {
        let normalized = Data(data)
        return decodeCore(normalized) ?? decodeCore(normalized.unpaddedBitchat)
    }

    private static func decodeCore(_ data: Data) -> MacBitchatPacket? {
        let bytes = [UInt8](data)
        guard bytes.count >= v1HeaderSize + senderIDSize else { return nil }
        let version = bytes[0]
        guard version == 1 || version == 2 else { return nil }
        let headerSize = version == 2 ? v2HeaderSize : v1HeaderSize
        guard bytes.count >= headerSize + senderIDSize else { return nil }
        let type = bytes[1]
        let ttl = bytes[2]
        let timestamp = UInt64(bigEndianBytes: bytes[3..<11])
        let flags = bytes[11]
        let hasRecipient = (flags & 0x01) != 0
        let hasSignature = (flags & 0x02) != 0
        let isCompressed = (flags & 0x04) != 0
        let hasRoute = version == 2 && (flags & 0x08) != 0
        let isRSR = (flags & 0x10) != 0
        guard !isCompressed else { return nil }

        let payloadLength: Int
        var offset: Int
        if version == 2 {
            payloadLength = Int(UInt32(bigEndianBytes: bytes[12..<16]))
            offset = v2HeaderSize
        } else {
            payloadLength = Int(UInt16(bigEndianBytes: bytes[12..<14]))
            offset = v1HeaderSize
        }
        guard payloadLength >= 0, bytes.count >= offset + senderIDSize else { return nil }
        let senderID = Data(bytes[offset..<offset + senderIDSize])
        offset += senderIDSize
        let recipientID: Data?
        if hasRecipient {
            guard bytes.count >= offset + senderIDSize else { return nil }
            recipientID = Data(bytes[offset..<offset + senderIDSize])
            offset += senderIDSize
        } else {
            recipientID = nil
        }
        var route: [Data] = []
        if hasRoute {
            guard bytes.count > offset else { return nil }
            let count = Int(bytes[offset])
            offset += 1
            for _ in 0..<count {
                guard bytes.count >= offset + senderIDSize else { return nil }
                route.append(Data(bytes[offset..<offset + senderIDSize]))
                offset += senderIDSize
            }
        }
        guard bytes.count >= offset + payloadLength else { return nil }
        let payload = Data(bytes[offset..<offset + payloadLength])
        offset += payloadLength

        let signature: Data?
        if hasSignature {
            guard bytes.count >= offset + signatureSize else { return nil }
            signature = Data(bytes[offset..<offset + signatureSize])
        } else {
            signature = nil
        }

        return MacBitchatPacket(
            version: version,
            type: type,
            ttl: ttl,
            timestampMs: timestamp,
            senderID: senderID,
            recipientID: recipientID,
            route: route,
            isRSR: isRSR,
            payload: payload,
            signature: signature
        )
    }
}

private struct MacBitchatNotificationAssembler {
    private var buffer = Data()

    mutating func append(_ chunk: Data) -> [Data] {
        buffer.append(chunk)
        var frames: [Data] = []

        while buffer.count >= MacBitchatPacket.v1HeaderSize + MacBitchatPacket.senderIDSize {
            guard let frameLength = expectedFrameLength(buffer) else {
                buffer.removeFirst()
                continue
            }
            guard frameLength <= 1_048_576 else {
                buffer.removeAll()
                return frames
            }
            guard buffer.count >= frameLength else {
                return frames
            }
            frames.append(Data(buffer.prefix(frameLength)))
            buffer.removeFirst(frameLength)
        }

        return frames
    }

    private func expectedFrameLength(_ data: Data) -> Int? {
        let bytes = [UInt8](data)
        let version = bytes[0]
        guard version == 1 || version == 2 else { return nil }
        let headerSize = version == 2 ? MacBitchatPacket.v2HeaderSize : MacBitchatPacket.v1HeaderSize
        guard bytes.count >= headerSize + MacBitchatPacket.senderIDSize else { return nil }
        let flags = bytes[11]
        let hasRecipient = (flags & 0x01) != 0
        let hasSignature = (flags & 0x02) != 0
        let hasRoute = version == 2 && (flags & 0x08) != 0
        let payloadLength = version == 2
            ? Int(UInt32(bigEndianBytes: bytes[12..<16]))
            : Int(UInt16(bigEndianBytes: bytes[12..<14]))
        var length = headerSize + MacBitchatPacket.senderIDSize + payloadLength
        if hasRecipient { length += MacBitchatPacket.senderIDSize }
        if hasSignature { length += MacBitchatPacket.signatureSize }
        if hasRoute {
            let routeOffset = headerSize + MacBitchatPacket.senderIDSize + (hasRecipient ? MacBitchatPacket.senderIDSize : 0)
            guard bytes.count > routeOffset else { return nil }
            length += 1 + Int(bytes[routeOffset]) * MacBitchatPacket.senderIDSize
        }
        return length
    }
}

private extension Data {
    var fixedPeerID: Data {
        var result = Data(prefix(MacBitchatPacket.senderIDSize))
        if result.count < MacBitchatPacket.senderIDSize {
            result.append(Data(repeating: 0, count: MacBitchatPacket.senderIDSize - result.count))
        }
        return result
    }

    var paddedForBitchat: Data {
        let blockSizes = [256, 512, 1024, 2048]
        let totalSize = count + 16
        guard let target = blockSizes.first(where: { totalSize <= $0 }), target > count else {
            return self
        }
        let padding = target - count
        guard padding > 0, padding <= UInt8.max else { return self }
        var data = self
        data.append(Data(repeating: UInt8(padding), count: padding))
        return data
    }

    var unpaddedBitchat: Data {
        guard let last else { return self }
        let padding = Int(last)
        guard padding > 0, padding <= count else { return self }
        let tail = suffix(padding)
        guard tail.allSatisfy({ $0 == last }) else { return self }
        return Data(prefix(count - padding))
    }
}

private extension FixedWidthInteger {
    var bigEndianBytes: [UInt8] {
        withUnsafeBytes(of: self.bigEndian) { Array($0) }
    }
}

private extension FixedWidthInteger {
    init<Bytes: Collection>(bigEndianBytes bytes: Bytes) where Bytes.Element == UInt8 {
        self = bytes.reduce(Self.zero) { ($0 << 8) | Self($1) }
    }
}

private extension UInt32 {
    init(bigEndianBytes bytes: Data.SubSequence) {
        self = bytes.reduce(0) { ($0 << 8) | UInt32($1) }
    }
}

private extension UInt64 {
    init(bigEndianBytes bytes: Data.SubSequence) {
        self = bytes.reduce(0) { ($0 << 8) | UInt64($1) }
    }
}
