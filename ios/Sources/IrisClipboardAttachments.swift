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
        let name = suggestedFilename(suggestedName)
        let ext = name.map { ($0 as NSString).pathExtension } ?? ""
        let isNamedFile = !ext.isEmpty
        let supported = identifiers.filter { identifier in
            guard let type = UTType(identifier), type.conforms(to: .data),
                  !type.conforms(to: .url), !type.conforms(to: .text) || isNamedFile else { return false }
            return type.conforms(to: .content) || type.conforms(to: .archive) || isNamedFile
        }
        if !ext.isEmpty, let filenameType = UTType(filenameExtension: ext) {
            if let exact = supported.first(where: { UTType($0) == filenameType }) { return exact }
            if let matching = supported.first(where: { UTType($0)?.conforms(to: filenameType) == true }) { return matching }
        }
        if name == nil, supported.contains(UTType.png.identifier) { return UTType.png.identifier }
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
                url = materialized.flatMap { Self.prepareImage($0, type: type, suggestedName: provider.suggestedName) }
            }
            // Never silently turn a partly unreadable selection into a partial send.
            guard let url, !Task.isCancelled else { return [] }
            urls.append(url)
        }
        return urls
    }

    private static func suggestedFilename(_ suggested: String?) -> String? {
        let name = (suggested.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty || name == "." || name == ".." ? nil : name
    }

    private static func filename(_ suggested: String?, type: String) -> String {
        let base = suggestedFilename(suggested) ?? "Pasted file"
        guard (base as NSString).pathExtension.isEmpty,
              let ext = UTType(type)?.preferredFilenameExtension else { return base }
        return "\(base).\(ext)"
    }

    private static func prepareImage(_ url: URL, type: String, suggestedName: String?) -> URL? {
        guard UTType(type)?.conforms(to: .image) == true,
              let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let detected = CGImageSourceGetType(source),
              let imageType = UTType(detected as String),
              let ext = imageType.preferredFilenameExtension else { return url }
        // Raw clipboard pixels should preview on every client. Keep original
        // named files intact, including animations and documents with images.
        let inlineExtensions: Set<String> = ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "avif"]
        if suggestedFilename(suggestedName) == nil, !inlineExtensions.contains(ext.lowercased()) {
            return makePNG(source, replacing: url)
        }
        let filenameType = url.pathExtension.isEmpty ? nil : UTType(filenameExtension: url.pathExtension)
        let matches = filenameType.map { imageType.conforms(to: $0) } ?? false
        guard url.pathExtension.isEmpty || (filenameType?.conforms(to: .image) == true && !matches) else {
            return url
        }
        let named = url.deletingPathExtension().appendingPathExtension(ext)
        do { try FileManager.default.moveItem(at: url, to: named); return named }
        catch { return url }
    }

    private static func makePNG(_ source: CGImageSource, replacing url: URL) -> URL? {
        guard !Task.isCancelled,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: max(1, max(width, height))
        ]
        guard let pixels = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary), !Task.isCancelled else { return nil }
        let png = url.deletingLastPathComponent().appendingPathComponent("Pasted image.png")
        guard let destination = CGImageDestinationCreateWithURL(png as CFURL, UTType.png.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, pixels, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        try? FileManager.default.removeItem(at: url)
        return png
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
