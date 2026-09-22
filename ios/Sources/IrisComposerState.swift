import Combine
import Foundation

/// ChatScreen retains this reference without observing it. Only the composer
/// observes text edits, so typing doesn't invalidate the message timeline.
@MainActor
final class IrisComposerState: ObservableObject {
    @Published var text = ""
    var lastTypingSentAt: Date?
    var sentTypingIndicator = false
    private var lastPersistedText: String?
    private var pendingSave: DispatchWorkItem?

    func restore(_ persisted: String, replaceExisting: Bool) {
        if replaceExisting || text.isEmpty {
            pendingSave?.cancel()
            pendingSave = nil
            lastPersistedText = persisted
            text = persisted
        } else if text == persisted {
            lastPersistedText = persisted
        }
    }

    func scheduleSave(_ persist: @escaping (String) -> Void) {
        pendingSave?.cancel()
        pendingSave = nil
        guard lastPersistedText != text else { return }
        let value = text
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.pendingSave = nil
            self.lastPersistedText = value
            persist(value)
        }
        pendingSave = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }

    func flush(_ persist: (String) -> Void) {
        pendingSave?.cancel()
        pendingSave = nil
        guard lastPersistedText != text else { return }
        lastPersistedText = text
        persist(text)
    }

    deinit {
        pendingSave?.cancel()
    }
}
