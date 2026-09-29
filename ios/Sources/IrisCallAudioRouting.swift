#if os(iOS)
import AVFoundation
import AVKit
import SwiftUI

struct IrisCallAudioRoute: Equatable {
    var speaker = false
    var external = false
    var name = "Audio"

    static func current(_ session: AVAudioSession) -> Self {
        let output = session.currentRoute.outputs.first
        let external = session.availableInputs?.contains { $0.portType != .builtInMic } == true ||
            session.currentRoute.outputs.contains { $0.portType != .builtInReceiver && $0.portType != .builtInSpeaker }
        return Self(speaker: output?.portType == .builtInSpeaker, external: external,
                    name: output?.portName ?? "Audio")
    }
}

/// The system owns route choice. Observe it instead of retaining a speaker
/// override that could replace a headset selected in CallKit or Control Center.
@MainActor
final class IrisCallAudioRouting {
    private let read: () -> IrisCallAudioRoute
    private let overrideSpeaker: (Bool) throws -> Void
    private var observer: NSObjectProtocol?
    var onChange: ((IrisCallAudioRoute) -> Void)?
    private(set) var route: IrisCallAudioRoute

    init(read: @escaping () -> IrisCallAudioRoute,
         overrideSpeaker: @escaping (Bool) throws -> Void) {
        self.read = read
        self.overrideSpeaker = overrideSpeaker
        route = read()
    }

    convenience init() {
        let session = AVAudioSession.sharedInstance()
        self.init(read: { .current(session) }, overrideSpeaker: {
            try session.overrideOutputAudioPort($0 ? .speaker : .none)
        })
        observer = NotificationCenter.default.addObserver(
            forName: AVAudioSession.routeChangeNotification, object: session, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func refresh() {
        route = read()
        onChange?(route)
    }

    func toggleSpeaker() throws {
        try overrideSpeaker(!read().speaker)
        refresh()
    }

    deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }
}

/// Let UIKit present and operate its route picker directly, without depending
/// on AVRoutePickerView's private subview hierarchy.
struct IrisCallRoutePicker: UIViewRepresentable {
    let routeName: String

    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.tintColor = .white
        view.activeTintColor = .white
        view.prioritizesVideoDevices = false
        view.accessibilityIdentifier = "callAudioRouteButton"
        return view
    }

    func updateUIView(_ view: AVRoutePickerView, context: Context) {
        view.accessibilityLabel = "Audio output"
        view.accessibilityValue = routeName
    }
}
#endif
