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

/// Shows a pointing-hand cursor while hovered. No-op outside macOS.
struct IrisHoverPointerModifier: ViewModifier {
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

struct IrisUnpressedButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .irisHoverPointer()
    }
}

extension ButtonStyle where Self == IrisUnpressedButtonStyle {
    static var irisUnpressed: IrisUnpressedButtonStyle { IrisUnpressedButtonStyle() }
}

struct IrisControlTintModifier: ViewModifier {
    @Environment(\.irisPalette) private var palette

    func body(content: Content) -> some View {
        content
            // brand-purple ok: control tint colors switch tracks and
            // other filled control surfaces, not custom text/icons.
            .tint(palette.accent)
    }
}

extension View {
    func irisControlTint() -> some View {
        modifier(IrisControlTintModifier())
    }
}

// Translucent surface used for the chat composer, the floating
// scroll-to-bottom button, and any other "floating chrome" element
// that should let the timeline ghost through. Mirrors Signal-iOS's
// strategy:
//   * iOS 26+ : SwiftUI's native `.glassEffect`, which is the
//     Liquid Glass material — adapts to content underneath, stays
//     interactive, no extra tint needed.
//   * iOS 16-25: `.regularMaterial` blur — the thickest of the
//     visible-translucent materials, no palette tint so the blur
//     itself reads as the surface.
//   * Reduce Transparency: solid panel-tone fill so accessibility
//     users get the same affordance without the blur.
struct IrisGlassSurface<S: Shape>: ViewModifier {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.irisPalette) private var palette
    let shape: S
    let isInteractive: Bool
    let solidWhenReduced: Bool

    init(shape: S, isInteractive: Bool = true, solidWhenReduced: Bool = true) {
        self.shape = shape
        self.isInteractive = isInteractive
        self.solidWhenReduced = solidWhenReduced
    }

    func body(content: Content) -> some View {
        if reduceTransparency && solidWhenReduced {
            content.background(palette.toolbar, in: shape)
        } else {
            #if os(iOS)
            if #available(iOS 26.0, *) {
                // Apple Liquid Glass — the real thing. Content
                // underneath shows through with light bending,
                // depth, and adaptive contrast.
                content.glassEffect(
                    isInteractive ? .regular.interactive() : .regular,
                    in: shape
                )
            } else {
                content.background(.regularMaterial, in: shape)
            }
            #elseif os(macOS)
            if #available(macOS 26.0, *) {
                content.glassEffect(
                    isInteractive ? .regular.interactive() : .regular,
                    in: shape
                )
            } else {
                content.background(.regularMaterial, in: shape)
            }
            #else
            content.background(.regularMaterial, in: shape)
            #endif
        }
    }
}

extension View {
    /// Apply a Signal-style translucent "glass" surface to the view.
    /// `shape` is the bounds the surface fills (typically a
    /// `RoundedRectangle` or `Capsule`).
    func irisGlassSurface<S: Shape>(
        in shape: S,
        isInteractive: Bool = true,
        solidWhenReduced: Bool = true
    ) -> some View {
        modifier(IrisGlassSurface(shape: shape, isInteractive: isInteractive, solidWhenReduced: solidWhenReduced))
    }

    /// Anchor a `ScrollView`'s initial offset to the bottom edge so a
    /// long chat opens already scrolled to the latest message — the
    /// SwiftUI equivalent of Signal's "measure all rows, then
    /// `setContentOffset(maxY)` instantly" trick. Newer OS releases
    /// scope this to the initial offset only, so content growth does
    /// not fight a user's manual scroll. iOS 16/macOS 13 fall back to
    /// the manual `proxy.scrollTo(.bottom)` path.
    @ViewBuilder
    func irisDefaultScrollAnchorBottom() -> some View {
        if #available(iOS 18.0, macOS 15.0, *) {
            self.defaultScrollAnchor(.bottom, for: .initialOffset)
        } else if #available(iOS 17.0, macOS 14.0, *) {
            self.defaultScrollAnchor(.bottom)
        } else {
            self
        }
    }
}

