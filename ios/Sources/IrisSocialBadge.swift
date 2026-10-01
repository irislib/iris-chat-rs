import SwiftUI

struct IrisSocialBadge: View {
    @Environment(\.irisPalette) private var palette
    let connection: SocialConnectionSnapshot
    var size: CGFloat = 16

    var body: some View {
        if let badge = connection.badge {
            Image(systemName: badge == .warning ? "exclamationmark" : badge == .muted ? "speaker.slash.fill" : "checkmark")
                .font(.system(size: size * 0.6, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: size, height: size)
                .background(color(badge), in: Circle())
                .overlay(Circle().strokeBorder(palette.panel, lineWidth: 1))
                .accessibilityLabel(connection.description)
                .help(connection.description)
        }
    }

    private func color(_ badge: SocialBadge) -> Color {
        switch badge {
        case .warning: palette.accentAlt
        case .following: palette.accent
        case .friend: Color(red: 0.39, green: 0.45, blue: 0.55)
        case .trusted: palette.accentAlt
        case .muted: .red
        }
    }
}

struct IrisSocialConnectionLabel: View {
    @Environment(\.irisPalette) private var palette
    let connection: SocialConnectionSnapshot

    var body: some View {
        HStack(spacing: 6) {
            IrisSocialBadge(connection: connection).accessibilityHidden(true)
            Text(connection.description)
                .font(.subheadline)
                .foregroundStyle(palette.muted)
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("profileSocialConnection")
    }
}
