import Foundation

enum AppleBlePeerToken {
    static func forCharacteristic(_ peripheral: UUID, instance: Int) -> String {
        "\(peripheral.uuidString).\(instance)"
    }

    static func peripheralIdentifier(_ token: String) -> UUID? {
        let parts = token.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 1 || (parts.count == 2 && Int(parts[1]).map { $0 >= 0 } == true) else {
            return nil
        }
        return UUID(uuidString: String(parts[0]))
    }
}
