import Foundation

/// The app supplies profile thumbnails; the extension reads this local snapshot.
/// Keeping this handoff independent of the messaging engine lets native tests
/// exercise the same persistence and loading code without opening an account.
@MainActor
final class ShareSuggestionsStore {
    private let container: URL?
    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.share-suggestions", qos: .utility)
    private var lastWritten: Data?
    private var exportTask: Task<Void, Never>?
    private var thumbnails: [String: Data] = [:]
    private var savedEntries: [ShareSuggestionEntry] = []
    private var thumbnailKeys: [String: String] = [:]
    private var loadThumbnail: ((String) async -> Data?)?
    private var onAvatars: (([String: Data]) -> Void)?

    init(container: URL?) {
        self.container = container
        if let container,
           let data = try? Data(contentsOf: container.appendingPathComponent("share-suggestions.json")),
           let entries = try? JSONDecoder().decode([ShareSuggestionEntry].self, from: data) {
            savedEntries = entries
        }
    }

    func export(
        entries: [ShareSuggestionEntry], thumbnailKeys: [String: String],
        cachedThumbnails: [String: Data] = [:],
        loadThumbnail: @escaping (String) async -> Data?,
        onAvatars: @escaping ([String: Data]) -> Void
    ) {
        self.thumbnailKeys = thumbnailKeys
        self.loadThumbnail = loadThumbnail
        self.onAvatars = onAvatars
        var entries = Array(entries.sorted { ($0.lastMessageAtSecs ?? 0) > ($1.lastMessageAtSecs ?? 0) }.prefix(20))
        let activeKeys = Set(entries.compactMap { thumbnailKeys[$0.chatId] })
        thumbnails = thumbnails.filter { activeKeys.contains($0.key) }
        for index in entries.indices {
            guard let key = thumbnailKeys[entries[index].chatId] else { continue }
            let saved = savedEntries.first {
                $0.chatId == entries[index].chatId && $0.pictureUrl == entries[index].pictureUrl
            }?.avatarData
            entries[index].avatarData = thumbnails[key] ?? saved ?? cachedThumbnails[key]
            if let data = entries[index].avatarData { thumbnails[key] = data }
        }
        publish(entries)
        // A new message must not cancel an in-flight profile photo. Follow the
        // latest recipient list when the existing request completes instead.
        guard exportTask == nil else { return }
        exportTask = Task { [weak self] in
            var attempted = Set<String>()
            while let self, !Task.isCancelled,
                  let key = self.savedEntries.compactMap({ self.thumbnailKeys[$0.chatId] }).first(where: {
                      self.thumbnails[$0] == nil && !attempted.contains($0)
                  }) {
                attempted.insert(key)
                guard let data = await self.loadThumbnail?(key), !data.isEmpty,
                      !Task.isCancelled, self.thumbnailKeys.values.contains(key) else { continue }
                self.thumbnails[key] = data
                let updated = self.savedEntries.map { entry in
                    var entry = entry
                    if self.thumbnailKeys[entry.chatId] == key { entry.avatarData = data }
                    return entry
                }
                self.publish(updated)
            }
            self?.exportTask = nil
        }
    }

    private func publish(_ entries: [ShareSuggestionEntry]) {
        savedEntries = entries
        onAvatars?(Dictionary(entries.compactMap { entry in
            entry.avatarData.map { (entry.chatId, $0) }
        }, uniquingKeysWith: { _, latest in latest }))
        guard let container, let data = try? JSONEncoder().encode(entries), data != lastWritten else { return }
        lastWritten = data
        queue.async {
            try? data.write(to: container.appendingPathComponent("share-suggestions.json"), options: .atomic)
        }
    }
}
