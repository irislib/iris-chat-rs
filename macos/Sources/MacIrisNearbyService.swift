import CoreBluetooth
import Compression
import Foundation

struct MacNearbyIrisPeer: Identifiable, Equatable {
    let id: String
    var name: String
    var lastSeen: Date
}

final class MacIrisNearbyService: NSObject, ObservableObject {
    @Published private(set) var isVisible = false
    @Published private(set) var status = "Off"
    @Published private(set) var peers: [MacNearbyIrisPeer] = []

    private static let serviceUUID = CBUUID(string: "8A0DAE01-D8E5-4F27-9F20-A616F1FBA6D0")
    private static let characteristicUUID = CBUUID(string: "8A0DAE02-D8E5-4F27-9F20-A616F1FBA6D0")
    fileprivate static let singleFrameBytes = 480
    fileprivate static let fragmentPayloadBytes = 180
    fileprivate static let maxEventFragments = 1024
    fileprivate static let maxIncomingFragmentSets = 64
    fileprivate static let fragmentTTL: TimeInterval = 30
    fileprivate static let maxEventBytes = 128 * 1024
    fileprivate static let maxMailbagEvents = 500
    fileprivate static let maxFrameBytes = 256 * 1024

    private let peerID = UUID().uuidString.lowercased()
    private var centralManager: CBCentralManager?
    private var peripheralManager: CBPeripheralManager?
    private var localCharacteristic: CBMutableCharacteristic?
    private var peripherals: [UUID: CBPeripheral] = [:]
    private var writableCharacteristics: [UUID: CBCharacteristic] = [:]
    private var peripheralAssemblers: [UUID: IrisNearbyFrameAssembler] = [:]
    private var subscribedCentrals: [UUID: MacIrisNearbyCentralChannel] = [:]
    private var centralAssemblers: [UUID: IrisNearbyFrameAssembler] = [:]
    private var pendingNotifications: [(data: Data, channel: MacIrisNearbyCentralChannel?)] = []
    private var peerIDByPeripheral: [UUID: String] = [:]
    private var peerIDByCentral: [UUID: String] = [:]
    private var ownOutbound: [String: IrisNearbyStoredEvent] = [:]
    private var forwarded: [String: IrisNearbyStoredEvent] = [:]
    private var incomingFragments: [String: IrisNearbyIncomingFragment] = [:]

    var ingestEventJson: ((String) -> Bool)?

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

    func setVisible(_ visible: Bool) {
        guard visible != isVisible else {
            if visible {
                announceToConnectedPeers()
            }
            return
        }
        isVisible = visible
        if visible {
            NSLog("Iris nearby: visible on")
            start()
        } else {
            NSLog("Iris nearby: visible off")
            stop()
        }
    }

    func publish(
        eventID: String,
        kind: UInt32,
        createdAtSecs: UInt64,
        eventJson: String
    ) {
        let record = IrisNearbyStoredEvent(
            id: eventID,
            kind: kind,
            createdAtSecs: createdAtSecs,
            eventJson: eventJson,
            storedAt: Date()
        )
        ownOutbound[eventID] = record
        forwarded.removeValue(forKey: eventID)
        pruneMailbags()
        guard isVisible else { return }
        sendEvent(record, excludingPeerID: nil)
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
    }

    private func stop() {
        status = "Off"
        centralManager?.stopScan()
        peripheralManager?.stopAdvertising()
        peripheralManager?.removeAllServices()
        for peripheral in peripherals.values {
            centralManager?.cancelPeripheralConnection(peripheral)
        }
        localCharacteristic = nil
        peripherals.removeAll()
        writableCharacteristics.removeAll()
        peripheralAssemblers.removeAll()
        subscribedCentrals.removeAll()
        centralAssemblers.removeAll()
        pendingNotifications.removeAll()
        peerIDByPeripheral.removeAll()
        peerIDByCentral.removeAll()
        incomingFragments.removeAll()
        peers.removeAll()
    }

