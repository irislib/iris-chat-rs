import Foundation

/// Preview decryption can load persisted ratchets, wait for SQLite, and sleep
/// while the live core catches up. Never run it on the UI executor.
final class MobilePushNotificationResolver: @unchecked Sendable {
    typealias Resolve = @Sendable (String, StoredAccountBundle?, String) -> MobilePushNotificationResolution

    private let queue = DispatchQueue(label: "iris.push-preview", qos: .userInitiated)
    private let decrypt: Resolve
#if os(iOS)
    @MainActor var beginBackgroundTask: () -> IrisSuspendBackgroundTask = { IrisSuspendBackgroundTask() }
#endif

    init(decrypt: @escaping Resolve = { dataDir, bundle, payload in
        if let bundle {
            return decryptMobilePushNotificationPayload(
                dataDir: dataDir, ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec, rawPayloadJson: payload
            )
        }
        return resolveMobilePushNotificationPayload(rawPayloadJson: payload)
    }) {
        self.decrypt = decrypt
    }

    @MainActor
    func resolve(dataDir: String, bundle: StoredAccountBundle?, payloadJson: String) async -> MobilePushNotificationResolution? {
#if os(iOS)
        // A notification tap can start decryption before the scene activates,
        // and the phone may lock again while this synchronous Rust read runs.
        let allowance = beginBackgroundTask()
        defer { allowance.finish() }
        guard allowance.isActive else { return nil }
#endif
        let resolution = await withCheckedContinuation { continuation in
            queue.async { [decrypt] in
                continuation.resume(returning: decrypt(dataDir, bundle, payloadJson))
            }
        }
#if os(iOS)
        guard allowance.isActive else { return nil }
#endif
        return resolution
    }

    func waitUntilIdle() async {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume() }
        }
    }
}
