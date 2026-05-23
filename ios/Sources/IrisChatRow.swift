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

enum IrisChatListRowMetrics {
    static let avatarSize: CGFloat = 56
    static let horizontalPadding: CGFloat = 16
    static let verticalPadding: CGFloat = 12
    static let avatarTextSpacing: CGFloat = 12
    static let textRowSpacing: CGFloat = 6
    static let titleAccessorySpacing: CGFloat = 5
    static let textStackSpacing: CGFloat = 1
    static let muteIconSize: CGFloat = 16
}

struct IrisChatRow: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let isMuted: Bool
    let isPinned: Bool
    let preview: String
    let draftPreview: String?
    let subtitle: String?
    let timeLabel: String?
    let unreadCount: UInt64
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let leading: AnyView?
    let previewLeading: AnyView?
    let onTap: () -> Void

    init(
        title: String,
        isMuted: Bool = false,
        isPinned: Bool = false,
        preview: String,
        draftPreview: String? = nil,
        subtitle: String?,
        timeLabel: String?,
        unreadCount: UInt64,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        leading: AnyView? = nil,
        previewLeading: AnyView? = nil,
        onTap: @escaping () -> Void
    ) {
        self.title = title
        self.isMuted = isMuted
        self.isPinned = isPinned
        self.preview = preview
        self.draftPreview = draftPreview
        self.subtitle = subtitle
        self.timeLabel = timeLabel
        self.unreadCount = unreadCount
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.leading = leading
        self.previewLeading = previewLeading
        self.onTap = onTap
    }

    var body: some View {
        Button(action: onTap) {
            HStack(alignment: .center, spacing: IrisChatListRowMetrics.avatarTextSpacing) {
                if let leading {
                    leading
                } else {
                    IrisAvatar(
                        label: title,
                        size: IrisChatListRowMetrics.avatarSize,
                        emphasize: unreadCount > 0,
                        pictureUrl: pictureUrl,
                        preferences: preferences,
                        manager: manager
                    )
                }

                VStack(alignment: .leading, spacing: IrisChatListRowMetrics.textStackSpacing) {
                    HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.textRowSpacing) {
                        HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.titleAccessorySpacing) {
                            Text(title)
                                .font(.headline)
                                .foregroundStyle(palette.textPrimary)
                                .lineLimit(1)

                            if isMuted {
                                Image(systemName: "bell.slash.fill")
                                    .font(.system(size: IrisChatListRowMetrics.muteIconSize, weight: .semibold))
                                    .foregroundStyle(palette.muted)
                                    .accessibilityLabel("muted")
                            }

                        }
                        .layoutPriority(1)

                        Spacer(minLength: 8)

                        if let timeLabel, !timeLabel.isEmpty {
                            Text(timeLabel)
                                .font(.subheadline)
                                .foregroundStyle(palette.muted)
                                .lineLimit(1)
                        }
                    }

                    HStack(alignment: .center, spacing: IrisChatListRowMetrics.textRowSpacing) {
                        if let previewLeading {
                            previewLeading
                        }
                        previewText
                            .font(.subheadline)
                            .foregroundStyle(palette.muted)
                            .lineLimit(2, reservesSpace: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .layoutPriority(1)

                        unreadBadge
                    }

                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.caption.weight(.medium))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
            }
            .padding(.horizontal, IrisChatListRowMetrics.horizontalPadding)
            .padding(.vertical, IrisChatListRowMetrics.verticalPadding)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
    }

    @ViewBuilder
    private var unreadBadge: some View {
        IrisUnreadBadge(count: unreadCount)
    }

    private var previewText: Text {
        if let draftPreview {
            return Text("Draft: ").italic() + Text(draftPreview)
        }
        return Text(preview)
    }
}

// Signal-style send button label. On iOS 26+ we get the real
// `prominentGlass` configuration via .glassEffect with a tint —
// the button reads as a colored glass disc that the timeline
// bends light through, distinct from a flat accent bubble. On
// older iOS we approximate with a solid accent fill + a bright
// halo ring so it can't be confused with an outgoing bubble.
struct IrisSendButtonLabel: View {
    @Environment(\.irisPalette) private var palette
    let isSending: Bool

    var body: some View {
        let icon = Image(systemName: isSending ? "ellipsis.circle.fill" : "arrow.up")
            .font(.system(size: 17, weight: .bold))
            .foregroundStyle(palette.onAccent)
            .frame(width: 40, height: 40)
        #if os(iOS)
        if #available(iOS 26.0, *) {
            return AnyView(
                icon
                    .glassEffect(
                        .regular.tint(palette.accent).interactive(),
                        in: Circle()
                    )
                    .overlay(
                        Circle()
                            .strokeBorder(Color.white.opacity(0.55), lineWidth: 1)
                    )
            )
        } else {
            return AnyView(
                icon
                    .background(Circle().fill(palette.accent))
                    .overlay(
                        Circle()
                            .strokeBorder(Color.white.opacity(0.55), lineWidth: 1)
                    )
                    .overlay(
                        Circle()
                            .strokeBorder(palette.accent.opacity(0.6), lineWidth: 1)
                            .padding(-2)
                    )
            )
        }
        #else
        return AnyView(
            icon
                .background(Circle().fill(palette.accent))
                .overlay(
                    Circle()
                        .strokeBorder(Color.white.opacity(0.55), lineWidth: 1)
                )
        )
        #endif
    }
}

struct IrisDayChip: View {
    @Environment(\.irisPalette) private var palette
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.footnote, weight: .medium))
            .foregroundStyle(palette.textPrimary)
            .padding(.horizontal, 12)
            .padding(.vertical, 3)
            // Signal-style glass day separator. iOS 26+ gets a real
            // capsule glass effect; older iOS falls back to a
            // regular-material blur — both via IrisGlassSurface so
            // the same modifier path applies as the composer and FAB.
            .irisGlassSurface(in: Capsule(style: .continuous), isInteractive: false)
    }
}

struct IrisInlineDaySeparator: View {
    @Environment(\.irisPalette) private var palette
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.footnote, weight: .medium))
            .foregroundStyle(palette.muted.opacity(0.78))
            .padding(.horizontal, 12)
            .padding(.vertical, 3)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(text)
    }
}
