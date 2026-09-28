#if os(iOS)
import Foundation

/// Share-extension files can arrive while the app is running. Keep directory
/// scans, decoding and cleanup serialized away from the UI executor.
actor IrisPendingShares {
    private let containerOverride: URL?
    private let appGroupIdentifier: String
    private let fileManager: FileManager
    private var payloadURLs: [String: URL] = [:]

    init(containerOverride: URL?, appGroupIdentifier: String, fileManager: FileManager) {
        self.containerOverride = containerOverride
        self.appGroupIdentifier = appGroupIdentifier
        self.fileManager = fileManager
    }

    private var directory: URL? {
        let container = containerOverride ?? fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)
        return container?.appendingPathComponent("pending-shares", isDirectory: true)
    }

    func load(id: String) throws -> PendingShare {
        guard let directory else { throw CocoaError(.fileReadNoSuchFile) }
        return try load(url: directory.appendingPathComponent(id).appendingPathExtension("json"))
    }

    func next(excluding consumedIDs: Set<String>) -> PendingShare? {
        guard let directory,
              let urls = try? fileManager.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) else { return nil }
        let payloads = urls.filter { $0.pathExtension == "json" }.map { url in
            (url: url, date: (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast)
        }.sorted { $0.date < $1.date }
        for payload in payloads {
            guard !Task.isCancelled else { return nil }
            if let share = try? load(url: payload.url), !consumedIDs.contains(share.id) { return share }
        }
        return nil
    }

    private func load(url: URL) throws -> PendingShare {
        guard let data = fileManager.contents(atPath: url.path) else { throw CocoaError(.fileReadNoSuchFile) }
        let share = try JSONDecoder().decode(PendingShare.self, from: data)
        payloadURLs[share.id] = url
        return share
    }

    func remove(_ share: PendingShare) {
        let payload = payloadURLs.removeValue(forKey: share.id)
            ?? directory?.appendingPathComponent(share.id).appendingPathExtension("json")
        if let payload { try? fileManager.removeItem(at: payload) }
        if let files = directory?.appendingPathComponent("\(share.id)-files", isDirectory: true) {
            try? fileManager.removeItem(at: files)
        }
    }
}
#endif
