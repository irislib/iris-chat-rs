import Combine
import Foundation

/// ChatScreen retains this reference without observing it. Only the composer
/// observes text edits, so typing doesn't invalidate the message timeline.
@MainActor
final class IrisComposerState: ObservableObject {
    @Published var text = "" {
        didSet {
            if text != oldValue { hasLocalEdits = true }
        }
    }
    var lastTypingSentAt: Date?
    var sentTypingIndicator = false
    private var lastPersistedText: String?
    private var pendingSave: DispatchWorkItem?
    // An empty composer can be a local edit (delete/send), not an invitation
    // to restore an older snapshot. Reset ownership only when opening a chat.
    private var hasLocalEdits = false

    func restore(_ persisted: String, replaceExisting: Bool) {
        if replaceExisting || !hasLocalEdits {
            pendingSave?.cancel()
            pendingSave = nil
            lastPersistedText = persisted
            text = persisted
            hasLocalEdits = false
        } else if text == persisted {
            lastPersistedText = persisted
        }
    }

    func clearForSend(_ persist: (String) -> Void) {
        hasLocalEdits = true
        text = ""
        // Cancel the old debounce immediately, before queued core updates or
        // the next SwiftUI onChange can bring the outgoing text back.
        flush(persist)
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
