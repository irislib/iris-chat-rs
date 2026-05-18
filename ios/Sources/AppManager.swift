import Foundation
#if os(macOS)
import AppKit
#endif
#if os(iOS)
import Intents
#endif
import Security
import SwiftUI
#if os(iOS) || os(macOS)
import UserNotifications
#endif

#if os(macOS)
private let defaultIrisUpdateManifestUrl = URL(
    string: "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest/release.json"
)!
#endif
#if os(iOS)
private let appManagerPendingShareNotificationName = "to.iris.chat.pending-share"
#endif

struct StoredAccountBundle: Codable, Equatable {
    let ownerNsec: String?
    let ownerPubkeyHex: String
    let deviceNsec: String
}

struct StagedAttachment: Identifiable, Equatable {
    let id = UUID()
    let path: String
    let filename: String
}

struct PendingShareAttachment: Codable, Equatable {
    let path: String
    let filename: String
}

struct PendingShare: Codable, Identifiable, Equatable {
    let id: String
    let text: String
    let attachments: [PendingShareAttachment]
    let suggestedChatId: String?
    let suggestedChatIds: [String]?
    let autoSend: Bool?
    let isForward: Bool?

    var isForwarding: Bool {
        isForward == true
    }

    var suggestedTargetChatIds: [String] {
        var seen = Set<String>()
        var result = [String]()
        for raw in (suggestedChatIds ?? []) + [suggestedChatId].compactMap({ $0 }) {
            let chatId = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if chatId.isEmpty || seen.contains(chatId) {
                continue
            }
            seen.insert(chatId)
            result.append(chatId)
        }
        return result
    }

    var shouldAutoSend: Bool {
        autoSend == true
    }

    func withAutoSend(_ enabled: Bool) -> PendingShare {
        PendingShare(
            id: id,
            text: text,
            attachments: attachments,
            suggestedChatId: suggestedChatId,
            suggestedChatIds: suggestedChatIds,
            autoSend: autoSend == true || enabled,
            isForward: isForward
        )
    }
}

#if os(iOS)
private final class ShareSuggestionDonor {
    private let groupIdentifier = "iris-chat-share-suggestions"
    private var donatedIdentifiers = Set<String>()

    func syncRecentChats(_ chats: [ChatThreadSnapshot]) {
        chats
            .filter { $0.lastMessageAtSecs != nil }
            .sorted { ($0.lastMessageAtSecs ?? 0) > ($1.lastMessageAtSecs ?? 0) }
            .prefix(8)
            .forEach { chat in
                donate(chat: chat, timestampSecs: chat.lastMessageAtSecs)
            }
    }

    func donateSelectedChats(_ chats: [ChatThreadSnapshot]) {
        chats.forEach { chat in
            donate(chat: chat, timestampSecs: nil, force: true)
        }
    }

    private func donate(chat: ChatThreadSnapshot, timestampSecs: UInt64?, force: Bool = false) {
        let chatId = chat.chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !chatId.isEmpty else {
            return
        }
        let timestampKey = timestampSecs.map(String.init) ?? String(Int(Date().timeIntervalSince1970))
        let identifier = "share-\(chatId)-\(timestampKey)"
        guard force || !donatedIdentifiers.contains(identifier) else {
            return
        }
        donatedIdentifiers.insert(identifier)

        let displayName = chat.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = displayName.isEmpty ? "Chat" : displayName
        let recipient = INPerson(
            personHandle: INPersonHandle(value: chatId, type: .unknown),
            nameComponents: nil,
            displayName: title,
            image: nil,
            contactIdentifier: nil,
            customIdentifier: chatId,
            isContactSuggestion: false,
            suggestionType: .instantMessageAddress
        )
        let groupName = chat.kind == .group ? INSpeakableString(spokenPhrase: title) : nil
        let intent = INSendMessageIntent(
            recipients: chat.kind == .direct ? [recipient] : nil,
            outgoingMessageType: .outgoingMessageText,
            content: nil,
            speakableGroupName: groupName,
            conversationIdentifier: chatId,
            serviceName: "Iris Chat",
            sender: nil,
            attachments: nil
        )
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        interaction.identifier = identifier
        interaction.groupIdentifier = groupIdentifier
        if let timestampSecs {
            interaction.dateInterval = DateInterval(
                start: Date(timeIntervalSince1970: TimeInterval(timestampSecs)),
                duration: 1
            )
        }
        interaction.donate(completion: nil)
    }
}

private struct ShareSuggestionEntry: Codable {
    let chatId: String
    let displayName: String
    let subtitle: String?
    let pictureUrl: String?
    let isGroup: Bool
    let lastMessageAtSecs: UInt64?
}

private final class ShareSuggestionsExporter {
    private let appGroupIdentifier: String
    private let fileManager: FileManager
    private let queue = DispatchQueue(label: "to.iris.chat.share-suggestions", qos: .utility)
    private var lastWritten: Data?

    init(appGroupIdentifier: String, fileManager: FileManager = .default) {
        self.appGroupIdentifier = appGroupIdentifier
        self.fileManager = fileManager
    }

    func export(chats: [ChatThreadSnapshot]) {
        let entries = chats
            .sorted { ($0.lastMessageAtSecs ?? 0) > ($1.lastMessageAtSecs ?? 0) }
            .prefix(20)
            .map { chat in
                ShareSuggestionEntry(
                    chatId: chat.chatId,
                    displayName: chat.displayName,
                    subtitle: chat.subtitle,
                    pictureUrl: chat.pictureUrl,
                    isGroup: chat.kind == .group,
                    lastMessageAtSecs: chat.lastMessageAtSecs
                )
            }
        guard let data = try? JSONEncoder().encode(Array(entries)) else {
            return
        }
        if data == lastWritten {
            return
        }
        lastWritten = data
        let groupId = appGroupIdentifier
        let fm = fileManager
        queue.async {
            guard let dir = fm.containerURL(
                forSecurityApplicationGroupIdentifier: groupId
            ) else {
                return
            }
            let url = dir.appendingPathComponent("share-suggestions.json")
            try? data.write(to: url, options: .atomic)
        }
    }
}

struct IosMobilePushSyncInput: Equatable {
    let enabled: Bool
    let ownerPubkeyHex: String?
    let ownerSecretAvailable: Bool
    let messageAuthorPubkeys: [String]
    let inviteResponsePubkeys: [String]
    let mobilePushServerUrl: String

    init(state: AppState, ownerNsec: String?) {
        self.enabled = state.preferences.desktopNotificationsEnabled
        self.ownerPubkeyHex = nonEmptyTrimmedString(state.mobilePush.ownerPubkeyHex)
        self.ownerSecretAvailable = nonEmptyTrimmedString(ownerNsec) != nil
        self.messageAuthorPubkeys = state.mobilePush.messageAuthorPubkeys
        self.inviteResponsePubkeys = state.mobilePush.inviteResponsePubkeys
        self.mobilePushServerUrl = state.preferences.mobilePushServerUrl
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

private func nonEmptyTrimmedString(_ value: String?) -> String? {
    let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return trimmed.isEmpty ? nil : trimmed
}

struct IosStateSideEffectGate {
    private var lastShareChatList: [ChatThreadSnapshot]?
    private var lastMobilePushInput: IosMobilePushSyncInput?

    mutating func shouldSyncShareSuggestions(chatList: [ChatThreadSnapshot]) -> Bool {
        guard lastShareChatList != chatList else {
            return false
        }
        lastShareChatList = chatList
        return true
    }

    mutating func shouldSyncMobilePush(state: AppState, ownerNsec: String?) -> Bool {
        let input = IosMobilePushSyncInput(state: state, ownerNsec: ownerNsec)
        guard lastMobilePushInput != input else {
            return false
        }
        lastMobilePushInput = input
        return true
    }

    mutating func resetMobilePush() {
        lastMobilePushInput = nil
    }
}
#endif

private struct ClientDebugLogEntry: Sendable {
    let timestampSecs: UInt64
    let category: String
    let detail: String

    var jsonObject: [String: Any] {
        [
            "timestamp_secs": timestampSecs,
            "category": category,
            "detail": detail
        ]
    }
}

protocol AccountSecretStore {
    func load() -> StoredAccountBundle?
    func save(_ bundle: StoredAccountBundle)
    @discardableResult
    func clear() -> Bool
}

final class KeychainSecretStore: AccountSecretStore {
    private let service: String
    private let account: String
    private let accessGroup: String?
    private let accessibility: CFString?

#if os(iOS)
    private static let defaultAccessibility: CFString? = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
#else
    private static let defaultAccessibility: CFString? = nil
#endif

    init(
        service: String = "to.iris.chat",
        account: String = "stored-account-bundle",
        accessGroup: String? = nil,
        accessibility: CFString? = KeychainSecretStore.defaultAccessibility
    ) {
        self.service = service
        self.account = account
        self.accessGroup = accessGroup
        self.accessibility = accessibility
    }

    private func baseQuery() -> [CFString: Any] {
        var query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
        ]
        if let accessGroup, !accessGroup.isEmpty {
            // Most callers should omit this and let iOS use the first
            // keychain-access-groups entitlement. The app and notification
            // service extension share that default group.
            query[kSecAttrAccessGroup] = accessGroup
        }
        return query
    }

    func load() -> StoredAccountBundle? {
        var query = baseQuery()
        query[kSecReturnData] = true
        query[kSecMatchLimit] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }
        return try? JSONDecoder().decode(StoredAccountBundle.self, from: data)
    }

    func save(_ bundle: StoredAccountBundle) {
        guard let data = try? JSONEncoder().encode(bundle) else {
            return
        }
        let query = baseQuery()
        var update: [CFString: Any] = [kSecValueData: data]
        if let accessibility {
            update[kSecAttrAccessible] = accessibility
        }
        let updateStatus = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        guard updateStatus != errSecSuccess else {
            return
        }

        var insert = query
        insert[kSecValueData] = data
        if let accessibility {
            insert[kSecAttrAccessible] = accessibility
        }
        SecItemAdd(insert as CFDictionary, nil)
    }

    private func deletionQueries() -> [[CFString: Any]] {
        var queries = [baseQuery()]
        var synchronized = baseQuery()
        synchronized[kSecAttrSynchronizable] = kSecAttrSynchronizableAny
        queries.append(synchronized)
        return queries
    }

    @discardableResult
    func clear() -> Bool {
        var deletedAll = true
        for query in deletionQueries() {
            let status = SecItemDelete(query as CFDictionary)
            if status != errSecSuccess && status != errSecItemNotFound {
                deletedAll = false
                NSLog("Iris Chat keychain secret clear failed: status=%d service=%@ account=%@", status, service, account)
            }
        }
        if load() != nil {
            NSLog("Iris Chat keychain secret clear failed: item still readable service=%@ account=%@", service, account)
            return false
        }
        return deletedAll
    }
}

final class FileAccountSecretStore: AccountSecretStore {
    private let url: URL
    private let fileManager: FileManager

    init(url: URL, fileManager: FileManager = .default) {
        self.url = url
        self.fileManager = fileManager
    }

    func load() -> StoredAccountBundle? {
        guard let data = try? Data(contentsOf: url) else {
            return nil
        }
        return try? JSONDecoder().decode(StoredAccountBundle.self, from: data)
    }

    func save(_ bundle: StoredAccountBundle) {
        guard let data = try? JSONEncoder().encode(bundle) else {
            return
        }
        do {
            try fileManager.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try data.write(to: url, options: .atomic)
            try? fileManager.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: url.path
            )
        } catch {
            NSLog("Iris Chat file secret save failed: %@", "\(error)")
        }
    }

    @discardableResult
    func clear() -> Bool {
        do {
            try fileManager.removeItem(at: url)
        } catch let error as CocoaError where error.code == .fileNoSuchFile {
            return true
        } catch {
            NSLog("Iris Chat file secret clear failed: %@", "\(error)")
            return false
        }
        return load() == nil
    }
}

protocol RustAppClient: AnyObject {
    func state() -> AppState
    func dispatch(action: AppAction) throws
    func search(query: String, scopeChatId: String?, limit: UInt32) -> SearchResultSnapshot
    func chatSnapshot(chatId: String, limit: UInt32) -> CurrentChatSnapshot?
    func chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt32) -> CurrentChatSnapshot?
    func chatSnapshotAroundMessage(chatId: String, messageId: String, beforeLimit: UInt32, afterLimit: UInt32) -> CurrentChatSnapshot?
    func mutualGroups(ownerInput: String) -> [ChatThreadSnapshot]
    func ingestNearbyEventJson(eventJson: String) -> Bool
    func ingestNearbyEventJsonWithTransport(eventJson: String, transport: String) -> Bool
    func buildNearbyPresenceEventJson(peerID: String, myNonce: String, theirNonce: String, profileEventID: String) -> String
    func verifyNearbyPresenceEventJson(eventJson: String, peerID: String, myNonce: String, theirNonce: String) -> String
    func nearbyEncodeFrame(envelopeJson: String) -> Data
    func nearbyDecodeFrame(frame: Data) -> String
    func nearbyFrameBodyLenFromHeader(header: Data) -> Int
    func exportSupportBundleJson() -> String
    func peerProfileDebug(ownerInput: String) -> PeerProfileDebugSnapshot?
    func prepareForSuspend()
    func shutdown()
    func listenForUpdates(reconciler: AppReconciler)
}

final class LiveRustAppClient: RustAppClient {
    private let ffi: FfiApp

    init(dataDir: String, appVersion: String) {
        self.ffi = FfiApp(dataDir: dataDir, keychainGroup: "", appVersion: appVersion)
    }

    func state() -> AppState {
        ffi.stateSafely()
    }

    func dispatch(action: AppAction) throws {
        try ffi.dispatchSafely(action: action)
    }

    func search(query: String, scopeChatId: String?, limit: UInt32) -> SearchResultSnapshot {
        ffi.searchSafely(query: query, scopeChatId: scopeChatId, limit: limit)
    }

    func chatSnapshot(chatId: String, limit: UInt32) -> CurrentChatSnapshot? {
        ffi.chatSnapshotSafely(chatId: chatId, limit: limit)
    }

    func chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt32) -> CurrentChatSnapshot? {
        ffi.chatSnapshotBeforeSafely(chatId: chatId, beforeMessageId: beforeMessageId, limit: limit)
    }

    func chatSnapshotAroundMessage(chatId: String, messageId: String, beforeLimit: UInt32, afterLimit: UInt32) -> CurrentChatSnapshot? {
        ffi.chatSnapshotAroundMessageSafely(
            chatId: chatId,
            messageId: messageId,
            beforeLimit: beforeLimit,
            afterLimit: afterLimit
        )
    }

    func mutualGroups(ownerInput: String) -> [ChatThreadSnapshot] {
        ffi.mutualGroupsSafely(ownerInput: ownerInput)
    }

    func ingestNearbyEventJson(eventJson: String) -> Bool {
        ffi.ingestNearbyEventJsonSafely(eventJson: eventJson)
    }

    func ingestNearbyEventJsonWithTransport(eventJson: String, transport: String) -> Bool {
        ffi.ingestNearbyEventJsonWithTransportSafely(eventJson: eventJson, transport: transport)
    }

    func buildNearbyPresenceEventJson(peerID: String, myNonce: String, theirNonce: String, profileEventID: String) -> String {
        ffi.buildNearbyPresenceEventJsonSafely(
            peerID: peerID,
            myNonce: myNonce,
            theirNonce: theirNonce,
            profileEventID: profileEventID
        )
    }

    func verifyNearbyPresenceEventJson(eventJson: String, peerID: String, myNonce: String, theirNonce: String) -> String {
        ffi.verifyNearbyPresenceEventJsonSafely(
            eventJson: eventJson,
            peerID: peerID,
            myNonce: myNonce,
            theirNonce: theirNonce
        )
    }

    func nearbyEncodeFrame(envelopeJson: String) -> Data {
        ffi.nearbyEncodeFrameSafely(envelopeJson: envelopeJson)
    }

    func nearbyDecodeFrame(frame: Data) -> String {
        ffi.nearbyDecodeFrameSafely(frame: frame)
    }

    func nearbyFrameBodyLenFromHeader(header: Data) -> Int {
        ffi.nearbyFrameBodyLenFromHeaderSafely(header: header)
    }

    func exportSupportBundleJson() -> String {
        ffi.exportSupportBundleJsonSafely()
    }

    func peerProfileDebug(ownerInput: String) -> PeerProfileDebugSnapshot? {
        ffi.peerProfileDebugSafely(ownerInput: ownerInput)
    }

    func prepareForSuspend() {
        ffi.prepareForSuspendSafely()
    }

    func shutdown() {
        ffi.shutdownSafely()
    }

    func listenForUpdates(reconciler: AppReconciler) {
        ffi.listenForUpdatesSafely(reconciler: reconciler)
    }
}

