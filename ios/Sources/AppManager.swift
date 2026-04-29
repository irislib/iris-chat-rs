import Foundation
import Security
import SwiftUI
#if os(iOS)
import UserNotifications
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

private struct ClientDebugLogEntry {
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
    func clear()
}

final class KeychainSecretStore: AccountSecretStore {
    private let service: String
    private let account: String
    private let accessGroup: String?

    init(
        service: String = "to.iris.chat",
        account: String = "stored-account-bundle",
        accessGroup: String? = nil
    ) {
        self.service = service
        self.account = account
        self.accessGroup = accessGroup
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
        let update: [CFString: Any] = [kSecValueData: data]
        let updateStatus = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if updateStatus == errSecItemNotFound {
            var insert = query
            insert[kSecValueData] = data
            SecItemAdd(insert as CFDictionary, nil)
        }
    }

    func clear() {
        SecItemDelete(baseQuery() as CFDictionary)
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
        } catch {
            NSLog("Iris Chat file secret save failed: %@", "\(error)")
        }
    }

    func clear() {
        try? fileManager.removeItem(at: url)
    }
}

protocol RustAppClient: AnyObject {
    func state() -> AppState
    func dispatch(action: AppAction) throws
    func ingestNearbyEventJson(eventJson: String) -> Bool
    func exportSupportBundleJson() -> String
    func listenForUpdates(reconciler: AppReconciler)
}

final class LiveRustAppClient: RustAppClient {
    private let ffi: FfiApp

    init(dataDir: String, appVersion: String) {
        self.ffi = FfiApp(dataDir: dataDir, keychainGroup: "", appVersion: appVersion)
    }

    func state() -> AppState {
        ffi.state()
    }

    func dispatch(action: AppAction) throws {
        try ffi.dispatchSafely(action: action)
    }

    func ingestNearbyEventJson(eventJson: String) -> Bool {
        ffi.ingestNearbyEventJson(eventJson: eventJson)
    }

    func exportSupportBundleJson() -> String {
        ffi.exportSupportBundleJson()
    }

    func listenForUpdates(reconciler: AppReconciler) {
        ffi.listenForUpdates(reconciler: reconciler)
    }
}

enum AppPaths {
    static let appGroupIdentifier = "group.to.iris.chat"

    static func appVersion(bundle: Bundle = .main) -> String {
        bundle.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.0"
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
#if os(macOS)
        if testRunId(environment: environment) != nil || environment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] == "1" {
            return FileAccountSecretStore(
                url: dataDir.appendingPathComponent("account-secret.json"),
                fileManager: fileManager
            )
        }
#endif
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
}

@MainActor
final class AppManager: ObservableObject {
    private static let downloadedAttachmentCacheLimitBytes = 128 * 1024 * 1024
    private static let maxClientDebugLogEntries = 50
    private static let dispatchFailureToast = "Action failed. Copy support bundle in Settings."

    @Published private(set) var state: AppState
    @Published private(set) var bootstrapInFlight = true
    @Published var toastMessage: String?

    private let rust: RustAppClient
    private let secretStore: AccountSecretStore
    private let desktopNotifications: DesktopNotificationPosting
    private let dataDir: URL
    private let fileManager: FileManager
#if os(macOS)
    let nearbyBitchat = MacBitchatNearbyService()
#endif
#if os(iOS) || os(macOS)
    let nearbyIris = IrisNearbyService()
#endif
#if os(iOS)
    private let mobilePushRuntime = MobilePushRuntime()
#endif
    private var clientDebugLog: [ClientDebugLogEntry] = []
    private var lastRevApplied: UInt64
    private lazy var reconciler = UpdateBridge(owner: self)

