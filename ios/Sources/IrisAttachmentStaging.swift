import Foundation

// FileManager's filesystem operations are thread-safe. AppManager supplies a
// manager without a delegate; injected test probes are configured before handoff.
// This immutable wrapper never mutates that manager, and each copy has its own
// destination. Older Foundation SDKs do not declare FileManager Sendable.
struct IrisAttachmentStaging: @unchecked Sendable {
    let dataDir: URL
    let fileManager: FileManager

    private struct Source: Sendable {
        let url: URL
        let sharedFilename: String?
    }

    func stage(_ url: URL) throws -> StagedAttachment {
        try stage(Source(url: url, sharedFilename: nil))
    }

    private func stage(_ input: Source) throws -> StagedAttachment {
        let sourceURL = input.url
        let accessed = sourceURL.startAccessingSecurityScopedResource()
        defer { if accessed { sourceURL.stopAccessingSecurityScopedResource() } }
        guard sourceURL.isFileURL,
              try sourceURL.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else {
            throw CocoaError(.fileReadUnsupportedScheme)
        }
        let directory = dataDir.appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("outgoing", isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let sharedName = (input.sharedFilename ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let filename = sharedName.isEmpty
            ? sourceURL.lastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines)
            : (sharedName as NSString).lastPathComponent
        var displayName = filename.isEmpty ? "attachment" : filename
        let photo = input.sharedFilename == nil ? nil : try irisSharedPhotoForConversion(at: sourceURL, filename: displayName)
        if photo != nil { displayName = (displayName as NSString).deletingPathExtension + ".jpg" }
        let destination = directory.appendingPathComponent("\(UUID().uuidString)-\(displayName)")
        guard !fileManager.fileExists(atPath: destination.path) else {
            throw CocoaError(.fileWriteFileExists)
        }
        do {
            if let photo {
                try irisWriteSharedPhoto(photo, to: destination)
            } else {
                try fileManager.copyItem(at: sourceURL, to: destination)
            }
            return StagedAttachment(path: destination.path, filename: displayName)
        } catch {
            try? fileManager.removeItem(at: destination)
            throw error
        }
    }

    func stage(_ sourceURLs: [URL], checkingCancellation: Bool = false) throws -> [StagedAttachment] {
        try stage(sourceURLs.map { Source(url: $0, sharedFilename: nil) }, checkingCancellation: checkingCancellation)
    }

    private func stage(_ sources: [Source], checkingCancellation: Bool) throws -> [StagedAttachment] {
        var staged: [StagedAttachment] = []
        do {
            for source in sources {
                if checkingCancellation { try Task.checkCancellation() }
                staged.append(try stage(source))
            }
            if checkingCancellation { try Task.checkCancellation() }
            return staged
        } catch {
            discard(staged)
            throw error
        }
    }

    func stageAsync(_ sourceURLs: [URL]) async throws -> [StagedAttachment] {
        try await stageAsync(sourceURLs.map { Source(url: $0, sharedFilename: nil) })
    }

    func stageAsync(shared attachments: [PendingShareAttachment]) async throws -> [StagedAttachment] {
        try await stageAsync(attachments.map { Source(url: URL(fileURLWithPath: $0.path), sharedFilename: $0.filename) })
    }

    private func stageAsync(_ sources: [Source]) async throws -> [StagedAttachment] {
        let work = Task.detached(priority: .userInitiated) {
            try stage(sources, checkingCancellation: true)
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

    func discard(_ attachments: [StagedAttachment]) {
        for attachment in attachments {
            try? fileManager.removeItem(at: URL(fileURLWithPath: attachment.path))
        }
    }
}
