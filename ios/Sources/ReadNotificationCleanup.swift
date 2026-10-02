#if os(iOS)
import Foundation
import UserNotifications

@MainActor
final class ReadNotificationCleanup {
    private struct Request: Equatable {
        let dataDir: String
        let bundle: StoredAccountBundle
    }

    private var task: Task<Void, Never>?
    private var pending: Request?
    private var latest: Request?
    private let delivered: () async -> [(String, String)]
    private let resolve: @Sendable (String, StoredAccountBundle, [String]) -> [UInt64]
    private let remove: ([String]) -> Void
    private let beginBackgroundTask: @MainActor () -> IrisSuspendBackgroundTask

    init(
        delivered: @escaping () async -> [(String, String)] = {
            await UNUserNotificationCenter.current().deliveredNotifications().compactMap { notification in
                let info = notification.request.content.userInfo
                guard JSONSerialization.isValidJSONObject(info),
                      let data = try? JSONSerialization.data(withJSONObject: info),
                      let json = String(data: data, encoding: .utf8) else { return nil }
                return (notification.request.identifier, json)
            }
        },
        resolve: @escaping @Sendable (String, StoredAccountBundle, [String]) -> [UInt64] = { dataDir, bundle, payloads in
            readMobilePushNotificationIndexes(
                dataDir: dataDir, ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec, payloads: payloads
            )
        },
        remove: @escaping ([String]) -> Void = {
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: $0)
        },
        beginBackgroundTask: @escaping @MainActor () -> IrisSuspendBackgroundTask = {
            IrisSuspendBackgroundTask()
        }
    ) {
        self.delivered = delivered
        self.resolve = resolve
        self.remove = remove
        self.beginBackgroundTask = beginBackgroundTask
    }

    func schedule(dataDir: String, bundle: StoredAccountBundle) {
        let request = Request(dataDir: dataDir, bundle: bundle)
        latest = request
        pending = request
        guard task == nil else { return }
        task = Task { [weak self] in
            guard let self else { return }
            // Every entry point joins this worker. Cancelling a Swift task does
            // not stop an in-flight synchronous Rust call, even on account changes.
            while let request = self.pending {
                self.pending = nil
                guard await self.dismissRead(request) else { break }
            }
            self.pending = nil
            self.task = nil
        }
    }

    func dismissRead(dataDir: String, bundle: StoredAccountBundle) async {
        schedule(dataDir: dataDir, bundle: bundle)
        await task?.value
    }

    private func dismissRead(_ request: Request) async -> Bool {
        // State updates and silent pushes can arrive after the core's suspension
        // flush. Keep a separate allowance until all notification database reads
        // finish, including the detached Rust work.
        let allowance = beginBackgroundTask()
        defer { allowance.finish() }
        guard allowance.isActive else { return false }
        let candidates = await delivered()
        guard allowance.isActive else { return false }
        guard !candidates.isEmpty, latest == request else { return true }
        let payloads = candidates.map { $0.1 }
        let resolve = self.resolve
        let indexes = await Task.detached(priority: .utility) {
            resolve(request.dataDir, request.bundle, payloads)
        }.value
        guard allowance.isActive else { return false }
        guard latest == request else { return true }
        let identifiers = indexes.compactMap { index -> String? in
            guard index < UInt64(candidates.count) else { return nil }
            return candidates[Int(index)].0
        }
        if !identifiers.isEmpty { remove(identifiers) }
        return true
    }
}
#endif
