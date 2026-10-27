import Foundation

// Optional thumbnail bytes keep older app-group snapshots readable. The extension
// never fetches profile URLs or starts an account/network session.
struct ShareSuggestionEntry: Codable {
    let chatId: String
    let displayName: String
    let subtitle: String?
    let pictureUrl: String?
    let isGroup: Bool
    let lastMessageAtSecs: UInt64?
    var avatarData: Data? = nil
}
