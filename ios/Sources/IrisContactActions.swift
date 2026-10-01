import SwiftUI

struct IrisContactActions: View {
    @ObservedObject var manager: AppManager
    let chat: CurrentChatSnapshot

    var body: some View {
        if let contact = chat.contactIdentity {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 16) {
                    Button {
                        manager.dispatch(.setPublicFollow(ownerPubkeyHex: chat.chatId, following: !contact.isFollowing))
                    } label: {
                        Label(contact.updatingFollow ? "Saving…" : contact.isFollowing ? "Unfollow (public)" : "Follow (public)", systemImage: contact.isFollowing ? "person.badge.minus" : "person.badge.plus")
                    }
                    .disabled(!contact.canFollow || contact.updatingFollow)
                    .accessibilityIdentifier("publicFollowButton")
                    .help(contact.canFollow ? "Visible to everyone" : "Use your main device to change public follows")
                    Button {
                        manager.dispatch(.setContactFavorite(ownerPubkeyHex: chat.chatId, favorite: !contact.isFavorite))
                    } label: {
                        Label(contact.isFavorite ? "Favorited" : "Favorite", systemImage: contact.isFavorite ? "star.fill" : "star")
                    }
                    .help("Only you can see this")
                    .accessibilityIdentifier("contactFavoriteButton")
                }
                Text("Favorites are only visible to you")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                IrisNameChangeNotice(manager: manager, chat: chat)
                if let first = contact.firstSeenName, first != contact.savedName {
                    Text("First known as \(first)").font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct IrisNameChangeNotice: View {
    @ObservedObject var manager: AppManager
    let chat: CurrentChatSnapshot

    var body: some View {
        if let contact = chat.contactIdentity, let proposed = contact.pendingName {
            VStack(alignment: .leading, spacing: 6) {
                Text("New profile name: \(proposed)")
                    .font(.subheadline)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Use new name") {
                    manager.dispatch(.approveContactName(ownerPubkeyHex: chat.chatId, name: proposed))
                }
                .accessibilityIdentifier("approveContactNameButton")
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
            .accessibilityIdentifier("contactNameChangeNotice")
        }
    }
}
