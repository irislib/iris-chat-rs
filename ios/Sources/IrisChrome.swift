import Foundation
import ImageIO
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

/// Shows a pointing-hand cursor while hovered. No-op outside macOS.
private struct IrisHoverPointerModifier: ViewModifier {
    func body(content: Content) -> some View {
#if canImport(AppKit)
        content.onHover { hovering in
            if hovering {
                NSCursor.pointingHand.push()
            } else {
                NSCursor.pop()
            }
        }
#else
        content
#endif
    }
}

extension View {
    /// Shows a pointing-hand cursor while hovered. Inert on iOS.
    func irisHoverPointer() -> some View {
        modifier(IrisHoverPointerModifier())
    }
}

/// `.plain` look-alike that adds a pointing-hand cursor on macOS hover.
/// Use everywhere instead of the system `.plain` style.
struct IrisPlainButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1)
            .irisHoverPointer()
    }
}

extension ButtonStyle where Self == IrisPlainButtonStyle {
    static var irisPlain: IrisPlainButtonStyle { IrisPlainButtonStyle() }
}

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
        background: Color(hex: 0x101010),
        panel: Color(hex: 0x242424),
        panelAlt: Color(hex: 0x343434),
        border: Color.white.opacity(0.12),
        toolbar: Color(hex: 0x181818).opacity(0.96),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0x3A3A3A),
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
    let subtitle: String?
    let subtitleSystemImage: String?
    let canGoBack: Bool
    let onBack: () -> Void
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView
    let titleAccessoryLeading: AnyView
    let onTitleTap: (() -> Void)?

    init(
        title: String,
        subtitle: String? = nil,
        subtitleSystemImage: String? = nil,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView()),
        titleAccessoryLeading: AnyView = AnyView(EmptyView()),
        onTitleTap: (() -> Void)? = nil
    ) {
        self.title = title
        self.subtitle = subtitle
        self.subtitleSystemImage = subtitleSystemImage
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
        self.titleAccessoryLeading = titleAccessoryLeading
        self.onTitleTap = onTitleTap
    }

    @ViewBuilder
    private var titleContent: some View {
        HStack(spacing: 8) {
            titleAccessoryLeading
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .lineLimit(1)
                    .foregroundStyle(palette.textPrimary)

                if let subtitle, !subtitle.isEmpty {
                    HStack(spacing: 4) {
                        if let subtitleSystemImage {
                            Image(systemName: subtitleSystemImage)
                                .font(.system(size: 10, weight: .semibold))
                        }

                        Text(subtitle)
                            .font(.system(.caption2, design: .rounded, weight: .semibold))
                    }
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
                }
            }
        }
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
                .buttonStyle(.irisPlain)
                .accessibilityLabel("Back")
                .accessibilityIdentifier("navigationBackButton")
            } else {
                leading
                    .frame(minWidth: 44, alignment: .leading)
            }

            Group {
                if let onTitleTap {
                    Button(action: onTitleTap) {
                        titleContent
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("chatHeaderTitleButton")
                } else {
                    titleContent
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

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

func irisHtreeNhash(from rawURL: String?) -> String? {
    guard let rawURL else { return nil }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    let prefix: String
    if trimmed.hasPrefix("htree://") {
        prefix = "htree://"
    } else if trimmed.hasPrefix("nhash://") {
        prefix = "nhash://"
    } else {
        return nil
    }
    let remainder = trimmed.dropFirst(prefix.count)
    return remainder.split(separator: "/", maxSplits: 1).first.map(String.init)
}

func irisHttpAvatarURL(
    _ rawURL: String?,
    preferences: PreferencesSnapshot,
    pixelSize: CGFloat
) -> String? {
    guard let rawURL else { return nil }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") else {
        return nil
    }
    let dim = UInt32(max(1, pixelSize.rounded()))
    return proxiedImageUrl(
        originalSrc: trimmed,
        preferences: preferences,
        width: dim,
        height: dim,
        square: true
    )
}

private enum IrisAvatarImageSource: Equatable {
    case hashtree(String)
    case http(String)

    var cacheKey: String {
        switch self {
        case .hashtree(let nhash): return "htree:\(nhash)"
        case .http(let url): return "http:\(url)"
        }
    }
}

private enum IrisAvatarImageCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 160
        cache.totalCostLimit = 24 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: irisAvatarImageCost(image))
    }
}

