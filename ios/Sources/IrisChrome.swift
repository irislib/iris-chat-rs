import SwiftUI
import UniformTypeIdentifiers

enum IrisLayout {
    #if canImport(AppKit)
    static let usesDesktopChrome = true
    static let chromeMaxWidth: CGFloat = 1240
    static let scrollMaxWidth: CGFloat = 1100
    static let chatMaxWidth: CGFloat = 1240
    static let topBarCornerRadius: CGFloat = 18
    static let sectionCornerRadius: CGFloat = 22
    static let inputCornerRadius: CGFloat = 14
    static let buttonCornerRadius: CGFloat = 16
    static let compactButtonCornerRadius: CGFloat = 14
    static let pillCornerRadius: CGFloat = 14
    static let contentHorizontalPadding: CGFloat = 22
    static let contentTopPadding: CGFloat = 10
    static let contentBottomPadding: CGFloat = 32
    #else
    static let usesDesktopChrome = false
    static let chromeMaxWidth: CGFloat? = nil
    static let scrollMaxWidth: CGFloat? = nil
    static let chatMaxWidth: CGFloat? = nil
    static let topBarCornerRadius: CGFloat = 24
    static let sectionCornerRadius: CGFloat = 26
    static let inputCornerRadius: CGFloat = 18
    static let buttonCornerRadius: CGFloat = 999
    static let compactButtonCornerRadius: CGFloat = 999
    static let pillCornerRadius: CGFloat = 999
    static let contentHorizontalPadding: CGFloat = 16
    static let contentTopPadding: CGFloat = 8
    static let contentBottomPadding: CGFloat = 28
    #endif
}

struct IrisPalette {
    let background: Color
    let panel: Color
    let panelAlt: Color
    let border: Color
    let toolbar: Color
    let bubbleMine: Color
    let bubbleTheirs: Color
    let accent: Color
    let accentAlt: Color
    let textPrimary: Color
    let muted: Color
    let onAccent: Color
    let onBubbleMine: Color
    let onBubbleTheirs: Color

    static let light = IrisPalette(
        background: Color(hex: 0xFFFFFF),
        panel: Color(hex: 0xF7F9FA),
        panelAlt: Color(hex: 0xE1E8ED),
        border: Color.black.opacity(0.08),
        toolbar: Color(hex: 0xF7F9FA).opacity(0.96),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0xF7F9FA),
        accent: Color(hex: 0x702ACE),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: Color(hex: 0x0F1419),
        muted: Color(hex: 0x536471),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: Color(hex: 0x0F1419)
    )

    static let dark = IrisPalette(
        background: Color(hex: 0x030918),
        panel: Color(hex: 0x1B1C48),
        panelAlt: Color(hex: 0x121212),
        border: Color.white.opacity(0.16),
        toolbar: Color(hex: 0x030918).opacity(0.92),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0x2A3C5E),
        accent: Color(hex: 0x702ACE),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: .white,
        muted: Color(hex: 0xD1D5DB),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: .white
    )
}

private struct IrisPaletteKey: EnvironmentKey {
    static let defaultValue = IrisPalette.light
}

extension EnvironmentValues {
    var irisPalette: IrisPalette {
        get { self[IrisPaletteKey.self] }
        set { self[IrisPaletteKey.self] = newValue }
    }
}

struct IrisTheme<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    var body: some View {
        let palette = colorScheme == .dark ? IrisPalette.dark : IrisPalette.light
        content()
            .environment(\.irisPalette, palette)
            .tint(palette.accent)
            .preferredColorScheme(nil)
    }
}

private extension Color {
    init(hex: UInt32) {
        let red = Double((hex >> 16) & 0xFF) / 255.0
        let green = Double((hex >> 8) & 0xFF) / 255.0
        let blue = Double(hex & 0xFF) / 255.0
        self.init(.sRGB, red: red, green: green, blue: blue, opacity: 1)
    }
}

