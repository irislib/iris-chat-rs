import Foundation
import SwiftUI

/// Presentation only: cached identifiers and generated labels never become a
/// saved profile name. Explicit profile names and nicknames keep their style.
extension PersonNamePresentation {
    init(_ displayName: String, identity: String?, explicitName: String? = nil) {
        self = presentPersonName(displayName: displayName, identity: identity ?? "", explicitName: explicitName)
    }
}

func personNameText(_ name: String, identity: String?, explicitName: String? = nil) -> Text {
    let presentation = PersonNamePresentation(name, identity: identity, explicitName: explicitName)
    return presentation.isFallback ? Text(presentation.name).fontDesign(.default).italic() : Text(presentation.name)
}

func explicitPersonName(for owner: String?, state: AppState?) -> String? {
    guard let owner, let state else { return nil }
    if let chat = state.currentChat, chat.kind == .direct, chat.chatId == owner {
        return explicitPersonName(nickname: chat.nickname, profileName: chat.profileName)
    }
    return state.chatList.first { $0.kind == .direct && $0.chatId == owner }.flatMap { explicitPersonName(nickname: $0.nickname, profileName: $0.profileName) }
}

func explicitPersonName(nickname: String?, profileName: String?) -> String? {
    [nickname, profileName].compactMap { value in
        value?.trimmingCharacters(in: .whitespacesAndNewlines)
    }.first { !$0.isEmpty }
}

func fallbackProfileNameForIdentity(_ identity: String) -> String {
    presentPersonName(displayName: "", identity: identity, explicitName: nil).name
}
