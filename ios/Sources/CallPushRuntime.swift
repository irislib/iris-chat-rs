#if os(iOS)
import Foundation
import PushKit

/// Call invitations share the message notification service, using its own
/// subscription so ordinary chat messages can never become VoIP pushes.
@MainActor
final class CallPushRuntime {
    private struct DesiredSubscription: Equatable {
        let ownerNsec: String
        let device: String
        let authors: [String]
        let enabled: Bool
        let token: String?
        let serverOverride: String?
    }

    private var task: Task<Void, Never>?
    private var retryTask: Task<Void, Never>?
    private var desired: DesiredSubscription?
    private var confirmed: DesiredSubscription?
    private let defaults: UserDefaults
    private let session: URLSession
    private let retryDelayNanoseconds: UInt64
    private let storageKey = "settings.call_push_subscription_id.ios"

    init(defaults: UserDefaults = .standard, session: URLSession = .shared,
         retryDelayNanoseconds: UInt64 = 5_000_000_000) {
        self.defaults = defaults
        self.session = session
        self.retryDelayNanoseconds = retryDelayNanoseconds
    }

    func sync(state: AppState, ownerNsec: String?, token: String?) {
        guard let ownerNsec, let device = state.mobilePush.callDevicePubkeyHex else { return }
        setDesired(DesiredSubscription(ownerNsec: ownerNsec, device: device,
            authors: state.mobilePush.callAuthorPubkeys,
            enabled: state.preferences.voiceCallsEnabled || state.preferences.videoCallsEnabled,
            token: token, serverOverride: serverOverride(state)))
    }

    func unregister(state: AppState, ownerNsec: String?) {
        guard let ownerNsec else { return }
        setDesired(DesiredSubscription(ownerNsec: ownerNsec, device: "", authors: [],
            enabled: false, token: nil, serverOverride: serverOverride(state)))
    }

    private func serverOverride(_ state: AppState) -> String? {
        nonEmptyTrimmedString(state.preferences.mobilePushServerUrl)
            ?? nonEmptyTrimmedString(ProcessInfo.processInfo.environment["IRIS_NOTIFICATION_SERVER_URL"])
    }

    private func setDesired(_ next: DesiredSubscription) {
        guard next != desired else { return }
        desired = next
        retryTask?.cancel()
        retryTask = nil
        reconcile()
    }

    private func reconcile() {
        guard task == nil, desired != confirmed else { return }
        task = Task { [weak self] in
            guard let self else { return }
            while let next = self.desired, next != self.confirmed {
                // A request may reach the server even when its response fails.
                // Reconcile serially, including when the desired state returns
                // to an earlier value while this mutation is in flight.
                self.confirmed = nil
                let success: Bool
                if !next.enabled || next.authors.isEmpty || next.token == nil {
                    success = await self.remove(ownerNsec: next.ownerNsec, override: next.serverOverride)
                } else {
                    success = await self.register(ownerNsec: next.ownerNsec, device: next.device,
                        authors: next.authors, token: next.token!, override: next.serverOverride)
                }
                if success { self.confirmed = next }
                if self.desired != next { continue }
                if !success {
                    self.retryTask = Task { [weak self, delay = self.retryDelayNanoseconds] in
                        do { try await Task.sleep(nanoseconds: delay) } catch { return }
                        guard let self else { return }
                        self.retryTask = nil
                        self.reconcile()
                    }
                    break
                }
            }
            self.task = nil
        }
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
