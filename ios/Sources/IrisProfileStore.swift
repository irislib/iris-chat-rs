import Foundation
import SwiftUI

struct IrisUserProfile: Equatable {
    let name: String
    let isFallback: Bool
    let pictureUrl: String?
    let socialConnection: SocialConnectionSnapshot?
}

/// Thin observation adapter. Rust owns profile data and precedence; a row
/// performs one memory lookup by identity, with no snapshot merging here.
@MainActor
final class IrisProfileStore: ObservableObject {
    private var client: RustAppClient?
    private var accountID: String?
    private var revision: UInt64?

    func bind(_ client: RustAppClient) {
        self.client = client
        revision = nil
        objectWillChange.send()
    }

    subscript(owner: String) -> IrisUserProfile? {
        guard accountID != nil, let profile = client?.userProfile(ownerPubkeyHex: owner) else { return nil }
        return IrisUserProfile(name: profile.name.name, isFallback: profile.name.isFallback,
            pictureUrl: profile.pictureUrl, socialConnection: profile.socialConnection)
    }

    func apply(_ state: AppState) {
        let account = state.account?.publicKeyHex
        guard account != accountID || revision != state.userDiscoveryRevision else { return }
        objectWillChange.send()
        accountID = account
        revision = state.userDiscoveryRevision
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
