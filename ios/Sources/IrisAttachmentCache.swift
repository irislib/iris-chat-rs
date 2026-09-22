import Foundation

/// Serializes cache I/O away from the UI executor, including directory scans
/// during eviction. Attachment and profile-picture keys retain the on-disk format.
actor IrisAttachmentCache {
    private let directory: URL
    private let fileManager: FileManager
    private let limitBytes: Int

    init(dataDir: URL, fileManager: FileManager = .default, limitBytes: Int = 128 * 1024 * 1024) {
        self.directory = dataDir
            .appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("downloaded", isDirectory: true)
        self.fileManager = fileManager
        self.limitBytes = limitBytes
    }

    nonisolated static func attachmentKey(nhash: String, filename: String) -> String {
        "\(safeFilename(nhash))-\(safeFilename(filename))"
    }

    nonisolated static func pictureKey(nhash: String) -> String {
        "picture-\(safeFilename(nhash))"
    }

    private nonisolated static func safeFilename(_ value: String) -> String {
        let pieces = value.components(separatedBy: CharacterSet(charactersIn: "/\\:"))
            .joined(separator: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return pieces.isEmpty ? "attachment" : pieces
    }

    func data(for key: String) -> Data? {
        let url = directory.appendingPathComponent(key)
        guard let data = try? Data(contentsOf: url) else { return nil }
        try? fileManager.setAttributes([.modificationDate: Date()], ofItemAtPath: url.path)
        return data
    }

    /// Cached blobs are content-addressed, so an existing file can be reused
    /// when opening an attachment instead of rewriting it and rescanning the cache.
    func store(_ data: Data, for key: String) throws -> URL {
        let destination = directory.appendingPathComponent(key)
        if fileManager.fileExists(atPath: destination.path) {
            try? fileManager.setAttributes([.modificationDate: Date()], ofItemAtPath: destination.path)
            return destination
        }
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: destination, options: [.atomic])
        try prune(protecting: destination)
        return destination
    }

    private func prune(protecting protectedURL: URL) throws {
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
            guard values.isRegularFile == true else { continue }
            let size = values.fileSize ?? 0
            totalSize += size
            cachedFiles.append((file, values.contentModificationDate ?? .distantPast, size))
        }
        guard totalSize > limitBytes else { return }
        let protectedPath = protectedURL.standardizedFileURL.path
        for file in cachedFiles.sorted(by: { $0.modified < $1.modified }) {
            guard file.url.standardizedFileURL.path != protectedPath else { continue }
            do {
                try fileManager.removeItem(at: file.url)
                totalSize -= file.size
            } catch {
                continue
            }
            if totalSize <= limitBytes { break }
        }
    }
}