    private func startScanningIfReady() {
        guard isVisible, let centralManager else { return }
        guard centralManager.state == .poweredOn else {
            status = bluetoothStatus(centralManager.state)
            return
        }
        status = "Scanning"
        centralManager.scanForPeripherals(
            withServices: [Self.serviceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
        NSLog("Iris nearby: scanning")
    }

    private func startAdvertisingIfReady() {
        guard isVisible, let peripheralManager else { return }
        guard peripheralManager.state == .poweredOn else {
            status = bluetoothStatus(peripheralManager.state)
            return
        }
        if localCharacteristic == nil {
            addLocalService()
            return
        }
        guard !peripheralManager.isAdvertising else { return }
        peripheralManager.startAdvertising([
            CBAdvertisementDataServiceUUIDsKey: [Self.serviceUUID],
            CBAdvertisementDataLocalNameKey: "Iris"
        ])
        NSLog("Iris nearby: advertising")
        status = peers.isEmpty ? "Visible" : sidebarSubtitle
    }

    private func addLocalService() {
        guard let peripheralManager else { return }
        peripheralManager.removeAllServices()
        let characteristic = CBMutableCharacteristic(
            type: Self.characteristicUUID,
            properties: [.write, .writeWithoutResponse, .notify],
            value: nil,
            permissions: [.writeable]
        )
        let service = CBMutableService(type: Self.serviceUUID, primary: true)
        service.characteristics = [characteristic]
        localCharacteristic = characteristic
        peripheralManager.add(service)
    }

    private func announceToConnectedPeers() {
        sendHello(excludingPeerID: nil)
        sendInventory(excludingPeerID: nil)
    }

    private func sendHello(excludingPeerID: String?) {
        let name = Host.current().localizedName ?? "Iris"
        sendEnvelope(
            [
                "v": 1,
                "type": "hello",
                "peer_id": peerID,
                "name": name
            ],
            excludingPeerID: excludingPeerID
        )
    }

    private func sendInventory(excludingPeerID: String?) {
        let records = Array(mailbagEvents().prefix(200))
        guard !records.isEmpty else { return }
        let events = records.map {
            [
                "id": $0.id,
                "kind": Int($0.kind),
                "created_at": NSNumber(value: $0.createdAtSecs),
                "size": $0.eventJson.utf8.count
            ] as [String: Any]
        }
        sendEnvelope(
            [
                "v": 1,
                "type": "inv",
                "peer_id": peerID,
                "events": events
            ],
            excludingPeerID: excludingPeerID
        )
    }

    private func sendWant(_ ids: [String], excludingPeerID: String?) {
        guard !ids.isEmpty else { return }
        sendEnvelope(
            [
                "v": 1,
                "type": "want",
                "peer_id": peerID,
                "ids": ids
            ],
            excludingPeerID: excludingPeerID
        )
    }

    private func sendEvent(_ record: IrisNearbyStoredEvent, excludingPeerID: String?) {
        let envelope: [String: Any] = [
            "v": 1,
            "type": "event",
            "peer_id": peerID,
            "event_json": record.eventJson
        ]
        if let frame = IrisNearbyFrame.encode(envelope), frame.count <= Self.singleFrameBytes {
            sendFrame(frame, excludingPeerID: excludingPeerID)
        } else {
            sendEventFragments(record, excludingPeerID: excludingPeerID)
        }
    }

    private func sendEventFragments(_ record: IrisNearbyStoredEvent, excludingPeerID: String?) {
        let bytes = Array(record.eventJson.utf8)
        let total = (bytes.count + Self.fragmentPayloadBytes - 1) / Self.fragmentPayloadBytes
        guard total > 1, total <= Self.maxEventFragments else { return }
        let fragmentID = UUID().uuidString.lowercased()
        for index in 0..<total {
            let start = index * Self.fragmentPayloadBytes
            let end = min(start + Self.fragmentPayloadBytes, bytes.count)
            let chunk = Data(bytes[start..<end]).base64EncodedString()
            sendEnvelope(
                [
                    "v": 1,
                    "type": "event_frag",
                    "peer_id": peerID,
                    "frag_id": fragmentID,
                    "event_id": record.id,
                    "index": index,
                    "total": total,
                    "data": chunk
                ],
                excludingPeerID: excludingPeerID
            )
        }
    }

    private func sendEnvelope(_ object: [String: Any], excludingPeerID: String?) {
        guard isVisible, let frame = IrisNearbyFrame.encode(object) else { return }
        sendFrame(frame, excludingPeerID: excludingPeerID)
    }

    private func sendFrame(_ frame: Data, excludingPeerID: String?) {
        guard isVisible else { return }
        for (id, characteristic) in writableCharacteristics {
            if let remotePeerID = peerIDByPeripheral[id], remotePeerID == excludingPeerID {
                continue
            }
            guard let peripheral = peripherals[id] else { continue }
            write(frame, to: peripheral, characteristic: characteristic)
        }
        for (id, channel) in subscribedCentrals {
            if let remotePeerID = peerIDByCentral[id], remotePeerID == excludingPeerID {
                continue
            }
            notify(frame, to: channel)
        }
    }

    private func write(_ data: Data, to peripheral: CBPeripheral, characteristic: CBCharacteristic) {
        let writeType: CBCharacteristicWriteType =
            characteristic.properties.contains(.write) ? .withResponse : .withoutResponse
        let maxLength = max(20, peripheral.maximumWriteValueLength(for: writeType))
        var offset = 0
        while offset < data.count {
            let end = min(offset + maxLength, data.count)
            peripheral.writeValue(Data(data[offset..<end]), for: characteristic, type: writeType)
            offset = end
        }
    }

    private func notify(_ data: Data, to channel: MacIrisNearbyCentralChannel?) {
        guard let peripheralManager else { return }
        guard let characteristic = channel?.characteristic ?? localCharacteristic else { return }
        let maxLength = max(20, channel?.central.maximumUpdateValueLength ?? 180)
        var offset = 0
        while offset < data.count {
            let end = min(offset + maxLength, data.count)
            let sent = peripheralManager.updateValue(
                Data(data[offset..<end]),
                for: characteristic,
                onSubscribedCentrals: channel.map { [$0.central] }
            )
            if !sent {
                pendingNotifications.append((Data(data[offset..<data.count]), channel))
                return
            }
            offset = end
        }
    }

    private func ingestFrame(_ frame: Data, source: IrisNearbySource) {
        guard let envelope = IrisNearbyFrame.decode(frame),
              let type = envelope["type"] as? String else { return }
        let remotePeerID = (envelope["peer_id"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        if remotePeerID == peerID {
            return
        }

        switch type {
        case "hello":
            guard let remotePeerID, !remotePeerID.isEmpty else { return }
            rememberPeer(remotePeerID, name: envelope["name"] as? String, source: source)
            sendInventory(excludingPeerID: nil)
        case "inv":
            handleInventory(envelope)
        case "want":
            handleWant(envelope)
        case "event":
            handleEventEnvelope(envelope, remotePeerID: remotePeerID)
        case "event_frag":
            handleEventFragment(envelope, remotePeerID: remotePeerID)
        default:
            break
        }
    }

    private func handleInventory(_ envelope: [String: Any]) {
        guard let events = envelope["events"] as? [[String: Any]] else { return }
        let wanted = events.compactMap { item -> String? in
            guard let id = item["id"] as? String,
                  id.count == 64,
                  ownOutbound[id] == nil,
                  forwarded[id] == nil else { return nil }
            let size = item["size"] as? Int ?? 0
            guard size > 0, size <= Self.maxEventBytes else { return nil }
            return id
        }
        sendWant(Array(wanted.prefix(64)), excludingPeerID: nil)
    }

    private func handleWant(_ envelope: [String: Any]) {
        guard let ids = envelope["ids"] as? [String] else { return }
        for id in ids.prefix(64) {
            if let record = ownOutbound[id] ?? forwarded[id] {
                sendEvent(record, excludingPeerID: nil)
            }
        }
    }

    private func handleEventEnvelope(_ envelope: [String: Any], remotePeerID: String?) {
        guard let eventJson = envelope["event_json"] as? String else { return }
        handleEventJson(eventJson, remotePeerID: remotePeerID)
    }

    private func handleEventFragment(_ envelope: [String: Any], remotePeerID: String?) {
        pruneIncomingFragments()
        guard let fragmentID = envelope["frag_id"] as? String,
              let dataString = envelope["data"] as? String,
              let data = Data(base64Encoded: dataString) else { return }
        let index = (envelope["index"] as? NSNumber)?.intValue ?? -1
        let total = (envelope["total"] as? NSNumber)?.intValue ?? -1
        guard !fragmentID.isEmpty,
              index >= 0,
              index < total,
              total > 0,
              total <= Self.maxEventFragments,
              !data.isEmpty else { return }

        var fragment = incomingFragments[fragmentID] ?? IrisNearbyIncomingFragment(
            total: total,
            parts: [:],
            storedAt: Date(),
            remotePeerID: remotePeerID
        )
        guard fragment.total == total else {
            incomingFragments.removeValue(forKey: fragmentID)
            return
        }
        fragment.parts[index] = data
        incomingFragments[fragmentID] = fragment
        guard fragment.parts.count == total else { return }

        var eventData = Data()
        for partIndex in 0..<total {
            guard let part = fragment.parts[partIndex] else { return }
            eventData.append(part)
            if eventData.count > Self.maxEventBytes {
                incomingFragments.removeValue(forKey: fragmentID)
                return
            }
        }
        incomingFragments.removeValue(forKey: fragmentID)
        guard let eventJson = String(data: eventData, encoding: .utf8) else { return }
        handleEventJson(eventJson, remotePeerID: remotePeerID ?? fragment.remotePeerID)
    }

    private func handleEventJson(_ eventJson: String, remotePeerID: String?) {
        guard eventJson.utf8.count <= Self.maxEventBytes,
              let record = IrisNearbyStoredEvent.fromEventJson(eventJson) else { return }
        if ownOutbound[record.id] != nil || forwarded[record.id] != nil {
            return
        }
        let accepted = ingestEventJson?(eventJson) ?? false
        guard accepted else { return }
        forwarded[record.id] = record
        pruneMailbags()
        sendInventory(excludingPeerID: remotePeerID)
        NSLog("Iris nearby: accepted event kind %u %@", record.kind, record.id)
    }

    private func pruneIncomingFragments() {
        let cutoff = Date().addingTimeInterval(-Self.fragmentTTL)
        incomingFragments = incomingFragments.filter { $0.value.storedAt >= cutoff }
        while incomingFragments.count > Self.maxIncomingFragmentSets,
              let oldest = incomingFragments.min(by: { $0.value.storedAt < $1.value.storedAt })?.key {
            incomingFragments.removeValue(forKey: oldest)
        }
    }

    private func rememberPeer(_ peerID: String, name: String?, source: IrisNearbySource) {
        switch source {
        case .peripheral(let peripheral):
            peerIDByPeripheral[peripheral.identifier] = peerID
        case .central(let central):
            peerIDByCentral[central.identifier] = peerID
        }
        let peer = MacNearbyIrisPeer(
            id: peerID,
            name: name?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false ? name! : "Iris",
            lastSeen: Date()
        )
        if let index = peers.firstIndex(where: { $0.id == peerID }) {
            peers[index] = peer
        } else {
            peers.append(peer)
            peers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        }
        status = sidebarSubtitle
    }

    private func mailbagEvents() -> [IrisNearbyStoredEvent] {
        (Array(ownOutbound.values) + Array(forwarded.values))
            .sorted { $0.createdAtSecs > $1.createdAtSecs }
    }

    private func pruneMailbags() {
        prune(&ownOutbound)
        prune(&forwarded)
    }

    private func prune(_ bag: inout [String: IrisNearbyStoredEvent]) {
        guard bag.count > Self.maxMailbagEvents else { return }
        let keep = Set(
            bag.values
                .sorted { $0.createdAtSecs > $1.createdAtSecs }
                .prefix(Self.maxMailbagEvents)
                .map(\.id)
        )
        bag = bag.filter { keep.contains($0.key) }
    }

    private func bluetoothStatus(_ state: CBManagerState) -> String {
        switch state {
        case .poweredOff: return "Bluetooth off"
        case .unauthorized: return "No Bluetooth access"
        case .unsupported: return "Bluetooth unavailable"
        case .resetting: return "Bluetooth reset"
        case .unknown: return "Bluetooth"
        case .poweredOn: return peers.isEmpty ? "Visible" : sidebarSubtitle
        @unknown default: return "Bluetooth"
        }
    }
}

extension MacIrisNearbyService: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn {
            startScanningIfReady()
        } else {
            status = bluetoothStatus(central.state)
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi RSSI: NSNumber
    ) {
        guard isVisible, peripherals[peripheral.identifier] == nil else { return }
        peripherals[peripheral.identifier] = peripheral
        peripheral.delegate = self
        status = "Connecting"
        central.connect(peripheral)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        status = "Connected"
        peripheral.delegate = self
        peripheral.discoverServices([Self.serviceUUID])
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        peripherals.removeValue(forKey: peripheral.identifier)
        writableCharacteristics.removeValue(forKey: peripheral.identifier)
        peripheralAssemblers.removeValue(forKey: peripheral.identifier)
        if let peerID = peerIDByPeripheral.removeValue(forKey: peripheral.identifier) {
            peers.removeAll { $0.id == peerID }
        }
        if isVisible {
            status = peers.isEmpty ? "Scanning" : sidebarSubtitle
            startScanningIfReady()
        }
    }
}

extension MacIrisNearbyService: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard error == nil, let services = peripheral.services else { return }
        for service in services where service.uuid == Self.serviceUUID {
            peripheral.discoverCharacteristics([Self.characteristicUUID], for: service)
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard error == nil,
              let characteristic = service.characteristics?.first(where: { $0.uuid == Self.characteristicUUID }) else {
            return
        }
        writableCharacteristics[peripheral.identifier] = characteristic
        peripheralAssemblers[peripheral.identifier] = IrisNearbyFrameAssembler()
        if characteristic.properties.contains(.notify) {
            peripheral.setNotifyValue(true, for: characteristic)
        }
        sendHello(excludingPeerID: nil)
        sendInventory(excludingPeerID: nil)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard error == nil, let value = characteristic.value else { return }
        var assembler = peripheralAssemblers[peripheral.identifier] ?? IrisNearbyFrameAssembler()
        let frames = assembler.append(value)
        peripheralAssemblers[peripheral.identifier] = assembler
        for frame in frames {
            ingestFrame(frame, source: .peripheral(peripheral))
        }
    }
}

extension MacIrisNearbyService: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        if peripheral.state == .poweredOn {
            startAdvertisingIfReady()
        } else {
            status = bluetoothStatus(peripheral.state)
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService, error: Error?) {
        guard error == nil else {
            status = "Bluetooth failed"
            return
        }
        startAdvertisingIfReady()
    }

    func peripheralManagerDidStartAdvertising(_ peripheral: CBPeripheralManager, error: Error?) {
        status = error == nil ? (peers.isEmpty ? "Visible" : sidebarSubtitle) : "Advertise failed"
    }

    func peripheralManager(
        _ peripheral: CBPeripheralManager,
        central: CBCentral,
        didSubscribeTo characteristic: CBCharacteristic
    ) {
        guard let mutableCharacteristic = characteristic as? CBMutableCharacteristic else { return }
        let channel = MacIrisNearbyCentralChannel(central: central, characteristic: mutableCharacteristic)
        subscribedCentrals[central.identifier] = channel
        centralAssemblers[central.identifier] = IrisNearbyFrameAssembler()
        sendHello(excludingPeerID: nil)
        sendInventory(excludingPeerID: nil)
    }

    func peripheralManager(
        _ peripheral: CBPeripheralManager,
        central: CBCentral,
        didUnsubscribeFrom characteristic: CBCharacteristic
    ) {
        subscribedCentrals.removeValue(forKey: central.identifier)
        centralAssemblers.removeValue(forKey: central.identifier)
        if let peerID = peerIDByCentral.removeValue(forKey: central.identifier) {
            peers.removeAll { $0.id == peerID }
        }
        status = peers.isEmpty ? "Visible" : sidebarSubtitle
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for request in requests {
            guard request.characteristic.uuid == Self.characteristicUUID,
                  let value = request.value else {
                peripheral.respond(to: request, withResult: .requestNotSupported)
                continue
            }
            let central = request.central
            var assembler = centralAssemblers[central.identifier] ?? IrisNearbyFrameAssembler()
            let frames = assembler.append(value)
            centralAssemblers[central.identifier] = assembler
            for frame in frames {
                ingestFrame(frame, source: .central(central))
            }
            peripheral.respond(to: request, withResult: .success)
        }
    }

    func peripheralManagerIsReady(toUpdateSubscribers peripheral: CBPeripheralManager) {
        let pending = pendingNotifications
        pendingNotifications.removeAll()
        for item in pending {
            notify(item.data, to: item.channel)
        }
    }
}

private struct MacIrisNearbyCentralChannel {
    let central: CBCentral
    let characteristic: CBMutableCharacteristic
}

private enum IrisNearbySource {
    case peripheral(CBPeripheral)
    case central(CBCentral)
}

private struct IrisNearbyStoredEvent {
    let id: String
    let kind: UInt32
    let createdAtSecs: UInt64
    let eventJson: String
    let storedAt: Date

