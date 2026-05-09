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

private struct ShareSuggestionEntry: Codable {
    let chatId: String
    let displayName: String
    let subtitle: String?
    let pictureUrl: String?
    let isGroup: Bool
    let lastMessageAtSecs: UInt64?
}

final class ShareViewController: UIViewController {
    private let titleLabel = UILabel()
    private let statusLabel = UILabel()
    private let activityIndicator = UIActivityIndicatorView(style: .medium)
    private let chatTable = UITableView(frame: .zero, style: .plain)
    private let openAppButton = UIButton(type: .system)
    private let cancelButton = UIButton(type: .system)
    private let buttonStack = UIStackView()

    private var suggestions: [ShareSuggestionEntry] = []
    private var stagedShareID: String?
    private var stagedPayload: StoredSharePayload?
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
            await stageShare()
        }
    }

    private func configureView() {
        view.backgroundColor = .systemBackground

        titleLabel.text = "Send to…"
        titleLabel.font = .preferredFont(forTextStyle: .headline)
        titleLabel.textAlignment = .center
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        statusLabel.text = "Preparing…"
        statusLabel.font = .preferredFont(forTextStyle: .footnote)
        statusLabel.textColor = .secondaryLabel
        statusLabel.textAlignment = .center
        statusLabel.numberOfLines = 0
        statusLabel.translatesAutoresizingMaskIntoConstraints = false

        activityIndicator.startAnimating()
        activityIndicator.translatesAutoresizingMaskIntoConstraints = false

        chatTable.dataSource = self
        chatTable.delegate = self
        chatTable.register(ShareChatCell.self, forCellReuseIdentifier: ShareChatCell.reuseId)
        chatTable.rowHeight = 60
        chatTable.tableFooterView = UIView()
        chatTable.separatorInset = UIEdgeInsets(top: 0, left: 70, bottom: 0, right: 0)
        chatTable.isHidden = true
        chatTable.translatesAutoresizingMaskIntoConstraints = false

        openAppButton.setTitle("Open iris chat", for: .normal)
        openAppButton.titleLabel?.font = .preferredFont(forTextStyle: .body)
        openAppButton.addTarget(self, action: #selector(openMainApp), for: .touchUpInside)
        openAppButton.isHidden = true

        cancelButton.setTitle("Cancel", for: .normal)
        cancelButton.titleLabel?.font = .preferredFont(forTextStyle: .body)
        cancelButton.addTarget(self, action: #selector(cancelShare), for: .touchUpInside)

        buttonStack.axis = .vertical
        buttonStack.alignment = .center
        buttonStack.spacing = 4
        buttonStack.addArrangedSubview(openAppButton)
        buttonStack.addArrangedSubview(cancelButton)
        buttonStack.translatesAutoresizingMaskIntoConstraints = false

        let header = UIStackView(arrangedSubviews: [titleLabel, activityIndicator, statusLabel])
        header.axis = .vertical
        header.alignment = .center
        header.spacing = 8
        header.translatesAutoresizingMaskIntoConstraints = false

        view.addSubview(header)
        view.addSubview(chatTable)
        view.addSubview(buttonStack)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            header.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            header.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),

            chatTable.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 12),
            chatTable.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            chatTable.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            chatTable.bottomAnchor.constraint(equalTo: buttonStack.topAnchor, constant: -8),

            buttonStack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            buttonStack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            buttonStack.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
        ])
    }

    private func stageShare() async {
        let intentChatId = suggestedChatIdFromIntent
        guard let payload = await stageShareToDisk(suggestedChatId: intentChatId) else {
            await MainActor.run {
                activityIndicator.stopAnimating()
                titleLabel.text = "Nothing to share"
                statusLabel.text = nil
                openAppButton.isHidden = true
            }
            return
        }
        stagedShareID = payload.id
        stagedPayload = payload
        donateSuggestedInteraction()

        if intentChatId != nil {
            // The user picked a specific contact suggestion in iOS's share sheet.
            // Send it straight through.
            await openMainApp(autoSend: true)
            return
        }

        let loaded = readSuggestions()
        await MainActor.run {
            activityIndicator.stopAnimating()
            suggestions = loaded
            if loaded.isEmpty {
                titleLabel.text = "Send to…"
                statusLabel.text = "Open iris chat to choose a chat."
                chatTable.isHidden = true
            } else {
                statusLabel.text = nil
                chatTable.isHidden = false
                chatTable.reloadData()
            }
            openAppButton.isHidden = false
        }
    }

    @objc private func openMainApp() {
        Task { await openMainApp(autoSend: false) }
    }

    @objc private func cancelShare() {
        complete()
    }

    private func openMainApp(autoSend: Bool) async {
        guard let url = shareURL(autoSend: autoSend) else {
            return
        }
        let opened = await extensionContext?.open(url) ?? false
        if opened {
            complete()
        }
    }

    private func sendToChat(_ entry: ShareSuggestionEntry) {
        guard let payload = stagedPayload else { return }
        let updated = StoredSharePayload(
            id: payload.id,
            text: payload.text,
            attachments: payload.attachments,
            suggestedChatId: entry.chatId
        )
        stagedPayload = updated
        rewritePayloadOnDisk(updated)
        donateChatInteraction(entry)
        Task { await openMainApp(autoSend: true) }
    }

    private func shareURL(autoSend: Bool) -> URL? {
        guard let id = stagedShareID else { return nil }
        var comps = URLComponents()
        comps.scheme = "irischat"
        comps.host = "share"
        comps.path = "/\(id)"
        if autoSend {
            comps.queryItems = [URLQueryItem(name: "send", value: "1")]
        }
        return comps.url
    }

    private func rewritePayloadOnDisk(_ payload: StoredSharePayload) {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            return
        }
        let url = container
            .appendingPathComponent("pending-shares", isDirectory: true)
            .appendingPathComponent(payload.id)
            .appendingPathExtension("json")
        if let data = try? JSONEncoder().encode(payload) {
            try? data.write(to: url, options: .atomic)
        }
    }

    private func readSuggestions() -> [ShareSuggestionEntry] {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            return []
        }
        let url = container.appendingPathComponent("share-suggestions.json")
        guard let data = try? Data(contentsOf: url),
              let entries = try? JSONDecoder().decode([ShareSuggestionEntry].self, from: data) else {
            return []
        }
        return entries
    }

    private func stageShareToDisk(suggestedChatId: String?) async -> StoredSharePayload? {
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
        return payload
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

    private var suggestedChatIdFromIntent: String? {
        let chatId = (extensionContext?.intent as? INSendMessageIntent)?
            .conversationIdentifier?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return chatId?.isEmpty == false ? chatId : nil
    }

    private func donateSuggestedInteraction() {
        guard let intent = extensionContext?.intent as? INSendMessageIntent,
              let chatId = suggestedChatIdFromIntent else {
            return
        }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        interaction.identifier = "share-extension-\(chatId)-\(Int(Date().timeIntervalSince1970))"
        interaction.groupIdentifier = "iris-chat-share-suggestions"
        interaction.donate(completion: nil)
    }

    private func donateChatInteraction(_ entry: ShareSuggestionEntry) {
        let chatId = entry.chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !chatId.isEmpty else { return }
        let displayName = entry.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = displayName.isEmpty ? "Chat" : displayName
        let recipient = INPerson(
            personHandle: INPersonHandle(value: chatId, type: .unknown),
            nameComponents: nil,
            displayName: title,
            image: nil,
            contactIdentifier: nil,
            customIdentifier: chatId,
            isContactSuggestion: false,
            suggestionType: .instantMessageAddress
        )
        let groupName = entry.isGroup ? INSpeakableString(spokenPhrase: title) : nil
        let intent = INSendMessageIntent(
            recipients: entry.isGroup ? nil : [recipient],
            outgoingMessageType: .outgoingMessageText,
            content: nil,
            speakableGroupName: groupName,
            conversationIdentifier: chatId,
            serviceName: "Iris Chat",
            sender: nil,
            attachments: nil
        )
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        interaction.identifier = "share-extension-pick-\(chatId)-\(Int(Date().timeIntervalSince1970))"
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

extension ShareViewController: UITableViewDataSource, UITableViewDelegate {
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        suggestions.count
    }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(
            withIdentifier: ShareChatCell.reuseId,
            for: indexPath
        ) as? ShareChatCell ?? ShareChatCell(style: .default, reuseIdentifier: ShareChatCell.reuseId)
        cell.configure(with: suggestions[indexPath.row])
        return cell
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        sendToChat(suggestions[indexPath.row])
    }
}

