import Foundation
import UniformTypeIdentifiers

extension NSItemProvider {
    func sharedURLText() async -> String? {
        guard hasItemConformingToTypeIdentifier(UTType.url.identifier) else { return nil }

        let text: String? = await withCheckedContinuation { continuation in
            loadItem(forTypeIdentifier: UTType.url.identifier, options: nil) { item, _ in
                continuation.resume(returning: (item as? URL)?.absoluteString ?? (item as? String))
            }
        }
        if let text {
            return URL(string: text)?.isFileURL == true ? nil : text
        }

        // Some senders vend encoded URL data through loadItem.
        guard canLoadObject(ofClass: NSURL.self) else { return nil }
        return await withCheckedContinuation { continuation in
            loadObject(ofClass: NSURL.self) { item, _ in
                guard let url = item as? URL, !url.isFileURL else {
                    continuation.resume(returning: nil)
                    return
                }
                continuation.resume(returning: url.absoluteString)
            }
        }
    }
}
