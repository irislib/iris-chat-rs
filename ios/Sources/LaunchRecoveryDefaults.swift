import Foundation

enum LaunchRecoveryDefaults {
    static let pendingKey = "launchRecovery.pending"
    static let launchIDKey = "launchRecovery.launchID"
    static let versionKey = "launchRecovery.version"
    static let startedAtKey = "launchRecovery.startedAt"
    static let disabledVersionKey = "launchRecovery.disabledVersion"

    static func clear(userDefaults: UserDefaults) {
        userDefaults.removeObject(forKey: pendingKey)
        userDefaults.removeObject(forKey: launchIDKey)
        userDefaults.removeObject(forKey: versionKey)
        userDefaults.removeObject(forKey: startedAtKey)
        userDefaults.removeObject(forKey: disabledVersionKey)
    }
}
