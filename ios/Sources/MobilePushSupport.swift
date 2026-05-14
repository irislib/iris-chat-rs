#if os(iOS)
import Foundation
import UIKit
import UserNotifications

final class IrisPushAppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    weak var manager: AppManager?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        guard !AppPaths.notificationsDisabledForAutomation(
            environment: ProcessInfo.processInfo.environment
        ) else {
            return true
        }
#if targetEnvironment(simulator)
        return true
#else
        UNUserNotificationCenter.current().delegate = self
        return true
#endif
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        MobilePushTokenCenter.shared.setApnsToken(deviceToken.map { String(format: "%02x", $0) }.joined())
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        MobilePushTokenCenter.shared.setApnsToken(nil)
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
        manager?.appBackgrounded()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        guard let manager else {
            return [.banner, .sound, .list]
        }
        return await manager.foregroundPushPresentationOptions(
            content: notification.request.content
        )
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        manager?.handlePushNotificationTap(userInfo: response.notification.request.content.userInfo)
    }
}

@MainActor
final class MobilePushTokenCenter {
    static let shared = MobilePushTokenCenter()

    private var apnsToken: String?
    private var waiters: [CheckedContinuation<String?, Never>] = []

    func setApnsToken(_ token: String?) {
        let normalized = token?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        apnsToken = normalized
        guard let normalized else {
            return
        }
        let pending = waiters
        waiters.removeAll()
        pending.forEach { $0.resume(returning: normalized) }
    }

    func currentApnsToken() -> String? {
        apnsToken
    }

    func waitForApnsToken(timeoutNanoseconds: UInt64) async -> String? {
        if let apnsToken {
            return apnsToken
        }
        return await withTaskGroup(of: String?.self) { group in
            group.addTask { @MainActor in
                await withCheckedContinuation { continuation in
                    self.waiters.append(continuation)
                }
            }
            group.addTask {
                try? await Task.sleep(nanoseconds: timeoutNanoseconds)
                return nil
            }
            let result = await group.next() ?? nil
            group.cancelAll()
            return result
        }
    }
}

final class MobilePushRuntime {
    private let userDefaults: UserDefaults
    private let urlSession: URLSession
    private var lastSyncSignature: String?
    private var currentSyncTask: Task<Void, Never>?

    init(userDefaults: UserDefaults = .standard, urlSession: URLSession = .shared) {
        self.userDefaults = userDefaults
        self.urlSession = urlSession
    }

    @MainActor
    func sync(state: AppState, ownerNsec: String?) {
        guard !AppPaths.notificationsDisabledForAutomation(
            environment: ProcessInfo.processInfo.environment
        ) else {
            return
        }
#if targetEnvironment(simulator)
        return
#else
        let owner = state.mobilePush.ownerPubkeyHex?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        let ownerSecret = ownerNsec?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        let authors = state.mobilePush.messageAuthorPubkeys
        let inviteResponses = state.mobilePush.inviteResponsePubkeys
        let enabled = state.preferences.desktopNotificationsEnabled
        let userServerOverride = state.preferences.mobilePushServerUrl.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        let serverOverride = userServerOverride ?? mobilePushBuildServerOverride
        let signature = [
            enabled ? "1" : "0",
            owner ?? "",
            ownerSecret == nil ? "0" : "1",
            authors.joined(separator: ","),
            inviteResponses.joined(separator: ","),
            serverOverride ?? "",
        ].joined(separator: "|")

        guard signature != lastSyncSignature else {
            return
        }
        lastSyncSignature = signature
        currentSyncTask?.cancel()
        currentSyncTask = Task { [weak self] in
            await self?.sync(
                enabled: enabled,
                ownerNsec: ownerSecret,
                messageAuthorPubkeys: authors,
                inviteResponsePubkeys: inviteResponses,
                serverOverride: serverOverride
            )
        }
#endif
    }

