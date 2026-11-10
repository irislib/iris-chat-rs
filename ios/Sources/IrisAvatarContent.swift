import SwiftUI

// Dependency-free avatar rendering shared with the system share extension.
struct IrisAvatarContent: View {
    let image: Image?
    let label: String
    let size: CGFloat
    let background: Color
    let foreground: Color
    let border: Color

    var body: some View {
        ZStack {
            Circle().fill(background)
                .overlay(Circle().stroke(border, lineWidth: 1))
            if let image {
                image.resizable().scaledToFill()
                    .frame(width: size, height: size)
                    .clipShape(Circle())
            } else {
                Text(String((label.trimmingCharacters(in: .whitespacesAndNewlines).first ?? "?")).uppercased())
                    .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                    .foregroundStyle(foreground)
            }
        }
        .frame(width: size, height: size)
    }
}