    static func fromEventJson(_ eventJson: String) -> IrisNearbyStoredEvent? {
        guard let data = eventJson.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = object["id"] as? String else { return nil }
        let kind = (object["kind"] as? NSNumber)?.uint32Value ?? 0
        let createdAt = (object["created_at"] as? NSNumber)?.uint64Value ?? 0
        return IrisNearbyStoredEvent(
            id: id,
            kind: kind,
            createdAtSecs: createdAt,
            eventJson: eventJson,
            storedAt: Date()
        )
    }
}

private struct IrisNearbyIncomingFragment {
    let total: Int
    var parts: [Int: Data]
    let storedAt: Date
    let remotePeerID: String?
}

private enum IrisNearbyFrame {
    static let magic = Data([0x49, 0x52, 0x49, 0x53])
    private static let compressedFlag: UInt8 = 0x01
    private static let headerSize = 13
    private static let compressionThreshold = 100

    static func encode(_ object: [String: Any]) -> Data? {
        guard JSONSerialization.isValidJSONObject(object),
              let payload = try? JSONSerialization.data(withJSONObject: object),
              payload.count <= MacIrisNearbyService.maxFrameBytes else { return nil }
        let compressed = compressIfBeneficial(payload)
        let body = compressed ?? payload
        let flags: UInt8 = compressed == nil ? 0 : compressedFlag
        guard body.count <= MacIrisNearbyService.maxFrameBytes else { return nil }
        var data = Data()
        data.append(magic)
        data.append(flags)
        data.append(contentsOf: UInt32(body.count).bigEndianBytes)
        data.append(contentsOf: UInt32(payload.count).bigEndianBytes)
        data.append(body)
        return data
    }