    init(
        rust: RustAppClient? = nil,
        secretStore: AccountSecretStore? = nil,
        desktopNotifications: DesktopNotificationPosting? = nil,
        dataDir: URL? = nil,
        fileManager: FileManager = .default,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        appVersion: String = AppPaths.appVersion()
    ) {
        self.fileManager = fileManager
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
        try? fileManager.createDirectory(at: resolvedDataDir, withIntermediateDirectories: true)

        let resolvedRust = rust ?? LiveRustAppClient(dataDir: resolvedDataDir.path, appVersion: appVersion)
        let initialState = resolvedRust.state()

        self.rust = resolvedRust
        self.secretStore = resolvedSecretStore
        self.desktopNotifications = desktopNotifications ?? SystemDesktopNotificationPoster()
        self.dataDir = resolvedDataDir
        self.state = initialState
        self.lastRevApplied = initialState.rev

        resolvedRust.listenForUpdates(reconciler: reconciler)

#if os(iOS) || os(macOS)
        nearbyIris.ingestEventJson = { [weak self] eventJson in
            self?.rust.ingestNearbyEventJson(eventJson: eventJson) ?? false
        }
#endif

        Task {
            restorePersistedSession()
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
    }

    var activeScreen: Screen {
        state.router.screenStack.last ?? state.router.defaultScreen
    }

    var canNavigateBack: Bool {
        !state.router.screenStack.isEmpty
    }

    func navigateBack() {
        guard !state.router.screenStack.isEmpty else {
            return
        }
        var stack = state.router.screenStack
        _ = stack.removeLast()
        dispatchToRust(.updateScreenStack(stack: stack))
    }

    func dispatch(_ action: AppAction) {
        dispatchToRust(action)
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

#if os(iOS)
    func foregroundPushPresentationOptions(
        content: UNNotificationContent
    ) async -> UNNotificationPresentationOptions {
        let userInfo = content.userInfo
        if userInfo[foregroundDecryptedPushMarkerKey] as? Bool == true {
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
        if let chatID = chatID(fromPushPayloadJson: resolution.payloadJson),
           state.currentChat?.chatId.caseInsensitiveCompare(chatID) == .orderedSame {
            return []
        }
        await postForegroundDecryptedPush(resolution: resolution)
        return []
    }

    func foregroundPushPresentationOptions(
        userInfo: [AnyHashable: Any]
    ) async -> UNNotificationPresentationOptions {
        if userInfo[foregroundDecryptedPushMarkerKey] as? Bool == true {
            return [.banner, .sound, .list]
        }
        guard let resolution = resolvePushNotification(userInfo: userInfo) else {
            return fallbackForegroundPushPresentationOptions(userInfo: userInfo)
        }
        guard resolution.shouldShow else {
            return []
        }
        if let chatID = chatID(fromPushPayloadJson: resolution.payloadJson),
           state.currentChat?.chatId.caseInsensitiveCompare(chatID) == .orderedSame {
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
        guard let chatID = chatID(fromPushPayloadJson: resolution.payloadJson) else {
            return false
        }
        return state.currentChat?.chatId.caseInsensitiveCompare(chatID) == .orderedSame
    }

    func handlePushNotificationTap(userInfo: [AnyHashable: Any]) {
        guard let resolution = resolvePushNotification(userInfo: userInfo),
              let chatID = chatID(fromPushPayloadJson: resolution.payloadJson),
              !chatID.isEmpty else {
            return
        }
        dispatchToRust(.openChat(chatId: chatID))
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

    func createAccount(name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        dispatchToRust(.createAccount(name: trimmed))
    }

    func updateProfileMetadata(name: String, pictureURL: String?) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        let trimmedPictureURL = pictureURL?.trimmingCharacters(in: .whitespacesAndNewlines)
        dispatchToRust(.updateProfileMetadata(
            name: trimmed,
            pictureUrl: trimmedPictureURL?.isEmpty == false ? trimmedPictureURL : nil
        ))
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
        print("[upload-profile-picture] picked: \(fileURL.path)")
        do {
            let staged = try stageOutgoingAttachment(fileURL)
            print("[upload-profile-picture] staged: \(staged.path)")
            dispatchToRust(.uploadProfilePicture(filePath: staged.path))
        } catch {
            print("[upload-profile-picture] stage failed: \(error)")
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
        supportBundleJsonWithClientLog(rust.exportSupportBundleJson())
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
        mobilePushRuntime.unregisterStoredSubscription(state: state, ownerNsec: secretStore.load()?.ownerNsec)
#endif
        dispatchToRust(.logout)
        secretStore.clear()
        try? fileManager.removeItem(at: dataDir)
        try? fileManager.createDirectory(at: dataDir, withIntermediateDirectories: true)
        apply(update: .fullState(rust.state()))
    }

    func apply(update: AppUpdate) {
        switch update {
        case .persistAccountBundle(_, let ownerNsec, let ownerPubkeyHex, let deviceNsec):
            // Secure persistence is a shell side effect and must be applied even if snapshot revs race.
            secretStore.save(
                StoredAccountBundle(
                    ownerNsec: ownerNsec,
                    ownerPubkeyHex: ownerPubkeyHex,
                    deviceNsec: deviceNsec
                )
            )
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
            // Rust owns authoritative state. The shell only accepts the newest full snapshot.
            guard nextState.rev > lastRevApplied else {
                return
            }
            lastRevApplied = nextState.rev
            postDesktopNotifications(from: state, to: nextState)
            state = nextState
#if os(iOS)
            mobilePushRuntime.sync(state: nextState, ownerNsec: secretStore.load()?.ownerNsec)
#endif
            bootstrapInFlight = false
            if let toast = nextState.toast, !toast.isEmpty {
                showToast(toast)
            }
        }
    }

    private func restorePersistedSession() {
        // Native restore only rehydrates secure inputs. Rust rebuilds the authoritative app state.
        defer {
            bootstrapInFlight = false
        }
        guard let bundle = secretStore.load() else {
            return
        }
        dispatchToRust(
            .restoreAccountBundle(
                ownerNsec: bundle.ownerNsec,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec
            )
        )
    }

    @discardableResult
    private func dispatchToRust(
        _ action: AppAction,
        showsToastOnFailure: Bool = true
    ) -> Bool {
        do {
            try rust.dispatch(action: action)
            return true
        } catch {
            logDispatchFailure(action: action, error: error)
            if showsToastOnFailure {
                showToast(Self.dispatchFailureToast)
            }
            return false
        }
    }

    private func logDispatchFailure(action: AppAction, error: Error) {
        let actionName = actionLogName(action)
        let message = "Iris Chat FFI dispatch failed (\(actionName)): \(error)"
        appendClientDebugLog(category: "ffi.dispatch.failed", detail: "\(actionName): \(error)")
        NSLog("%@", message)
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

    private func supportBundleJsonWithClientLog(_ rustJson: String) -> String {
        guard !clientDebugLog.isEmpty,
              let data = rustJson.data(using: .utf8),
              var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return rustJson
        }
        object["client_log"] = clientDebugLog.map(\.jsonObject)
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
        toastMessage = text
        let message = text
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
            guard self?.toastMessage == message else {
                return
            }
            self?.toastMessage = nil
        }
    }

    private func postDesktopNotifications(from oldState: AppState, to nextState: AppState) {
        guard oldState.account != nil, nextState.preferences.desktopNotificationsEnabled else {
            return
        }
        let oldUnreadByChat = Dictionary(
            uniqueKeysWithValues: oldState.chatList.map { ($0.chatId, $0.unreadCount) }
        )
        for chat in nextState.chatList {
            guard chat.lastMessageIsOutgoing == false else {
                continue
            }
            guard chat.chatId != nextState.currentChat?.chatId else {
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

private func isInviteChatLink(_ url: URL) -> Bool {
    if url.pathComponents.dropFirst().first?.lowercased() == "invite",
       url.pathComponents.count >= 3 {
        return true
    }

    let fragmentComponents = chatLinkFragmentComponents(url)
    return fragmentComponents.first?.lowercased() == "invite" && fragmentComponents.count >= 2
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
    pushEventKind(userInfo["event"]) == encryptedMobilePushOuterKind
        || pushEventKind(userInfo["inner_event_json"]) == encryptedMobilePushOuterKind
}

private func hasPushEventPayload(userInfo: [AnyHashable: Any]) -> Bool {
    userInfo["event"] != nil || userInfo["inner_event_json"] != nil || userInfo["inner_event"] != nil
}

private func isGenericIrisFallback(content: UNNotificationContent) -> Bool {
    let title = content.title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let body = content.body.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    return title == "iris chat" && (body.isEmpty || body == "new activity" || body == "new message")
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
    guard let data = payloadJson.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return nil
    }
    if let groupID = normalizedPushString(object["group_id"]) {
        return groupID
    }
    if let sender = normalizedPushString(object["sender_pubkey"]) {
        return sender
    }
    return nil
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