private final class SuspendPreparationRunner: @unchecked Sendable {
    private let rust: RustAppClient

    init(rust: RustAppClient) {
        self.rust = rust
    }

    func prepareForSuspend() {
        rust.prepareForSuspend()
    }
}

private final class RustDispatchRunner: @unchecked Sendable {
    private let rust: RustAppClient

    init(rust: RustAppClient) {
        self.rust = rust
    }

    func dispatch(action: AppAction) throws {
        try rust.dispatch(action: action)
    }
}

private final class SupportBundleExportRunner: @unchecked Sendable {
    private let rust: RustAppClient

    init(rust: RustAppClient) {
        self.rust = rust
    }

    func export() -> String {
        rust.exportSupportBundleJson()
    }
}

private final class ChatPageLoadRunner: @unchecked Sendable {
    private let rust: RustAppClient

    init(rust: RustAppClient) {
        self.rust = rust
    }

    func latest(chatId: String, limit: UInt32) -> CurrentChatSnapshot? {
        rust.chatSnapshot(chatId: chatId, limit: limit)
    }

    func before(chatId: String, beforeMessageId: String, limit: UInt32) -> CurrentChatSnapshot? {
        rust.chatSnapshotBefore(chatId: chatId, beforeMessageId: beforeMessageId, limit: limit)
    }

    func around(
        chatId: String,
        messageId: String,
        beforeLimit: UInt32,
        afterLimit: UInt32
    ) -> CurrentChatSnapshot? {
        rust.chatSnapshotAroundMessage(
            chatId: chatId,
            messageId: messageId,
            beforeLimit: beforeLimit,
            afterLimit: afterLimit
        )
    }
}

/// Trampoline so `AppPaths.appVersion(bundle:)` can call the FFI
/// `appVersion()` free function without name-resolving back to itself.
private func irisCoreAppVersion() -> String {
    appVersion()
}

enum AppPaths {
    static let appGroupIdentifier = "group.to.iris.chat"

    static func appVersion(bundle: Bundle = .main) -> String {
        // CFBundleShortVersionString gets stripped to 3 parts before reaching
        // Apple, so reading it alone makes the update comparator think
        // 2026.5.10.1 is newer than the running 2026.5.10. Reconstruct the
        // optional 4th .build segment from CFBundleVersion (= the integer
        // IRIS_APP_VERSION_CODE = major*10000 + minor*1000 + patch*100 + build);
        // its last two digits are the build segment.
        let short = (bundle.infoDictionary?["CFBundleShortVersionString"] as? String).flatMap {
            $0.isEmpty ? nil : $0
        }
        if let short {
            if let buildString = bundle.infoDictionary?["CFBundleVersion"] as? String,
               let code = Int(buildString) {
                let buildSegment = code % 100
                if buildSegment > 0 {
                    return "\(short).\(buildSegment)"
                }
            }
            return short
        }
        // Local dev builds skip release.env, so MARKETING_VERSION
        // substitutes to empty and the bundle plist carries no version.
        // Use the Rust crate's compiled-in version (build.rs falls back
        // to CARGO_PKG_VERSION) so the About panel still shows a real
        // number instead of a stale hardcoded fallback.
        return irisCoreAppVersion()
    }

    static func testRunId(environment: [String: String]) -> String? {
        if let runId = environment["IRIS_UI_TEST_RUN_ID"], !runId.isEmpty {
            return runId
        }
        if let sessionId = environment["XCTestSessionIdentifier"], !sessionId.isEmpty {
            return "xctest-\(sessionId)"
        }
        if let bundlePath = environment["XCTestBundlePath"], !bundlePath.isEmpty {
            return "xctest"
        }
        return nil
    }

    static func notificationsDisabledForAutomation(environment: [String: String]) -> Bool {
        environment["IRIS_DISABLE_NOTIFICATIONS"] == "1" || testRunId(environment: environment) != nil
    }

    static func keychainService(environment: [String: String]) -> String {
        let base = "to.iris.chat"
        guard let runId = testRunId(environment: environment) else {
            return base
        }
        return "\(base).\(runId)"
    }

    static func secretStore(
        dataDir: URL,
        fileManager: FileManager,
        environment: [String: String]
    ) -> AccountSecretStore {
        // On the iOS simulator, Keychain items written through
        // `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` do not
        // reliably survive a terminate+relaunch — the second launch
        // sees `errSecItemNotFound`, `restorePersistedSession()` skips
        // the bundle dispatch, and the app falls back to the welcome
        // screen. UI tests already opt in to the file-based store via
        // `IRIS_UI_TEST_BYPASS_KEYCHAIN`; honour it on every platform
        // so the relaunch flow is exercised the same way it is on
        // macOS.
        if testRunId(environment: environment) != nil || environment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] == "1" {
            return FileAccountSecretStore(
                url: dataDir.appendingPathComponent("account-secret.json"),
                fileManager: fileManager
            )
        }
        return KeychainSecretStore(service: keychainService(environment: environment))
    }

    static func dataDir(fileManager: FileManager, environment: [String: String]) -> URL {
        let suffix = testRunId(environment: environment) ?? "iris-chat"
        let legacyBase = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let legacy = legacyBase.appendingPathComponent(suffix, isDirectory: true)
        #if os(iOS)
            // Prefer the App Group container so the Notification
            // Service Extension reads the *same* persisted ratchet
            // state. Older installs lived in the per-app
            // `applicationSupportDirectory`, so on first launch with
            // this version migrate the legacy tree into the shared
            // container.
            if let shared = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier) {
                let target = shared.appendingPathComponent(suffix, isDirectory: true)
                migrateLegacyDataDir(from: legacy, to: target, fileManager: fileManager)
                return target
            }
        #endif
        // macOS has no Notification Service Extension, so the App
        // Group adds nothing and triggers a "would like to access
        // data from other apps" privacy prompt at first launch.
        // Stay in `Application Support`.
        return legacy
    }

    private static func migrateLegacyDataDir(
        from legacy: URL,
        to target: URL,
        fileManager: FileManager
    ) {
        let targetExists = fileManager.fileExists(atPath: target.path)
        let legacyExists = fileManager.fileExists(atPath: legacy.path)
        guard legacyExists, !targetExists else {
            return
        }
        do {
            try fileManager.createDirectory(
                at: target.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try fileManager.moveItem(at: legacy, to: target)
        } catch {
            // Best effort. If the move fails the user just appears
            // logged out and re-logs in; their key never left the
            // device.
        }
    }

    static func prepareDataDirForBackgroundNotificationReads(
        _ dataDir: URL,
        fileManager: FileManager
    ) {
#if os(iOS)
        setBackgroundReadableProtection(at: dataDir, fileManager: fileManager)
        guard let enumerator = fileManager.enumerator(
            at: dataDir,
            includingPropertiesForKeys: [.isRegularFileKey, .isDirectoryKey],
            options: []
        ) else {
            return
        }
        for case let url as URL in enumerator {
            setBackgroundReadableProtection(at: url, fileManager: fileManager)
        }
#endif
    }

#if os(iOS)
    private static func setBackgroundReadableProtection(at url: URL, fileManager: FileManager) {
        try? fileManager.setAttributes(
            [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
            ofItemAtPath: url.path
        )
    }
#endif
}

enum LaunchRecoveryDefaults {
    static let pendingKey = "launchRecovery.pending"
    static let launchIDKey = "launchRecovery.launchID"
    static let versionKey = "launchRecovery.version"
    static let startedAtKey = "launchRecovery.startedAt"
    static let disabledVersionKey = "launchRecovery.disabledVersion"

    static func clear(userDefaults: UserDefaults) {
        userDefaults.removeObject(forKey: pendingKey)
        userDefaults.removeObject(forKey: launchIDKey)
        userDefaults.removeObject(forKey: versionKey)
        userDefaults.removeObject(forKey: startedAtKey)
        userDefaults.removeObject(forKey: disabledVersionKey)
    }
}

@MainActor
final class ToastCenter: ObservableObject {
    @Published var message: String?
    private var clearTask: Task<Void, Never>?

    func show(_ text: String, duration: TimeInterval = 3) {
        message = text
        clearTask?.cancel()
        let pending = text
        clearTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(duration * 1_000_000_000))
            guard let self, self.message == pending else { return }
            self.message = nil
        }
    }
}

#if os(macOS)
@MainActor
final class DesktopUpdateController: ObservableObject {
    @Published private(set) var checking = false
    @Published private(set) var installing = false
    @Published private(set) var available = false
    @Published private(set) var version = ""
    @Published private(set) var status = ""
    @Published var autoCheck: Bool = UserDefaults.standard.object(forKey: "updates.autoCheck") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(autoCheck, forKey: "updates.autoCheck")
            if autoCheck {
                startAutomaticChecks()
            } else {
                stopAutomaticChecks()
            }
        }
    }
    @Published var autoInstall: Bool = UserDefaults.standard.bool(forKey: "updates.autoInstall") {
        didSet {
            UserDefaults.standard.set(autoInstall, forKey: "updates.autoInstall")
            if autoInstall, canInstall {
                install()
            }
        }
    }

    private let manifestUrl: URL
    private let currentVersion: () -> String
    private var assetUrl: URL?
    private var task: Task<Void, Never>?
    private var automaticCheckTask: Task<Void, Never>?
    private var startupCheckDone = false

    init(manifestUrl: URL, currentVersion: @escaping () -> String) {
        self.manifestUrl = manifestUrl
        self.currentVersion = currentVersion
    }

    deinit {
        automaticCheckTask?.cancel()
        task?.cancel()
    }

    var canInstall: Bool {
        available && assetUrl != nil && !checking && !installing
    }

    func runStartupCheckIfNeeded() {
        startAutomaticChecks()
    }

    func startAutomaticChecks() {
        guard autoCheck else {
            stopAutomaticChecks()
            return
        }
        if !startupCheckDone {
            startupCheckDone = true
            check(manual: false)
        }
        guard automaticCheckTask == nil else { return }
        automaticCheckTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.automaticCheckIntervalNanoseconds)
                guard let self, !Task.isCancelled else { return }
                guard self.autoCheck else {
                    self.stopAutomaticChecks()
                    return
                }
                self.check(manual: false)
            }
        }
    }

    func stopAutomaticChecks() {
        automaticCheckTask?.cancel()
        automaticCheckTask = nil
    }

    func check(manual: Bool = true) {
        guard !checking else { return }
        task?.cancel()
        checking = true
        if manual {
            status = "Checking for updates"
        }
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let result = try await self.fetch()
                await MainActor.run {
                    self.apply(result, manual: manual)
                }
            } catch {
                await MainActor.run {
                    self.checking = false
                    if manual {
                        self.status = error.localizedDescription
                    }
                }
            }
        }
    }

    private static var automaticCheckIntervalNanoseconds: UInt64 {
        if let raw = ProcessInfo.processInfo.environment["IRIS_UPDATE_POLL_SECONDS"],
           let seconds = Double(raw),
           seconds > 0 {
            return UInt64(seconds * 1_000_000_000)
        }
        return 6 * 60 * 60 * 1_000_000_000
    }

    func install() {
        guard let assetUrl else {
            status = "No macOS update found"
            return
        }
        guard !installing else { return }
        installing = true
        status = "Downloading \(version)"
        Task { [weak self] in
            guard let self else { return }
            do {
                let savedUrl = try await self.download(from: assetUrl)
                try await MainActor.run {
                    try self.installDownloaded(savedUrl)
                }
            } catch {
                await MainActor.run {
                    self.installing = false
                    self.status = error.localizedDescription
                }
            }
        }
    }

    private func fetch() async throws -> IrisUpdateCheck {
        let data = try await loadIrisUpdateData(from: manifestUrl)
        let manifest = try JSONDecoder().decode(IrisReleaseManifest.self, from: data)
        let asset = manifest.preferredMacAsset()
        let url = asset.flatMap { URL(string: $0.path, relativeTo: manifestUrl)?.absoluteURL }
        return IrisUpdateCheck(
            manifest: manifest,
            asset: asset,
            assetUrl: url,
            isNewer: irisVersionIsNewer(manifest.tag, than: currentVersion())
        )
    }

    private func apply(_ check: IrisUpdateCheck, manual: Bool) {
        checking = false
        available = check.isNewer
        version = check.manifest.tag
        assetUrl = check.isNewer ? check.assetUrl : nil
        if check.isNewer {
            status = check.assetUrl == nil
                ? "Update \(check.manifest.tag) found without a macOS app"
                : "Update \(check.manifest.tag) available"
            if autoInstall, check.assetUrl != nil {
                install()
            }
        } else if manual {
            status = "Up to date"
        } else {
            status = ""
        }
    }

    private func download(from url: URL) async throws -> URL {
        let downloadedUrl: URL
        if url.isFileURL {
            downloadedUrl = FileManager.default.temporaryDirectory
                .appendingPathComponent("iris-chat-update-download-\(UUID().uuidString)")
            try FileManager.default.copyItem(at: url, to: downloadedUrl)
        } else {
            (downloadedUrl, _) = try await URLSession.shared.download(from: url)
        }
        return try moveIrisDownloadedUpdate(downloadedUrl, from: url)
    }

    private func installDownloaded(_ archiveUrl: URL) throws {
        status = "Installing \(version)"
        if archiveUrl.lastPathComponent.hasSuffix(".app.tar.gz") {
            let unpackDir = FileManager.default.temporaryDirectory
                .appendingPathComponent("IrisChatUpdate-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: unpackDir, withIntermediateDirectories: true)
            try runIrisUpdateProcess("/usr/bin/tar", arguments: ["-xzf", archiveUrl.path, "-C", unpackDir.path])
            guard let newApp = findIrisAppBundle(in: unpackDir) else {
                throw IrisUpdateError.missingAppBundle
            }
            let script = try irisUpdateInstallScript()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/sh")
            process.arguments = [script.path, Bundle.main.bundleURL.path, newApp.path]
            try process.run()
            NSApp.terminate(nil)
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([archiveUrl])
            installing = false
            status = "Downloaded \(archiveUrl.lastPathComponent)"
        }
    }
}
#endif

@MainActor
final class AppManager: ObservableObject {
    private static let downloadedAttachmentCacheLimitBytes = 128 * 1024 * 1024
    private static let activeChatSeenIdleLimit: TimeInterval = 5 * 60
    private static let maxClientDebugLogEntries = 50
    private static let dispatchFailureToast = "Action failed. Copy support bundle in Settings."
    private static let navigationOverrideTTL: TimeInterval = 10
    private static let chatPageSize: UInt32 = 80
    private static let chatAroundBeforeLimit: UInt32 = 40
    private static let chatAroundAfterLimit: UInt32 = 40
    private static let chatSnapshotCacheLimit = 12
    private static let chatPageQueue = DispatchQueue(label: "to.iris.chat.chat-pages", qos: .userInitiated)
    private static let optimisticNavigationDispatchQueue = DispatchQueue(
        label: "to.iris.chat.optimistic-navigation-dispatch",
        qos: .userInitiated
    )
    private static let supportBundleQueue = DispatchQueue(label: "to.iris.chat.support-bundle", qos: .utility)
    private static let nearbyLanPermissionPromptAttemptedKey = "nearbyLanPermissionPromptAttempted"
    private static let nearbyLanPermissionGrantedKey = "nearbyLanPermissionGranted"

