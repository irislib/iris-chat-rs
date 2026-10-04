import Foundation
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

struct IrisComposerBar: View {
    @Environment(\.irisPalette) private var palette

    @ObservedObject var composerState: IrisComposerState
    @Binding var attachments: [StagedAttachment]
    @Binding var sendFilesDirectly: Bool
    let directFilesAllowed: Bool
    @State private var showingAttachmentPicker = false
    @State private var attachmentContentTypes: [UTType] = [.item]
    @State private var showingEmojiPicker = false
    @State private var isPreparingAttachments = false
    @State private var attachmentTask: Task<Void, Never>?
    #if os(iOS)
    @StateObject private var voiceRecorder = IrisVoiceMessageRecorder.forComposer()
    @State private var voiceSendTask: Task<Void, Never>?
    @State private var isStagingVoice = false
    @State private var showingAttachmentSheet = false
    @State private var showingAttachmentCamera = false
    @State private var pendingAttachmentSource: IrisAttachmentSource?
    #endif
    #if canImport(PhotosUI)
    @State private var showingPhotoPicker = false
    @State private var pickedPhotos: [PhotosPickerItem] = []
    #endif

    let placeholder: String
    let isSending: Bool
    let isUploading: Bool
    let uploadFraction: Double?
    @Binding var isFocused: Bool
    let onUserEdit: (String) -> Void
    let onDraftChange: () -> Void
    let onAttach: (() async -> [URL]) async -> Void
    let voiceRecordingAllowed: Bool
    let onStageVoice: (URL) async throws -> [StagedAttachment]
    let onSendVoice: ([StagedAttachment]) -> Bool
    var sendAllowed = true
    var isEditing = false
    var isPreparingDroppedAttachments = false
    var onFileDropAvailabilityChange: (Bool) -> Void = { _ in }
    let onSend: (String) -> Void

    private var draft: String { composerState.text }

    private func canSend(text: String) -> Bool {
        (
            !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
            !attachments.isEmpty
        ) && sendAllowed && !isSending && !isUploading && !isPreparingAttachments && !isPreparingDroppedAttachments && !voiceActive
    }

    private var canSend: Bool { canSend(text: draft) && !voiceActive }

    private var voiceActive: Bool {
        #if os(iOS)
        voiceRecorder.phase != .idle
        #else
        false
        #endif
    }


