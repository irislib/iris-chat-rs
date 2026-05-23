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

    private var pillRadius: CGFloat { 22 }

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(.system(.body, design: .rounded))
            .padding(.horizontal, 16)
            // Vertical padding tuned so the input pill is the same
            // 40pt height as the attach + send glass buttons —
            // .bottom alignment in the composer's HStack then reads
            // as center-aligned in the single-line case.
            .padding(.vertical, 9)
            // Signal-style input pill: a glass capsule that floats
            // over the timeline. No solid fill — the OS glass effect
            // handles tint + blur. Hairline border keeps the shape
            // visible against same-tone backdrops.
            .irisGlassSurface(in: RoundedRectangle(cornerRadius: pillRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: pillRadius, style: .continuous)
                    .strokeBorder(palette.border.opacity(0.32), lineWidth: 0.5)
            )
    }
}

extension View {
    func irisInputField() -> some View {
        modifier(IrisInputFieldModifier())
    }
}

/// A reusable button that copies a string and briefly swaps its label to
/// "Copied" without changing the row's shape. Defaults to the rounded
/// "secondary" pill style; pass `style: .menuRow` to render as a flat,
/// left-aligned settings/options row (matches Mute chat / Block user).
struct IrisCopyButton: View {
    enum Style {
        case secondary
        case menuRow
    }

    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String
    var copiedLabel: String = "Copied"
    var systemImage: String? = "doc.on.doc"
    var copiedSystemImage: String? = "checkmark"
    var compact: Bool = true
    var feedbackDuration: Double = 1.4
    var style: Style = .secondary

    @State private var copied = false
    @State private var resetTask: Task<Void, Never>?