struct IrisTopBar: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let canGoBack: Bool
    let onBack: () -> Void
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView

    init(
        title: String,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView())
    ) {
        self.title = title
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
    }

    var body: some View {
        HStack(spacing: 10) {
            Color.clear
                .frame(width: 0, height: 0)
                .accessibilityIdentifier("navigationTopBar")

            if canGoBack {
                Button(action: onBack) {
                    ZStack(alignment: .topTrailing) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 20, weight: .bold))
                            .foregroundStyle(palette.textPrimary)
                            .frame(width: 44, height: 44)
                            .background(
                                RoundedRectangle(cornerRadius: 14, style: .continuous)
                                    .fill(palette.panel.opacity(0.64))
                            )
                        if backBadgeCount > 0 {
                            Text(backBadgeCount > 99 ? "99+" : "\(backBadgeCount)")
                                .font(.system(size: 10, weight: .bold))
                                .foregroundStyle(Color.white)
                                .padding(.horizontal, 5)
                                .frame(minWidth: 18, minHeight: 18)
                                .background(Capsule().fill(palette.accent))
                                .offset(x: 5, y: -5)
                        }
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Back")
                .accessibilityIdentifier("navigationBackButton")
            } else {
                leading
                    .frame(minWidth: 44, alignment: .leading)
            }

            Text(title)
                .font(.system(.title3, design: .rounded, weight: .bold))
                .lineLimit(1)
                .foregroundStyle(palette.textPrimary)
                .frame(maxWidth: .infinity, alignment: .center)

            trailing
                .frame(minWidth: 44, alignment: .trailing)
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 12 : 14)
        .padding(.vertical, IrisLayout.usesDesktopChrome ? 6 : 8)
        .background(
            Rectangle()
                .fill(palette.toolbar)
        )
        .frame(maxWidth: IrisLayout.chromeMaxWidth)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 0)
        .padding(.top, IrisLayout.usesDesktopChrome ? 0 : 4)
        .padding(.bottom, 0)
    }
}

struct IrisSectionCard<Content: View>: View {
    @Environment(\.irisPalette) private var palette

    let accent: Bool
    let content: () -> Content

    init(
        accent: Bool = false,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.accent = accent
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14, content: content)
            .padding(18)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                    .fill(accent ? palette.panelAlt : palette.panel)
                    .overlay(
                        RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                            .stroke(accent ? palette.accent.opacity(0.24) : palette.border, lineWidth: 1)
                    )
            )
            .shadow(
                color: Color.black.opacity(IrisLayout.usesDesktopChrome ? 0.04 : 0),
                radius: IrisLayout.usesDesktopChrome ? 22 : 0,
                y: IrisLayout.usesDesktopChrome ? 12 : 0
            )
    }
}

struct IrisScrollScreen<Content: View>: View {
    let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16, content: content)
                .frame(maxWidth: IrisLayout.scrollMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, IrisLayout.contentHorizontalPadding)
                .padding(.top, IrisLayout.contentTopPadding)
                .padding(.bottom, IrisLayout.contentBottomPadding)
        }
        .scrollIndicators(.hidden)
    }
}

struct IrisAdaptiveColumns<Leading: View, Trailing: View>: View {
    let alignment: VerticalAlignment
    let spacing: CGFloat
    let leading: () -> Leading
    let trailing: () -> Trailing

    init(
        alignment: VerticalAlignment = .top,
        spacing: CGFloat = 16,
        @ViewBuilder leading: @escaping () -> Leading,
        @ViewBuilder trailing: @escaping () -> Trailing
    ) {
        self.alignment = alignment
        self.spacing = spacing
        self.leading = leading
        self.trailing = trailing
    }

