import Intents
import UIKit
import UniformTypeIdentifiers

private let appGroupIdentifier = "group.to.iris.chat"
private let pendingShareNotificationName = "to.iris.chat.pending-share"

private struct StoredShareAttachment: Codable {
    let path: String
    let filename: String
}

private struct StoredSharePayload: Codable {
    let id: String
    let text: String
    let attachments: [StoredShareAttachment]
    let suggestedChatId: String?
    let suggestedChatIds: [String]?
    let autoSend: Bool?
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
    private let titleBar = UIView()
    private let titleLabel = UILabel()
    private let searchBar = UISearchBar(frame: .zero)
    private let statusLabel = UILabel()
    private let activityIndicator = UIActivityIndicatorView(style: .medium)
    private let chatTable = UITableView(frame: .zero, style: .plain)
    private let sendButton = UIButton(type: .system)
    private let openAppButton = UIButton(type: .system)
    private let cancelButton = UIButton(type: .system)
    private let footerView = UIView()
    private let footerHairline = UIView()
    private let footerStack = UIStackView()
    private let selectedNamesScrollView = UIScrollView()
    private let selectedNamesLabel = UILabel()

    private var suggestions: [ShareSuggestionEntry] = []
    private var searchText = ""
    private var selectedChatIds = Set<String>()
    private var stagedShareID: String?
    private var stagedPayload: StoredSharePayload?
    private var didStart = false
    private var isOpeningApp = false