    static func decode(_ frame: Data) -> [String: Any]? {
        guard frame.count >= headerSize, Data(frame.prefix(4)) == magic else { return nil }
        let header = Array(frame.prefix(headerSize))
        let flags = header[4]
        let payload = Data(frame.dropFirst(headerSize))
        let originalSize = Int(UInt32(bigEndianBytes: header[9..<13]))
        if (flags & compressedFlag) != 0 {
            guard let decompressed = decompress(payload, originalSize: originalSize) else {
                return nil
            }
            return try? JSONSerialization.jsonObject(with: decompressed) as? [String: Any]
        }
        return try? JSONSerialization.jsonObject(with: payload) as? [String: Any]
    }

    private static func compressIfBeneficial(_ data: Data) -> Data? {
        guard data.count >= compressionThreshold else { return nil }
        let destinationSize = data.count + 64
        let destination = UnsafeMutablePointer<UInt8>.allocate(capacity: destinationSize)
        defer { destination.deallocate() }
        let compressedSize = data.withUnsafeBytes { sourceBuffer -> Int in
            guard let source = sourceBuffer.bindMemory(to: UInt8.self).baseAddress else { return 0 }
            return compression_encode_buffer(
                destination,
                destinationSize,
                source,
                data.count,
                nil,
                COMPRESSION_ZLIB
            )
        }
        guard compressedSize > 0, compressedSize < data.count else { return nil }
        return Data(bytes: destination, count: compressedSize)
    }