    var body: some View {
        Group {
            if IrisLayout.usesDesktopChrome {
                HStack(alignment: alignment, spacing: spacing) {
                    leading()
                        .frame(maxWidth: .infinity, alignment: .leading)
                    trailing()
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else {
                VStack(alignment: .leading, spacing: spacing) {
                    leading()
                    trailing()
                }
            }
        }
    }
}

struct IrisAvatar: View {
    @Environment(\.irisPalette) private var palette

    let label: String
    let size: CGFloat
    let emphasize: Bool
    let imageURL: String?

    init(label: String, size: CGFloat = 42, emphasize: Bool = false, imageURL: String? = nil) {
        self.label = label
        self.size = size
        self.emphasize = emphasize
        self.imageURL = imageURL
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(emphasize ? palette.accent : palette.panelAlt)
                .overlay(Circle().stroke(palette.border, lineWidth: 1))

            if let imageURL, let url = URL(string: imageURL) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFill()
                    default:
                        avatarInitial
                    }
                }
                .clipShape(Circle())
            } else {
                avatarInitial
            }
        }
        .frame(width: size, height: size)
    }

    private var avatarInitial: some View {
        Text(String((label.trimmingCharacters(in: .whitespacesAndNewlines).first ?? "?")).uppercased())
            .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
            .foregroundStyle(emphasize ? palette.onAccent : palette.textPrimary)
    }
}

struct IrisPrimaryButtonStyle: ButtonStyle {
    @Environment(\.irisPalette) private var palette
    let compact: Bool

    init(compact: Bool = false) {
        self.compact = compact
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(compact ? .subheadline : .body, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.onAccent)
            .padding(.horizontal, compact ? 14 : 18)
            .padding(.vertical, compact ? 10 : 14)
            .frame(maxWidth: compact ? nil : .infinity)
            .background(
                Group {
                    if IrisLayout.usesDesktopChrome {
                        RoundedRectangle(
                            cornerRadius: compact ? IrisLayout.compactButtonCornerRadius : IrisLayout.buttonCornerRadius,
                            style: .continuous
                        )
                        .fill(palette.accent.opacity(configuration.isPressed ? 0.86 : 1))
                    } else {
                        Capsule(style: .continuous)
                            .fill(palette.accent.opacity(configuration.isPressed ? 0.86 : 1))
                    }
                }
            )
            .scaleEffect(configuration.isPressed ? 0.985 : 1)
            .animation(.easeOut(duration: 0.14), value: configuration.isPressed)
    }
}

struct IrisSecondaryButtonStyle: ButtonStyle {
    @Environment(\.irisPalette) private var palette
    let compact: Bool

    init(compact: Bool = false) {
        self.compact = compact
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(compact ? .subheadline : .body, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.textPrimary)
            .padding(.horizontal, compact ? 14 : 18)
            .padding(.vertical, compact ? 10 : 14)
            .frame(maxWidth: compact ? nil : .infinity)
            .background(
                Group {
                    if IrisLayout.usesDesktopChrome {
                        RoundedRectangle(
                            cornerRadius: compact ? IrisLayout.compactButtonCornerRadius : IrisLayout.buttonCornerRadius,
                            style: .continuous
                        )
                        .fill(palette.panel.opacity(0.72))
                    } else {
                        Capsule(style: .continuous)
                            .fill(palette.panel)
                    }
                }
            )
            .opacity(configuration.isPressed ? 0.9 : 1)
    }
}

struct IrisInputFieldModifier: ViewModifier {
    @Environment(\.irisPalette) private var palette

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(.system(.body, design: .rounded))
            .padding(.horizontal, 14)
            .padding(.vertical, 13)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.inputCornerRadius, style: .continuous)
                    .fill(palette.panel.opacity(IrisLayout.usesDesktopChrome ? 0.62 : 1))
            )
    }
}

extension View {
    func irisInputField() -> some View {
        modifier(IrisInputFieldModifier())
    }
}

struct IrisInfoPill: View {
    @Environment(\.irisPalette) private var palette

    let text: String
    let tint: Color?

    init(_ text: String, tint: Color? = nil) {
        self.text = text
        self.tint = tint
    }

    var body: some View {
        Text(text)
            .font(.system(.caption, design: .rounded, weight: .semibold))
            .foregroundStyle(tint ?? palette.muted)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                    .fill((tint ?? palette.panelAlt).opacity(0.14))
            )
    }
}