    private var filteredSuggestions: [ShareSuggestionEntry] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return suggestions }
        return suggestions.filter { entry in
            shareDisplayName(for: entry).lowercased().contains(query)
                || (entry.subtitle?.lowercased().contains(query) ?? false)
        }
    }

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
        view.backgroundColor = ShareColors.presentedBackground

        titleBar.translatesAutoresizingMaskIntoConstraints = false

        titleLabel.text = "Choose recipients"
        titleLabel.font = .systemFont(ofSize: 17, weight: .semibold)
        titleLabel.textColor = .label
        titleLabel.textAlignment = .center
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        cancelButton.setImage(UIImage(systemName: "xmark"), for: .normal)
        cancelButton.tintColor = .label
        cancelButton.backgroundColor = ShareColors.cellBackground
        cancelButton.layer.cornerRadius = 20
        cancelButton.clipsToBounds = true
        cancelButton.addTarget(self, action: #selector(cancelShare), for: .touchUpInside)
        cancelButton.translatesAutoresizingMaskIntoConstraints = false
        cancelButton.accessibilityLabel = "Close"

        searchBar.placeholder = "Search"
        searchBar.searchBarStyle = .minimal
        searchBar.delegate = self
        searchBar.isHidden = true
        searchBar.translatesAutoresizingMaskIntoConstraints = false

        statusLabel.text = "Preparing..."
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
        chatTable.register(ShareSectionHeaderView.self, forHeaderFooterViewReuseIdentifier: ShareSectionHeaderView.reuseId)
        chatTable.rowHeight = 60
        chatTable.backgroundColor = ShareColors.presentedBackground
        chatTable.tableFooterView = UIView()
        chatTable.separatorStyle = .none
        if #available(iOS 15.0, *) {
            chatTable.sectionHeaderTopPadding = 0
        }
        chatTable.isHidden = true
        chatTable.translatesAutoresizingMaskIntoConstraints = false

        sendButton.addTarget(self, action: #selector(sendSelectedChats), for: .touchUpInside)
        sendButton.isEnabled = false
        sendButton.translatesAutoresizingMaskIntoConstraints = false
        sendButton.accessibilityLabel = "Send"

        openAppButton.setTitle("Open Iris Chat", for: .normal)
        openAppButton.titleLabel?.font = .preferredFont(forTextStyle: .body)
        openAppButton.addTarget(self, action: #selector(openMainApp), for: .touchUpInside)
        openAppButton.isHidden = true
        openAppButton.translatesAutoresizingMaskIntoConstraints = false

        footerView.backgroundColor = ShareColors.footerBackground
        footerView.isHidden = true
        footerView.translatesAutoresizingMaskIntoConstraints = false

        footerHairline.backgroundColor = ShareColors.separator
        footerHairline.translatesAutoresizingMaskIntoConstraints = false

        selectedNamesScrollView.showsHorizontalScrollIndicator = false
        selectedNamesScrollView.translatesAutoresizingMaskIntoConstraints = false

        selectedNamesLabel.font = .systemFont(ofSize: 15)
        selectedNamesLabel.textColor = .secondaryLabel
        selectedNamesLabel.lineBreakMode = .byClipping
        selectedNamesLabel.numberOfLines = 1
        selectedNamesLabel.translatesAutoresizingMaskIntoConstraints = false

        footerStack.axis = .horizontal
        footerStack.alignment = .center
        footerStack.spacing = 12
        footerStack.translatesAutoresizingMaskIntoConstraints = false
        footerStack.addArrangedSubview(selectedNamesScrollView)
        footerStack.addArrangedSubview(sendButton)

        selectedNamesScrollView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        sendButton.setContentHuggingPriority(.required, for: .horizontal)

        view.addSubview(titleBar)
        titleBar.addSubview(titleLabel)
        titleBar.addSubview(cancelButton)
        view.addSubview(searchBar)
        view.addSubview(activityIndicator)
        view.addSubview(statusLabel)
        view.addSubview(openAppButton)
        view.addSubview(chatTable)
        view.addSubview(footerView)
        footerView.addSubview(footerHairline)
        footerView.addSubview(footerStack)
        selectedNamesScrollView.addSubview(selectedNamesLabel)

        NSLayoutConstraint.activate([
            titleBar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 6),
            titleBar.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            titleBar.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            titleBar.heightAnchor.constraint(equalToConstant: 44),

            titleLabel.centerXAnchor.constraint(equalTo: titleBar.centerXAnchor),
            titleLabel.centerYAnchor.constraint(equalTo: titleBar.centerYAnchor),
            titleLabel.leadingAnchor.constraint(greaterThanOrEqualTo: titleBar.leadingAnchor, constant: 48),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: cancelButton.leadingAnchor, constant: -12),

            cancelButton.trailingAnchor.constraint(equalTo: titleBar.trailingAnchor),
            cancelButton.centerYAnchor.constraint(equalTo: titleBar.centerYAnchor),
            cancelButton.widthAnchor.constraint(equalToConstant: 40),
            cancelButton.heightAnchor.constraint(equalToConstant: 40),

            searchBar.topAnchor.constraint(equalTo: titleBar.bottomAnchor, constant: 2),
            searchBar.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 8),
            searchBar.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -8),

            activityIndicator.topAnchor.constraint(equalTo: searchBar.bottomAnchor, constant: 18),
            activityIndicator.centerXAnchor.constraint(equalTo: view.centerXAnchor),

            statusLabel.topAnchor.constraint(equalTo: activityIndicator.bottomAnchor, constant: 10),
            statusLabel.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            statusLabel.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),

            openAppButton.topAnchor.constraint(equalTo: statusLabel.bottomAnchor, constant: 16),
            openAppButton.centerXAnchor.constraint(equalTo: view.centerXAnchor),

            chatTable.topAnchor.constraint(equalTo: searchBar.bottomAnchor, constant: 8),
            chatTable.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            chatTable.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            chatTable.bottomAnchor.constraint(equalTo: footerView.topAnchor),

            footerView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            footerView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            footerView.bottomAnchor.constraint(equalTo: view.bottomAnchor),

            footerHairline.topAnchor.constraint(equalTo: footerView.topAnchor),
            footerHairline.leadingAnchor.constraint(equalTo: footerView.leadingAnchor),
            footerHairline.trailingAnchor.constraint(equalTo: footerView.trailingAnchor),
            footerHairline.heightAnchor.constraint(equalToConstant: 0.5),

            footerStack.topAnchor.constraint(equalTo: footerView.topAnchor, constant: 9),
            footerStack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            footerStack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            footerStack.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -9),

            selectedNamesScrollView.heightAnchor.constraint(equalToConstant: 44),
            selectedNamesLabel.leadingAnchor.constraint(equalTo: selectedNamesScrollView.contentLayoutGuide.leadingAnchor, constant: 2),
            selectedNamesLabel.trailingAnchor.constraint(equalTo: selectedNamesScrollView.contentLayoutGuide.trailingAnchor, constant: -2),
            selectedNamesLabel.centerYAnchor.constraint(equalTo: selectedNamesScrollView.frameLayoutGuide.centerYAnchor),

            sendButton.widthAnchor.constraint(equalToConstant: 48),
            sendButton.heightAnchor.constraint(equalToConstant: 48),
        ])

        updateActionButtons()
    }

    private func stageShare() async {
        let intentChatId = suggestedChatIdFromIntent
        guard let payload = await stageShareToDisk(suggestedChatId: intentChatId) else {
            await MainActor.run {
                activityIndicator.stopAnimating()
                activityIndicator.isHidden = true
                titleLabel.text = "Nothing to share"
                statusLabel.isHidden = false
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
            await queueStagedShare(autoSend: true)
            return
        }

        let loaded = readSuggestions()
        await MainActor.run {
            activityIndicator.stopAnimating()
            activityIndicator.isHidden = true
            suggestions = loaded
            searchText = ""
            searchBar.text = nil
            selectedChatIds.removeAll()
            if loaded.isEmpty {
                titleLabel.text = "Choose recipients"
                statusLabel.isHidden = false
                statusLabel.text = "Open Iris Chat to choose a chat."
                chatTable.isHidden = true
                searchBar.isHidden = true
                footerView.isHidden = true
                openAppButton.isHidden = false
            } else {
                titleLabel.text = "Choose recipients"
                statusLabel.text = nil
                statusLabel.isHidden = true
                searchBar.isHidden = false
                chatTable.isHidden = false
                footerView.isHidden = false
                openAppButton.isHidden = true
                chatTable.reloadData()
            }
            updateActionButtons()
        }
    }

    @objc private func openMainApp() {
        Task { await openMainApp(autoSend: false) }
    }

    @objc private func cancelShare() {
        deleteStagedPayload()
        complete()
    }

    @objc private func sendSelectedChats() {
        guard let payload = stagedPayload else { return }
        let selectedIds = suggestions
            .map(\.chatId)
            .filter { selectedChatIds.contains($0) }
        guard !selectedIds.isEmpty else { return }

        let updated = StoredSharePayload(
            id: payload.id,
            text: payload.text,
            attachments: payload.attachments,
            suggestedChatId: selectedIds.first,
            suggestedChatIds: selectedIds,
            autoSend: true
        )
        stagedPayload = updated
        rewritePayloadOnDisk(updated)
        suggestions
            .filter { selectedChatIds.contains($0.chatId) }
            .forEach(donateChatInteraction)
        Task { await queueStagedShare(autoSend: true) }
    }

    private func queueStagedShare(autoSend: Bool) async {
        updateStagedPayload(autoSend: autoSend)
        await MainActor.run {
            isOpeningApp = true
            activityIndicator.isHidden = false
            activityIndicator.startAnimating()
            statusLabel.isHidden = false
            statusLabel.text = "Sending..."
            updateActionButtons()
        }
        notifyMainAppAboutPendingShare()
        await MainActor.run {
            complete()
        }
    }

    private func openMainApp(autoSend: Bool) async {
        updateStagedPayload(autoSend: autoSend)
        guard let url = shareURL(autoSend: autoSend) else {
            return
        }
        await MainActor.run {
            isOpeningApp = true
            activityIndicator.isHidden = false
            activityIndicator.startAnimating()
            statusLabel.isHidden = false
            statusLabel.text = "Opening Iris Chat..."
            updateActionButtons()
        }
        let opened = await openURLFromExtension(url)
        if opened {
            await MainActor.run {
                complete()
            }
        } else {
            await MainActor.run {
                isOpeningApp = false
                activityIndicator.stopAnimating()
                activityIndicator.isHidden = true
                statusLabel.isHidden = false
                statusLabel.text = autoSend ? "Open Iris Chat to finish." : "Could not open Iris Chat."
                updateActionButtons()
            }
        }
    }

    private func toggleChatSelection(_ entry: ShareSuggestionEntry, at indexPath: IndexPath) {
        if selectedChatIds.contains(entry.chatId) {
            selectedChatIds.remove(entry.chatId)
        } else {
            selectedChatIds.insert(entry.chatId)
        }
        chatTable.reloadRows(at: [indexPath], with: .none)
        updateActionButtons()
    }

    private func updateActionButtons() {
        let count = selectedChatIds.count
        let enabled = !isOpeningApp && count > 0 && stagedPayload != nil
        var sendConfig = sendButton.configuration ?? UIButton.Configuration.filled()
        sendConfig.title = nil
        sendConfig.image = UIImage(systemName: "arrow.up")
        sendConfig.imagePlacement = .leading
        sendConfig.cornerStyle = .capsule
        sendConfig.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0)
        sendConfig.baseForegroundColor = enabled ? .white : .secondaryLabel
        sendConfig.baseBackgroundColor = enabled ? ShareColors.action : ShareColors.disabledControl
        sendButton.configuration = sendConfig
        sendButton.isEnabled = enabled
        sendButton.accessibilityLabel = count > 1 ? "Send \(count)" : "Send"

        let names = suggestions
            .filter { selectedChatIds.contains($0.chatId) }
            .map { shareDisplayName(for: $0) }
            .joined(separator: ", ")
        selectedNamesLabel.text = names.isEmpty ? " " : names
        selectedNamesLabel.textColor = names.isEmpty ? .secondaryLabel : .label
        selectedNamesScrollView.isAccessibilityElement = !names.isEmpty
        selectedNamesScrollView.accessibilityLabel = names

        openAppButton.isEnabled = !isOpeningApp && stagedPayload != nil
        cancelButton.isEnabled = !isOpeningApp
    }

    private func openURLFromExtension(_ url: URL) async -> Bool {
        await extensionContext?.open(url) == true
    }

    private func notifyMainAppAboutPendingShare() {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName(pendingShareNotificationName as CFString),
            nil,
            nil,
            true
        )
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

    private func updateStagedPayload(autoSend: Bool) {
        guard let payload = stagedPayload else { return }
        let updated = StoredSharePayload(
            id: payload.id,
            text: payload.text,
            attachments: payload.attachments,
            suggestedChatId: payload.suggestedChatId,
            suggestedChatIds: payload.suggestedChatIds,
            autoSend: payload.autoSend == true || autoSend
        )
        stagedPayload = updated
        rewritePayloadOnDisk(updated)
    }

    private func deleteStagedPayload() {
        guard let payload = stagedPayload else { return }
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            return
        }
        let sharesDir = container.appendingPathComponent("pending-shares", isDirectory: true)
        let payloadURL = sharesDir.appendingPathComponent(payload.id).appendingPathExtension("json")
        let filesURL = sharesDir.appendingPathComponent("\(payload.id)-files", isDirectory: true)
        try? FileManager.default.removeItem(at: payloadURL)
        try? FileManager.default.removeItem(at: filesURL)
        stagedPayload = nil
        stagedShareID = nil
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
            suggestedChatId: suggestedChatId,
            suggestedChatIds: suggestedChatId.map { [$0] },
            autoSend: suggestedChatId != nil
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
                    continuation.resume(returning: url.isFileURL ? nil : url.absoluteString)
                } else if let string = item as? String {
                    let url = URL(string: string)
                    continuation.resume(returning: url?.isFileURL == true ? nil : string)
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
            $0 != UTType.plainText.identifier &&
                $0 != UTType.text.identifier
        }
        guard let type else {
            return nil
        }
        let suggestedName = provider.suggestedName
        let fallbackExtension = UTType(type)?.preferredFilenameExtension
        let copiedFile = await withCheckedContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { sourceURL, _ in
                if let sourceURL {
                    continuation.resume(
                        returning: copySharedAttachment(
                            from: sourceURL,
                            to: filesDir,
                            suggestedName: suggestedName,
                            fallbackExtension: fallbackExtension
                        )
                    )
                    return
                }
                continuation.resume(returning: nil)
            }
        }
        if let copiedFile {
            return copiedFile
        }
        return await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                if let sourceURL = item as? URL, sourceURL.isFileURL {
                    continuation.resume(
                        returning: copySharedAttachment(
                            from: sourceURL,
                            to: filesDir,
                            suggestedName: suggestedName,
                            fallbackExtension: fallbackExtension
                        )
                    )
                } else {
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
        filteredSuggestions.count
    }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(
            withIdentifier: ShareChatCell.reuseId,
            for: indexPath
        ) as? ShareChatCell ?? ShareChatCell(style: .default, reuseIdentifier: ShareChatCell.reuseId)
        let entry = filteredSuggestions[indexPath.row]
        cell.configure(with: entry, selected: selectedChatIds.contains(entry.chatId))
        return cell
    }

    func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        guard !filteredSuggestions.isEmpty else { return nil }
        let header = tableView.dequeueReusableHeaderFooterView(withIdentifier: ShareSectionHeaderView.reuseId)
            as? ShareSectionHeaderView ?? ShareSectionHeaderView(reuseIdentifier: ShareSectionHeaderView.reuseId)
        let title = searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Recent chats" : "Chats"
        header.configure(title: title)
        return header
    }

    func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
        filteredSuggestions.isEmpty ? .leastNormalMagnitude : 34
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        toggleChatSelection(filteredSuggestions[indexPath.row], at: indexPath)
    }
}

