import SwiftUI

func groupMemberCandidates(
    chats: [ChatThreadSnapshot],
    localOwner: String?,
    memberOwners: Set<String>,
    query: String
) -> [ChatThreadSnapshot] {
    chats.filter {
        $0.kind == .direct && $0.chatId != localOwner && !memberOwners.contains($0.chatId)
    }.filteredByQuery(query)
}

struct GroupMemberCandidates: View {
    @Environment(\.irisPalette) private var palette
    @ScaledMetric(relativeTo: .body) private var rowHeight = 56.0
    let chats: [ChatThreadSnapshot]
    let query: String
    let selectedOwners: Set<String>
    let isBusy: Bool
    let manager: AppManager?
    let onSelect: (String) -> Void
    let onClose: () -> Void

    var body: some View {
        IrisSectionCard {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                CardHeader(title: query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Known users" : "Search results")
                Spacer()
                Button(action: onClose) {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 18, weight: .semibold))
                }
                .buttonStyle(.irisPlain)
                .foregroundStyle(palette.muted)
                .accessibilityLabel("Close search results")
                .accessibilityIdentifier("groupDetailsCloseMemberResultsButton")
            }

            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(chats.enumerated()), id: \.element.chatId) { index, chat in
                        let selected = selectedOwners.contains(chat.chatId)
                        Button {
                            onSelect(chat.chatId)
                        } label: {
                            HStack(spacing: 12) {
                                IrisAvatar(socialConnection: chat.socialConnection, ownerPubkeyHex: chat.chatId, label: chat.displayName, size: 38, emphasize: selected, manager: manager)
                                VStack(alignment: .leading, spacing: 4) {
                                    personNameText(chat.displayName, identity: chat.chatId, explicitName: explicitPersonName(nickname: chat.nickname, profileName: chat.profileName))
                                        .font(.system(.headline, design: .rounded, weight: .semibold))
                                        .foregroundStyle(palette.textPrimary)
                                    if let subtitle = secondaryDisplayName(chat.subtitle, primary: chat.displayName) {
                                        Text(subtitle)
                                            .font(.system(.footnote, design: .rounded))
                                            .foregroundStyle(palette.muted)
                                    }
                                }
                                Spacer()
                                Image(systemName: selected ? "checkmark.square.fill" : "square")
                                    .font(.system(size: 22, weight: .semibold))
                                    .foregroundStyle(selected ? palette.textPrimary : palette.muted)
                            }
                            .contentShape(Rectangle())
                        }
                        .frame(minHeight: rowHeight)
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("groupDetailsKnownUser-\(String(chat.chatId.prefix(12)))")
                        .accessibilityValue(selected ? "Selected" : "Not selected")
                        .disabled(isBusy)

                        if index < chats.count - 1 {
                            Divider().overlay(palette.border)
                        }
                    }
                }
            }
            .frame(height: CGFloat(min(chats.count, 6)) * (rowHeight + 1))
            .id(query.trimmingCharacters(in: .whitespacesAndNewlines))
            .accessibilityIdentifier("groupDetailsKnownUsersList")
        }
    }
}
