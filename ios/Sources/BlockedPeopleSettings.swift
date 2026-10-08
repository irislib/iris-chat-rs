import SwiftUI

struct BlockedPeopleSettings: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisSectionCard {
            CardHeader(title: "Blocked people")
            if manager.state.blockedPeople.isEmpty {
                Text("No blocked people")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(manager.state.blockedPeople, id: \.ownerPubkeyHex) { person in
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(person.displayLabel).lineLimit(1)
                            Text(person.userId)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        Spacer(minLength: 0)
                        Button("Unblock") {
                            manager.setUserBlocked(person.ownerPubkeyHex, blocked: false)
                        }
                        .accessibilityLabel("Unblock \(person.displayLabel)")
                        .accessibilityIdentifier("settingsUnblock-\(person.ownerPubkeyHex)")
                    }
                }
            }
        }
        .accessibilityIdentifier("settingsBlockedPeople")
    }
}