    @Published private(set) var state: AppState
    @Published private(set) var bootstrapInFlight = true
    @Published private(set) var pendingShare: PendingShare?
    @Published private(set) var lastForegroundedAt = Date()
    @Published private(set) var appSceneIsActive = true
    @Published private(set) var lastUserActivityAt = Date()
    /// Set when the user taps a hit in the search bar's Messages
    /// section — ChatScreen reads it on appear, scrolls the timeline
    /// to that message id, then clears via `consumePendingScroll()`.
    /// Stays nil for normal `openChat` taps so we don't re-scroll on
    /// every chat-open.
    @Published private(set) var pendingScrollMessageId: String?

    private var olderChatPageLoads = Set<String>()
    private var exhaustedOlderChatPages = Set<String>()
    private var aroundChatPageLoads = Set<String>()
    private var initialChatPageLoads = Set<String>()
    private var chatSnapshotCache: [String: CurrentChatSnapshot] = [:]
    private var chatSnapshotCacheOrder: [String] = []
    private var lastSyncedDeviceLabelsKey: String?

    // Keeps user-driven navigation stable while queued Rust snapshots catch up.
    private struct PendingNavigationOverride {
        let stack: [Screen]
        let expiresAt: Date
    }

    // Domain-scoped sub-controllers — split out of the previous fat
    // ObservableObject so views that only care about toasts or the desktop
    // updater don't re-render on every relay event that publishes `state`.
    let toasts = ToastCenter()
#if os(macOS)
    let updates: DesktopUpdateController
#endif

    private var rust: RustAppClient
    private let makeRustClient: () -> RustAppClient
    private let secretStore: AccountSecretStore
    private let desktopNotifications: DesktopNotificationPosting
    private let dataDir: URL
    private let fileManager: FileManager
    private let sharedContainerOverride: URL?
#if os(macOS)
    private let currentAppVersion: String
#endif
#if os(macOS)
    let nearbyBitchat = MacBitchatNearbyService()
#endif
#if os(iOS) || os(macOS)
    let nearbyIris = IrisNearbyService()
    private var nearbyLanPermissionPromptAttempted = false
    private var nearbyLanPermissionGranted = false
#endif
#if os(iOS)
    private let mobilePushRuntime = MobilePushRuntime()
    private let shareSuggestionDonor = ShareSuggestionDonor()
    private let shareSuggestionsExporter = ShareSuggestionsExporter(
        appGroupIdentifier: AppPaths.appGroupIdentifier
    )
    private var iosSideEffectGate = IosStateSideEffectGate()
    private let isUiTestRun: Bool
#endif
    private var clientDebugLog: [ClientDebugLogEntry] = []
    private var lastRevApplied: UInt64
    private var pendingNavigationOverride: PendingNavigationOverride?
    private var pendingSharePayloadURLs: [String: URL] = [:]
    private var backgroundSuspendPrepared = false
    private var storedAccountBundle: StoredAccountBundle?
    private var persistedRestoreInFlight = false
    private var nearbySettingsWasOpened = false
    // UI-test escape hatch: when IRIS_UI_TEST_SEED_PEER + IRIS_UI_TEST_SEED_COUNT
    // are set, AppManager auto-creates a chat with that peer once the account
    // is ready, then dispatches `count` outgoing messages back-to-back. Lets
    // tests build a long-chat scenario in milliseconds instead of paying the
    // ~15s/message tax of XCUITest's typeText loop.
    private struct PendingTestSeed {
        let peer: String
        let count: Int
        let daySplitIndex: Int?
    }
    private var pendingTestSeed: PendingTestSeed?
    private var seedTestMessagesDispatched = false
    private var uiTestSeedCount: Int?
    private var uiTestSeedDaySplitIndex: Int?
#if os(iOS)
    private let screenshotFixture: ScreenshotFixture?
    private let screenshotFixtureReferenceDate: Date
#endif
    private lazy var reconciler = UpdateBridge(owner: self)
    init(
        rust: RustAppClient? = nil,
        secretStore: AccountSecretStore? = nil,
        desktopNotifications: DesktopNotificationPosting? = nil,
        dataDir: URL? = nil,
        fileManager: FileManager = .default,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        appVersion: String = AppPaths.appVersion(),
        rustFactory: (() -> RustAppClient)? = nil
    ) {
        self.fileManager = fileManager
        self.sharedContainerOverride = environment["IRIS_SHARE_CONTAINER_DIR"]
            .flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0, isDirectory: true) }
        let resolvedDataDir = dataDir ?? AppPaths.dataDir(fileManager: fileManager, environment: environment)
        let resolvedSecretStore = secretStore ?? AppPaths.secretStore(
            dataDir: resolvedDataDir,
            fileManager: fileManager,
            environment: environment
        )

        if environment["IRIS_UI_TEST_RESET"] == "1" {
            resolvedSecretStore.clear()
            try? fileManager.removeItem(at: resolvedDataDir)
        }
        if let peer = environment["IRIS_UI_TEST_SEED_PEER"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !peer.isEmpty,
           let count = environment["IRIS_UI_TEST_SEED_COUNT"].flatMap(Int.init),
           count > 0 {
            let daySplitIndex = environment["IRIS_UI_TEST_SEED_DAY_SPLIT_INDEX"].flatMap(Int.init)
            self.uiTestSeedCount = count
            self.uiTestSeedDaySplitIndex = daySplitIndex
            self.pendingTestSeed = PendingTestSeed(peer: peer, count: count, daySplitIndex: daySplitIndex)
        }
#if os(iOS)
        self.screenshotFixture = ScreenshotFixture.enabled(environment: environment) ? .default : nil
        self.screenshotFixtureReferenceDate = Date()
#endif
        try? fileManager.createDirectory(at: resolvedDataDir, withIntermediateDirectories: true)
        AppPaths.prepareDataDirForBackgroundNotificationReads(resolvedDataDir, fileManager: fileManager)

        LaunchRecoveryDefaults.clear(userDefaults: .standard)
        let resolvedRustFactory: () -> RustAppClient
        if let rustFactory {
            resolvedRustFactory = rustFactory
        } else if let rust {
            resolvedRustFactory = { rust }
        } else {
            resolvedRustFactory = {
                LiveRustAppClient(dataDir: resolvedDataDir.path, appVersion: appVersion)
            }
        }
        let resolvedRust = rust ?? resolvedRustFactory()
        let initialState = resolvedRust.state()

        self.rust = resolvedRust
        self.makeRustClient = resolvedRustFactory
        self.secretStore = resolvedSecretStore
#if os(iOS)
        self.isUiTestRun = AppPaths.testRunId(environment: environment) != nil
        self.desktopNotifications = desktopNotifications ?? NoopDesktopNotificationPoster()
#else
        self.desktopNotifications = desktopNotifications ?? SystemDesktopNotificationPoster(environment: environment)
#endif
        self.dataDir = resolvedDataDir
#if os(macOS)
        self.currentAppVersion = appVersion
        let manifestUrl = environment["IRIS_UPDATE_MANIFEST_URL"]
            .flatMap(URL.init(string:))
            ?? defaultIrisUpdateManifestUrl
        let resolvedAppVersion = appVersion
        self.updates = DesktopUpdateController(
            manifestUrl: manifestUrl,
            currentVersion: { resolvedAppVersion }
        )
#endif
        self.state = initialState
        irisSetDebugLoggingEnabled(initialState.preferences.debugLoggingEnabled)
        self.lastRevApplied = initialState.rev
#if os(iOS) || os(macOS)
        self.nearbyLanPermissionPromptAttempted = UserDefaults.standard.bool(
            forKey: Self.nearbyLanPermissionPromptAttemptedKey
        )
        self.nearbyLanPermissionGranted = UserDefaults.standard.bool(
            forKey: Self.nearbyLanPermissionGrantedKey
        )
#endif

        resolvedRust.listenForUpdates(reconciler: reconciler)
        syncCurrentDeviceLabelsIfNeeded(state: initialState)
        if AppPaths.testRunId(environment: environment) == nil {
            syncStartupAtLoginPreference(initialState.preferences.startupAtLoginEnabled)
        }
#if os(iOS)
        syncShareSuggestionsIfNeeded(chatList: initialState.chatList)
#endif
#if os(iOS)
        registerPendingShareObserver()
        processPendingShareFilesIfNeeded()
#endif

#if os(iOS) || os(macOS)
        nearbyIris.ingestEventJson = { [weak self] eventJson, transport in
            self?.rust.ingestNearbyEventJsonWithTransport(eventJson: eventJson, transport: transport) ?? false
        }
        nearbyIris.buildPresenceEventJson = { [weak self] peerID, myNonce, theirNonce, profileEventID in
            self?.rust.buildNearbyPresenceEventJson(
                peerID: peerID,
                myNonce: myNonce,
                theirNonce: theirNonce,
                profileEventID: profileEventID ?? ""
            ) ?? ""
        }
        nearbyIris.verifyPresenceEventJson = { [weak self] eventJson, peerID, myNonce, theirNonce in
            self?.rust.verifyNearbyPresenceEventJson(
                eventJson: eventJson,
                peerID: peerID,
                myNonce: myNonce,
                theirNonce: theirNonce
            ) ?? ""
        }
        nearbyIris.encodeFrameJson = { [weak self] envelopeJson in
            guard let self else { return nil }
            let frame = self.rust.nearbyEncodeFrame(envelopeJson: envelopeJson)
            return frame.isEmpty ? nil : frame
        }
        nearbyIris.decodeFrame = { [weak self] frame in
            self?.rust.nearbyDecodeFrame(frame: frame) ?? ""
        }
        nearbyIris.frameBodyLength = { [weak self] header in
            self?.rust.nearbyFrameBodyLenFromHeader(header: header) ?? -1
        }
        nearbyIris.onBluetoothPermissionDenied = { [weak self] in
            self?.handleNearbyBluetoothPermissionDenied()
        }
        nearbyIris.onLanPermissionDenied = { [weak self] in
            self?.handleNearbyLanPermissionDenied()
        }
        nearbyIris.onLanPermissionGranted = { [weak self] in
            self?.markNearbyLanPermissionGranted()
        }
        nearbyIris.isMailbagEnabled = { [weak self] in
            self?.state.preferences.nearbyMailbagEnabled ?? true
        }
        if initialState.preferences.nearbyBluetoothEnabled, nearbyIris.bluetoothPermissionGranted {
            nearbyIris.setVisible(true)
        }
        if initialState.preferences.nearbyLanEnabled, canAutoStartNearbyLan {
            nearbyIris.setLanVisible(true)
        }
#endif

        Task {
            restorePersistedSession()
        }
        Task {
            // Safety net: the loading overlay must never be permanently
            // stuck. The earlier version of this guard bailed when
            // `persistedRestoreInFlight` was true, which meant a Rust
            // restore that never published a follow-up FullState (panic in
            // start_session, listener thread not yet attached, etc.) left
            // the app frozen on the splash forever. Always clear after the
            // timeout — if Rust later catches up, the next FullState lands
            // normally; persistedRestoreInFlight is reset here so
            // settleBootstrapIfNeeded won't re-flip the overlay.
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            guard bootstrapInFlight else {
                return
            }
            appendClientDebugLog(
                category: "bootstrap.timeout",
                detail: "persistedRestoreInFlight=\(persistedRestoreInFlight)"
            )
            persistedRestoreInFlight = false
            bootstrapInFlight = false
        }

#if os(macOS)
#if DEBUG
        nearbyBitchat.configureDebugMessageToSendOnFirstPeer(environment["IRIS_BITCHAT_NEARBY_TEST_MESSAGE"])
        if environment["IRIS_BITCHAT_NEARBY_AUTOSTART"] == "1" {
            nearbyBitchat.setVisible(true)
        }
#endif
#endif
#if os(iOS) || os(macOS)
#if DEBUG
        if environment["IRIS_NEARBY_AUTOSTART"] == "1" {
            nearbyIris.setVisible(true)
        }
#endif
#endif
#if os(iOS)
        if let fixture = screenshotFixture {
            // Env-only escape hatch: makes the first fixture nearby
            // peer tappable for the e2e test that exercises
            // tap-peer → chat navigation. Unset for the regular
            // screenshot path so the modal still renders with every
            // peer disabled.
            let firstPeerOwnerHex = ProcessInfo.processInfo
                .environment["IRIS_UI_TEST_NEARBY_TAPPABLE_FIRST_PEER_HEX"]
            nearbyIris.applyScreenshotFixturePeers(
                peers: fixture.nearbyPeerSnapshots(firstPeerOwnerHex: firstPeerOwnerHex),
                bluetoothPeerIDs: fixture.nearbyBluetoothPeerIDs(),
                lanPeerIDs: fixture.nearbyLanPeerIDs()
            )
        }
#endif
    }

    var activeScreen: Screen {
        state.router.screenStack.last ?? state.router.defaultScreen
    }

    var canNavigateBack: Bool {
        !state.router.screenStack.isEmpty
    }

    func isUserBlocked(_ userId: String) -> Bool {
        let normalized = Self.normalizedBlockedUserId(userId)
        guard !normalized.isEmpty else { return false }
        return state.preferences.blockedOwnerPubkeys.contains(normalized)
    }

    func setUserBlocked(_ userId: String, blocked: Bool) {
        let normalized = Self.normalizedBlockedUserId(userId)
        guard !normalized.isEmpty else { return }
        // Block list lives in the Rust core now (Signal-style): a
        // single source of truth that also drives the nostr + push
        // subscription filters and the local-ingest guard. The UI
        // toast still fires here so the user sees immediate feedback
        // before the state round-trip lands.
        dispatch(.setUserBlocked(ownerPubkeyHex: normalized, blocked: blocked))
        showToast(blocked ? "User blocked" : "User unblocked")
    }

    func navigateBack() {
        let currentStack = state.router.screenStack
        guard !currentStack.isEmpty else {
            return
        }
        let nextStack = Array(currentStack.dropLast())
        navigateOptimistically(
            to: nextStack,
            action: .updateScreenStack(stack: nextStack),
            showsToastOnFailure: false
        )
    }

#if os(iOS)
    /// Deep-link from anywhere in the iOS UI to the Messaging page in
    /// the settings modal. Implemented via NotificationCenter so the
    /// originating view doesn't need to thread a binding to
    /// `IrisRoot`'s `showingSettingsSheet`.
    func openMessagingSettings() {
        NotificationCenter.default.post(
            name: irisOpenSettingsNotification,
            object: nil,
            userInfo: ["focus": SettingsFocusSection.messaging]
        )
    }
#endif

    func dispatch(_ action: AppAction) {
        if shouldBlockOutgoingAction(action) {
            showToast("User is blocked")
            return
        }
#if os(iOS)
        if interceptScreenshotFixtureAction(action) {
            return
        }
#endif
        if handleOptimisticNavigation(action) {
            return
        }
        dispatchToRust(action)
    }

