import Foundation

/// The browser's explicit Open in app action uses the registered scheme to
/// avoid same-domain universal links remaining in Safari. Keep invitation
/// fragments intact and only accept the canonical chat host.
enum IrisChatLinks {
    static func action(for incomingURL: URL) -> AppAction? {
        guard var components = URLComponents(url: incomingURL, resolvingAgainstBaseURL: false),
              ["https", "irischat"].contains(components.scheme?.lowercased() ?? ""),
              components.host?.lowercased() == "chat.iris.to",
              components.user == nil, components.password == nil,
              components.port == nil || components.port == 443 else { return nil }
        components.scheme = "https"
        guard let url = components.url else { return nil }
        if isInviteChatLink(url) {
            return .acceptInvite(inviteInput: url.absoluteString)
        }
        for candidate in chatLinkPeerCandidates(url) {
            let normalized = normalizePeerInput(input: candidate)
            if !normalized.isEmpty, isValidPeerInput(input: normalized) {
                return .createChat(peerInput: normalized)
            }
        }
        return nil
    }
}

private func isInviteChatLink(_ url: URL) -> Bool {
    if url.pathComponents.dropFirst().first?.lowercased() == "invite",
       url.pathComponents.count >= 3 {
        return true
    }

    let fragmentComponents = chatLinkFragmentComponents(url)
    if fragmentComponents.first?.lowercased() == "invite" && fragmentComponents.count >= 2 {
        return true
    }

    guard let fragment = url.fragment else {
        return false
    }
    let decoded = fragment.removingPercentEncoding ?? fragment
    return decoded.contains("\"ephemeralKey\"") && decoded.contains("\"sharedSecret\"")
}

private func chatLinkPeerCandidates(_ url: URL) -> [String] {
    var candidates: [String] = []

    if let lastPathComponent = url.pathComponents.last,
       lastPathComponent != "/" {
        candidates.append(lastPathComponent)
    }

    if let firstFragmentComponent = chatLinkFragmentComponents(url).first {
        candidates.append(firstFragmentComponent)
    }

    if let fragment = url.fragment,
       !fragment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        candidates.append(fragment)
    }

    return candidates
}

private func chatLinkFragmentComponents(_ url: URL) -> [String] {
    guard let fragment = url.fragment else {
        return []
    }

    return fragment
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .drop(while: { $0 == "/" })
        .split(separator: "/")
        .map(String.init)
        .filter { !$0.isEmpty }
}