    @MainActor
    func unregisterStoredSubscription(state: AppState, ownerNsec: String?) {
        let storageKey = mobilePushSubscriptionIdKey(platformKey: "ios")
        let userServerOverride = state.preferences.mobilePushServerUrl.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        let serverOverride = userServerOverride ?? mobilePushBuildServerOverride
        let ownerSecret = ownerNsec?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        lastSyncSignature = nil
        currentSyncTask?.cancel()
        currentSyncTask = Task { [weak self] in
            await self?.disableStoredSubscription(
                ownerNsec: ownerSecret,
                storageKey: storageKey,
                serverOverride: serverOverride
            )
        }
    }

    private func sync(
        enabled: Bool,
        ownerNsec: String?,
        messageAuthorPubkeys: [String],
        inviteResponsePubkeys: [String],
        serverOverride: String?
    ) async {
        let storageKey = mobilePushSubscriptionIdKey(platformKey: "ios")
        guard enabled,
              let ownerNsec,
              !messageAuthorPubkeys.isEmpty || !inviteResponsePubkeys.isEmpty else {
            await disableStoredSubscription(ownerNsec: ownerNsec, storageKey: storageKey, serverOverride: serverOverride)
            return
        }

        guard let token = await requestApnsToken() else {
            return
        }

        let storedId = userDefaults.string(forKey: storageKey)?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        let existingId = await resolveExistingSubscriptionId(
            ownerNsec: ownerNsec,
            pushToken: token,
            storedId: storedId,
            serverOverride: serverOverride
        )
        if let existingId,
           await updateSubscription(
               ownerNsec: ownerNsec,
               subscriptionId: existingId,
               pushToken: token,
               messageAuthorPubkeys: messageAuthorPubkeys,
               inviteResponsePubkeys: inviteResponsePubkeys,
               storageKey: storageKey,
               serverOverride: serverOverride
           ) {
            return
        }

        await createSubscription(
            ownerNsec: ownerNsec,
            pushToken: token,
            messageAuthorPubkeys: messageAuthorPubkeys,
            inviteResponsePubkeys: inviteResponsePubkeys,
            storageKey: storageKey,
            serverOverride: serverOverride
        )
    }

    private func requestApnsToken() async -> String? {
#if targetEnvironment(simulator)
        return nil
#else
        guard AppPaths.testRunId(environment: ProcessInfo.processInfo.environment) == nil else {
            return nil
        }
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        var status = settings.authorizationStatus
        if status == .notDetermined {
            do {
                let options: UNAuthorizationOptions = [.alert, .badge, .sound]
                let granted = try await center.requestAuthorization(options: options)
                status = granted ? .authorized : .denied
            } catch {
                return nil
            }
        }
        guard status == .authorized || status == .provisional || status == .ephemeral else {
            return nil
        }

        await MainActor.run {
            UIApplication.shared.registerForRemoteNotifications()
        }
        if let token = await MainActor.run(body: {
            MobilePushTokenCenter.shared.currentApnsToken()
        }) {
            return token
        }
        return await MobilePushTokenCenter.shared.waitForApnsToken(timeoutNanoseconds: 15_000_000_000)
#endif
    }

