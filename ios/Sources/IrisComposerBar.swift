import Foundation
import ImageIO
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

    @Binding var draft: String
    @Binding var attachments: [StagedAttachment]
    @State private var showingAttachmentPicker = false
    @State private var showingEmojiPicker = false
    @State private var isDropTargeted = false
    #if canImport(PhotosUI)
    @State private var showingPhotoPicker = false
    @State private var pickedPhotos: [PhotosPickerItem] = []
    #endif

    let placeholder: String
    let isSending: Bool
    let isUploading: Bool
    let uploadFraction: Double?
    @FocusState.Binding var isFocused: Bool
    let onDraftChange: () -> Void
    let onAttach: ([URL]) -> Void
    let onSend: (String) -> Void

    private var canSend: Bool {
        (
            !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
            !attachments.isEmpty
        ) && !isSending && !isUploading
    }

    var body: some View {
        VStack(spacing: 8) {
            if !attachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(attachments) { attachment in
                            IrisSelectedAttachmentChip(
                                attachment: attachment,
                                enabled: !isSending && !isUploading
                            ) {
                                attachments.removeAll { $0 == attachment }
                            }
                        }
                    }
                    .padding(.horizontal, 1)
                }
                .accessibilityIdentifier("chatSelectedAttachments")
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

            HStack(alignment: .bottom, spacing: 8) {
                attachmentControl

                if IrisLayout.usesDesktopChrome {
                    Button {
                        showingEmojiPicker.toggle()
                    } label: {
                        Image(systemName: "face.smiling.fill")
                            .font(.system(size: 18, weight: .semibold))
                            .foregroundStyle(isSending || isUploading ? palette.muted.opacity(0.54) : palette.textPrimary)
                            .frame(width: 40, height: 40)
                            .irisGlassSurface(in: Circle())
                    }
                    .buttonStyle(.irisPlain)
                    .disabled(isSending || isUploading)
                    .popover(isPresented: $showingEmojiPicker, arrowEdge: .bottom) {
                        IrisEmojiPicker { emoji in
                            insertEmoji(emoji)
                            showingEmojiPicker = false
                        }
                    }
                    .accessibilityIdentifier("chatEmojiButton")
                }

                composerInput

                // Mobile keeps the Signal-style explicit send affordance.
                // Desktop sends with Return, so showing this button only
                // after typing causes a distracting composer width shift.
                if !IrisLayout.usesDesktopChrome && (canSend || isSending) {
                    Button(action: submitDraft) {
                        IrisSendButtonLabel(isSending: isSending)
                            .frame(width: 40, height: 40)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.irisPlain)
                    .disabled(!canSend)
                    .accessibilityIdentifier("chatSendButton")
                    .transition(
                        .asymmetric(
                            insertion: .scale(scale: 0.4, anchor: .center)
                                .combined(with: .opacity)
                                .combined(with: .move(edge: .trailing)),
                            removal: .scale(scale: 0.4, anchor: .center)
                                .combined(with: .opacity)
                        )
                    )
                }
            }
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
        .overlay {
            if isDropTargeted {
                RoundedRectangle(cornerRadius: IrisLayout.inputCornerRadius + 8, style: .continuous)
                    .stroke(palette.accent.opacity(0.78), lineWidth: 2)
                    .padding(.horizontal, IrisLayout.usesDesktopChrome ? 8 : 10)
                    .padding(.vertical, 6)
            }
        }
        .frame(maxWidth: .infinity)
        .irisOnChange(of: draft) { _ in
            onDraftChange()
        }
        .onDrop(of: [UTType.fileURL.identifier], isTargeted: $isDropTargeted) { providers in
            handleDroppedFiles(providers)
        }
        .fileImporter(
            isPresented: $showingAttachmentPicker,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            guard case .success(let urls) = result, !urls.isEmpty else {
                return
            }
            onAttach(urls)
        }
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
                text: $draft,
                isFocused: $isFocused
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
                    .padding(.top, 1)
                    .allowsHitTesting(false)
            }
            IrisAppKitComposerTextView(
                text: $draft,
                isFocused: $isFocused,
                onSubmit: submitDraft
            )
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .irisInputField()
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
        Menu {
            Button("Photo Library") { showingPhotoPicker = true }
            Button("Files") { showingAttachmentPicker = true }
        } label: {
            attachmentControlLabel
        }
        .disabled(isSending || isUploading)
        .accessibilityIdentifier("chatAttachButton")
        #else
        Button {
            showingAttachmentPicker = true
        } label: {
            attachmentControlLabel
        }
        .buttonStyle(.irisPlain)
        .disabled(isSending || isUploading)
        .accessibilityIdentifier("chatAttachButton")
        #endif
    }

    private var attachmentControlLabel: some View {
        Image(systemName: isUploading ? "ellipsis" : "plus")
            .font(.system(size: 19, weight: .semibold))
            .foregroundStyle((isSending || isUploading) ? palette.muted.opacity(0.54) : palette.textPrimary)
            .frame(width: 40, height: 40)
            .contentShape(Circle())
            .irisGlassSurface(in: Circle())
            .accessibilityLabel("Add")
    }

    #if canImport(PhotosUI)
    private func handlePickedPhotos(_ items: [PhotosPickerItem]) {
        guard !items.isEmpty else { return }
        let snapshot = items
        pickedPhotos = []
        Task {
            var urls: [URL] = []
            for item in snapshot {
                guard let url = await loadPickedPhoto(item) else { continue }
                urls.append(url)
            }
            if !urls.isEmpty {
                let captured = urls
                await MainActor.run {
                    onAttach(captured)
                }
            }
        }
    }

    private func loadPickedPhoto(_ item: PhotosPickerItem) async -> URL? {
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
        guard canSend else {
            return
        }
        #if os(iOS)
        let text = IrisUIKitComposerTextView.currentText ?? draft
        #elseif canImport(AppKit)
        let text = IrisAppKitComposerTextView.currentText ?? draft
        #else
        let text = draft
        #endif
        onSend(text)
    }

    private func insertEmoji(_ emoji: String) {
        #if canImport(AppKit)
        if let text = IrisAppKitComposerTextView.insertTextAtSelection(emoji) {
            draft = text
            return
        }
        #endif
        draft.append(emoji)
    }

    private func handleDroppedFiles(_ providers: [NSItemProvider]) -> Bool {
        let fileProviders = providers.filter {
            $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
        }
        guard !fileProviders.isEmpty else {
            return false
        }

        let group = DispatchGroup()
        let lock = NSLock()
        var urls: [URL] = []

        for provider in fileProviders {
            group.enter()
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                if let url = droppedFileURL(from: item) {
                    lock.lock()
                    urls.append(url)
                    lock.unlock()
                }
                group.leave()
            }
        }

        group.notify(queue: .main) {
            guard !urls.isEmpty else {
                return
            }
            onAttach(urls)
        }

        return true
    }
}