enum IrisLayout {
    #if canImport(AppKit)
    static let usesDesktopChrome = true
    static let chromeMaxWidth: CGFloat = 1240
    static let scrollMaxWidth: CGFloat = 1100
    static let chatMaxWidth: CGFloat = 1240
    /// Per-message bubble width cap on macOS. Without this, a single
    /// long message could stretch most of the chat pane (~830pt) which
    /// looks weird in a chat UI — every other desktop messenger keeps
    /// the bubble at ~half the column. Short bubbles still hug content
    /// because the bubble is rendered inside a fixedSize+frame trick;
    /// only the wrap point shifts.
    static let chatBubbleMaxWidth: CGFloat = 480
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
    /// iOS phone screens are narrow enough that natural row sizing
    /// already keeps bubbles in a reasonable range — no cap.
    static let chatBubbleMaxWidth: CGFloat? = nil
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
    let action: Color
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
        bubbleTheirs: Color(hex: 0xE9E9E9),
        accent: Color(hex: 0x702ACE),
        action: Color(hex: 0x2267F5),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: Color(hex: 0x0F1419),
        muted: Color(hex: 0x536471),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: Color(hex: 0x0F1419)
    )

    static let dark = IrisPalette(
        background: Color(hex: 0x000000),
        panel: Color(hex: 0x161616),
        panelAlt: Color(hex: 0x262626),
        border: Color.white.opacity(0.12),
        toolbar: Color(hex: 0x0A0A0A).opacity(0.96),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0x2C2C2E),
        accent: Color(hex: 0x702ACE),
        action: Color(hex: 0x2D70FA),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: .white,
        muted: Color(hex: 0xD1D5DB),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: .white
    )

    static let lightPresented = IrisPalette(
        background: Color(hex: 0xEFEFF0),
        panel: Color(hex: 0xFFFFFF),
        panelAlt: Color(hex: 0xFFFFFF),
        border: Color.black.opacity(0.08),
        toolbar: Color(hex: 0xFFFFFF).opacity(0.96),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0xE9E9E9),
        accent: Color(hex: 0x702ACE),
        action: Color(hex: 0x2267F5),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: Color(hex: 0x0F1419),
        muted: Color(hex: 0x536471),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: Color(hex: 0x0F1419)
    )

    static let darkPresented = IrisPalette(
        background: Color(hex: 0x1C1C1E),
        panel: Color(hex: 0x2C2C2E),
        panelAlt: Color(hex: 0x2C2C2E),
        border: Color.white.opacity(0.12),
        toolbar: Color(hex: 0x1C1C1E).opacity(0.96),
        bubbleMine: Color(hex: 0x702ACE),
        bubbleTheirs: Color(hex: 0x2C2C2E),
        accent: Color(hex: 0x702ACE),
        action: Color(hex: 0x2D70FA),
        accentAlt: Color(hex: 0xDB8216),
        textPrimary: .white,
        muted: Color(hex: 0xEBEBF5).opacity(0.7),
        onAccent: .white,
        onBubbleMine: .white,
        onBubbleTheirs: .white
    )
}

struct IrisPaletteKey: EnvironmentKey {
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
    private static let centeredSideSlotWidth: CGFloat = 64