#if os(iOS)
    /// Routes UI taps that target a fixture chat (or write to one) past the
    /// Rust core, which has no record of these synthetic chats and would
    /// surface a dispatch-failed toast. The screen stack is still updated
    /// locally so navigation works in screenshot mode.
    private func interceptScreenshotFixtureAction(_ action: AppAction) -> Bool {
        guard let fixture = screenshotFixture else { return false }
        switch action {
        case .openChat(let chatId):
            let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard fixture.chatIsFixture(trimmed) else { return false }
            pendingNavigationOverride = nil
            applyLocalScreenStack([.chat(chatId: trimmed)])
            state = fixture.applyTo(state: state, referenceDate: screenshotFixtureReferenceDate)
            return true
        case .sendMessage(let chatId, _),
             .sendDisappearingMessage(let chatId, _, _),
             .sendAttachment(let chatId, _, _, _),
             .sendAttachments(let chatId, _, _),
             .sendTyping(let chatId),
             .stopTyping(let chatId),
             .deleteChat(let chatId),
             .setChatMuted(let chatId, _),
             .setChatPinned(let chatId, _),
             .setChatUnread(let chatId, _),
             .setChatMessageTtl(let chatId, _),
             .setContactNickname(let chatId, _),
             .setChatDraft(let chatId, _),
             .toggleReaction(let chatId, _, _),
             .markMessagesSeen(let chatId, _),
             .sendReceipt(let chatId, _, _):
            return fixture.chatIsFixture(chatId)
        default:
            return false
        }
    }
#endif

    func mutualGroups(ownerInput: String) -> [ChatThreadSnapshot] {
        rust.mutualGroups(ownerInput: ownerInput)
    }

    private func shouldBlockOutgoingAction(_ action: AppAction) -> Bool {
        switch action {
        case .sendMessage(chatId: let chatId, text: _),
             .sendDisappearingMessage(chatId: let chatId, text: _, expiresAtSecs: _),
             .sendAttachment(chatId: let chatId, filePath: _, filename: _, caption: _),
             .sendAttachments(chatId: let chatId, attachments: _, caption: _),
             .sendTyping(chatId: let chatId):
            return shouldBlockOutgoingChat(chatId: chatId)
        default:
            return false
        }
    }

    private func shouldBlockOutgoingChat(chatId: String) -> Bool {
        let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.lowercased().hasPrefix("group:") else {
            return false
        }
        return isUserBlocked(trimmed)
    }

    private static func normalizedBlockedUserId(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        let hex = peerInputToHex(input: trimmed)
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        if !hex.isEmpty {
            return hex
        }
        return normalizePeerInput(input: trimmed)
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
    }

    private func handleOptimisticNavigation(_ action: AppAction) -> Bool {
        switch action {
        case .navigateBack:
            navigateBack()
            return true
        case .openChat(let chatId):
            let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return true }
            navigateOptimistically(
                to: [.chat(chatId: trimmed)],
                action: .openChat(chatId: trimmed)
            )
            return true
        case .pushScreen(let screen):
            guard let stack = stackByApplyingPushScreen(screen) else {
                dispatchToRust(action)
                return true
            }
            navigateOptimistically(to: stack, action: action)
            return true
        case .updateScreenStack(let stack):
            navigateOptimistically(
                to: stack,
                action: action,
                showsToastOnFailure: false
            )
            return true
        default:
            return false
        }
    }

    private func stackByApplyingPushScreen(_ screen: Screen) -> [Screen]? {
        if state.account == nil {
            switch screen {
            case .welcome:
                return []
            case .createAccount, .restoreAccount, .addDevice:
                return [screen]
            default:
                return nil
            }
        }

        switch screen {
        case .chatList:
            return []
        case .newChat,
             .newGroup,
             .createInvite,
             .joinInvite,
             .settings,
             .deviceRoster:
            return [screen]
        case .chat(let chatId):
            let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : [.chat(chatId: trimmed)]
        case .groupDetails(let groupId):
            let trimmed = groupId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return nil }
            let groupChatId = "group:\(trimmed)"
            var stack = state.router.screenStack
            if activeChatID(in: state) != groupChatId {
                stack = [.chat(chatId: groupChatId)]
            }
            let detailsScreen = Screen.groupDetails(groupId: trimmed)
            if stack.last != detailsScreen {
                stack.append(detailsScreen)
            }
            return stack
        case .createAccount,
             .restoreAccount,
             .addDevice,
             .awaitingDeviceApproval,
             .deviceRevoked,
             .welcome:
            return nil
        }
    }

    private func navigateOptimistically(
        to stack: [Screen],
        action: AppAction,
        showsToastOnFailure: Bool = true
    ) {
        pendingNavigationOverride = PendingNavigationOverride(
            stack: stack,
            expiresAt: Date().addingTimeInterval(Self.navigationOverrideTTL)
        )
        applyLocalScreenStack(stack)
        appendClientDebugLog(
            category: "navigation.optimistic",
            detail: "\(actionLogName(action)) stack=\(stack)"
        )
        loadInitialChatPageForActiveChat(in: stack)
        dispatchOptimisticNavigationToRust(
            action,
            showsToastOnFailure: showsToastOnFailure
        )
    }

    /// Open a chat and queue a scroll-to-message hop on first paint.
    /// Used by the search result rows so tapping a message hit lands
    /// the chat at that bubble instead of the bottom of the
    /// timeline.
    func openChatAtMessage(chatId: String, messageId: String) {
        let trimmedChat = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedMessage = messageId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChat.isEmpty, !trimmedMessage.isEmpty else { return }
        pendingScrollMessageId = trimmedMessage
        dispatch(.openChat(chatId: trimmedChat))
        loadChatAroundMessage(chatId: trimmedChat, messageId: trimmedMessage)
    }

    /// ChatScreen calls this after it's actually scrolled the
    /// timeline to the target — clears the one-shot so navigating
    /// away and back doesn't re-scroll to the same hit.
    func consumePendingScrollMessage() {
        if pendingScrollMessageId != nil {
            pendingScrollMessageId = nil
        }
    }

    /// Run a grouped contacts / groups / messages search against the
    /// Rust core. Safe to call on every keystroke — the FTS index
    /// query is sub-millisecond and re-uses the core's open SQLite
    /// connection without going through the action queue.
    func search(_ query: String, scopeChatId: String? = nil, limit: UInt32 = 50) -> SearchResultSnapshot {
        rust.search(query: query, scopeChatId: scopeChatId, limit: limit)
    }

    private func loadInitialChatPageForActiveChat(in stack: [Screen]) {
        guard case .chat(let chatId) = stack.last else { return }
        loadInitialChatPage(chatId: chatId)
    }

    private func loadInitialChatPage(chatId: String) {
        let trimmedChat = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChat.isEmpty, !initialChatPageLoads.contains(trimmedChat) else {
            return
        }
        initialChatPageLoads.insert(trimmedChat)
        let runner = ChatPageLoadRunner(rust: rust)
        let pageSize = Self.chatPageSize
        Self.chatPageQueue.async { [weak self] in
            let page = runner.latest(chatId: trimmedChat, limit: pageSize)
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.initialChatPageLoads.remove(trimmedChat)
                guard let page else { return }
                self.rememberChatSnapshot(page)
                if page.messages.count < Int(pageSize) {
                    self.exhaustedOlderChatPages.insert(trimmedChat)
                } else {
                    self.exhaustedOlderChatPages.remove(trimmedChat)
                }
                guard self.activeChatID(in: self.state) == page.chatId else {
                    return
                }
                var nextState = self.state
                nextState.currentChat = self.mergedChatSnapshot(
                    existing: self.state.currentChat,
                    page: page
                )
                self.state = nextState
            }
        }
    }

    @discardableResult
    func loadOlderMessages(chatId: String, completion: @escaping (Bool) -> Void = { _ in }) -> Bool {
        let trimmedChat = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChat.isEmpty,
              !exhaustedOlderChatPages.contains(trimmedChat),
              let current = state.currentChat,
              current.chatId == trimmedChat,
              let firstMessage = current.messages.first else {
            return false
        }
        if olderChatPageLoads.contains(trimmedChat) {
            return true
        }
        olderChatPageLoads.insert(trimmedChat)
        let runner = ChatPageLoadRunner(rust: rust)
        let firstMessageId = firstMessage.id
        let pageSize = Self.chatPageSize
        Self.chatPageQueue.async { [weak self] in
            let page = runner.before(
                chatId: trimmedChat,
                beforeMessageId: firstMessageId,
                limit: pageSize
            )
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.olderChatPageLoads.remove(trimmedChat)
                if page?.messages.isEmpty != false {
                    if page?.messages.isEmpty == true {
                        self.exhaustedOlderChatPages.insert(trimmedChat)
                    }
                    completion(false)
                    return
                }
                let loadedPage = page!
                if loadedPage.messages.count < Int(pageSize) {
                    self.exhaustedOlderChatPages.insert(trimmedChat)
                }
                self.mergeCurrentChatSnapshot(loadedPage)
                completion(true)
            }
        }
        return true
    }

    func loadChatAroundMessage(chatId: String, messageId: String) {
        let trimmedChat = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedMessage = messageId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChat.isEmpty, !trimmedMessage.isEmpty else { return }
        if state.currentChat?.chatId == trimmedChat,
           state.currentChat?.messages.contains(where: { $0.id == trimmedMessage }) == true {
            return
        }
        let key = "\(trimmedChat)\u{1f}\(trimmedMessage)"
        guard !aroundChatPageLoads.contains(key) else { return }
        aroundChatPageLoads.insert(key)
        let runner = ChatPageLoadRunner(rust: rust)
        let beforeLimit = Self.chatAroundBeforeLimit
        let afterLimit = Self.chatAroundAfterLimit
        Self.chatPageQueue.async { [weak self] in
            let page = runner.around(
                chatId: trimmedChat,
                messageId: trimmedMessage,
                beforeLimit: beforeLimit,
                afterLimit: afterLimit
            )
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.aroundChatPageLoads.remove(key)
                if let page {
                    self.mergeCurrentChatSnapshot(page)
                }
            }
        }
    }

    func handleChatLink(_ url: URL) {
        guard url.scheme?.lowercased() == "https",
              url.host?.lowercased() == "chat.iris.to" else {
            return
        }

        if isInviteChatLink(url) {
            dispatchToRust(.acceptInvite(inviteInput: url.absoluteString))
            return
        }

        for candidate in chatLinkPeerCandidates(url) {
            let normalized = normalizePeerInput(input: candidate)
            if !normalized.isEmpty, isValidPeerInput(input: normalized) {
                dispatchToRust(.createChat(peerInput: normalized))
                return
            }
        }
    }

    func handleShareURL(_ url: URL) -> Bool {
        guard url.scheme?.lowercased() == "irischat",
              url.host?.lowercased() == "share",
              let shareID = url.pathComponents.dropFirst().first,
              !shareID.isEmpty else {
            return false
        }
        let autoSend = URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?
            .first { $0.name == "send" }?
            .value == "1"
        loadPendingShare(id: shareID, autoSend: autoSend)
        return true
    }

    func clearPendingShare() {
#if os(iOS)
        if let pendingShare {
            removePendingShareFiles(for: pendingShare)
        }
#endif
        pendingShare = nil
    }

    func startForward(text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        pendingShare = PendingShare(
            id: UUID().uuidString,
            text: trimmed,
            attachments: [],
            suggestedChatId: nil,
            suggestedChatIds: nil,
            autoSend: false,
            isForward: true
        )
    }

    func sendPendingShare(to chatId: String) {
        sendPendingShare(to: [chatId])
    }

    func sendPendingShare(to chatIds: [String]) {
        guard let share = pendingShare else {
            return
        }
        let targets = uniqueTrimmedChatIds(chatIds)
        guard !targets.isEmpty else {
            return
        }
        let stagedAttachments: [StagedAttachment]
        do {
            stagedAttachments = try stagePendingShareAttachments(share.attachments)
        } catch {
            showToast("Attachment could not be opened")
            return
        }
        var dispatchedAll = true
        for chatId in targets {
            if shouldBlockOutgoingChat(chatId: chatId) {
                dispatchedAll = false
                continue
            }
            if stagedAttachments.isEmpty {
                dispatchedAll = dispatchToRust(.sendMessage(chatId: chatId, text: share.text)) && dispatchedAll
            } else {
                dispatchedAll = dispatchToRust(
                    .sendAttachments(
                        chatId: chatId,
                        attachments: stagedAttachments.map {
                            OutgoingAttachment(filePath: $0.path, filename: $0.filename)
                        },
                        caption: share.text
                    )
                ) && dispatchedAll
            }
        }
        dispatch(.openChat(chatId: targets[0]))
        guard dispatchedAll else {
            return
        }
#if os(iOS)
        shareSuggestionDonor.donateSelectedChats(
            state.chatList.filter { targets.contains($0.chatId) }
        )
#endif
#if os(iOS)
        removePendingShareFiles(for: share)
#endif
        pendingShare = nil
    }

    private func uniqueTrimmedChatIds(_ chatIds: [String]) -> [String] {
        var seen = Set<String>()
        var result = [String]()
        for raw in chatIds {
            let chatId = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if chatId.isEmpty || seen.contains(chatId) {
                continue
            }
            seen.insert(chatId)
            result.append(chatId)
        }
        return result
    }

    private func stagePendingShareAttachments(_ attachments: [PendingShareAttachment]) throws -> [StagedAttachment] {
        try attachments.map { attachment in
            let staged = try stageOutgoingAttachment(URL(fileURLWithPath: attachment.path))
            let filename = attachment.filename.trimmingCharacters(in: .whitespacesAndNewlines)
            return StagedAttachment(
                path: staged.path,
                filename: filename.isEmpty ? staged.filename : filename
            )
        }
    }

    private func loadPendingShare(id: String, autoSend: Bool = false) {
#if os(iOS)
        guard let dir = pendingSharesDirectory() else {
            showToast("Sharing unavailable")
            return
        }
        let url = dir.appendingPathComponent(id).appendingPathExtension("json")
        guard loadPendingShare(from: url, autoSend: autoSend, showsToast: true) else {
            return
        }
#else
        _ = id
        _ = autoSend
#endif
    }

