import Foundation

/// The only cross-thread client calls here read an epoch or cancel cold restore;
/// neither waits for the core queue or the SQLite writer. An early expiration
/// stays latched until the client exists, without claiming constructor coverage.
final class IrisStartupRestoreCancellation: @unchecked Sendable {
    private enum Phase: Equatable { case pending, expired, finished }
    private let lock = NSLock()
    private var phase = Phase.pending
    private var bound = false
    private var request: (client: RustAppClient, epoch: UInt64)?

    func bind(to client: RustAppClient) {
        lock.withLock {
            guard !bound else { return }
            bound = true
            guard phase != .finished, let epoch = client.startupRestoreEpoch() else { return }
            if phase == .expired {
                client.cancelStartupRestore(epoch: epoch)
            } else {
                request = (client, epoch)
            }
        }
    }

    func expire() {
        lock.withLock {
            guard phase == .pending else { return }
            phase = .expired
            if let request {
                self.request = nil
                request.client.cancelStartupRestore(epoch: request.epoch)
            }
        }
    }

    func finish() {
        lock.withLock {
            // Expiration can finish the UIKit allowance before native binding.
            // Preserve that pending cancellation; normal completion disarms it.
            if phase == .pending { phase = .finished }
            request = nil
        }
    }
}
