import SwiftUI

struct IrisFavoriteBadge: View {
    @Environment(\.irisPalette) private var palette
    let size: CGFloat

    var body: some View {
        Image(systemName: "star.fill")
            .font(.system(size: size * 0.62, weight: .semibold))
            .foregroundStyle(Color(red: 53.0 / 255, green: 41.0 / 255, blue: 0))
            .frame(width: size, height: size)
            .background(Color(red: 251.0 / 255, green: 191.0 / 255, blue: 36.0 / 255), in: Circle())
            .overlay(Circle().strokeBorder(palette.panel, lineWidth: 1))
            .accessibilityLabel("Favorite")
            .accessibilityIdentifier("favoriteAvatarBadge")
            .help("Favorite · Only you")
            .allowsHitTesting(false)
    }
}