    var body: some View {
        VStack(spacing: 8) {
            if !attachments.isEmpty && sendFilesDirectly {
                Label("Send directly · Both devices must stay online", systemImage: "arrow.up.arrow.down")
                    .font(.caption)
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("chatDirectFileMode")
            }
            if !attachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(attachments) { attachment in
                            IrisSelectedAttachmentChip(
                                attachment: attachment,
                                enabled: !isSending && !isUploading && !isPreparingAttachments && !isPreparingDroppedAttachments
                            ) {
                                attachments.removeAll { $0 == attachment }
                                Task.detached(priority: .utility) {
                                    try? FileManager.default.removeItem(atPath: attachment.path)
                                }
                                if attachments.isEmpty { sendFilesDirectly = false }
                            }
                        }
                    }
                    .padding(.horizontal, 1)
                }
                .accessibilityIdentifier("chatSelectedAttachments")
            }

            if isPreparingAttachments || isPreparingDroppedAttachments {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Adding attachments…")
                        .font(.system(.caption, design: .rounded))
                        .foregroundStyle(palette.muted)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("chatAttachmentLoading")
            }

            if isUploading {
                VStack(alignment: .leading, spacing: 5) {
                    Text("Uploading")
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                    if let fraction = uploadFraction {
                        ProgressView(value: fraction)
                            .progressViewStyle(.linear)
                            .tint(palette.accent)
                    } else {
                        ProgressView()
                            .progressViewStyle(.linear)
                            .tint(palette.accent)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            composerRow
                .animation(.spring(response: 0.32, dampingFraction: 0.72), value: canSend)
                .animation(.spring(response: 0.32, dampingFraction: 0.72), value: isSending)
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 14 : 8)
        // 6pt vertical breathing room around the glass elements so
        // the composer doesn't sit flush against the keyboard top
        // edge (or the home-indicator on devices without a keyboard).
        // No outer background — the elements still float as separate
        // glass discs over the timeline.
        .padding(.vertical, 6)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatComposerBar")
        .irisOnChange(of: draft) { _ in onDraftChange() }
        .frame(maxWidth: .infinity)
        .onAppear { onFileDropAvailabilityChange(canPrepareAttachments) }
        #if os(macOS)
        .modifier(DesktopComposerKeyboardFocus(isFocused: $isFocused))
        #endif
        .irisOnChange(of: canPrepareAttachments) { onFileDropAvailabilityChange($0) }
        .fileImporter(
            isPresented: $showingAttachmentPicker,
            allowedContentTypes: attachmentContentTypes,
            allowsMultipleSelection: true
        ) { result in
            guard case .success(let urls) = result, !urls.isEmpty else {
                if attachments.isEmpty { sendFilesDirectly = false }
                return
            }
            prepareAttachments { urls }
        }
        .onDisappear {
            attachmentTask?.cancel()
            attachmentTask = nil
            isPreparingAttachments = false
            onFileDropAvailabilityChange(false)
        }
        #if os(iOS)
        .onDisappear { voiceSendTask?.cancel(); voiceRecorder.cancel() }
        .onReceive(NotificationCenter.default.publisher(for: UIApplication.didEnterBackgroundNotification)) { _ in
            voiceSendTask?.cancel()
        }
        .irisOnChange(of: voiceRecordingAllowed) { allowed in
            if !allowed {
                voiceSendTask?.cancel()
                Task { await voiceRecorder.finishForInterruption() }
            }
        }
        .alert("Voice message", isPresented: Binding(
            get: { voiceRecorder.errorMessage != nil },
            set: { if !$0 { voiceRecorder.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { voiceRecorder.errorMessage = nil }
        } message: { Text(voiceRecorder.errorMessage ?? "") }
        .sheet(isPresented: $showingAttachmentSheet, onDismiss: presentAttachmentSource) {
            IrisAttachmentPicker(
                directFilesAllowed: directFilesAllowed,
                onSource: { source in
                    sendFilesDirectly = irisDirectFileSendMode(current: sendFilesDirectly, hasFiles: !attachments.isEmpty, selectedDirectly: source == .directFiles)
                    pendingAttachmentSource = source
                    showingAttachmentSheet = false
                },
                onPhotos: { items in
                    if attachments.isEmpty { sendFilesDirectly = false }
                    showingAttachmentSheet = false
                    handlePickedPhotos(items)
                }
            )
            .irisModalSurface()
        }
        .fullScreenCover(isPresented: $showingAttachmentCamera) {
            IrisCameraImagePicker { url in prepareAttachments { [url] } }
                .ignoresSafeArea()
        }
        #endif
        #if canImport(PhotosUI)
        .photosPicker(
            isPresented: $showingPhotoPicker,
            selection: $pickedPhotos,
            maxSelectionCount: 10,
            matching: .any(of: [.images, .videos])
        )
        .irisOnChange(of: pickedPhotos) { items in
            handlePickedPhotos(items)
        }
        #endif
    }

    @ViewBuilder
    private var composerRow: some View {
        #if os(iOS)
        if voiceRecorder.phase == .ready, let url = voiceRecorder.recordingURL {
            HStack(spacing: 8) {
                Button { voiceRecorder.cancel(userInitiated: true) } label: {
                    Image(systemName: "trash.fill")
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)
                .foregroundStyle(palette.muted)
                .accessibilityLabel("Delete voice message")
                .accessibilityIdentifier("chatVoiceDeleteButton")
                .disabled(isStagingVoice)
                IrisAudioPlaybackControl(localURL: url, duration: voiceRecorder.duration)
                Button(action: sendVoiceMessage) {
                    IrisSendButtonLabel(isSending: isStagingVoice)
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)
                .disabled(!voiceRecordingAllowed || isSending || isUploading || isStagingVoice)
                .accessibilityLabel("Send voice message")
                .accessibilityIdentifier("chatVoiceSendButton")
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("chatVoicePreview")
        } else {
            HStack(alignment: .bottom, spacing: 8) {
                if voiceActive { IrisVoiceRecordingStatus(recorder: voiceRecorder) }
                else { textControls }
                if !isEditing && (voiceActive || (draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attachments.isEmpty)) {
                    IrisVoiceRecordButton(
                        recorder: voiceRecorder,
                        enabled: voiceRecordingAllowed && !isSending && !isUploading && !isPreparingAttachments,
                        onBegin: {
                            isFocused = false
                            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
                        },
                        onSend: sendVoiceMessage
                    )
                } else { sendControl }
            }
        }
        #else
        HStack(alignment: .bottom, spacing: 8) { textControls; sendControl }
        #endif
    }

    @ViewBuilder
    private var textControls: some View {
        if !isEditing { attachmentControl }
        if IrisLayout.usesDesktopChrome {
            Button { showingEmojiPicker.toggle() } label: {
                Image(systemName: "face.smiling.fill")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(isSending || isUploading ? palette.muted.opacity(0.54) : palette.textPrimary)
                    .frame(width: 40, height: 40)
                    .irisGlassSurface(in: Circle())
            }
            .buttonStyle(.irisPlain)
            .disabled(isSending || isUploading)
            .popover(isPresented: $showingEmojiPicker, arrowEdge: .bottom) {
                IrisEmojiPicker { emoji in insertEmoji(emoji); showingEmojiPicker = false }
            }
            .accessibilityIdentifier("chatEmojiButton")
        }
        composerInput
    }

    @ViewBuilder
    private var sendControl: some View {
        if isEditing || (!IrisLayout.usesDesktopChrome && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty || isSending)) {
            Button(action: submitDraft) {
                Group {
                    if isEditing {
                        Text("Save").font(.callout.weight(.semibold))
                    } else {
                        IrisSendButtonLabel(isSending: isSending)
                    }
                }
                    .frame(width: 40, height: 40)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .disabled(!canSend)
            .opacity(canSend || isSending ? 1 : 0.45)
            .accessibilityIdentifier("chatSendButton")
            .transition(.scale(scale: 0.4).combined(with: .opacity))
        }
    }

    #if os(iOS)
    private func sendVoiceMessage() {
        guard voiceRecordingAllowed, !isSending, !isUploading, !isStagingVoice else { return }
        isStagingVoice = true
        voiceSendTask = Task {
            defer { isStagingVoice = false; voiceSendTask = nil }
            let url = voiceRecorder.phase == .ready ? voiceRecorder.recordingURL : await voiceRecorder.finish()
            guard let url, !Task.isCancelled, voiceRecorder.phase == .ready else { return }
            IrisAudioPlayback.pauseAll()
            do {
                let staged = try await onStageVoice(url)
                guard !Task.isCancelled, !IrisAudioActivity.isCallActive,
                      UIApplication.shared.applicationState == .active,
                      voiceRecorder.phase == .ready, voiceRecorder.recordingURL == url,
                      onSendVoice(staged) else {
                    Task.detached(priority: .utility) {
                        for attachment in staged { try? FileManager.default.removeItem(atPath: attachment.path) }
                    }
                    return
                }
                voiceRecorder.cancel()
            } catch {
                if !Task.isCancelled, voiceRecorder.recordingURL == url {
                    voiceRecorder.errorMessage = "Couldn’t send the voice message. Try again."
                }
            }
        }
    }
    #endif

    @ViewBuilder
    private var composerInput: some View {
        #if os(iOS)
        ZStack(alignment: .topLeading) {
            if draft.isEmpty {
                Text(placeholder)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .padding(.top, 1)
                    .allowsHitTesting(false)
            }
            IrisUIKitComposerTextView(
                text: userEditingDraft,
                isFocused: $isFocused,
                onPasteAttachments: canPrepareAttachments ? pasteAttachments : nil
            )
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .irisGlassSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .strokeBorder(palette.border.opacity(0.32), lineWidth: 0.5)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            isFocused = true
        }
        #else
        ZStack(alignment: .topLeading) {
            if draft.isEmpty {
                Text(placeholder)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .offset(y: 1)
                    .allowsHitTesting(false)
            }
            IrisAppKitComposerTextView(
                text: userEditingDraft,
                isFocused: $isFocused,
                onSubmit: submitDraft,
                onPasteAttachments: canPrepareAttachments ? pasteAttachments : nil
            )
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .irisInputField(verticalPadding: IrisAppKitComposerTextView.verticalPadding)
        .contentShape(Rectangle())
        .onTapGesture {
            isFocused = true
        }
        .accessibilityIdentifier("chatMessageInput")
        #endif
    }

    @ViewBuilder
    private var attachmentControl: some View {
        #if os(iOS) && canImport(PhotosUI)
        Button {
            isFocused = false
            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
            pendingAttachmentSource = nil
            showingAttachmentSheet = true
        } label: {
            attachmentControlLabel
        }
        .buttonStyle(.irisPlain)
        .disabled(isSending || isUploading || isPreparingAttachments)
        .accessibilityIdentifier("chatAttachButton")
        #else
        Menu {
            Button {
                if attachments.isEmpty { sendFilesDirectly = false }
                attachmentContentTypes = [.image, .movie]
                showingAttachmentPicker = true
            } label: { Label("Photos and videos", systemImage: "photo.on.rectangle") }
            .labelStyle(.titleAndIcon)
            .accessibilityIdentifier("chatAttachmentPhotosButton")
            Button {
                if attachments.isEmpty { sendFilesDirectly = false }
                attachmentContentTypes = [.item]
                showingAttachmentPicker = true
            } label: { Label("File", systemImage: "doc.fill") }
            .labelStyle(.titleAndIcon)
            .accessibilityIdentifier("chatAttachmentFilesButton")
            if directFilesAllowed {
                Divider()
                Button {
                    sendFilesDirectly = true
                    attachmentContentTypes = [.item]
                    showingAttachmentPicker = true
                } label: { Label("Send directly", systemImage: "arrow.up.arrow.down") }
                .labelStyle(.titleAndIcon)
                .accessibilityIdentifier("chatDirectFileButton")
            }
        } label: {
            attachmentControlLabel
        }
        #if os(macOS)
        // Menu needs a visual ButtonStyle; the keyboard primitive style leaves native popup chrome.
        .menuStyle(.button)
        .menuIndicator(.hidden)
        .buttonStyle(IrisPlainButtonVisualStyle())
        .frame(width: 40, height: 40)
        #else
        .buttonStyle(.irisPlain)
        #endif
        .disabled(isSending || isUploading || isPreparingAttachments)
        .accessibilityIdentifier("chatAttachButton")
        #endif
    }

    private var attachmentControlLabel: some View {
        Image(systemName: isUploading || isPreparingAttachments ? "ellipsis" : "plus")
            .font(.system(size: 19, weight: .semibold))
            .foregroundStyle((isSending || isUploading) ? palette.muted.opacity(0.54) : palette.textPrimary)
            .frame(width: 40, height: 40)
            .contentShape(Circle())
            .irisGlassSurface(in: Circle())
            .accessibilityLabel("Add")
    }

    #if os(iOS)
    private func presentAttachmentSource() {
        let source = pendingAttachmentSource
        pendingAttachmentSource = nil
        // Present only after the attachment sheet has finished dismissing.
        switch source {
        case .camera: showingAttachmentCamera = true
        case .photos: showingPhotoPicker = true
        case .files, .directFiles: showingAttachmentPicker = true
        case nil: break
        }
    }
    #endif

    private var canPrepareAttachments: Bool {
        !isEditing && sendAllowed && !isSending && !isUploading && !voiceActive &&
            !isPreparingAttachments && !isPreparingDroppedAttachments
    }

    private func prepareAttachments(loadURLs: @escaping () async -> [URL]) {
        prepareAttachmentOperation { await onAttach(loadURLs) }
    }

    private func pasteAttachments(_ clipboard: IrisClipboardAttachments) {
        prepareAttachmentOperation { await clipboard.withURLs(onAttach) }
    }

    private func prepareAttachmentOperation(_ operation: @escaping () async -> Void) {
        guard canPrepareAttachments else { return }
        isPreparingAttachments = true
        attachmentTask = Task {
            defer {
                if !Task.isCancelled {
                    isPreparingAttachments = false
                    attachmentTask = nil
                }
            }
            await operation()
        }
    }

    #if canImport(PhotosUI)
    private func handlePickedPhotos(_ items: [PhotosPickerItem]) {
        guard !items.isEmpty else { return }
        let snapshot = items
        pickedPhotos = []
        prepareAttachments {
            var urls: [URL] = []
            for item in snapshot {
                guard !Task.isCancelled else { break }
                guard let url = await Self.loadPickedPhoto(item) else { continue }
                urls.append(url)
            }
            return urls
        }
    }

    nonisolated private static func loadPickedPhoto(_ item: PhotosPickerItem) async -> URL? {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            return nil
        }
        let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-photo-picks", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(UUID().uuidString).\(ext)")
        do {
            try data.write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }
    #endif

    private func submitDraft() {
        #if os(iOS)
        let text = IrisUIKitComposerTextView.currentText ?? draft
        #elseif canImport(AppKit)
        let text = IrisAppKitComposerTextView.currentText ?? draft
        #else
        let text = draft
        #endif
        _ = submitDraft(text)
    }

    private func submitDraft(_ text: String) -> IrisComposerSubmitResult {
        guard canSend(text: text) else {
            return .rejected
        }
        onSend(text)
        return .acceptedAndClear
    }

    private func insertEmoji(_ emoji: String) {
        #if canImport(AppKit)
        if IrisAppKitComposerTextView.insertTextAtSelection(emoji) != nil {
            return
        }
        #endif
        userEditingDraft.wrappedValue = draft + emoji
    }

    /// The parent owns draft restoration and clearing. Only mutations that
    /// enter through the editor are user activity and may emit typing.
    private var userEditingDraft: Binding<String> {
        irisComposerUserEditingBinding($composerState.text, onUserEdit: onUserEdit)
    }


}

enum IrisComposerSubmitResult: Equatable {
    case rejected
    case acceptedAndClear
}

func irisComposerUserEditingBinding(
    _ draft: Binding<String>,
    onUserEdit: @escaping (String) -> Void
) -> Binding<String> {
    Binding(
        get: { draft.wrappedValue },
        set: { newValue in
            guard draft.wrappedValue != newValue else { return }
            draft.wrappedValue = newValue
            onUserEdit(newValue)
        }
    )
}

#if os(iOS)
struct IrisUIKitComposerTextView: UIViewRepresentable {
    private static weak var activeTextView: UITextView?

    static var currentText: String? {
        activeTextView?.text
    }

    @Binding var text: String
    @Binding var isFocused: Bool
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)? = nil

    func makeUIView(context: Context) -> UITextView {
        // Explicit legacy layout keeps large paragraphs responsive while
        // retaining native keyboard, selection and undo behavior.
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        let container = NSTextContainer()
        storage.addLayoutManager(layoutManager)
        layoutManager.addTextContainer(container)
        // Use the designated initializer so Swift initializes the subclass's
        // stored properties along with the native text view.
        let textView = IrisComposerUITextView(frame: .zero, textContainer: container)
        textView.onPasteAttachments = onPasteAttachments
        Self.activeTextView = textView
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.font = UIFont.preferredFont(forTextStyle: .body)
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = UIColor.label
        textView.tintColor = UIColor.tintColor
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        // sizeThatFits owns the one-to-five-line height. Keep scrolling enabled
        // before insertion so a large paste does not change layout modes.
        textView.isScrollEnabled = true
        textView.scrollsToTop = false
        textView.returnKeyType = .default
        textView.keyboardDismissMode = .interactive
        textView.autocapitalizationType = .sentences
        textView.allowsEditingTextAttributes = false
        // Keep the native defaults so the user's keyboard preferences apply.
        textView.accessibilityIdentifier = "chatMessageInput"
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return textView
    }

    func updateUIView(_ uiView: UITextView, context: Context) {
        (uiView as? IrisComposerUITextView)?.onPasteAttachments = onPasteAttachments
        Self.activeTextView = uiView
        context.coordinator.parent = self
        var needsSelectionReveal = false
        if uiView.markedTextRange == nil, uiView.text != text {
            needsSelectionReveal = true
            let selectedRange = uiView.selectedRange
            uiView.text = text
            let textLength = (text as NSString).length
            if uiView.isFirstResponder, selectedRange.location <= textLength {
                uiView.selectedRange = NSRange(
                    location: selectedRange.location,
                    length: min(selectedRange.length, textLength - selectedRange.location)
                )
            } else {
                uiView.selectedRange = NSRange(location: textLength, length: 0)
            }
        }
        if needsSelectionReveal { (uiView as? IrisComposerUITextView)?.revealSelectionAfterNextLayout() }
        if isFocused && !uiView.isFirstResponder {
            DispatchQueue.main.async { [weak uiView, weak coordinator = context.coordinator] in
                guard let uiView, coordinator?.parent.isFocused == true else { return }
                uiView.becomeFirstResponder()
            }
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        let width = proposal.width ?? uiView.bounds.width
        guard width > 0 else { return nil }
        let height = min(max(measuredHeight(for: uiView, width: width), minHeight(for: uiView)), maxHeight(for: uiView))
        return CGSize(width: width, height: height)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    private func measuredHeight(for textView: UITextView, width: CGFloat) -> CGFloat {
        let measurement = (textView as? IrisComposerUITextView)?.composerMeasurement ?? IrisComposerTextMeasurement()
        return measurement.height(for: textView.textStorage, width: width, lineHeight: minHeight(for: textView))
    }

    private func minHeight(for textView: UITextView) -> CGFloat {
        ceil((textView.font ?? UIFont.preferredFont(forTextStyle: .body)).lineHeight)
    }

    private func maxHeight(for textView: UITextView) -> CGFloat {
        ceil((textView.font ?? UIFont.preferredFont(forTextStyle: .body)).lineHeight * 5)
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: IrisUIKitComposerTextView

        init(parent: IrisUIKitComposerTextView) {
            self.parent = parent
        }

        func textView(
            _ textView: UITextView,
            shouldChangeTextIn range: NSRange,
            replacementText text: String
        ) -> Bool {
            // Let UIKit apply the edit before publishing it back to SwiftUI.
            // Publishing here makes `updateUIView` race the in-flight selection.
            return true
        }

        func textViewDidChange(_ textView: UITextView) {
            guard parent.text != textView.text else { return }
            parent.text = textView.text
            (textView as? IrisComposerUITextView)?.revealSelectionAfterNextLayout()
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            parent.isFocused = true
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            parent.isFocused = false
        }
    }
}
#endif

#if canImport(AppKit)
struct IrisAppKitComposerTextView: NSViewRepresentable {
    private static weak var activeTextView: NSTextView?

    static var currentText: String? {
        activeTextView?.string
    }

    @discardableResult
    static func insertTextAtSelection(_ replacement: String) -> String? {
        guard let textView = activeTextView else {
            return nil
        }
        return insertTextAtSelection(replacement, into: textView)
    }

    @discardableResult
    static func insertTextAtSelection(_ replacement: String, into textView: NSTextView) -> String {
        textView.insertText(replacement, replacementRange: textView.selectedRange())
        return textView.string
    }

    @Binding var text: String
    @Binding var isFocused: Bool
    let onSubmit: (String) -> IrisComposerSubmitResult
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)? = nil

    func makeNSView(context: Context) -> IrisComposerScrollView {
        let scrollView = IrisComposerScrollView()
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.verticalScrollElasticity = .none
        scrollView.setAccessibilityIdentifier("chatMessageInput")

        let textView = IrisComposerNSTextView()
        textView.onPasteAttachments = onPasteAttachments
        Self.activeTextView = textView
        textView.drawsBackground = false
        textView.backgroundColor = .clear
        textView.font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        textView.textColor = .labelColor
        textView.insertionPointColor = .controlAccentColor
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = false
        textView.isRichText = false
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.isEditable = true
        textView.isSelectable = true
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = false
        textView.minSize = NSSize(width: 0, height: Self.lineHeight(for: textView))
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.autoresizingMask = [.width, .height]
        // Setting spelling options here overrides AppKit's system preferences.
        textView.setAccessibilityIdentifier("chatMessageInput")

        textView.string = text
        textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator

        scrollView.documentView = textView
        textView.composerFocusRequested = isFocused
        scrollView.revealSelectionAfterNextLayout(in: textView)
        return scrollView
    }

    func updateNSView(_ nsView: IrisComposerScrollView, context: Context) {
        guard let textView = nsView.documentView as? IrisComposerNSTextView else {
            return
        }

        Self.activeTextView = textView
        context.coordinator.parent = self
        textView.onPasteAttachments = onPasteAttachments
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator

        let nativeText = textView.string
        let nativeSelection = textView.selectedRange()
        context.coordinator.reconcile(textView)

        nsView.needsLayout = true
        if textView.string != nativeText || textView.selectedRange() != nativeSelection {
            nsView.revealSelectionAfterNextLayout(in: textView)
        }

        textView.composerFocusRequested = isFocused
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: IrisComposerScrollView, context: Context) -> CGSize? {
        guard let textView = nsView.documentView as? NSTextView else {
            return nil
        }
        return Self.fittingSize(
            for: textView,
            proposedWidth: proposal.width,
            actualWidth: nsView.contentView.bounds.width
        )
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    final class Coordinator: NSObject, NSTextViewDelegate, IrisComposerNSTextViewCommandDelegate {
        var parent: IrisAppKitComposerTextView
        var lastPublishedNativeText: String?

        init(parent: IrisAppKitComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else {
                return
            }
            let nativeText = textView.string
            let shouldPublish = parent.text != nativeText
            lastPublishedNativeText = shouldPublish ? nativeText : nil

            if let scrollView = textView.enclosingScrollView as? IrisComposerScrollView {
                scrollView.revealSelectionAfterNextLayout(in: textView)
            } else {
                textView.enclosingScrollView?.needsLayout = true
            }

            if shouldPublish {
                parent.text = nativeText
            }
        }

        func reconcile(_ textView: NSTextView) {
            irisReconcileComposerText(
                textView,
                bindingText: parent.text,
                lastPublishedNativeText: &lastPublishedNativeText
            )
        }

        func textDidBeginEditing(_ notification: Notification) {
            parent.isFocused = true
        }

        func textDidEndEditing(_ notification: Notification) {
            parent.isFocused = false
        }

        func composerTextViewDidSubmit(_ textView: NSTextView) {
            guard !textView.hasMarkedText() else {
                return
            }

            guard parent.onSubmit(textView.string) == .acceptedAndClear else {
                return
            }

            lastPublishedNativeText = nil
            textView.string = ""
            textView.setSelectedRange(NSRange(location: 0, length: 0))
            textView.enclosingScrollView?.needsLayout = true
        }
    }
}

#endif


struct IrisPrimaryCircleButtonVisualStyle: ButtonStyle {
    @Environment(\.irisPalette) private var palette

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(palette.onAccent)
            .background(
                Group {
                    if IrisLayout.usesDesktopChrome {
                        RoundedRectangle(cornerRadius: IrisLayout.buttonCornerRadius, style: .continuous)
                            .fill(palette.accent.opacity(configuration.isPressed ? 0.86 : 1))
                            .frame(width: 52, height: 46)
                    } else {
                        Circle()
                            .fill(palette.accent.opacity(configuration.isPressed ? 0.86 : 1))
                            .frame(width: 46, height: 46)
                    }
                }
            )
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.easeOut(duration: 0.14), value: configuration.isPressed)
            .irisHoverPointer()
    }
}
