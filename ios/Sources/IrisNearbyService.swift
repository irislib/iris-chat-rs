import Combine
import CoreBluetooth
import Foundation
#if os(iOS)
import UIKit
#endif

struct IrisNearbyPeer: Identifiable, Equatable {
    let id: String
    var name: String
    var ownerPubkeyHex: String?
    var pictureURL: String?
    var profileEventID: String?
    var lastSeen: Date
}

final class IrisNearbyService: NSObject, ObservableObject {
    @Published private(set) var isVisible = false
    @Published private(set) var status = "Off"
    @Published private(set) var peers: [IrisNearbyPeer] = []

    private static let serviceUUID = CBUUID(string: "8A0DAE01-D8E5-4F27-9F20-A616F1FBA6D0")
    private static let characteristicUUID = CBUUID(string: "8A0DAE02-D8E5-4F27-9F20-A616F1FBA6D0")
    fileprivate static let singleFrameBytes = 16 * 1024
    fileprivate static let fragmentPayloadBytes = 4 * 1024
    fileprivate static let maxEventFragments = 1024
    fileprivate static let maxIncomingFragmentSets = 64
    fileprivate static let fragmentTTL: TimeInterval = 30
    fileprivate static let maxEventBytes = 128 * 1024
    fileprivate static let maxMailbagEvents = 500
    private static let nearbyPresenceKind: UInt32 = 22242
    private static let nonIrisBackoff: TimeInterval = 60
    private static let maxSimultaneousPeripherals = 4

    private let peerID = UUID().uuidString.lowercased()
    private var centralManager: CBCentralManager?
    private var peripheralManager: CBPeripheralManager?
    private var localCharacteristic: CBMutableCharacteristic?
    private var peripherals: [UUID: CBPeripheral] = [:]
    private var writableCharacteristics: [UUID: CBCharacteristic] = [:]
    private var peripheralAssemblers: [UUID: IrisNearbyFrameAssembler] = [:]
    private var peripheralWriteQueues: [UUID: IrisNearbyPeripheralWriteQueue] = [:]
    private var subscribedCentrals: [UUID: IrisNearbyCentralChannel] = [:]
    private var centralAssemblers: [UUID: IrisNearbyFrameAssembler] = [:]
    private var pendingNotifications: [(data: Data, channel: IrisNearbyCentralChannel?)] = []
    private var peerIDByPeripheral: [UUID: String] = [:]
    private var peerIDByCentral: [UUID: String] = [:]
    private var peerNonces: [String: String] = [:]
    private var ignoredPeripherals: [UUID: Date] = [:]
    private var ownOutbound: [String: IrisNearbyStoredEvent] = [:]
    private var forwarded: [String: IrisNearbyStoredEvent] = [:]
    private var knownProfiles: [String: IrisNearbyProfileEvent] = [:]
    private var incomingFragments: [String: IrisNearbyIncomingFragment] = [:]
    private var localNonce = UUID().uuidString.lowercased()
    private var ownProfileEventID: String?
    private var lastCentralStateLog: String?
    private var lastPeripheralStateLog: String?

    var ingestEventJson: ((String) -> Bool)?
    var buildPresenceEventJson: ((String, String, String, String?) -> String)?
    var verifyPresenceEventJson: ((String, String, String, String) -> String)?
    var encodeFrameJson: ((String) -> Data?)?
    var decodeFrame: ((Data) -> String)?
    var frameBodyLength: ((Data) -> Int)?

    var sidebarSubtitle: String {
        if !isVisible {
            return "Click to enable"
        }
        if !peers.isEmpty {
            return Self.nearbySummary(for: peers)
        }
        if Self.isBlockingStatus(status) { return status }
        return "No users nearby"
    }

    private static func nearbySummary(for peers: [IrisNearbyPeer]) -> String {
        let names = peers.map(summaryName)
        switch names.count {
        case 1:
            return "\(names[0]) nearby"
        case 2:
            return "\(names[0]) and \(names[1]) nearby"
        case 3:
            return "\(names[0]), \(names[1]) and \(names[2]) nearby"
        default:
            let shown = names.prefix(3).joined(separator: ", ")
            let otherCount = names.count - 3
            let suffix = otherCount == 1 ? "other" : "others"
            return "\(shown) and \(otherCount) \(suffix) nearby"
        }
    }

    private static func summaryName(for peer: IrisNearbyPeer) -> String {
        let trimmed = peer.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Someone" : trimmed
    }

