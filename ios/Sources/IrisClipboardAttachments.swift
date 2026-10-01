import Foundation
import ImageIO
import UniformTypeIdentifiers
#if os(macOS)
import AppKit
#endif

/// One preferred representation per clipboard item, in clipboard order.
/// Sources live until the ordinary attachment staging pipeline has copied them.
struct IrisClipboardAttachments {
    let providers: [NSItemProvider]

    init?(providers: [NSItemProvider]) {
        self.providers = providers.filter {
            Self.preferredType($0.registeredTypeIdentifiers, suggestedName: $0.suggestedName) != nil
        }
        if self.providers.isEmpty { return nil }
    }

    static func preferredType(_ identifiers: [String], suggestedName: String? = nil) -> String? {
        if identifiers.contains(UTType.fileURL.identifier) { return UTType.fileURL.identifier }
        let isNamedFile = !(suggestedName.map { ($0 as NSString).pathExtension } ?? "").isEmpty
        let supported = identifiers.filter { identifier in
            guard let type = UTType(identifier), type.conforms(to: .data),
                  !type.conforms(to: .url), !type.conforms(to: .text) || isNamedFile else { return false }
            return type.conforms(to: .content) || type.conforms(to: .archive) || isNamedFile
        }
        return supported.first { UTType($0)?.preferredFilenameExtension != nil } ?? supported.first
    }

    func withURLs(in temporaryDirectory: URL = FileManager.default.temporaryDirectory,
                  _ consume: (() async -> [URL]) async -> Void) async {
        let directory = temporaryDirectory
            .appendingPathComponent("iris-paste-\(UUID().uuidString)", isDirectory: true)
        await consume { await load(into: directory) }
        await Task.detached(priority: .utility) {
            try? FileManager.default.removeItem(at: directory)
        }.value
    }

    private func load(into directory: URL) async -> [URL] {
        var urls: [URL] = []
        for (index, provider) in providers.enumerated() {
            guard !Task.isCancelled,
                  let type = Self.preferredType(provider.registeredTypeIdentifiers, suggestedName: provider.suggestedName) else { return [] }
            let url: URL?
            if type == UTType.fileURL.identifier {
                url = await IrisDroppedFiles.load([provider]).first
            } else {
                let itemDirectory = directory.appendingPathComponent(String(index), isDirectory: true)
                let destination = itemDirectory.appendingPathComponent(Self.filename(provider.suggestedName, type: type))
                let materialized = await Self.materialize(provider, type: type, destination: destination)
                url = materialized.map { Self.restoreImageExtension($0, type: type) }
            }
            // Never silently turn a partly unreadable selection into a partial send.
            guard let url, !Task.isCancelled else { return [] }
            urls.append(url)
        }
        return urls
    }

    private static func filename(_ suggested: String?, type: String) -> String {
        let name = (suggested.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let base = name.isEmpty || name == "." || name == ".." ? "Pasted file" : name
        guard (base as NSString).pathExtension.isEmpty,
              let ext = UTType(type)?.preferredFilenameExtension else { return base }
        return "\(base).\(ext)"
    }

    private static func restoreImageExtension(_ url: URL, type: String) -> URL {
        guard url.pathExtension.isEmpty, UTType(type)?.conforms(to: .image) == true,
              let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let detected = CGImageSourceGetType(source),
              let ext = UTType(detected as String)?.preferredFilenameExtension else { return url }
        let named = url.appendingPathExtension(ext)
        do { try FileManager.default.moveItem(at: url, to: named); return named }
        catch { return url }
    }

    private static func materialize(_ provider: NSItemProvider, type: String, destination: URL) async -> URL? {
        // A provider's temporary file is valid only inside its callback.
        let copied: URL? = await withCheckedContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { source, _ in
                do {
                    guard let source else { continuation.resume(returning: nil); return }
                    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try FileManager.default.copyItem(at: source, to: destination)
                    continuation.resume(returning: destination)
                } catch { continuation.resume(returning: nil) }
            }
        }
        if let copied { return copied }
        guard !Task.isCancelled else { return nil }
        return await withCheckedContinuation { continuation in
            provider.loadDataRepresentation(forTypeIdentifier: type) { data, _ in
                do {
                    guard let data else { continuation.resume(returning: nil); return }
                    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try data.write(to: destination, options: .atomic)
                    continuation.resume(returning: destination)
                } catch { continuation.resume(returning: nil) }
            }
        }
    }

    #if os(macOS)
    @MainActor
    init?(pasteboard: NSPasteboard) {
        let providers = (pasteboard.pasteboardItems ?? []).compactMap { item -> NSItemProvider? in
            guard let type = Self.preferredType(item.types.map(\.rawValue)) else { return nil }
            let pasteboardType = NSPasteboard.PasteboardType(type)
            if type == UTType.fileURL.identifier {
                let value = item.string(forType: pasteboardType) ?? ""
                return NSItemProvider(item: value as NSString, typeIdentifier: type)
            }
            let data = item.data(forType: pasteboardType)
            let provider = NSItemProvider()
            provider.registerDataRepresentation(forTypeIdentifier: type, visibility: .all) { completion in
                completion(data, data == nil ? CocoaError(.fileReadUnknown) : nil)
                return nil
            }
            return provider
        }
        self.init(providers: providers)
    }
    #endif
}