#if os(iOS)
    private func processPendingShareFilesIfNeeded() {
        guard pendingShare == nil,
              let dir = pendingSharesDirectory(),
              let urls = try? fileManager.contentsOfDirectory(
                at: dir,
                includingPropertiesForKeys: [.contentModificationDateKey],
                options: [.skipsHiddenFiles]
              ) else {
            processPendingShareIfReady()
            return
        }
        let payloadURLs = urls
            .filter { $0.pathExtension == "json" }
            .sorted { lhs, rhs in
                let lhsDate = (try? lhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                let rhsDate = (try? rhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                return lhsDate < rhsDate
            }
        for url in payloadURLs {
            if loadPendingShare(from: url, autoSend: nil, showsToast: false) {
                return
            }
        }
        processPendingShareIfReady()
    }

    @discardableResult
    private func loadPendingShare(from url: URL, autoSend: Bool?, showsToast: Bool) -> Bool {
        do {
            let data = try Data(contentsOf: url)
            let decoded = try JSONDecoder().decode(PendingShare.self, from: data)
            let share = decoded.withAutoSend(autoSend == true)
            pendingShare = share
            pendingSharePayloadURLs[share.id] = url
            dispatchToRust(.updateScreenStack(stack: []))
            processPendingShareIfReady()
            return true
        } catch {
            if showsToast {
                showToast("Sharing unavailable")
            }
            return false
        }
    }

    private func processPendingShareIfReady() {
        guard let share = pendingShare,
              share.shouldAutoSend,
              !share.suggestedTargetChatIds.isEmpty,
              state.account?.authorizationState == .authorized else {
            return
        }
        sendPendingShare(to: share.suggestedTargetChatIds)
    }

    private func removePendingShareFiles(for share: PendingShare) {
        if let url = pendingSharePayloadURLs.removeValue(forKey: share.id) {
            try? fileManager.removeItem(at: url)
        } else if let url = pendingShareURL(id: share.id) {
            try? fileManager.removeItem(at: url)
        }
        if let filesURL = pendingSharesDirectory()?.appendingPathComponent("\(share.id)-files", isDirectory: true) {
            try? fileManager.removeItem(at: filesURL)
        }
    }

    private func pendingShareURL(id: String) -> URL? {
        pendingSharesDirectory()?.appendingPathComponent(id).appendingPathExtension("json")
    }

    private func pendingSharesDirectory() -> URL? {
        shareContainerURL()?.appendingPathComponent("pending-shares", isDirectory: true)
    }

    private func shareContainerURL() -> URL? {
        if let sharedContainerOverride {
            return sharedContainerOverride
        }
        return fileManager.containerURL(forSecurityApplicationGroupIdentifier: AppPaths.appGroupIdentifier)
    }
#endif

#if os(iOS)
    func foregroundPushPresentationOptions(
        content: UNNotificationContent
    ) async -> UNNotificationPresentationOptions {
        let userInfo = content.userInfo
        if userInfo[foregroundDecryptedPushMarkerKey] as? Bool == true {
            if let payloadJson = serializedPushPayload(content: content),
               shouldBlockPushNotification(payloadJson: payloadJson) {
                return []
            }
            return [.banner, .sound, .list]
        }
        if isGenericIrisFallback(content: content) && !hasPushEventPayload(userInfo: userInfo) {
            return []
        }
        guard let resolution = resolvePushNotification(content: content) else {
            return fallbackForegroundPushPresentationOptions(content: content)
        }
        guard resolution.shouldShow else {
            return []
        }
        if shouldBlockPushNotification(payloadJson: resolution.payloadJson) {
            return []
        }
        await postForegroundDecryptedPush(resolution: resolution)
        return []
    }

    func foregroundPushPresentationOptions(
        userInfo: [AnyHashable: Any]
    ) async -> UNNotificationPresentationOptions {
        if userInfo[foregroundDecryptedPushMarkerKey] as? Bool == true {
            if let payloadJson = serializedPushPayload(userInfo: userInfo),
               shouldBlockPushNotification(payloadJson: payloadJson) {
                return []
            }
            return [.banner, .sound, .list]
        }
        guard let resolution = resolvePushNotification(userInfo: userInfo) else {
            return fallbackForegroundPushPresentationOptions(userInfo: userInfo)
        }
        guard resolution.shouldShow else {
            return []
        }
        if shouldBlockPushNotification(payloadJson: resolution.payloadJson) {
            return []
        }
        await postForegroundDecryptedPush(resolution: resolution)
        return []
    }

    func shouldSuppressPushNotification(userInfo: [AnyHashable: Any]) -> Bool {
        guard let resolution = resolvePushNotification(userInfo: userInfo) else {
            return false
        }
        guard resolution.shouldShow else {
            return true
        }
        return shouldBlockPushNotification(payloadJson: resolution.payloadJson)
    }

    func handlePushNotificationTap(userInfo: [AnyHashable: Any]) {
        guard let resolution = resolvePushNotification(userInfo: userInfo),
              let chatID = chatID(fromPushPayloadJson: resolution.payloadJson),
              !chatID.isEmpty else {
            return
        }
        dispatch(.openChat(chatId: chatID))
    }

    private func resolvePushNotification(userInfo: [AnyHashable: Any]) -> MobilePushNotificationResolution? {
        guard let payloadJson = serializedPushPayload(userInfo: userInfo) else {
            return nil
        }
        return resolvePushNotification(payloadJson: payloadJson)
    }

    private func resolvePushNotification(content: UNNotificationContent) -> MobilePushNotificationResolution? {
        guard let payloadJson = serializedPushPayload(content: content) else {
            return nil
        }
        return resolvePushNotification(payloadJson: payloadJson)
    }

    private func resolvePushNotification(payloadJson: String) -> MobilePushNotificationResolution? {
        dispatchToRust(.ingestMobilePushPayload(payloadJson: payloadJson), showsToastOnFailure: false)
        if let bundle = secretStore.load() {
            return decryptMobilePushNotificationPayload(
                dataDir: dataDir.path,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec,
                rawPayloadJson: payloadJson
            )
        }
        return resolveMobilePushNotificationPayload(rawPayloadJson: payloadJson)
    }

    private func fallbackForegroundPushPresentationOptions(
        userInfo: [AnyHashable: Any]
    ) -> UNNotificationPresentationOptions {
        isOpaqueEncryptedPush(userInfo: userInfo) ? [] : [.banner, .sound, .list]
    }

    private func fallbackForegroundPushPresentationOptions(
        content: UNNotificationContent
    ) -> UNNotificationPresentationOptions {
        isOpaqueEncryptedPush(userInfo: content.userInfo) || isGenericIrisFallback(content: content)
            ? []
            : [.banner, .sound, .list]
    }

    private func postForegroundDecryptedPush(
        resolution: MobilePushNotificationResolution
    ) async {
        guard !AppPaths.notificationsDisabledForAutomation(
            environment: ProcessInfo.processInfo.environment
        ) else {
            return
        }
#if targetEnvironment(simulator)
        return
#else
#if os(iOS)
        guard !isUiTestRun,
              AppPaths.testRunId(environment: ProcessInfo.processInfo.environment) == nil else {
            return
        }
#endif
        let content = UNMutableNotificationContent()
        content.title = resolution.title.isEmpty ? "Iris Chat" : resolution.title
        content.body = resolution.body
        content.sound = .default
        content.userInfo = foregroundDecryptedPushUserInfo(from: resolution.payloadJson)
        let request = UNNotificationRequest(
            identifier: UUID().uuidString,
            content: content,
            trigger: nil
        )
        try? await UNUserNotificationCenter.current().add(request)
#endif
    }

    private func isPushChatOpen(_ chatID: String) -> Bool {
        let openChatIDs = [
            currentScreenChatID,
            state.currentChat?.chatId
        ].compactMap { $0 }

        return openChatIDs.contains { openChatID in
            pushChatID(openChatID, matches: chatID)
        }
    }

    private func isPushChatMuted(_ chatID: String) -> Bool {
        state.chatList.contains { chat in
            chat.isMuted && pushChatID(chat.chatId, matches: chatID)
        }
    }

    private func isPushChatBlocked(_ chatID: String) -> Bool {
        let trimmed = chatID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.lowercased().hasPrefix(mobilePushGroupChatPrefix) else {
            return false
        }
        return isUserBlocked(trimmed)
    }

    private func shouldBlockPushNotification(payloadJson: String) -> Bool {
        chatIDs(fromPushPayloadJson: payloadJson).contains { chatID in
            isPushChatOpen(chatID) || isPushChatMuted(chatID) || isPushChatBlocked(chatID)
        }
    }

    private var currentScreenChatID: String? {
        guard case .chat(let chatID) = activeScreen else {
            return nil
        }
        return chatID
    }

    private func pushChatID(_ openChatID: String, matches pushChatID: String) -> Bool {
        let open = openChatID.trimmingCharacters(in: .whitespacesAndNewlines)
        let push = pushChatID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !open.isEmpty, !push.isEmpty else {
            return false
        }
        if open.caseInsensitiveCompare(push) == .orderedSame {
            return true
        }
        if open.lowercased().hasPrefix(mobilePushGroupChatPrefix) {
            let openGroupID = String(open.dropFirst(mobilePushGroupChatPrefix.count))
            return openGroupID.caseInsensitiveCompare(push) == .orderedSame
        }
        if push.lowercased().hasPrefix(mobilePushGroupChatPrefix) {
            let pushGroupID = String(push.dropFirst(mobilePushGroupChatPrefix.count))
            return open.caseInsensitiveCompare(pushGroupID) == .orderedSame
        }
        return false
    }
#endif

    func setStartupAtLoginEnabled(_ enabled: Bool) {
        do {
            try PlatformStartupAtLogin.setEnabled(enabled)
            dispatchToRust(.setStartupAtLoginEnabled(enabled: enabled))
        } catch {
            showToast("Startup setting unavailable")
        }
    }

    func setNearbyBluetoothEnabled(_ enabled: Bool) {
#if os(iOS) || os(macOS)
        let canStartTransport = state.preferences.nearbyEnabled
        if enabled && canStartTransport && nearbyIris.bluetoothPermissionNeedsSettings {
            showNearbySettingsHint("Allow Bluetooth in Settings")
            return
        }
        nearbyIris.setVisible(enabled && canStartTransport)
        dispatchToRust(.setNearbyBluetoothEnabled(enabled: enabled))
#endif
    }

    func setNearbyLanEnabled(_ enabled: Bool) {
#if os(iOS) || os(macOS)
        let canStartTransport = state.preferences.nearbyEnabled
        if enabled && canStartTransport && nearbyIris.lanPermissionNeedsSettings {
            showNearbySettingsHint("Allow Wi-Fi in Settings")
            return
        }
        if enabled && canStartTransport {
            markNearbyLanPermissionPromptAttempted()
        }
        nearbyIris.setLanVisible(enabled && canStartTransport)
        dispatchToRust(.setNearbyLanEnabled(enabled: enabled))
#endif
    }

    func setNearbyEnabled(_ enabled: Bool) {
#if os(iOS) || os(macOS)
        if !enabled {
            nearbyIris.setVisible(false)
            nearbyIris.setLanVisible(false)
        }
#endif
        dispatchToRust(.setNearbyEnabled(enabled: enabled))
    }

    deinit {
#if os(iOS)
        CFNotificationCenterRemoveObserver(
            CFNotificationCenterGetDarwinNotifyCenter(),
            Unmanaged.passUnretained(self).toOpaque(),
            CFNotificationName(appManagerPendingShareNotificationName as CFString),
            nil
        )
#endif
    }

#if os(iOS)
    private func registerPendingShareObserver() {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDarwinNotifyCenter(),
            Unmanaged.passUnretained(self).toOpaque(),
            { _, observer, _, _, _ in
                guard let observer else { return }
                let manager = Unmanaged<AppManager>
                    .fromOpaque(observer)
                    .takeUnretainedValue()
                Task { @MainActor in
                    manager.processPendingShareFilesIfNeeded()
                }
            },
            appManagerPendingShareNotificationName as CFString,
            nil,
            .deliverImmediately
        )
    }
#endif

    func appForegrounded() {
        lastForegroundedAt = Date()
        appSceneIsActive = true
        recordUserActivity()
        backgroundSuspendPrepared = false
        dispatchToRust(.appForegrounded)
#if os(macOS)
        updates.runStartupCheckIfNeeded()
#endif
#if os(iOS) || os(macOS)
        if nearbySettingsWasOpened {
            nearbySettingsWasOpened = false
            nearbyIris.refreshBluetoothAuthorizationStatus()
            nearbyIris.clearLanPermissionSettingsHint()
        }
        if state.preferences.nearbyEnabled,
           state.preferences.nearbyBluetoothEnabled,
           !nearbyIris.isVisible,
           nearbyIris.bluetoothPermissionGranted {
            nearbyIris.setVisible(true)
        }
        if state.preferences.nearbyEnabled,
           state.preferences.nearbyLanEnabled,
           canAutoStartNearbyLan,
           (!nearbyIris.isLanVisible || nearbyIris.shouldRestartLanAfterFailure) {
            nearbyIris.setLanVisible(true)
        }
#if os(iOS)
        processPendingShareFilesIfNeeded()
#endif
        if !AppPaths.notificationsDisabledForAutomation(environment: ProcessInfo.processInfo.environment) {
            UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        }
#endif
    }

    func appInactive() {
        appSceneIsActive = false
    }

    func appBackgrounded() {
        appSceneIsActive = false
#if os(iOS)
        guard !backgroundSuspendPrepared else {
            return
        }
        backgroundSuspendPrepared = true

        if nearbyIris.isVisible {
            nearbyIris.setVisible(false)
        }
        if nearbyIris.isLanVisible {
            nearbyIris.setLanVisible(false)
        }

        let runner = SuspendPreparationRunner(rust: rust)
        let taskID = UIApplication.shared.beginBackgroundTask(withName: "IrisSuspend") {}
        DispatchQueue.global(qos: .utility).async {
            runner.prepareForSuspend()
            guard taskID != .invalid else {
                return
            }
            DispatchQueue.main.async {
                UIApplication.shared.endBackgroundTask(taskID)
            }
        }
#endif
    }

    func recordUserActivity() {
        let now = Date()
        guard !canMarkActiveChatSeen || now.timeIntervalSince(lastUserActivityAt) >= 5 else {
            return
        }
        lastUserActivityAt = now
    }

    var canMarkActiveChatSeen: Bool {
        appSceneIsActive &&
            Date().timeIntervalSince(lastUserActivityAt) <= Self.activeChatSeenIdleLimit
    }

    var seenEligibilityToken: String {
        [
            appSceneIsActive ? "active" : "inactive",
            String(Int(lastUserActivityAt.timeIntervalSince1970))
        ].joined(separator: ":")
    }

#if os(iOS) || os(macOS)
    private var canAutoStartNearbyLan: Bool {
        nearbyLanPermissionGranted
    }

    private func markNearbyLanPermissionPromptAttempted() {
        setNearbyLanPermissionDefault(true, forKey: Self.nearbyLanPermissionPromptAttemptedKey)
    }

    private func markNearbyLanPermissionGranted() {
        setNearbyLanPermissionDefault(true, forKey: Self.nearbyLanPermissionPromptAttemptedKey)
        setNearbyLanPermissionDefault(true, forKey: Self.nearbyLanPermissionGrantedKey)
    }

    private func markNearbyLanPermissionDenied() {
        setNearbyLanPermissionDefault(true, forKey: Self.nearbyLanPermissionPromptAttemptedKey)
        setNearbyLanPermissionDefault(false, forKey: Self.nearbyLanPermissionGrantedKey)
    }

    private func setNearbyLanPermissionDefault(_ value: Bool, forKey key: String) {
        if key == Self.nearbyLanPermissionPromptAttemptedKey {
            guard nearbyLanPermissionPromptAttempted != value else {
                return
            }
            nearbyLanPermissionPromptAttempted = value
        } else if key == Self.nearbyLanPermissionGrantedKey {
            guard nearbyLanPermissionGranted != value else {
                return
            }
            nearbyLanPermissionGranted = value
        } else if UserDefaults.standard.bool(forKey: key) == value {
            return
        }
        UserDefaults.standard.set(value, forKey: key)
    }

    private func handleNearbyBluetoothPermissionDenied() {
        guard state.preferences.nearbyBluetoothEnabled || nearbyIris.isVisible else {
            return
        }
        nearbyIris.setVisible(false)
        dispatchToRust(.setNearbyBluetoothEnabled(enabled: false), showsToastOnFailure: false)
        showToast("Allow Bluetooth in Settings")
    }

    private func handleNearbyLanPermissionDenied() {
        guard state.preferences.nearbyLanEnabled || nearbyIris.isLanVisible else {
            return
        }
        markNearbyLanPermissionDenied()
        nearbyIris.setLanVisible(false)
        dispatchToRust(.setNearbyLanEnabled(enabled: false), showsToastOnFailure: false)
        showToast("Allow Wi-Fi in Settings")
    }

    private func showNearbySettingsHint(_ message: String) {
        showToast(message)
        nearbySettingsWasOpened = true
        PlatformAppSettings.open()
    }
#endif

    private func syncStartupAtLoginPreference(_ enabled: Bool) {
        guard PlatformStartupAtLogin.isSupported else {
            return
        }
        try? PlatformStartupAtLogin.setEnabled(enabled)
    }

    func createAccount(name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        dispatchToRust(.createAccount(name: trimmed))
    }

    func updateProfileMetadata(name: String, pictureURL: String?, about: String?) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        let trimmedPictureURL = pictureURL?.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedAbout = about?.trimmingCharacters(in: .whitespacesAndNewlines)
        dispatchToRust(.updateProfileMetadata(
            name: trimmed,
            pictureUrl: trimmedPictureURL?.isEmpty == false ? trimmedPictureURL : nil,
            about: trimmedAbout?.isEmpty == false ? trimmedAbout : nil
        ))
    }

    func deleteProfileAndLocalData() {
        guard dispatchToRust(.deleteProfileMetadata) else {
            return
        }
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            self?.logout()
        }
    }

    func restoreSession(ownerNsec: String) {
        let trimmed = ownerNsec.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            showToast("Invalid key.")
            return
        }
        dispatchToRust(.restoreSession(ownerNsec: trimmed))
    }

    func startLinkedDevice(ownerInput: String) {
        dispatchToRust(.startLinkedDevice(ownerInput: ownerInput.trimmingCharacters(in: .whitespacesAndNewlines)))
    }

    func addAuthorizedDevice(deviceInput: String) {
        let trimmed = deviceInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        dispatchToRust(.addAuthorizedDevice(deviceInput: trimmed))
    }

    func removeAuthorizedDevice(devicePubkeyHex: String) {
        let trimmed = devicePubkeyHex.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        dispatchToRust(.removeAuthorizedDevice(devicePubkeyHex: trimmed))
    }

    func setCurrentDeviceName(_ name: String, currentClientLabel: String?) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let deviceLabel = trimmed.isEmpty ? PlatformDeviceLabels.currentDeviceLabel : trimmed
        let clientLabel = nonEmptyLabel(currentClientLabel) ?? PlatformDeviceLabels.currentClientLabel
        let key = "\(deviceLabel)\u{1F}\(clientLabel)"
        lastSyncedDeviceLabelsKey = key
        if !dispatchToRust(
            .setCurrentDeviceLabels(deviceLabel: deviceLabel, clientLabel: clientLabel),
            showsToastOnFailure: true
        ) {
            lastSyncedDeviceLabelsKey = nil
        }
    }

    func copyToClipboard(_ value: String) {
        PlatformClipboard.setString(value)
        showToast("Copied")
    }

    func showAttachmentOpenError() {
        showToast("Attachment could not be opened")
    }

    func showSecretExportUnavailable() {
        showToast("Key unavailable")
    }

    func downloadAttachment(_ attachment: MessageAttachmentSnapshot) async -> Data? {
        if let cached = cachedDownloadedAttachmentData(for: attachment) {
            return cached
        }

        return await downloadHashtreeBytes(nhash: attachment.nhash).flatMap { data in
            _ = try? cachedDownloadedAttachmentURL(for: attachment, data: data)
            return data
        }
    }

    /// Resolves an `htree://` profile picture (or any nhash) using the same
    /// disk-backed cache that chat attachments use. Reads return cached bytes
    /// immediately and avoid re-downloading the same blob next launch.
    func resolveHashtreePictureBytes(nhash: String) async -> Data? {
        let trimmed = nhash.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let cacheUrl = downloadedAttachmentDirectory()
            .appendingPathComponent("picture-\(safeAttachmentFilename(trimmed))")
        if fileManager.fileExists(atPath: cacheUrl.path) {
            try? fileManager.setAttributes([.modificationDate: Date()], ofItemAtPath: cacheUrl.path)
            if let data = try? Data(contentsOf: cacheUrl) {
                return data
            }
        }
        guard let data = await downloadHashtreeBytes(nhash: trimmed) else {
            return nil
        }
        do {
            try fileManager.createDirectory(
                at: downloadedAttachmentDirectory(),
                withIntermediateDirectories: true
            )
            if fileManager.fileExists(atPath: cacheUrl.path) {
                try fileManager.removeItem(at: cacheUrl)
            }
            try data.write(to: cacheUrl, options: [.atomic])
            try pruneDownloadedAttachmentCache(protecting: cacheUrl)
        } catch {
            // Cache is best-effort; fall through with the in-memory data.
        }
        return data
    }

    func downloadHashtreeBytes(nhash: String) async -> Data? {
        return await Task.detached(priority: .userInitiated) { () -> Data? in
            let result = downloadHashtreeAttachment(
                nhash: nhash
            )
            guard let encoded = result.dataBase64, !encoded.isEmpty else {
                return nil
            }
            return Data(base64Encoded: encoded)
        }.value
    }

    func openAttachment(_ attachment: MessageAttachmentSnapshot) async {
        guard let data = await downloadAttachment(attachment) else {
            showAttachmentOpenError()
            return
        }

        do {
            let url = try cachedDownloadedAttachmentURL(for: attachment, data: data)
            guard PlatformDocumentOpener.open(url) else {
                showAttachmentOpenError()
                return
            }
        } catch {
            showAttachmentOpenError()
        }
    }

    func sendAttachment(chatId: String, fileURL: URL, caption: String) {
        let trimmedChatId = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChatId.isEmpty else {
            return
        }
        guard !shouldBlockOutgoingChat(chatId: trimmedChatId) else {
            showToast("User is blocked")
            return
        }

        do {
            let staged = try stageOutgoingAttachment(fileURL)
            dispatchToRust(
                .sendAttachment(
                    chatId: trimmedChatId,
                    filePath: staged.path,
                    filename: staged.filename,
                    caption: caption.trimmingCharacters(in: .whitespacesAndNewlines)
                )
            )
        } catch {
            showToast("Attachment could not be opened")
        }
    }

    func sendAttachments(chatId: String, attachments: [StagedAttachment], caption: String) {
        let trimmedChatId = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChatId.isEmpty, !attachments.isEmpty else {
            return
        }
        guard !shouldBlockOutgoingChat(chatId: trimmedChatId) else {
            showToast("User is blocked")
            return
        }
        dispatchToRust(
            .sendAttachments(
                chatId: trimmedChatId,
                attachments: attachments.map {
                    OutgoingAttachment(filePath: $0.path, filename: $0.filename)
                },
                caption: caption.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        )
    }

    func updateGroupPicture(groupId: String, fileURL: URL) {
        do {
            let staged = try stageOutgoingAttachment(fileURL)
            dispatchToRust(.updateGroupPicture(
                groupId: groupId,
                filePath: staged.path,
                filename: staged.filename
            ))
        } catch {
            showToast("Image could not be opened")
        }
    }

    func stageGroupPicture(fileURL: URL) -> StagedAttachment? {
        do {
            let staged = try stageOutgoingAttachment(fileURL)
            return StagedAttachment(path: staged.path, filename: staged.filename)
        } catch {
            showToast("Image could not be opened")
            return nil
        }
    }

    func createGroup(name: String, memberInputs: [String], picture: StagedAttachment?) {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let members = memberInputs
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard !trimmedName.isEmpty else {
            return
        }

        if let picture {
            dispatchToRust(.createGroupWithPicture(
                name: trimmedName,
                memberInputs: members,
                pictureFilePath: picture.path,
                pictureFilename: picture.filename
            ))
        } else {
            dispatchToRust(.createGroup(name: trimmedName, memberInputs: members))
        }
    }

    func setGroupAdmin(groupId: String, ownerPubkeyHex: String, isAdmin: Bool) {
        dispatchToRust(.setGroupAdmin(
            groupId: groupId,
            ownerPubkeyHex: ownerPubkeyHex,
            isAdmin: isAdmin
        ))
    }

    func uploadProfilePicture(fileURL: URL) {
        irisDebugLog("[upload-profile-picture] picked: %@", fileURL.path)
        do {
            let staged = try stageOutgoingAttachment(fileURL)
            irisDebugLog("[upload-profile-picture] staged: %@", staged.path)
            dispatchToRust(.uploadProfilePicture(filePath: staged.path))
        } catch {
            irisDebugLog("[upload-profile-picture] stage failed: %@", "\(error)")
            showToast("Image could not be opened: \(error.localizedDescription)")
        }
    }

    func stageOutgoingAttachments(_ sourceURLs: [URL]) throws -> [StagedAttachment] {
        try sourceURLs.map { url in
            let staged = try stageOutgoingAttachment(url)
            return StagedAttachment(path: staged.path, filename: staged.filename)
        }
    }

    func supportBundleJson() -> String {
        Self.supportBundleJsonWithClientLog(
            rustJson: rust.exportSupportBundleJson(),
            clientDebugLog: clientDebugLog
        )
    }

    func supportBundleJsonAsync() async -> String {
        let runner = SupportBundleExportRunner(rust: rust)
        let clientLog = clientDebugLog
        return await withCheckedContinuation { continuation in
            Self.supportBundleQueue.async {
                let rustJson = runner.export()
                let mergedJson = Self.supportBundleJsonWithClientLog(
                    rustJson: rustJson,
                    clientDebugLog: clientLog
                )
                continuation.resume(returning: mergedJson)
            }
        }
    }

    func peerProfileDebug(ownerInput: String) -> PeerProfileDebugSnapshot? {
        rust.peerProfileDebug(ownerInput: ownerInput)
    }

    func exportOwnerNsec() -> String? {
        secretStore.load()?.ownerNsec
    }

    func exportDeviceNsec() -> String? {
        secretStore.load()?.deviceNsec
    }

    func resetAppState() {
        logout()
    }

    func buildSummaryText() -> String {
        buildSummary()
    }

    func relaySetIdText() -> String {
        relaySetId()
    }

    func trustedTestBuildEnabled() -> Bool {
        isTrustedTestBuild()
    }

    func logout() {
        // Logout ownership stays in Rust. The shell clears native secrets and local files only.
#if os(iOS)
        mobilePushRuntime.unregisterStoredSubscription(state: state, ownerNsec: storedAccountBundle?.ownerNsec ?? secretStore.load()?.ownerNsec)
#endif
        guard secretStore.clear() else {
            showToast("Could not clear secret key.")
            return
        }
        dispatchToRust(.logout)
        storedAccountBundle = nil
        replaceRustCoreAfterLocalReset()
    }

    private func replaceRustCoreAfterLocalReset() {
        let previousRust = rust
        previousRust.shutdown()
        try? fileManager.removeItem(at: dataDir)
        try? fileManager.createDirectory(at: dataDir, withIntermediateDirectories: true)
        AppPaths.prepareDataDirForBackgroundNotificationReads(dataDir, fileManager: fileManager)
        pendingNavigationOverride = nil
        olderChatPageLoads.removeAll()
        exhaustedOlderChatPages.removeAll()
        aroundChatPageLoads.removeAll()
        initialChatPageLoads.removeAll()
        chatSnapshotCache.removeAll()
        chatSnapshotCacheOrder.removeAll()
        persistedRestoreInFlight = false
        bootstrapInFlight = false
        let nextRust = makeRustClient()
        rust = nextRust
        nextRust.listenForUpdates(reconciler: reconciler)
        applyFullState(nextRust.state(), force: true)
    }

    private func syncCurrentDeviceLabelsIfNeeded(state: AppState) {
        guard state.account != nil else {
            lastSyncedDeviceLabelsKey = nil
            return
        }
        let currentDevice = state.deviceRoster?.devices.first(where: \.isCurrentDevice)
        let deviceLabel = nonEmptyLabel(currentDevice?.deviceLabel) ?? PlatformDeviceLabels.currentDeviceLabel
        let clientLabel = nonEmptyLabel(currentDevice?.clientLabel) ?? PlatformDeviceLabels.currentClientLabel
        let key = "\(deviceLabel)\u{1F}\(clientLabel)"
        guard key != lastSyncedDeviceLabelsKey else { return }
        lastSyncedDeviceLabelsKey = key
        if !dispatchToRust(
            .setCurrentDeviceLabels(deviceLabel: deviceLabel, clientLabel: clientLabel),
            showsToastOnFailure: false
        ) {
            lastSyncedDeviceLabelsKey = nil
        }
    }

    private func nonEmptyLabel(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed?.isEmpty == false ? trimmed : nil
    }

#if os(iOS)
    private func syncShareSuggestionsIfNeeded(chatList: [ChatThreadSnapshot]) {
        guard iosSideEffectGate.shouldSyncShareSuggestions(chatList: chatList) else {
            return
        }
        shareSuggestionDonor.syncRecentChats(chatList)
        shareSuggestionsExporter.export(chats: chatList)
    }

    private func syncMobilePushIfNeeded(state: AppState) {
        guard !isUiTestRun else {
            iosSideEffectGate.resetMobilePush()
            return
        }
        guard nonEmptyTrimmedString(state.mobilePush.ownerPubkeyHex) != nil else {
            iosSideEffectGate.resetMobilePush()
            return
        }
        let ownerNsec = storedAccountBundle?.ownerNsec
        guard iosSideEffectGate.shouldSyncMobilePush(state: state, ownerNsec: ownerNsec) else {
            return
        }
        mobilePushRuntime.sync(state: state, ownerNsec: ownerNsec)
    }

    private func syncIosStateSideEffects(for state: AppState) {
        syncShareSuggestionsIfNeeded(chatList: state.chatList)
        syncMobilePushIfNeeded(state: state)
    }
#endif

    func apply(update: AppUpdate) {
        switch update {
        case .persistAccountBundle(_, let ownerNsec, let ownerPubkeyHex, let deviceNsec):
            // Secure persistence is a shell side effect and must be applied even if snapshot revs race.
            let bundle = StoredAccountBundle(
                ownerNsec: ownerNsec,
                ownerPubkeyHex: ownerPubkeyHex,
                deviceNsec: deviceNsec
            )
            secretStore.save(bundle)
            storedAccountBundle = bundle
#if os(iOS)
            syncMobilePushIfNeeded(state: state)
#endif
        case .nearbyPublishedEvent(let eventID, let kind, let createdAtSecs, let eventJson):
#if os(iOS) || os(macOS)
            nearbyIris.publish(
                eventID: eventID,
                kind: kind,
                createdAtSecs: createdAtSecs,
                eventJson: eventJson
            )
#else
            _ = eventID
            _ = kind
            _ = createdAtSecs
            _ = eventJson
#endif
        case .fullState(let nextState):
            applyFullState(nextState)
        }
    }

    private func applyFullState(_ nextState: AppState, force: Bool = false) {
        // Rust owns authoritative state. The shell only accepts the newest full snapshot,
        // except after a local reset where a freshly-bound core legitimately starts at rev 0.
        guard force || nextState.rev > lastRevApplied else {
            return
        }
        let oldState = state
        var reconciledState = stateByPreservingVisibleChatPage(
            from: oldState,
            into: stateByReconcilingPendingNavigation(nextState)
        )
#if os(iOS)
        reconciledState = stateByApplyingUiTestSeedDaySplit(reconciledState)
        reconciledState = stateByApplyingScreenshotFixture(reconciledState)
#endif
        lastRevApplied = nextState.rev
        postDesktopNotifications(from: oldState, to: reconciledState)
        state = reconciledState
        syncCurrentDeviceLabelsIfNeeded(state: reconciledState)
        rememberCurrentChatIfPresent()
        irisSetDebugLoggingEnabled(reconciledState.preferences.debugLoggingEnabled)
#if os(iOS) || os(macOS)
        syncNearbyBluetoothPreference(from: oldState, to: reconciledState)
        syncNearbyLanPreference(from: oldState, to: reconciledState)
#endif
#if os(iOS)
        syncIosStateSideEffects(for: reconciledState)
#endif
        settleBootstrapIfNeeded(with: reconciledState)
        if let toast = reconciledState.toast, !toast.isEmpty {
            showToast(toast)
        }
        runPendingTestSeedIfNeeded()
#if os(iOS)
        processPendingShareFilesIfNeeded()
#endif
    }

    private func runPendingTestSeedIfNeeded() {
        guard let seed = pendingTestSeed else { return }
        guard state.account != nil else { return }
        if state.chatList.isEmpty {
            let normalized = normalizePeerInput(input: seed.peer)
            guard !normalized.isEmpty, isValidPeerInput(input: normalized) else {
                pendingTestSeed = nil
                return
            }
            dispatchToRust(.createChat(peerInput: normalized))
            return
        }
        guard !seedTestMessagesDispatched, let chat = state.chatList.first else { return }
        seedTestMessagesDispatched = true
        for i in 1...seed.count {
            let label: String
            if i == 1 {
                label = "FIRST_SCROLL_SENTINEL"
            } else if i == seed.count {
                label = "LAST_SCROLL_SENTINEL"
            } else {
                label = "seed-msg-\(i)"
            }
            let body = "\(label) lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua"
            dispatchToRust(.sendMessage(chatId: chat.chatId, text: body))
        }
        // Pop back to the chat list so the test can re-enter the chat
        // from a clean state — that's the "open an existing long chat"
        // scenario the bug report describes. Without this we'd be
        // racing with the message-arrival auto-scroll on first paint,
        // which is a different (and easier) code path.
        dispatchToRust(.updateScreenStack(stack: []))
        pendingTestSeed = nil
    }

#if os(iOS)
    private func stateByApplyingScreenshotFixture(_ source: AppState) -> AppState {
        guard let fixture = screenshotFixture else { return source }
        // Rust has no record of fixture chats, so every Rust-driven state
        // update clears the screen stack — that would yank the user back
        // to the chat list mid-screenshot. Pin the current local stack
        // when it terminates in a fixture chat.
        var pinned = source
        if let active = state.router.screenStack.last,
           case let .chat(chatId) = active,
           fixture.chatIsFixture(chatId),
           source.router.screenStack.last != active {
            pinned.router = Router(
                defaultScreen: source.router.defaultScreen,
                screenStack: state.router.screenStack
            )
        }
        return fixture.applyTo(state: pinned, referenceDate: screenshotFixtureReferenceDate)
    }

    private func stateByApplyingUiTestSeedDaySplit(_ source: AppState) -> AppState {
        guard isUiTestRun,
              let splitIndex = uiTestSeedDaySplitIndex,
              splitIndex > 0,
              var currentChat = source.currentChat else {
            return source
        }

        let calendar = Calendar.current
        let todayStart = calendar.startOfDay(for: Date())
        let yesterdayStart = calendar.date(byAdding: .day, value: -1, to: todayStart)
            ?? todayStart.addingTimeInterval(-86_400)
        var changed = false

        for index in currentChat.messages.indices {
            guard let ordinal = uiTestSeedOrdinal(from: currentChat.messages[index].body) else {
                continue
            }
            let dayStart = ordinal <= splitIndex ? yesterdayStart : todayStart
            let timestamp = dayStart.addingTimeInterval(TimeInterval(ordinal))
            currentChat.messages[index].createdAtSecs = UInt64(max(0, timestamp.timeIntervalSince1970))
            changed = true
        }

        guard changed else { return source }
        currentChat.messages.sort(by: chatMessagePrecedes)
        var next = source
        next.currentChat = currentChat
        return next
    }

    private func uiTestSeedOrdinal(from body: String) -> Int? {
        if body.hasPrefix("FIRST_SCROLL_SENTINEL") {
            return 1
        }
        if body.hasPrefix("LAST_SCROLL_SENTINEL") {
            return uiTestSeedCount
        }
        guard body.hasPrefix("seed-msg-") else {
            return nil
        }
        let suffix = body.dropFirst("seed-msg-".count)
        let digits = suffix.prefix { $0.isNumber }
        return Int(digits)
    }
#endif

    private func restorePersistedSession() {
        // Native restore only rehydrates secure inputs. Rust rebuilds the authoritative app state.
        guard let bundle = secretStore.load() else {
            storedAccountBundle = nil
            bootstrapInFlight = false
            return
        }
        secretStore.save(bundle)
        storedAccountBundle = bundle
        persistedRestoreInFlight = true
        let dispatched = dispatchToRust(
            .restoreAccountBundle(
                ownerNsec: bundle.ownerNsec,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec
            )
        )
        if !dispatched {
            persistedRestoreInFlight = false
            bootstrapInFlight = false
        }
    }

    private func settleBootstrapIfNeeded(with nextState: AppState) {
        guard persistedRestoreInFlight else {
            bootstrapInFlight = false
            return
        }
        guard nextState.account == nil && nextState.busy.restoringSession else {
            persistedRestoreInFlight = false
            bootstrapInFlight = false
            return
        }
        bootstrapInFlight = true
    }

    private func stateByReconcilingPendingNavigation(_ nextState: AppState) -> AppState {
        guard let pending = pendingNavigationOverride else {
            return nextState
        }
        guard nextState.account != nil else {
            pendingNavigationOverride = nil
            return nextState
        }
        if nextState.router.screenStack == pending.stack {
            pendingNavigationOverride = nil
            return nextState
        }
        guard Date() < pending.expiresAt else {
            pendingNavigationOverride = nil
            return nextState
        }
        return stateByApplyingLocalScreenStack(pending.stack, to: nextState)
    }

    private func stateByPreservingVisibleChatPage(from oldState: AppState, into nextState: AppState) -> AppState {
        guard let oldChat = oldState.currentChat,
              let newChat = nextState.currentChat,
              oldChat.chatId == newChat.chatId,
              activeChatID(in: nextState) == newChat.chatId else {
            return nextState
        }
        let newMessageIDs = Set(newChat.messages.map(\.id))
        guard oldChat.messages.contains(where: { !newMessageIDs.contains($0.id) }) else {
            return nextState
        }
        var result = nextState
        result.currentChat = mergedChatSnapshot(existing: oldChat, page: newChat)
        return result
    }

    private func mergeCurrentChatSnapshot(_ page: CurrentChatSnapshot) {
        guard state.currentChat?.chatId == page.chatId else { return }
        var nextState = state
        nextState.currentChat = mergedChatSnapshot(existing: state.currentChat, page: page)
        state = nextState
        rememberChatSnapshot(nextState.currentChat)
    }

    private func mergedChatSnapshot(existing: CurrentChatSnapshot?, page: CurrentChatSnapshot) -> CurrentChatSnapshot {
        guard let existing, existing.chatId == page.chatId else {
            return page
        }
        var merged = page
        merged.messages = mergedChatMessages(existing: existing.messages, page: page.messages)
        if merged.participants.isEmpty {
            merged.participants = existing.participants
        }
        if merged.typingIndicators.isEmpty {
            merged.typingIndicators = existing.typingIndicators
        }
        if merged.draft.isEmpty, !existing.draft.isEmpty {
            merged.draft = existing.draft
        }
        return merged
    }

    private func mergedChatMessages(
        existing: [ChatMessageSnapshot],
        page: [ChatMessageSnapshot]
    ) -> [ChatMessageSnapshot] {
        var byID: [String: ChatMessageSnapshot] = [:]
        var orderByID: [String: Int] = [:]
        byID.reserveCapacity(existing.count + page.count)
        orderByID.reserveCapacity(existing.count + page.count)

        let pageIDs = Set(page.map(\.id))

        func rememberOrder(_ id: String) {
            if orderByID[id] == nil {
                orderByID[id] = orderByID.count
            }
        }

        for message in existing where !pageIDs.contains(message.id) {
            byID[message.id] = message
            rememberOrder(message.id)
        }
        // Rust page order is authoritative for equal-timestamp messages;
        // sorting by event id here makes same-second local sends jump around.
        for message in page {
            byID[message.id] = message
            rememberOrder(message.id)
        }

        return byID.values.sorted { lhs, rhs in
            if lhs.createdAtSecs != rhs.createdAtSecs {
                return lhs.createdAtSecs < rhs.createdAtSecs
            }
            return (orderByID[lhs.id] ?? .max) < (orderByID[rhs.id] ?? .max)
        }
    }

    private func chatMessagePrecedes(_ lhs: ChatMessageSnapshot, _ rhs: ChatMessageSnapshot) -> Bool {
        if lhs.createdAtSecs != rhs.createdAtSecs {
            return lhs.createdAtSecs < rhs.createdAtSecs
        }
        let lhsNumeric = UInt64(lhs.id)
        let rhsNumeric = UInt64(rhs.id)
        switch (lhsNumeric, rhsNumeric) {
        case let (lhs?, rhs?) where lhs != rhs:
            return lhs < rhs
        case (.some, .none):
            return true
        case (.none, .some):
            return false
        default:
            return lhs.id < rhs.id
        }
    }

    private func applyLocalScreenStack(_ stack: [Screen]) {
        state = stateByApplyingLocalScreenStack(stack, to: state)
    }

    private func stateByApplyingLocalScreenStack(_ stack: [Screen], to baseState: AppState) -> AppState {
        var nextState = baseState
        nextState.router = Router(defaultScreen: nextState.router.defaultScreen, screenStack: stack)
        let activeScreen = stack.last ?? nextState.router.defaultScreen
        switch activeScreen {
        case .chat(let chatId):
            if nextState.currentChat?.chatId != chatId {
                nextState.currentChat = cachedChatSnapshot(chatId: chatId)
            } else {
                rememberChatSnapshot(nextState.currentChat)
            }
            nextState.groupDetails = nil
        case .groupDetails(let groupId):
            if nextState.groupDetails?.groupId != groupId {
                nextState.groupDetails = nil
            }
        default:
            nextState.currentChat = nil
            nextState.groupDetails = nil
        }
        return nextState
    }

    private func cachedChatSnapshot(chatId: String) -> CurrentChatSnapshot? {
        guard let snapshot = chatSnapshotCache[chatId] else { return nil }
        touchCachedChatSnapshot(chatId)
        return snapshot
    }

    private func rememberCurrentChatIfPresent() {
        rememberChatSnapshot(state.currentChat)
    }

    private func rememberChatSnapshot(_ snapshot: CurrentChatSnapshot?) {
        guard let snapshot else { return }
        chatSnapshotCache[snapshot.chatId] = snapshot
        touchCachedChatSnapshot(snapshot.chatId)
        while chatSnapshotCacheOrder.count > Self.chatSnapshotCacheLimit {
            let evicted = chatSnapshotCacheOrder.removeFirst()
            chatSnapshotCache.removeValue(forKey: evicted)
        }
    }

    private func touchCachedChatSnapshot(_ chatId: String) {
        chatSnapshotCacheOrder.removeAll { $0 == chatId }
        chatSnapshotCacheOrder.append(chatId)
    }

#if os(iOS) || os(macOS)
    private func syncNearbyBluetoothPreference(from oldState: AppState, to nextState: AppState) {
        let wasVisible = oldState.preferences.nearbyEnabled && oldState.preferences.nearbyBluetoothEnabled
        let shouldBeVisible = nextState.preferences.nearbyEnabled && nextState.preferences.nearbyBluetoothEnabled
        if shouldBeVisible {
            if !nearbyIris.isVisible, nearbyIris.bluetoothPermissionGranted {
                nearbyIris.setVisible(true)
            }
        } else if wasVisible || nearbyIris.isVisible {
            nearbyIris.setVisible(false)
        }
    }

    private func syncNearbyLanPreference(from oldState: AppState, to nextState: AppState) {
        let wasVisible = oldState.preferences.nearbyEnabled && oldState.preferences.nearbyLanEnabled
        let shouldBeVisible = nextState.preferences.nearbyEnabled && nextState.preferences.nearbyLanEnabled
        if shouldBeVisible {
            if canAutoStartNearbyLan,
               (!nearbyIris.isLanVisible || nearbyIris.shouldRestartLanAfterFailure) {
                nearbyIris.setLanVisible(true)
            }
        } else if wasVisible || nearbyIris.isLanVisible {
            nearbyIris.setLanVisible(false)
        }
    }
#endif

    @discardableResult
    private func dispatchToRust(
        _ action: AppAction,
        showsToastOnFailure: Bool = true,
        preservesPendingNavigation: Bool = false
    ) -> Bool {
        if !preservesPendingNavigation, actionClearsPendingNavigation(action) {
            pendingNavigationOverride = nil
        }
        do {
            try rust.dispatch(action: action)
            return true
        } catch {
            handleDispatchFailure(action: action, error: error, showsToast: showsToastOnFailure)
            return false
        }
    }

    private func dispatchOptimisticNavigationToRust(
        _ action: AppAction,
        showsToastOnFailure: Bool
    ) {
        let runner = RustDispatchRunner(rust: rust)
        Self.optimisticNavigationDispatchQueue.async { [weak self] in
            do {
                try runner.dispatch(action: action)
            } catch {
                Task { @MainActor [weak self] in
                    self?.pendingNavigationOverride = nil
                    self?.handleDispatchFailure(
                        action: action,
                        error: error,
                        showsToast: showsToastOnFailure
                    )
                }
            }
        }
    }

    private func handleDispatchFailure(action: AppAction, error: Error, showsToast: Bool) {
        logDispatchFailure(action: action, error: error)
        if showsToast {
            showToast(Self.dispatchFailureToast)
        }
    }

    private func actionClearsPendingNavigation(_ action: AppAction) -> Bool {
        switch action {
        case .openChat,
             .pushScreen,
             .updateScreenStack,
             .navigateBack,
             .createChat,
             .createGroup,
             .createGroupWithPicture,
             .acceptInvite,
             .logout,
             .restoreSession,
             .restoreAccountBundle:
            return true
        default:
            return false
        }
    }

    private func logDispatchFailure(action: AppAction, error: Error) {
        let actionName = actionLogName(action)
        let message = "Iris Chat FFI dispatch failed (\(actionName)): \(error)"
        appendClientDebugLog(category: "ffi.dispatch.failed", detail: "\(actionName): \(error)")
        irisDebugLog("%@", message)
#if DEBUG
        print(message)
#endif
    }

    private func appendClientDebugLog(category: String, detail: String) {
        clientDebugLog.append(
            ClientDebugLogEntry(
                timestampSecs: UInt64(Date().timeIntervalSince1970),
                category: category,
                detail: detail
            )
        )
        let excessCount = clientDebugLog.count - Self.maxClientDebugLogEntries
        if excessCount > 0 {
            clientDebugLog.removeFirst(excessCount)
        }
    }

    nonisolated private static func supportBundleJsonWithClientLog(
        rustJson: String,
        clientDebugLog: [ClientDebugLogEntry]
    ) -> String {
        let mergedClientLog = clientDebugLog.map(\.jsonObject) + irisClientDebugLogObjects()
        guard !mergedClientLog.isEmpty,
              let data = rustJson.data(using: .utf8),
              var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return rustJson
        }
        object["client_log"] = mergedClientLog
        guard let mergedData = try? JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys]
        ) else {
            return rustJson
        }
        return String(data: mergedData, encoding: .utf8) ?? rustJson
    }

    private func actionLogName(_ action: AppAction) -> String {
        if let label = Mirror(reflecting: action).children.first?.label {
            return label
        }
        let description = String(describing: action)
        if let payloadStart = description.firstIndex(of: "(") {
            return String(description[..<payloadStart])
        }
        return description
    }

    private func showToast(_ text: String) {
        toasts.show(text)
    }

    private func postDesktopNotifications(from oldState: AppState, to nextState: AppState) {
        guard oldState.account != nil, nextState.preferences.desktopNotificationsEnabled else {
            return
        }
        let openChatIDs = [
            activeChatID(in: oldState),
            activeChatID(in: nextState),
            nextState.currentChat?.chatId
        ].compactMap { $0 }
        let oldUnreadByChat = Dictionary(
            uniqueKeysWithValues: oldState.chatList.map { ($0.chatId, $0.unreadCount) }
        )
        for chat in nextState.chatList {
            guard !chat.isMuted else {
                continue
            }
            guard chat.lastMessageIsOutgoing == false else {
                continue
            }
            guard !openChatIDs.contains(where: { appChatID($0, matches: chat.chatId) }) else {
                continue
            }
            let previousUnread = oldUnreadByChat[chat.chatId] ?? 0
            guard chat.unreadCount > previousUnread else {
                continue
            }
            let preview = chat.lastMessagePreview?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let body = preview.isEmpty ? "New message" : preview
            desktopNotifications.post(title: chat.displayName, body: body)
        }
    }

    private func activeChatID(in state: AppState) -> String? {
        let screen = state.router.screenStack.last ?? state.router.defaultScreen
        guard case .chat(let chatID) = screen else {
            return nil
        }
        return chatID
    }

    private func appChatID(_ openChatID: String, matches candidateChatID: String) -> Bool {
        let groupPrefix = "group:"
        let open = openChatID.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidate = candidateChatID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !open.isEmpty, !candidate.isEmpty else {
            return false
        }
        if open.caseInsensitiveCompare(candidate) == .orderedSame {
            return true
        }
        if open.lowercased().hasPrefix(groupPrefix) {
            let openGroupID = String(open.dropFirst(groupPrefix.count))
            return openGroupID.caseInsensitiveCompare(candidate) == .orderedSame
        }
        if candidate.lowercased().hasPrefix(groupPrefix) {
            let candidateGroupID = String(candidate.dropFirst(groupPrefix.count))
            return open.caseInsensitiveCompare(candidateGroupID) == .orderedSame
        }
        return false
    }

    private func stageOutgoingAttachment(_ sourceURL: URL) throws -> (path: String, filename: String) {
        let accessed = sourceURL.startAccessingSecurityScopedResource()
        defer {
            if accessed {
                sourceURL.stopAccessingSecurityScopedResource()
            }
        }

        let directory = dataDir
            .appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("outgoing", isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)

        let filename = sourceURL.lastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = filename.isEmpty ? "attachment" : filename
        let destination = directory.appendingPathComponent("\(UUID().uuidString)-\(displayName)")
        if fileManager.fileExists(atPath: destination.path) {
            try fileManager.removeItem(at: destination)
        }
        try fileManager.copyItem(at: sourceURL, to: destination)
        return (destination.path, displayName)
    }

    private func downloadedAttachmentDirectory() -> URL {
        dataDir
            .appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("downloaded", isDirectory: true)
    }

    private func downloadedAttachmentURL(for attachment: MessageAttachmentSnapshot) -> URL {
        downloadedAttachmentDirectory()
            .appendingPathComponent(safeAttachmentCacheFilename(for: attachment))
    }

    private func cachedDownloadedAttachmentData(for attachment: MessageAttachmentSnapshot) -> Data? {
        let url = downloadedAttachmentURL(for: attachment)
        guard fileManager.fileExists(atPath: url.path) else {
            return nil
        }
        try? fileManager.setAttributes([.modificationDate: Date()], ofItemAtPath: url.path)
        return try? Data(contentsOf: url)
    }

    @discardableResult
    private func cachedDownloadedAttachmentURL(for attachment: MessageAttachmentSnapshot, data: Data) throws -> URL {
        let directory = downloadedAttachmentDirectory()
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)

        let destination = downloadedAttachmentURL(for: attachment)
        if fileManager.fileExists(atPath: destination.path) {
            try fileManager.removeItem(at: destination)
        }
        try data.write(to: destination, options: [.atomic])
        try pruneDownloadedAttachmentCache(protecting: destination)
        return destination
    }

    private func safeAttachmentCacheFilename(for attachment: MessageAttachmentSnapshot) -> String {
        "\(safeAttachmentFilename(attachment.nhash))-\(safeAttachmentFilename(attachment.filename))"
    }

    private func safeAttachmentFilename(_ value: String) -> String {
        let separators = CharacterSet(charactersIn: "/\\:")
        let pieces = value
            .components(separatedBy: separators)
            .joined(separator: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return pieces.isEmpty ? "attachment" : pieces
    }

    private func pruneDownloadedAttachmentCache(protecting protectedURL: URL) throws {
        let directory = downloadedAttachmentDirectory()
        let resourceKeys: Set<URLResourceKey> = [.contentModificationDateKey, .fileSizeKey, .isRegularFileKey]
        let files = try fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: Array(resourceKeys),
            options: [.skipsHiddenFiles]
        )
        var cachedFiles: [(url: URL, modified: Date, size: Int)] = []
        var totalSize = 0

        for file in files {
            let values = try file.resourceValues(forKeys: resourceKeys)
            guard values.isRegularFile == true else {
                continue
            }
            let size = values.fileSize ?? 0
            totalSize += size
            cachedFiles.append((file, values.contentModificationDate ?? .distantPast, size))
        }

        guard totalSize > Self.downloadedAttachmentCacheLimitBytes else {
            return
        }

        let protectedPath = protectedURL.standardizedFileURL.path
        for file in cachedFiles.sorted(by: { $0.modified < $1.modified }) {
            guard file.url.standardizedFileURL.path != protectedPath else {
                continue
            }
            try? fileManager.removeItem(at: file.url)
            totalSize -= file.size
            if totalSize <= Self.downloadedAttachmentCacheLimitBytes {
                break
            }
        }
    }
}

