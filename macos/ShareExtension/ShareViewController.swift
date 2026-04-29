import AppKit
import UniformTypeIdentifiers

private let appGroupIdentifier = "group.to.iris.chat"

private struct StoredShareAttachment: Codable {
    let path: String
    let filename: String
}

private struct StoredSharePayload: Codable {
    let id: String
    let text: String
    let attachments: [StoredShareAttachment]
}

final class ShareViewController: NSViewController {
    private var didStart = false

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 320, height: 120))
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !didStart else {
            return
        }
        didStart = true
        Task {
            await storeAndOpenShare()
        }
    }

    private func storeAndOpenShare() async {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            complete()
            return
        }

        let shareID = UUID().uuidString
        let sharesDir = container.appendingPathComponent("pending-shares", isDirectory: true)
        let filesDir = sharesDir.appendingPathComponent("\(shareID)-files", isDirectory: true)

        do {
            try FileManager.default.createDirectory(at: filesDir, withIntermediateDirectories: true)
        } catch {
            complete()
            return
        }

        let collected = await collectSharedItems(filesDir: filesDir)
        let text = collected.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !collected.attachments.isEmpty else {
            complete()
            return
        }

        let payload = StoredSharePayload(id: shareID, text: text, attachments: collected.attachments)
        do {
            let data = try JSONEncoder().encode(payload)
            let payloadURL = sharesDir.appendingPathComponent(shareID).appendingPathExtension("json")
            try data.write(to: payloadURL, options: .atomic)
        } catch {
            complete()
            return
        }

        if let url = URL(string: "irischat://share/\(shareID)") {
            NSWorkspace.shared.open(url)
        }
        complete()
    }

    private func collectSharedItems(filesDir: URL) async -> (text: String, attachments: [StoredShareAttachment]) {
        var textParts = [String]()
        var attachments = [StoredShareAttachment]()
        let inputItems = extensionContext?.inputItems.compactMap { $0 as? NSExtensionItem } ?? []

        for item in inputItems {
            if let title = item.attributedTitle?.string, !title.isEmpty {
                textParts.append(title)
            }
            for provider in item.attachments ?? [] {
                if let urlText = await loadURLText(from: provider) {
                    textParts.append(urlText)
                    continue
                }
                if let plainText = await loadPlainText(from: provider) {
                    textParts.append(plainText)
                    continue
                }
                if let attachment = await copyAttachment(from: provider, to: filesDir) {
                    attachments.append(attachment)
                }
            }
        }

        return (textParts.joined(separator: "\n"), attachments)
    }

    private func loadURLText(from provider: NSItemProvider) async -> String? {
        guard provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) else {
            return nil
        }
        return await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil) { item, _ in
                if let url = item as? URL {
                    continuation.resume(returning: url.absoluteString)
                } else if let string = item as? String {
                    continuation.resume(returning: string)
                } else {
                    continuation.resume(returning: nil)
                }
            }
        }
    }

    private func loadPlainText(from provider: NSItemProvider) async -> String? {
        guard provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier) ||
            provider.hasItemConformingToTypeIdentifier(UTType.text.identifier) else {
            return nil
        }
        let type = provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier)
            ? UTType.plainText.identifier
            : UTType.text.identifier
        return await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                if let string = item as? String {
                    continuation.resume(returning: string)
                } else if let data = item as? Data {
                    continuation.resume(returning: String(data: data, encoding: .utf8))
                } else {
                    continuation.resume(returning: nil)
                }
            }
        }
    }

    private func copyAttachment(from provider: NSItemProvider, to filesDir: URL) async -> StoredShareAttachment? {
        let type = provider.registeredTypeIdentifiers.first {
            $0 != UTType.url.identifier &&
                $0 != UTType.plainText.identifier &&
                $0 != UTType.text.identifier
        }
        guard let type else {
            return nil
        }
        let suggestedName = provider.suggestedName
        let fallbackExtension = UTType(type)?.preferredFilenameExtension
        return await withCheckedContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { sourceURL, _ in
                guard let sourceURL else {
                    continuation.resume(returning: nil)
                    return
                }
                let filename = safeFilename(
                    suggestedName,
                    fallbackExtension: fallbackExtension
                )
                let destination = uniqueDestination(in: filesDir, filename: filename)
                do {
                    try FileManager.default.copyItem(at: sourceURL, to: destination)
                    continuation.resume(
                        returning: StoredShareAttachment(
                            path: destination.path,
                            filename: destination.lastPathComponent
                        )
                    )
                } catch {
                    continuation.resume(returning: nil)
                }
            }
        }
    }

    private func complete() {
        extensionContext?.completeRequest(returningItems: nil)
    }
}

private func safeFilename(_ suggestedName: String?, fallbackExtension: String?) -> String {
    var basename = suggestedName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    basename = basename
        .components(separatedBy: CharacterSet(charactersIn: "/\\:"))
        .filter { !$0.isEmpty }
        .last ?? ""
    if basename.isEmpty {
        basename = "attachment"
    }
    if !basename.contains("."), let fallbackExtension, !fallbackExtension.isEmpty {
        basename += ".\(fallbackExtension)"
    }
    return basename
}

private func uniqueDestination(in dir: URL, filename: String) -> URL {
    let base = (filename as NSString).deletingPathExtension
    let ext = (filename as NSString).pathExtension
    var candidate = dir.appendingPathComponent(filename)
    var index = 2
    while FileManager.default.fileExists(atPath: candidate.path) {
        let suffix = ext.isEmpty ? "-\(index)" : "-\(index).\(ext)"
        candidate = dir.appendingPathComponent("\(base)\(suffix)")
        index += 1
    }
    return candidate
}