private func makeIrisAvatarImage(data: Data, maxPixelSize: Int) -> PlatformImage? {
    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions as CFDictionary) else {
        return nil
    }

    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: max(1, maxPixelSize)
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) else {
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

private func irisAvatarImageCost(_ image: PlatformImage) -> Int {
    #if os(iOS)
    let width = max(1, Int(image.size.width * image.scale))
    let height = max(1, Int(image.size.height * image.scale))
    return width * height * 4
    #elseif os(macOS)
    let width = max(1, Int(image.size.width))
    let height = max(1, Int(image.size.height))
    return width * height * 4
    #else
    return 1
    #endif
}

private func irisAvatarImageSource(
    pictureUrl: String?,
    preferences: PreferencesSnapshot?,
    pixelSize: CGFloat
) -> IrisAvatarImageSource? {
    if let nhash = irisHtreeNhash(from: pictureUrl) {
        return .hashtree(nhash)
    }
    guard let preferences,
          let httpURL = irisHttpAvatarURL(pictureUrl, preferences: preferences, pixelSize: pixelSize) else {
        return nil
    }
    return .http(httpURL)
}

struct IrisAvatar: View {
    @Environment(\.irisPalette) private var palette

    let label: String
    let size: CGFloat
    let emphasize: Bool
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let loadedImageIdentifier: String?

    @State private var avatarImage: PlatformImage?

    init(
        label: String,
        size: CGFloat = 42,
        emphasize: Bool = false,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        loadedImageIdentifier: String? = nil
    ) {
        self.label = label
        self.size = size
        self.emphasize = emphasize
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.loadedImageIdentifier = loadedImageIdentifier
        let source = irisAvatarImageSource(
            pictureUrl: pictureUrl,
            preferences: preferences,
            pixelSize: size * 2
        )
        _avatarImage = State(initialValue: source.flatMap { IrisAvatarImageCache.image(for: $0.cacheKey) })
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(emphasize ? palette.accent : palette.panelAlt)
                .overlay(Circle().stroke(palette.border, lineWidth: 1))

            if let avatarImage {
                Image(platformImage: avatarImage)
                    .resizable()
                    .scaledToFill()
                    .clipShape(Circle())
                if let loadedImageIdentifier {
                    Color.clear
                        .frame(width: 1, height: 1)
                        .accessibilityIdentifier(loadedImageIdentifier)
                        .allowsHitTesting(false)
                }
            } else {
                avatarInitial
            }
        }
        .frame(width: size, height: size)
        .task(id: imageSourceKey) {
            await loadAvatarImage()
        }
    }

    private var imageSource: IrisAvatarImageSource? {
        irisAvatarImageSource(
            pictureUrl: pictureUrl,
            preferences: preferences,
            pixelSize: size * 2
        )
    }

    private var imageSourceKey: String? {
        imageSource?.cacheKey
    }

    private func loadAvatarImage() async {
        guard let source = imageSource else {
            avatarImage = nil
            return
        }
        let key = source.cacheKey
        if let cached = IrisAvatarImageCache.image(for: key) {
            avatarImage = cached
            return
        }

        let loaded: Data?
        switch source {
        case .hashtree(let nhash):
            guard let manager else {
                avatarImage = nil
                return
            }
            loaded = await manager.resolveHashtreePictureBytes(nhash: nhash)
        case .http(let urlString):
            guard let url = URL(string: urlString) else {
                avatarImage = nil
                return
            }
            if let response = try? await URLSession.shared.data(from: url) {
                loaded = response.0
            } else {
                loaded = nil
            }
        }

        guard imageSourceKey == key else { return }
        guard let loaded, !loaded.isEmpty else {
            avatarImage = nil
            return
        }
        guard let image = makeIrisAvatarImage(data: loaded, maxPixelSize: Int(ceil(size * 3))) else {
            avatarImage = nil
            return
        }
        IrisAvatarImageCache.store(image, for: key)
        avatarImage = image
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
            .irisHoverPointer()
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
            .irisHoverPointer()
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

/// A reusable button that copies a string and briefly swaps its label to
/// "Copied" without changing the button's width.
struct IrisCopyButton: View {
    let label: String
    let value: String
    var copiedLabel: String = "Copied"
    var systemImage: String? = "doc.on.doc"
    var copiedSystemImage: String? = "checkmark"
    var compact: Bool = true
    var feedbackDuration: Double = 1.4

    @State private var copied = false
    @State private var resetTask: Task<Void, Never>?

    var body: some View {
        Button(action: copy) {
            ZStack {
                inner(text: label, icon: systemImage)
                    .opacity(copied ? 0 : 1)
                inner(text: copiedLabel, icon: copiedSystemImage)
                    .opacity(copied ? 1 : 0)
                    .accessibilityHidden(true)
            }
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: compact))
        .accessibilityLabel(copied ? copiedLabel : label)
    }

    @ViewBuilder
    private func inner(text: String, icon: String?) -> some View {
        if let icon, !icon.isEmpty {
            Label(text, systemImage: icon)
        } else {
            Text(text)
        }
    }

    private func copy() {
        PlatformClipboard.setString(value)
        resetTask?.cancel()
        withAnimation(.easeInOut(duration: 0.15)) {
            copied = true
        }
        resetTask = Task { [feedbackDuration] in
            try? await Task.sleep(nanoseconds: UInt64(feedbackDuration * 1_000_000_000))
            if !Task.isCancelled {
                await MainActor.run {
                    withAnimation(.easeInOut(duration: 0.15)) {
                        copied = false
                    }
                }
            }
        }
    }
}

struct IrisModalCloseButton: View {
    enum Tone {
        case standard
        case light
    }

    @Environment(\.irisPalette) private var palette
    let accessibilityLabel: String
    let tone: Tone
    let iconSize: CGFloat
    let hitSize: CGFloat
    let action: () -> Void

    init(
        accessibilityLabel: String = "Close",
        tone: Tone = .standard,
        iconSize: CGFloat = 22,
        hitSize: CGFloat = 38,
        action: @escaping () -> Void
    ) {
        self.accessibilityLabel = accessibilityLabel
        self.tone = tone
        self.iconSize = iconSize
        self.hitSize = hitSize
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: "xmark.circle.fill")
                .font(.system(size: iconSize, weight: .semibold))
                .foregroundStyle(foregroundColor)
                .frame(width: hitSize, height: hitSize)
                .contentShape(Circle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel(accessibilityLabel)
    }

    private var foregroundColor: Color {
        switch tone {
        case .standard:
            return palette.muted
        case .light:
            return Color.white.opacity(0.9)
        }
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
    let isMuted: Bool
    let isPinned: Bool
    let preview: String
    let subtitle: String?
    let timeLabel: String?
    let unreadCount: UInt64
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let onTap: () -> Void

    init(
        title: String,
        isMuted: Bool = false,
        isPinned: Bool = false,
        preview: String,
        subtitle: String?,
        timeLabel: String?,
        unreadCount: UInt64,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        onTap: @escaping () -> Void
    ) {
        self.title = title
        self.isMuted = isMuted
        self.isPinned = isPinned
        self.preview = preview
        self.subtitle = subtitle
        self.timeLabel = timeLabel
        self.unreadCount = unreadCount
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.onTap = onTap
    }

    var body: some View {
        Button(action: onTap) {
            HStack(alignment: .top, spacing: 14) {
                IrisAvatar(
                    label: title,
                    emphasize: unreadCount > 0,
                    pictureUrl: pictureUrl,
                    preferences: preferences,
                    manager: manager
                )

                VStack(alignment: .leading, spacing: 5) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        HStack(alignment: .firstTextBaseline, spacing: 5) {
                            Text(title)
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)
                                .lineLimit(1)

                            if isMuted {
                                Image(systemName: "bell.slash.fill")
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(palette.muted)
                                    .accessibilityLabel("muted")
                            }

                            if isPinned {
                                Image(systemName: "pin.fill")
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(palette.muted)
                                    .accessibilityLabel("pinned")
                            }
                        }
                        .layoutPriority(1)

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

                Text(unreadCount > 99 ? "99+" : "\(max(unreadCount, 1))")
                    .font(.system(.caption, design: .rounded, weight: .bold))
                    .foregroundStyle(unreadCount > 0 ? palette.onAccent : Color.clear)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Capsule(style: .continuous).fill(unreadCount > 0 ? palette.accent : Color.clear))
                    .accessibilityHidden(unreadCount == 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
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
    @State private var showingAttachmentMenu = false
    #if canImport(PhotosUI)
    @State private var showingPhotoPicker = false
    @State private var pickedPhotos: [PhotosPickerItem] = []
    #endif

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
                    presentAttachmentSource()
                } label: {
                    Image(systemName: isUploading ? "ellipsis.circle.fill" : "paperclip")
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle((isSending || isUploading) ? palette.muted.opacity(0.54) : palette.textPrimary)
                        .frame(width: 42, height: 46)
                }
                .buttonStyle(.irisPlain)
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
                    .buttonStyle(.irisPlain)
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
        .padding(.vertical, 10)
        .background(
            Rectangle()
                .fill(palette.toolbar)
        )
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
        .confirmationDialog("Attach", isPresented: $showingAttachmentMenu, titleVisibility: .hidden) {
            #if canImport(PhotosUI)
            Button("Photo Library") { showingPhotoPicker = true }
            #endif
            Button("Files") { showingAttachmentPicker = true }
            Button("Cancel", role: .cancel) {}
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

    private func presentAttachmentSource() {
        #if canImport(PhotosUI)
        showingAttachmentMenu = true
        #else
        showingAttachmentPicker = true
        #endif
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
            .irisHoverPointer()
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
