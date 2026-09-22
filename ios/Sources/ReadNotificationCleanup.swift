#if os(iOS)
import Foundation
import UserNotifications

@MainActor
final class ReadNotificationCleanup {
    private var task: Task<Void, Never>?
    private var pending = false
    private var owner: String?
    private var generation = 0

    func schedule(dataDir: String, bundle: StoredAccountBundle) {
        if owner != bundle.ownerPubkeyHex {
            generation += 1
            task?.cancel()
            task = nil
            owner = bundle.ownerPubkeyHex
        }
        pending = true
        guard task == nil else { return }
        let expectedGeneration = generation
        task = Task { [weak self] in
            guard let self else { return }
            repeat {
                self.pending = false
                await self.dismissRead(dataDir: dataDir, bundle: bundle)
            } while self.pending && !Task.isCancelled
            if self.generation == expectedGeneration { self.task = nil }
        }
    }

    func dismissRead(dataDir: String, bundle: StoredAccountBundle) async {
        let center = UNUserNotificationCenter.current()
        let delivered = await center.deliveredNotifications()
        let candidates: [(String, String)] = delivered.compactMap { notification in
            let info = notification.request.content.userInfo
            guard JSONSerialization.isValidJSONObject(info),
                  let data = try? JSONSerialization.data(withJSONObject: info),
                  let json = String(data: data, encoding: .utf8) else { return nil }
            return (notification.request.identifier, json)
        }
        guard !candidates.isEmpty, !Task.isCancelled else { return }
        let payloads = candidates.map { $0.1 }
        let owner = bundle.ownerPubkeyHex
        let device = bundle.deviceNsec
        let indexes = await Task.detached(priority: .utility) {
            readMobilePushNotificationIndexes(
                dataDir: dataDir, ownerPubkeyHex: owner,
                deviceNsec: device, payloads: payloads
            )
        }.value
        guard !Task.isCancelled else { return }
        let identifiers = indexes.compactMap { index -> String? in
            guard index < UInt64(candidates.count) else { return nil }
            return candidates[Int(index)].0
        }
        center.removeDeliveredNotifications(withIdentifiers: identifiers)
    }
}
#endif
