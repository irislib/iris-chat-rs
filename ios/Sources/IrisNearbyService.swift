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
    @Published private(set) var isLanVisible = false
    @Published private(set) var status = "Off"
    @Published private(set) var lanStatus = "Off"
    @Published private(set) var lanPermissionNeedsSettings = false
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
    private static let maxWriteWithoutResponseChunksPerFlush = 32
    private static let maxPendingWriteChunks = 512
    private static let maxPendingWriteBytes = 2 * 1024 * 1024
    private static let maxNotificationChunksPerDrain = 32
    private static let maxPendingNotifications = 256
    private static let maxPendingNotificationBytes = 2 * 1024 * 1024
    private static let nearbyPresenceKind: UInt32 = 22242
    private static let nonIrisBackoff: TimeInterval = 60
    private static let helloInterval: TimeInterval = 5
    private static let inventoryResendInterval: TimeInterval = 60
    private static let presenceResendInterval: TimeInterval = 60
    private static let peerSweepInterval: TimeInterval = 2
    private static let peerTTL: TimeInterval = 15
    private static let dedupeReconnectBackoff: TimeInterval = 30
    private static let maxSimultaneousPeripherals = 4
    private static let bluetoothSourcePrefix = "bt:"
    private static let centralSourcePrefix = "central:"
    private static let lanSourcePrefix = "lan:"

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
    private var pendingNotificationBytes = 0
    private var notificationDrainScheduled = false
    private var peerIDByPeripheral: [UUID: String] = [:]
    private var peerIDByCentral: [UUID: String] = [:]
    private var bluetoothPeerLastSeen: [String: Date] = [:]
    private var peerInventorySentAt: [String: Date] = [:]
    private var presenceSentAt: [String: Date] = [:]
    private var connectionNonces: [String: String] = [:]
    private var suppressedPeripheralReconnectUntil: [UUID: Date] = [:]
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
    private var maintenanceTimer: Timer?
    private var lastHelloAt = Date.distantPast
    private var lanService: IrisNearbyLanService?

    var ingestEventJson: ((_ eventJson: String, _ transport: String) -> Bool)?
    var buildPresenceEventJson: ((String, String, String, String?) -> String)?
    var verifyPresenceEventJson: ((String, String, String, String) -> String)?
    var encodeFrameJson: ((String) -> Data?)?
    var decodeFrame: ((Data) -> String)?
    var frameBodyLength: ((Data) -> Int)?
    var onBluetoothPermissionDenied: (() -> Void)?
    var onLanPermissionDenied: (() -> Void)?
    var onLanPermissionGranted: (() -> Void)?

    override init() {
        super.init()
        lanService = IrisNearbyLanService(
            peerID: peerID,
            bodyLengthFromHeader: { [weak self] header in
                self?.frameBodyLength?(header) ?? -1
            },
            onFrame: { [weak self] connectionID, frame in
                self?.ingestFrame(frame, source: .lan(connectionID))
            },
            onStatus: { [weak self] status in
                self?.handleLanStatus(status)
            }
        )
    }

    var sidebarSubtitle: String {
        if !isNearbyActive {
            if shouldShowBluetoothPermissionPrompt {
                return "Click to enable"
            }
            if !bluetoothPermissionGranted {
                return "No Bluetooth access"
            }
            return "Off"
        }
        if !peers.isEmpty {
            return Self.nearbySummary(for: peers)
        }
        if !isLanVisible, Self.isBlockingStatus(status) { return status }
        if isLanVisible, Self.isBlockingLanStatus(lanStatus) {
            return Self.wifiStatusLabel(lanStatus)
        }
        return "No users nearby"
    }

    var shouldShowBluetoothPermissionPrompt: Bool {
        CBManager.authorization == .notDetermined
    }

    var bluetoothPermissionGranted: Bool {
        CBManager.authorization == .allowedAlways
    }

    var bluetoothPermissionNeedsSettings: Bool {
        switch CBManager.authorization {
        case .denied, .restricted:
            return true
        default:
            return false
        }
    }

    var isBluetoothOn: Bool {
        guard isVisible else {
            return false
        }
        if centralManager?.state == .poweredOn || peripheralManager?.state == .poweredOn {
            return true
        }
        guard status != "Off", !Self.isBlockingStatus(status) else {
            return false
        }
        return true
    }

    var bluetoothTransportWarning: String? {
        guard isVisible, Self.isBlockingStatus(status) else {
            return nil
        }
        return status
    }

    var lanTransportWarning: String? {
        guard isLanVisible, Self.isBlockingLanStatus(lanStatus) else {
            return nil
        }
        return Self.wifiStatusLabel(lanStatus)
    }

    var shouldRestartLanAfterFailure: Bool {
        isLanVisible && Self.isRecoverableLanFailure(lanStatus)
    }

    var bluetoothPeers: [IrisNearbyPeer] {
        let peerIDs = recentBluetoothPeerIDs
        return peers.filter { peerIDs.contains($0.id) }
    }

    var lanPeers: [IrisNearbyPeer] {
        let peerIDs = lanService?.peerIDs() ?? []
        return peers.filter { peerIDs.contains($0.id) }
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

    private static func nearbyPeerName(
        advertisedName: String?,
        ownerPubkeyHex: String?,
        profileDisplayName: String?,
        existingName: String?
    ) -> String {
        if let name = normalizedPeerName(profileDisplayName) {
            return name
        }
        if let owner = normalizedPeerName(ownerPubkeyHex) {
            return fallbackProfileName(for: owner)
        }
        return normalizedPeerName(advertisedName)
            ?? normalizedPeerName(existingName)
            ?? "Iris"
    }

    private static func normalizedPeerName(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty || trimmed == "Iris" ? nil : trimmed
    }

    private static func fallbackProfileName(for identity: String) -> String {
        let adjectives = [
            "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar",
            "Nova", "Quiet", "Silver", "Solar", "Velvet", "Wild"
        ]
        let nouns = [
            "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener",
            "Otter", "Raven", "Signal", "Sparrow", "Tide", "Voyager"
        ]
        let trimmed = identity.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "Quiet Listener" }

        let hash = trimmed.utf8.reduce(UInt32(0)) { partial, byte in
            partial &* 31 &+ UInt32(byte)
        }
        let adjective = adjectives[Int(hash) % adjectives.count]
        let noun = nouns[(Int(hash) / adjectives.count) % nouns.count]
        return "\(adjective) \(noun)"
    }

    fileprivate static func eventAuthorHex(_ eventJson: String) -> String? {
        guard let data = eventJson.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let pubkey = object["pubkey"] as? String else { return nil }
        let trimmed = pubkey.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.count == 64 ? trimmed : nil
    }

    private static func nearbyPresencePeerID(_ eventJson: String) -> String? {
        guard let data = eventJson.data(using: .utf8),
              let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = event["kind"] as? NSNumber,
              kind.uint32Value == Self.nearbyPresenceKind,
              let content = event["content"] as? String,
              let contentData = content.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: contentData) as? [String: Any],
              let peerID = object["peer_id"] as? String else { return nil }
        let trimmed = peerID.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func isBlockingStatus(_ status: String) -> Bool {
        switch status {
        case "No Bluetooth access", "Bluetooth off", "Bluetooth unavailable", "Bluetooth failed", "Bluetooth reset", "Advertise failed":
            return true
        default:
            return false
        }
    }

    private static func isBlockingLanStatus(_ status: String) -> Bool {
        switch status {
        case "No local network access", "Local network failed", "Local network unavailable":
            return true
        default:
            return false
        }
    }

    private static func isRecoverableLanFailure(_ status: String) -> Bool {
        switch status {
        case "Local network failed", "Local network unavailable":
            return true
        default:
            return false
        }
    }

    private static func wifiStatusLabel(_ status: String) -> String {
        switch status {
        case "No local network access":
            return "No Wi-Fi access"
        case "Local network failed":
            return "Wi-Fi failed"
        case "Local network unavailable":
            return "Wi-Fi unavailable"
        default:
            return status
        }
    }

    var isNearbyActive: Bool {
        isVisible || isLanVisible
    }

    func toggleVisibility() {
        setVisible(!isVisible)
    }

    func startBluetoothStateMonitoring() {
        if centralManager == nil {
            centralManager = CBCentralManager(
                delegate: self,
                queue: .main,
                options: [CBCentralManagerOptionShowPowerAlertKey: false]
            )
        }
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
            startBluetooth()
        } else {
            NSLog("Iris nearby: visible off")
            stopBluetooth()
        }
    }

    func setLanVisible(_ visible: Bool) {
        guard visible != isLanVisible else {
            if visible {
                if Self.isRecoverableLanFailure(lanStatus) {
                    lanStatus = "Starting"
                    lanService?.restart()
                    startMaintenance()
                }
                announceToConnectedPeers()
            }
            return
        }
        isLanVisible = visible
        if visible {
            NSLog("Iris nearby LAN: visible on")
            localNonce = UUID().uuidString.lowercased()
            if lanStatus != "No local network access" {
                lanPermissionNeedsSettings = false
            }
            lanStatus = "Starting"
            lanService?.start()
            startMaintenance()
        } else {
            NSLog("Iris nearby LAN: visible off")
            let lanPeerIDs = lanService?.peerIDs() ?? []
            lanService?.stop()
            lanStatus = "Off"
            removeLanOnlyPeers(lanPeerIDs)
            if !isNearbyActive {
                stopMaintenance()
            }
        }
    }

    func clearLanPermissionSettingsHint() {
        lanPermissionNeedsSettings = false
    }

    private func handleLanStatus(_ status: String) {
        guard isLanVisible || status == "Off" else {
            return
        }
        let previousStatus = lanStatus
        lanStatus = status
        if status == "No local network access" {
            lanPermissionNeedsSettings = true
            onLanPermissionDenied?()
        } else if status == "Visible" || status == "Connected" {
            lanPermissionNeedsSettings = false
            onLanPermissionGranted?()
        }
        if status == "Connected", previousStatus != "Connected" {
            announceToConnectedPeersIfDue()
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
            authorPubkeyHex: Self.eventAuthorHex(eventJson),
            storedAt: Date()
        )
        ownOutbound[eventID] = record
        forwarded.removeValue(forKey: eventID)
        if kind == 0, let profile = IrisNearbyProfileEvent.fromEventJson(eventJson) {
            ownProfileEventID = eventID
            knownProfiles[eventID] = profile
        }
        pruneMailbags()
        guard isNearbyActive else { return }
        if kind == 0 {
            sendHello(excludingPeerID: nil)
        }
        sendEvent(record, excludingPeerID: nil)
    }

    private func startBluetooth() {
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
        startMaintenance()
    }

    private func stopBluetooth() {
        status = "Off"
        if !isLanVisible {
            stopMaintenance()
        }
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
        clearPendingNotifications()
        let bluetoothPeerIDs = recentBluetoothPeerIDs
        peerIDByPeripheral.removeAll()
        peerIDByCentral.removeAll()
        bluetoothPeerLastSeen.removeAll()
        peerInventorySentAt.removeAll()
        connectionNonces.removeAll()
        suppressedPeripheralReconnectUntil.removeAll()
        if isLanVisible {
            let lanPeerIDs = lanService?.peerIDs() ?? []
            peers.removeAll { bluetoothPeerIDs.contains($0.id) && !lanPeerIDs.contains($0.id) }
            for peerID in bluetoothPeerIDs where !lanPeerIDs.contains(peerID) {
                peerNonces.removeValue(forKey: peerID)
            }
        } else {
            peerNonces.removeAll()
            peers.removeAll()
        }
        ignoredPeripherals.removeAll()
        if !isLanVisible {
            incomingFragments.removeAll()
        }
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
    }

    private func announceToConnectedPeersIfDue(now: Date = Date()) {
        guard now.timeIntervalSince(lastHelloAt) >= Self.helloInterval else { return }
        announceToConnectedPeers()
    }

    private func startMaintenance() {
        stopMaintenance()
        lastHelloAt = .distantPast
        let timer = Timer(timeInterval: Self.peerSweepInterval, repeats: true) { [weak self] _ in
            self?.runMaintenance()
        }
        timer.tolerance = 0.5
        RunLoop.main.add(timer, forMode: .common)
        maintenanceTimer = timer
        runMaintenance()
    }

    private func stopMaintenance() {
        maintenanceTimer?.invalidate()
        maintenanceTimer = nil
        lastHelloAt = .distantPast
    }

    private func runMaintenance() {
        guard isNearbyActive else { return }
        let now = Date()
        pruneStalePeers(now: now)
        if now.timeIntervalSince(lastHelloAt) >= Self.helloInterval {
            sendHello(excludingPeerID: nil)
            lastHelloAt = now
        }
    }

    private func shouldConnect(to peripheralID: UUID, advertisementData: [String: Any]) -> Bool {
        if let suppressedUntil = suppressedPeripheralReconnectUntil[peripheralID] {
            if suppressedUntil > Date() {
                return false
            }
            suppressedPeripheralReconnectUntil.removeValue(forKey: peripheralID)
        }
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
        peerIDByPeripheral.removeValue(forKey: peripheral.identifier)
        connectionNonces.removeValue(forKey: Self.bluetoothSourcePrefix + peripheral.identifier.uuidString)
        centralManager?.cancelPeripheralConnection(peripheral)
        if isVisible {
            status = peers.isEmpty ? "Scanning" : sidebarSubtitle
        }
    }

    private func rejectLegacyNearbySource(_ source: IrisNearbySource, type: String) {
        switch source {
        case .peripheral(let peripheral):
            rejectNonIrisPeripheral(
                peripheral,
                reason: "legacy nearby \(type) frame from \(debugPeripheralLabel(peripheral))"
            )
        case .central(let central):
            let centralID = central.identifier
            NSLog("Iris nearby: legacy nearby \(type) frame from central \(centralID.uuidString)")
            subscribedCentrals.removeValue(forKey: centralID)
            centralAssemblers.removeValue(forKey: centralID)
            peerIDByCentral.removeValue(forKey: centralID)
            connectionNonces.removeValue(forKey: Self.centralSourcePrefix + centralID.uuidString)
            removePendingNotifications { item in
                item.channel?.central.identifier == centralID
            }
        case .lan:
            break
        }
    }

    private func sendHello(excludingPeerID: String?) {
        guard isNearbyActive else { return }
        lastHelloAt = Date()
        let envelope: [String: Any] = [
            "v": 1,
            "type": "hello",
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
        for record in records {
            var envelope = [
                "v": 1,
                "type": "inv",
                "id": record.id,
                "kind": Int(record.kind),
                "created_at": NSNumber(value: record.createdAtSecs),
                "size": record.eventJson.utf8.count
            ] as [String: Any]
            if let author = record.authorPubkeyHex {
                envelope["author"] = author
            }
            sendEnvelope(envelope, excludingPeerID: excludingPeerID)
        }
    }

    private func sendInventoryAfterHelloIfNeeded(remotePeerID: String, force: Bool) {
        let now = Date()
        if !force,
           let lastSent = peerInventorySentAt[remotePeerID],
           now.timeIntervalSince(lastSent) < Self.inventoryResendInterval {
            return
        }
        peerInventorySentAt[remotePeerID] = now
        sendInventory(excludingPeerID: nil)
    }

    private func sendWant(_ ids: [String], excludingPeerID: String?) {
        guard !ids.isEmpty else { return }
        for id in ids.prefix(64) {
            sendEnvelope(
                [
                    "v": 1,
                    "type": "want",
                    "id": id
                ],
                excludingPeerID: excludingPeerID
            )
        }
    }

    private func sendEvent(_ record: IrisNearbyStoredEvent, excludingPeerID: String?) {
        sendEventJson(record.eventJson, excludingPeerID: excludingPeerID)
    }

    private func sendEventJson(_ eventJson: String, excludingPeerID: String?) {
        let envelope: [String: Any] = [
            "v": 1,
            "type": "event",
            "event_json": eventJson
        ]
        if let frame = encodeFrame(envelope), frame.count <= Self.singleFrameBytes {
            sendEncodedFrame(frame, excludingPeerID: excludingPeerID)
        } else {
            if let record = IrisNearbyStoredEvent.fromEventJson(eventJson) {
                sendEventFragments(record, excludingPeerID: excludingPeerID)
            }
        }
    }

    private func sendPresence(remoteNonce: String) {
        let eventJson = buildPresenceEventJson?(
            peerID,
            localNonce,
            remoteNonce,
            ownProfileEventID
        ) ?? ""
        guard !eventJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        sendEventJson(eventJson, excludingPeerID: nil)
    }

    private func sendPresenceIfNeeded(remoteNonce: String, responseKey: String, force: Bool) {
        let key = "\(responseKey)|\(remoteNonce)"
        let now = Date()
        if !force,
           let lastSent = presenceSentAt[key],
           now.timeIntervalSince(lastSent) < Self.presenceResendInterval {
            return
        }
        presenceSentAt[key] = now
        prunePresenceSentAt(now: now)
        sendPresence(remoteNonce: remoteNonce)
    }

    private func prunePresenceSentAt(now: Date = Date()) {
        let cutoff = now.addingTimeInterval(-Self.presenceResendInterval * 2)
        presenceSentAt = presenceSentAt.filter { $0.value >= cutoff }
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
        guard isNearbyActive, let frame = encodeFrame(object) else { return }
        sendEncodedFrame(frame, excludingPeerID: excludingPeerID)
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

    private func debugPeripheralLabel(_ peripheral: CBPeripheral) -> String {
        let name = peripheral.name ?? "nil"
        return "\(peripheral.identifier.uuidString) name=\(name)"
    }

    private func sendEncodedFrame(_ frame: Data, excludingPeerID: String?) {
        if isLanVisible {
            lanService?.send(frame, excludingPeerID: excludingPeerID)
        }
        sendBluetoothFrame(frame, excludingPeerID: excludingPeerID)
    }

    private func sendBluetoothFrame(_ frame: Data, excludingPeerID: String?) {
        guard isVisible else { return }
        for (id, characteristic) in writableCharacteristics {
            if !shouldSendViaOutgoingBluetoothRoute(peripheralID: id, excludingPeerID: excludingPeerID) {
                continue
            }
            guard let peripheral = peripherals[id] else { continue }
            write(frame, to: peripheral, characteristic: characteristic)
        }
        for (id, channel) in subscribedCentrals {
            if !shouldSendViaIncomingBluetoothRoute(centralID: id, excludingPeerID: excludingPeerID) {
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
        if queue.isEmpty {
            queue.writeType = writeType
        }
        var offset = 0
        while offset < data.count {
            let end = min(offset + maxLength, data.count)
            queue.append(Data(data[offset..<end]))
            offset = end
        }
        let droppedChunks = queue.trimToLimits(
            maxChunks: Self.maxPendingWriteChunks,
            maxBytes: Self.maxPendingWriteBytes
        )
        if droppedChunks > 0 {
            NSLog("Iris nearby: dropped stale Bluetooth write chunks \(droppedChunks)")
        }
        peripheralWriteQueues[peripheral.identifier] = queue
        flushWriteQueue(for: peripheral.identifier)
    }

    private func flushWriteQueue(for peripheralID: UUID) {
        var chunkBudget = Self.maxWriteWithoutResponseChunksPerFlush
        flushWriteQueue(for: peripheralID, chunkBudget: &chunkBudget)
        if chunkBudget == 0, peripheralWriteQueues[peripheralID]?.writeType == .withoutResponse {
            DispatchQueue.main.async { [weak self] in
                self?.flushWriteQueue(for: peripheralID)
            }
        }
    }

    private func flushWriteQueue(for peripheralID: UUID, chunkBudget: inout Int) {
        guard var queue = peripheralWriteQueues[peripheralID],
              let peripheral = peripherals[peripheralID],
              let characteristic = writableCharacteristics[peripheralID] else { return }

        switch queue.writeType {
        case .withResponse:
            if queue.isEmpty {
                peripheralWriteQueues.removeValue(forKey: peripheralID)
                return
            }
            guard !queue.waitingForResponse else {
                peripheralWriteQueues[peripheralID] = queue
                return
            }
            guard let chunk = queue.popFirst() else {
                peripheralWriteQueues.removeValue(forKey: peripheralID)
                return
            }
            queue.waitingForResponse = true
            peripheralWriteQueues[peripheralID] = queue
            peripheral.writeValue(chunk, for: characteristic, type: .withResponse)
        case .withoutResponse:
            while !queue.isEmpty && chunkBudget > 0 && peripheral.canSendWriteWithoutResponse {
                guard let chunk = queue.popFirst() else { break }
                peripheral.writeValue(chunk, for: characteristic, type: .withoutResponse)
                chunkBudget -= 1
            }
            if queue.isEmpty {
                peripheralWriteQueues.removeValue(forKey: peripheralID)
            } else {
                peripheralWriteQueues[peripheralID] = queue
            }
        @unknown default:
            peripheralWriteQueues.removeValue(forKey: peripheralID)
        }
    }

    private func notify(_ data: Data, to channel: IrisNearbyCentralChannel?) {
        var chunkBudget = Self.maxNotificationChunksPerDrain
        if !notify(data, to: channel, chunkBudget: &chunkBudget), chunkBudget == 0 {
            scheduleNotificationDrain()
        }
    }

    @discardableResult
    private func notify(
        _ data: Data,
        to channel: IrisNearbyCentralChannel?,
        chunkBudget: inout Int
    ) -> Bool {
        guard let peripheralManager else { return true }
        guard let characteristic = channel?.characteristic ?? localCharacteristic else { return true }
        let maxLength = max(20, channel?.central.maximumUpdateValueLength ?? 180)
        var offset = 0
        while offset < data.count, chunkBudget > 0 {
            let end = min(offset + maxLength, data.count)
            let sent = peripheralManager.updateValue(
                Data(data[offset..<end]),
                for: characteristic,
                onSubscribedCentrals: channel.map { [$0.central] }
            )
            if !sent {
                enqueuePendingNotification(Data(data[offset..<data.count]), channel)
                return false
            }
            offset = end
            chunkBudget -= 1
        }
        if offset < data.count {
            enqueuePendingNotification(Data(data[offset..<data.count]), channel)
            return false
        }
        return true
    }

    private func enqueuePendingNotification(_ data: Data, _ channel: IrisNearbyCentralChannel?) {
        guard !data.isEmpty else { return }
        pendingNotifications.append((data, channel))
        pendingNotificationBytes += data.count
        trimPendingNotifications()
    }

    private func trimPendingNotifications() {
        while pendingNotifications.count > Self.maxPendingNotifications ||
            pendingNotificationBytes > Self.maxPendingNotificationBytes {
            let dropped = pendingNotifications.removeFirst()
            pendingNotificationBytes = max(0, pendingNotificationBytes - dropped.data.count)
        }
    }

    private func clearPendingNotifications() {
        pendingNotifications.removeAll()
        pendingNotificationBytes = 0
        notificationDrainScheduled = false
    }

    private func removePendingNotifications(where shouldRemove: ((data: Data, channel: IrisNearbyCentralChannel?)) -> Bool) {
        pendingNotifications.removeAll(where: shouldRemove)
        pendingNotificationBytes = pendingNotifications.reduce(0) { $0 + $1.data.count }
    }

    private func scheduleNotificationDrain() {
        guard !notificationDrainScheduled else { return }
        notificationDrainScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.drainPendingNotifications()
        }
    }

    private func drainPendingNotifications() {
        notificationDrainScheduled = false
        var chunkBudget = Self.maxNotificationChunksPerDrain
        while chunkBudget > 0, !pendingNotifications.isEmpty {
            let item = pendingNotifications.removeFirst()
            pendingNotificationBytes = max(0, pendingNotificationBytes - item.data.count)
            if let centralID = item.channel?.central.identifier,
               subscribedCentrals[centralID] == nil {
                continue
            }
            if !notify(item.data, to: item.channel, chunkBudget: &chunkBudget) {
                break
            }
        }
        if !pendingNotifications.isEmpty, chunkBudget == 0 {
            scheduleNotificationDrain()
        }
    }

    private func ingestFrame(_ frame: Data, source: IrisNearbySource) {
        let sourceKey = sourceKey(for: source)
        guard let envelope = decodeFrameJson(frame),
              let type = envelope["type"] as? String else { return }
        if envelope["peer_id"] != nil {
            rejectLegacyNearbySource(source, type: type)
            return
        }
        let remotePeerID = peerIDForSource(source)
        if let remotePeerID, !remotePeerID.isEmpty {
            touchPeer(remotePeerID)
            markTransportPeer(remotePeerID, source: source)
        }

        switch type {
        case "hello":
            let remoteNonce = sanitizedNonce(envelope["nonce"] as? String)
            if let remoteNonce {
                connectionNonces[sourceKey] = remoteNonce
            }
            if let remotePeerID, !remotePeerID.isEmpty {
                let previousNonce = peerNonces[remotePeerID]
                let wasNew = !peers.contains { $0.id == remotePeerID }
                rememberPeer(
                    remotePeerID,
                    name: envelope["name"] as? String,
                    profileEventID: nil,
                    source: source
                )
                let nonceChanged = remoteNonce != nil && remoteNonce != previousNonce
                if wasNew || nonceChanged {
                    sendHello(excludingPeerID: nil)
                }
                if let remoteNonce {
                    peerNonces[remotePeerID] = remoteNonce
                    sendPresenceIfNeeded(
                        remoteNonce: remoteNonce,
                        responseKey: remotePeerID,
                        force: wasNew || nonceChanged
                    )
                }
                sendInventoryAfterHelloIfNeeded(remotePeerID: remotePeerID, force: wasNew || nonceChanged)
            } else if let remoteNonce {
                sendPresenceIfNeeded(remoteNonce: remoteNonce, responseKey: sourceKey, force: false)
                sendInventoryAfterHelloIfNeeded(remotePeerID: sourceKey, force: false)
            }
        case "inv":
            handleInventory(envelope)
        case "want":
            handleWant(envelope)
        case "event":
            handleEventEnvelope(envelope, remotePeerID: remotePeerID, sourceKey: sourceKey)
        case "event_frag":
            handleEventFragment(envelope, remotePeerID: remotePeerID, sourceKey: sourceKey)
        default:
            break
        }
    }

    private func handleInventory(_ envelope: [String: Any]) {
        guard let id = envelope["id"] as? String,
              id.count == 64,
              ownOutbound[id] == nil,
              forwarded[id] == nil else { return }
        let size = (envelope["size"] as? NSNumber)?.intValue ?? (envelope["size"] as? Int ?? 0)
        if size > 0, size <= Self.maxEventBytes {
            sendWant([id], excludingPeerID: nil)
        }
    }

    private func handleWant(_ envelope: [String: Any]) {
        guard let id = envelope["id"] as? String,
              let record = ownOutbound[id] ?? forwarded[id] else { return }
        sendEvent(record, excludingPeerID: nil)
    }

    private func handleEventEnvelope(_ envelope: [String: Any], remotePeerID: String?, sourceKey: String?) {
        guard let eventJson = envelope["event_json"] as? String else { return }
        handleEventJson(eventJson, remotePeerID: remotePeerID, sourceKey: sourceKey)
    }

    private func handleEventFragment(_ envelope: [String: Any], remotePeerID: String?, sourceKey: String?) {
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
            remotePeerID: remotePeerID,
            sourceKey: sourceKey
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
        handleEventJson(eventJson, remotePeerID: remotePeerID ?? fragment.remotePeerID, sourceKey: sourceKey ?? fragment.sourceKey)
    }

    private func handleEventJson(_ eventJson: String, remotePeerID: String?, sourceKey: String?) {
        guard eventJson.utf8.count <= Self.maxEventBytes,
              let record = IrisNearbyStoredEvent.fromEventJson(eventJson) else { return }
        if record.kind == Self.nearbyPresenceKind {
            if handlePresenceEvent(eventJson, remotePeerID: remotePeerID, sourceKey: sourceKey) {
                NSLog("Iris nearby: accepted presence")
            }
            return
        }
        if let existing = ownOutbound[record.id] ?? forwarded[record.id] {
            rememberProfile(from: existing.eventJson, remotePeerID: remotePeerID)
            return
        }
        let transport = transportLabel(for: remotePeerID)
        let accepted = ingestEventJson?(eventJson, transport) ?? false
        guard accepted else { return }
        rememberProfile(from: eventJson, remotePeerID: remotePeerID)
        forwarded[record.id] = record
        pruneMailbags()
        sendInventory(excludingPeerID: remotePeerID)
        NSLog("Iris nearby: accepted event kind %u %@", record.kind, record.id)
    }

    private func handlePresenceEvent(_ eventJson: String, remotePeerID: String?, sourceKey: String?) -> Bool {
        let peerID = remotePeerID.flatMap { nonempty($0) } ?? Self.nearbyPresencePeerID(eventJson)
        guard let peerID else { return false }
        for candidate in presenceNonceCandidates(remotePeerID: remotePeerID, sourceKey: sourceKey) {
            let result = verifyPresenceEventJson?(
                eventJson,
                peerID,
                localNonce,
                candidate.nonce
            ) ?? ""
            guard let data = result.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let ownerPubkeyHex = object["owner_pubkey_hex"] as? String,
                  ownerPubkeyHex.count == 64 else {
                continue
            }
            if let sourceKey {
                markTransportPeer(peerID, sourceKey: sourceKey)
                connectionNonces.removeValue(forKey: sourceKey)
            }
            if let nonceKey = candidate.key {
                connectionNonces.removeValue(forKey: nonceKey)
            }
            rememberPresence(
                peerID: peerID,
                ownerPubkeyHex: ownerPubkeyHex,
                profileEventID: sanitizedEventID(object["profile_event_id"] as? String)
            )
            return true
        }
        return false
    }

    private func presenceNonceCandidates(remotePeerID: String?, sourceKey: String?) -> [(key: String?, nonce: String)] {
        if let remotePeerID, let nonce = peerNonces[remotePeerID] {
            return [(nil, nonce)]
        }
        var candidates: [(key: String?, nonce: String)] = []
        var seen = Set<String>()
        if let sourceKey, let nonce = connectionNonces[sourceKey] {
            candidates.append((sourceKey, nonce))
            seen.insert(sourceKey)
        }
        for (key, nonce) in connectionNonces where !seen.contains(key) {
            candidates.append((key, nonce))
        }
        return candidates
    }

    private func pruneIncomingFragments() {
        let cutoff = Date().addingTimeInterval(-Self.fragmentTTL)
        incomingFragments = incomingFragments.filter { $0.value.storedAt >= cutoff }
        while incomingFragments.count > Self.maxIncomingFragmentSets,
              let oldest = incomingFragments.min(by: { $0.value.storedAt < $1.value.storedAt })?.key {
            incomingFragments.removeValue(forKey: oldest)
        }
    }

    private func pruneStalePeers(now: Date) {
        bluetoothPeerLastSeen = bluetoothPeerLastSeen.filter { now.timeIntervalSince($0.value) <= Self.peerTTL }
        let stalePeerIDs = Set(
            peers
                .filter { now.timeIntervalSince($0.lastSeen) > Self.peerTTL }
                .map { $0.id }
        )
        guard !stalePeerIDs.isEmpty else { return }

        peers.removeAll { stalePeerIDs.contains($0.id) }
        for peerID in stalePeerIDs {
            bluetoothPeerLastSeen.removeValue(forKey: peerID)
            peerInventorySentAt.removeValue(forKey: peerID)
            presenceSentAt = presenceSentAt.filter { !$0.key.hasPrefix("\(peerID)|") }
            peerNonces.removeValue(forKey: peerID)
        }

        let stalePeripheralIDs = peerIDByPeripheral
            .filter { stalePeerIDs.contains($0.value) }
            .map { $0.key }
        for peripheralID in stalePeripheralIDs {
            peerIDByPeripheral.removeValue(forKey: peripheralID)
            connectionNonces.removeValue(forKey: Self.bluetoothSourcePrefix + peripheralID.uuidString)
            writableCharacteristics.removeValue(forKey: peripheralID)
            peripheralAssemblers.removeValue(forKey: peripheralID)
            peripheralWriteQueues.removeValue(forKey: peripheralID)
            if let peripheral = peripherals.removeValue(forKey: peripheralID) {
                centralManager?.cancelPeripheralConnection(peripheral)
            }
        }

        let staleCentralIDs = peerIDByCentral
            .filter { stalePeerIDs.contains($0.value) }
            .map { $0.key }
        for centralID in staleCentralIDs {
            peerIDByCentral.removeValue(forKey: centralID)
            connectionNonces.removeValue(forKey: Self.centralSourcePrefix + centralID.uuidString)
            subscribedCentrals.removeValue(forKey: centralID)
            centralAssemblers.removeValue(forKey: centralID)
        }
        removePendingNotifications { item in
            guard let centralID = item.channel?.central.identifier else { return false }
            return staleCentralIDs.contains(centralID)
        }

        pruneKnownProfiles()
        restartScanningAfterPruning()
        status = peers.isEmpty ? visibleIdleStatus : sidebarSubtitle
        NSLog("Iris nearby: expired stale peers \(stalePeerIDs.count)")
    }

    private func removeLanOnlyPeers(_ lanPeerIDs: Set<String>) {
        let bluetoothPeerIDs = recentBluetoothPeerIDs
        peers.removeAll { lanPeerIDs.contains($0.id) && !bluetoothPeerIDs.contains($0.id) }
        for peerID in lanPeerIDs where !bluetoothPeerIDs.contains(peerID) {
            peerNonces.removeValue(forKey: peerID)
        }
        status = peers.isEmpty ? visibleIdleStatus : sidebarSubtitle
    }

    private func transportLabel(for remotePeerID: String?) -> String {
        guard let remotePeerID, !remotePeerID.isEmpty else { return "nearby" }
        if recentBluetoothPeerIDs.contains(remotePeerID) {
            return "bluetooth"
        }
        return "wifi"
    }

    private var recentBluetoothPeerIDs: Set<String> {
        let cutoff = Date().addingTimeInterval(-Self.peerTTL)
        return Set(bluetoothPeerLastSeen.filter { $0.value >= cutoff }.keys)
    }

    private func shouldSendViaOutgoingBluetoothRoute(peripheralID: UUID, excludingPeerID: String?) -> Bool {
        guard let remotePeerID = peerIDByPeripheral[peripheralID] else {
            return true
        }
        if let excludingPeerID, remotePeerID == excludingPeerID {
            return false
        }
        if lanService?.hasPeer(remotePeerID) == true {
            return false
        }
        if hasOutgoingBluetoothRoute(remotePeerID), hasIncomingBluetoothRoute(remotePeerID) {
            return shouldUseOutgoingBluetoothRoute(remotePeerID)
        }
        return true
    }

    private func shouldSendViaIncomingBluetoothRoute(centralID: UUID, excludingPeerID: String?) -> Bool {
        guard let remotePeerID = peerIDByCentral[centralID] else {
            return true
        }
        if let excludingPeerID, remotePeerID == excludingPeerID {
            return false
        }
        if lanService?.hasPeer(remotePeerID) == true {
            return false
        }
        if hasOutgoingBluetoothRoute(remotePeerID), hasIncomingBluetoothRoute(remotePeerID) {
            return !shouldUseOutgoingBluetoothRoute(remotePeerID)
        }
        return true
    }

    private func shouldUseOutgoingBluetoothRoute(_ remotePeerID: String) -> Bool {
        peerID < remotePeerID
    }

    private func hasOutgoingBluetoothRoute(_ remotePeerID: String) -> Bool {
        writableCharacteristics.keys.contains { peerIDByPeripheral[$0] == remotePeerID }
    }

    private func hasIncomingBluetoothRoute(_ remotePeerID: String) -> Bool {
        subscribedCentrals.keys.contains { peerIDByCentral[$0] == remotePeerID }
    }

    private func pruneDuplicateBluetoothRoutes(for remotePeerID: String) {
        guard hasOutgoingBluetoothRoute(remotePeerID), hasIncomingBluetoothRoute(remotePeerID) else {
            return
        }
        guard !shouldUseOutgoingBluetoothRoute(remotePeerID) else {
            return
        }
        let peripheralIDs = peerIDByPeripheral
            .filter { $0.value == remotePeerID }
            .map(\.key)
        for peripheralID in peripheralIDs {
            suppressedPeripheralReconnectUntil[peripheralID] = Date().addingTimeInterval(Self.dedupeReconnectBackoff)
            peerIDByPeripheral.removeValue(forKey: peripheralID)
            connectionNonces.removeValue(forKey: Self.bluetoothSourcePrefix + peripheralID.uuidString)
            writableCharacteristics.removeValue(forKey: peripheralID)
            peripheralAssemblers.removeValue(forKey: peripheralID)
            peripheralWriteQueues.removeValue(forKey: peripheralID)
            if let peripheral = peripherals.removeValue(forKey: peripheralID) {
                centralManager?.cancelPeripheralConnection(peripheral)
            }
        }
    }

    private func markTransportPeer(_ peerID: String, source: IrisNearbySource) {
        switch source {
        case .peripheral(let peripheral):
            peerIDByPeripheral[peripheral.identifier] = peerID
            bluetoothPeerLastSeen[peerID] = Date()
        case .central(let central):
            peerIDByCentral[central.identifier] = peerID
            bluetoothPeerLastSeen[peerID] = Date()
        case .lan(let connectionID):
            lanService?.markPeer(connectionID: connectionID, peerID: peerID)
        }
        pruneDuplicateBluetoothRoutes(for: peerID)
    }

    private func markTransportPeer(_ peerID: String, sourceKey: String) {
        if sourceKey.hasPrefix(Self.bluetoothSourcePrefix) {
            let value = String(sourceKey.dropFirst(Self.bluetoothSourcePrefix.count))
            guard let peripheralID = UUID(uuidString: value) else { return }
            peerIDByPeripheral[peripheralID] = peerID
            bluetoothPeerLastSeen[peerID] = Date()
        } else if sourceKey.hasPrefix(Self.centralSourcePrefix) {
            let value = String(sourceKey.dropFirst(Self.centralSourcePrefix.count))
            guard let centralID = UUID(uuidString: value) else { return }
            peerIDByCentral[centralID] = peerID
            bluetoothPeerLastSeen[peerID] = Date()
        } else if sourceKey.hasPrefix(Self.lanSourcePrefix) {
            lanService?.markPeer(
                connectionID: String(sourceKey.dropFirst(Self.lanSourcePrefix.count)),
                peerID: peerID
            )
        }
        pruneDuplicateBluetoothRoutes(for: peerID)
    }

    private func peerIDForSource(_ source: IrisNearbySource) -> String? {
        switch source {
        case .peripheral(let peripheral):
            return peerIDByPeripheral[peripheral.identifier]
        case .central(let central):
            return peerIDByCentral[central.identifier]
        case .lan(let connectionID):
            return lanService?.peerIDForConnection(connectionID)
        }
    }

    private func sourceKey(for source: IrisNearbySource) -> String {
        switch source {
        case .peripheral(let peripheral):
            return Self.bluetoothSourcePrefix + peripheral.identifier.uuidString
        case .central(let central):
            return Self.centralSourcePrefix + central.identifier.uuidString
        case .lan(let connectionID):
            return Self.lanSourcePrefix + connectionID
        }
    }

    private func restartScanningAfterPruning() {
        guard isVisible, let centralManager, centralManager.state == .poweredOn else { return }
        centralManager.stopScan()
        centralManager.scanForPeripherals(
            withServices: [Self.serviceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
    }

    private func rememberPeer(
        _ peerID: String,
        name: String?,
        profileEventID: String?,
        source: IrisNearbySource
    ) {
        markTransportPeer(peerID, source: source)
        let sanitizedProfileEventID = sanitizedEventID(profileEventID)
        let existing = peers.first(where: { $0.id == peerID })
        let peer = IrisNearbyPeer(
            id: peerID,
            name: Self.nearbyPeerName(
                advertisedName: name,
                ownerPubkeyHex: existing?.ownerPubkeyHex,
                profileDisplayName: nil,
                existingName: existing?.name
            ),
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

    private func touchPeer(_ peerID: String) {
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        peers[index].lastSeen = Date()
    }

    private var visibleIdleStatus: String {
        if centralManager?.state == .poweredOn || peripheralManager?.state == .poweredOn {
            return "Visible"
        }
        if let centralManager {
            return bluetoothStatus(centralManager.state)
        }
        if let peripheralManager {
            return bluetoothStatus(peripheralManager.state)
        }
        return "Visible"
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
                        name: Self.nearbyPeerName(
                            advertisedName: nil,
                            ownerPubkeyHex: profile.ownerPubkeyHex,
                            profileDisplayName: profile.displayName,
                            existingName: nil
                        ),
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
        if peers.firstIndex(where: { $0.id == peerID }) == nil {
            peers.append(
                IrisNearbyPeer(
                    id: peerID,
                    name: Self.nearbyPeerName(
                        advertisedName: nil,
                        ownerPubkeyHex: ownerPubkeyHex,
                        profileDisplayName: nil,
                        existingName: nil
                    ),
                    ownerPubkeyHex: nil,
                    pictureURL: nil,
                    profileEventID: nil,
                    lastSeen: Date()
                )
            )
        }
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        let nextProfileEventID = profileEventID ?? peers[index].profileEventID
        peers[index].ownerPubkeyHex = ownerPubkeyHex
        peers[index].profileEventID = nextProfileEventID
        peers[index].name = Self.nearbyPeerName(
            advertisedName: nil,
            ownerPubkeyHex: ownerPubkeyHex,
            profileDisplayName: nil,
            existingName: peers[index].name
        )
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
        peers[index].name = Self.nearbyPeerName(
            advertisedName: nil,
            ownerPubkeyHex: profile.ownerPubkeyHex,
            profileDisplayName: profile.displayName,
            existingName: peers[index].name
        )
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

    private func sanitizedEventID(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.count == 64 ? trimmed : nil
    }

    private func sanitizedNonce(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return (16...128).contains(trimmed.count) ? trimmed : nil
    }

    private func nonempty(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
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
            if central.state == .unauthorized {
                onBluetoothPermissionDenied?()
            }
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
        let remotePeerID = peerIDByPeripheral[peripheral.identifier]
        peripherals.removeValue(forKey: peripheral.identifier)
        writableCharacteristics.removeValue(forKey: peripheral.identifier)
        peripheralAssemblers.removeValue(forKey: peripheral.identifier)
        peripheralWriteQueues.removeValue(forKey: peripheral.identifier)
        peerIDByPeripheral.removeValue(forKey: peripheral.identifier)
        connectionNonces.removeValue(forKey: Self.bluetoothSourcePrefix + peripheral.identifier.uuidString)
        if let remotePeerID, !shouldUseOutgoingBluetoothRoute(remotePeerID) {
            suppressedPeripheralReconnectUntil[peripheral.identifier] =
                Date().addingTimeInterval(Self.dedupeReconnectBackoff)
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
            if peripheral.state == .unauthorized {
                onBluetoothPermissionDenied?()
            }
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
    }

    func peripheralManager(
        _ peripheral: CBPeripheralManager,
        central: CBCentral,
        didUnsubscribeFrom characteristic: CBCharacteristic
    ) {
        subscribedCentrals.removeValue(forKey: central.identifier)
        centralAssemblers.removeValue(forKey: central.identifier)
        peerIDByCentral.removeValue(forKey: central.identifier)
        connectionNonces.removeValue(forKey: Self.centralSourcePrefix + central.identifier.uuidString)
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
        drainPendingNotifications()
    }
}

private struct IrisNearbyCentralChannel {
    let central: CBCentral
    let characteristic: CBMutableCharacteristic
}

struct IrisNearbyPeripheralWriteQueue {
    private var chunks: [Data] = []
    private var headIndex = 0
    private(set) var pendingBytes = 0
    var writeType: CBCharacteristicWriteType = .withResponse
    var waitingForResponse = false

    var isEmpty: Bool {
        headIndex >= chunks.count
    }

    var count: Int {
        max(0, chunks.count - headIndex)
    }

    mutating func append(_ chunk: Data) {
        guard !chunk.isEmpty else { return }
        chunks.append(chunk)
        pendingBytes += chunk.count
    }

    mutating func popFirst() -> Data? {
        guard headIndex < chunks.count else {
            compactIfNeeded()
            return nil
        }
        let chunk = chunks[headIndex]
        headIndex += 1
        pendingBytes = max(0, pendingBytes - chunk.count)
        compactIfNeeded()
        return chunk
    }

    @discardableResult
    mutating func trimToLimits(maxChunks: Int, maxBytes: Int) -> Int {
        var dropped = 0
        while count > maxChunks || pendingBytes > maxBytes {
            guard popFirst() != nil else { break }
            dropped += 1
        }
        return dropped
    }

    private mutating func compactIfNeeded() {
        guard headIndex > 0 else { return }
        if headIndex >= chunks.count {
            chunks.removeAll(keepingCapacity: true)
            headIndex = 0
        } else if headIndex > 64, headIndex * 2 >= chunks.count {
            chunks.removeFirst(headIndex)
            headIndex = 0
        }
    }
}

private enum IrisNearbySource {
    case peripheral(CBPeripheral)
    case central(CBCentral)
    case lan(String)
}

private struct IrisNearbyStoredEvent {
    let id: String
    let kind: UInt32
    let createdAtSecs: UInt64
    let eventJson: String
    let authorPubkeyHex: String?
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
            authorPubkeyHex: IrisNearbyService.eventAuthorHex(eventJson),
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
    let sourceKey: String?
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