extension ShareViewController: UISearchBarDelegate {
    func searchBar(_ searchBar: UISearchBar, textDidChange searchText: String) {
        self.searchText = searchText
        chatTable.reloadData()
    }

    func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
        searchBar.resignFirstResponder()
    }
}

private final class ShareChatCell: UITableViewCell {
    static let reuseId = "ShareChatCell"

    private let selectionView = UIImageView()
    private let avatarLabel = UILabel()
    private let nameLabel = UILabel()
    private let subtitleLabel = UILabel()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)

        selectionStyle = .none
        backgroundColor = ShareColors.cellBackground
        contentView.backgroundColor = ShareColors.cellBackground

        selectionView.contentMode = .scaleAspectFit
        selectionView.preferredSymbolConfiguration = UIImage.SymbolConfiguration(
            pointSize: 24,
            weight: .semibold
        )
        selectionView.translatesAutoresizingMaskIntoConstraints = false

        avatarLabel.textAlignment = .center
        avatarLabel.textColor = .white
        avatarLabel.font = .systemFont(ofSize: 17, weight: .semibold)
        avatarLabel.layer.cornerRadius = 20
        avatarLabel.layer.masksToBounds = true
        avatarLabel.translatesAutoresizingMaskIntoConstraints = false

        nameLabel.font = .systemFont(ofSize: 17)
        nameLabel.textColor = .label
        nameLabel.translatesAutoresizingMaskIntoConstraints = false

        subtitleLabel.font = .systemFont(ofSize: 13)
        subtitleLabel.textColor = .secondaryLabel
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false

        contentView.addSubview(avatarLabel)
        contentView.addSubview(nameLabel)
        contentView.addSubview(subtitleLabel)
        contentView.addSubview(selectionView)

        NSLayoutConstraint.activate([
            avatarLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            avatarLabel.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            avatarLabel.widthAnchor.constraint(equalToConstant: 40),
            avatarLabel.heightAnchor.constraint(equalToConstant: 40),

            nameLabel.leadingAnchor.constraint(equalTo: avatarLabel.trailingAnchor, constant: 12),
            nameLabel.trailingAnchor.constraint(equalTo: selectionView.leadingAnchor, constant: -12),
            nameLabel.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 9),

            subtitleLabel.leadingAnchor.constraint(equalTo: nameLabel.leadingAnchor),
            subtitleLabel.trailingAnchor.constraint(equalTo: nameLabel.trailingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: nameLabel.bottomAnchor, constant: 2),

            selectionView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            selectionView.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            selectionView.widthAnchor.constraint(equalToConstant: 32),
            selectionView.heightAnchor.constraint(equalToConstant: 40),
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func configure(with entry: ShareSuggestionEntry, selected: Bool) {
        let display = shareDisplayName(for: entry)
        nameLabel.text = display
        let trimmedSubtitle = entry.subtitle?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        subtitleLabel.text = trimmedSubtitle.isEmpty ? nil : trimmedSubtitle
        subtitleLabel.isHidden = subtitleLabel.text == nil

        let firstChar = display.unicodeScalars.first.map { String($0).uppercased() } ?? "?"
        avatarLabel.text = firstChar
        avatarLabel.backgroundColor = avatarColor(for: entry.chatId)
        selectionView.image = UIImage(systemName: selected ? "checkmark.circle.fill" : "circle")
        selectionView.tintColor = selected ? ShareColors.action : .tertiaryLabel
        accessibilityLabel = "\(display), \(selected ? "selected" : "not selected")"
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

private final class ShareSectionHeaderView: UITableViewHeaderFooterView {
    static let reuseId = "ShareSectionHeaderView"

    private let titleLabel = UILabel()

    override init(reuseIdentifier: String?) {
        super.init(reuseIdentifier: reuseIdentifier)

        contentView.backgroundColor = ShareColors.presentedBackground
        titleLabel.font = .systemFont(ofSize: 13, weight: .semibold)
        titleLabel.textColor = .secondaryLabel
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(titleLabel)

        NSLayoutConstraint.activate([
            titleLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            titleLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            titleLabel.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -7),
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func configure(title: String) {
        titleLabel.text = title
    }
}

private enum ShareColors {
    static let presentedBackground = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 28.0 / 255.0, green: 28.0 / 255.0, blue: 30.0 / 255.0, alpha: 1)
            : UIColor(red: 239.0 / 255.0, green: 239.0 / 255.0, blue: 240.0 / 255.0, alpha: 1)
    }

    static let cellBackground = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 44.0 / 255.0, green: 44.0 / 255.0, blue: 46.0 / 255.0, alpha: 1)
            : .white
    }

    static let footerBackground = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 27.0 / 255.0, green: 27.0 / 255.0, blue: 27.0 / 255.0, alpha: 1)
            : UIColor(red: 246.0 / 255.0, green: 246.0 / 255.0, blue: 246.0 / 255.0, alpha: 1)
    }

    static let action = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(red: 45.0 / 255.0, green: 112.0 / 255.0, blue: 250.0 / 255.0, alpha: 1)
            : UIColor(red: 34.0 / 255.0, green: 103.0 / 255.0, blue: 245.0 / 255.0, alpha: 1)
    }

    static let disabledControl = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(white: 1, alpha: 0.18)
            : UIColor(white: 0, alpha: 0.12)
    }

    static let separator = UIColor { trait in
        trait.userInterfaceStyle == .dark
            ? UIColor(white: 1, alpha: 0.12)
            : UIColor(white: 0, alpha: 0.08)
    }
}

private func shareDisplayName(for entry: ShareSuggestionEntry) -> String {
    let trimmed = entry.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? "Chat" : trimmed
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

private func copySharedAttachment(
    from sourceURL: URL,
    to filesDir: URL,
    suggestedName: String?,
    fallbackExtension: String?
) -> StoredShareAttachment? {
    let filename = safeFilename(
        suggestedName ?? sourceURL.lastPathComponent,
        fallbackExtension: fallbackExtension
    )
    let destination = uniqueDestination(in: filesDir, filename: filename)
    let accessed = sourceURL.startAccessingSecurityScopedResource()
    defer {
        if accessed {
            sourceURL.stopAccessingSecurityScopedResource()
        }
    }
    do {
        try FileManager.default.copyItem(at: sourceURL, to: destination)
        return StoredShareAttachment(
            path: destination.path,
            filename: destination.lastPathComponent
        )
    } catch {
        return nil
    }
}