    private static func decompress(_ data: Data, originalSize: Int) -> Data? {
        guard originalSize > 0, originalSize <= MacIrisNearbyService.maxFrameBytes else { return nil }
        let destination = UnsafeMutablePointer<UInt8>.allocate(capacity: originalSize)
        defer { destination.deallocate() }
        let decodedSize = data.withUnsafeBytes { sourceBuffer -> Int in
            guard let source = sourceBuffer.bindMemory(to: UInt8.self).baseAddress else { return 0 }
            return compression_decode_buffer(
                destination,
                originalSize,
                source,
                data.count,
                nil,
                COMPRESSION_ZLIB
            )
        }
        guard decodedSize == originalSize else { return nil }
        return Data(bytes: destination, count: decodedSize)
    }
}

private struct IrisNearbyFrameAssembler {
    private var buffer = Data()

    mutating func append(_ chunk: Data) -> [Data] {
        buffer.append(chunk)
        var frames: [Data] = []
        while buffer.count >= 13 {
            if Data(buffer.prefix(4)) != IrisNearbyFrame.magic {
                buffer.removeFirst()
                continue
            }
            let header = Array(buffer.prefix(13))
            let length = Int(UInt32(bigEndianBytes: header[5..<9]))
            if length <= 0 || length > MacIrisNearbyService.maxFrameBytes {
                buffer.removeFirst()
                continue
            }
            let frameLength = 13 + length
            guard buffer.count >= frameLength else { break }
            frames.append(Data(buffer.prefix(frameLength)))
            buffer.removeFirst(frameLength)
        }
        return frames
    }
}

private extension FixedWidthInteger {
    var bigEndianBytes: [UInt8] {
        withUnsafeBytes(of: self.bigEndian) { Array($0) }
    }
}

private extension UInt32 {
    init<S: Sequence>(bigEndianBytes bytes: S) where S.Element == UInt8 {
        self = bytes.reduce(0) { ($0 << 8) | UInt32($1) }
    }
}
