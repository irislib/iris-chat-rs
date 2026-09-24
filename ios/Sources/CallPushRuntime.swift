#if os(iOS)
import Foundation
import PushKit

/// Call invitations share the message notification service, using its own
/// subscription so ordinary chat messages can never become VoIP pushes.
@MainActor
final class CallPushRuntime {
    private var task: Task<Void, Never>?
    private var signature: String?
    private var pendingSignature: String?
    private let defaults: UserDefaults
    private let session: URLSession
    private let storageKey = "settings.call_push_subscription_id.ios"

    init(defaults: UserDefaults = .standard, session: URLSession = .shared) {
        self.defaults = defaults
        self.session = session
    }

    func sync(state: AppState, ownerNsec: String?, token: String?) {
        guard let ownerNsec, let device = state.mobilePush.callDevicePubkeyHex else { return }
        let authors = state.mobilePush.callAuthorPubkeys
        let enabled = state.preferences.voiceCallsEnabled || state.preferences.videoCallsEnabled
        let override = nonEmptyTrimmedString(state.preferences.mobilePushServerUrl)
            ?? nonEmptyTrimmedString(ProcessInfo.processInfo.environment["IRIS_NOTIFICATION_SERVER_URL"])
        let next = [device, authors.joined(separator: ","), enabled ? "1" : "0", token ?? "", override ?? ""].joined(separator: "|")
        guard next != signature, next != pendingSignature else { return }
        pendingSignature = next
        task?.cancel()
        task = Task { [weak self] in
            guard let self else { return }
            let success: Bool
            if !enabled || authors.isEmpty || token == nil {
                success = await self.remove(ownerNsec: ownerNsec, override: override)
            } else {
                success = await self.register(ownerNsec: ownerNsec, device: device,
                    authors: authors, token: token!, override: override)
            }
            guard !Task.isCancelled, self.pendingSignature == next else { return }
            self.pendingSignature = nil
            if success { self.signature = next }
            else {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard !Task.isCancelled else { return }
                self.sync(state: state, ownerNsec: ownerNsec, token: token)
            }
        }
    }

    func unregister(state: AppState, ownerNsec: String?) {
        task?.cancel(); signature = nil; pendingSignature = nil
        guard let ownerNsec else { return }
        let override = nonEmptyTrimmedString(state.preferences.mobilePushServerUrl)
            ?? nonEmptyTrimmedString(ProcessInfo.processInfo.environment["IRIS_NOTIFICATION_SERVER_URL"])
        task = Task { [weak self] in _ = await self?.remove(ownerNsec: ownerNsec, override: override) }
    }

    private func register(ownerNsec: String, device: String, authors: [String], token: String, override: String?) async -> Bool {
        let id = defaults.string(forKey: storageKey)
        guard let request = buildCallPushSubscriptionRequest(ownerNsec: ownerNsec,
            devicePubkeyHex: device, authorPubkeys: authors, subscriptionId: id,
            platformKey: "ios", pushToken: token, apnsTopic: Bundle.main.bundleIdentifier,
            isRelease: isRelease, serverUrlOverride: override) else { return false }
        let (status, data) = await perform(request)
        if status == 404, id != nil {
            defaults.removeObject(forKey: storageKey)
            return await register(ownerNsec: ownerNsec, device: device, authors: authors, token: token, override: override)
        }
        guard (200..<300).contains(status) else { return false }
        if id == nil {
            guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let newID = nonEmptyTrimmedString(object["id"] as? String) else { return false }
            defaults.set(newID, forKey: storageKey)
        }
        return true
    }

    private func remove(ownerNsec: String, override: String?) async -> Bool {
        guard let id = defaults.string(forKey: storageKey) else { return true }
        guard let request = buildMobilePushDeleteSubscriptionRequest(ownerNsec: ownerNsec,
            subscriptionId: id, platformKey: "ios", isRelease: isRelease,
            serverUrlOverride: override) else { return false }
        let (status, _) = await perform(request)
        guard (200..<300).contains(status) || status == 404 else { return false }
        defaults.removeObject(forKey: storageKey)
        return true
    }

    private func perform(_ request: MobilePushSubscriptionRequest) async -> (Int, Data) {
        guard let url = URL(string: request.url) else { return (0, Data()) }
        var http = URLRequest(url: url, timeoutInterval: 15)
        http.httpMethod = request.method
        http.setValue(request.authorizationHeader, forHTTPHeaderField: "authorization")
        http.setValue("application/json", forHTTPHeaderField: "content-type")
        http.httpBody = request.bodyJson.map { Data($0.utf8) }
        do {
            let (data, response) = try await session.data(for: http)
            return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
        } catch { return (0, Data()) }
    }

    private var isRelease: Bool {
#if DEBUG
        false
#else
        true
#endif
    }
}
#endif
