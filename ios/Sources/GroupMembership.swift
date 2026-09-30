import SwiftUI

extension CurrentChatSnapshot {
    var isRemovedFromGroup: Bool {
        kind == .group && !participants.contains(where: \.isLocalOwner)
    }
}

struct IrisRemovedGroupBar: View {
    var body: some View {
        Text("You’re no longer in this group")
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 16)
            .padding(.vertical, 18)
            .background(.regularMaterial)
            .accessibilityIdentifier("removedGroupBar")
    }
}