    private static func isBlockingStatus(_ status: String) -> Bool {
        switch status {
        case "No Bluetooth access", "Bluetooth off", "Bluetooth unavailable", "Bluetooth failed", "Bluetooth reset", "Advertise failed":
            return true
        default:
            return false
        }
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
            localNonce = UUID().uuidString.lowercased()
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
        if kind == 0, let profile = IrisNearbyProfileEvent.fromEventJson(eventJson) {
            ownProfileEventID = eventID
            knownProfiles[eventID] = profile
        }
        pruneMailbags()
        guard isVisible else { return }
        if kind == 0 {
            sendHello(excludingPeerID: nil)
        }
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
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            guard let self, self.isVisible else { return }
            self.logBluetoothStates()
            self.startScanningIfReady()
            self.startAdvertisingIfReady()
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
        peripheralWriteQueues.removeAll()
        subscribedCentrals.removeAll()
        centralAssemblers.removeAll()
        pendingNotifications.removeAll()
        peerIDByPeripheral.removeAll()
        peerIDByCentral.removeAll()
        peerNonces.removeAll()
        ignoredPeripherals.removeAll()
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

    private func shouldConnect(to peripheralID: UUID, advertisementData: [String: Any]) -> Bool {
        if let ignoredUntil = ignoredPeripherals[peripheralID] {
            if ignoredUntil > Date() {
                return false
            }
            ignoredPeripherals.removeValue(forKey: peripheralID)
        }
        guard peripherals.count < Self.maxSimultaneousPeripherals else {
            return false
        }
        guard let serviceUUIDs = advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID],
              !serviceUUIDs.isEmpty else {
            return true
        }
        return serviceUUIDs.contains(Self.serviceUUID)
    }

    private func rejectNonIrisPeripheral(_ peripheral: CBPeripheral, reason: String) {
        NSLog("Iris nearby: \(reason)")
        ignoredPeripherals[peripheral.identifier] = Date().addingTimeInterval(Self.nonIrisBackoff)
        peripherals.removeValue(forKey: peripheral.identifier)
        writableCharacteristics.removeValue(forKey: peripheral.identifier)
        peripheralAssemblers.removeValue(forKey: peripheral.identifier)
        peripheralWriteQueues.removeValue(forKey: peripheral.identifier)
        if let peerID = peerIDByPeripheral.removeValue(forKey: peripheral.identifier) {
            peers.removeAll { $0.id == peerID }
            peerNonces.removeValue(forKey: peerID)
        }
        centralManager?.cancelPeripheralConnection(peripheral)
        if isVisible {
            status = peers.isEmpty ? "Scanning" : sidebarSubtitle
        }
    }

    private func sendHello(excludingPeerID: String?) {
        let envelope: [String: Any] = [
            "v": 1,
            "type": "hello",
            "peer_id": peerID,
            "nonce": localNonce,
            "name": localDeviceName
        ]
        sendEnvelope(envelope, excludingPeerID: excludingPeerID)
    }

    private var localDeviceName: String {
#if os(iOS)
        let name = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? "Iris" : name
#elseif os(macOS)
        let name = Host.current().localizedName?.trimmingCharacters(in: .whitespacesAndNewlines)
        return name?.isEmpty == false ? name! : "Iris"
#else
        return "Iris"
#endif
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
        sendEventJson(record.eventJson, excludingPeerID: excludingPeerID)
    }

    private func sendEventJson(_ eventJson: String, excludingPeerID: String?) {
        let envelope: [String: Any] = [
            "v": 1,
            "type": "event",
            "peer_id": peerID,
            "event_json": eventJson
        ]
        if let frame = encodeFrame(envelope), frame.count <= Self.singleFrameBytes {
            sendFrame(frame, excludingPeerID: excludingPeerID)
        } else {
            if let record = IrisNearbyStoredEvent.fromEventJson(eventJson) {
                sendEventFragments(record, excludingPeerID: excludingPeerID)
            }
        }
    }

    private func sendPresence(remotePeerID: String, remoteNonce: String) {
        let eventJson = buildPresenceEventJson?(
            peerID,
            localNonce,
            remoteNonce,
            ownProfileEventID
        ) ?? ""
        guard !eventJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        sendEventJson(eventJson, excludingPeerID: nil)
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
        guard isVisible, let frame = encodeFrame(object) else { return }
        sendFrame(frame, excludingPeerID: excludingPeerID)
    }

