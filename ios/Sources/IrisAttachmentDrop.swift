import Foundation
import SwiftUI
import UniformTypeIdentifiers

/// A drop adds files to the existing draft; only the composer can send them.
struct IrisAttachmentDropModifier: ViewModifier {
    @Environment(\.irisPalette) private var palette
    let enabled: Bool
    @Binding var isPreparing: Bool
    let onAttach: (() async -> [URL]) async -> Void
    @State private var isTargeted = false
    @State private var task: Task<Void, Never>?

    func body(content: Content) -> some View {
        content
            .overlay {
                if enabled && isTargeted {
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(palette.accent, lineWidth: 2)
                        .padding(4)
                        .allowsHitTesting(false)
                }
            }
            .onDrop(of: enabled ? [UTType.fileURL.identifier] : [], isTargeted: $isTargeted) { providers in
                guard enabled, !isPreparing,
                      providers.contains(where: { $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) }) else {
                    return false
                }
                isPreparing = true
                task = Task {
                    defer { isPreparing = false; task = nil }
                    await onAttach { await IrisDroppedFiles.load(providers) }
                }
                return true
            }
            .onDisappear { task?.cancel(); task = nil; isPreparing = false }
    }
}

enum IrisDroppedFiles {
    /// Load in selection order, including providers which finish at different times.
    static func load(_ providers: [NSItemProvider]) async -> [URL] {
        var urls: [URL] = []
        for provider in providers {
            guard !Task.isCancelled else { return [] }
            guard provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) else { continue }
            let url: URL? = await withCheckedContinuation { continuation in
                provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                    continuation.resume(returning: droppedFileURL(from: item))
                }
            }
            // A partly unreadable selection must not silently drop some files.
            guard let url else { return [] }
            urls.append(url)
        }
        return Task.isCancelled ? [] : urls
    }
}

func droppedFileURL(from item: NSSecureCoding?) -> URL? {
    let url: URL?
    if let value = item as? URL {
        url = value
    } else if let data = item as? Data {
        url = URL(dataRepresentation: data, relativeTo: nil)
    } else if let value = item as? String {
        url = URL(string: value.trimmingCharacters(in: .whitespacesAndNewlines))
    } else {
        url = nil
    }
    return url?.isFileURL == true ? url : nil
}