private final class ShareChatCell: UITableViewCell {
    static let reuseId = "ShareChatCell"

    private let avatarLabel = UILabel()
    private let nameLabel = UILabel()
    private let subtitleLabel = UILabel()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)

        avatarLabel.textAlignment = .center
        avatarLabel.textColor = .white
        avatarLabel.font = .systemFont(ofSize: 18, weight: .semibold)
        avatarLabel.layer.cornerRadius = 22
        avatarLabel.layer.masksToBounds = true
        avatarLabel.translatesAutoresizingMaskIntoConstraints = false

        nameLabel.font = .preferredFont(forTextStyle: .body)
        nameLabel.translatesAutoresizingMaskIntoConstraints = false

        subtitleLabel.font = .preferredFont(forTextStyle: .footnote)
        subtitleLabel.textColor = .secondaryLabel
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false

        contentView.addSubview(avatarLabel)
        contentView.addSubview(nameLabel)
        contentView.addSubview(subtitleLabel)

        NSLayoutConstraint.activate([
            avatarLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            avatarLabel.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            avatarLabel.widthAnchor.constraint(equalToConstant: 44),
            avatarLabel.heightAnchor.constraint(equalToConstant: 44),

            nameLabel.leadingAnchor.constraint(equalTo: avatarLabel.trailingAnchor, constant: 12),
            nameLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            nameLabel.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),

            subtitleLabel.leadingAnchor.constraint(equalTo: nameLabel.leadingAnchor),
            subtitleLabel.trailingAnchor.constraint(equalTo: nameLabel.trailingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: nameLabel.bottomAnchor, constant: 2),
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func configure(with entry: ShareSuggestionEntry) {
        let trimmed = entry.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let display = trimmed.isEmpty ? "Chat" : trimmed
        nameLabel.text = display
        let trimmedSubtitle = entry.subtitle?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        subtitleLabel.text = trimmedSubtitle.isEmpty ? nil : trimmedSubtitle
        subtitleLabel.isHidden = subtitleLabel.text == nil

        let firstChar = display.unicodeScalars.first.map { String($0).uppercased() } ?? "?"
        avatarLabel.text = firstChar
        avatarLabel.backgroundColor = avatarColor(for: entry.chatId)
    }

    private func avatarColor(for seed: String) -> UIColor {
        var hash: UInt64 = 5381
        for ch in seed.unicodeScalars {
            hash = hash &* 33 &+ UInt64(ch.value)
        }
        let hue = CGFloat(hash % 360) / 360
        return UIColor(hue: hue, saturation: 0.55, brightness: 0.78, alpha: 1.0)
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
