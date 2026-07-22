import Foundation

enum MobilePushDeliveryProbe {
    private static let appGroupIdentifier = "group.fi.siriusbusiness.irischat"
    private static let armedFilename = "mobile-push-e2e-armed"
    private static let receiptFilename = "mobile-push-e2e-receipt"

    static func arm(fileManager: FileManager = .default) throws -> String {
        let id = UUID().uuidString
        let urls = try probeURLs(fileManager: fileManager)
        try? fileManager.removeItem(at: urls.receipt)
        try Data(id.utf8).write(to: urls.armed, options: .atomic)
        return id
    }

    static func recordIfArmed(fileManager: FileManager = .default) {
        guard let urls = try? probeURLs(fileManager: fileManager),
              let id = try? Data(contentsOf: urls.armed),
              !id.isEmpty else {
            return
        }
        try? id.write(to: urls.receipt, options: .atomic)
        try? fileManager.removeItem(at: urls.armed)
    }

    static func received(id: String, fileManager: FileManager = .default) -> Bool {
        guard let urls = try? probeURLs(fileManager: fileManager),
              let data = try? Data(contentsOf: urls.receipt) else {
            return false
        }
        return String(decoding: data, as: UTF8.self) == id
    }

    static func clear(fileManager: FileManager = .default) {
        guard let urls = try? probeURLs(fileManager: fileManager) else {
            return
        }
        try? fileManager.removeItem(at: urls.armed)
        try? fileManager.removeItem(at: urls.receipt)
    }

    private static func probeURLs(
        fileManager: FileManager
    ) throws -> (armed: URL, receipt: URL) {
        guard let root = fileManager.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            throw CocoaError(.fileNoSuchFile)
        }
        return (
            root.appendingPathComponent(armedFilename),
            root.appendingPathComponent(receiptFilename)
        )
    }
}
