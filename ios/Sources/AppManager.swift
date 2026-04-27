import Foundation
import Security
import SwiftUI

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

protocol RustAppClient: AnyObject {
    func state() -> AppState
    func dispatch(action: AppAction)
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

    func dispatch(action: AppAction) {
        ffi.dispatch(action: action)
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

    static func keychainService(environment: [String: String]) -> String {
        let base = "to.iris.chat"
        guard let runId = environment["NDR_UI_TEST_RUN_ID"], !runId.isEmpty else {
            return base
        }
        return "\(base).\(runId)"
    }

    static func dataDir(fileManager: FileManager, environment: [String: String]) -> URL {
        let suffix = environment["NDR_UI_TEST_RUN_ID"].flatMap { $0.isEmpty ? nil : $0 } ?? "iris-chat"
        // Prefer the App Group container so the Notification Service
        // Extension reads the *same* persisted ratchet state. Older
        // installs lived in the per-app `applicationSupportDirectory`,
        // so on first launch with this version migrate the legacy tree
        // into the shared container.
        let legacyBase = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let legacy = legacyBase.appendingPathComponent(suffix, isDirectory: true)
        if let shared = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier) {
            let target = shared.appendingPathComponent(suffix, isDirectory: true)
            migrateLegacyDataDir(from: legacy, to: target, fileManager: fileManager)
            return target
        }
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

    @Published private(set) var state: AppState
    @Published private(set) var bootstrapInFlight = true
    @Published var toastMessage: String?

    private let rust: RustAppClient
    private let secretStore: AccountSecretStore
    private let desktopNotifications: DesktopNotificationPosting
    private let dataDir: URL
    private let fileManager: FileManager
#if os(iOS)
    private let mobilePushRuntime = MobilePushRuntime()
#endif
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
        let resolvedSecretStore = secretStore ?? KeychainSecretStore(service: AppPaths.keychainService(environment: environment))

        if environment["NDR_UI_TEST_RESET"] == "1" {
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

        Task {
            restorePersistedSession()
        }
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
        rust.dispatch(action: .updateScreenStack(stack: stack))
    }

    func dispatch(_ action: AppAction) {
        rust.dispatch(action: action)
    }

    func handleChatLink(_ url: URL) {
        guard url.scheme?.lowercased() == "https",
              url.host?.lowercased() == "chat.iris.to" else {
            return
        }

        if isInviteChatLink(url) {
            rust.dispatch(action: .acceptInvite(inviteInput: url.absoluteString))
            return
        }

        for candidate in chatLinkPeerCandidates(url) {
            let normalized = normalizePeerInput(input: candidate)
            if !normalized.isEmpty, isValidPeerInput(input: normalized) {
                rust.dispatch(action: .createChat(peerInput: normalized))
                return
            }
        }
    }

#if os(iOS)
    func shouldSuppressPushNotification(userInfo: [AnyHashable: Any]) -> Bool {
        guard let resolution = resolvePushNotification(userInfo: userInfo),
              resolution.shouldShow,
              let chatID = chatID(fromPushPayloadJson: resolution.payloadJson) else {
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
        rust.dispatch(action: .openChat(chatId: chatID))
    }

    private func resolvePushNotification(userInfo: [AnyHashable: Any]) -> MobilePushNotificationResolution? {
        guard let payloadJson = serializedPushPayload(userInfo: userInfo) else {
            return nil
        }
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
#endif

    func setStartupAtLoginEnabled(_ enabled: Bool) {
        do {
            try PlatformStartupAtLogin.setEnabled(enabled)
            rust.dispatch(action: .setStartupAtLoginEnabled(enabled: enabled))
        } catch {
            showToast("Startup setting unavailable")
        }
    }

    func createAccount(name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        rust.dispatch(action: .createAccount(name: trimmed))
    }

    func updateProfileMetadata(name: String, pictureURL: String?) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        let trimmedPictureURL = pictureURL?.trimmingCharacters(in: .whitespacesAndNewlines)
        rust.dispatch(action: .updateProfileMetadata(
            name: trimmed,
            pictureUrl: trimmedPictureURL?.isEmpty == false ? trimmedPictureURL : nil
        ))
    }

    func restoreSession(ownerNsec: String) {
        let trimmed = ownerNsec.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        rust.dispatch(action: .restoreSession(ownerNsec: trimmed))
    }

    func startLinkedDevice(ownerInput: String) {
        let normalized = normalizePeerInput(input: ownerInput.trimmingCharacters(in: .whitespacesAndNewlines))
        guard !normalized.isEmpty, isValidPeerInput(input: normalized) else {
            return
        }
        rust.dispatch(action: .startLinkedDevice(ownerInput: normalized))
    }

    func addAuthorizedDevice(deviceInput: String) {
        let trimmed = deviceInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        rust.dispatch(action: .addAuthorizedDevice(deviceInput: trimmed))
    }

    func removeAuthorizedDevice(devicePubkeyHex: String) {
        let trimmed = devicePubkeyHex.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        rust.dispatch(action: .removeAuthorizedDevice(devicePubkeyHex: trimmed))
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
            rust.dispatch(
                action: .sendAttachment(
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
        rust.dispatch(
            action: .sendAttachments(
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
            rust.dispatch(action: .updateGroupPicture(
                groupId: groupId,
                filePath: staged.path,
                filename: staged.filename
            ))
        } catch {
            showToast("Image could not be opened")
        }
    }

    func setGroupAdmin(groupId: String, ownerPubkeyHex: String, isAdmin: Bool) {
        rust.dispatch(action: .setGroupAdmin(
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
            rust.dispatch(action: .uploadProfilePicture(filePath: staged.path))
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
        rust.exportSupportBundleJson()
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
        rust.dispatch(action: .logout)
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
        rust.dispatch(
            action: .restoreAccountBundle(
                ownerNsec: bundle.ownerNsec,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec
            )
        )
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
private func serializedPushPayload(userInfo: [AnyHashable: Any]) -> String? {
    var dict: [String: Any] = [:]
    for (key, value) in userInfo {
        guard let key = key as? String else {
            continue
        }
        dict[key] = value
    }
    guard JSONSerialization.isValidJSONObject(dict),
          let data = try? JSONSerialization.data(withJSONObject: dict),
          let json = String(data: data, encoding: .utf8) else {
        return nil
    }
    return json
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