    private func resolveExistingSubscriptionId(
        ownerNsec: String,
        pushToken: String,
        storedId: String?,
        serverOverride: String?
    ) async -> String? {
        guard let request = buildMobilePushListSubscriptionsRequest(
            ownerNsec: ownerNsec,
            platformKey: "ios",
            isRelease: isMobilePushReleaseBuild,
            serverUrlOverride: serverOverride
        ) else {
            return storedId
        }
        guard let data = await perform(request).data else {
            return storedId
        }
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return storedId
        }
        if let storedId, object[storedId] != nil {
            return storedId
        }
        for (subscriptionId, value) in object {
            guard let subscription = value as? [String: Any],
                  let tokens = subscription["apns_tokens"] as? [String],
                  tokens.contains(pushToken) else {
                continue
            }
            return subscriptionId
        }
        return nil
    }

    private func updateSubscription(
        ownerNsec: String,
        subscriptionId: String,
        pushToken: String,
        messageAuthorPubkeys: [String],
        inviteResponsePubkeys: [String],
        storageKey: String,
        serverOverride: String?
    ) async -> Bool {
        guard let request = buildMobilePushUpdateSubscriptionRequest(
            ownerNsec: ownerNsec,
            subscriptionId: subscriptionId,
            platformKey: "ios",
            pushToken: pushToken,
            apnsTopic: Bundle.main.bundleIdentifier,
            messageAuthorPubkeys: messageAuthorPubkeys,
            inviteResponsePubkeys: inviteResponsePubkeys,
            isRelease: isMobilePushReleaseBuild,
            serverUrlOverride: serverOverride
        ) else {
            return false
        }
        let response = await perform(request)
        if response.isSuccess {
            userDefaults.set(subscriptionId, forKey: storageKey)
            return true
        }
        if response.statusCode == 404 {
            userDefaults.removeObject(forKey: storageKey)
        }
        return false
    }

    private func createSubscription(
        ownerNsec: String,
        pushToken: String,
        messageAuthorPubkeys: [String],
        inviteResponsePubkeys: [String],
        storageKey: String,
        serverOverride: String?
    ) async {
        guard let request = buildMobilePushCreateSubscriptionRequest(
            ownerNsec: ownerNsec,
            platformKey: "ios",
            pushToken: pushToken,
            apnsTopic: Bundle.main.bundleIdentifier,
            messageAuthorPubkeys: messageAuthorPubkeys,
            inviteResponsePubkeys: inviteResponsePubkeys,
            isRelease: isMobilePushReleaseBuild,
            serverUrlOverride: serverOverride
        ) else {
            return
        }
        let response = await perform(request)
        guard response.isSuccess,
              let data = response.data,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = object["id"] as? String,
              !id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        userDefaults.set(id, forKey: storageKey)
    }

    private func disableStoredSubscription(ownerNsec: String?, storageKey: String, serverOverride: String?) async {
        guard let storedId = userDefaults.string(forKey: storageKey)?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty else {
            return
        }
        guard let ownerNsec,
              let request = buildMobilePushDeleteSubscriptionRequest(
                  ownerNsec: ownerNsec,
                  subscriptionId: storedId,
                  platformKey: "ios",
                  isRelease: isMobilePushReleaseBuild,
                  serverUrlOverride: serverOverride
              ) else {
            userDefaults.removeObject(forKey: storageKey)
            return
        }
        let response = await perform(request)
        if response.isSuccess || response.statusCode == 404 {
            userDefaults.removeObject(forKey: storageKey)
        }
    }

    private func perform(_ request: MobilePushSubscriptionRequest) async -> MobilePushHTTPResponse {
        guard let url = URL(string: request.url) else {
            return MobilePushHTTPResponse(statusCode: 0, data: nil)
        }
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method
        urlRequest.setValue("application/json", forHTTPHeaderField: "accept")
        urlRequest.setValue(request.authorizationHeader, forHTTPHeaderField: "authorization")
        if let body = request.bodyJson {
            urlRequest.setValue("application/json", forHTTPHeaderField: "content-type")
            urlRequest.httpBody = Data(body.utf8)
        }
        do {
            let (data, response) = try await urlSession.data(for: urlRequest)
            let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
            return MobilePushHTTPResponse(statusCode: statusCode, data: data)
        } catch {
            return MobilePushHTTPResponse(statusCode: 0, data: nil)
        }
    }
}

private struct MobilePushHTTPResponse {
    let statusCode: Int
    let data: Data?

    var isSuccess: Bool {
        (200..<300).contains(statusCode)
    }
}

private var isMobilePushReleaseBuild: Bool {
#if DEBUG
    false
#else
    true
#endif
}

private var mobilePushBuildServerOverride: String? {
    ProcessInfo.processInfo.environment["IRIS_NOTIFICATION_SERVER_URL"]?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
#endif
