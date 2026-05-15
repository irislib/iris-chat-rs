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
    @Published private(set) var bluetoothAuthorization: CBManagerAuthorization = CBManager.authorization
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
    private static let maxPeerDedupEntries = 512
    private static let nearbyPresenceKind: UInt32 = 22242
    /// Event kinds that are ephemeral / per-recipient signals and shouldn't
    /// be carried in the offline mailbag — relaying a "seen" or "typing"
    /// hours later via a third-party peer would leak metadata about a
    /// conversation the original sender didn't intend to broadcast. They
    /// still go out over the live BT / Wi-Fi link to currently-connected
    /// peers; just no store-and-forward.
    /// 7 = reaction (kept; reactions are legitimate content)
    /// 14 = chat message (kept)
    /// 15 = receipt (delivered / seen) — suppressed
    /// 25 = typing — suppressed
    private static let mailbagSuppressedKinds: Set<UInt32> = [15, 25]

    private static func shouldStoreInMailbag(kind: UInt32) -> Bool {
        !mailbagSuppressedKinds.contains(kind)
    }
    private static let nonIrisBackoff: TimeInterval = 60
    private static let helloInterval: TimeInterval = 5
    private static let inventoryResendInterval: TimeInterval = 60
    private static let presenceResendInterval: TimeInterval = 60
    private static let unverifiedPresenceResendInterval: TimeInterval = 5
    private static let peerSweepInterval: TimeInterval = 2
    private static let peerTTL: TimeInterval = 15
    private static let peerTouchPublishInterval: TimeInterval = 5
    private static let dedupeReconnectBackoff: TimeInterval = 30
    private static let maxSimultaneousPeripherals = 4
    private static let bluetoothSourcePrefix = "bt:"
    private static let centralSourcePrefix = "central:"
    private static let lanSourcePrefix = "lan:"

    private struct RecentIDSet {
        private var ids: Set<String> = []
        private var recentIDs: [String] = []

        mutating func insert(_ id: String, limit: Int) -> Bool {
            if ids.contains(id) {
                recentIDs.removeAll { $0 == id }
                recentIDs.append(id)
                return false
            }
            ids.insert(id)
            recentIDs.append(id)
            while ids.count > limit, let oldest = recentIDs.first {
                recentIDs.removeFirst()
                ids.remove(oldest)
            }
            return true
        }

        func contains(_ id: String) -> Bool {
            ids.contains(id)
        }
    }

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
    private var inventoryIDsSentByPeer: [String: RecentIDSet] = [:]
    private var wantIDsSentByPeer: [String: RecentIDSet] = [:]
    private var eventIDsSeenByPeer: [String: RecentIDSet] = [:]
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
    private var cachedHelloNonce: String?
    private var cachedHelloFrame: Data?
    private var ownProfileEventID: String?
    private var lastCentralStateLog: String?
    private var lastPeripheralStateLog: String?
    private var maintenanceTimer: Timer?
    private var lastHelloAt = Date.distantPast
    private var lanService: IrisNearbyLanService?
    private let localDeviceName = IrisNearbyService.resolveLocalDeviceName()
    private let codecQueue = DispatchQueue(label: "to.iris.chat.nearby.codec", qos: .utility)

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
                return "Tap to enable"
            }
            if !bluetoothPermissionGranted {
                return "No Bluetooth access"
            }
            return "Tap to enable"
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
        bluetoothAuthorization == .notDetermined
    }

    var bluetoothPermissionGranted: Bool {
        bluetoothAuthorization == .allowedAlways
    }

    var bluetoothPermissionNeedsSettings: Bool {
        switch bluetoothAuthorization {
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
        if screenshotFixtureBluetoothPeerIDs != nil { return nil }
        guard isVisible, Self.isBlockingStatus(status) else {
            return nil
        }
        return status
    }

    var lanTransportWarning: String? {
        if screenshotFixtureLanPeerIDs != nil { return nil }
        guard isLanVisible, Self.isBlockingLanStatus(lanStatus) else {
            return nil
        }
        return Self.wifiStatusLabel(lanStatus)
    }

    var shouldRestartLanAfterFailure: Bool {
        isLanVisible && Self.isRecoverableLanFailure(lanStatus)
    }

    var bluetoothPeers: [IrisNearbyPeer] {
        if let override = screenshotFixtureBluetoothPeerIDs {
            return peers.filter { override.contains($0.id) }
        }
        let peerIDs = recentBluetoothPeerIDs
        return peers.filter { peerIDs.contains($0.id) }
    }

    /// Concise one-liner for the Nearby UI: "N yours · M from others"
    /// when the mailbag has anything queued. Reads in-memory dictionaries
    /// already maintained by the ingest path; no extra storage or polling.
    var mailbagSummary: String? {
        let mine = ownOutbound.count
        let others = forwarded.count
        guard mine + others > 0 else { return nil }
        return "\(mine) yours · \(others) from others"
    }

    var lanPeers: [IrisNearbyPeer] {
        if let override = screenshotFixtureLanPeerIDs {
            return peers.filter { override.contains($0.id) }
        }
        let peerIDs = lanService?.peerIDs() ?? []
        return peers.filter { peerIDs.contains($0.id) }
    }

    private var screenshotFixtureBluetoothPeerIDs: Set<String>?
    private var screenshotFixtureLanPeerIDs: Set<String>?

    /// Test escape hatch used by `ScreenshotFixture` to paint a populated
    /// Nearby modal without any real Bluetooth or LAN traffic.
    func applyScreenshotFixturePeers(
        peers: [IrisNearbyPeer],
        bluetoothPeerIDs: [String],
        lanPeerIDs: [String]
    ) {
        self.peers = peers
        self.screenshotFixtureBluetoothPeerIDs = Set(bluetoothPeerIDs)
        self.screenshotFixtureLanPeerIDs = Set(lanPeerIDs)
        if !isVisible {
            isVisible = true
        }
        if !isLanVisible {
            isLanVisible = true
        }
        if !bluetoothPeerIDs.isEmpty {
            status = "Visible"
        }
        if !lanPeerIDs.isEmpty {
            lanStatus = "Visible"
        }
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

    func refreshBluetoothAuthorizationStatus() {
        setBluetoothAuthorization(CBManager.authorization)
    }

    private func setBluetoothAuthorization(_ authorization: CBManagerAuthorization) {
        guard bluetoothAuthorization != authorization else { return }
        bluetoothAuthorization = authorization
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
            irisDebugLog("Iris nearby: visible on")
            localNonce = UUID().uuidString.lowercased()
            invalidateCachedHelloFrame()
            startBluetooth()
        } else {
            irisDebugLog("Iris nearby: visible off")
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
            irisDebugLog("Iris nearby LAN: visible on")
            localNonce = UUID().uuidString.lowercased()
            invalidateCachedHelloFrame()
            if lanStatus != "No local network access" {
                lanPermissionNeedsSettings = false
            }
            lanStatus = "Starting"
            lanService?.start()
            startMaintenance()
        } else {
            irisDebugLog("Iris nearby LAN: visible off")
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
        // Receipts + typing indicators go direct to currently-connected
        // peers but never into the mailbag for store-and-forward. See
        // `mailbagSuppressedKinds`.
        if Self.shouldStoreInMailbag(kind: kind) {
            ownOutbound[eventID] = record
            forwarded.removeValue(forKey: eventID)
        }
        if kind == 0, let profile = IrisNearbyProfileEvent.fromEventJson(eventJson) {
            ownProfileEventID = eventID
            knownProfiles[eventID] = profile
        }
        pruneMailbags()
        guard isNearbyActive else { return }
        if kind == 0 {
            sendHello(excludingPeerID: nil)
        }
        sendEvent(record, excludingPeerID: nil, onlyPeerID: nil)
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
        inventoryIDsSentByPeer.removeAll()
        wantIDsSentByPeer.removeAll()
        eventIDsSeenByPeer.removeAll()
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
        irisDebugLog("Iris nearby: scanning")
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
        irisDebugLog("Iris nearby: advertising")
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
        irisDebugLog("Iris nearby: \(reason)")
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
            irisDebugLog("Iris nearby: legacy nearby \(type) frame from central \(centralID.uuidString)")
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

    private func sendHello(excludingPeerID: String?, force: Bool = false) {
        guard isNearbyActive else { return }
        let now = Date()
        if !force, now.timeIntervalSince(lastHelloAt) < 1 {
            return
        }
        lastHelloAt = now
        let nonce = localNonce
        if cachedHelloNonce == nonce, let cachedHelloFrame {
            sendEncodedFrame(
                cachedHelloFrame,
                excludingPeerID: excludingPeerID,
                onlyPeerID: nil,
                allowBluetoothWhenLanPeer: true
            )
            return
        }
        let envelope: [String: Any] = [
            "v": 1,
            "type": "hello",
            "nonce": nonce,
            "name": localDeviceName
        ]
        encodeFrame(envelope) { [weak self] frame in
            guard let self,
                  self.isNearbyActive,
                  self.localNonce == nonce,
                  let frame else { return }
            self.cachedHelloNonce = nonce
            self.cachedHelloFrame = frame
            self.sendEncodedFrame(
                frame,
                excludingPeerID: excludingPeerID,
                onlyPeerID: nil,
                allowBluetoothWhenLanPeer: true
            )
        }
    }

    private func invalidateCachedHelloFrame() {
        cachedHelloNonce = nil
        cachedHelloFrame = nil
    }

    private static func resolveLocalDeviceName() -> String {
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

    private func sendInventory(excludingPeerID: String?, onlyPeerID: String?) {
        let records = Array(mailbagEvents().prefix(200))
        guard !records.isEmpty else { return }
        for record in records {
            sendInventoryRecord(record, excludingPeerID: excludingPeerID, onlyPeerID: onlyPeerID)
        }
    }

    private func sendInventoryRecord(
        _ record: IrisNearbyStoredEvent,
        excludingPeerID: String?,
        onlyPeerID: String?
    ) {
        if let onlyPeerID,
           peerHasSeenEvent(record.id, peerID: onlyPeerID) || !markInventorySent(record.id, to: onlyPeerID) {
            return
        }
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
        sendEnvelope(envelope, excludingPeerID: excludingPeerID, onlyPeerID: onlyPeerID)
    }

    private func sendInventoryAfterHelloIfNeeded(remotePeerID: String, force: Bool) {
        let now = Date()
        if !force,
           let lastSent = peerInventorySentAt[remotePeerID],
           now.timeIntervalSince(lastSent) < Self.inventoryResendInterval {
            return
        }
        peerInventorySentAt[remotePeerID] = now
        sendInventory(excludingPeerID: nil, onlyPeerID: remotePeerID)
    }

    private func sendWant(_ ids: [String], excludingPeerID: String?, onlyPeerID: String?) {
        guard !ids.isEmpty else { return }
        for id in ids.prefix(64) {
            if let onlyPeerID, !markWantSent(id, to: onlyPeerID) {
                continue
            }
            sendEnvelope(
                [
                    "v": 1,
                    "type": "want",
                    "id": id
                ],
                excludingPeerID: excludingPeerID,
                onlyPeerID: onlyPeerID
            )
        }
    }

    private func sendEvent(_ record: IrisNearbyStoredEvent, excludingPeerID: String?, onlyPeerID: String?) {
        sendEventJson(record.eventJson, excludingPeerID: excludingPeerID, onlyPeerID: onlyPeerID)
    }

    private func sendEventJson(
        _ eventJson: String,
        excludingPeerID: String?,
        onlyPeerID: String?,
        allowBluetoothWhenLanPeer: Bool = false
    ) {
        let eventID = IrisNearbyStoredEvent.fromEventJson(eventJson)?.id
        let envelope: [String: Any] = [
            "v": 1,
            "type": "event",
            "event_json": eventJson
        ]
        encodeFrame(envelope) { [weak self] frame in
            guard let self, self.isNearbyActive else { return }
            if let frame, frame.count <= Self.singleFrameBytes {
                self.sendEncodedFrame(
                    frame,
                    excludingPeerID: excludingPeerID,
                    onlyPeerID: onlyPeerID,
                    allowBluetoothWhenLanPeer: allowBluetoothWhenLanPeer
                )
            } else {
                guard let record = IrisNearbyStoredEvent.fromEventJson(eventJson) else { return }
                self.sendEventFragments(record, excludingPeerID: excludingPeerID, onlyPeerID: onlyPeerID)
            }
            if let eventID, let onlyPeerID {
                self.markPeerSeenEvent(eventID, peerID: onlyPeerID)
            }
        }
    }

    private func sendPresence(remoteNonce: String) {
        let builder = buildPresenceEventJson
        let peerID = peerID
        let localNonce = localNonce
        let profileEventID = ownProfileEventID
        codecQueue.async { [weak self] in
            let eventJson = builder?(
                peerID,
                localNonce,
                remoteNonce,
                profileEventID
            ) ?? ""
            DispatchQueue.main.async {
                guard let self,
                      self.isNearbyActive,
                      !eventJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                else { return }
                self.sendEventJson(
                    eventJson,
                    excludingPeerID: nil,
                    onlyPeerID: nil,
                    allowBluetoothWhenLanPeer: true
                )
            }
        }
    }

    private func sendPresenceIfNeeded(
        remoteNonce: String,
        responseKey: String,
        force: Bool,
        resendInterval: TimeInterval = IrisNearbyService.presenceResendInterval
    ) {
        let key = "\(responseKey)|\(remoteNonce)"
        let now = Date()
        if !force,
           let lastSent = presenceSentAt[key],
           now.timeIntervalSince(lastSent) < resendInterval {
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

    private func sendEventFragments(_ record: IrisNearbyStoredEvent, excludingPeerID: String?, onlyPeerID: String?) {
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
                excludingPeerID: excludingPeerID,
                onlyPeerID: onlyPeerID
            )
        }
    }

    private func sendEnvelope(_ object: [String: Any], excludingPeerID: String?, onlyPeerID: String? = nil) {
        guard isNearbyActive else { return }
        encodeFrame(object) { [weak self] frame in
            guard let self, self.isNearbyActive, let frame else { return }
            self.sendEncodedFrame(
                frame,
                excludingPeerID: excludingPeerID,
                onlyPeerID: onlyPeerID
            )
        }
    }

    private func encodeFrame(_ object: [String: Any], completion: @escaping (Data?) -> Void) {
        guard JSONSerialization.isValidJSONObject(object) else {
            completion(nil)
            return
        }
        let encoder = self.encodeFrameJson
        codecQueue.async { [weak self] in
            let frame: Data?
            if let data = try? JSONSerialization.data(withJSONObject: object),
               let json = String(data: data, encoding: .utf8) {
                frame = encoder?(json)
            } else {
                frame = nil
            }
            DispatchQueue.main.async {
                guard self != nil else { return }
                completion(frame?.isEmpty == false ? frame : nil)
            }
        }
    }

    private func decodeFrameJson(_ frame: Data, completion: @escaping ([String: Any]?) -> Void) {
        let decoder = self.decodeFrame
        codecQueue.async { [weak self] in
            let object: [String: Any]?
            if let json = decoder?(frame),
               !json.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
               let data = json.data(using: .utf8) {
                object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            } else {
                object = nil
            }
            DispatchQueue.main.async {
                guard self != nil else { return }
                completion(object)
            }
        }
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

    private func sendEncodedFrame(
        _ frame: Data,
        excludingPeerID: String?,
        onlyPeerID: String?,
        allowBluetoothWhenLanPeer: Bool = false
    ) {
        if isLanVisible {
            lanService?.send(frame, excludingPeerID: excludingPeerID, onlyPeerID: onlyPeerID)
        }
        sendBluetoothFrame(
            frame,
            excludingPeerID: excludingPeerID,
            onlyPeerID: onlyPeerID,
            allowBluetoothWhenLanPeer: allowBluetoothWhenLanPeer
        )
    }

    private func sendBluetoothFrame(
        _ frame: Data,
        excludingPeerID: String?,
        onlyPeerID: String?,
        allowBluetoothWhenLanPeer: Bool
    ) {
        guard isVisible else { return }
        for (id, characteristic) in writableCharacteristics {
            if !shouldSendViaOutgoingBluetoothRoute(
                peripheralID: id,
                excludingPeerID: excludingPeerID,
                onlyPeerID: onlyPeerID,
                allowLanDuplicate: allowBluetoothWhenLanPeer
            ) {
                continue
            }
            guard let peripheral = peripherals[id] else { continue }
            write(frame, to: peripheral, characteristic: characteristic)
        }
        for (id, channel) in subscribedCentrals {
            if !shouldSendViaIncomingBluetoothRoute(
                centralID: id,
                excludingPeerID: excludingPeerID,
                onlyPeerID: onlyPeerID,
                allowLanDuplicate: allowBluetoothWhenLanPeer
            ) {
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
            irisDebugLog("Iris nearby: dropped stale Bluetooth write chunks \(droppedChunks)")
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
        decodeFrameJson(frame) { [weak self] envelope in
            self?.ingestDecodedFrame(envelope, source: source, sourceKey: sourceKey)
        }
    }

    private func ingestDecodedFrame(_ decodedEnvelope: [String: Any]?, source: IrisNearbySource, sourceKey: String) {
        guard var envelope = decodedEnvelope,
              let type = envelope["type"] as? String else { return }
        if envelope["peer_id"] != nil {
            rejectLegacyNearbySource(source, type: type)
            return
        }
        let remotePeerID = peerIDForSource(source)
        envelope["_source_key"] = sourceKey
        if let remotePeerID, !remotePeerID.isEmpty {
            touchPeer(remotePeerID)
            markTransportPeer(remotePeerID, source: source)
        }

        switch type {
        case "hello":
            let remoteNonce = sanitizedNonce(envelope["nonce"] as? String)
            let previousSourceNonce = connectionNonces[sourceKey]
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
                    sendHello(excludingPeerID: nil, force: true)
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
                sendPresenceIfNeeded(
                    remoteNonce: remoteNonce,
                    responseKey: sourceKey,
                    force: previousSourceNonce != remoteNonce,
                    resendInterval: Self.unverifiedPresenceResendInterval
                )
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
            let sender = envelopeRemotePeerID(envelope)
            guard let sender else { return }
            markPeerSeenEvent(id, peerID: sender)
            sendWant([id], excludingPeerID: nil, onlyPeerID: sender)
        }
    }

    private func handleWant(_ envelope: [String: Any]) {
        guard let id = envelope["id"] as? String,
              let record = ownOutbound[id] ?? forwarded[id] else { return }
        let requester = envelopeRemotePeerID(envelope)
        sendEvent(record, excludingPeerID: nil, onlyPeerID: requester)
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
                irisDebugLog("Iris nearby: accepted presence")
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
        // Ephemeral kinds are ingested locally (so the user's own
        // delivered/seen state updates when a peer's receipt reaches
        // them) but are never stored or re-broadcast — see
        // `mailbagSuppressedKinds`.
        guard Self.shouldStoreInMailbag(kind: record.kind) else {
            if let remotePeerID {
                markPeerSeenEvent(record.id, peerID: remotePeerID)
            }
            return
        }
        forwarded[record.id] = record
        pruneMailbags()
        if let remotePeerID {
            markPeerSeenEvent(record.id, peerID: remotePeerID)
        }
        sendInventoryRecord(record, excludingPeerID: remotePeerID, onlyPeerID: nil)
        irisDebugLog("Iris nearby: accepted event kind %u %@", record.kind, record.id)
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
            sendInventoryAfterHelloIfNeeded(remotePeerID: peerID, force: true)
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
            inventoryIDsSentByPeer.removeValue(forKey: peerID)
            wantIDsSentByPeer.removeValue(forKey: peerID)
            eventIDsSeenByPeer.removeValue(forKey: peerID)
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
        irisDebugLog("Iris nearby: expired stale peers \(stalePeerIDs.count)")
    }

    private func removeLanOnlyPeers(_ lanPeerIDs: Set<String>) {
        let bluetoothPeerIDs = recentBluetoothPeerIDs
        peers.removeAll { lanPeerIDs.contains($0.id) && !bluetoothPeerIDs.contains($0.id) }
        for peerID in lanPeerIDs where !bluetoothPeerIDs.contains(peerID) {
            peerNonces.removeValue(forKey: peerID)
            peerInventorySentAt.removeValue(forKey: peerID)
            inventoryIDsSentByPeer.removeValue(forKey: peerID)
            wantIDsSentByPeer.removeValue(forKey: peerID)
            eventIDsSeenByPeer.removeValue(forKey: peerID)
        }
        status = peers.isEmpty ? visibleIdleStatus : sidebarSubtitle
    }

    private func transportLabel(for remotePeerID: String?) -> String {
        guard let remotePeerID, !remotePeerID.isEmpty else { return "nearby" }
        let kind = recentBluetoothPeerIDs.contains(remotePeerID) ? "bluetooth" : "wifi"
        if let peerName = peers.first(where: { $0.id == remotePeerID })?.name,
           !peerName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "\(kind) · \(peerName)"
        }
        return kind
    }

    private var recentBluetoothPeerIDs: Set<String> {
        let cutoff = Date().addingTimeInterval(-Self.peerTTL)
        return Set(bluetoothPeerLastSeen.filter { $0.value >= cutoff }.keys)
    }

    private func shouldSendViaOutgoingBluetoothRoute(
        peripheralID: UUID,
        excludingPeerID: String?,
        onlyPeerID: String?,
        allowLanDuplicate: Bool
    ) -> Bool {
        guard let remotePeerID = peerIDByPeripheral[peripheralID] else {
            return onlyPeerID == nil
        }
        if let onlyPeerID, remotePeerID != onlyPeerID {
            return false
        }
        if let excludingPeerID, remotePeerID == excludingPeerID {
            return false
        }
        if !allowLanDuplicate, lanService?.hasPeer(remotePeerID) == true {
            return false
        }
        if hasOutgoingBluetoothRoute(remotePeerID), hasIncomingBluetoothRoute(remotePeerID) {
            return shouldUseOutgoingBluetoothRoute(remotePeerID)
        }
        return true
    }

    private func shouldSendViaIncomingBluetoothRoute(
        centralID: UUID,
        excludingPeerID: String?,
        onlyPeerID: String?,
        allowLanDuplicate: Bool
    ) -> Bool {
        guard let remotePeerID = peerIDByCentral[centralID] else {
            return onlyPeerID == nil
        }
        if let onlyPeerID, remotePeerID != onlyPeerID {
            return false
        }
        if let excludingPeerID, remotePeerID == excludingPeerID {
            return false
        }
        if !allowLanDuplicate, lanService?.hasPeer(remotePeerID) == true {
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

    private func envelopeRemotePeerID(_ envelope: [String: Any]) -> String? {
        let sourceKey = envelope["_source_key"] as? String
        return sourceKey.flatMap { peerIDForSourceKey($0) }
    }

    private func peerIDForSourceKey(_ sourceKey: String) -> String? {
        if sourceKey.hasPrefix(Self.bluetoothSourcePrefix) {
            let value = String(sourceKey.dropFirst(Self.bluetoothSourcePrefix.count))
            return UUID(uuidString: value).flatMap { peerIDByPeripheral[$0] }
        }
        if sourceKey.hasPrefix(Self.centralSourcePrefix) {
            let value = String(sourceKey.dropFirst(Self.centralSourcePrefix.count))
            return UUID(uuidString: value).flatMap { peerIDByCentral[$0] }
        }
        if sourceKey.hasPrefix(Self.lanSourcePrefix) {
            return lanService?.peerIDForConnection(String(sourceKey.dropFirst(Self.lanSourcePrefix.count)))
        }
        return nil
    }

    private func boundedInsert(_ id: String, in map: inout [String: RecentIDSet], peerID: String) -> Bool {
        var recent = map[peerID] ?? RecentIDSet()
        let inserted = recent.insert(id, limit: Self.maxPeerDedupEntries)
        map[peerID] = recent
        return inserted
    }

    private func markInventorySent(_ id: String, to peerID: String) -> Bool {
        boundedInsert(id, in: &inventoryIDsSentByPeer, peerID: peerID)
    }

    private func markWantSent(_ id: String, to peerID: String) -> Bool {
        boundedInsert(id, in: &wantIDsSentByPeer, peerID: peerID)
    }

    private func markPeerSeenEvent(_ id: String, peerID: String) {
        _ = boundedInsert(id, in: &eventIDsSeenByPeer, peerID: peerID)
    }

    private func peerHasSeenEvent(_ id: String, peerID: String) -> Bool {
        eventIDsSeenByPeer[peerID]?.contains(id) == true
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
        let now = Date()
        let sanitizedProfileEventID = sanitizedEventID(profileEventID)
        let existingIndex = peers.firstIndex(where: { $0.id == peerID })
        let existing = existingIndex.map { peers[$0] }
        let nextName = Self.nearbyPeerName(
            advertisedName: name,
            ownerPubkeyHex: existing?.ownerPubkeyHex,
            profileDisplayName: nil,
            existingName: existing?.name
        )
        let nextProfileEventID = sanitizedProfileEventID ?? existing?.profileEventID
        if let existingIndex, let existing {
            let changed = existing.name != nextName ||
                existing.profileEventID != nextProfileEventID
            if changed || now.timeIntervalSince(existing.lastSeen) >= Self.peerTouchPublishInterval {
                peers[existingIndex] = IrisNearbyPeer(
                    id: peerID,
                    name: nextName,
                    ownerPubkeyHex: existing.ownerPubkeyHex,
                    pictureURL: existing.pictureURL,
                    profileEventID: nextProfileEventID,
                    lastSeen: now
                )
                sortPeers()
                status = sidebarSubtitle
            }
        } else {
            peers.append(
                IrisNearbyPeer(
                    id: peerID,
                    name: nextName,
                    ownerPubkeyHex: nil,
                    pictureURL: nil,
                    profileEventID: nextProfileEventID,
                    lastSeen: now
                )
            )
            sortPeers()
            status = sidebarSubtitle
            irisDebugLog("Iris nearby: saw peer")
        }
        if let profileEventID = nextProfileEventID,
           let profile = knownProfiles[profileEventID] {
            applyAdvertisedProfile(profile, toPeerID: peerID)
        }
    }

    private func touchPeer(_ peerID: String) {
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        let now = Date()
        guard now.timeIntervalSince(peers[index].lastSeen) >= Self.peerTouchPublishInterval else {
            return
        }
        peers[index].lastSeen = now
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
        let now = Date()
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
                    lastSeen: now
                )
            )
        }
        guard let index = peers.firstIndex(where: { $0.id == peerID }) else { return }
        let nextProfileEventID = profileEventID ?? peers[index].profileEventID
        let nextName = Self.nearbyPeerName(
            advertisedName: nil,
            ownerPubkeyHex: ownerPubkeyHex,
            profileDisplayName: nil,
            existingName: peers[index].name
        )
        let changed = peers[index].ownerPubkeyHex != ownerPubkeyHex ||
            peers[index].profileEventID != nextProfileEventID ||
            peers[index].name != nextName
        if changed || now.timeIntervalSince(peers[index].lastSeen) >= Self.peerTouchPublishInterval {
            peers[index].ownerPubkeyHex = ownerPubkeyHex
            peers[index].profileEventID = nextProfileEventID
            peers[index].name = nextName
            peers[index].lastSeen = now
            sortPeers()
            status = sidebarSubtitle
        }
        if let nextProfileEventID, let profile = knownProfiles[nextProfileEventID] {
            applyAdvertisedProfile(profile, toPeerID: peerID)
        } else if let nextProfileEventID,
                  ownOutbound[nextProfileEventID] == nil,
                  forwarded[nextProfileEventID] == nil {
            sendWant([nextProfileEventID], excludingPeerID: nil, onlyPeerID: peerID)
        }
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
        let now = Date()
        let nextName = Self.nearbyPeerName(
            advertisedName: nil,
            ownerPubkeyHex: profile.ownerPubkeyHex,
            profileDisplayName: profile.displayName,
            existingName: peers[index].name
        )
        let nextPictureURL = profile.pictureURL ?? peers[index].pictureURL
        let changed = peers[index].ownerPubkeyHex != profile.ownerPubkeyHex ||
            peers[index].profileEventID != profile.id ||
            peers[index].name != nextName ||
            peers[index].pictureURL != nextPictureURL
        guard changed || now.timeIntervalSince(peers[index].lastSeen) >= Self.peerTouchPublishInterval else {
            return
        }
        peers[index].ownerPubkeyHex = profile.ownerPubkeyHex
        peers[index].profileEventID = profile.id
        peers[index].name = nextName
        peers[index].pictureURL = nextPictureURL
        peers[index].lastSeen = now
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
        irisDebugLog("Iris nearby: %@ state %@", label, state)
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
        setBluetoothAuthorization(CBManager.authorization)
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
            irisDebugLog("Iris nearby: write failed \(error.localizedDescription)")
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
            irisDebugLog("Iris nearby: notification setup failed \(error.localizedDescription)")
        } else {
            irisDebugLog("Iris nearby: notifications ready")
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
        setBluetoothAuthorization(CBManager.authorization)
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