#if os(macOS)
private struct IrisReleaseManifest: Decodable {
    let tag: String
    let assets: [IrisReleaseAsset]

    func preferredMacAsset() -> IrisReleaseAsset? {
        assets.first { $0.name.hasSuffix("-macos-arm64.app.tar.gz") }
            ?? assets.first { $0.name.hasSuffix("-macos-arm64.dmg") }
    }
}

private struct IrisReleaseAsset: Decodable {
    let name: String
    let path: String
}

private struct IrisUpdateCheck {
    let manifest: IrisReleaseManifest
    let asset: IrisReleaseAsset?
    let assetUrl: URL?
    let isNewer: Bool
}

private enum IrisUpdateError: LocalizedError {
    case missingAppBundle

    var errorDescription: String? {
        switch self {
        case .missingAppBundle:
            return "Downloaded update did not contain Iris Chat.app."
        }
    }
}

private func loadIrisUpdateData(from url: URL) async throws -> Data {
    if url.isFileURL {
        return try Data(contentsOf: url)
    }
    let (data, _) = try await URLSession.shared.data(from: url)
    return data
}

private func moveIrisDownloadedUpdate(_ downloadedUrl: URL, from assetUrl: URL) throws -> URL {
    let fileName = assetUrl.lastPathComponent.isEmpty ? "iris-chat-update" : assetUrl.lastPathComponent
    let destination = FileManager.default.temporaryDirectory
        .appendingPathComponent("IrisChatDownloads", isDirectory: true)
        .appendingPathComponent(fileName)
    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
    if FileManager.default.fileExists(atPath: destination.path) {
        try FileManager.default.removeItem(at: destination)
    }
    try FileManager.default.moveItem(at: downloadedUrl, to: destination)
    return destination
}