#if os(iOS)
struct IrisUIKitComposerTextView: UIViewRepresentable {
    private static weak var activeTextView: UITextView?

    static var currentText: String? {
        activeTextView?.text
    }

    @Binding var text: String
    @FocusState.Binding var isFocused: Bool

    func makeUIView(context: Context) -> UITextView {
        let textView = UITextView()
        Self.activeTextView = textView
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.font = UIFont.preferredFont(forTextStyle: .body)
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = UIColor.label
        textView.tintColor = UIColor.tintColor
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        textView.isScrollEnabled = false
        textView.returnKeyType = .default
        textView.keyboardDismissMode = .interactive
        textView.autocapitalizationType = .sentences
        textView.autocorrectionType = .yes
        textView.spellCheckingType = .yes
        textView.accessibilityIdentifier = "chatMessageInput"
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return textView
    }

    func updateUIView(_ uiView: UITextView, context: Context) {
        Self.activeTextView = uiView
        context.coordinator.parent = self
        if uiView.markedTextRange == nil, uiView.text != text {
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
        let shouldScroll = measuredHeight(for: uiView, width: uiView.bounds.width) >= maxHeight(for: uiView)
        if uiView.isScrollEnabled != shouldScroll {
            uiView.isScrollEnabled = shouldScroll
        }
        if isFocused && !uiView.isFirstResponder {
            DispatchQueue.main.async {
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
        textView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
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
    @FocusState.Binding var isFocused: Bool
    let onSubmit: () -> Void

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.verticalScrollElasticity = .none
        scrollView.setAccessibilityIdentifier("chatMessageInput")

        let textView = IrisComposerNSTextView()
        Self.activeTextView = textView
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator
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
        textView.isVerticallyResizable = true
        textView.minSize = NSSize(width: 0, height: Self.lineHeight(for: textView))
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.autoresizingMask = [.width]
        textView.isContinuousSpellCheckingEnabled = true
        textView.isAutomaticSpellingCorrectionEnabled = true
        textView.setAccessibilityIdentifier("chatMessageInput")

        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        guard let textView = nsView.documentView as? IrisComposerNSTextView else {
            return
        }

        Self.activeTextView = textView
        context.coordinator.parent = self
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator

        if textView.string != text, !textView.hasMarkedText() {
            let selectedRange = textView.selectedRange()
            textView.string = text
            Self.restoreSelection(selectedRange, in: textView)
        }

        Self.updateScrollState(in: nsView, textView: textView)

        if isFocused, textView.window?.firstResponder !== textView {
            DispatchQueue.main.async { [weak textView] in
                guard let textView else { return }
                textView.window?.makeFirstResponder(textView)
            }
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        guard let textView = nsView.documentView as? NSTextView else {
            return nil
        }
        let width = max(proposal.width ?? nsView.bounds.width, 1)
        let height = min(
            max(Self.measuredHeight(for: textView, width: width), Self.lineHeight(for: textView)),
            Self.maxHeight(for: textView)
        )
        return CGSize(width: width, height: height)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    private static func updateScrollState(in scrollView: NSScrollView, textView: NSTextView) {
        let width = max(scrollView.bounds.width, 1)
        let contentHeight = measuredHeight(for: textView, width: width)
        scrollView.hasVerticalScroller = contentHeight > maxHeight(for: textView)
    }

    private static func restoreSelection(_ selectedRange: NSRange, in textView: NSTextView) {
        let textLength = (textView.string as NSString).length
        let location = min(selectedRange.location, textLength)
        let length = min(selectedRange.length, textLength - location)
        textView.setSelectedRange(NSRange(location: location, length: length))
    }

    private static func measuredHeight(for textView: NSTextView, width: CGFloat) -> CGFloat {
        guard let layoutManager = textView.layoutManager,
              let textContainer = textView.textContainer else {
            return lineHeight(for: textView)
        }
        textContainer.containerSize = NSSize(width: max(width, 1), height: CGFloat.greatestFiniteMagnitude)
        layoutManager.ensureLayout(for: textContainer)
        return ceil(max(layoutManager.usedRect(for: textContainer).height, lineHeight(for: textView)))
    }

    private static func lineHeight(for textView: NSTextView) -> CGFloat {
        let font = textView.font ?? NSFont.systemFont(ofSize: NSFont.systemFontSize)
        if let layoutManager = textView.layoutManager {
            return ceil(layoutManager.defaultLineHeight(for: font))
        }
        return ceil(font.ascender - font.descender + font.leading)
    }

    private static func maxHeight(for textView: NSTextView) -> CGFloat {
        lineHeight(for: textView) * 5
    }

    final class Coordinator: NSObject, NSTextViewDelegate, IrisComposerNSTextViewCommandDelegate {
        var parent: IrisAppKitComposerTextView

        init(parent: IrisAppKitComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else {
                return
            }
            guard parent.text != textView.string else {
                return
            }
            parent.text = textView.string
        }

        func textDidBeginEditing(_ notification: Notification) {
            parent.isFocused = true
        }

        func textDidEndEditing(_ notification: Notification) {
            parent.isFocused = false
        }

        func composerTextViewDidSubmit(_ textView: NSTextView) {
            if parent.text != textView.string {
                parent.text = textView.string
            }
            parent.onSubmit()
        }
    }
}

private protocol IrisComposerNSTextViewCommandDelegate: AnyObject {
    func composerTextViewDidSubmit(_ textView: NSTextView)
}

final class IrisComposerNSTextView: NSTextView {
    weak var composerCommandDelegate: IrisComposerNSTextViewCommandDelegate?

    override func doCommand(by selector: Selector) {
        if selector == #selector(NSResponder.insertNewline(_:)), !shouldInsertLineBreakForCurrentEvent {
            composerCommandDelegate?.composerTextViewDidSubmit(self)
            return
        }
        super.doCommand(by: selector)
    }

    private var shouldInsertLineBreakForCurrentEvent: Bool {
        guard let event = NSApp.currentEvent, event.type == .keyDown else {
            return false
        }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        return flags.contains(.shift) || flags.contains(.option)
    }
}
#endif

func droppedFileURL(from item: NSSecureCoding?) -> URL? {
    if let url = item as? URL {
        return url
    }
    if let url = item as? NSURL {
        return url as URL
    }
    if let data = item as? Data {
        if let url = URL(dataRepresentation: data, relativeTo: nil) {
            return url
        }
        if let string = String(data: data, encoding: .utf8) {
            return URL(string: string.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }
    if let string = item as? String {
        return URL(string: string.trimmingCharacters(in: .whitespacesAndNewlines))
    }
    return nil
}

enum IrisAttachmentCategory: String {
    case image = "Image"
    case video = "Video"
    case audio = "Audio"
    case archive = "Archive"
    case document = "Document"
    case file = "File"

    var systemIcon: String {
        switch self {
        case .image:
            return "photo.fill"
        case .video:
            return "play.rectangle.fill"
        case .audio:
            return "waveform"
        case .archive:
            return "archivebox.fill"
        case .document:
            return "doc.text.fill"
        case .file:
            return "doc.fill"
        }
    }
}

let irisImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
let irisVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
let irisAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
let irisArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
let irisDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

func irisAttachmentCategory(from filename: String) -> IrisAttachmentCategory {
    let ext = filename
        .split(separator: ".")
        .last
        .map { String($0).lowercased() }

    guard let extensionValue = ext, !extensionValue.isEmpty else {
        return .file
    }

    if irisImageExtensions.contains(extensionValue) {
        return .image
    }
    if irisVideoExtensions.contains(extensionValue) {
        return .video
    }
    if irisAudioExtensions.contains(extensionValue) {
        return .audio
    }
    if irisArchiveExtensions.contains(extensionValue) {
        return .archive
    }
    if irisDocumentExtensions.contains(extensionValue) {
        return .document
    }
    return .file
}

struct IrisSelectedAttachmentChip: View {
    @Environment(\.irisPalette) private var palette
    let attachment: StagedAttachment
    let enabled: Bool
    let onRemove: () -> Void

    @State private var thumbnail: PlatformImage?

    private static let thumbSize: CGFloat = 56

    var body: some View {
        let category = irisAttachmentCategory(from: attachment.filename)

        if category == .image {
            imageChip
                .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
                .task(id: attachment.path) {
                    await loadThumbnailIfNeeded()
                }
        } else {
            fileChip(category: category)
        }
    }

    private var imageChip: some View {
        ZStack(alignment: .topTrailing) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(palette.panel)
                if let thumbnail {
                    Image(platformImage: thumbnail)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: "photo.fill")
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
            }
            .frame(width: Self.thumbSize, height: Self.thumbSize)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            Button(action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.palette)
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(palette.textPrimary, palette.panel)
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .disabled(!enabled)
            .accessibilityIdentifier("chatSelectedAttachmentRemove")
        }
    }

    private func fileChip(category: IrisAttachmentCategory) -> some View {
        HStack(spacing: 7) {
            Image(systemName: category.systemIcon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(palette.muted)
            VStack(alignment: .leading, spacing: 2) {
                Text(attachment.filename)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(category.rawValue)
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
            }
            .frame(maxWidth: 220, alignment: .leading)
            Button(action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(enabled ? palette.muted : palette.muted.opacity(0.45))
            }
            .buttonStyle(.irisPlain)
            .disabled(!enabled)
            .accessibilityIdentifier("chatSelectedAttachmentRemove")
        }
        .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
        .padding(.leading, 11)
        .padding(.trailing, 7)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(palette.panel)
        )
    }

    @MainActor
    private func loadThumbnailIfNeeded() async {
        if thumbnail != nil { return }
        if let cached = IrisStagedThumbnailCache.image(for: attachment.path) {
            thumbnail = cached
            return
        }
        let path = attachment.path
        let image: PlatformImage? = await Task.detached(priority: .userInitiated) {
            irisLoadStagedThumbnail(path: path)
        }.value
        guard let image else { return }
        IrisStagedThumbnailCache.store(image, for: attachment.path)
        thumbnail = image
    }
}

enum IrisStagedThumbnailCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 32
        cache.totalCostLimit = 16 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: irisAvatarImageCost(image))
    }
}

func irisLoadStagedThumbnail(path: String) -> PlatformImage? {
    let url = URL(fileURLWithPath: path) as CFURL
    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithURL(url, sourceOptions as CFDictionary) else {
        return nil
    }
    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: 256
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(
        source,
        0,
        thumbnailOptions as CFDictionary
    ) else {
        return nil
    }
    #if os(iOS)
    return PlatformImage(cgImage: cgImage)
    #elseif os(macOS)
    return PlatformImage(
        cgImage: cgImage,
        size: NSSize(width: cgImage.width, height: cgImage.height)
    )
    #else
    return nil
    #endif
}

struct IrisPrimaryCircleButtonStyle: ButtonStyle {
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
