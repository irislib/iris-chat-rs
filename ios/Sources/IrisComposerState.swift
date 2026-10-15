import Combine
import Foundation

/// ChatScreen retains this reference without observing it. Only the composer
/// observes text edits, so typing doesn't invalidate the message timeline.
@MainActor
final class IrisComposerState: ObservableObject {
    @Published var text = "" {
        didSet {
            if text != oldValue {
                if !restoringMentions, pendingNativeText != text { mentions.discard() }
                pendingNativeText = nil
                hasLocalEdits = true
            }
        }
    }
    private var mentions = IrisMentionDraft()
    private var restoringMentions = false
    private var pendingNativeText: String?
    @Published private(set) var mentionRevision = 0

    func prepareNativeEdit(_ edit: IrisComposerNativeEdit) {
        pendingNativeText = edit.text
        guard edit.undoManager?.isUndoing != true, edit.undoManager?.isRedoing != true else { return }
        let previous = mentions
        edit.undoManager?.registerUndo(withTarget: self) { [weak undo = edit.undoManager] target in
            target.restoreMentionMetadata(previous, undo: undo)
        }
        let inserted = (edit.text as NSString).substring(with: NSRange(location: edit.range.location, length: edit.replacementLength))
        mentions.replace(range: edit.range, replacementLength: edit.replacementLength,
            insertingWord: inserted.first.map { $0.isLetter || $0.isNumber } ?? false)
        mentionRevision &+= 1
    }

    private func restoreMentionMetadata(_ value: IrisMentionDraft, undo: UndoManager?) {
        let previous = mentions
        undo?.registerUndo(withTarget: self) { [weak undo] target in
            target.restoreMentionMetadata(previous, undo: undo)
        }
        mentions = value
        mentionRevision &+= 1
    }
    var wireText: String { mentions.wireText(from: text) }

    func insertMention(label: String, token: String, owner: String, at location: Int) {
        mentions.insert(label: label, token: token, owner: owner, at: location)
        mentionRevision &+= 1
    }

    var lastTypingSentAt: Date?
    var sentTypingIndicator = false
    private(set) var attachmentGeneration = UUID()
    private var lastPersistedText: String?
    private var pendingSave: DispatchWorkItem?
    // An empty composer can be a local edit (delete/send), not an invitation
    // to restore an older snapshot. Reset ownership only when opening a chat.
    private var hasLocalEdits = false

    func restore(_ persisted: String, replaceExisting: Bool) {
        if replaceExisting { invalidatePendingAttachments() }
        if replaceExisting || !hasLocalEdits {
            pendingSave?.cancel()
            pendingSave = nil
            lastPersistedText = persisted
            restoringMentions = true
            let encoded = persisted as NSString
            let spans = messageMentions(text: persisted).map { mention in
                let range = NSRange(location: Int(mention.start), length: Int(mention.length))
                return IrisMentionDraft.Span(range: range, label: mention.label,
                    token: encoded.substring(with: range), owner: mention.ownerPubkeyHex)
            }
            text = mentions.restore(wireText: persisted, encodedSpans: spans)
            restoringMentions = false
            hasLocalEdits = false
        } else if wireText == persisted {
            lastPersistedText = persisted
        }
    }

    func clearForSend(_ persist: (String) -> Void) {
        invalidatePendingAttachments()
        hasLocalEdits = true
        text = ""
        // Cancel the old debounce immediately, before queued core updates or
        // the next SwiftUI onChange can bring the outgoing text back.
        flush(persist)
    }

    func scheduleSave(_ persist: @escaping (String) -> Void) {
        pendingSave?.cancel()
        pendingSave = nil
        guard lastPersistedText != wireText else { return }
        let value = wireText
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.pendingSave = nil
            self.lastPersistedText = value
            persist(value)
        }
        pendingSave = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }

    func invalidatePendingAttachments() {
        attachmentGeneration = UUID()
    }

    func flush(_ persist: (String) -> Void) {
        pendingSave?.cancel()
        pendingSave = nil
        guard lastPersistedText != wireText else { return }
        lastPersistedText = wireText
        persist(wireText)
    }

    deinit {
        pendingSave?.cancel()
    }
}
