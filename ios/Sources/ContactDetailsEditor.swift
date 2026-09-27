import SwiftUI

struct ContactDetailsEditor: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.dismiss) private var dismiss
    @State private var nickname: String
    @State private var note: String
    private let initialNickname: String
    private let initialNote: String
    private let onSave: (String, String) -> Void

    init(nickname: String, note: String, onSave: @escaping (String, String) -> Void) {
        _nickname = State(initialValue: nickname)
        _note = State(initialValue: note)
        initialNickname = nickname
        initialNote = note
        self.onSave = onSave
    }

    private var normalizedNickname: String {
        nickname.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }
    private var normalizedNote: String {
        note.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
    private var valid: Bool { normalizedNickname.unicodeScalars.count <= 80 && normalizedNote.unicodeScalars.count <= 240 }
    private var changed: Bool { normalizedNickname != initialNickname || normalizedNote != initialNote }

    var body: some View {
        VStack(spacing: 18) {
            HStack(spacing: 12) {
                Button("Cancel") { dismiss() }
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("directChatCancelNicknameButton")
                Spacer(minLength: 0)
                Button("Save") { save(nickname, note) }
                    .buttonStyle(IrisPrimaryButtonStyle(compact: true))
                    .disabled(!valid || !changed)
                    .accessibilityIdentifier("directChatSaveNicknameButton")
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Nickname and note")
                        .font(.system(.title2, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                    Text("Only you can see this.")
                        .font(.footnote)
                        .foregroundStyle(palette.muted)
                    TextField("Nickname", text: $nickname)
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .submitLabel(.done)
                        .onSubmit { if valid && changed { save(nickname, note) } }
                        .accessibilityIdentifier("directChatNicknameField")
                    if normalizedNickname.unicodeScalars.count > 80 {
                        Text("Use up to 80 characters for a nickname.")
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                    TextField("Note", text: $note, axis: .vertical)
                        .lineLimit(3...6)
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("directChatNoteField")
                    if normalizedNote.unicodeScalars.count >= 140 {
                        Text("\(normalizedNote.unicodeScalars.count)/240")
                            .font(.footnote)
                            .foregroundStyle(palette.muted)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                    if !initialNickname.isEmpty || !initialNote.isEmpty {
                        Button("Remove nickname and note", role: .destructive) { save("", "") }
                            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                            .accessibilityIdentifier("directChatRemoveNicknameButton")
                    }
                }
            }
            .irisInteractiveKeyboardDismiss()
        }
        .padding(20)
        .background(palette.background)
        #if os(macOS)
        .frame(width: 440, height: 400)
        #else
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        #endif
    }

    private func save(_ nickname: String, _ note: String) {
        onSave(nickname, note)
        dismiss()
    }
}