    let title: String
    let subtitle: String?
    let subtitleSystemImage: String?
    let isChatHeader: Bool
    let centerTitle: Bool
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
        isChatHeader: Bool = false,
        centerTitle: Bool = false,
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
        self.isChatHeader = isChatHeader
        self.centerTitle = centerTitle
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
        HStack(spacing: isChatHeader ? 12 : 8) {
            titleAccessoryLeading
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(
                        isChatHeader
                            ? .system(size: 17, weight: .semibold)
                            : .system(.title3, design: .rounded, weight: .bold)
                    )
                    .lineLimit(1)
                    .foregroundStyle(palette.textPrimary)

                if let subtitle, !subtitle.isEmpty {
                    HStack(spacing: 4) {
                        if let subtitleSystemImage {
                            Image(systemName: subtitleSystemImage)
                                .font(.system(size: 10, weight: .semibold))
                        }

                        Text(subtitle)
                            .font(
                                isChatHeader
                                    ? .system(size: 13, weight: .medium)
                                    : .system(.caption2, design: .rounded, weight: .semibold)
                            )
                    }
                    .foregroundStyle(isChatHeader ? palette.textPrimary : palette.muted)
                    .lineLimit(1)
                }
            }
        }
    }

    var body: some View {
        Group {
            if centerTitle && !canGoBack {
                centeredTitleBar
            } else {
                leadingTitleBar
            }
        }
        // Tight horizontal padding — the chevron / attach button
        // sits closer to the screen edge so it lines up cleanly with
        // the composer's leading control. 6pt bottom padding gives
        // the title cluster breathing room from whatever sits
        // beneath the bar (the offline banner stripe, the day chip
        // at the top of the timeline, …).
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 12 : 8)
        .padding(.bottom, isChatHeader ? 4 : 6)
        .frame(maxWidth: IrisLayout.chromeMaxWidth)
        .frame(maxWidth: .infinity)
    }

    private var leadingTitleBar: some View {
        HStack(spacing: isChatHeader ? 8 : 14) {
            leadingSlot

            Group {
                if let onTitleTap {
                    Button(action: onTitleTap) {
                        titleContent
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("chatHeaderTitleButton")
                } else {
                    titleContent
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            trailing
                .frame(minWidth: 44, alignment: .trailing)
        }
    }

    private var centeredTitleBar: some View {
        ZStack {
            titleContent
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, Self.centeredSideSlotWidth + 24)
                .allowsHitTesting(false)
                .zIndex(0)

            HStack(spacing: 0) {
                leadingSlot
                    .frame(width: Self.centeredSideSlotWidth, alignment: .leading)
                    .zIndex(2)
                Spacer(minLength: 0)
                trailing
                    .frame(width: Self.centeredSideSlotWidth, height: 48, alignment: .trailing)
                    .zIndex(2)
            }
            .zIndex(1)
        }
        .frame(height: 48)
    }

    @ViewBuilder
    private var leadingSlot: some View {
        if canGoBack {
            Button(action: onBack) {
                ZStack(alignment: .topTrailing) {
                    // Match the composer's attach button: 40pt
                    // glass circle so the two are visually a
                    // pair sitting at the same horizontal inset.
                    // The 48pt content shape (visible disc still
                    // pinned to the leading edge so the chevron
                    // stays at x=8) gives the button some extra
                    // hit area on the trailing side, so an off-
                    // center thumb tap toward the title doesn't
                    // slip past the disc.
                    Image(systemName: "chevron.left")
                        .font(.system(size: 17, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                        .frame(width: 40, height: 40)
                        .irisGlassSurface(in: Circle())
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
                .frame(width: 48, height: 48, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Back")
            .accessibilityIdentifier("navigationBackButton")
        } else {
            leading
                .frame(minWidth: 44, alignment: .leading)
        }
    }
}

enum IrisNavigationHeaderMetrics {
    static let barHeight: CGFloat = 48
    static let fadeTailHeight: CGFloat = 28

    static func contentTopInset(topSafeArea: CGFloat, isChatHeader: Bool) -> CGFloat {
        topSafeArea + barHeight + (isChatHeader ? 4 : 6)
    }

    static func chromeHeight(topSafeArea: CGFloat, isChatHeader: Bool) -> CGFloat {
        contentTopInset(topSafeArea: topSafeArea, isChatHeader: isChatHeader) + fadeTailHeight
    }
}

struct IrisNavigationHeaderTopInsetKey: EnvironmentKey {
    static let defaultValue: CGFloat = 0
}

extension EnvironmentValues {
    var irisNavigationHeaderTopInset: CGFloat {
        get { self[IrisNavigationHeaderTopInsetKey.self] }
        set { self[IrisNavigationHeaderTopInsetKey.self] = newValue }
    }
}

struct IrisNavigationHeaderChrome: View {
    let palette: IrisPalette
    let height: CGFloat

    init(palette: IrisPalette, height: CGFloat = IrisNavigationHeaderMetrics.chromeHeight(topSafeArea: 0, isChatHeader: true)) {
        self.palette = palette
        self.height = height
    }

    var body: some View {
        LinearGradient(
            colors: [
                palette.background.opacity(0.74),
                palette.background.opacity(0.60),
                palette.background.opacity(0.28),
                palette.background.opacity(0)
            ],
            startPoint: .top,
            endPoint: .bottom
        )
        .frame(height: height, alignment: .top)
        .allowsHitTesting(false)
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
