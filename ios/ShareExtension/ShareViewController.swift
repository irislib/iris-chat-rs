import Intents
import UIKit
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
    let suggestedChatId: String?
}

final class ShareViewController: UIViewController {
    private let statusLabel = UILabel()
    private let chooseButton = UIButton(type: .system)
    private let cancelButton = UIButton(type: .system)
    private let activityIndicator = UIActivityIndicatorView(style: .medium)
    private var stagedShareURL: URL?
    private var didStart = false

    override func viewDidLoad() {
        super.viewDidLoad()
        configureView()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !didStart else { return }
        didStart = true
        Task {
            await stageAndOpenShare()
        }
    }

    private func configureView() {
        view.backgroundColor = .systemBackground

        statusLabel.text = "Preparing..."
        statusLabel.font = .preferredFont(forTextStyle: .headline)
        statusLabel.textAlignment = .center
        statusLabel.numberOfLines = 0

        chooseButton.setTitle("Choose chat", for: .normal)
        chooseButton.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        chooseButton.isHidden = true
        chooseButton.addTarget(self, action: #selector(chooseChat), for: .touchUpInside)

        cancelButton.setTitle("Cancel", for: .normal)
        cancelButton.addTarget(self, action: #selector(cancelShare), for: .touchUpInside)

        activityIndicator.startAnimating()

        let stack = UIStackView(arrangedSubviews: [
            activityIndicator,
            statusLabel,
            chooseButton,
            cancelButton,
        ])
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 18
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.centerXAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerYAnchor),
            statusLabel.widthAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.widthAnchor, constant: -48),
            chooseButton.widthAnchor.constraint(greaterThanOrEqualToConstant: 180),
        ])
    }

    private func stageAndOpenShare() async {
        guard let shareURL = await storeShare() else {
            statusLabel.text = "Nothing to share"
            activityIndicator.stopAnimating()
            chooseButton.isHidden = true
            return
        }
        stagedShareURL = shareURL

        if await openStagedShare() {
            complete()
        } else {
            activityIndicator.stopAnimating()
            statusLabel.text = "Choose a chat in iris chat"
            chooseButton.isHidden = false
        }
    }

    private func storeShare() async -> URL? {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            return nil
        }

        let shareID = UUID().uuidString
        let sharesDir = container.appendingPathComponent("pending-shares", isDirectory: true)
        let filesDir = sharesDir.appendingPathComponent("\(shareID)-files", isDirectory: true)

        do {
            try FileManager.default.createDirectory(at: filesDir, withIntermediateDirectories: true)
        } catch {
            return nil
        }

        let collected = await collectSharedItems(filesDir: filesDir)
        let text = collected.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !collected.attachments.isEmpty else {
            return nil
        }

        let payload = StoredSharePayload(
            id: shareID,
            text: text,
            attachments: collected.attachments,
            suggestedChatId: suggestedChatId
        )
        do {
            let data = try JSONEncoder().encode(payload)
            let payloadURL = sharesDir.appendingPathComponent(shareID).appendingPathExtension("json")
            try data.write(to: payloadURL, options: .atomic)
        } catch {
            return nil
        }

        donateSuggestedInteraction()
        return URL(string: "irischat://share/\(shareID)")
    }

    @objc private func chooseChat() {
        Task {
            if await openStagedShare() {
                complete()
            }
        }
    }

    @objc private func cancelShare() {
        complete()
    }

    private func openStagedShare() async -> Bool {
        guard let stagedShareURL else {
            return false
        }
        return await extensionContext?.open(stagedShareURL) ?? false
    }

    private func collectSharedItems(filesDir: URL) async -> (text: String, attachments: [StoredShareAttachment]) {
        var textParts = [String]()
        var attachments = [StoredShareAttachment]()

        let inputItems = extensionContext?.inputItems.compactMap { $0 as? NSExtensionItem } ?? []
        for item in inputItems {
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

    private var suggestedChatId: String? {
        let chatId = (extensionContext?.intent as? INSendMessageIntent)?
            .conversationIdentifier?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return chatId?.isEmpty == false ? chatId : nil
    }

    private func donateSuggestedInteraction() {
        guard let intent = extensionContext?.intent as? INSendMessageIntent,
              let chatId = suggestedChatId else {
            return
        }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        interaction.identifier = "share-extension-\(chatId)-\(Int(Date().timeIntervalSince1970))"
        interaction.groupIdentifier = "iris-chat-share-suggestions"
        interaction.donate(completion: nil)
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
