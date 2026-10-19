import Foundation
import SwiftUI

struct IrisUserProfile: Equatable {
    let name: String
    let pictureUrl: String?
    let socialConnection: SocialConnectionSnapshot?
}

/// Account-scoped presentation data from the core. Views select a user by ID;
/// they do not maintain their own copies of profile metadata or fetch images.
@MainActor
final class IrisProfileStore: ObservableObject {
    @Published private(set) var profiles: [String: IrisUserProfile] = [:]
    private var accountID: String?
    private var authoritativeOwners = Set<String>()

    subscript(owner: String) -> IrisUserProfile? { profiles[owner] }

    func apply(_ state: AppState) {
        let account = state.account?.publicKeyHex
        var next = account == accountID ? profiles : [:]
        accountID = account
        authoritativeOwners = []
        guard account != nil else { publish([:]); return }
        func put(_ owner: String, _ name: String, _ picture: String?, _ social: SocialConnectionSnapshot?) {
            guard !owner.isEmpty else { return }
            next[owner] = IrisUserProfile(name: name, pictureUrl: picture, socialConnection: social)
            authoritativeOwners.insert(owner)
        }
        for chat in state.chatList where chat.kind == .direct {
            put(chat.chatId, chat.displayName, chat.pictureUrl, chat.socialConnection)
        }
        for person in state.blockedPeople {
            put(person.ownerPubkeyHex, person.displayLabel, person.pictureUrl, person.socialConnection)
        }
        if let group = state.groupDetails {
            for member in group.members {
                put(member.ownerPubkeyHex, member.displayName, member.pictureUrl, member.socialConnection)
            }
        }
        if let chat = state.currentChat {
            // Historical message metadata must never restore a photo that the
            // current profile deliberately removed.
            for message in chat.messages {
                if let owner = message.authorOwnerPubkeyHex, next[owner] == nil {
                    next[owner] = IrisUserProfile(name: message.author, pictureUrl: message.authorPictureUrl, socialConnection: nil)
                }
            }
            for person in chat.participants {
                put(person.ownerPubkeyHex, person.displayName, person.pictureUrl, person.socialConnection)
            }
            if chat.kind == .direct {
                put(chat.chatId, chat.displayName, chat.pictureUrl, chat.socialConnection)
            }
        }
        if let local = state.account {
            put(local.publicKeyHex, local.displayName, local.pictureUrl, nil)
        }
        publish(next)
    }

    func apply(_ results: SearchResultSnapshot, account: String?, replaceExisting: Bool = true) {
        guard account != nil, account == accountID else { return }
        var next = profiles
        for person in results.people {
            guard replaceExisting || next[person.ownerPubkeyHex] == nil else { continue }
            next[person.ownerPubkeyHex] = IrisUserProfile(name: person.displayLabel,
                pictureUrl: person.pictureUrl, socialConnection: person.socialConnection)
        }
        for chat in results.contacts where chat.kind == .direct {
            guard replaceExisting || next[chat.chatId] == nil else { continue }
            next[chat.chatId] = IrisUserProfile(name: chat.displayName,
                pictureUrl: chat.pictureUrl, socialConnection: chat.socialConnection)
        }
        publish(next)
    }

    func apply(_ nearby: DesktopNearbySnapshot) {
        guard accountID != nil else { return }
        var next = profiles
        for peer in nearby.peers {
            guard let owner = peer.ownerPubkeyHex, !authoritativeOwners.contains(owner) else { continue }
            next[owner] = IrisUserProfile(name: peer.name, pictureUrl: peer.pictureUrl, socialConnection: nil)
        }
        publish(next)
    }

    private func publish(_ next: [String: IrisUserProfile]) {
        if profiles != next { profiles = next }
    }
}

struct IrisAvatarContext {
    let manager: AppManager
    let profiles: IrisProfileStore
    let preferences: PreferencesSnapshot

    @MainActor init(manager: AppManager) {
        self.manager = manager
        self.profiles = manager.profiles
        self.preferences = manager.state.preferences
    }
}

private struct IrisAvatarContextKey: EnvironmentKey {
    static let defaultValue: IrisAvatarContext? = nil
}

extension EnvironmentValues {
    var irisAvatarContext: IrisAvatarContext? {
        get { self[IrisAvatarContextKey.self] }
        set { self[IrisAvatarContextKey.self] = newValue }
    }
}