    private func encodeFrame(_ object: [String: Any]) -> Data? {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object),
              let json = String(data: data, encoding: .utf8) else { return nil }
        return encodeFrameJson?(json)
    }

    private func decodeFrameJson(_ frame: Data) -> [String: Any]? {
        guard let json = decodeFrame?(frame),
              !json.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let data = json.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    private func newFrameAssembler() -> IrisNearbyFrameAssembler {
        IrisNearbyFrameAssembler { [weak self] header in
            self?.frameBodyLength?(header) ?? -1
        }
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
        let writeType: CBCharacteristicWriteType
        if characteristic.properties.contains(.writeWithoutResponse) {
            writeType = .withoutResponse
        } else if characteristic.properties.contains(.write) {
            writeType = .withResponse
        } else {
            return
        }

        let maxLength = max(20, peripheral.maximumWriteValueLength(for: writeType))
        var queue = peripheralWriteQueues[peripheral.identifier] ?? IrisNearbyPeripheralWriteQueue()
        if queue.chunks.isEmpty {
            queue.writeType = writeType
        }
        var offset = 0
        while offset < data.count {
            let end = min(offset + maxLength, data.count)
            queue.chunks.append(Data(data[offset..<end]))
            offset = end
        }
        peripheralWriteQueues[peripheral.identifier] = queue
        flushWriteQueue(for: peripheral.identifier)
    }

    private func flushWriteQueue(for peripheralID: UUID) {
        guard var queue = peripheralWriteQueues[peripheralID],
              let peripheral = peripherals[peripheralID],
              let characteristic = writableCharacteristics[peripheralID] else { return }

        switch queue.writeType {
        case .withResponse:
            if queue.chunks.isEmpty {
                peripheralWriteQueues.removeValue(forKey: peripheralID)
                return
            }
            guard !queue.waitingForResponse else {
                peripheralWriteQueues[peripheralID] = queue
                return
            }
            let chunk = queue.chunks.removeFirst()
            queue.waitingForResponse = true
            peripheralWriteQueues[peripheralID] = queue
            peripheral.writeValue(chunk, for: characteristic, type: .withResponse)
        case .withoutResponse:
            while !queue.chunks.isEmpty && peripheral.canSendWriteWithoutResponse {
                let chunk = queue.chunks.removeFirst()
                peripheral.writeValue(chunk, for: characteristic, type: .withoutResponse)
            }
            if queue.chunks.isEmpty {
                peripheralWriteQueues.removeValue(forKey: peripheralID)
            } else {
                peripheralWriteQueues[peripheralID] = queue
            }
        @unknown default:
            peripheralWriteQueues.removeValue(forKey: peripheralID)
        }
    }

    private func notify(_ data: Data, to channel: IrisNearbyCentralChannel?) {
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
        guard let envelope = decodeFrameJson(frame),
              let type = envelope["type"] as? String else { return }
        let remotePeerID = (envelope["peer_id"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        if remotePeerID == peerID {
            return
        }

        switch type {
        case "hello":
            guard let remotePeerID, !remotePeerID.isEmpty else { return }
            let remoteNonce = sanitizedNonce(envelope["nonce"] as? String)
            rememberPeer(
                remotePeerID,
                name: envelope["name"] as? String,
                profileEventID: nil,
                source: source
            )
            if let remoteNonce {
                peerNonces[remotePeerID] = remoteNonce
                sendPresence(remotePeerID: remotePeerID, remoteNonce: remoteNonce)
            }
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
        if record.kind == Self.nearbyPresenceKind {
            if handlePresenceEvent(eventJson, remotePeerID: remotePeerID) {
                NSLog("Iris nearby: accepted presence")
            }
            return
        }
        if let existing = ownOutbound[record.id] ?? forwarded[record.id] {
            rememberProfile(from: existing.eventJson, remotePeerID: remotePeerID)
            return
        }
        let accepted = ingestEventJson?(eventJson) ?? false
        guard accepted else { return }
        rememberProfile(from: eventJson, remotePeerID: remotePeerID)
        forwarded[record.id] = record
        pruneMailbags()
        sendInventory(excludingPeerID: remotePeerID)
        NSLog("Iris nearby: accepted event kind %u %@", record.kind, record.id)
    }

    private func handlePresenceEvent(_ eventJson: String, remotePeerID: String?) -> Bool {
        guard let remotePeerID, !remotePeerID.isEmpty,
              let remoteNonce = peerNonces[remotePeerID] else { return false }
        let result = verifyPresenceEventJson?(
            eventJson,
            remotePeerID,
            localNonce,
            remoteNonce
        ) ?? ""
        guard let data = result.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let ownerPubkeyHex = object["owner_pubkey_hex"] as? String,
              ownerPubkeyHex.count == 64 else {
            return false
        }
        rememberPresence(
            peerID: remotePeerID,
            ownerPubkeyHex: ownerPubkeyHex,
            profileEventID: sanitizedEventID(object["profile_event_id"] as? String)
        )
        return true
    }

    private func pruneIncomingFragments() {
        let cutoff = Date().addingTimeInterval(-Self.fragmentTTL)
        incomingFragments = incomingFragments.filter { $0.value.storedAt >= cutoff }
        while incomingFragments.count > Self.maxIncomingFragmentSets,
              let oldest = incomingFragments.min(by: { $0.value.storedAt < $1.value.storedAt })?.key {
            incomingFragments.removeValue(forKey: oldest)
        }
    }

    private func rememberPeer(
        _ peerID: String,
        name: String?,
        profileEventID: String?,
        source: IrisNearbySource
    ) {
        switch source {
        case .peripheral(let peripheral):
            peerIDByPeripheral[peripheral.identifier] = peerID
        case .central(let central):
            peerIDByCentral[central.identifier] = peerID
        }
        let sanitizedProfileEventID = sanitizedEventID(profileEventID)
        let existing = peers.first(where: { $0.id == peerID })
        let peer = IrisNearbyPeer(
            id: peerID,
            name: sanitizedName(name) ?? existing?.name ?? "Iris",
            ownerPubkeyHex: existing?.ownerPubkeyHex,
            pictureURL: existing?.pictureURL,
            profileEventID: sanitizedProfileEventID ?? existing?.profileEventID,
            lastSeen: Date()
        )
        if let index = peers.firstIndex(where: { $0.id == peerID }) {
            peers[index] = peer
        } else {
            peers.append(peer)
        }
        sortPeers()
        if let profileEventID = sanitizedProfileEventID ?? existing?.profileEventID,
           let profile = knownProfiles[profileEventID] {
            applyAdvertisedProfile(profile, toPeerID: peerID)
        }
        status = sidebarSubtitle
        NSLog("Iris nearby: saw peer")
    }

    private func mailbagEvents() -> [IrisNearbyStoredEvent] {
        var records = (Array(ownOutbound.values) + Array(forwarded.values))
            .sorted { $0.createdAtSecs > $1.createdAtSecs }
        if let ownProfileEventID, let profile = ownOutbound[ownProfileEventID] {
            records.removeAll { $0.id == profile.id }
            records.insert(profile, at: 0)
        }
        return records
    }

    private func pruneMailbags() {
        prune(&ownOutbound, preserving: ownProfileEventID)
        prune(&forwarded, preserving: nil)
        pruneKnownProfiles()
    }

    private func prune(_ bag: inout [String: IrisNearbyStoredEvent], preserving protectedID: String?) {
        guard bag.count > Self.maxMailbagEvents else { return }
        let keep = Set(
            bag.values
                .sorted { $0.createdAtSecs > $1.createdAtSecs }
                .prefix(Self.maxMailbagEvents)
                .map(\.id)
        )
        bag = bag.filter { keep.contains($0.key) || $0.key == protectedID }
    }

    private func pruneKnownProfiles() {
        var keep = Set(ownOutbound.keys)
        keep.formUnion(forwarded.keys)
        if let ownProfileEventID {
            keep.insert(ownProfileEventID)
        }
        for peer in peers {
            if let profileEventID = peer.profileEventID {
                keep.insert(profileEventID)
            }
        }
        knownProfiles = knownProfiles.filter { keep.contains($0.key) }
    }

    private func rememberProfile(from eventJson: String, remotePeerID: String?) {
        guard let profile = IrisNearbyProfileEvent.fromEventJson(eventJson) else { return }
        knownProfiles[profile.id] = profile
        if let remotePeerID {
            if !peers.contains(where: { $0.id == remotePeerID }) {
                peers.append(
                    IrisNearbyPeer(
                        id: remotePeerID,
                        name: profile.displayName ?? "Iris",
                        ownerPubkeyHex: profile.ownerPubkeyHex,
                        pictureURL: profile.pictureURL,
                        profileEventID: profile.id,
                        lastSeen: Date()
                    )
                )
                sortPeers()
                status = sidebarSubtitle
            }
            applyAdvertisedProfile(profile, toPeerID: remotePeerID)
        }
    }

    private func rememberPresence(peerID: String, ownerPubkeyHex: String, profileEventID: String?) {
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        let nextProfileEventID = profileEventID ?? peers[index].profileEventID
        peers[index].ownerPubkeyHex = ownerPubkeyHex
        peers[index].profileEventID = nextProfileEventID
        peers[index].lastSeen = Date()
        if let nextProfileEventID, let profile = knownProfiles[nextProfileEventID] {
            applyAdvertisedProfile(profile, toPeerID: peerID)
        } else if let nextProfileEventID,
                  ownOutbound[nextProfileEventID] == nil,
                  forwarded[nextProfileEventID] == nil {
            sendWant([nextProfileEventID], excludingPeerID: nil)
        }
        sortPeers()
        status = sidebarSubtitle
    }

    private func applyAdvertisedProfile(_ profile: IrisNearbyProfileEvent, toPeerID peerID: String) {
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        if let ownerPubkeyHex = peers[index].ownerPubkeyHex,
           ownerPubkeyHex.caseInsensitiveCompare(profile.ownerPubkeyHex) != .orderedSame {
            return
        }
        if let profileEventID = peers[index].profileEventID, profileEventID != profile.id {
            return
        }
        peers[index].ownerPubkeyHex = profile.ownerPubkeyHex
        peers[index].profileEventID = profile.id
        if let displayName = profile.displayName {
            peers[index].name = displayName
        }
        if let pictureURL = profile.pictureURL {
            peers[index].pictureURL = pictureURL
        }
        peers[index].lastSeen = Date()
        sortPeers()
        status = sidebarSubtitle
    }

    private func sortPeers() {
        peers.sort {
            let comparison = $0.name.localizedCaseInsensitiveCompare($1.name)
            if comparison == .orderedSame {
                return $0.id < $1.id
            }
            return comparison == .orderedAscending
        }
    }

    private func sanitizedName(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    private func sanitizedEventID(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.count == 64 ? trimmed : nil
    }

    private func sanitizedNonce(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return (16...128).contains(trimmed.count) ? trimmed : nil
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

    private func logBluetoothStates() {
        if let centralManager {
            logState(
                bluetoothDescription(centralManager.state),
                previous: &lastCentralStateLog,
                label: "central"
            )
        }
        if let peripheralManager {
            logState(
                bluetoothDescription(peripheralManager.state),
                previous: &lastPeripheralStateLog,
                label: "peripheral"
            )
        }
    }

    private func logState(_ state: String, previous: inout String?, label: String) {
        guard previous != state else { return }
        previous = state
        NSLog("Iris nearby: %@ state %@", label, state)
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
}

extension IrisNearbyService: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        logBluetoothStates()
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
        guard shouldConnect(to: peripheral.identifier, advertisementData: advertisementData) else { return }
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
        peripheralWriteQueues.removeValue(forKey: peripheral.identifier)
        if let peerID = peerIDByPeripheral.removeValue(forKey: peripheral.identifier) {
            peers.removeAll { $0.id == peerID }
            peerNonces.removeValue(forKey: peerID)
        }
        if isVisible {
            status = peers.isEmpty ? "Scanning" : sidebarSubtitle
            startScanningIfReady()
        }
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didWriteValueFor characteristic: CBCharacteristic,
        error: Error?
    ) {
        guard characteristic.uuid == Self.characteristicUUID else { return }
        guard var queue = peripheralWriteQueues[peripheral.identifier] else { return }
        if let error {
            NSLog("Iris nearby: write failed \(error.localizedDescription)")
            peripheralWriteQueues.removeValue(forKey: peripheral.identifier)
            return
        }
        queue.waitingForResponse = false
        peripheralWriteQueues[peripheral.identifier] = queue
        flushWriteQueue(for: peripheral.identifier)
    }

    func peripheralIsReady(toSendWriteWithoutResponse peripheral: CBPeripheral) {
        flushWriteQueue(for: peripheral.identifier)
    }
}

extension IrisNearbyService: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard error == nil, let services = peripheral.services else {
            rejectNonIrisPeripheral(peripheral, reason: "service discovery failed")
            return
        }
        var foundService = false
        for service in services where service.uuid == Self.serviceUUID {
            foundService = true
            peripheral.discoverCharacteristics([Self.characteristicUUID], for: service)
        }
        if !foundService {
            rejectNonIrisPeripheral(peripheral, reason: "missing Iris service")
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard error == nil,
              let characteristic = service.characteristics?.first(where: { $0.uuid == Self.characteristicUUID }) else {
            rejectNonIrisPeripheral(peripheral, reason: "missing Iris characteristic")
            return
        }
        writableCharacteristics[peripheral.identifier] = characteristic
        peripheralAssemblers[peripheral.identifier] = newFrameAssembler()
        if characteristic.properties.contains(.notify) {
            peripheral.setNotifyValue(true, for: characteristic)
        } else {
            sendHello(excludingPeerID: nil)
            sendInventory(excludingPeerID: nil)
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        guard characteristic.uuid == Self.characteristicUUID else { return }
        if let error {
            NSLog("Iris nearby: notification setup failed \(error.localizedDescription)")
        } else {
            NSLog("Iris nearby: notifications ready")
        }
        sendHello(excludingPeerID: nil)
        sendInventory(excludingPeerID: nil)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard error == nil, let value = characteristic.value else { return }
        var assembler = peripheralAssemblers[peripheral.identifier] ?? newFrameAssembler()
        let frames = assembler.append(value)
        peripheralAssemblers[peripheral.identifier] = assembler
        for frame in frames {
            ingestFrame(frame, source: .peripheral(peripheral))
        }
    }
}

extension IrisNearbyService: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        logBluetoothStates()
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
        let channel = IrisNearbyCentralChannel(central: central, characteristic: mutableCharacteristic)
        subscribedCentrals[central.identifier] = channel
        centralAssemblers[central.identifier] = newFrameAssembler()
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
            peerNonces.removeValue(forKey: peerID)
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
            var assembler = centralAssemblers[central.identifier] ?? newFrameAssembler()
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

private struct IrisNearbyCentralChannel {
    let central: CBCentral
    let characteristic: CBMutableCharacteristic
}

private struct IrisNearbyPeripheralWriteQueue {
    var chunks: [Data] = []
    var writeType: CBCharacteristicWriteType = .withResponse
    var waitingForResponse = false
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

private struct IrisNearbyProfileEvent {
    let id: String
    let ownerPubkeyHex: String
    let displayName: String?
    let pictureURL: String?

    static func fromEventJson(_ eventJson: String) -> IrisNearbyProfileEvent? {
        guard let data = eventJson.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = object["id"] as? String,
              id.count == 64,
              let kind = object["kind"] as? NSNumber,
              kind.uint32Value == 0,
              let ownerPubkeyHex = object["pubkey"] as? String,
              ownerPubkeyHex.count == 64,
              let content = object["content"] as? String,
              let contentData = content.data(using: .utf8),
              let metadata = try? JSONSerialization.jsonObject(with: contentData) as? [String: Any] else {
            return nil
        }
        return IrisNearbyProfileEvent(
            id: id,
            ownerPubkeyHex: ownerPubkeyHex,
            displayName: normalized(metadata["display_name"]) ?? normalized(metadata["name"]),
            pictureURL: normalized(metadata["picture"])
        )
    }

    private static func normalized(_ value: Any?) -> String? {
        guard let string = value as? String else { return nil }
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

private struct IrisNearbyIncomingFragment {
    let total: Int
    var parts: [Int: Data]
    let storedAt: Date
    let remotePeerID: String?
}

private struct IrisNearbyFrameAssembler {
    private static let headerSize = 13

    private let bodyLengthFromHeader: (Data) -> Int
    private var buffer = Data()

    init(bodyLengthFromHeader: @escaping (Data) -> Int) {
        self.bodyLengthFromHeader = bodyLengthFromHeader
    }

    mutating func append(_ chunk: Data) -> [Data] {
        buffer.append(chunk)
        var frames: [Data] = []
        while buffer.count >= Self.headerSize {
            let length = bodyLengthFromHeader(Data(buffer.prefix(Self.headerSize)))
            if length <= 0 {
                buffer.removeFirst()
                continue
            }
            let frameLength = Self.headerSize + length
            guard buffer.count >= frameLength else { break }
            frames.append(Data(buffer.prefix(frameLength)))
            buffer.removeFirst(frameLength)
        }
        return frames
    }
}