    var body: some View {
        switch style {
        case .secondary:
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
        case .menuRow:
            Button(action: copy) {
                HStack(spacing: 12) {
                    ZStack {
                        Image(systemName: systemImage ?? "doc.on.doc")
                            .opacity(copied ? 0 : 1)
                        Image(systemName: copiedSystemImage ?? "checkmark")
                            .opacity(copied ? 1 : 0)
                            .accessibilityHidden(true)
                    }
                    .frame(width: 24, height: 24)
                    // Keep the row width stable across the copy/copied
                    // swap so adjacent rows don't shift — overlay the
                    // two labels at the same leading edge.
                    ZStack(alignment: .leading) {
                        Text(label)
                            .opacity(copied ? 0 : 1)
                        Text(copiedLabel)
                            .opacity(copied ? 1 : 0)
                            .accessibilityHidden(true)
                    }
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Spacer(minLength: 0)
                }
                .foregroundStyle(palette.textPrimary)
                .padding(.vertical, 2)
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel(copied ? copiedLabel : label)
        }
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
    let accessibilityIdentifier: String?
    let tone: Tone
    let iconSize: CGFloat
    let hitSize: CGFloat
    let action: () -> Void

    init(
        accessibilityLabel: String = "Close",
        accessibilityIdentifier: String? = nil,
        tone: Tone = .standard,
        iconSize: CGFloat = 15,
        hitSize: CGFloat = 36,
        action: @escaping () -> Void
    ) {
        self.accessibilityLabel = accessibilityLabel
        self.accessibilityIdentifier = accessibilityIdentifier
        self.tone = tone
        self.iconSize = iconSize
        self.hitSize = hitSize
        self.action = action
    }

    @ViewBuilder
    var body: some View {
        let closeButton = Button(action: action) {
            IrisGlassCircleButtonLabel(
                systemName: "xmark",
                iconSize: iconSize,
                hitSize: hitSize,
                tone: tone == .light ? .dark : .light,
                glyphColor: glyphColor
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel(accessibilityLabel)

        if let accessibilityIdentifier {
            closeButton.accessibilityIdentifier(accessibilityIdentifier)
        } else {
            closeButton
        }
    }

    private var glyphColor: Color {
        switch tone {
        case .standard:
            return palette.textPrimary
        case .light:
            return Color.white
        }
    }
}

/// Translucent circular SF-Symbol button matching the native glass /
/// vibrancy look used in Signal's media viewer chrome. The blur is
/// applied with `UIVisualEffectView` so the live system blur shows
/// through over the photo behind it instead of falling back to a flat
/// translucent gray, which happens in some contexts when you use the
/// SwiftUI `Material` fill on a shape inside a `fullScreenCover`.
struct IrisGlassCircleButtonLabel: View {
    enum Tone {
        case light
        case dark
    }

    let systemName: String
    let iconSize: CGFloat
    let hitSize: CGFloat
    let tone: Tone
    let glyphColor: Color

    var body: some View {
        ZStack {
            IrisGlassCircleBackground(tone: tone)
                .clipShape(Circle())
            Circle()
                .strokeBorder(Color.white.opacity(tone == .dark ? 0.12 : 0.06), lineWidth: 0.5)
            Image(systemName: systemName)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .fontWeight(.semibold)
                .foregroundStyle(glyphColor)
                .frame(width: iconSize, height: iconSize)
        }
        .frame(width: hitSize, height: hitSize)
        .shadow(color: Color.black.opacity(tone == .dark ? 0.28 : 0.12), radius: 6, x: 0, y: 2)
        .contentShape(Circle())
    }
}

#if canImport(UIKit)
struct IrisGlassCircleBackground: UIViewRepresentable {
    let tone: IrisGlassCircleButtonLabel.Tone

    func makeUIView(context: Context) -> UIVisualEffectView {
        let view = UIVisualEffectView(effect: UIBlurEffect(style: style))
        view.backgroundColor = .clear
        return view
    }

    func updateUIView(_ uiView: UIVisualEffectView, context: Context) {
        uiView.effect = UIBlurEffect(style: style)
    }

    private var style: UIBlurEffect.Style {
        tone == .dark ? .systemUltraThinMaterialDark : .systemUltraThinMaterial
    }
}
#elseif canImport(AppKit)
struct IrisGlassCircleBackground: NSViewRepresentable {
    let tone: IrisGlassCircleButtonLabel.Tone

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = tone == .dark ? .hudWindow : .menu
        view.state = .active
        view.blendingMode = .withinWindow
        view.wantsLayer = true
        view.layer?.masksToBounds = true
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = tone == .dark ? .hudWindow : .menu
        let side = min(nsView.bounds.width, nsView.bounds.height)
        nsView.layer?.cornerRadius = side / 2
    }
}
#else
struct IrisGlassCircleBackground: View {
    let tone: IrisGlassCircleButtonLabel.Tone
    var body: some View {
        Circle().fill(Color.black.opacity(tone == .dark ? 0.42 : 0.12))
    }
}
#endif

struct IrisModalBackButton: View {
    @Environment(\.irisPalette) private var palette
    let accessibilityLabel: String
    let action: () -> Void

    init(accessibilityLabel: String = "Back", action: @escaping () -> Void) {
        self.accessibilityLabel = accessibilityLabel
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: "chevron.left")
                .font(.system(size: 17, weight: .bold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 40, height: 40)
                .background(Circle().fill(palette.panelAlt.opacity(0.95)))
                .frame(width: 44, height: 44)
                .contentShape(Circle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel(accessibilityLabel)
    }
}

struct IrisUnreadBadge: View {
    @Environment(\.irisPalette) private var palette

    let count: UInt64

    var body: some View {
        if count > 0 {
            Text(count > 99 ? "99+" : "\(count)")
                .font(.system(size: 13, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(palette.onAccent)
                .lineLimit(1)
                .minimumScaleFactor(0.85)
                .fixedSize(horizontal: true, vertical: false)
                .padding(.horizontal, 7)
                .frame(minWidth: 22, minHeight: 20)
                .background(Capsule(style: .continuous).fill(palette.accent))
                .layoutPriority(2)
                .accessibilityLabel("\(count) unread")
        }
    }
}

struct IrisModalSurfaceModifier: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    @ViewBuilder
    func body(content: Content) -> some View {
        let modalPalette = colorScheme == .dark ? IrisPalette.darkPresented : IrisPalette.lightPresented
#if os(iOS)
        if #available(iOS 16.4, *) {
            content
                .environment(\.irisPalette, modalPalette)
                .background(modalPalette.background)
                .tint(modalPalette.action)
                .toolbarBackground(modalPalette.background, for: .navigationBar)
                .toolbarBackground(.visible, for: .navigationBar)
                .toolbarColorScheme(colorScheme, for: .navigationBar)
                .presentationBackground(modalPalette.background)
        } else {
            content
                .environment(\.irisPalette, modalPalette)
                .background(modalPalette.background)
                .tint(modalPalette.action)
                .toolbarBackground(modalPalette.background, for: .navigationBar)
                .toolbarBackground(.visible, for: .navigationBar)
                .toolbarColorScheme(colorScheme, for: .navigationBar)
        }
#else
        content
            .environment(\.irisPalette, modalPalette)
            .background(modalPalette.background)
            .tint(modalPalette.action)
#endif
    }
}

extension View {
    func irisModalSurface() -> some View {
        modifier(IrisModalSurfaceModifier())
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
