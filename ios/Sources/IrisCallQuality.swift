import Foundation

enum IrisCallQuality: String, CaseIterable, Identifiable {
    case automatic = "auto", high, saveData = "data", custom

    var id: String { rawValue }
    var label: String {
        switch self {
        case .automatic: return "Auto"
        case .high: return "High quality"
        case .saveData: return "Save data"
        case .custom: return "Custom"
        }
    }
    var captureHeight: Int { self == .high ? 1080 : 720 }

    func maximumBitrate(customKilobits: Int) -> Int {
        switch self {
        case .automatic: return 2_000_000
        case .high: return 4_000_000
        case .saveData: return 400_000
        case .custom: return min(10_000, max(100, customKilobits)) * 1_000
        }
    }
}