private func irisVersionIsNewer(_ candidate: String, than current: String) -> Bool {
    // Very old dev builds used the Xcode placeholder "0.1.0". Treat that
    // (and anything with a major version below the year-style release scheme)
    // as a local build that always supersedes whatever the manifest says.
    if irisIsDevPlaceholderVersion(current) {
        return false
    }
    let left = irisVersionParts(candidate)
    let right = irisVersionParts(current)
    for index in 0..<max(left.count, right.count) {
        let leftValue = index < left.count ? left[index] : 0
        let rightValue = index < right.count ? right[index] : 0
        if leftValue != rightValue {
            return leftValue > rightValue
        }
    }
    return false
}

private func irisIsDevPlaceholderVersion(_ value: String) -> Bool {
    let parts = irisVersionParts(value)
    // Releases are tagged YYYY.M.D[.N] (year as major). Anything with a
    // major below 2000 is the Xcode template default or a hand-built dev
    // version — treat it as ahead of every release so the banner stays off.
    return (parts.first ?? 0) < 2000
}

private func irisVersionParts(_ value: String) -> [Int] {
    value
        .trimmingCharacters(in: CharacterSet(charactersIn: "vV "))
        .split { !$0.isNumber }
        .map { Int($0) ?? 0 }
}

private func runIrisUpdateProcess(_ executable: String, arguments: [String]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    try process.run()
    process.waitUntilExit()
    if process.terminationStatus != 0 {
        throw CocoaError(.executableLoad)
    }
}