struct IrisChatRow: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let preview: String
    let subtitle: String?
    let timeLabel: String?
    let unreadCount: UInt64
    let onTap: () -> Void

    var body: some View {
        Button(action: onTap) {
            HStack(alignment: .top, spacing: 14) {
                IrisAvatar(label: title, emphasize: unreadCount > 0)

                VStack(alignment: .leading, spacing: 5) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(title)
                            .font(.system(.headline, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)

                        Spacer(minLength: 8)

                        if let timeLabel, !timeLabel.isEmpty {
                            Text(timeLabel)
                                .font(.system(.caption, design: .rounded, weight: .medium))
                                .foregroundStyle(palette.muted)
                                .lineLimit(1)
                        }
                    }

                    Text(preview)
                        .font(.system(.subheadline, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .lineLimit(2)

                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.system(.caption, design: .rounded, weight: .medium))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }

                if unreadCount > 0 {
                    Text("\(unreadCount)")
                        .font(.system(.caption, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.onAccent)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(Capsule(style: .continuous).fill(palette.accent))
                }
            }
            .padding(.vertical, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct IrisDayChip: View {
    @Environment(\.irisPalette) private var palette
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.caption, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.muted)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                    .fill(palette.panel.opacity(0.64))
            )
    }
}

struct IrisComposerBar: View {
    @Environment(\.irisPalette) private var palette

    @Binding var draft: String
    @Binding var attachments: [StagedAttachment]
    @State private var showingAttachmentPicker = false
    @State private var showingEmojiPicker = false
    @State private var isDropTargeted = false

    let placeholder: String
    let isSending: Bool
    let isUploading: Bool
    @FocusState.Binding var isFocused: Bool
    let onDraftChange: () -> Void
    let onAttach: ([URL]) -> Void
    let onSend: () -> Void

    private var canSend: Bool {
        (
            !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
            !attachments.isEmpty
        ) && !isSending && !isUploading
    }

    var body: some View {
        VStack(spacing: 8) {
            Color.clear
                .frame(width: 0, height: 0)
                .accessibilityIdentifier("chatComposerBar")

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
                    ProgressView()
                        .progressViewStyle(.linear)
                        .tint(palette.accent)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack(alignment: .bottom, spacing: 12) {
                Button {
                    showingAttachmentPicker = true
                } label: {
                    Image(systemName: isUploading ? "ellipsis.circle.fill" : "paperclip")
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle((isSending || isUploading) ? palette.muted.opacity(0.54) : palette.textPrimary)
                        .frame(width: 42, height: 46)
                }
                .buttonStyle(.plain)
                .disabled(isSending || isUploading)
                .accessibilityIdentifier("chatAttachButton")

                if IrisLayout.usesDesktopChrome {
                    Button {
                        showingEmojiPicker.toggle()
                    } label: {
                        Image(systemName: "face.smiling.fill")
                            .font(.system(size: 18, weight: .semibold))
                            .foregroundStyle(isSending || isUploading ? palette.muted.opacity(0.54) : palette.textPrimary)
                            .frame(width: 42, height: 46)
                    }
                    .buttonStyle(.plain)
                    .disabled(isSending || isUploading)
                    .popover(isPresented: $showingEmojiPicker, arrowEdge: .bottom) {
                        IrisEmojiPicker { emoji in
                            draft.append(emoji)
                            showingEmojiPicker = false
                        }
                    }
                    .accessibilityIdentifier("chatEmojiButton")
                }

                TextField(placeholder, text: $draft)
                    .irisDraftInputModifiers()
                    .irisInputField()
                    .irisDesktopSubmit(submitDraft)
                    .focused($isFocused)
                    .irisOnChange(of: draft) { _ in onDraftChange() }
                    .accessibilityIdentifier("chatMessageInput")

                Button(action: submitDraft) {
                    Image(systemName: isSending ? "ellipsis.circle.fill" : "paperplane.fill")
                        .font(.system(size: 18, weight: .bold))
                        .frame(width: IrisLayout.usesDesktopChrome ? 52 : 46, height: 46)
                }
                .buttonStyle(IrisPrimaryCircleButtonStyle())
                .disabled(!canSend)
                .accessibilityIdentifier("chatSendButton")
            }
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 16 : IrisLayout.contentHorizontalPadding)
        .padding(.top, 10)
        .padding(.bottom, 12)
        .background(
            Rectangle()
                .fill(palette.toolbar)
        )
        .overlay {
            if isDropTargeted {
                RoundedRectangle(cornerRadius: IrisLayout.inputCornerRadius + 8, style: .continuous)
                    .stroke(palette.accent.opacity(0.78), lineWidth: 2)
                    .padding(.horizontal, IrisLayout.usesDesktopChrome ? 8 : 10)
                    .padding(.vertical, 6)
            }
        }
        .frame(maxWidth: .infinity)
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
    }

    private func submitDraft() {
        guard canSend else {
            return
        }
        onSend()
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

private func droppedFileURL(from item: NSSecureCoding?) -> URL? {
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

private struct IrisEmojiPicker: View {
    @Environment(\.irisPalette) private var palette
    let onSelect: (String) -> Void

    private let columns = Array(repeating: GridItem(.fixed(34), spacing: 6), count: 8)

    var body: some View {
        LazyVGrid(columns: columns, spacing: 6) {
            ForEach(IrisComposerEmojiChoices, id: \.self) { emoji in
                Button {
                    onSelect(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 21))
                        .frame(width: 34, height: 34)
                }
                .buttonStyle(.plain)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(palette.panel.opacity(0.001))
                )
            }
        }
        .padding(10)
        .background(palette.background)
    }
}

private let IrisComposerEmojiChoices = [
    "😀", "😂", "😊", "😍", "🥰", "😎", "🤔", "😭",
    "❤️", "🔥", "✨", "🙏", "👍", "👀", "🎉", "💜",
    "🌞", "🌙", "⭐️", "🍓", "☕️", "🌊", "🚀", "✅"
]

private enum IrisAttachmentCategory: String {
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

private let irisImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
private let irisVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
private let irisAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
private let irisArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
private let irisDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

private func irisAttachmentCategory(from filename: String) -> IrisAttachmentCategory {
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

private struct IrisSelectedAttachmentChip: View {
    @Environment(\.irisPalette) private var palette
    let attachment: StagedAttachment
    let enabled: Bool
    let onRemove: () -> Void

    var body: some View {
        let category = irisAttachmentCategory(from: attachment.filename)

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
            .buttonStyle(.plain)
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
}

private struct IrisPrimaryCircleButtonStyle: ButtonStyle {
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
    }
}

func irisRelativeTime(_ secs: UInt64?, relativeTo now: Date = Date()) -> String? {
    guard let secs else { return nil }
    let date = Date(timeIntervalSince1970: TimeInterval(secs))
    let elapsedSeconds = abs(date.timeIntervalSince(now))
    if elapsedSeconds < 60 {
        return "now"
    }
    if elapsedSeconds < 60 * 60 {
        return "\(Int(elapsedSeconds / 60))m"
    }
    if elapsedSeconds < 24 * 60 * 60 {
        return "\(Int(elapsedSeconds / (60 * 60)))h"
    }
    return "\(Int(elapsedSeconds / (24 * 60 * 60)))d"
}

func irisTimelineDay(_ secs: UInt64) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(secs))
    let calendar = Calendar.current
    if calendar.isDateInToday(date) {
        return "Today"
    }
    if calendar.isDateInYesterday(date) {
        return "Yesterday"
    }
    return irisDayFormatter.string(from: date)
}

func irisMessageClock(_ secs: UInt64) -> String {
    irisTimeFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

func irisSameTimelineDay(_ lhs: UInt64, _ rhs: UInt64) -> Bool {
    Calendar.current.isDate(
        Date(timeIntervalSince1970: TimeInterval(lhs)),
        inSameDayAs: Date(timeIntervalSince1970: TimeInterval(rhs))
    )
}

func irisDeliveryLabel(_ delivery: DeliveryState) -> String {
    switch delivery {
    case .queued:
        return "Queued"
    case .pending:
        return "Pending"
    case .sent:
        return "Sent"
    case .received:
        return "Received"
    case .seen:
        return "Seen"
    case .failed:
        return "Failed"
    }
}

private let irisDayFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "EEE, MMM d"
    return formatter
}()

private let irisTimeFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .none
    formatter.timeStyle = .short
    return formatter
}()
