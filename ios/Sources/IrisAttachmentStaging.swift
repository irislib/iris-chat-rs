import Foundation

// FileManager's filesystem operations are thread-safe. AppManager supplies a
// manager without a delegate; injected test probes are configured before handoff.
// This immutable wrapper never mutates that manager, and each copy has its own
// destination. Older Foundation SDKs do not declare FileManager Sendable.
struct IrisAttachmentStaging: @unchecked Sendable {
    let dataDir: URL
    let fileManager: FileManager

    func stage(_ sourceURL: URL) throws -> StagedAttachment {
        let accessed = sourceURL.startAccessingSecurityScopedResource()
        defer { if accessed { sourceURL.stopAccessingSecurityScopedResource() } }
        let directory = dataDir.appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("outgoing", isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let filename = sourceURL.lastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = filename.isEmpty ? "attachment" : filename
        let destination = directory.appendingPathComponent("\(UUID().uuidString)-\(displayName)")
        guard !fileManager.fileExists(atPath: destination.path) else {
            throw CocoaError(.fileWriteFileExists)
        }
        do {
            try fileManager.copyItem(at: sourceURL, to: destination)
            return StagedAttachment(path: destination.path, filename: displayName)
        } catch {
            try? fileManager.removeItem(at: destination)
            throw error
        }
    }

    func stage(_ sourceURLs: [URL], checkingCancellation: Bool = false) throws -> [StagedAttachment] {
        var staged: [StagedAttachment] = []
        do {
            for sourceURL in sourceURLs {
                if checkingCancellation { try Task.checkCancellation() }
                staged.append(try stage(sourceURL))
            }
            if checkingCancellation { try Task.checkCancellation() }
            return staged
        } catch {
            discard(staged)
            throw error
        }
    }

    func stageAsync(_ sourceURLs: [URL]) async throws -> [StagedAttachment] {
        let work = Task.detached(priority: .userInitiated) {
            try stage(sourceURLs, checkingCancellation: true)
        }
        return try await withTaskCancellationHandler {
            let staged = try await work.value
            guard !Task.isCancelled else {
                await Task.detached(priority: .utility) { discard(staged) }.value
                throw CancellationError()
            }
            return staged
        } onCancel: {
            work.cancel()
        }
    }

    private func discard(_ attachments: [StagedAttachment]) {
        for attachment in attachments {
            try? fileManager.removeItem(at: URL(fileURLWithPath: attachment.path))
        }
    }
}
