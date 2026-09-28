#if os(iOS) || os(macOS)
import Foundation
import Combine
import Network
import SwiftUI

enum IrisAudioDownloadPreference: String, CaseIterable {
    case never, wifi, wifiAndCellular
    static let defaultsKey = "audioAutoDownloadNetwork"
    var title: String {
        switch self {
        case .never: return "Never"
        case .wifi: return "Wi-Fi"
        case .wifiAndCellular: return "Wi-Fi and mobile data"
        }
    }
}

@MainActor
final class IrisAudioDownloads: ObservableObject {
    static let shared = IrisAudioDownloads()
    nonisolated static let previewLimit: UInt64 = 5 * 1024 * 1024
    @Published private(set) var network = NetworkState()
    @Published private(set) var callActive = IrisAudioActivity.isCallActive
    private let monitor = NWPathMonitor()
    private nonisolated static let latestNetwork = AudioNetworkSnapshot()
    private var callObservation: AnyCancellable?

    struct NetworkState: Equatable, Sendable {
        var connected = false
        var expensive = false
        var constrained = false
    }

    init() {
        monitor.pathUpdateHandler = { [weak self] path in
            let state = NetworkState(connected: path.status == .satisfied,
                                     expensive: path.isExpensive, constrained: path.isConstrained)
            Self.latestNetwork.set(state)
            Task { @MainActor in self?.network = state }
        }
        monitor.start(queue: DispatchQueue(label: "iris.audio-network", qos: .utility))
        callObservation = NotificationCenter.default.publisher(for: IrisAudioActivity.callDidChange)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.callActive = IrisAudioActivity.isCallActive }
    }

    deinit { monitor.cancel() }

    nonisolated static func allowsPreview(preference: IrisAudioDownloadPreference, network: NetworkState, callActive: Bool) -> Bool {
        guard network.connected, !network.constrained, !callActive else { return false }
        switch preference {
        case .never: return false
        case .wifi: return !network.expensive
        case .wifiAndCellular: return true
        }
    }

    var allowsPreview: Bool { Self.permitsAutomaticDownload() }

    nonisolated static func permitsAutomaticDownload() -> Bool {
        let preference = IrisAudioDownloadPreference(rawValue: UserDefaults.standard.string(
            forKey: IrisAudioDownloadPreference.defaultsKey) ?? "") ?? .wifiAndCellular
        return Self.allowsPreview(preference: preference, network: latestNetwork.get(), callActive: IrisAudioActivity.isCallActive)
    }
}

private final class AudioNetworkSnapshot: @unchecked Sendable {
    private let lock = NSLock()
    private var state = IrisAudioDownloads.NetworkState()
    func set(_ value: IrisAudioDownloads.NetworkState) { lock.lock(); defer { lock.unlock() }; state = value }
    func get() -> IrisAudioDownloads.NetworkState { lock.lock(); defer { lock.unlock() }; return state }
}

/// Automatic downloads run off the UI executor, one at a time.
/// Cancelled cells and newly disallowed previews are skipped before any network I/O.
actor IrisAudioPreviewDownloader {
    static let shared = IrisAudioPreviewDownloader()
    func download(nhash: String) -> Data? {
        guard !Task.isCancelled else { return nil }
        guard IrisAudioDownloads.permitsAutomaticDownload() else { return nil }
        let result = downloadHashtreeAttachmentWithLimit(nhash: nhash, maxBytes: IrisAudioDownloads.previewLimit)
        guard !Task.isCancelled, let encoded = result.dataBase64 else { return nil }
        return Data(base64Encoded: encoded)
    }
}

struct AudioDownloadSettingsSection: View {
    @AppStorage(IrisAudioDownloadPreference.defaultsKey) private var preference = IrisAudioDownloadPreference.wifiAndCellular.rawValue
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Download audio automatically")
            Picker("Download audio automatically", selection: $preference) {
                ForEach(IrisAudioDownloadPreference.allCases, id: \.rawValue) { value in
                    Text(value.title).tag(value.rawValue)
                }
            }
            .labelsHidden()
            .irisControlTint()
            .accessibilityIdentifier("audioAutoDownloadPreference")
            Text("Audio over 5 MB downloads when you tap Play.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}
#endif