private func findIrisAppBundle(in directory: URL) -> URL? {
    guard let enumerator = FileManager.default.enumerator(
        at: directory,
        includingPropertiesForKeys: [.isDirectoryKey],
        options: [.skipsHiddenFiles]
    ) else {
        return nil
    }
    for case let url as URL in enumerator where url.pathExtension == "app" {
        if url.lastPathComponent == "Iris Chat.app" || url.lastPathComponent == "IrisChatMac.app" {
            return url
        }
    }
    return nil
}

private func irisUpdateInstallScript() throws -> URL {
    let script = FileManager.default.temporaryDirectory
        .appendingPathComponent("iris-chat-install-update-\(UUID().uuidString).sh")
    let contents = """
    #!/bin/sh
    set -eu
    current_app="$1"
    new_app="$2"
    sleep 1
    rm -rf "$current_app"
    ditto "$new_app" "$current_app"
    open "$current_app"
    """
    try contents.write(to: script, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    return script
}
#endif

private func isInviteChatLink(_ url: URL) -> Bool {
    if url.pathComponents.dropFirst().first?.lowercased() == "invite",
       url.pathComponents.count >= 3 {
        return true
    }

    let fragmentComponents = chatLinkFragmentComponents(url)
    if fragmentComponents.first?.lowercased() == "invite" && fragmentComponents.count >= 2 {
        return true
    }

    guard let fragment = url.fragment else {
        return false
    }
    let decoded = fragment.removingPercentEncoding ?? fragment
    return decoded.contains("\"ephemeralKey\"") && decoded.contains("\"sharedSecret\"")
}

private func chatLinkPeerCandidates(_ url: URL) -> [String] {
    var candidates: [String] = []

    if let lastPathComponent = url.pathComponents.last,
       lastPathComponent != "/" {
        candidates.append(lastPathComponent)
    }

    if let firstFragmentComponent = chatLinkFragmentComponents(url).first {
        candidates.append(firstFragmentComponent)
    }

    if let fragment = url.fragment,
       !fragment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        candidates.append(fragment)
    }

    return candidates
}

private func chatLinkFragmentComponents(_ url: URL) -> [String] {
    guard let fragment = url.fragment else {
        return []
    }

    return fragment
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .drop(while: { $0 == "/" })
        .split(separator: "/")
        .map(String.init)
        .filter { !$0.isEmpty }
}

#if os(iOS)
private let foregroundDecryptedPushMarkerKey = "iris_foreground_decrypted_push"
private let encryptedMobilePushOuterKind = 1060
private let mobilePushGroupChatPrefix = "group:"
private let encryptedMobilePushPayloadKeys = [
    "event",
    "outer_event",
    "outer_event_json",
    "nostr_event",
    "nostr_event_json",
]

private func serializedPushPayload(userInfo: [AnyHashable: Any]) -> String? {
    serializedPushPayload(dictionary: pushPayloadDictionary(userInfo: userInfo))
}

private func serializedPushPayload(content: UNNotificationContent) -> String? {
    var dict = pushPayloadDictionary(userInfo: content.userInfo)
    if !dict.keys.contains("title") {
        dict["title"] = content.title
    }
    if !dict.keys.contains("body") {
        dict["body"] = content.body
    }
    return serializedPushPayload(dictionary: dict)
}

private func pushPayloadDictionary(userInfo: [AnyHashable: Any]) -> [String: Any] {
    var dict: [String: Any] = [:]
    for (key, value) in userInfo {
        guard let key = key as? String else {
            continue
        }
        dict[key] = value
    }
    return dict
}

private func serializedPushPayload(dictionary dict: [String: Any]) -> String? {
    guard JSONSerialization.isValidJSONObject(dict),
          let data = try? JSONSerialization.data(withJSONObject: dict),
          let json = String(data: data, encoding: .utf8) else {
        return nil
    }
    return json
}

private func isOpaqueEncryptedPush(userInfo: [AnyHashable: Any]) -> Bool {
    encryptedMobilePushPayloadKeys.contains { key in
        pushEventKind(userInfo[key]) == encryptedMobilePushOuterKind
    }
}

private func hasPushEventPayload(userInfo: [AnyHashable: Any]) -> Bool {
    encryptedMobilePushPayloadKeys.contains { userInfo[$0] != nil } ||
        userInfo["inner_event_json"] != nil ||
        userInfo["inner_event"] != nil
}

private func isGenericIrisFallback(content: UNNotificationContent) -> Bool {
    let title = content.title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let body = content.body.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let genericBody = body.isEmpty || body == "new activity" || body == "new message"
    let genericTitle = title.isEmpty ||
        title == "iris chat" ||
        title == "new activity" ||
        title == "new message" ||
        title == "someone" ||
        title.hasPrefix("dm by ")
    return genericTitle && genericBody
}

private func pushEventKind(_ value: Any?) -> Int? {
    if let dict = value as? [String: Any] {
        return normalizedPushInt(dict["kind"])
    }
    if let dict = value as? [AnyHashable: Any] {
        return normalizedPushInt(dict["kind"])
    }
    if let string = value as? String,
       let data = string.data(using: .utf8),
       let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
        return normalizedPushInt(object["kind"])
    }
    return nil
}

private func normalizedPushInt(_ value: Any?) -> Int? {
    if let intValue = value as? Int {
        return intValue
    }
    if let number = value as? NSNumber {
        return number.intValue
    }
    if let string = value as? String {
        return Int(string.trimmingCharacters(in: .whitespacesAndNewlines))
    }
    return nil
}

private func foregroundDecryptedPushUserInfo(from payloadJson: String) -> [AnyHashable: Any] {
    guard let data = payloadJson.data(using: .utf8),
          var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return [foregroundDecryptedPushMarkerKey: true]
    }
    object[foregroundDecryptedPushMarkerKey] = true
    return object
}

private func chatID(fromPushPayloadJson payloadJson: String) -> String? {
    chatIDs(fromPushPayloadJson: payloadJson).first
}

private func chatIDs(fromPushPayloadJson payloadJson: String) -> [String] {
    guard let data = payloadJson.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return []
    }
    var seen = Set<String>()
    var result: [String] = []

    func append(_ raw: String?) {
        guard let candidate = raw?.trimmingCharacters(in: .whitespacesAndNewlines),
              !candidate.isEmpty else {
            return
        }
        let key = candidate.lowercased()
        guard seen.insert(key).inserted else {
            return
        }
        result.append(candidate)
    }

    func appendGroup(_ raw: String?) {
        guard let groupID = raw?.trimmingCharacters(in: .whitespacesAndNewlines),
              !groupID.isEmpty else {
            return
        }
        if groupID.lowercased().hasPrefix(mobilePushGroupChatPrefix) {
            append(groupID)
        } else {
            append("\(mobilePushGroupChatPrefix)\(groupID)")
        }
    }

    [
        "chat_id",
        "chatId",
        "conversation_id",
        "conversationId",
        "thread_id",
        "threadId",
        "sender_pubkey",
        "senderPubkey",
        "author_pubkey",
        "authorPubkey"
    ].forEach { key in
        append(normalizedPushString(object[key]))
    }
    [
        "group_id",
        "groupId",
        "group_chat_id",
        "groupChatId"
    ].forEach { key in
        appendGroup(normalizedPushString(object[key]))
    }

    return result
}

private func normalizedPushString(_ value: Any?) -> String? {
    guard let string = value as? String else {
        return nil
    }
    let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}
#endif

final class UpdateBridge: NSObject, AppReconciler, @unchecked Sendable {
    weak var owner: AppManager?

    init(owner: AppManager) {
        self.owner = owner
    }

    func reconcile(update: AppUpdate) {
        Task { @MainActor [weak owner] in
            owner?.apply(update: update)
        }
    }
}
